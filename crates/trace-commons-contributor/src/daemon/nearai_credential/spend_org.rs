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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PendingRevocation {
    pub(crate) workspace_id: String,
    pub(crate) key_id: String,
    pub(crate) recorded_at: DateTime<Utc>,
}

impl PendingRevocation {
    pub(crate) fn new(workspace_id: &str, key_id: &str) -> Self {
        Self {
            workspace_id: workspace_id.to_owned(),
            key_id: key_id.to_owned(),
            recorded_at: Utc::now(),
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
    record.pending_revocations.truncate(MAX_PENDING * 2);
    Ok(record)
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

pub(crate) fn record_pending(store: &ConfigStore, entries: &[PendingRevocation]) -> Result<()> {
    update(store, |record| {
        for entry in entries {
            if !record
                .pending_revocations
                .iter()
                .any(|held| held.same_key(entry))
            {
                record.pending_revocations.push(entry.clone());
            }
        }
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

/// Revoke every recorded key that is not the active one. Answers whether any
/// remain. Never revokes the active key: an entry naming it is dropped.
async fn sweep(shared: &DaemonShared, api: &CloudApi, tokens: SessionTokens) -> Result<bool> {
    let pending = load(&shared.store)?.pending_revocations;
    if pending.is_empty() {
        return Ok(false);
    }
    // Which key is active must be known for certain before anything is
    // revoked. A daemon whose memory and disk disagree answers an error
    // here, and nothing is touched.
    let active = current_key(shared)?.map(|key| PendingRevocation::of(&key));
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
        match api
            .revoke_inference_key(&tokens, &entry.workspace_id, &entry.key_id)
            .await
        {
            Ok(RevokeOutcome::Revoked | RevokeOutcome::AlreadyGone) => settled.push(entry),
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
    if load(&shared.store)?.pending_revocations.len() >= MAX_PENDING {
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
    let fresh = PendingRevocation::new(&minted.workspace_id, &minted.key_id);
    // Recorded before the new key is relied on. Whichever of the two is not
    // active when a sweep runs is revoked; the active one never is.
    if record_pending(&shared.store, &[PendingRevocation::of(&old), fresh.clone()]).is_err() {
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
