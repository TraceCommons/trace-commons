//! Moving a legacy `tenant-…` invite identity onto the contributor's NEAR AI
//! account, at the contributor's request and only then.
//!
//! The server half is #1066 (`docs/operator/legacy-invite-migration.md`).
//! This is the client half the consent spec asks for ("The invite-to-account
//! migration re-baselines rather than voids"), with the decisions of
//! 2026-09-27: a **staged second key**, the NEAR AI login first, and the
//! wallet later through the same [`AccountProvisioner`] seam.
//!
//! # Opt-in
//!
//! Nothing here runs on its own. The only entry point is the
//! `legacy_invite_migrate` IPC method, which a shell calls when the
//! contributor chooses to move. Coexistence is the default: a legacy invite
//! device keeps contributing under its invite for as long as it likes, and a
//! pooled invite (a shared event code) is refused by the server and keeps
//! working exactly as before.
//!
//! # The flow
//!
//! 1. The legacy config, device key and a `Kind::Device` snapshot are read.
//!    The snapshot is the logout guard: the switch at the end refuses if it
//!    has moved.
//! 2. A **second device key** is generated into the staging slot, and the
//!    NEAR AI login ceremony provisions it into the contributor's NEAR AI
//!    account. That yields a NEAR account session, held in memory only.
//! 3. The configured ingest is asked, under that session, whether it admits
//!    that account by account (`/v1/account/contribution-status`). Only an
//!    affirmative, `ready` answer goes on: an invitee switched into `nearai-`
//!    before then would meet #706 and have their folders held.
//! 4. The daemon asks for a challenge, signs the link statement with the
//!    **legacy** key, posts it, and verifies the countersigned record it gets
//!    back: the device signature against its own legacy key, and the account
//!    against the one it is signed into. The server countersignature is kept
//!    and not verified; see [`verify_record`].
//! 5. Under the watcher's pass lock, so no pass can run between the steps:
//!    the new config is written, the staged key promoted and the new session
//!    stored ([`commons_credentials::switch_identity`]); then every armed
//!    folder's grant is re-recorded with the new identity term and nothing
//!    else ([`rebaseline_identity`]); then the link record and the notice are
//!    written; and **last** the legacy key is retired.
//!
//! A failure anywhere before the last step leaves the legacy identity fully
//! working: everything before step 5 changes nothing local except a staged
//! key, which is discarded, and step 5 rolls its own writes back -- in
//! process, or at the next start through [`recover`] if the daemon died
//! mid-way.
//!
//! # The re-baseline is not an IPC method
//!
//! [`rebaseline_identity`] is private to this module and called from the
//! commit step and from recovery only. No shell and no CLI can re-record a
//! grant's terms; `the_rebaseline_is_not_reachable_over_ipc` pins that.
//!
//! # What the contributor is told
//!
//! A notice, until they acknowledge it: their contributions now go under
//! their NEAR AI account, and their automatic folders stay automatic. The
//! words are `consent_copy::legacy_migration_notice`. Queued sessions that
//! were already approved return to pending after the switch, because their
//! approval was bound to the old identity (`input_fingerprint`); the watcher
//! re-approves them in armed folders on its next pass. That is expected.
//!
//! Every error is a fixed label. None carries a tenant id, account id, key
//! id, token or URL.

use std::future::Future;

use anyhow::{Result, anyhow, bail};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use trace_commons_protocol::legacy_invite_link::{
    LegacyInviteLinkChallenge, LegacyInviteLinkRecord, LegacyInviteLinkRequest,
    LegacyInviteLinkResponse, LegacyInviteLinkStatement, legacy_invite_link_statement_bytes,
};

use super::commons_credentials::{self, Kind, SwitchPhase};
use super::grant_terms::IdentityTerm;
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::policy::{ProjectMode, ProjectPolicy};
use crate::config::{ConfigStore, ContributorConfig, LEGACY_INVITE_LINK_FILE, allowlist_for};
use crate::identity::DeviceIdentity;

pub const CHALLENGE_PATH: &str = "/v1/account/invites/legacy-link/challenge";
pub const LINK_PATH: &str = "/v1/account/invites/legacy-link";

/// The labels a migration can end with. Anything else becomes
/// `legacy_migration_unavailable`.
pub const LABELS: &[&str] = &[
    "legacy_migration_not_enrolled",
    "legacy_migration_not_applicable",
    "legacy_migration_already_migrated",
    "legacy_migration_device_key_missing",
    "legacy_migration_pending",
    "legacy_migration_invite_needed",
    "legacy_migration_invite_invalid",
    "legacy_migration_invite_other_commons",
    "legacy_migration_no_near_ai_session",
    "legacy_migration_account_unavailable",
    "legacy_migration_commons_changed",
    "legacy_migration_admission_not_ready",
    "legacy_migration_link_not_enabled",
    "legacy_migration_tenant_pooled",
    "legacy_migration_tenant_claimed",
    "legacy_migration_invite_revoked",
    "legacy_migration_device_not_eligible",
    "legacy_migration_link_refused",
    "legacy_migration_verification_failed",
    "legacy_migration_identity_changed",
    "legacy_migration_unavailable",
];

fn label(error: &anyhow::Error) -> &'static str {
    let text = error.to_string();
    if let Some(known) = LABELS.iter().find(|l| **l == text) {
        return known;
    }
    match text.as_str() {
        "near_ai_enroll_no_session" => "legacy_migration_no_near_ai_session",
        "commons_credential_state_changed"
        | "identity_switch_not_enrolled"
        | "identity_switch_no_staged_key" => "legacy_migration_identity_changed",
        "identity_switch_pending" => "legacy_migration_pending",
        t if t.starts_with("near_ai_enroll_") => "legacy_migration_account_unavailable",
        _ => "legacy_migration_unavailable",
    }
}

/// The account a provisioner put the staged key into, and the session it
/// holds for it. `access_token` is a secret: this type is not `Debug`.
pub struct ProvisionedAccount {
    pub tenant_id: String,
    pub account_id: Uuid,
    pub access_token: String,
    pub expires_in_secs: i64,
    /// The commons facts the ceremony ran against, checked against the
    /// config so the migration changes nothing but identity.
    pub issuer_url: String,
    pub audience: String,
}

/// Provisions the staged key into a NEAR account and returns a session for
/// it. The NEAR AI login is the only implementation today; the wallet
/// (`near-`) ceremony is the next, and plugs in here.
pub trait AccountProvisioner: Sync {
    fn provision<'a>(
        &'a self,
        shared: &'a DaemonShared,
        cfg: &'a ContributorConfig,
        staged: &'a DeviceIdentity,
    ) -> impl Future<Output = Result<ProvisionedAccount>> + Send + 'a;
}

/// The NEAR AI login: the contributor's retained NEAR AI session is
/// exchanged for a JWT the commons introspects, exactly as enrollment does.
pub struct NearAiLogin {
    api: super::nearai_credential::api::CloudApi,
}

impl AccountProvisioner for NearAiLogin {
    async fn provision(
        &self,
        shared: &DaemonShared,
        cfg: &ContributorConfig,
        staged: &DeviceIdentity,
    ) -> Result<ProvisionedAccount> {
        let prepared = super::nearai_onboarding::prepare(shared, &cfg.ingest_url).await?;
        let provisioned =
            super::nearai_onboarding::provision(shared, &self.api, prepared, staged).await?;
        super::nearai_onboarding::provisioned_account(provisioned, staged)
    }
}

/// Whether `tenant_id` names a legacy invite identity: not in either NEAR
/// namespace. The server's own rule, applied here only to decide whether to
/// offer the move; the server decides whether it may happen.
pub fn is_legacy_tenant(tenant_id: &str) -> bool {
    !tenant_id.is_empty()
        && !trace_commons_protocol::admission::is_anchored_tenant(tenant_id)
        && !tenant_id.starts_with("near-")
        && !tenant_id.starts_with("nearai-")
}

/// Whether `cfg` can be offered the move at all: a legacy tenant, enrolled
/// by an invite rather than an instance (an instance-enrolled device
/// redeemed no invite, and the server refuses it).
pub fn offered_for(cfg: &ContributorConfig) -> bool {
    is_legacy_tenant(&cfg.tenant_id) && cfg.instance_id.is_empty()
}

/// The invite's subject hash, as the issuer computes it from the code.
pub fn invite_subject_hash(code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"invite:");
    hasher.update(code.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

fn host_of(url: &str) -> Option<String> {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_ascii_lowercase))
}

/// The subject hash for a pasted invite: the full link, whose issuer must be
/// this daemon's own, or the bare code.
fn invite_hash_from_paste(raw: &str, cfg: &ContributorConfig) -> Result<String> {
    let raw = raw.trim();
    if raw.contains("://") {
        let parsed = crate::commands::parse_invite(raw)
            .map_err(|_| anyhow!("legacy_migration_invite_invalid"))?;
        if host_of(&parsed.issuer_url).is_none()
            || host_of(&parsed.issuer_url) != host_of(&cfg.issuer_url)
        {
            bail!("legacy_migration_invite_other_commons");
        }
        return Ok(invite_subject_hash(&parsed.code));
    }
    if raw.is_empty()
        || raw.len() > 128
        || !raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        bail!("legacy_migration_invite_invalid");
    }
    Ok(invite_subject_hash(raw))
}

/// What is kept after a migration: the countersigned record, and whether the
/// contributor has seen the notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredLink {
    pub record: LegacyInviteLinkRecord,
    /// Ingest's signature over the record, standard base64. Kept, not
    /// verified: see [`verify_record`].
    pub server_signature: String,
    pub migrated_at: DateTime<Utc>,
    /// How many armed folders, and whether the automatic grant, were carried
    /// over. Counts only.
    pub folders_kept: usize,
    pub automatic_grant_kept: bool,
    pub notice_acknowledged: bool,
}

pub fn load_link(store: &ConfigStore) -> Option<StoredLink> {
    let bytes = store.read_daemon_file(LEGACY_INVITE_LINK_FILE).ok()??;
    serde_json::from_slice(&bytes).ok()
}

fn save_link(store: &ConfigStore, link: &StoredLink) -> Result<()> {
    store.write_daemon_file(
        LEGACY_INVITE_LINK_FILE,
        &serde_json::to_vec(link).map_err(|_| anyhow!("legacy_migration_unavailable"))?,
    )
}

/// `status.legacy_invite_migration`: whether the move can be offered, and
/// the notice while it is unacknowledged. No identifiers.
pub fn status_value(store: &ConfigStore, cfg: Option<&ContributorConfig>) -> serde_json::Value {
    let link = load_link(store);
    let notice = link.as_ref().filter(|l| !l.notice_acknowledged).map(|l| {
        serde_json::json!({
            "folders_kept": l.folders_kept,
            "automatic_grant_kept": l.automatic_grant_kept,
        })
    });
    serde_json::json!({
        "offered": link.is_none() && cfg.is_some_and(offered_for),
        "notice": notice,
    })
}

/// The countersigned record binds *this* legacy identity to *this* account.
///
/// Checked: the statement names this daemon's legacy tenant, its own legacy
/// device key, the invite it asked about, and exactly the account tenant and
/// account id it is signed into; and the device signature verifies against
/// its own legacy public key over the statement bytes. Not required: that the
/// nonce is the one just issued -- a retry of a link that already landed
/// gets the original record back, signed by this same key over its own
/// nonce, and that is still proof that this key chose this account.
///
/// Not checked: the server countersignature. The only key to check it
/// against is the attestation keyset the same origin publishes, which would
/// add nothing to the TLS session the record came over; it is kept, with the
/// record, for whoever can check it later.
pub fn verify_record(
    response: &LegacyInviteLinkResponse,
    legacy: &DeviceIdentity,
    legacy_tenant_id: &str,
    invite_subject_hash: &str,
    account: &ProvisionedAccount,
) -> Result<()> {
    let fail = || anyhow!("legacy_migration_verification_failed");
    let s = &response.record.statement;
    if s.legacy_tenant_id != legacy_tenant_id
        || s.device_key_id != legacy.device_key_id
        || s.invite_subject_hash != invite_subject_hash
        || s.account_tenant_id != account.tenant_id
        || s.account_id != account.account_id
    {
        return Err(fail());
    }
    let engine = base64::engine::general_purpose::STANDARD;
    let signature = engine
        .decode(&response.record.device_signature)
        .map_err(|_| fail())?;
    let public_key = engine.decode(&legacy.public_key_b64).map_err(|_| fail())?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(&legacy_invite_link_statement_bytes(s), &signature)
        .map_err(|_| fail())?;
    let countersignature = engine
        .decode(&response.server_signature)
        .map_err(|_| fail())?;
    if countersignature.len() != 64 {
        return Err(fail());
    }
    Ok(())
}

fn account_client(
    cfg: &ContributorConfig,
    token: &str,
) -> Result<trace_commons_operator_client::Client> {
    trace_commons_operator_client::Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(token)
    .host_allowlist(allowlist_for(cfg.allowed_hosts.as_deref()))
    .timeout(std::time::Duration::from_secs(30))
    .build()
    .map_err(|_| anyhow!("legacy_migration_unavailable"))
}

/// A server refusal, by the label #1066 sends.
fn link_refusal(error: &trace_commons_operator_client::Error) -> anyhow::Error {
    use trace_commons_operator_client::Error as E;
    let label = match error {
        E::ServerLabel { label, .. } => match label.as_str() {
            "legacy_invite_link_not_enabled" => "legacy_migration_link_not_enabled",
            "legacy_link_tenant_pooled" => "legacy_migration_tenant_pooled",
            "legacy_link_tenant_claimed" => "legacy_migration_tenant_claimed",
            "legacy_link_invite_revoked" => "legacy_migration_invite_revoked",
            "legacy_link_device_not_eligible" => "legacy_migration_device_not_eligible",
            "legacy_link_unavailable" => "legacy_migration_unavailable",
            _ => "legacy_migration_link_refused",
        },
        E::HttpFailure { .. } => "legacy_migration_link_refused",
        _ => "legacy_migration_unavailable",
    };
    anyhow!(label)
}

/// Challenge, sign with the legacy key, link. Returns the server's answer,
/// unverified.
async fn link(
    new_cfg: &ContributorConfig,
    account: &ProvisionedAccount,
    legacy: &DeviceIdentity,
    legacy_tenant_id: &str,
    invite_subject_hash: &str,
) -> Result<LegacyInviteLinkResponse> {
    let client = account_client(new_cfg, &account.access_token)?;
    let challenge: LegacyInviteLinkChallenge = client
        .call_json(
            reqwest::Method::POST,
            CHALLENGE_PATH,
            &[],
            Some(&serde_json::json!({})),
        )
        .await
        .map_err(|e| link_refusal(&e))?;
    let now = Utc::now().timestamp();
    if challenge.expires_at <= now
        || challenge.nonce.len() != 64
        || !challenge
            .nonce
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        bail!("legacy_migration_link_refused");
    }
    let statement = LegacyInviteLinkStatement {
        legacy_tenant_id: legacy_tenant_id.to_string(),
        device_key_id: legacy.device_key_id.clone(),
        invite_subject_hash: invite_subject_hash.to_string(),
        account_tenant_id: account.tenant_id.clone(),
        account_id: account.account_id,
        nonce: challenge.nonce.clone(),
        issued_at: challenge.issued_at,
    };
    let request = LegacyInviteLinkRequest {
        legacy_tenant_id: statement.legacy_tenant_id.clone(),
        device_key_id: statement.device_key_id.clone(),
        device_public_key: legacy.public_key_b64.clone(),
        invite_subject_hash: statement.invite_subject_hash.clone(),
        nonce: statement.nonce.clone(),
        issued_at: statement.issued_at,
        signature: legacy.sign_b64(&legacy_invite_link_statement_bytes(&statement)),
    };
    client
        .call_json(reqwest::Method::POST, LINK_PATH, &[], Some(&request))
        .await
        .map_err(|e| link_refusal(&e))
}

/// The switch journal's context: what the re-baseline changed, so recovery
/// can undo or finish it.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SwitchContext {
    old: IdentityTerm,
    new: IdentityTerm,
    link: StoredLink,
}

/// What a re-baseline carried over.
#[derive(Debug, Default, PartialEq, Eq)]
struct Rebaselined {
    project_labels: Vec<String>,
    automatic_grant: bool,
}

/// Re-record the identity term of every armed folder, and of the automatic
/// grant, granted under `from`, as `to`. Only the identity term: the rest of
/// each grant's terms stays as it was granted, so anything else the
/// migration changed still voids on the next pass. A folder armed under some
/// other identity is left alone and voids as it would have.
///
/// Private to this module on purpose. It is the one way to change a grant's
/// terms without the contributor re-arming, so it is reachable only from the
/// migration's commit step and from recovery -- never from IPC.
fn rebaseline_identity(
    policy: &mut ProjectPolicy,
    from: &IdentityTerm,
    to: &IdentityTerm,
) -> Rebaselined {
    let mut out = Rebaselined::default();
    for entry in policy.projects.values_mut() {
        if entry.mode != ProjectMode::AutoUpload {
            continue;
        }
        if let Some(terms) = entry.armed_under.as_mut() {
            if terms.identity() == *from {
                *terms = terms.with_identity(to);
                out.project_labels.push(entry.label.clone());
            }
        }
    }
    if let Some(grant) = policy.automatic_grant.as_mut() {
        if grant.granted_under.identity() == *from {
            grant.granted_under = grant.granted_under.with_identity(to);
            out.automatic_grant = true;
        }
    }
    out
}

#[cfg(test)]
fn rebaseline_fail_points()
-> &'static std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>> {
    static POINTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>,
    > = std::sync::OnceLock::new();
    POINTS.get_or_init(Default::default)
}

/// Make the next migration on `store` fail after its re-baseline is saved
/// and before it commits.
#[cfg(test)]
pub(crate) fn fail_after_rebaseline_for_test(store: &ConfigStore) {
    rebaseline_fail_points()
        .lock()
        .unwrap()
        .insert(store.dir().to_path_buf());
}

fn after_rebaseline_fail_point(_store: &ConfigStore) -> Result<()> {
    #[cfg(test)]
    if rebaseline_fail_points()
        .lock()
        .unwrap()
        .remove(_store.dir())
    {
        bail!("legacy_migration_unavailable");
    }
    Ok(())
}

/// Undo the re-baseline and the switch, for a failure before commit.
fn undo(shared: &DaemonShared, context: &SwitchContext) {
    {
        let mut policy = shared.policy.lock().expect("policy lock");
        rebaseline_identity(&mut policy, &context.new, &context.old);
        if policy.save(&shared.store).is_err() {
            tracing::warn!("could not persist the undone re-baseline");
        }
    }
    let _ = shared.store.remove_daemon_file(LEGACY_INVITE_LINK_FILE);
    if commons_credentials::roll_back_switch(&shared.store).is_err() {
        tracing::warn!("identity switch roll-back left for the next start");
    }
}

/// Step 5, under the pass lock. Blocking.
fn commit(
    shared: &DaemonShared,
    legacy_snapshot: &commons_credentials::Snapshot,
    new_cfg: &ContributorConfig,
    session: &[u8],
    context: SwitchContext,
) -> Result<Rebaselined> {
    let _pass = shared.pass_lock.lock().expect("pass lock");
    commons_credentials::switch_identity(
        &shared.store,
        legacy_snapshot,
        new_cfg,
        session,
        serde_json::to_value(&context).map_err(|_| anyhow!("legacy_migration_unavailable"))?,
    )?;
    let rebaselined = {
        let mut policy = shared.policy.lock().expect("policy lock");
        let rebaselined = rebaseline_identity(&mut policy, &context.old, &context.new);
        let saved = policy.save(&shared.store);
        drop(policy);
        if saved.is_err() {
            undo(shared, &context);
            bail!("legacy_migration_unavailable");
        }
        rebaselined
    };
    let mut link = context.link.clone();
    link.folders_kept = rebaselined.project_labels.len();
    link.automatic_grant_kept = rebaselined.automatic_grant;
    if let Err(error) = after_rebaseline_fail_point(&shared.store)
        .and_then(|()| save_link(&shared.store, &link))
        .and_then(|()| commons_credentials::mark_switch_committed(&shared.store))
    {
        undo(shared, &context);
        return Err(error);
    }
    // Committed. Everything below is best effort: recovery finishes the
    // retirement at the next start if it does not happen here.
    let now = Utc::now();
    for project_label in &rebaselined.project_labels {
        let entry = super::audit::AuditEntry {
            at: now,
            action: "auto-upload-rebaselined".to_string(),
            project_label: Some(project_label.clone()),
            detail: Some("identity".to_string()),
        };
        if super::audit::append(&shared.store, &entry).is_err() {
            tracing::warn!("could not record a re-baselined grant");
        }
    }
    let entry = super::audit::AuditEntry {
        at: now,
        action: "legacy-invite-migrated".to_string(),
        project_label: None,
        detail: Some(format!(
            "folders={},automatic_grant={}",
            rebaselined.project_labels.len(),
            rebaselined.automatic_grant
        )),
    };
    if super::audit::append(&shared.store, &entry).is_err() {
        tracing::warn!("could not record the legacy invite migration");
    }
    if commons_credentials::retire_legacy(&shared.store).is_err() {
        tracing::warn!("legacy device key retirement left for the next start");
    }
    shared.account_admission.forget_session();
    Ok(rebaselined)
}

/// Finish or undo a switch the daemon died in the middle of. Runs before
/// the policy is loaded and before any pass.
pub(crate) fn recover(store: &ConfigStore) -> Result<()> {
    let Some(pending) = commons_credentials::pending_switch(store)? else {
        return Ok(());
    };
    let context: SwitchContext = serde_json::from_value(pending.context)
        .map_err(|_| anyhow!("identity_switch_journal_invalid"))?;
    match pending.phase {
        SwitchPhase::Switching => {
            // The policy first: while the journal stands, a second crash
            // repeats both steps; once it is gone, nothing points back.
            let mut policy = ProjectPolicy::load(store)?;
            rebaseline_identity(&mut policy, &context.new, &context.old);
            policy.save(store)?;
            store.remove_daemon_file(LEGACY_INVITE_LINK_FILE)?;
            commons_credentials::roll_back_switch(store)?;
            tracing::info!("an unfinished legacy invite migration was rolled back");
        }
        SwitchPhase::Committed => {
            if load_link(store).is_none() {
                save_link(store, &context.link)?;
            }
            commons_credentials::retire_legacy(store)?;
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RememberedInvite {
    invite_subject_hash: String,
}

/// Save the subject hash of the invite this device just enrolled with.
/// Best effort, and a hash only: the code itself is a credential.
pub(crate) fn remember_invite_subject(store: &ConfigStore, code: &str) {
    let body = RememberedInvite {
        invite_subject_hash: invite_subject_hash(code),
    };
    let saved = serde_json::to_vec(&body)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| store.write_daemon_file(crate::config::INVITE_SUBJECT_FILE, &bytes));
    if saved.is_err() {
        tracing::debug!("the invite subject hash could not be saved");
    }
}

fn remembered_invite_subject(store: &ConfigStore) -> Option<String> {
    let bytes = store
        .read_daemon_file(crate::config::INVITE_SUBJECT_FILE)
        .ok()??;
    serde_json::from_slice::<RememberedInvite>(&bytes)
        .ok()
        .map(|r| r.invite_subject_hash)
}

/// The invite subject hash for this device, from, in order: the invite the
/// contributor pasted, the hash saved when this device enrolled, and the
/// issuer (`POST /v1/device/invite-subject`, signed by the legacy key).
/// `legacy_migration_invite_needed` when none of them can say, so a shell
/// can ask for the invite.
async fn resolve_invite_subject(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    legacy: &DeviceIdentity,
    pasted: Option<&str>,
) -> Result<String> {
    if let Some(raw) = pasted.filter(|r| !r.trim().is_empty()) {
        return invite_hash_from_paste(raw, cfg);
    }
    if let Some(hash) = remembered_invite_subject(store) {
        return Ok(hash);
    }
    match crate::issuer_client::fetch_device_invite_subject(cfg, legacy).await {
        Ok(Some(hash)) => Ok(hash),
        Ok(None) => bail!("legacy_migration_invite_needed"),
        Err(error) => Err(error),
    }
}

/// The whole migration. See the module docs for the order and why.
pub(crate) async fn migrate<P: AccountProvisioner>(
    shared: &DaemonShared,
    provisioner: &P,
    pasted_invite: Option<&str>,
) -> Result<serde_json::Value> {
    let store = &shared.store;
    if commons_credentials::pending_switch(store)?.is_some() {
        bail!("legacy_migration_pending");
    }
    let legacy_snapshot = commons_credentials::snapshot(store, Kind::Device)?;
    let Some(cfg) = store.load_config()? else {
        bail!("legacy_migration_not_enrolled");
    };
    if load_link(store).is_some() {
        bail!("legacy_migration_already_migrated");
    }
    if !offered_for(&cfg) {
        bail!("legacy_migration_not_applicable");
    }
    let legacy = DeviceIdentity::load_async(store)
        .await?
        .filter(|k| k.device_key_id == cfg.device_key_id)
        .ok_or_else(|| anyhow!("legacy_migration_device_key_missing"))?;
    let invite_hash = resolve_invite_subject(store, &cfg, &legacy, pasted_invite).await?;

    let staged = {
        let store = store.clone();
        tokio::task::spawn_blocking(move || commons_credentials::stage_device_key(&store))
            .await
            .map_err(|_| anyhow!("legacy_migration_unavailable"))??
    };
    let result = migrate_staged(
        shared,
        provisioner,
        &cfg,
        &legacy,
        &legacy_snapshot,
        &staged,
        &invite_hash,
    )
    .await;
    if result.is_err() {
        // A staged key that never became the identity is worth nothing.
        // Nothing else local has changed: `commit` put back its own writes.
        let store = store.clone();
        let _ = tokio::task::spawn_blocking(move || {
            commons_credentials::discard_staged_device_key(&store)
        })
        .await;
    }
    result
}

async fn migrate_staged<P: AccountProvisioner>(
    shared: &DaemonShared,
    provisioner: &P,
    cfg: &ContributorConfig,
    legacy: &DeviceIdentity,
    legacy_snapshot: &commons_credentials::Snapshot,
    staged: &DeviceIdentity,
    invite_hash: &str,
) -> Result<serde_json::Value> {
    let account = provisioner.provision(shared, cfg, staged).await?;
    if account.issuer_url != cfg.issuer_url || account.audience != cfg.audience {
        // Only the identity may change. A commons that now publishes another
        // issuer or audience is a destination change, which is not this.
        bail!("legacy_migration_commons_changed");
    }
    let mut new_cfg = cfg.clone();
    new_cfg.tenant_id = account.tenant_id.clone();
    new_cfg.instance_id = String::new();
    new_cfg.user_subject = staged.device_key_id.clone();
    new_cfg.device_key_id = staged.device_key_id.clone();

    let admission = shared
        .account_admission
        .read_now(&new_cfg, &account.access_token, Utc::now())
        .await;
    if admission != super::automatic_gate::AccountAdmission::Advertised {
        bail!("legacy_migration_admission_not_ready");
    }

    let response = link(&new_cfg, &account, legacy, &cfg.tenant_id, invite_hash).await?;
    verify_record(&response, legacy, &cfg.tenant_id, invite_hash, &account)?;

    let session = serde_json::to_vec(&crate::account_auth::AccountSession {
        access_token: account.access_token.clone(),
        expires_at: Utc::now() + chrono::Duration::seconds(account.expires_in_secs),
        account_id: account.account_id.to_string(),
    })
    .map_err(|_| anyhow!("legacy_migration_unavailable"))?;
    let context = SwitchContext {
        old: IdentityTerm::of_config(cfg),
        new: IdentityTerm::of_config(&new_cfg),
        link: StoredLink {
            record: response.record,
            server_signature: response.server_signature,
            migrated_at: Utc::now(),
            folders_kept: 0,
            automatic_grant_kept: false,
            notice_acknowledged: false,
        },
    };
    let rebaselined =
        super::run_blocking(|| commit(shared, legacy_snapshot, &new_cfg, &session, context))?;
    shared.publish(super::ipc::EVENT_STATUS_CHANGED, serde_json::json!({}));
    // Nothing identifying: the shell reads the notice from status.
    Ok(serde_json::json!({
        "migrated": true,
        "folders_kept": rebaselined.project_labels.len(),
        "automatic_grant_kept": rebaselined.automatic_grant,
    }))
}

/// `legacy_invite_migrate`: the contributor chose to move. `invite` is
/// optional -- the issuer is asked first, and the shell asks the contributor
/// for their invite only when the answer is `legacy_migration_invite_needed`.
pub(super) async fn handle_migrate(shared: &DaemonShared, req: &Request) -> Response {
    let invite = match req.params.get("invite") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(_) => return Response::err(req.id, ERR_BAD_PARAMS, "legacy_migration_invite_invalid"),
    };
    let api = match super::nearai_credential::api::CloudApi::live() {
        Ok(api) => api,
        Err(_) => {
            return Response::err(
                req.id,
                ERR_UNAVAILABLE,
                "legacy_migration_account_unavailable",
            );
        }
    };
    match migrate(shared, &NearAiLogin { api }, invite.as_deref()).await {
        Ok(value) => Response::ok(req.id, value),
        Err(error) => Response::err(req.id, ERR_UNAVAILABLE, label(&error)),
    }
}

/// `acknowledge_legacy_invite_migration`: the notice was shown.
pub(super) fn handle_acknowledge(shared: &DaemonShared, req: &Request) -> Response {
    let Some(mut link) = load_link(&shared.store) else {
        return Response::ok(req.id, serde_json::json!({ "acknowledged": false }));
    };
    if !link.notice_acknowledged {
        link.notice_acknowledged = true;
        if save_link(&shared.store, &link).is_err() {
            return Response::err(req.id, ERR_UNAVAILABLE, "legacy_migration_unavailable");
        }
        shared.publish(super::ipc::EVENT_STATUS_CHANGED, serde_json::json!({}));
    }
    Response::ok(req.id, serde_json::json!({ "acknowledged": true }))
}

#[cfg(test)]
#[path = "legacy_migration_tests.rs"]
mod tests;
