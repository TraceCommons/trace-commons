//! IPC handlers for connecting inference, the route by which an invited
//! contributor receives a redaction witness (K12 of the connect-and-forget
//! consent design; server half #1019).
//!
//! Five methods, each an explicit contributor action or a read:
//!
//! - `inference_connection_offers` -- the operator's offers, identifiers and
//!   digests only;
//! - `inference_connection_current` -- the account's selection, whether this
//!   device has installed it, and whether a revocation was just applied here;
//! - `inference_connection_select` -- select the exact offer revision, config
//!   digest and disclosure version the shell showed. Installs nothing: the
//!   witness material the server returns is checked against the chosen digest
//!   and held for this device only, until the contributor also confirms
//!   installation;
//! - `inference_connection_install` -- the separate, explicit step that writes
//!   `ContributorConfig.witness` (and the receipt endpoint, when the selection
//!   carries one). Only a selection made on this device can be installed here;
//! - `inference_connection_disconnect` -- remove from this device what the
//!   installation wrote, then revoke the selection on the server.
//!
//! # One installed device per account
//!
//! The server keeps one live connection per account, every select revokes it
//! and mints a new `connection_id`, and the witness material is returned only
//! in the select response -- the current-selection route carries identifiers
//! and digests, never a URL or pin. So a device can install only what it
//! selected itself, and **at most one device per account holds an installed
//! witness**. Selecting on a second device replaces the first device's
//! connection: the first device's next `inference_connection_current` reports
//! `revocation_applied: true`, its witness is removed, and -- since that is a
//! witness change -- its armed grants are voided. Selecting on this device
//! removes this device's own previously installed witness at once, since the
//! server has just revoked it.
//!
//! # What this does not do
//!
//! A selection grants no folder, trace, raw-session or standing contribution
//! consent, and nothing here touches consent scopes or project modes. There
//! is no server-pushed enablement: the witness is written only by
//! `inference_connection_install`, on a contributor's confirmation of the
//! connection it names.
//!
//! Installing changes the witness, which is a change of recipient under R6
//! (`daemon::grant_terms`): the watcher's next grant sweep voids every armed
//! `auto_upload` grant made under the old terms, and the shells show the void
//! notice. That is intended and is not bypassed here.
//!
//! # A retired revision
//!
//! `connection_reselection_required` is surfaced under the server's own name
//! and installs nothing. The current operator configuration is never
//! substituted for what the contributor selected: the pending selection is
//! discarded and the contributor must choose again. A witness already
//! installed from a revision that has since been retired stays installed
//! until the contributor reselects or disconnects: the connection is still
//! the account's live, unrevoked selection, retirement says only that the
//! operator now offers something else, and removing a working witness
//! unprompted would void the contributor's grants for a change they did not
//! make. An operator who must stop use of a witness revokes the connection,
//! which every device applies.
//!
//! # Concurrency
//!
//! Handlers read the config and `daemon-inference-connection.json` before a
//! network call only to build the request. After it they take
//! `DaemonShared::inference_connection_lock`, re-read both, and apply their
//! change to the fresh copies, acting only on the pending or installed entry
//! they saw before the call -- so a select made while an install or a
//! `current` was in flight is not discarded, and an install whose held
//! selection was replaced meanwhile refuses
//! (`inference-connection-selection-replaced`). The lock is never held across
//! an await.
//!
//! # The account session
//!
//! Every method presents the account session from `crate::account_auth`,
//! never the device key, and answers `account-session-required` when there is
//! no live one -- exactly as withdrawal does. Disconnect is the exception in
//! one respect: it removes the local witness first, whatever the session, and
//! reports the server step's refusal alongside. Rotated sessions the server
//! hands back are kept whatever the outcome.
//!
//! Audit rows (`inference-connection-selected`, `-installed`,
//! `-disconnected`, `-revocation-applied`) are label-only: no URL, token,
//! signing address, pin or connection id.

use std::sync::{MutexGuard, PoisonError};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use trace_commons_protocol::inference_connection::{
    ConnectionWitnessConfig, SelectInferenceConnection, SelectedInferenceConnection,
    valid_identifier,
};

use super::audit::{self, AuditEntry};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::withdraw::{ERR_ACCOUNT_SESSION_REQUIRED, account_session, keep_rotated_token};
use crate::account_auth::LoadedAccountSession;
use crate::config::{
    ContributorConfig, DAEMON_INFERENCE_CONNECTION_FILE, WitnessSettings, allowlist_for,
};
use crate::inference_connection::{self as client, ConnectionError, ConnectionStatus, Endpoint};

pub const AUDIT_SELECTED: &str = "inference-connection-selected";
pub const AUDIT_INSTALLED: &str = "inference-connection-installed";
pub const AUDIT_DISCONNECTED: &str = "inference-connection-disconnected";
pub const AUDIT_REVOCATION_APPLIED: &str = "inference-connection-revocation-applied";

/// `detail` of a revocation-applied row: the installed connection is no
/// longer the account's selection (disconnected, or replaced elsewhere).
pub const REVOCATION_NOT_LIVE: &str = "not-live";
/// `detail`: the account reports the installed connection with a different
/// revision, digest or disclosure than was installed.
pub const REVOCATION_CONFIGURATION_CHANGED: &str = "configuration-changed";
/// `detail`: this device selected again, and the server revoked what it had
/// installed.
pub const REVOCATION_REPLACED_HERE: &str = "replaced-by-selection";

/// No selection made on this device is waiting to be installed.
pub const ERR_SELECT_REQUIRED: &str = "inference-connection-select-required";
/// The confirmation named a different connection or digest than the one held.
pub const ERR_CONFIRMATION_MISMATCH: &str = "inference-connection-confirmation-mismatch";
/// The account's selection is no longer the one held for installation.
pub const ERR_NOT_CURRENT: &str = "inference-connection-not-current";
/// The held selection was replaced by another select while this install was
/// asking the server; the newer one is kept, and nothing is installed.
pub const ERR_SELECTION_REPLACED: &str = "inference-connection-selection-replaced";
/// The selected witness's pins do not parse, or its host is refused.
pub const ERR_WITNESS_REFUSED: &str = "inference-connection-witness-refused";
/// The selected receipt endpoint is refused by this client's endpoint rules.
pub const ERR_RECEIPT_REFUSED: &str = "inference-connection-receipt-endpoint-refused";
pub const ERR_STATE_UNREADABLE: &str = "inference-connection-state-unreadable";
pub const ERR_STATE_WRITE_FAILED: &str = "inference-connection-state-write-failed";

/// What this device holds, beside the config it describes.
#[derive(Default, Serialize, Deserialize)]
struct LocalState {
    /// Selected from this device and not yet installed here.
    #[serde(default)]
    pending: Option<Pending>,
    /// What the last installation wrote, so a disconnect or an observed
    /// revocation removes exactly that and leaves any other local
    /// configuration alone.
    #[serde(default)]
    installed: Option<Installed>,
}

/// A selection awaiting installation, with the offer it was chosen from. The
/// select response carries neither `offer_id` nor `provider_id`, and both are
/// bound into the digests the witness material is checked against.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pending {
    offer_id: String,
    provider_id: String,
    selected: SelectedInferenceConnection,
}

#[derive(Clone, Serialize, Deserialize)]
struct Installed {
    connection_id: Uuid,
    state_version: i64,
    revision: String,
    config_digest: String,
    disclosure_version: String,
    witness: ConnectionWitnessConfig,
    #[serde(default)]
    inference_receipt_endpoint: Option<String>,
}

fn load_state(shared: &DaemonShared) -> Result<LocalState, ()> {
    match shared
        .store
        .read_daemon_file(DAEMON_INFERENCE_CONNECTION_FILE)
    {
        Ok(None) => Ok(LocalState::default()),
        Ok(Some(body)) => serde_json::from_slice(&body).map_err(|_| ()),
        Err(_) => Err(()),
    }
}

fn save_state(shared: &DaemonShared, state: &LocalState) -> Result<(), ()> {
    let body = serde_json::to_vec(state).map_err(|_| ())?;
    shared
        .store
        .write_daemon_file(DAEMON_INFERENCE_CONNECTION_FILE, &body)
        .map_err(|_| ())
}

/// Serializes this module's read-modify-write of the config and state file.
/// Taken only after a handler's network call, never across an await.
fn lock(shared: &DaemonShared) -> MutexGuard<'_, ()> {
    shared
        .inference_connection_lock
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

fn audit_row(shared: &DaemonShared, action: &str, detail: Option<&str>) -> Result<(), ()> {
    audit::append(
        &shared.store,
        &AuditEntry {
            at: Utc::now(),
            action: action.to_string(),
            project_label: None,
            detail: detail.map(str::to_string),
        },
    )
    .map_err(|_| ())
}

/// Whether the config's witness is the one this installation wrote.
fn witness_is(cfg: &ContributorConfig, witness: &ConnectionWitnessConfig) -> bool {
    cfg.witness.as_ref().is_some_and(|current| {
        current.url == witness.url
            && current.signing_address == witness.signing_address
            && current.expected_measurements == witness.expected_measurements
    })
}

/// Remove what `installed` wrote, where the config still holds it. A witness
/// or receipt endpoint configured some other way since is left alone.
fn remove_installed(cfg: &mut ContributorConfig, installed: &Installed) -> bool {
    let mut changed = false;
    if witness_is(cfg, &installed.witness) {
        cfg.clear_witness();
        changed = true;
    }
    if installed.inference_receipt_endpoint.is_some()
        && cfg.inference_receipt_endpoint == installed.inference_receipt_endpoint
    {
        cfg.inference_receipt_endpoint = None;
        changed = true;
    }
    changed
}

/// Whether the account's selection is still exactly what was installed. A
/// retired revision (`reselection_required`) still is: see the module docs.
fn installed_is_live(installed: &Installed, current: &ConnectionStatus) -> bool {
    installed.connection_id == current.connection_id
        && installed.revision == current.revision
        && installed.config_digest == current.config_digest
        && installed.disclosure_version == current.disclosure_version
}

/// The account session and enrolled config every method needs, or the
/// refusal. The session is checked first, so a shell that has not signed in
/// is routed to sign-in whatever else is missing.
fn session_and_config(
    shared: &DaemonShared,
    req: &Request,
) -> Result<(LoadedAccountSession, ContributorConfig), Box<Response>> {
    let session = match account_session(shared) {
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

fn refusal(req: &Request, error: ConnectionError) -> Response {
    Response::err(req.id, ERR_UNAVAILABLE, error.label())
}

fn finish(req: &Request, mut value: serde_json::Value, credential_persisted: bool) -> Response {
    if !credential_persisted {
        super::public_run::attach_credential_warning(&mut value);
    }
    Response::ok(req.id, value)
}

fn required_str<'a>(req: &'a Request, name: &'static str) -> Result<&'a str, Box<Response>> {
    req.params
        .get(name)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Box::new(Response::err(req.id, ERR_BAD_PARAMS, name)))
}

fn parse_uuid(req: &Request, name: &'static str) -> Result<Uuid, Box<Response>> {
    required_str(req, name)?
        .parse()
        .map_err(|_| Box::new(Response::err(req.id, ERR_BAD_PARAMS, name)))
}

/// `inference_connection_offers`. A read; selects nothing.
pub(super) async fn handle_offers(shared: &DaemonShared, req: &Request) -> Response {
    let (mut session, cfg) = match session_and_config(shared, req) {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let call = client::list_offers(&endpoint(&cfg, &session)).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    match call.result {
        Ok(offers) => finish(req, serde_json::json!({ "offers": offers }), persisted),
        Err(error) => refusal(req, error),
    }
}

/// `inference_connection_current`. Reports the account's selection and what
/// this device holds. When the connection this device installed is no longer
/// exactly the account's live selection -- disconnected or replaced
/// elsewhere, or reported with another revision, digest or disclosure -- that
/// revocation is applied here: what the installation wrote is removed, since
/// disconnect stops new use once a cooperating client observes it. A retired
/// revision (`reselection_required`) is reported and changes nothing.
pub(super) async fn handle_current(shared: &DaemonShared, req: &Request) -> Response {
    let (mut session, cfg) = match session_and_config(shared, req) {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    // What this device held before asking. Only these entries are judged
    // against the answer: anything selected or installed while the request
    // was in flight is newer than the answer and is left alone.
    let Ok(before) = load_state(shared) else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_UNREADABLE);
    };
    let call = client::current_selection(&endpoint(&cfg, &session)).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let selection = match call.result {
        Ok(selection) => selection,
        Err(error) => return refusal(req, error),
    };

    let _guard = lock(shared);
    let Ok(mut state) = load_state(shared) else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_UNREADABLE);
    };
    let Ok(Some(mut cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    let seen_installed = before.installed.as_ref().map(|i| i.connection_id);
    let mut revocation_applied = false;
    if let Some(installed) = state
        .installed
        .clone()
        .filter(|i| Some(i.connection_id) == seen_installed)
        .filter(|i| {
            !selection
                .as_ref()
                .is_some_and(|current| installed_is_live(i, current))
        })
    {
        let detail = if selection
            .as_ref()
            .is_some_and(|current| current.connection_id == installed.connection_id)
        {
            REVOCATION_CONFIGURATION_CHANGED
        } else {
            REVOCATION_NOT_LIVE
        };
        if remove_installed(&mut cfg, &installed) {
            // Recorded after the change: removing a witness is the safe
            // direction, so a failed append must not keep it in use.
            if shared.store.save_config(&cfg).is_err() {
                return Response::err(req.id, ERR_UNAVAILABLE, "config-write-failed");
            }
            revocation_applied = true;
            let _ = audit_row(shared, AUDIT_REVOCATION_APPLIED, Some(detail));
        }
        state.installed = None;
    }
    if state.pending.is_some()
        && state.pending == before.pending
        && state
            .pending
            .as_ref()
            .is_some_and(|pending| still_current(selection.as_ref(), &pending.selected).is_some())
    {
        state.pending = None;
    }
    if save_state(shared, &state).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_WRITE_FAILED);
    }
    let installed_here = match (&selection, &state.installed) {
        (Some(selection), Some(installed)) => {
            installed_is_live(installed, selection) && witness_is(&cfg, &installed.witness)
        }
        _ => false,
    };
    let pending_install = state.pending.is_some();
    finish(
        req,
        serde_json::json!({
            "selection": selection,
            "installed_on_this_device": installed_here,
            "pending_install": pending_install,
            "revocation_applied": revocation_applied,
        }),
        persisted,
    )
}

/// `inference_connection_select`. The caller passes exactly what it showed
/// the contributor; the daemon neither fills in nor corrects any of it.
pub(super) async fn handle_select(shared: &DaemonShared, req: &Request) -> Response {
    let (request, provider_id) = match select_request(req) {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let (mut session, cfg) = match session_and_config(shared, req) {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    if load_state(shared).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_UNREADABLE);
    }
    let call = client::select(&endpoint(&cfg, &session), &request, &provider_id).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let selected = match call.result {
        Ok(selected) => selected,
        Err(error) => {
            if error == ConnectionError::ReselectionRequired {
                // Nothing from a retired revision may be installed.
                let _guard = lock(shared);
                if let Ok(mut state) = load_state(shared) {
                    state.pending = None;
                    let _ = save_state(shared, &state);
                }
            }
            return refusal(req, error);
        }
    };
    if audit_row(shared, AUDIT_SELECTED, Some(&request.offer_id)).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, "audit-write-failed");
    }

    let _guard = lock(shared);
    let Ok(mut state) = load_state(shared) else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_UNREADABLE);
    };
    // The server revoked whatever this account had selected before, so a
    // witness this device installed from it stops being used now, not at the
    // next install or `current`.
    let mut previous_removed = false;
    if let Some(installed) = state
        .installed
        .clone()
        .filter(|i| i.connection_id != selected.connection_id)
    {
        let Ok(Some(mut cfg)) = shared.store.load_config() else {
            return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
        };
        if remove_installed(&mut cfg, &installed) {
            if shared.store.save_config(&cfg).is_err() {
                return Response::err(req.id, ERR_UNAVAILABLE, "config-write-failed");
            }
            previous_removed = true;
            let _ = audit_row(
                shared,
                AUDIT_REVOCATION_APPLIED,
                Some(REVOCATION_REPLACED_HERE),
            );
        }
        state.installed = None;
    }
    let receipt_offered = selected.inference_receipt_endpoint.is_some();
    let value = serde_json::json!({
        "selected": true,
        "connection_id": selected.connection_id,
        "state_version": selected.state_version,
        "offer_id": request.offer_id,
        "revision": selected.revision,
        "config_digest": selected.config_digest,
        "disclosure_version": selected.disclosure_version,
        "receipt_endpoint_offered": receipt_offered,
        "install_required": true,
        "previous_witness_removed": previous_removed,
    });
    state.pending = Some(Pending {
        offer_id: request.offer_id,
        provider_id,
        selected,
    });
    if save_state(shared, &state).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_WRITE_FAILED);
    }
    finish(req, value, persisted)
}

fn select_request(req: &Request) -> Result<(SelectInferenceConnection, String), Box<Response>> {
    let expected_current_version = match req.params.get("expected_current_version") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(value.as_i64().ok_or_else(|| {
            Box::new(Response::err(
                req.id,
                ERR_BAD_PARAMS,
                "expected_current_version",
            ))
        })?),
    };
    let idempotency_key = match req.params.get("idempotency_key") {
        None | Some(serde_json::Value::Null) => Uuid::new_v4(),
        Some(_) => parse_uuid(req, "idempotency_key")?,
    };
    let provider_id = required_str(req, "provider_id")?.to_string();
    if !valid_identifier(&provider_id) {
        return Err(Box::new(Response::err(
            req.id,
            ERR_BAD_PARAMS,
            "provider_id_invalid",
        )));
    }
    let request = SelectInferenceConnection {
        offer_id: required_str(req, "offer_id")?.to_string(),
        revision: required_str(req, "revision")?.to_string(),
        config_digest: required_str(req, "config_digest")?.to_string(),
        disclosure_version: required_str(req, "disclosure_version")?.to_string(),
        idempotency_key,
        expected_current_version,
    };
    request
        .validate()
        .map_err(|label| Box::new(Response::err(req.id, ERR_BAD_PARAMS, label)))?;
    Ok((request, provider_id))
}

/// `inference_connection_install`: the contributor's confirmation, on this
/// device, of the connection it names. Re-checks the held witness material
/// against its digest, then re-reads the account's selection, so a
/// revocation or retirement since the select is observed and nothing is
/// installed from it.
pub(super) async fn handle_install(shared: &DaemonShared, req: &Request) -> Response {
    let connection_id = match parse_uuid(req, "connection_id") {
        Ok(id) => id,
        Err(response) => return *response,
    };
    let config_digest = match required_str(req, "config_digest") {
        Ok(digest) => digest.to_string(),
        Err(response) => return *response,
    };
    let (mut session, cfg) = match session_and_config(shared, req) {
        Ok(pair) => pair,
        Err(response) => return *response,
    };
    let Ok(state) = load_state(shared) else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_UNREADABLE);
    };
    let Some(pending) = state.pending.clone() else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_SELECT_REQUIRED);
    };
    if pending.selected.connection_id != connection_id
        || pending.selected.config_digest != config_digest
    {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_CONFIRMATION_MISMATCH);
    }
    // The held material was checked at select; it has since sat in a file.
    if let Err(error) =
        client::verify_selected(&pending.selected, &pending.offer_id, &pending.provider_id)
    {
        let _guard = lock(shared);
        if let Ok(mut fresh) = load_state(shared)
            && fresh.pending.as_ref() == Some(&pending)
        {
            fresh.pending = None;
            let _ = save_state(shared, &fresh);
        }
        return refusal(req, error);
    }
    let call = client::current_selection(&endpoint(&cfg, &session)).await;
    let persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    let current = match call.result {
        Ok(current) => current,
        Err(error) => return refusal(req, error),
    };

    let _guard = lock(shared);
    let Ok(mut state) = load_state(shared) else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_UNREADABLE);
    };
    if state.pending.as_ref() != Some(&pending) {
        // Selected again (or disconnected) while this install was asking the
        // server. The answer is about a connection that is no longer the one
        // held here; the newer state stands and nothing is installed.
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_SELECTION_REPLACED);
    }
    if let Some(label) = still_current(current.as_ref(), &pending.selected) {
        state.pending = None;
        let _ = save_state(shared, &state);
        return Response::err(req.id, ERR_UNAVAILABLE, label);
    }
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    let mut next = cfg.clone();
    if let Some(previous) = &state.installed {
        remove_installed(&mut next, previous);
    }
    let selected = &pending.selected;
    if let Err(label) = apply(&mut next, &cfg, selected) {
        return Response::err(req.id, ERR_UNAVAILABLE, label);
    }
    let receipt_installed = selected.inference_receipt_endpoint.is_some();
    // Recorded first, then written: a witness change with no record of it is
    // the outcome this row exists to prevent.
    let detail = if receipt_installed {
        "witness,receipt-endpoint"
    } else {
        "witness"
    };
    if audit_row(shared, AUDIT_INSTALLED, Some(detail)).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, "audit-write-failed");
    }
    if shared.store.save_config(&next).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, "config-write-failed");
    }
    state.installed = Some(Installed {
        connection_id: selected.connection_id,
        state_version: selected.state_version,
        revision: selected.revision.clone(),
        config_digest: selected.config_digest.clone(),
        disclosure_version: selected.disclosure_version.clone(),
        witness: selected.witness.clone(),
        inference_receipt_endpoint: selected.inference_receipt_endpoint.clone(),
    });
    state.pending = None;
    if save_state(shared, &state).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_WRITE_FAILED);
    }
    finish(
        req,
        serde_json::json!({
            "installed": true,
            "connection_id": selected.connection_id,
            "state_version": selected.state_version,
            "receipt_endpoint_installed": receipt_installed,
        }),
        persisted,
    )
}

/// `None` when `current` is still exactly the selection held for install,
/// otherwise the refusal label.
fn still_current(
    current: Option<&ConnectionStatus>,
    pending: &SelectedInferenceConnection,
) -> Option<&'static str> {
    let Some(current) = current else {
        return Some(ERR_NOT_CURRENT);
    };
    if current.connection_id != pending.connection_id {
        return Some(ERR_NOT_CURRENT);
    }
    if current.reselection_required
        || current.revision != pending.revision
        || current.config_digest != pending.config_digest
        || current.disclosure_version != pending.disclosure_version
    {
        return Some(ConnectionError::ReselectionRequired.label());
    }
    None
}

/// Write the selected witness, and the receipt endpoint when it carries one,
/// into `next`. Nothing else changes. `admission_evidence` is not part of the
/// selection, so an existing witness's value carries over and a first
/// installation leaves it off.
fn apply(
    next: &mut ContributorConfig,
    original: &ContributorConfig,
    selected: &SelectedInferenceConnection,
) -> Result<(), &'static str> {
    let witness = WitnessSettings {
        admission_evidence: original
            .witness
            .as_ref()
            .is_some_and(|w| w.admission_evidence),
        url: selected.witness.url.clone(),
        signing_address: selected.witness.signing_address.clone(),
        expected_measurements: selected.witness.expected_measurements.clone(),
    };
    if !witness.trust().is_ok_and(|trust| trust.is_pinned()) {
        return Err(ERR_WITNESS_REFUSED);
    }
    // An operator's own allowlist, when set, still governs. Otherwise the
    // witness host is admitted on the basis signup admits one: the commons
    // the contributor enrolled with named it.
    let operator = allowlist_for(original.allowed_hosts.as_deref());
    let witness_url = reqwest::Url::parse(&witness.url).map_err(|_| ERR_WITNESS_REFUSED)?;
    if operator.is_enforcing() && operator.check(&witness_url).is_err() {
        return Err(ERR_WITNESS_REFUSED);
    }
    if let Some(endpoint) = selected.inference_receipt_endpoint.as_deref() {
        let allowed = super::account_onboarding::published_host_allowlist(
            &operator,
            &original.ingest_url,
            endpoint,
        )
        .map_err(|_| ERR_RECEIPT_REFUSED)?;
        crate::config::validate_inference_receipt_endpoint(endpoint, &allowed)
            .map_err(|_| ERR_RECEIPT_REFUSED)?;
        next.inference_receipt_endpoint = Some(endpoint.to_string());
    }
    next.set_witness(witness, crate::config::WitnessOrigin::ConnectedInference);
    Ok(())
}

/// `server_disconnect` values in a disconnect result.
pub const SERVER_REVOKED: &str = "revoked";
/// The server has no such live connection for this account.
pub const SERVER_NOT_FOUND: &str = "not-found";
/// The server step did not happen or did not succeed; `server_refusal` says
/// why, and the shell retries once that is resolved.
pub const SERVER_PENDING: &str = "pending";

/// `inference_connection_disconnect`. Removes from this device what the
/// installation wrote, **then** asks the server to revoke the selection. The
/// local step never depends on the server: with no account session, an
/// unreachable commons or a refusal, the witness is still removed and the
/// result says the server step is pending (`server_disconnect: "pending"`,
/// with `server_refusal` the label). Recalls nothing already disclosed and
/// revokes no provider credential.
pub(super) async fn handle_disconnect(shared: &DaemonShared, req: &Request) -> Response {
    let connection_id = match parse_uuid(req, "connection_id") {
        Ok(id) => id,
        Err(response) => return *response,
    };

    let removed = {
        let _guard = lock(shared);
        let Ok(mut state) = load_state(shared) else {
            return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_UNREADABLE);
        };
        let mut removed = false;
        if let Some(installed) = state
            .installed
            .clone()
            .filter(|i| i.connection_id == connection_id)
        {
            if let Ok(Some(mut cfg)) = shared.store.load_config()
                && remove_installed(&mut cfg, &installed)
            {
                if shared.store.save_config(&cfg).is_err() {
                    return Response::err(req.id, ERR_UNAVAILABLE, "config-write-failed");
                }
                removed = true;
            }
            state.installed = None;
        }
        if state
            .pending
            .as_ref()
            .is_some_and(|p| p.selected.connection_id == connection_id)
        {
            state.pending = None;
        }
        if save_state(shared, &state).is_err() {
            return Response::err(req.id, ERR_UNAVAILABLE, ERR_STATE_WRITE_FAILED);
        }
        removed
    };

    let mut persisted = true;
    let mut state_version = None;
    let (server, server_refusal) = match (account_session(shared), shared.store.load_config()) {
        (Ok(None), _) => (SERVER_PENDING, Some(ERR_ACCOUNT_SESSION_REQUIRED)),
        (Err(_), _) => (
            SERVER_PENDING,
            Some("commons_credential_storage_unavailable"),
        ),
        (Ok(Some(_)), Err(_) | Ok(None)) => (SERVER_PENDING, Some("not-logged-in")),
        (Ok(Some(mut session)), Ok(Some(cfg))) => {
            let call = client::disconnect(&endpoint(&cfg, &session), connection_id).await;
            persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
            match call.result {
                Ok(outcome) => {
                    state_version = Some(outcome.state_version);
                    (SERVER_REVOKED, None)
                }
                Err(ConnectionError::NotFound) => {
                    (SERVER_NOT_FOUND, Some(ConnectionError::NotFound.label()))
                }
                Err(error) => (SERVER_PENDING, Some(error.label())),
            }
        }
    };
    let detail = format!(
        "{},server-{server}",
        if removed {
            "local-witness-removed"
        } else {
            "nothing-installed-here"
        }
    );
    // The local removal has already happened and stands whatever this
    // append does: a failed audit write must not read as a failed stop.
    let _ = audit_row(shared, AUDIT_DISCONNECTED, Some(&detail));
    finish(
        req,
        serde_json::json!({
            "disconnected": server == SERVER_REVOKED,
            "connection_id": connection_id,
            "state_version": state_version,
            "local_witness_removed": removed,
            "server_disconnect": server,
            "server_refusal": server_refusal,
        }),
        persisted,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference_connection::test_support::{self, Server};
    use trace_commons_protocol::inference_connection::DISCLOSURE_VERSION;

    fn shared() -> DaemonShared {
        let (_d, store) = crate::config::tests_support::temp_store();
        std::mem::forget(_d);
        DaemonShared::load(store).unwrap()
    }

    fn req(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 7,
            method: method.to_string(),
            params,
        }
    }

    const SESSION: &str = "tcn1_dGVuYW50.account";

    fn sign_in(s: &DaemonShared) {
        let session = crate::account_auth::AccountSession {
            access_token: SESSION.to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "acct".to_string(),
        };
        s.store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
    }

    fn enrolled(base: &str) -> DaemonShared {
        let s = shared();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = base.to_string();
        s.store.save_config(&cfg).unwrap();
        sign_in(&s);
        s
    }

    /// The params a shell sends for the offer it showed, whose witness
    /// material carries `receipt`.
    fn select_params_with(receipt: Option<&str>) -> serde_json::Value {
        serde_json::json!({
            "offer_id": "near-ai",
            "provider_id": "near-ai",
            "revision": test_support::revision_with(receipt),
            "config_digest": test_support::config_digest_with(receipt),
            "disclosure_version": DISCLOSURE_VERSION,
            "idempotency_key": "6f7d3c1e-6b1a-4c5e-9d2f-2a4b8c9e0f11",
        })
    }

    fn select_params() -> serde_json::Value {
        select_params_with(None)
    }

    async fn select(s: &DaemonShared) -> Response {
        handle_select(s, &req("inference_connection_select", select_params())).await
    }

    async fn install(s: &DaemonShared, server: &Server) -> Response {
        let (id, receipt) = {
            let stub = server.stub.lock().unwrap();
            (stub.connection_id, stub.receipt.clone())
        };
        handle_install(
            s,
            &req(
                "inference_connection_install",
                serde_json::json!({
                    "connection_id": id.to_string(),
                    "config_digest": test_support::config_digest_with(receipt.as_deref()),
                }),
            ),
        )
        .await
    }

    async fn current(s: &DaemonShared) -> serde_json::Value {
        handle_current(
            s,
            &req("inference_connection_current", serde_json::json!({})),
        )
        .await
        .result
        .expect("current answered")
    }

    async fn disconnect(s: &DaemonShared, id: Uuid) -> serde_json::Value {
        handle_disconnect(
            s,
            &req(
                "inference_connection_disconnect",
                serde_json::json!({ "connection_id": id.to_string() }),
            ),
        )
        .await
        .result
        .expect("disconnect always answers once the local step is done")
    }

    /// Select and install the stub's connection, as a shell would.
    async fn installed(s: &DaemonShared, server: &Server) {
        select(s).await.result.expect("selected");
        server.stub.lock().unwrap().current = Some(server.status(false));
        install(s, server).await.result.expect("installed");
        assert!(config(s).witness.is_some());
    }

    fn audit_rows(s: &DaemonShared) -> Vec<(String, Option<String>)> {
        audit::load(&s.store)
            .unwrap()
            .into_iter()
            .map(|e| (e.action, e.detail))
            .collect()
    }

    fn config(s: &DaemonShared) -> ContributorConfig {
        s.store.load_config().unwrap().unwrap()
    }

    fn audit_actions(s: &DaemonShared) -> Vec<String> {
        audit::load(&s.store)
            .unwrap()
            .into_iter()
            .map(|e| e.action)
            .collect()
    }

    fn error_message(response: Response) -> String {
        response.error.expect("an error response").message
    }

    #[tokio::test]
    async fn every_method_refuses_without_an_account_session() {
        let server = test_support::spawn().await;
        let s = shared();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = server.base.clone();
        s.store.save_config(&cfg).unwrap();
        let id = Uuid::new_v4().to_string();
        let install_params = serde_json::json!({
            "connection_id": id,
            "config_digest": test_support::config_digest(),
        });
        let responses = [
            handle_offers(
                &s,
                &req("inference_connection_offers", serde_json::json!({})),
            )
            .await,
            handle_current(
                &s,
                &req("inference_connection_current", serde_json::json!({})),
            )
            .await,
            select(&s).await,
            handle_install(&s, &req("inference_connection_install", install_params)).await,
        ];
        for response in responses {
            let error = response.error.expect("refused");
            assert_eq!(error.code, ERR_UNAVAILABLE);
            assert_eq!(error.message, ERR_ACCOUNT_SESSION_REQUIRED);
        }
        // Disconnect does its local step and reports the server step as
        // waiting on a session, under the same label.
        let result = disconnect(&s, Uuid::new_v4()).await;
        assert_eq!(result["disconnected"], false);
        assert_eq!(result["server_disconnect"], SERVER_PENDING);
        assert_eq!(result["server_refusal"], ERR_ACCOUNT_SESSION_REQUIRED);
        // Not one call reached the server.
        assert!(server.seen.lock().unwrap().auth.is_empty());
    }

    #[tokio::test]
    async fn an_expired_session_is_the_same_refusal() {
        let s = shared();
        let session = crate::account_auth::AccountSession {
            access_token: SESSION.to_string(),
            expires_at: Utc::now() - chrono::TimeDelta::hours(1),
            account_id: "acct".to_string(),
        };
        s.store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        assert_eq!(
            error_message(select(&s).await),
            ERR_ACCOUNT_SESSION_REQUIRED
        );
    }

    #[tokio::test]
    async fn the_account_session_is_the_bearer_and_select_sends_the_shown_values() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let before = serde_json::to_value(config(&s)).unwrap();
        let response = select(&s).await;
        let result = response.result.expect("selected");
        assert_eq!(result["install_required"], true);
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.auth, vec![format!("Bearer {SESSION}")]);
        let mut expected = select_params();
        expected["expected_current_version"] = serde_json::Value::Null;
        // The provider is bound into the digest check, never sent.
        expected.as_object_mut().unwrap().remove("provider_id");
        assert_eq!(seen.selects, vec![expected]);
        // Selecting installs nothing.
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), before);
        assert_eq!(audit_actions(&s), vec![AUDIT_SELECTED.to_string()]);
    }

    #[tokio::test]
    async fn a_malformed_selection_is_a_param_error_and_never_sent() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let mut params = select_params();
        params["disclosure_version"] = serde_json::json!("other");
        let response = handle_select(&s, &req("inference_connection_select", params)).await;
        assert_eq!(response.error.unwrap().code, ERR_BAD_PARAMS);
        assert!(server.seen.lock().unwrap().selects.is_empty());
    }

    #[tokio::test]
    async fn reselection_required_installs_nothing() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let before = serde_json::to_value(config(&s)).unwrap();
        server.stub.lock().unwrap().select_refusal = Some((
            axum::http::StatusCode::CONFLICT,
            "connection_reselection_required",
        ));
        assert_eq!(
            error_message(select(&s).await),
            "connection_reselection_required"
        );
        assert_eq!(
            error_message(install(&s, &server).await),
            ERR_SELECT_REQUIRED
        );
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), before);
    }

    #[tokio::test]
    async fn a_revision_retired_between_select_and_install_installs_nothing() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let before = serde_json::to_value(config(&s)).unwrap();
        select(&s).await.result.expect("selected");
        let retired = server.status(true);
        server.stub.lock().unwrap().current = Some(retired);
        assert_eq!(
            error_message(install(&s, &server).await),
            "connection_reselection_required"
        );
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), before);
        // And the held selection is gone: no later confirmation installs it.
        server.stub.lock().unwrap().current = Some(server.status(false));
        assert_eq!(
            error_message(install(&s, &server).await),
            ERR_SELECT_REQUIRED
        );
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), before);
    }

    #[tokio::test]
    async fn install_writes_exactly_the_returned_witness_and_nothing_else() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let before = config(&s);
        select(&s).await.result.expect("selected");
        let current = server.status(false);
        server.stub.lock().unwrap().current = Some(current);
        let result = install(&s, &server).await.result.expect("installed");
        assert_eq!(result["receipt_endpoint_installed"], false);
        let after = config(&s);
        let returned = test_support::witness();
        let witness = after.witness.clone().expect("a witness");
        // K11: the install says where the witness came from.
        assert_eq!(
            after.witness_origin_view(),
            Some(crate::config::WitnessOriginView::Recorded(
                crate::config::WitnessOrigin::ConnectedInference
            ))
        );
        assert_eq!(witness.url, returned.url);
        assert_eq!(witness.signing_address, returned.signing_address);
        assert_eq!(
            witness.expected_measurements,
            returned.expected_measurements
        );
        assert!(!witness.admission_evidence);
        let mut expected = serde_json::to_value(&before).unwrap();
        expected["witness"] = serde_json::to_value(&witness).unwrap();
        // And the record of where it came from, for the disclosure screens.
        expected["witness_origin"] =
            serde_json::to_value(crate::config::WitnessOriginRecord::for_witness(
                &witness,
                crate::config::WitnessOrigin::ConnectedInference,
            ))
            .unwrap();
        assert_eq!(serde_json::to_value(&after).unwrap(), expected);
        assert_eq!(
            audit_actions(&s),
            vec![AUDIT_SELECTED.to_string(), AUDIT_INSTALLED.to_string()]
        );
    }

    #[tokio::test]
    async fn install_writes_the_receipt_endpoint_when_the_selection_carries_one() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        server.stub.lock().unwrap().receipt = Some("https://receipt.example/v1".into());
        handle_select(
            &s,
            &req(
                "inference_connection_select",
                select_params_with(Some("https://receipt.example/v1")),
            ),
        )
        .await
        .result
        .expect("selected");
        server.stub.lock().unwrap().current = Some(server.status(false));
        let result = install(&s, &server).await.result.expect("installed");
        assert_eq!(result["receipt_endpoint_installed"], true);
        assert_eq!(
            config(&s).inference_receipt_endpoint.as_deref(),
            Some("https://receipt.example/v1")
        );
    }

    #[tokio::test]
    async fn install_needs_a_selection_made_on_this_device() {
        // A selection made on another device is an account record, not
        // permission for this one: the current route reports it, and install
        // still refuses until this device selects and confirms.
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        server.stub.lock().unwrap().current = Some(server.status(false));
        assert_eq!(
            error_message(install(&s, &server).await),
            ERR_SELECT_REQUIRED
        );
        assert!(config(&s).witness.is_none());
        let current = handle_current(
            &s,
            &req("inference_connection_current", serde_json::json!({})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(current["installed_on_this_device"], false);
        assert_eq!(current["pending_install"], false);
    }

    #[tokio::test]
    async fn install_refuses_a_confirmation_for_another_configuration() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        select(&s).await.result.expect("selected");
        server.stub.lock().unwrap().current = Some(server.status(false));
        let id = server.stub.lock().unwrap().connection_id;
        let response = handle_install(
            &s,
            &req(
                "inference_connection_install",
                serde_json::json!({
                    "connection_id": id.to_string(),
                    "config_digest": format!("sha256:{}", "c".repeat(64)),
                }),
            ),
        )
        .await;
        assert_eq!(error_message(response), ERR_CONFIRMATION_MISMATCH);
        assert!(config(&s).witness.is_none());
    }

    #[tokio::test]
    async fn disconnect_revokes_and_removes_what_install_wrote() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let before = serde_json::to_value(config(&s)).unwrap();
        select(&s).await.result.expect("selected");
        server.stub.lock().unwrap().current = Some(server.status(false));
        install(&s, &server).await.result.expect("installed");
        let id = server.stub.lock().unwrap().connection_id;
        let result = handle_disconnect(
            &s,
            &req(
                "inference_connection_disconnect",
                serde_json::json!({ "connection_id": id.to_string() }),
            ),
        )
        .await
        .result
        .expect("disconnected");
        assert_eq!(result["local_witness_removed"], true);
        assert_eq!(result["disconnected"], true);
        assert_eq!(result["server_disconnect"], SERVER_REVOKED);
        assert_eq!(
            server.seen.lock().unwrap().disconnects,
            vec![id.to_string()]
        );
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), before);
        assert_eq!(
            audit_actions(&s).last().map(String::as_str),
            Some(AUDIT_DISCONNECTED)
        );
    }

    #[tokio::test]
    async fn disconnect_leaves_a_witness_configured_some_other_way() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let mut cfg = config(&s);
        cfg.witness = Some(WitnessSettings {
            admission_evidence: false,
            url: "https://own-witness.example".into(),
            signing_address: format!("0x{}", "cd".repeat(20)),
            expected_measurements: vec![format!("mrtd={}", "cd".repeat(48))],
        });
        s.store.save_config(&cfg).unwrap();
        let result = handle_disconnect(
            &s,
            &req(
                "inference_connection_disconnect",
                serde_json::json!({ "connection_id": Uuid::new_v4().to_string() }),
            ),
        )
        .await
        .result
        .expect("disconnected");
        assert_eq!(result["local_witness_removed"], false);
        assert_eq!(config(&s).witness, cfg.witness);
    }

    #[tokio::test]
    async fn an_observed_revocation_removes_the_installed_witness() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        select(&s).await.result.expect("selected");
        server.stub.lock().unwrap().current = Some(server.status(false));
        install(&s, &server).await.result.expect("installed");
        // Disconnected from another device.
        server.stub.lock().unwrap().current = None;
        let result = handle_current(
            &s,
            &req("inference_connection_current", serde_json::json!({})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(result["revocation_applied"], true);
        assert!(config(&s).witness.is_none());
    }

    #[tokio::test]
    async fn a_retired_revision_is_reported_and_changes_nothing() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        select(&s).await.result.expect("selected");
        server.stub.lock().unwrap().current = Some(server.status(false));
        install(&s, &server).await.result.expect("installed");
        let installed = serde_json::to_value(config(&s)).unwrap();
        server.stub.lock().unwrap().current = Some(server.status(true));
        let result = handle_current(
            &s,
            &req("inference_connection_current", serde_json::json!({})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(result["selection"]["reselection_required"], true);
        assert_eq!(result["revocation_applied"], false);
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), installed);
    }

    #[tokio::test]
    async fn offers_are_listed_with_the_account_session() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let result = handle_offers(
            &s,
            &req("inference_connection_offers", serde_json::json!({})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(result["offers"][0]["offer_id"], "near-ai");
        assert_eq!(
            server.seen.lock().unwrap().auth,
            vec![format!("Bearer {SESSION}")]
        );
    }

    /// Every audit row a full flow writes: the action is one of the four
    /// constants, and the `detail` actually written is one of the fixed
    /// labels -- never a URL, host, signing address, pin, token or id.
    #[tokio::test]
    async fn audit_rows_carry_no_url_or_identifier() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        server.stub.lock().unwrap().receipt = Some("https://receipt.example/v1".into());
        handle_select(
            &s,
            &req(
                "inference_connection_select",
                select_params_with(Some("https://receipt.example/v1")),
            ),
        )
        .await
        .result
        .expect("selected");
        server.stub.lock().unwrap().current = Some(server.status(false));
        install(&s, &server).await.result.expect("installed");
        let first = server.stub.lock().unwrap().connection_id;
        // Reselect here: the old witness goes at once.
        {
            let mut stub = server.stub.lock().unwrap();
            stub.connection_id = Uuid::new_v4();
            stub.receipt = None;
        }
        select(&s).await.result.expect("selected again");
        server.stub.lock().unwrap().current = Some(server.status(false));
        install(&s, &server).await.result.expect("installed again");
        // Replaced from another device (applied by `current`), then
        // disconnected here, where nothing is left to remove.
        server.stub.lock().unwrap().current = None;
        current(&s).await;
        let second = server.stub.lock().unwrap().connection_id;
        disconnect(&s, second).await;

        let rows = audit_rows(&s);
        let allowed_details = [
            "near-ai",
            "witness",
            "witness,receipt-endpoint",
            REVOCATION_NOT_LIVE,
            REVOCATION_CONFIGURATION_CHANGED,
            REVOCATION_REPLACED_HERE,
            "local-witness-removed,server-revoked",
            "local-witness-removed,server-pending",
            "local-witness-removed,server-not-found",
            "nothing-installed-here,server-revoked",
            "nothing-installed-here,server-pending",
            "nothing-installed-here,server-not-found",
        ];
        let witness = test_support::witness();
        let forbidden = [
            witness.url.clone(),
            "witness.example".to_string(),
            "receipt.example".to_string(),
            witness.signing_address.clone(),
            witness.signing_address.trim_start_matches("0x").to_string(),
            witness.expected_measurements[0].clone(),
            "ab".repeat(48),
            first.to_string(),
            second.to_string(),
            SESSION.to_string(),
            test_support::config_digest(),
        ];
        assert_eq!(
            rows.iter().map(|(a, _)| a.as_str()).collect::<Vec<_>>(),
            vec![
                AUDIT_SELECTED,
                AUDIT_INSTALLED,
                AUDIT_SELECTED,
                AUDIT_REVOCATION_APPLIED,
                AUDIT_INSTALLED,
                AUDIT_REVOCATION_APPLIED,
                AUDIT_DISCONNECTED,
            ],
            "{rows:?}"
        );
        for (action, detail) in &rows {
            assert!(action.bytes().all(|b| b.is_ascii_lowercase() || b == b'-'));
            let detail = detail.as_deref().expect("every row carries a detail");
            assert!(
                allowed_details.contains(&detail),
                "{action} wrote an unexpected detail {detail:?}"
            );
            for secret in &forbidden {
                assert!(
                    !detail.contains(secret.as_str()),
                    "{action} leaked {secret}"
                );
            }
        }
        assert_eq!(
            rows.iter()
                .map(|(_, d)| d.as_deref().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "near-ai",
                "witness,receipt-endpoint",
                "near-ai",
                REVOCATION_REPLACED_HERE,
                "witness",
                REVOCATION_NOT_LIVE,
                "nothing-installed-here,server-revoked",
            ]
        );
    }

    // ---- Disconnect removes locally first (finding: fail-open stop) ----

    #[tokio::test]
    async fn disconnect_without_a_session_still_removes_the_local_witness() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let before = serde_json::to_value(config(&s)).unwrap();
        installed(&s, &server).await;
        s.store
            .remove_daemon_file(crate::config::ACCOUNT_SESSION_FILE)
            .unwrap();
        let id = server.stub.lock().unwrap().connection_id;
        let result = disconnect(&s, id).await;
        assert_eq!(result["local_witness_removed"], true);
        assert_eq!(result["disconnected"], false);
        assert_eq!(result["server_disconnect"], SERVER_PENDING);
        assert_eq!(result["server_refusal"], ERR_ACCOUNT_SESSION_REQUIRED);
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), before);
        assert!(server.seen.lock().unwrap().disconnects.is_empty());
        assert_eq!(
            audit_rows(&s).last().unwrap(),
            &(
                AUDIT_DISCONNECTED.to_string(),
                Some("local-witness-removed,server-pending".to_string())
            )
        );
    }

    /// A network failure: the transport never reaches the commons (here the
    /// client's host allowlist refuses it before connecting), and separately
    /// a commons that answers 503. The witness goes either way.
    #[tokio::test]
    async fn disconnect_with_the_commons_unreachable_still_removes_the_local_witness() {
        for unreachable in [true, false] {
            let server = test_support::spawn().await;
            let s = enrolled(&server.base);
            installed(&s, &server).await;
            if unreachable {
                let mut cfg = config(&s);
                cfg.allowed_hosts = Some("commons.invalid".into());
                s.store.save_config(&cfg).unwrap();
            } else {
                server.stub.lock().unwrap().disconnect_refusal =
                    Some((axum::http::StatusCode::SERVICE_UNAVAILABLE, "unavailable"));
            }
            let id = server.stub.lock().unwrap().connection_id;
            let result = disconnect(&s, id).await;
            assert_eq!(result["local_witness_removed"], true, "{unreachable}");
            assert_eq!(result["disconnected"], false);
            assert_eq!(result["server_disconnect"], SERVER_PENDING);
            assert_eq!(
                result["server_refusal"],
                ConnectionError::Unavailable.label()
            );
            assert!(config(&s).witness.is_none());
            let reached = server.seen.lock().unwrap().disconnects.len();
            assert_eq!(reached, usize::from(!unreachable));
        }
    }

    #[tokio::test]
    async fn disconnect_refused_by_the_server_still_removes_the_local_witness() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        installed(&s, &server).await;
        server.stub.lock().unwrap().disconnect_refusal =
            Some((axum::http::StatusCode::FORBIDDEN, "forbidden"));
        let id = server.stub.lock().unwrap().connection_id;
        let result = disconnect(&s, id).await;
        assert_eq!(result["local_witness_removed"], true);
        assert_eq!(result["disconnected"], false);
        assert_eq!(result["server_disconnect"], SERVER_PENDING);
        assert_eq!(result["server_refusal"], ERR_ACCOUNT_SESSION_REQUIRED);
        assert_eq!(
            server.seen.lock().unwrap().disconnects,
            vec![id.to_string()]
        );
        assert!(config(&s).witness.is_none());
        // Nothing is left for a later `current` to apply.
        server.stub.lock().unwrap().current = None;
        assert_eq!(current(&s).await["revocation_applied"], false);
    }

    // ---- Digest verification of the witness material ----

    #[tokio::test]
    async fn select_refuses_witness_material_that_does_not_match_its_digest() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let before = serde_json::to_value(config(&s)).unwrap();
        server.stub.lock().unwrap().witness.url = "https://other-witness.example/v1".into();
        assert_eq!(
            error_message(select(&s).await),
            ConnectionError::DigestMismatch.label()
        );
        // Nothing is held for install.
        server.stub.lock().unwrap().current = Some(server.status(false));
        assert_eq!(
            error_message(install(&s, &server).await),
            ERR_SELECT_REQUIRED
        );
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), before);
    }

    #[tokio::test]
    async fn install_re_verifies_the_held_material_against_its_digest() {
        type Tamper = fn(&mut SelectedInferenceConnection);
        let tampers: [Tamper; 3] = [
            |sel| sel.witness.url = "https://other-witness.example/v1".into(),
            |sel| sel.witness.signing_address = format!("0x{}", "cd".repeat(20)),
            |sel| sel.witness.expected_measurements = vec![format!("mrtd={}", "cd".repeat(48))],
        ];
        for tamper in tampers {
            let server = test_support::spawn().await;
            let s = enrolled(&server.base);
            select(&s).await.result.expect("selected");
            server.stub.lock().unwrap().current = Some(server.status(false));
            // The held selection is altered on disk, digest unchanged.
            let mut state = load_state(&s).unwrap();
            tamper(&mut state.pending.as_mut().unwrap().selected);
            save_state(&s, &state).unwrap();
            assert_eq!(
                error_message(install(&s, &server).await),
                ConnectionError::DigestMismatch.label()
            );
            assert!(config(&s).witness.is_none());
            assert!(load_state(&s).unwrap().pending.is_none());
        }
    }

    // ---- Exactness of the install-time currency check ----

    #[tokio::test]
    async fn install_refuses_when_the_account_reports_another_digest_or_revision() {
        type Change = fn(&mut ConnectionStatus);
        let changes: [Change; 2] = [
            |c| c.config_digest = format!("sha256:{}", "e".repeat(64)),
            |c| c.revision = format!("sha256:{}", "e".repeat(64)),
        ];
        for change in changes {
            let server = test_support::spawn().await;
            let s = enrolled(&server.base);
            select(&s).await.result.expect("selected");
            let mut status = server.status(false);
            change(&mut status);
            server.stub.lock().unwrap().current = Some(status);
            assert_eq!(
                error_message(install(&s, &server).await),
                "connection_reselection_required"
            );
            assert!(config(&s).witness.is_none());
        }
    }

    #[test]
    fn still_current_compares_every_field_of_the_held_selection() {
        let witness = test_support::witness();
        let digest = test_support::config_digest();
        let pending = SelectedInferenceConnection {
            connection_id: Uuid::new_v4(),
            state_version: 1,
            revision: test_support::revision(),
            config_digest: digest,
            disclosure_version: DISCLOSURE_VERSION.into(),
            witness,
            inference_receipt_endpoint: None,
        };
        let status = ConnectionStatus {
            connection_id: pending.connection_id,
            state_version: 1,
            offer_id: "near-ai".into(),
            revision: pending.revision.clone(),
            config_digest: pending.config_digest.clone(),
            disclosure_version: pending.disclosure_version.clone(),
            reselection_required: false,
        };
        assert_eq!(still_current(Some(&status), &pending), None);
        assert_eq!(still_current(None, &pending), Some(ERR_NOT_CURRENT));
        type Change = fn(&mut ConnectionStatus);
        let changes: [(Change, &str); 5] = [
            (|c| c.connection_id = Uuid::new_v4(), ERR_NOT_CURRENT),
            (
                |c| c.reselection_required = true,
                "connection_reselection_required",
            ),
            (
                |c| c.revision = format!("sha256:{}", "e".repeat(64)),
                "connection_reselection_required",
            ),
            (
                |c| c.config_digest = format!("sha256:{}", "e".repeat(64)),
                "connection_reselection_required",
            ),
            (
                |c| c.disclosure_version = "inference-connection-disclosure-v2".into(),
                "connection_reselection_required",
            ),
        ];
        for (change, label) in changes {
            let mut altered = status.clone();
            change(&mut altered);
            assert_eq!(still_current(Some(&altered), &pending), Some(label));
        }
    }

    #[tokio::test]
    async fn current_removes_a_witness_whose_connection_reports_another_configuration() {
        type Change = fn(&mut ConnectionStatus);
        let changes: [Change; 2] = [
            |c| c.config_digest = format!("sha256:{}", "e".repeat(64)),
            |c| c.revision = format!("sha256:{}", "e".repeat(64)),
        ];
        for change in changes {
            let server = test_support::spawn().await;
            let s = enrolled(&server.base);
            installed(&s, &server).await;
            let mut status = server.status(false);
            change(&mut status);
            server.stub.lock().unwrap().current = Some(status);
            let result = current(&s).await;
            assert_eq!(result["revocation_applied"], true);
            assert_eq!(result["installed_on_this_device"], false);
            assert!(config(&s).witness.is_none());
            assert_eq!(
                audit_rows(&s).last().unwrap(),
                &(
                    AUDIT_REVOCATION_APPLIED.to_string(),
                    Some(REVOCATION_CONFIGURATION_CHANGED.to_string())
                )
            );
        }
    }

    #[test]
    fn an_installed_connection_is_live_only_with_every_field_unchanged() {
        let installed = Installed {
            connection_id: Uuid::new_v4(),
            state_version: 1,
            revision: test_support::revision(),
            config_digest: test_support::config_digest(),
            disclosure_version: DISCLOSURE_VERSION.into(),
            witness: test_support::witness(),
            inference_receipt_endpoint: None,
        };
        let status = ConnectionStatus {
            connection_id: installed.connection_id,
            state_version: 1,
            offer_id: "near-ai".into(),
            revision: installed.revision.clone(),
            config_digest: installed.config_digest.clone(),
            disclosure_version: installed.disclosure_version.clone(),
            reselection_required: false,
        };
        assert!(installed_is_live(&installed, &status));
        // Retirement alone keeps it (see the module docs).
        let mut retired = status.clone();
        retired.reselection_required = true;
        assert!(installed_is_live(&installed, &retired));
        type Change = fn(&mut ConnectionStatus);
        let changes: [Change; 4] = [
            |c| c.connection_id = Uuid::new_v4(),
            |c| c.revision = format!("sha256:{}", "e".repeat(64)),
            |c| c.config_digest = format!("sha256:{}", "e".repeat(64)),
            |c| c.disclosure_version = "inference-connection-disclosure-v2".into(),
        ];
        for change in changes {
            let mut altered = status.clone();
            change(&mut altered);
            assert!(!installed_is_live(&installed, &altered));
        }
    }

    // ---- Reselecting on this device ----

    #[tokio::test]
    async fn reselecting_removes_the_previously_installed_witness_at_once() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        let before = serde_json::to_value(config(&s)).unwrap();
        installed(&s, &server).await;
        server.stub.lock().unwrap().connection_id = Uuid::new_v4();
        let result = select(&s).await.result.expect("selected again");
        assert_eq!(result["previous_witness_removed"], true);
        assert_eq!(serde_json::to_value(config(&s)).unwrap(), before);
        assert!(load_state(&s).unwrap().installed.is_none());
        assert_eq!(
            audit_rows(&s).last().unwrap(),
            &(
                AUDIT_REVOCATION_APPLIED.to_string(),
                Some(REVOCATION_REPLACED_HERE.to_string())
            )
        );
    }

    #[tokio::test]
    async fn an_idempotent_reselect_of_the_installed_connection_keeps_it() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        installed(&s, &server).await;
        let result = select(&s).await.result.expect("same answer");
        assert_eq!(result["previous_witness_removed"], false);
        assert!(config(&s).witness.is_some());
    }

    // ---- Re-reading after the network await ----

    #[tokio::test]
    async fn install_keeps_config_written_while_it_awaited_the_server() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        select(&s).await.result.expect("selected");
        {
            let mut stub = server.stub.lock().unwrap();
            stub.current = Some(server_status_unlocked(&stub));
        }
        let (entered, release) = hold_current(&server);
        let concurrent_write = async {
            entered.notified().await;
            let mut cfg = config(&s);
            cfg.pii_filter = Some("written-meanwhile".into());
            s.store.save_config(&cfg).unwrap();
            release.notify_one();
        };
        let (response, ()) = tokio::join!(install(&s, &server), concurrent_write);
        response.result.expect("installed");
        let after = config(&s);
        assert_eq!(after.pii_filter.as_deref(), Some("written-meanwhile"));
        assert!(after.witness.is_some());
    }

    #[tokio::test]
    async fn install_refuses_when_a_select_replaced_its_selection_meanwhile() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        select(&s).await.result.expect("selected");
        let first = server.stub.lock().unwrap().connection_id;
        {
            let mut stub = server.stub.lock().unwrap();
            // The server still reports the first selection to this install.
            stub.current = Some(server_status_unlocked(&stub));
        }
        let (entered, release) = hold_current(&server);
        let install_request = req(
            "inference_connection_install",
            serde_json::json!({
                "connection_id": first.to_string(),
                "config_digest": test_support::config_digest(),
            }),
        );
        let first_install = handle_install(&s, &install_request);
        let reselect = async {
            entered.notified().await;
            server.stub.lock().unwrap().connection_id = Uuid::new_v4();
            let mut params = select_params();
            params["idempotency_key"] = serde_json::json!(Uuid::new_v4().to_string());
            let second = handle_select(&s, &req("inference_connection_select", params))
                .await
                .result
                .expect("selected again");
            release.notify_one();
            second
        };
        let (response, second) = tokio::join!(first_install, reselect);
        assert_eq!(error_message(response), ERR_SELECTION_REPLACED);
        assert!(config(&s).witness.is_none());
        // The newer selection survives, and is what install now takes.
        let held = load_state(&s).unwrap().pending.expect("still held");
        assert_eq!(
            held.selected.connection_id.to_string(),
            second["connection_id"].as_str().unwrap()
        );
    }

    #[tokio::test]
    async fn current_keeps_a_selection_made_while_it_awaited_the_server() {
        let server = test_support::spawn().await;
        let s = enrolled(&server.base);
        {
            let mut stub = server.stub.lock().unwrap();
            // The account has no selection when `current` asks.
            stub.current = None;
        }
        let (entered, release) = hold_current(&server);
        let current_request = req("inference_connection_current", serde_json::json!({}));
        let asking = handle_current(&s, &current_request);
        let selecting = async {
            entered.notified().await;
            let selected = select(&s).await.result.expect("selected");
            release.notify_one();
            selected
        };
        let (response, _) = tokio::join!(asking, selecting);
        response.result.expect("answered");
        assert!(load_state(&s).unwrap().pending.is_some());
    }

    /// Hold the stub's current route; returns (entered, release).
    fn hold_current(
        server: &Server,
    ) -> (
        std::sync::Arc<tokio::sync::Notify>,
        std::sync::Arc<tokio::sync::Notify>,
    ) {
        let entered = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        server.stub.lock().unwrap().current_hold = Some((entered.clone(), release.clone()));
        (entered, release)
    }

    fn server_status_unlocked(stub: &test_support::Stub) -> ConnectionStatus {
        ConnectionStatus {
            connection_id: stub.connection_id,
            state_version: stub.state_version,
            offer_id: "near-ai".into(),
            revision: test_support::revision_with(stub.receipt.as_deref()),
            config_digest: test_support::config_digest_with(stub.receipt.as_deref()),
            disclosure_version: DISCLOSURE_VERSION.into(),
            reselection_required: false,
        }
    }
}
