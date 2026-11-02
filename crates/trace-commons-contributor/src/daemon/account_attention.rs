//! Whether this app's account needs attention (#1836 section 3.2a, fix F1).
//!
//! #1837 lets a NEAR AI sign-in through without the Commons check in two
//! cases no retry fixes: the stored account record is not JSON, or this
//! build's signup allowlist refuses the address the account was set up
//! with ([`super::account_session_renewal::FreshLogin::Unchecked`]). The
//! Commons session cannot renew in either, so uploads and account reads
//! fail while the app looks signed in. [`account_attention`] reports the
//! same two cases on every status build, signed in to NEAR AI or not, as
//! `status.account_state: "needs_attention"` with a fixed reason. Signing
//! in again does not fix it; unenrolling and setting up again does. The
//! daemon never unenrolls on its own.
//!
//! # Same cases as #1837
//!
//! The gates are [`super::account_session_renewal::check_fresh_login`]'s,
//! in its order: a NEAR AI tenant, a device key not revoked, a device that
//! did not join by one-time code, and a stored record (signed out is not
//! attention). It does not load the device identity, which is a second
//! credential-store read; the config is what says the device is enrolled.
//!
//! # Credential storage is read once per stored record
//!
//! Telling a malformed record from a good one reads the OS credential
//! store, which status never did before. The answer is kept per state
//! directory, keyed on [`commons_credentials::Snapshot::digest`]: the OS
//! entry a record names is immutable, so the answer cannot change until the
//! files do. Storage that does not answer (locked, a prompt) is not
//! attention, since it may pass, and is asked again after
//! [`STORAGE_RETRY`], not on every status read.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::account_session_renewal::{AccountRecordError, key_revoked, load_account_record};
use super::commons_credentials::{self, Kind};
use crate::config::{ConfigStore, ContributorConfig};
use crate::consent_copy::{ACCOUNT_ATTENTION_ADDRESS_REFUSED, ACCOUNT_ATTENTION_RECORD_UNREADABLE};

/// How long a credential-store read that did not answer is remembered.
const STORAGE_RETRY: Duration = Duration::from_secs(60);

/// Per state directory: the snapshot digest an answer was read under, the
/// answer, and when storage last failed to answer (`None` once it did).
type Remembered = ([u8; 32], Option<&'static str>, Option<Instant>);

static REMEMBERED: Mutex<Option<HashMap<PathBuf, Remembered>>> = Mutex::new(None);

/// The reason this app's account needs attention, or `None` when it does
/// not (module doc): [`ACCOUNT_ATTENTION_RECORD_UNREADABLE`] or
/// [`ACCOUNT_ATTENTION_ADDRESS_REFUSED`]. Blocking: it may read credential
/// storage.
pub(crate) fn account_attention(store: &ConfigStore) -> Option<&'static str> {
    let Ok(Some(cfg)) = store.load_config() else {
        return None;
    };
    if !super::nearai_onboarding::is_near_ai_tenant_id(&cfg.tenant_id)
        || key_revoked(store)
        || super::device_codes::joined_by_code(store)
    {
        return None;
    }
    let snapshot = commons_credentials::snapshot(store, Kind::Account).ok()?;
    if !snapshot.holds_record() {
        return None;
    }
    let digest = snapshot.digest();
    let dir = store.dir().to_path_buf();
    if let Some((seen, answer, storage_failed)) = remembered(&dir)
        && seen == digest
        && storage_failed.is_none_or(|at| at.elapsed() < STORAGE_RETRY)
    {
        return answer;
    }
    let (answer, storage_failed) = match super::run_blocking(|| load_account_record(store)) {
        Ok(None) => (None, None),
        Err(AccountRecordError::Malformed) => (Some(ACCOUNT_ATTENTION_RECORD_UNREADABLE), None),
        Err(AccountRecordError::Storage) => (None, Some(Instant::now())),
        Ok(Some(_)) => (
            address_refused(store, &cfg).then_some(ACCOUNT_ATTENTION_ADDRESS_REFUSED),
            None,
        ),
    };
    REMEMBERED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(dir, (digest, answer, storage_failed));
    answer
}

fn remembered(dir: &std::path::Path) -> Option<Remembered> {
    REMEMBERED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()?
        .get(dir)
        .copied()
}

/// Whether the enrolled address fails the check every NEAR AI ceremony
/// makes before it dials the commons (`RenewVia::commons`). No call is
/// made. Under `cfg(test)` a directory routed to a test's stub passes, as
/// `check_fresh_login_live` does.
fn address_refused(store: &ConfigStore, cfg: &ContributorConfig) -> bool {
    #[cfg(test)]
    if super::account_session_renewal::test_route(store).is_some() {
        return false;
    }
    let _ = store;
    super::account_onboarding::signup_allowlist(&cfg.ingest_url, &[])
        .and_then(|allowed| super::account_onboarding::client(&cfg.ingest_url, &allowed))
        .is_err()
}
