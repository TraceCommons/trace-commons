//! Which NEAR AI organization Private AI spends from.
//!
//! The spending organization is the organization the stored inference key
//! was minted in: the key carries it, the balance reads it
//! (`balance::stored_connection`), and billing opens it. So *choosing* a
//! spending organization is minting a key there and moving onto it -- there
//! is no separate pointer to keep in step.
//!
//! Most accounts have one organization, and for them nothing here shows: a
//! listing of one is not "multiple", and a listing that failed is not a
//! count. The list is read lazily, on the balance's lock and session (see
//! [`balance::read_organizations`]), never on the sign-in or the proxy's
//! key hand-off.
//!
//! A switch is ordered so the install always holds one working key:
//!
//! 1. mint a key in the chosen organization -- a failure here changes nothing;
//! 2. record both the old and the new key as revocation candidates;
//! 3. publish the new key, which moves the proxy onto it;
//! 4. revoke every recorded candidate that is not the active key.
//!
//! Step 4's rule -- never revoke the active key, whatever the record says --
//! is what makes step 2 safe to take early: if publishing fails, the new key
//! is the inactive one and is revoked instead, and the old key is kept. A
//! revoke that fails stays recorded and is retried on the next listing.
//!
//! The record lives in its own daemon file rather than in the settings
//! document. Preference writers save the in-memory settings snapshot they
//! hold, and a field written beside them on disk would be reverted by the
//! next unrelated toggle. Everything in it is a service id, never a secret,
//! and nothing here logs an id or a name.

use super::api::{
    AccountOrganization, CloudApi, MintedKey, OrganizationChoice, RevokeOutcome, is_path_segment,
};
use super::balance::{self, BalanceReport};
use super::loopback::SessionTokens;
use crate::config::ConfigStore;
use crate::daemon::ipc::DaemonShared;
use crate::daemon::settings::NearAiInferenceCredential;
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[cfg(test)]
#[path = "spend_org_tests.rs"]
mod tests;

/// The daemon file holding the explicit choice and pending revocations.
pub const SPEND_ORG_FILE: &str = "near-ai-spend-org.json";

/// The per-install salt [`identity_digest`] hashes with. Local only.
pub const IDENTITY_SALT_FILE: &str = "near-ai-identity-salt";

/// A digest of a NEAR AI user id, salted per install: `sha256:` and hex.
/// Local only -- never logged, never sent -- so the record can tell one
/// identity's revocations from another's without holding the id.
pub(crate) fn identity_digest(store: &ConfigStore, user_id: &str) -> Result<String> {
    use sha2::Digest as _;
    let salt = match store
        .read_daemon_file(IDENTITY_SALT_FILE)
        .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?
    {
        Some(salt) if salt.len() == 32 => salt,
        // A salt of the wrong length is damage. Every tag made with the old
        // salt is now meaningless, so they are removed first -- the entries
        // become untagged, tried with the current session and dropped on a
        // `403`, as before tags -- and only then is a new salt made. Kept,
        // the damaged salt would park every tagged entry for good.
        Some(_) => {
            update(store, |record| {
                for entry in &mut record.pending_revocations {
                    entry.identity = None;
                }
                record.session_identity = None;
            })?;
            crate::daemon::diagnostics_log::record_labels(
                tracing::Level::WARN,
                "near_ai_spend_org_identity_salt_replaced",
                &[("reason", "damaged")],
            );
            new_salt(store)?
        }
        None => new_salt(store)?,
    };

    let mut hasher = sha2::Sha256::new();
    hasher.update(&salt);
    hasher.update(user_id.as_bytes());
    Ok(format!("sha256:{}", hex::encode(hasher.finalize())))
}

fn new_salt(store: &ConfigStore) -> Result<Vec<u8>> {
    let mut salt = vec![0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut salt)
        .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?;
    store
        .write_daemon_file(IDENTITY_SALT_FILE, &salt)
        .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?;
    Ok(salt)
}

/// The digest of the identity `tokens` belong to, read from NEAR AI, or
/// `None` when it cannot be read.
pub(crate) async fn read_identity(
    store: &ConfigStore,
    api: &CloudApi,
    tokens: &SessionTokens,
) -> Option<String> {
    let user_id = api.current_user_id(tokens).await.ok()?;
    identity_digest(store, &user_id).ok()
}

/// The identity of the stored login, recorded once and bound to the stored
/// inference key: a sign-in writes it, a switch moves it to the key it
/// minted (a switch never changes the identity), and a forget removes the
/// file. Valid only while `key_id` is the stored key.
///
/// Only a sign-in and a switch, which hold the login and the key they
/// record together, write it outright. A read made elsewhere
/// ([`identity_of`]) records only into an empty record and only while the
/// key is still the stored one: its session tokens were taken before it
/// looked at the key, and a sign-in landing between the two must not have
/// its identity overwritten with the replaced login's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SessionIdentity {
    pub(crate) key_id: String,
    pub(crate) digest: String,
}

/// The recorded identity of the login whose key is `key_id`, if any.
fn recorded_identity(record: &SpendOrgRecord, key_id: &str) -> Option<String> {
    record
        .session_identity
        .as_ref()
        .filter(|identity| identity.key_id == key_id)
        .map(|identity| identity.digest.clone())
}

/// Record `digest` as the identity of the login whose key is `key_id`,
/// outright: for a sign-in and a switch, which hold both. Best effort: a
/// lost write is read again.
pub(crate) fn record_identity(store: &ConfigStore, key_id: &str, digest: &str) {
    let recorded = update(store, |record| {
        record.session_identity = Some(SessionIdentity {
            key_id: key_id.to_owned(),
            digest: digest.to_owned(),
        });
    });
    if recorded.is_err() {
        tracing::warn!(target: "near_ai_spend_org", "near_ai_spend_org_identity_unrecorded");
    }
}

/// The identity of the stored login whose key is `key_id`: the recorded
/// one, else read from NEAR AI with `tokens`. The read is recorded only
/// into an empty record, and only if `key_id` is still the stored key
/// after it ([`SessionIdentity`]); otherwise it serves this call alone.
async fn identity_of(
    shared: &DaemonShared,
    api: &CloudApi,
    tokens: &SessionTokens,
    key_id: &str,
) -> Option<String> {
    let store = &shared.store;
    if let Some(recorded) = load(store)
        .ok()
        .and_then(|record| recorded_identity(&record, key_id))
    {
        return Some(recorded);
    }
    let digest = read_identity(store, api, tokens).await?;
    let still_stored = current_key(shared)
        .ok()
        .flatten()
        .is_some_and(|key| key.key_id == key_id);
    if still_stored {
        let _ = update(store, |record| {
            if record.session_identity.is_none() {
                record.session_identity = Some(SessionIdentity {
                    key_id: key_id.to_owned(),
                    digest: digest.clone(),
                });
            }
        });
    }
    Some(digest)
}

/// More pending revocations than this and a switch is refused: something is
/// persistently refusing revokes, and minting further keys would only add to
/// what has to be cleaned up.
const MAX_PENDING: usize = 16;

/// How long the service may take to accept a newly minted key. Its own
/// documentation for `POST /v1/workspaces/{id}/api-keys`: "The key may take
/// up to 10 seconds to become active for authentication due to internal
/// caching." The old key keeps serving until then, so the proxy is not moved
/// onto a key that would be refused.
pub const KEY_PROPAGATION: Duration = Duration::from_secs(10);

/// The IPC error labels a switch can answer with.
pub const LABEL_SPEND_ORG_INVALID: &str = "near_ai_spend_org_invalid";
pub const LABEL_SPEND_ORG_NO_SESSION: &str = "near_ai_spend_org_no_session";
pub const LABEL_SPEND_ORG_SESSION_EXPIRED: &str = "near_ai_spend_org_session_expired";
pub const LABEL_SPEND_ORG_NO_KEY: &str = "near_ai_spend_org_no_key";
pub const LABEL_SPEND_ORG_UNKNOWN: &str = "near_ai_spend_org_unknown";
pub const LABEL_SPEND_ORG_REFUSED: &str = "near_ai_spend_org_refused";
pub const LABEL_SPEND_ORG_BUSY: &str = "near_ai_spend_org_busy";
pub const LABEL_SPEND_ORG_UNAVAILABLE: &str = "near_ai_spend_org_unavailable";

/// The listing's `state` labels.
pub const LABEL_ORGANIZATIONS_LISTED: &str = "listed";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct SpendOrgRecord {
    /// The organization the contributor chose. Explicit only while it is
    /// also the stored key's organization: a later sign-in that minted
    /// elsewhere makes it a stale preference, not a choice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) chosen_organization_id: Option<String>,
    /// Keys this install minted that should no longer exist.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) pending_revocations: Vec<PendingRevocation>,
    /// The stored login's identity ([`SessionIdentity`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) session_identity: Option<SessionIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingRevocation {
    pub(crate) workspace_id: String,
    pub(crate) key_id: String,
    pub(crate) recorded_at: DateTime<Utc>,
    /// The NEAR AI identity that recorded it, as [`identity_digest`]: only
    /// that identity's session can revoke the key, so the sweep runs it with
    /// no other (spec 2026-11-02 B8). `None` for an entry recorded before
    /// tags, or when the identity could not be read; the sweep tries those
    /// with the current session and drops one answered `403`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) identity: Option<String>,
}

impl PendingRevocation {
    pub(crate) fn new(workspace_id: &str, key_id: &str) -> Self {
        Self {
            workspace_id: workspace_id.to_owned(),
            key_id: key_id.to_owned(),
            recorded_at: Utc::now(),
            identity: None,
        }
    }

    fn tagged(mut self, identity: Option<&str>) -> Self {
        self.identity = identity.map(str::to_owned);
        self
    }

    /// Whether the sweep may run this entry with a session of `identity`:
    /// an untagged entry, or one tagged with that identity.
    fn belongs_to(&self, identity: Option<&str>) -> bool {
        match (&self.identity, identity) {
            (None, _) => true,
            (Some(tag), Some(current)) => tag == current,
            (Some(_), None) => false,
        }
    }

    fn of(key: &NearAiInferenceCredential) -> Self {
        Self::new(&key.workspace_id, &key.key_id)
    }

    fn same_key(&self, other: &Self) -> bool {
        self.workspace_id == other.workspace_id && self.key_id == other.key_id
    }
}

/// Read the record. Absent is empty; unreadable is an error, so a caller
/// fails closed rather than believing an empty one. Ids that could not be
/// a path segment are dropped.
pub(crate) fn load(store: &ConfigStore) -> Result<SpendOrgRecord> {
    let Some(bytes) = store
        .read_daemon_file(SPEND_ORG_FILE)
        .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?
    else {
        return Ok(SpendOrgRecord::default());
    };
    if bytes.len() > 16 * 1024 {
        bail!(LABEL_SPEND_ORG_UNAVAILABLE)
    }
    let mut record: SpendOrgRecord =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?;
    record.chosen_organization_id = record
        .chosen_organization_id
        .filter(|id| is_path_segment(id));
    record
        .pending_revocations
        .retain(|entry| is_path_segment(&entry.workspace_id) && is_path_segment(&entry.key_id));
    // A record written past its size by an older build: oldest first.
    // `record_pending` keeps it in bounds from here on.
    trim(&mut record.pending_revocations, None);
    Ok(record)
}

/// Bring the entries within `2 * MAX_PENDING`, dropping another identity's
/// parked entries first, oldest first, then the oldest of the rest. `keep`
/// is the identity whose entries go last; with none known, the oldest go
/// first. A dropped entry's key stays at NEAR AI, so each trim is logged.
fn trim(entries: &mut Vec<PendingRevocation>, keep: Option<&str>) {
    let mut dropped = false;
    while entries.len() > MAX_PENDING * 2 {
        let index = keep
            .and_then(|keep| {
                entries
                    .iter()
                    .position(|entry| !entry.belongs_to(Some(keep)))
            })
            .unwrap_or(0);
        entries.remove(index);
        dropped = true;
    }
    if dropped {
        crate::daemon::diagnostics_log::record_labels(
            tracing::Level::WARN,
            "near_ai_spend_org_revocations_trimmed",
            &[("reason", "over_capacity")],
        );
    }
}

/// Change the record under the credential commit lock, so two writers in
/// this process or another cannot lose each other's entries.
fn update(store: &ConfigStore, change: impl FnOnce(&mut SpendOrgRecord)) -> Result<()> {
    let locks = super::session::coordination(store.dir())?;
    let _commit = locks
        .commit
        .lock()
        .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?;
    let mut record = load(store)?;
    change(&mut record);
    let bytes =
        serde_json::to_vec_pretty(&record).map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?;
    store
        .write_daemon_file(SPEND_ORG_FILE, &bytes)
        .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))
}

/// The remembered choice, if one can be read. The sign-in passes it as a
/// preference so signing in again lands where the contributor chose.
pub(crate) fn chosen_organization(store: &ConfigStore) -> Option<String> {
    load(store)
        .ok()
        .and_then(|record| record.chosen_organization_id)
}

/// Whether a sign-in landed in another NEAR AI account (owner, 2026-11-02:
/// signing in to a different account with a different NEAR AI organization
/// switches to its organization if it has one, or asks).
///
/// The signal, when both are known, is the identity: the recorded identity
/// of the login being replaced and that of the new one
/// ([`identity_digest`]), compared in memory and never logged. When either
/// is unknown, it is the organization listing the sign-in's mint reads
/// anyway ([`super::api::CloudApi::mint_inference_key_listing`]): a choice is
/// remembered and the account does not list that organization at all --
/// most often a GitHub or Google login beside a wallet one. An organization
/// the account still lists but that is inactive for now is the same
/// account, and the choice stands.
pub(crate) fn account_changed(
    remembered: Option<&str>,
    remembered_listed: bool,
    previous_identity: Option<&str>,
    new_identity: Option<&str>,
) -> bool {
    remembered.is_some()
        && match (previous_identity, new_identity) {
            (Some(previous), Some(new)) => previous != new,
            _ => !remembered_listed,
        }
}

/// The recorded identity of the stored login, whatever key it is bound
/// to. Only for [`account_changed`] at a sign-in, where a wrong answer at
/// worst asks the chooser again or keeps a preference the mint falls back
/// from.
pub(crate) fn last_identity(store: &ConfigStore) -> Option<String> {
    load(store)
        .ok()?
        .session_identity
        .map(|identity| identity.digest)
}

/// After a sign-in to another account stored its key: forget the choice
/// made in the old one, so the new account's organization stands. With one
/// organization nothing is asked; with several,
/// [`OrganizationsReport::prompt`] asks again, since no choice is recorded
/// for this account.
///
/// Pending revocations are kept: a missing organization is not proof of
/// another account (it may have been deleted or left in the same one), and
/// those keys may still be revocable. An entry tagged with the old identity
/// is parked: never sent with the new session, and counted toward neither
/// `revoke_pending` nor the [`MAX_PENDING`] limit, until that identity signs
/// in again. An untagged entry is tried with the new session, and dropped
/// on a `403` ([`RevokeOutcome::NotThisAccount`]). Best effort: a record that
/// cannot be written keeps the old preference, which the mint already falls
/// back from.
pub(crate) fn forget_other_account(
    store: &ConfigStore,
    remembered: Option<&str>,
    remembered_listed: bool,
    previous_identity: Option<&str>,
    new_identity: Option<&str>,
) {
    if !account_changed(
        remembered,
        remembered_listed,
        previous_identity,
        new_identity,
    ) {
        return;
    }
    if update(store, |record| record.chosen_organization_id = None).is_err() {
        tracing::warn!(target: "near_ai_spend_org", "near_ai_spend_org_choice_not_cleared");
    }
}

/// Forget the record. The caller holds the credential commit lock: this is
/// the forget path's, which removes the session every revoke needs, and the
/// forget sentence already says keys are not revoked at NEAR AI. Best
/// effort -- a record left behind only names keys, and a forget must not fail
/// over it.
pub(crate) fn clear_locked(store: &ConfigStore) {
    match std::fs::remove_file(store.daemon_path(SPEND_ORG_FILE)) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            tracing::warn!(target: "near_ai_spend_org", "near_ai_spend_org_record_not_cleared")
        }
    }
}

/// Record `entries`. An entry already held keeps its place; one held
/// untagged takes the incoming tag, so a later sweep under another
/// identity parks it instead of sending it.
pub(crate) fn record_pending(store: &ConfigStore, entries: &[PendingRevocation]) -> Result<()> {
    update(store, |record| {
        for entry in entries {
            match record
                .pending_revocations
                .iter_mut()
                .find(|held| held.same_key(entry))
            {
                Some(held) if held.identity.is_none() => held.identity = entry.identity.clone(),
                Some(_) => {}
                None => record.pending_revocations.push(entry.clone()),
            }
        }
        let keep = entries.iter().find_map(|entry| entry.identity.clone());
        trim(&mut record.pending_revocations, keep.as_deref());
    })
}

fn remember_choice(store: &ConfigStore, organization_id: &str) -> Result<()> {
    update(store, |record| {
        record.chosen_organization_id = Some(organization_id.to_owned());
    })
}

/// The stored key, as the running daemon holds it, checked against disk.
fn current_key(shared: &DaemonShared) -> Result<Option<NearAiInferenceCredential>> {
    let locks = super::session::coordination(shared.store.dir())?;
    let commit = locks
        .commit
        .lock()
        .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?;
    Ok(super::session::runtime_snapshot(shared, &commit)?.near_ai_inference)
}

// ---- Listing ----

/// What a listing found.
#[derive(Debug, Clone)]
pub enum OrganizationsReport {
    /// One of the balance's not-known states: `no_session`,
    /// `session_expired`, `no_organization` or `unavailable`.
    NotListed(&'static str),
    Listed {
        organizations: Vec<AccountOrganization>,
        /// The stored key's organization, `None` with no key.
        current: Option<String>,
        /// The contributor chose `current` themselves.
        explicit: bool,
        /// A choice has been recorded for this sign-in, wherever the key now
        /// is. Any choice ends the prompt for good (owner, on #1590); only
        /// forgetting the sign-in, which clears the record, brings it back.
        chosen: bool,
        /// A replaced key is still waiting to be revoked.
        revoke_pending: bool,
    },
}

impl OrganizationsReport {
    #[must_use]
    pub fn state(&self) -> &'static str {
        match self {
            Self::NotListed(state) => state,
            Self::Listed { .. } => LABEL_ORGANIZATIONS_LISTED,
        }
    }

    /// Several active organizations, counted from a complete listing.
    #[must_use]
    pub fn multiple(&self) -> bool {
        matches!(self, Self::Listed { organizations, .. } if organizations.len() >= 2)
    }

    /// Whether the Private AI screen asks the contributor to choose: several
    /// organizations, a key spending from one of them, and no choice ever
    /// recorded for this sign-in. Never for one organization, never for a
    /// listing that failed, and never once anything has been chosen -- even
    /// if the key later spends from another organization.
    #[must_use]
    pub fn prompt(&self) -> bool {
        self.multiple()
            && matches!(
                self,
                Self::Listed {
                    current: Some(_),
                    chosen: false,
                    ..
                }
            )
    }

    /// The IPC body. Every key is present in every state, so a shell never
    /// has to tell an old daemon from a state that has nothing to say.
    #[must_use]
    pub fn to_value(&self) -> serde_json::Value {
        let (organizations, current, explicit, chosen, revoke_pending) = match self {
            Self::Listed {
                organizations,
                current,
                explicit,
                chosen,
                revoke_pending,
            } => (
                organizations.as_slice(),
                current.as_deref(),
                *explicit,
                *chosen,
                *revoke_pending,
            ),
            Self::NotListed(_) => (&[][..], None, false, false, false),
        };
        let current_name = current.and_then(|id| {
            organizations
                .iter()
                .find(|organization| organization.id == id)
                .map(|organization| organization.name.as_str())
        });
        serde_json::json!({
            "state": self.state(),
            "organizations": organizations
                .iter()
                .map(|organization| serde_json::json!({
                    "id": organization.id,
                    "name": organization.name,
                    "role": organization.role,
                    "current": Some(organization.id.as_str()) == current,
                }))
                .collect::<Vec<_>>(),
            "current_organization_id": current,
            "explicit": explicit,
            "chosen": chosen,
            "multiple": self.multiple(),
            "prompt": self.prompt(),
            "revoke_pending": revoke_pending,
            "view": crate::private_inference_copy::spend_org_view(current_name),
        })
    }
}

/// The live listing.
pub async fn organizations(shared: &DaemonShared, fresh: bool) -> OrganizationsReport {
    match CloudApi::live() {
        Ok(api) => organizations_with(shared, &api, fresh).await,
        Err(_) => OrganizationsReport::NotListed(balance::LABEL_BALANCE_UNAVAILABLE),
    }
}

/// [`organizations`] against an explicit client.
pub(crate) async fn organizations_with(
    shared: &DaemonShared,
    api: &CloudApi,
    fresh: bool,
) -> OrganizationsReport {
    let max_age = if fresh {
        balance::ORGANIZATIONS_FRESH_FLOOR
    } else {
        balance::ORGANIZATIONS_TTL
    };
    let (organizations, current) = match balance::read_organizations(shared, api, max_age).await {
        Ok(listed) => listed,
        Err(report) => return OrganizationsReport::NotListed(not_listed(report)),
    };
    // An unreadable record is no choice and no claim about revocations; the
    // sweep below refuses to act on it.
    let record = load(&shared.store).ok();
    let recorded = record
        .as_ref()
        .and_then(|record| record.chosen_organization_id.as_deref());
    let explicit = recorded.is_some_and(|chosen| current.as_deref() == Some(chosen));
    // An unreadable record counts as no choice: the prompt only asks, and
    // asking again is the safe side of not knowing.
    let chosen = recorded.is_some();
    let revoke_pending = match &record {
        // Only another identity's parked keys (or the active one): there is
        // nothing this session can revoke, so no session is taken and no
        // call is made.
        Some(record) if only_parked(shared, record) => false,
        Some(record) if !record.pending_revocations.is_empty() => {
            balance::with_management_session(shared, api, |tokens| sweep(shared, api, tokens))
                .await
                .unwrap_or(true)
        }
        Some(_) => false,
        None => false,
    };
    OrganizationsReport::Listed {
        organizations,
        current,
        explicit,
        chosen,
        revoke_pending,
    }
}

fn not_listed(report: BalanceReport) -> &'static str {
    match report {
        BalanceReport::Known(_) => balance::LABEL_BALANCE_UNAVAILABLE,
        other => other.state(),
    }
}

/// Whether every pending entry is tagged with an identity other than the
/// stored login's recorded one: nothing a sweep could send. An entry naming
/// the active key is not parked: only a sweep settles it.
fn only_parked(shared: &DaemonShared, record: &SpendOrgRecord) -> bool {
    let Ok(Some(active)) = current_key(shared) else {
        return false;
    };
    let Some(identity) = recorded_identity(record, &active.key_id) else {
        return false;
    };
    let active = PendingRevocation::of(&active);
    !record.pending_revocations.is_empty()
        && record
            .pending_revocations
            .iter()
            .all(|entry| !entry.same_key(&active) && !entry.belongs_to(Some(&identity)))
}

/// Revoke every recorded key that is not the active one. Answers whether any
/// remain. Never revokes the active key: an entry naming it is dropped.
///
/// It reads the active key and the record again, under the commit lock,
/// rather than taking them from the caller: the management session it runs
/// in may have re-exchanged, and a sign-in may have landed, since.
async fn sweep(shared: &DaemonShared, api: &CloudApi, tokens: SessionTokens) -> Result<bool> {
    let pending = load(&shared.store)?.pending_revocations;
    if pending.is_empty() {
        return Ok(false);
    }
    // Which key is active must be known for certain before anything is
    // revoked. A daemon whose memory and disk disagree answers an error
    // here, and nothing is touched.
    let active_key = current_key(shared)?;
    let active = active_key.as_ref().map(PendingRevocation::of);
    // The identity is needed only to tell a tagged entry's owner.
    let needs_identity = pending.iter().any(|entry| {
        entry.identity.is_some() && !active.as_ref().is_some_and(|active| active.same_key(entry))
    });
    let identity = match (&active_key, needs_identity) {
        (_, false) => None,
        (Some(key), true) => identity_of(shared, api, &tokens, &key.key_id).await,
        (None, true) => read_identity(&shared.store, api, &tokens).await,
    };
    let mut settled = Vec::new();
    let mut remaining = false;
    for entry in pending {
        if active
            .as_ref()
            .is_some_and(|active| active.same_key(&entry))
        {
            settled.push(entry);
            continue;
        }
        // Another identity's key stays parked: this session cannot revoke
        // it, and it is not this identity's to report. With the identity
        // unread, a tagged entry waits for a sweep that can read it.
        if !entry.belongs_to(identity.as_deref()) {
            remaining |= identity.is_none();
            continue;
        }
        match api
            .revoke_inference_key(&tokens, &entry.workspace_id, &entry.key_id)
            .await
        {
            Ok(RevokeOutcome::Revoked | RevokeOutcome::AlreadyGone) => settled.push(entry),
            // Another account's key: this session can never revoke it, and
            // keeping it would hold `revoke_pending` and the limit forever.
            // The key stays at NEAR AI in that account, as after a forget.
            Ok(RevokeOutcome::NotThisAccount) => {
                crate::daemon::diagnostics_log::record_labels(
                    tracing::Level::WARN,
                    "near_ai_spend_org_revocation_dropped",
                    &[("reason", "not_this_account")],
                );
                settled.push(entry);
            }
            Err(_) => {
                tracing::warn!(target: "near_ai_spend_org", "near_ai_key_revoke_deferred");
                remaining = true;
            }
        }
    }
    update(&shared.store, |record| {
        record
            .pending_revocations
            .retain(|held| !settled.iter().any(|done| done.same_key(held)));
    })?;
    Ok(remaining)
}

// ---- Switching ----

/// What a switch did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectOutcome {
    /// A new key was published in the chosen organization. `false` when the
    /// chosen organization was already the current one.
    pub switched: bool,
    /// The replaced key could not be revoked yet; it is recorded and retried.
    pub revoke_pending: bool,
}

/// The live switch.
pub async fn select(shared: &DaemonShared, target: &str) -> Result<SelectOutcome> {
    let api = CloudApi::live().map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?;
    select_with(shared, &api, target, KEY_PROPAGATION).await
}

/// [`select`] against an explicit client and propagation wait.
pub(crate) async fn select_with(
    shared: &DaemonShared,
    api: &CloudApi,
    target: &str,
    propagation: Duration,
) -> Result<SelectOutcome> {
    if !is_path_segment(target) {
        bail!(LABEL_SPEND_ORG_INVALID)
    }
    // A sign-in in flight is about to replace the key; switching under it
    // would mint a key it then orphans.
    if matches!(
        super::ceremony::attempt_status(shared.store.dir()),
        Some("starting" | "waiting_for_browser")
    ) {
        bail!(LABEL_SPEND_ORG_BUSY)
    }
    let switched = balance::with_management_session(shared, api, |tokens| {
        switch_step(shared, api, target, tokens, propagation)
    })
    .await
    .map_err(|error| anyhow!(switch_label(&error)))?;
    if !switched {
        return Ok(SelectOutcome {
            switched: false,
            revoke_pending: false,
        });
    }
    // The published key reaches the proxy here, before the old key is
    // revoked: absorbing the change advances the generation that restarts
    // IronWire on the new key.
    shared.reconcile_private_inference().await;
    let revoke_pending =
        balance::with_management_session(shared, api, |tokens| sweep(shared, api, tokens))
            .await
            .unwrap_or(true);
    Ok(SelectOutcome {
        switched: true,
        revoke_pending,
    })
}

/// Map whatever a step raised onto a label a shell can switch on. The
/// service's own words never reach one.
fn switch_label(error: &anyhow::Error) -> &'static str {
    match error.to_string().as_str() {
        LABEL_SPEND_ORG_INVALID => LABEL_SPEND_ORG_INVALID,
        LABEL_SPEND_ORG_NO_KEY => LABEL_SPEND_ORG_NO_KEY,
        LABEL_SPEND_ORG_UNKNOWN => LABEL_SPEND_ORG_UNKNOWN,
        LABEL_SPEND_ORG_REFUSED => LABEL_SPEND_ORG_REFUSED,
        LABEL_SPEND_ORG_BUSY => LABEL_SPEND_ORG_BUSY,
        "near_ai_credential_session_missing" => LABEL_SPEND_ORG_NO_SESSION,
        "near_ai_credential_session_expired" => LABEL_SPEND_ORG_SESSION_EXPIRED,
        _ => LABEL_SPEND_ORG_UNAVAILABLE,
    }
}

/// One attempt at the switch, run under the management session. Safe to
/// repeat after a re-exchange: a repeat that finds the key already in the
/// target organization only records the choice.
async fn switch_step(
    shared: &DaemonShared,
    api: &CloudApi,
    target: &str,
    tokens: SessionTokens,
    propagation: Duration,
) -> Result<bool> {
    let old = current_key(shared)?.ok_or_else(|| anyhow!(LABEL_SPEND_ORG_NO_KEY))?;
    if old.organization_id == target {
        remember_choice(&shared.store, target)?;
        return Ok(false);
    }
    // Tagged with the identity that mints, so the sweep never runs these
    // with another's session. Only this identity's entries (and untagged
    // ones) count toward the limit: another's parked entries cannot jam it.
    let identity = identity_of(shared, api, &tokens, &old.key_id).await;
    // With the identity unknown, every entry counts: the limit fails closed.
    let actionable = load(&shared.store)?
        .pending_revocations
        .iter()
        .filter(|entry| identity.is_none() || entry.belongs_to(identity.as_deref()))
        .count();
    if actionable >= MAX_PENDING {
        bail!(LABEL_SPEND_ORG_BUSY)
    }
    let key_name = super::ceremony::key_name(&super::loopback::random_state()?);
    let minted = match api
        .mint_inference_key_in(&tokens, &key_name, OrganizationChoice::Exactly(target))
        .await
    {
        Ok(minted) => minted,
        Err(error) => {
            return Err(match error.to_string().as_str() {
                // Passed through so the session helper can re-exchange once.
                "near_ai_credential_session_expired" => error,
                "near_ai_credential_no_organization" => anyhow!(LABEL_SPEND_ORG_UNKNOWN),
                "near_ai_credential_refused"
                | "near_ai_credential_no_workspace"
                | "near_ai_credential_not_returned" => anyhow!(LABEL_SPEND_ORG_REFUSED),
                _ => anyhow!(LABEL_SPEND_ORG_UNAVAILABLE),
            });
        }
    };
    let fresh =
        PendingRevocation::new(&minted.workspace_id, &minted.key_id).tagged(identity.as_deref());
    // Recorded before the new key is relied on. Whichever of the two is not
    // active when a sweep runs is revoked; the active one never is.
    let old_entry = PendingRevocation::of(&old).tagged(identity.as_deref());
    if record_pending(&shared.store, &[old_entry, fresh.clone()]).is_err() {
        // Nothing recorded means nothing would ever revoke the new key, so
        // it is revoked now, and the switch fails having changed nothing.
        let _ = api
            .revoke_inference_key(&tokens, &fresh.workspace_id, &fresh.key_id)
            .await;
        bail!(LABEL_SPEND_ORG_UNAVAILABLE)
    }
    tokio::time::sleep(propagation).await;
    if publish(shared, &old, minted).await.is_err() {
        // Nothing was published. The new key is revoked now; the old key is
        // no business of this switch any more -- it is either still active
        // or was replaced by something else (a sign-in), which owns it.
        let revoked = api
            .revoke_inference_key(&tokens, &fresh.workspace_id, &fresh.key_id)
            .await
            .is_ok();
        let old = PendingRevocation::of(&old);
        let _ = update(&shared.store, |record| {
            record
                .pending_revocations
                .retain(|held| !held.same_key(&old) && !(revoked && held.same_key(&fresh)));
        });
        bail!(LABEL_SPEND_ORG_UNAVAILABLE)
    }
    // Best effort: a choice that did not save leaves the prompt up, which is
    // the honest state, and the switch itself has happened.
    if remember_choice(&shared.store, target).is_err() {
        tracing::warn!(target: "near_ai_spend_org", "near_ai_spend_org_choice_unsaved");
    }
    // Same login, new key: the recorded identity moves to it.
    if let Some(identity) = identity.as_deref() {
        record_identity(&shared.store, &fresh.key_id, identity);
    }
    Ok(true)
}

/// Publish the new key beside the stored session, only if the stored key is
/// still the one this switch replaced.
async fn publish(
    shared: &DaemonShared,
    old: &NearAiInferenceCredential,
    minted: MintedKey,
) -> Result<()> {
    let expected = {
        let locks = super::session::coordination(shared.store.dir())?;
        let commit = locks
            .commit
            .lock()
            .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))?;
        super::session::runtime_snapshot(shared, &commit)?
    };
    if expected
        .near_ai_inference
        .as_ref()
        .is_none_or(|key| key.key_id != old.key_id || key.workspace_id != old.workspace_id)
    {
        bail!("near_ai_credential_session_changed")
    }
    let inference = NearAiInferenceCredential {
        key: minted.key,
        key_id: minted.key_id,
        key_prefix: minted.key_prefix,
        organization_id: minted.organization_id,
        workspace_id: minted.workspace_id,
        minted_at: Utc::now(),
    };
    let session = expected.near_ai_session.clone();
    let store = shared.store.clone();
    tokio::task::spawn_blocking(move || {
        crate::daemon::cloud_credential_lifecycle::native(&store)?.replace(
            &expected,
            Some(inference),
            session,
            || Ok(()),
            |_| {
                super::ceremony::record_credential_change(store.dir());
                Ok(())
            },
        )
    })
    .await
    .map_err(|_| anyhow!(LABEL_SPEND_ORG_UNAVAILABLE))??;
    Ok(())
}
