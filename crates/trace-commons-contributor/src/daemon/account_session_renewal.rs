//! The account session, renewed when it expired on a NEAR AI-provisioned
//! account.
//!
//! A native account session lives 12 hours and is never extended (see
//! [`super::nearai_onboarding::renew_account_session`]). A passkey account
//! signs in again through the browser. A NEAR AI-provisioned account has no
//! such sign-in: its only one is the provisioning ceremony, which `enroll`
//! refuses once the device is enrolled. So after 12 hours every
//! account-session route answered `account-session-required` for good, and
//! the macOS app's payout section, which needs one, never appeared.
//!
//! [`account_session`] is [`super::withdraw::account_session`] plus that
//! renewal: when the stored session is expired (never when it is absent),
//! the device is enrolled in a `nearai-` tenant and a NEAR AI login is
//! retained, the ceremony runs again and the fresh session is stored. The
//! contributor's own NEAR AI login is what signs them in, as the first time;
//! the device key gains no authority.
//!
//! # Bounded
//!
//! Renewals run one at a time, so the app's status read and org list, which
//! arrive together, renew once and both get the session. A renewal that
//! fails is not tried again for the same stored session and NEAR AI login
//! for [`RETRY_AFTER`]: each attempt rotates the NEAR AI refresh token and
//! counts against the commons' provisioning rate limit. Signing in to NEAR
//! AI again lifts the wait.
//!
//! # Not renewed
//!
//! - **An absent record.** Sign-out removes it; the person left, and is not
//!   signed back in behind their back.
//! - **Any other tenant**, including a passkey account: it signs in again
//!   with its passkey.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use super::commons_credentials::{self, Kind};
use super::ipc::DaemonShared;
use super::nearai_credential::api::CloudApi;
use crate::account_auth::LoadedAccountSession;
use crate::identity::DeviceIdentity;

/// How long a failed renewal of one stored record waits before the next.
pub(super) const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// One renewal at a time, across every store this process serves.
static RENEWING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Per state directory: the [`attempt_key`] a renewal last failed for, and
/// when.
static FAILED: Mutex<Option<HashMap<PathBuf, (String, Instant)>>> = Mutex::new(None);

/// Where a renewal's calls go: NEAR AI for the token exchange and the
/// enrolled commons for the ceremony.
pub(super) struct RenewVia<'a> {
    api: &'a CloudApi,
    #[cfg(test)]
    commons: Option<trace_commons_operator_client::Client>,
}

impl<'a> RenewVia<'a> {
    /// The live NEAR AI service, and the enrolled commons under the signup
    /// allowlist every NEAR AI ceremony uses.
    pub(super) fn live(api: &'a CloudApi) -> Self {
        Self {
            api,
            #[cfg(test)]
            commons: None,
        }
    }

    /// A stub commons on loopback, which the signup allowlist refuses.
    #[cfg(test)]
    pub(super) fn at(api: &'a CloudApi, commons: trace_commons_operator_client::Client) -> Self {
        Self {
            api,
            commons: Some(commons),
        }
    }

    fn commons(
        &self,
        cfg: &crate::config::ContributorConfig,
    ) -> anyhow::Result<trace_commons_operator_client::Client> {
        #[cfg(test)]
        if let Some(commons) = &self.commons {
            return Ok(commons.clone());
        }
        let allowed = super::account_onboarding::signup_allowlist(&cfg.ingest_url, &[])?;
        super::account_onboarding::client(&cfg.ingest_url, &allowed)
    }
}

/// The account session, renewed first if it expired and may be renewed
/// (module doc). `Ok(None)` exactly when [`super::withdraw::account_session`]
/// would answer it and no renewal produced one; `Err` only when the
/// credential store cannot be read.
pub(super) async fn account_session(
    shared: &DaemonShared,
    via: Option<&RenewVia<'_>>,
) -> anyhow::Result<Option<LoadedAccountSession>> {
    if let Some(session) = super::withdraw::account_session(shared)? {
        return Ok(Some(session));
    }
    let Some(via) = via else {
        return Ok(None);
    };
    let _turn = RENEWING.lock().await;
    // Renewed by another call while this one waited.
    if let Some(session) = super::withdraw::account_session(shared)? {
        return Ok(Some(session));
    }
    if renew(shared, via).await {
        super::withdraw::account_session(shared)
    } else {
        Ok(None)
    }
}

/// Whether a session was renewed and stored. Every refusal is `false`.
async fn renew(shared: &DaemonShared, via: &RenewVia<'_>) -> bool {
    let store = &shared.store;
    let Ok(Some(cfg)) = store.load_config() else {
        return false;
    };
    if !super::nearai_onboarding::is_near_ai_tenant_id(&cfg.tenant_id) {
        return false;
    }
    let Ok(Some((raw, expected))) =
        super::run_blocking(|| commons_credentials::load_with_snapshot(store, Kind::Account))
    else {
        // Absent: signed out, never renewed.
        return false;
    };
    // The stored session and the NEAR AI login, by digest: a renewal
    // stores a new session, and a new NEAR AI sign-in is a new login.
    if recently_failed(store.dir(), &attempt_key(shared, &raw)) {
        return false;
    }
    let Ok(previous) = serde_json::from_slice::<serde_json::Value>(&raw) else {
        return false;
    };
    let Ok(Some(identity)) = DeviceIdentity::load(store) else {
        return false;
    };
    if shared
        .settings
        .lock()
        .map(|settings| settings.near_ai_session.is_none())
        .unwrap_or(true)
    {
        return false;
    }
    let renewed = match via.commons(&cfg) {
        Ok(commons) => {
            super::nearai_onboarding::renew_account_session(
                shared, via.api, &commons, &cfg, &identity, previous, expected,
            )
            .await
        }
        Err(error) => Err(error),
    };
    match renewed {
        Ok(()) => {
            tracing::info!(target: "account_session", "account_session_renewed");
            true
        }
        Err(_) => {
            // Label only: the errors underneath can quote a remote body.
            tracing::warn!(target: "account_session", "account_session_renewal_failed");
            // Keyed after the attempt, so the NEAR AI rotation it made is
            // the login a retry would present.
            failed(store.dir(), attempt_key(shared, &raw));
            false
        }
    }
}

/// The stored session and the retained NEAR AI login an attempt ran with,
/// by digest. The login is named by when it was stored, never its token.
fn attempt_key(shared: &DaemonShared, raw: &[u8]) -> String {
    let near_ai = shared
        .settings
        .lock()
        .ok()
        .and_then(|settings| settings.near_ai_session.as_ref().map(|s| s.stored_at))
        .map(|at| at.to_rfc3339())
        .unwrap_or_default();
    let mut hash = Sha256::new();
    hash.update(b"trace-commons/account-session-renewal/v1");
    hash.update((raw.len() as u64).to_be_bytes());
    hash.update(raw);
    hash.update(near_ai.as_bytes());
    format!("sha256:{}", hex::encode(hash.finalize()))
}

fn recently_failed(dir: &std::path::Path, record: &str) -> bool {
    let failed = FAILED.lock().unwrap_or_else(PoisonError::into_inner);
    failed
        .as_ref()
        .and_then(|by_dir| by_dir.get(dir))
        .is_some_and(|(digest, at)| digest == record && at.elapsed() < RETRY_AFTER)
}

fn failed(dir: &std::path::Path, record: String) {
    FAILED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(dir.to_path_buf(), (record, Instant::now()));
}

#[cfg(test)]
#[path = "account_session_renewal_tests.rs"]
mod tests;
