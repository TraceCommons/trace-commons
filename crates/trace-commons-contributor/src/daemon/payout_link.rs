//! IPC handlers for directing payout credit to a NEAR AI organization
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 5; ingest client in `crate::payout_link`).
//!
//! Five methods, each a read or an explicit contributor action:
//!
//! - `payout_rails_status` -- each enabled rail's link state and `version`
//!   from the commons, with this device's own check of the recipient ref;
//! - `payout_link_orgs` -- the NEAR AI organizations the signed-in NEAR AI
//!   user owns and that are active, by name, and an `orgs_revision` over
//!   what was shown. Talks only to NEAR AI;
//! - `payout_link_start` -- start linking the chosen org. Changes nothing
//!   yet;
//! - `payout_link_confirm` -- the person's confirmation, after the app shows
//!   the consent for the org they chose. Only this saves the link;
//! - `payout_link_unlink` -- start stopping payouts on a rail; confirmed
//!   with `payout_link_confirm` like a link.
//!
//! # Who authorizes a link
//!
//! The contributor's signed-in account session is the authority: the daemon
//! presents it on start and on confirm, and there is no step-up page. What stands behind
//! a link is the commons' own check with NEAR AI that the org is owned and
//! active, the "changed since shown" version, and the audit trail on both
//! sides.
//!
//! # What never leaves this device
//!
//! Org names. They are read from NEAR AI and never sent to the commons.
//! The name of the org a link from this device names (linked, or pending a
//! confirm) -- that one only, never the rest of the listing -- is kept in
//! `daemon-payout-org-names.json` (0600, written atomically), so status
//! shows it after a restart without listing again. Every read of the list
//! refreshes it, and forgetting the NEAR AI session deletes it. Names are
//! never logged. The commons receives the
//! chosen org's id and a fresh NEAR AI access token, in the start request
//! body only; the token is exchanged for that one request and dropped, never
//! stored, logged, returned over IPC or sent with the confirm.
//!
//! # Changed since shown
//!
//! Two checks, one per side. `orgs_revision` binds a start to the list the
//! person was shown: the daemon reads the list again with the fresh token
//! and refuses (`payout-link-orgs-changed`) if it differs, before anything
//! reaches the commons. `expected_current_version` (required) is the
//! `version` the app showed from `payout_rails_status`, passed through so the
//! commons refuses a start or a confirm if the account's link changed since.
//! A confirm must name the action and org the pending start was for.
//!
//! # The recipient ref
//!
//! A started link must answer the ref for the idempotency key and the org
//! this daemon chose, or it is refused and never confirmed. After a confirm
//! the daemon keeps the key and the org id (not the name) in
//! `daemon-payout-link.json`, so status can recompute the ref and report a
//! commons that holds another org as `mismatch`.
//!
//! Audit rows (`payout-link-started`, `payout-link-confirmed`,
//! `payout-unlink-started`, `payout-unlink-confirmed`) are label-only: no
//! org, name, token, handle or version.

use std::collections::BTreeMap;
use std::sync::{MutexGuard, PoisonError};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use trace_commons_protocol::payout_link::{
    NEAR_AI_PAYOUT_RAIL_ID, PayoutLinkAction, PayoutLinkOutcome, near_ai_recipient_ref,
};

use super::account_session_renewal::{RenewVia, account_session};
use super::audit::{self, AuditEntry};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::nearai_credential::api::{CloudApi, OwnedOrganization};
use super::nearai_credential::session;
use super::settings::NearAiSession;
use super::withdraw::{ERR_ACCOUNT_SESSION_REQUIRED, keep_rotated_token};
use crate::account_auth::LoadedAccountSession;
use crate::config::{
    ConfigStore, ContributorConfig, DAEMON_PAYOUT_LINK_FILE, DAEMON_PAYOUT_ORG_NAMES_FILE,
};
use crate::payout_link::{self as client, Endpoint, PayoutLinkError};

pub const AUDIT_LINK_STARTED: &str = "payout-link-started";
pub const AUDIT_LINK_CONFIRMED: &str = "payout-link-confirmed";
pub const AUDIT_UNLINK_STARTED: &str = "payout-unlink-started";
pub const AUDIT_UNLINK_CONFIRMED: &str = "payout-unlink-confirmed";

/// `payout_link_confirm` was called without a top-level `confirmed: true`.
pub const ERR_CONFIRMATION_REQUIRED: &str = "payout-link-confirmation-required";
/// The org list NEAR AI reports now is not the one the person was shown.
/// Nothing was sent to the commons; list again.
pub const ERR_ORGS_CHANGED: &str = "payout-link-orgs-changed";
/// No started request on this rail waits for a confirm on this device.
pub const ERR_START_REQUIRED: &str = "payout-link-start-required";
/// The confirm names another action or org than the started request.
pub const ERR_CHANGED: &str = "payout-link-changed";
/// No NEAR AI session on this device: sign in with NEAR AI first.
pub const ERR_NEAR_AI_SIGN_IN_REQUIRED: &str = "payout-link-near-ai-sign-in-required";
/// The NEAR AI session expired: sign in with NEAR AI again.
pub const ERR_NEAR_AI_SESSION_EXPIRED: &str = "payout-link-near-ai-session-expired";
/// The NEAR AI session changed while this call ran; try again.
pub const ERR_NEAR_AI_SESSION_CHANGED: &str = "payout-link-near-ai-session-changed";
pub const ERR_NEAR_AI_UNAVAILABLE: &str = "payout-link-near-ai-unavailable";
pub const ERR_STATE_UNREADABLE: &str = "payout-link-state-unreadable";

/// What this daemon holds in memory: the org list it last showed (names
/// included; only the linked or pending org's name is written, to
/// `daemon-payout-org-names.json`) and the started requests awaiting a
/// confirm, per rail. Behind `DaemonShared::payout_link`, which also guards
/// `daemon-payout-link.json` and the names file; never held across an
/// await.
#[derive(Default)]
pub(crate) struct PayoutLinkMemory {
    listing: Option<Vec<OwnedOrganization>>,
    pending: BTreeMap<String, Pending>,
}

#[derive(Clone)]
struct Pending {
    action: PayoutLinkAction,
    handle: String,
    organization_id: Option<Uuid>,
    idempotency_key: Uuid,
    expires_at: DateTime<Utc>,
}

/// `daemon-payout-link.json`: per rail, what the last link confirmed from
/// this device was for. Never a name.
#[derive(Default, Serialize, Deserialize)]
struct LocalState {
    #[serde(default)]
    rails: BTreeMap<String, Linked>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Linked {
    idempotency_key: Uuid,
    organization_id: Uuid,
}

fn memory(shared: &DaemonShared) -> MutexGuard<'_, PayoutLinkMemory> {
    shared
        .payout_link
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn load_state(shared: &DaemonShared) -> Result<LocalState, ()> {
    match shared.store.read_daemon_file(DAEMON_PAYOUT_LINK_FILE) {
        Ok(None) => Ok(LocalState::default()),
        Ok(Some(bytes)) => serde_json::from_slice(&bytes).map_err(|_| ()),
        Err(_) => Err(()),
    }
}

fn save_state(shared: &DaemonShared, state: &LocalState) -> Result<(), ()> {
    let bytes = serde_json::to_vec(state).map_err(|_| ())?;
    shared
        .store
        .write_daemon_file(DAEMON_PAYOUT_LINK_FILE, &bytes)
        .map_err(|_| ())
}

/// `daemon-payout-org-names.json`: org id to the name last read from NEAR
/// AI, for the orgs a link from this device names. Display only.
#[derive(Default, PartialEq, Serialize, Deserialize)]
struct OrgNames {
    #[serde(default)]
    names: BTreeMap<Uuid, String>,
}

/// The saved names. Unreadable reads as none: a name is display only.
fn load_names(store: &ConfigStore) -> OrgNames {
    store
        .read_daemon_file(DAEMON_PAYOUT_ORG_NAMES_FILE)
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Keep the names of the orgs a link from this device names -- linked here,
/// or pending a confirm -- taking each from `listing` when it has it (a
/// rename lands here) and otherwise keeping the saved one. Every other id is
/// dropped, and the file goes when none is left. Best effort: a failure
/// costs only the name after a restart. The caller holds the
/// `payout_link` mutex.
fn remember_names(
    shared: &DaemonShared,
    memory: &PayoutLinkMemory,
    listing: Option<&[OwnedOrganization]>,
) {
    let saved = load_names(&shared.store);
    let mut ids: Vec<Uuid> = load_state(shared)
        .map(|state| {
            state
                .rails
                .values()
                .map(|linked| linked.organization_id)
                .collect()
        })
        .unwrap_or_default();
    ids.extend(
        memory
            .pending
            .values()
            .filter_map(|pending| pending.organization_id),
    );
    let mut kept = OrgNames::default();
    for id in ids {
        let fresh = listing
            .and_then(|orgs| orgs.iter().find(|org| org.id == id))
            .map(|org| org.name.clone());
        if let Some(name) = fresh.or_else(|| saved.names.get(&id).cloned()) {
            kept.names.insert(id, name);
        }
    }
    if kept == saved {
        return;
    }
    let written = if kept.names.is_empty() {
        shared
            .store
            .remove_daemon_file(DAEMON_PAYOUT_ORG_NAMES_FILE)
            .is_ok()
    } else {
        serde_json::to_vec(&kept).is_ok_and(|bytes| {
            shared
                .store
                .write_daemon_file(DAEMON_PAYOUT_ORG_NAMES_FILE, &bytes)
                .is_ok()
        })
    };
    if !written {
        tracing::warn!(
            target: "payout_link",
            "payout_link_org_name_not_saved"
        );
    }
}

/// Forget the saved org names with the NEAR AI session they were read
/// with. Called beside the other NEAR AI records' clear; the caller holds
/// the NEAR AI commit lock.
pub(crate) fn clear_org_names_locked(store: &ConfigStore) {
    if store
        .remove_daemon_file(DAEMON_PAYOUT_ORG_NAMES_FILE)
        .is_err()
    {
        tracing::warn!(target: "payout_link", "payout_link_org_names_not_cleared");
    }
}

/// Drop the org list this process holds, names included, when the NEAR AI
/// session it was read with is forgotten.
pub(crate) fn forget_listing(shared: &DaemonShared) {
    memory(shared).listing = None;
}

fn audit_row(shared: &DaemonShared, action: &str) {
    // Best effort, like every other label-only row: a full disk must not
    // turn a change the commons already made into a reported failure.
    let _ = audit::append(
        &shared.store,
        &AuditEntry {
            at: Utc::now(),
            action: action.to_string(),
            project_label: None,
            detail: None,
        },
    );
}

/// What was shown: every owned active org, id and name, in id order and
/// length-prefixed, under its own domain. A rename, an org gained or an org
/// lost changes it.
fn orgs_revision(orgs: &[OwnedOrganization]) -> String {
    let mut sorted: Vec<&OwnedOrganization> = orgs.iter().collect();
    sorted.sort_by_key(|org| org.id);
    let mut hash = Sha256::new();
    hash.update(b"trace-commons-payout-orgs-v1");
    hash.update((sorted.len() as u64).to_be_bytes());
    for org in sorted {
        let id = org.id.to_string();
        for field in [id.as_str(), org.name.as_str()] {
            hash.update((field.len() as u64).to_be_bytes());
            hash.update(field.as_bytes());
        }
    }
    format!("{:x}", hash.finalize())
}

fn near_ai_label(error: &anyhow::Error) -> &'static str {
    match error.to_string().as_str() {
        "near_ai_credential_session_missing" => ERR_NEAR_AI_SIGN_IN_REQUIRED,
        "near_ai_credential_session_expired" => ERR_NEAR_AI_SESSION_EXPIRED,
        "near_ai_credential_session_changed" => ERR_NEAR_AI_SESSION_CHANGED,
        _ => ERR_NEAR_AI_UNAVAILABLE,
    }
}

/// The retained NEAR AI session, read under the credential commit lock and
/// released before any await.
fn retained_near_ai_session(shared: &DaemonShared) -> anyhow::Result<NearAiSession> {
    let locks = session::coordination(shared.store.dir())?;
    let commit = locks
        .commit
        .lock()
        .map_err(|_| anyhow::anyhow!("near_ai_credential_unavailable"))?;
    let settings = session::runtime_snapshot(shared, &commit)?;
    settings
        .near_ai_session
        .ok_or_else(|| anyhow::anyhow!("near_ai_credential_session_missing"))
}

/// Exchange the retained session for a fresh access token and read the
/// owned active orgs with it. The token is returned to the caller, who
/// either sends it once to the commons or drops it.
async fn owned_orgs_with_fresh_token(
    shared: &DaemonShared,
    api: &CloudApi,
) -> Result<(Vec<OwnedOrganization>, String), &'static str> {
    super::nearai_credential::balance::with_fresh_management_session(
        shared,
        api,
        |tokens, retained| async move {
            // The coordinator refreshed while holding the same lock as balance
            // and spending-org reads. Bind this response to that exact login;
            // forgetting or replacing it during the HTTP read must not publish
            // the prior account's organizations.
            let orgs = api.owned_organizations(&tokens).await?;
            if retained_near_ai_session(shared)? != retained {
                anyhow::bail!("near_ai_credential_session_changed");
            }
            Ok((orgs, tokens.access_token))
        },
    )
    .await
    .map_err(|error| near_ai_label(&error))
}

/// The account session and enrolled config every method needs, or the
/// refusal. The session is checked first, so a shell that has not signed in
/// is routed to sign-in whatever else is missing.
///
/// An expired session on a NEAR AI-provisioned account is renewed through
/// `via` first ([`super::account_session_renewal`]); `None` renews nothing.
async fn session_and_config(
    shared: &DaemonShared,
    req: &Request,
    via: Option<&RenewVia<'_>>,
) -> Result<(LoadedAccountSession, ContributorConfig), Box<Response>> {
    let session = match account_session(shared, via).await {
        Ok(Some(session)) => session,
        Ok(None) => {
            return Err(Box::new(Response::err(
                req.id,
                ERR_UNAVAILABLE,
                ERR_ACCOUNT_SESSION_REQUIRED,
            )));
        }
        Err(_) => {
            return Err(Box::new(Response::err(
                req.id,
                ERR_UNAVAILABLE,
                "commons_credential_storage_unavailable",
            )));
        }
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Err(Box::new(Response::err(
            req.id,
            ERR_UNAVAILABLE,
            "not-logged-in",
        )));
    };
    Ok((session, cfg))
}

fn endpoint<'a>(cfg: &'a ContributorConfig, session: &'a LoadedAccountSession) -> Endpoint<'a> {
    Endpoint {
        ingest_url: &cfg.ingest_url,
        allowed_hosts: cfg.allowed_hosts.as_deref(),
        account_session_token: &session.session.access_token,
    }
}

fn refused(req: &Request, error: PayoutLinkError) -> Response {
    Response::err(req.id, ERR_UNAVAILABLE, error.label())
}

fn finish(req: &Request, mut value: serde_json::Value, credential_persisted: bool) -> Response {
    if !credential_persisted {
        super::public_run::attach_credential_warning(&mut value);
    }
    Response::ok(req.id, value)
}

fn bad(req: &Request, name: &'static str) -> Box<Response> {
    Box::new(Response::err(req.id, ERR_BAD_PARAMS, name))
}

fn param_str<'a>(req: &'a Request, name: &'static str) -> Result<&'a str, Box<Response>> {
    req.params
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| bad(req, name))
}

fn param_rail(req: &Request) -> Result<String, Box<Response>> {
    let rail_id = param_str(req, "rail_id")?;
    if !client::valid_rail_id(rail_id) {
        return Err(bad(req, "rail_id"));
    }
    Ok(rail_id.to_owned())
}

fn param_org(req: &Request) -> Result<Uuid, Box<Response>> {
    param_str(req, "organization_id")?
        .parse::<Uuid>()
        .ok()
        .filter(|id| !id.is_nil())
        .ok_or_else(|| bad(req, "organization_id"))
}

/// Required: the `version` the app showed for this rail. Never defaulted,
/// or the commons' "changed since shown" check would be vacuous.
fn param_version(req: &Request) -> Result<i64, Box<Response>> {
    req.params
        .get("expected_current_version")
        .and_then(serde_json::Value::as_i64)
        .filter(|version| *version >= 0)
        .ok_or_else(|| bad(req, "expected_current_version"))
}

fn param_action(req: &Request) -> Result<PayoutLinkAction, Box<Response>> {
    match param_str(req, "action")? {
        "link" => Ok(PayoutLinkAction::Link),
        "unlink" => Ok(PayoutLinkAction::Unlink),
        _ => Err(bad(req, "action")),
    }
}

fn action_label(action: PayoutLinkAction) -> &'static str {
    match action {
        PayoutLinkAction::Link => "link",
        PayoutLinkAction::Unlink => "unlink",
    }
}

/// Whether `req.params` carries a top-level `"confirmed": true`: the
/// person's click on the consent, which names the org shown.
fn confirmed(req: &Request) -> bool {
    req.params
        .get("confirmed")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

/// Run a short local operation only while the payout-link generation still
/// matches the one read before an asynchronous operation. The caller takes
/// the credential commit lock first when writing credential-owned state.
pub(crate) fn with_near_ai_recipient_ref<R>(
    shared: &DaemonShared,
    expected: Option<&str>,
    operation: impl FnOnce() -> R,
) -> Option<R> {
    let _guard = memory(shared);
    let state = load_state(shared).ok()?;
    let current = state
        .rails
        .get(NEAR_AI_PAYOUT_RAIL_ID)
        .map(|linked| near_ai_recipient_ref(linked.idempotency_key, linked.organization_id));
    (current.as_deref() == expected).then(operation)
}

pub(crate) fn near_ai_recipient_ref_snapshot(shared: &DaemonShared) -> Result<Option<String>, ()> {
    let _guard = memory(shared);
    Ok(load_state(shared)?
        .rails
        .get(NEAR_AI_PAYOUT_RAIL_ID)
        .map(|linked| near_ai_recipient_ref(linked.idempotency_key, linked.organization_id)))
}

/// The payout recipient this device linked, only while a fresh account read
/// confirms that exact recipient. A spending key is never a payout identity.
/// Links made on another device remain unknown here instead of being guessed.
pub(crate) async fn verified_near_ai_recipient(
    shared: &DaemonShared,
    api: &CloudApi,
) -> Result<Option<String>, ()> {
    let candidate = {
        let _guard = memory(shared);
        load_state(shared)?
            .rails
            .get(NEAR_AI_PAYOUT_RAIL_ID)
            .cloned()
    };
    let Some(candidate) = candidate else {
        return Ok(None);
    };
    let req = Request {
        id: 0,
        method: "payout_rails_status".into(),
        params: serde_json::json!({}),
    };
    let via = RenewVia::live(api);
    let (mut session, cfg) = session_and_config(shared, &req, Some(&via))
        .await
        .map_err(|_| ())?;
    let call = client::status(&endpoint(&cfg, &session)).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let status = call.result.map_err(|_| ())?;
    if !persisted {
        return Err(());
    }
    let Some(rail) = status
        .rails
        .iter()
        .find(|rail| rail.rail_id == NEAR_AI_PAYOUT_RAIL_ID)
    else {
        return Ok(None);
    };
    if !rail.linked || rail.recipient_unavailable {
        return Ok(None);
    }
    if rail.recipient_ref.as_deref()
        != Some(
            near_ai_recipient_ref(candidate.idempotency_key, candidate.organization_id).as_str(),
        )
    {
        return Err(());
    }
    // A link/unlink committed while the status read ran invalidates this
    // answer, even when the server read happened to match the earlier one.
    let _guard = memory(shared);
    let unchanged = load_state(shared)?
        .rails
        .get(NEAR_AI_PAYOUT_RAIL_ID)
        .is_some_and(|current| {
            current.idempotency_key == candidate.idempotency_key
                && current.organization_id == candidate.organization_id
        });
    if !unchanged {
        return Err(());
    }
    Ok(Some(candidate.organization_id.to_string()))
}

/// `payout_rails_status`, against the live NEAR AI service.
pub(super) async fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    // A status read: an expired session is renewed in the background and
    // waited for briefly, never for the whole ceremony
    // (`account_session_renewal`, "Never waited for by a read").
    // `status` then reads whatever session that left, renewing nothing.
    let _ = super::account_session_renewal::account_session_for_read(shared).await;
    status(shared, req, None).await
}

/// `payout_rails_status`.
pub(super) async fn status(
    shared: &DaemonShared,
    req: &Request,
    via: Option<&RenewVia<'_>>,
) -> Response {
    let (mut session, cfg) = match session_and_config(shared, req, via).await {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let call = client::status(&endpoint(&cfg, &session)).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let status = match call.result {
        Ok(status) => status,
        Err(error) => return refused(req, error),
    };
    let memory = memory(shared);
    let Ok(local) = load_state(shared) else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_UNREADABLE);
    };
    let saved_names = load_names(&shared.store);
    let now = Utc::now();
    let rails: Vec<serde_json::Value> = status
        .rails
        .iter()
        .map(|rail| {
            let linked_here = local.rails.get(&rail.rail_id);
            // `match` only when this device made the link the commons
            // reports; a ref this device cannot recompute is `unknown`.
            let check = match (&rail.recipient_ref, linked_here) {
                (None, _) => None,
                (Some(reference), Some(linked)) if rail.rail_id == NEAR_AI_PAYOUT_RAIL_ID => Some(
                    if *reference
                        == near_ai_recipient_ref(linked.idempotency_key, linked.organization_id)
                    {
                        "match"
                    } else {
                        "mismatch"
                    },
                ),
                (Some(_), _) => Some("unknown"),
            };
            let name_of = |id: Uuid| {
                memory
                    .listing
                    .as_ref()
                    .and_then(|orgs| orgs.iter().find(|org| org.id == id))
                    .map(|org| org.name.clone())
                    // After a restart nothing is listed; the saved name.
                    .or_else(|| saved_names.names.get(&id).cloned())
            };
            let organization_id = (check == Some("match"))
                .then(|| linked_here.map(|linked| linked.organization_id))
                .flatten();
            let organization_name = (check == Some("match"))
                .then(|| linked_here.and_then(|linked| name_of(linked.organization_id)))
                .flatten();
            let pending_here = memory
                .pending
                .get(&rail.rail_id)
                .filter(|pending| pending.expires_at > now)
                .map(|pending| {
                    serde_json::json!({
                        "action": action_label(pending.action),
                        "organization_name": pending.organization_id.and_then(name_of),
                        "expires_at": pending.expires_at,
                    })
                });
            let mut shown = serde_json::json!({
                "rail_id": rail.rail_id,
                "mode": rail.mode,
                "linked": rail.linked,
                "state": rail.state,
                "version": rail.version,
                "request_state": rail.request_state,
                "unlink_requested": rail.unlink_requested,
                "recipient_unavailable": rail.recipient_unavailable,
                "recipient_ref_check": check,
                "organization_id": organization_id,
                "organization_name": organization_name,
                "pending_here": pending_here,
            });
            // Own credit and referral credit apart, only while the rail runs
            // a referral stream: an answer without streams keeps the
            // pre-referral shape.
            if !rail.streams.is_empty() {
                shown["streams"] = serde_json::json!(rail.streams);
            }
            shown
        })
        .collect();
    drop(memory);
    finish(req, serde_json::json!({ "rails": rails }), persisted)
}

/// `payout_link_orgs`, against the live NEAR AI service.
pub(super) async fn handle_orgs(shared: &DaemonShared, req: &Request) -> Response {
    match CloudApi::live() {
        Ok(api) => orgs(shared, req, &api).await,
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, ERR_NEAR_AI_UNAVAILABLE),
    }
}

/// `payout_link_start`, against the live NEAR AI service.
pub(super) async fn handle_start(shared: &DaemonShared, req: &Request) -> Response {
    match CloudApi::live() {
        Ok(api) => start(shared, req, &api).await,
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, ERR_NEAR_AI_UNAVAILABLE),
    }
}

/// List the orgs the person may choose. Nothing is preselected, and the
/// commons is not asked anything.
pub(super) async fn orgs(shared: &DaemonShared, req: &Request, api: &CloudApi) -> Response {
    if let Err(response) = session_and_config(shared, req, Some(&RenewVia::live(api))).await {
        return *response;
    }
    let (orgs, token) = match owned_orgs_with_fresh_token(shared, api).await {
        Ok(found) => found,
        Err(label) => return Response::err(req.id, ERR_UNAVAILABLE, label),
    };
    // Spent on the read; this token goes nowhere else.
    drop(token);
    let revision = orgs_revision(&orgs);
    let shown: Vec<serde_json::Value> = orgs
        .iter()
        .map(|org| serde_json::json!({ "id": org.id, "name": org.name }))
        .collect();
    {
        let mut memory = memory(shared);
        remember_names(shared, &memory, Some(&orgs));
        memory.listing = Some(orgs);
    }
    Response::ok(
        req.id,
        serde_json::json!({ "organizations": shown, "orgs_revision": revision }),
    )
}

/// Start linking the chosen org: re-read the list with a fresh token, refuse
/// if it is not what was shown, then send the org id and that token to the
/// commons in one request and drop the token.
pub(super) async fn start(shared: &DaemonShared, req: &Request, api: &CloudApi) -> Response {
    let parsed = (|| {
        let rail_id = param_rail(req)?;
        if rail_id != NEAR_AI_PAYOUT_RAIL_ID {
            return Err(bad(req, "rail_id"));
        }
        let organization_id = param_org(req)?;
        let shown = param_str(req, "orgs_revision")?;
        if shown.len() != 64 || !client::valid_handle(shown) {
            return Err(bad(req, "orgs_revision"));
        }
        Ok((organization_id, shown.to_owned(), param_version(req)?))
    })();
    let (organization_id, shown_revision, version) = match parsed {
        Ok(parsed) => parsed,
        Err(response) => return *response,
    };
    let (mut session, cfg) = match session_and_config(shared, req, Some(&RenewVia::live(api))).await
    {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let (orgs, token) = match owned_orgs_with_fresh_token(shared, api).await {
        Ok(found) => found,
        Err(label) => return Response::err(req.id, ERR_UNAVAILABLE, label),
    };
    if orgs_revision(&orgs) != shown_revision {
        drop(token);
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_ORGS_CHANGED);
    }
    let Some(chosen) = orgs.iter().find(|org| org.id == organization_id).cloned() else {
        // The list matches what was shown, and this id was not in it.
        drop(token);
        return *bad(req, "organization_id");
    };
    let idempotency_key = Uuid::new_v4();
    let call = client::start_near_ai_link(
        &endpoint(&cfg, &session),
        organization_id,
        token,
        idempotency_key,
        Some(version),
    )
    .await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let started = match call.result {
        Ok(started) => started,
        Err(error) => return refused(req, error),
    };
    {
        let mut memory = memory(shared);
        memory.listing = Some(orgs);
        memory.pending.insert(
            NEAR_AI_PAYOUT_RAIL_ID.to_owned(),
            Pending {
                action: PayoutLinkAction::Link,
                handle: started.handle,
                organization_id: Some(organization_id),
                idempotency_key,
                expires_at: started.expires_at,
            },
        );
        remember_names(shared, &memory, memory.listing.as_deref());
    }
    audit_row(shared, AUDIT_LINK_STARTED);
    finish(
        req,
        serde_json::json!({
            "outcome": PayoutLinkOutcome::PendingConfirmation.as_str(),
            "rail_id": NEAR_AI_PAYOUT_RAIL_ID,
            "action": "link",
            "organization_id": organization_id,
            "organization_name": chosen.name,
            "expires_at": started.expires_at,
        }),
        persisted,
    )
}

/// `payout_link_unlink`: start stopping payouts on a rail. Carries no proof,
/// asks NEAR AI nothing, and changes nothing until confirmed.
pub(super) async fn handle_unlink(shared: &DaemonShared, req: &Request) -> Response {
    let parsed = (|| Ok::<_, Box<Response>>((param_rail(req)?, param_version(req)?)))();
    let (rail_id, version) = match parsed {
        Ok(parsed) => parsed,
        Err(response) => return *response,
    };
    let api = CloudApi::live().ok();
    let via = api.as_ref().map(RenewVia::live);
    let (mut session, cfg) = match session_and_config(shared, req, via.as_ref()).await {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let idempotency_key = Uuid::new_v4();
    let call = client::start_unlink(
        &endpoint(&cfg, &session),
        &rail_id,
        idempotency_key,
        Some(version),
    )
    .await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let started = match call.result {
        Ok(started) => started,
        Err(error) => return refused(req, error),
    };
    memory(shared).pending.insert(
        rail_id.clone(),
        Pending {
            action: PayoutLinkAction::Unlink,
            handle: started.handle,
            organization_id: None,
            idempotency_key,
            expires_at: started.expires_at,
        },
    );
    audit_row(shared, AUDIT_UNLINK_STARTED);
    finish(
        req,
        serde_json::json!({
            "outcome": PayoutLinkOutcome::PendingConfirmation.as_str(),
            "rail_id": rail_id,
            "action": "unlink",
            "expires_at": started.expires_at,
        }),
        persisted,
    )
}

/// `payout_link_confirm`: the person's click on the consent. Must name the
/// action and org the started request was for, and carry `confirmed: true`.
pub(super) async fn handle_confirm(shared: &DaemonShared, req: &Request) -> Response {
    let parsed = (|| {
        let rail_id = param_rail(req)?;
        let action = param_action(req)?;
        let organization_id = match action {
            PayoutLinkAction::Link => Some(param_org(req)?),
            PayoutLinkAction::Unlink => None,
        };
        Ok::<_, Box<Response>>((rail_id, action, organization_id))
    })();
    let (rail_id, action, organization_id) = match parsed {
        Ok(parsed) => parsed,
        Err(response) => return *response,
    };
    let api = CloudApi::live().ok();
    let via = api.as_ref().map(RenewVia::live);
    let (mut session, cfg) = match session_and_config(shared, req, via.as_ref()).await {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    if !confirmed(req) {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_CONFIRMATION_REQUIRED);
    }
    let pending = {
        let mut memory = memory(shared);
        let Some(pending) = memory.pending.get(&rail_id).cloned() else {
            return Response::err(req.id, ERR_UNAVAILABLE, ERR_START_REQUIRED);
        };
        if pending.action != action || pending.organization_id != organization_id {
            return Response::err(req.id, ERR_UNAVAILABLE, ERR_CHANGED);
        }
        if pending.expires_at <= Utc::now() {
            memory.pending.remove(&rail_id);
            return refused(req, PayoutLinkError::Refused(PayoutLinkOutcome::Expired));
        }
        pending
    };
    let call = client::confirm(&endpoint(&cfg, &session), &rail_id, &pending.handle, action).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);

    let mut memory = memory(shared);
    let still_ours = memory
        .pending
        .get(&rail_id)
        .is_some_and(|current| current.handle == pending.handle);
    let confirmed = match call.result {
        Ok(confirmed) => confirmed,
        // Nothing was decided: the request may still be confirmed.
        Err(error @ (PayoutLinkError::Unavailable | PayoutLinkError::SessionInvalid)) => {
            return refused(req, error);
        }
        // The commons consumed or refused it; it cannot be confirmed again.
        Err(error) => {
            if still_ours {
                memory.pending.remove(&rail_id);
            }
            return refused(req, error);
        }
    };
    if still_ours {
        memory.pending.remove(&rail_id);
    }
    // What this device remembers about the link, for the ref check. A
    // failure here leaves the check `unknown`; the link itself is saved.
    let recorded = load_state(shared)
        .and_then(|mut state| {
            match (action, pending.organization_id) {
                (PayoutLinkAction::Link, Some(organization_id)) => {
                    state.rails.insert(
                        rail_id.clone(),
                        Linked {
                            idempotency_key: pending.idempotency_key,
                            organization_id,
                        },
                    );
                }
                _ => {
                    state.rails.remove(&rail_id);
                }
            }
            save_state(shared, &state)
        })
        .is_ok();
    remember_names(shared, &memory, memory.listing.as_deref());
    drop(memory);
    audit_row(
        shared,
        match action {
            PayoutLinkAction::Link => AUDIT_LINK_CONFIRMED,
            PayoutLinkAction::Unlink => AUDIT_UNLINK_CONFIRMED,
        },
    );
    finish(
        req,
        serde_json::json!({
            "outcome": PayoutLinkOutcome::Confirmed.as_str(),
            "rail_id": rail_id,
            "action": action_label(action),
            "version": confirmed.version,
            "recorded_here": recorded,
        }),
        persisted,
    )
}

#[cfg(test)]
#[path = "payout_link_tests.rs"]
mod tests;
