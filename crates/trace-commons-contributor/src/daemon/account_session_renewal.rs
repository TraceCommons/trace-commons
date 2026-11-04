//! The account session, renewed when it expired on a NEAR AI-provisioned
//! account.
//!
//! A native account session lives 12 hours and is never extended (see
//! [`super::nearai_onboarding::renew_account_session`]). A NEAR
//! AI-provisioned account has no other sign-in: its only one is the provisioning ceremony, which `enroll`
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
//! # Every feature
//!
//! [`account_session_live`] is the entry point every daemon method that
//! presents the account session uses: payout link, withdrawal, Trace
//! Credit, activity missions, the inference connection, the public run,
//! account contributions, and `account_session_status`, so the app's
//! signed-in state follows a renewal. Automatic upload's admission read
//! (`account_admission`, the gate's R3) runs on the supervisor's pass,
//! which never waits on a ceremony; it asks for one off the pass
//! ([`renew_off_pass`]) and the next pass reads the renewed session.
//!
//! A read that only validates a session already loaded (that it is still
//! the same account after a call) never renews: a renewal there would adopt
//! a session the call was not made with.
//!
//! # Bounded
//!
//! Renewals run one at a time across every caller, so the app's status
//! read, org list and Home reads, which arrive together, renew once and all
//! get the session. A renewal that
//! fails is not tried again for the same stored session and NEAR AI login
//! for [`RETRY_AFTER`]: each attempt rotates the NEAR AI refresh token and
//! counts against the commons' provisioning rate limit. Signing in to NEAR
//! AI again lifts the wait.
//!
//! # Never waited for by a read
//!
//! A ceremony is three HTTPS calls, each allowed 30 seconds. The status
//! reads the app makes on every screen (`account_session_status`,
//! `payout_rails_status`, `commons_credit_summary`,
//! `activity_missions_status`) used to wait for all of it, and a renewal
//! that stalled held each of them as long. On a running daemon they now
//! start the renewal on the daemon's own runtime ([`account_session_for_read`])
//! and wait at most [`READ_WAIT`]: a quick renewal is reported by the read
//! that started it, a slow one answers `renewing` and finishes on its own.
//! Actions (withdraw, payout link changes, publish) still wait: they were
//! asked for by a click and can show their own progress.
//!
//! # Failure classes and the log
//!
//! Every attempt that reaches the ceremony is classified ([`classify`]) into
//! a fixed label and appended to [`RENEWAL_LOG_FILE`] in the state
//! directory, label only: when, what triggered it, renewed or failed, and
//! the class. `near_ai_sign_in_required` is the one a person acts on: NEAR
//! AI refused the stored login (spent, revoked, or bound to another agent),
//! and no retry can fix it; the app offers Sign in again, which signs in to
//! NEAR AI first.
//!
//! `device_key_revoked` is the other: the account revoked this device's
//! key (`DELETE /v1/account/devices/{id}`), and the commons never revives
//! one. Sign-out does not recover it: it removes the account session and
//! keeps the device key. The device recovers by unenrolling (`unenroll`),
//! which drops the key with the enrollment, then enrolling again, which
//! makes a new key and joins as a new device. The class is sticky for that
//! key ([`key_revoked`]): neither renewal nor Sign in again runs another
//! ceremony for it.
//!
//! # Sign in again
//!
//! [`sign_in`] is the same ceremony run because the person asked
//! (`near_ai_account_sign_in`): the backoff does not apply, and it also
//! signs in a NEAR AI account whose record is absent -- after a sign-out, a
//! NEAR AI-only contributor had no other way back short of unenrolling.
//!
//! # A device that joined by code
//!
//! A server linked with a one-time code from a signed-in laptop
//! (`account_device_code_redeem`) has no NEAR AI login to present. It keeps
//! a marker file ([`super::device_codes::JOINED_BY_CODE_FILE`]) and renews
//! by signing a fresh nonce with its device key instead
//! ([`super::nearai_onboarding::renew_account_session_by_device_key`]). The
//! commons answers only while that device is linked: revoking it from
//! another device ends renewal too. Every other rule here applies.
//!
//! # Not renewed
//!
//! - **An absent record**, automatically. Sign-out removes it; the person
//!   left, and is not signed back in behind their back. Only [`sign_in`]
//!   signs them in. A NEAR AI sign-in on such a device runs one
//!   provisioning finish to check the login ([`check_fresh_login`]); the
//!   session that answer carries is dropped, never stored.
//! - **Any other tenant.**

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

/// How long a status read waits for a renewal it started (module doc).
pub(crate) const READ_WAIT: Duration = if cfg!(test) {
    Duration::from_millis(1_000)
} else {
    Duration::from_secs(2)
};

/// The renewal log, in the state directory (module doc). JSON lines.
pub const RENEWAL_LOG_FILE: &str = "account-session-renewal.jsonl";

/// The renewal log's ceiling, in lines; the oldest go first.
const RENEWAL_LOG_LINES: usize = 200;

/// The class [`classify`] gives a refusal of the stored NEAR AI login.
pub const FAILURE_NEAR_AI_SIGN_IN_REQUIRED: &str = "near_ai_sign_in_required";

/// The class [`classify`] gives the commons refusing this device's key
/// because its account revoked it. No retry, no NEAR AI sign-in and no
/// sign-out fixes it: the device unenrolls, which drops the key, and enrolls
/// again with a new one.
pub const FAILURE_DEVICE_KEY_REVOKED: &str = "device_key_revoked";

/// Whether the commons refused the enrolled device key as revoked. The
/// marker ([`crate::config::DEVICE_KEY_REVOKED_FILE`]) names the refused key
/// by digest, so it holds for that key only: a renewal of the old key that
/// answers after an unenroll and a new enrollment leaves the new key alone.
/// Every retry of the refused key would rotate the NEAR AI refresh token and
/// spend the provisioning rate limit for an answer that cannot change.
/// Unenroll removes the marker with the enrollment; sign-out does not.
pub(crate) fn key_revoked(store: &crate::config::ConfigStore) -> bool {
    let Ok(Some(cfg)) = store.load_config() else {
        return false;
    };
    key_revoked_for(store, &cfg)
}

/// [`key_revoked`] for a configuration already read.
pub(crate) fn key_revoked_for(
    store: &crate::config::ConfigStore,
    cfg: &crate::config::ContributorConfig,
) -> bool {
    let Ok(Some(marker)) = store.read_daemon_file(crate::config::DEVICE_KEY_REVOKED_FILE) else {
        return false;
    };
    marker.trim_ascii() == revoked_key_digest(&cfg.device_key_id).as_bytes()
}

/// The marker's content: the refused key, by digest, never its id.
fn revoked_key_digest(device_key_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"trace-commons/device-key-revoked/v1");
    hash.update(device_key_id.as_bytes());
    format!("sha256:{}", hex::encode(hash.finalize()))
}

/// Mark `device_key_id` revoked when `class` says so: the key the refused
/// ceremony signed with, not whatever is enrolled when it answers. Best
/// effort: without the marker the 10-minute backoff still bounds the
/// retries.
fn remember_if_revoked(store: &crate::config::ConfigStore, class: &str, device_key_id: &str) {
    if class == FAILURE_DEVICE_KEY_REVOKED {
        let mut marker = revoked_key_digest(device_key_id);
        marker.push('\n');
        let _ = store.write_daemon_file(crate::config::DEVICE_KEY_REVOKED_FILE, marker.as_bytes());
    }
}

/// The stored account record, as `sign_in`, `renew` and
/// [`check_fresh_login`] read it.
pub(super) enum AccountRecord {
    Present {
        raw: Vec<u8>,
        previous: serde_json::Value,
        expected: commons_credentials::Snapshot,
    },
    /// No record (signed out), with the snapshot a write against its
    /// absence is checked with.
    SignedOut {
        expected: commons_credentials::Snapshot,
    },
}

/// Why the account record could not be read.
pub(super) enum AccountRecordError {
    /// Credential storage did not answer (locked, a prompt): may pass.
    Storage,
    /// The record is stored but cannot be read: not JSON, or a reference or
    /// OS entry no retry reads ([`commons_credentials::AccountLoadFailure`]).
    Malformed,
}

/// Read the account record, in one read of credential storage. Blocking:
/// that storage can wait on a prompt. A record no retry reads (not JSON, or
/// a reference or OS entry that is gone or does not match) is `Malformed`
/// (`commons_credentials::AccountLoadFailure`), the same classification the
/// "needs attention" status reads.
pub(super) fn load_account_record(
    store: &crate::config::ConfigStore,
) -> Result<AccountRecord, AccountRecordError> {
    let (raw, expected) =
        commons_credentials::load_account(store).map_err(|failure| match failure {
            commons_credentials::AccountLoadFailure::Storage => AccountRecordError::Storage,
            commons_credentials::AccountLoadFailure::Unreadable => AccountRecordError::Malformed,
        })?;
    let Some(raw) = raw else {
        return Ok(AccountRecord::SignedOut { expected });
    };
    let previous = serde_json::from_slice::<serde_json::Value>(&raw)
        .map_err(|_| AccountRecordError::Malformed)?;
    Ok(AccountRecord::Present {
        raw,
        previous,
        expected,
    })
}

/// Whether `cfg` is an account whose NEAR AI login the commons checks: an
/// account that signs in with a NEAR AI login (its recorded identity kind,
/// or a `nearai-` tenant when none is recorded; F10 decision 4), a device
/// key the account has not revoked, and a device
/// that did not join by one-time code. [`check_fresh_login`] and the
/// "needs attention" status (`super::account_attention`) both gate on it.
pub(super) fn checks_near_ai_login(
    store: &crate::config::ConfigStore,
    cfg: &crate::config::ContributorConfig,
) -> bool {
    cfg.signs_in_with_near_ai()
        && !key_revoked_for(store, cfg)
        && !super::device_codes::joined_by_code(store)
}

/// The enrolled commons under the signup allowlist every NEAR AI ceremony
/// uses. `Err` when this build refuses the address
/// ([`UNCHECKED_COMMONS_ADDRESS_REFUSED`]).
pub(super) fn commons_client(
    cfg: &crate::config::ContributorConfig,
) -> anyhow::Result<trace_commons_operator_client::Client> {
    let allowed = super::account_onboarding::signup_allowlist(&cfg.ingest_url, &[])?;
    super::account_onboarding::client(&cfg.ingest_url, &allowed)
}

/// Whether this build refuses the enrolled address, as [`commons_client`]
/// would, without building a client.
pub(super) fn commons_address_refused(cfg: &crate::config::ContributorConfig) -> bool {
    super::account_onboarding::signup_allowlist(&cfg.ingest_url, &[])
        .and_then(|allowed| super::account_onboarding::check_address(&cfg.ingest_url, &allowed))
        .is_err()
}

/// Per state directory: one renewal at a time, so the reads that arrive
/// together renew once.
static RENEWING: Mutex<Option<HashMap<PathBuf, std::sync::Arc<tokio::sync::Mutex<()>>>>> =
    Mutex::new(None);

fn renewing(dir: &std::path::Path) -> std::sync::Arc<tokio::sync::Mutex<()>> {
    std::sync::Arc::clone(
        RENEWING
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_or_insert_with(HashMap::new)
            .entry(dir.to_path_buf())
            .or_default(),
    )
}

/// Per state directory: the [`attempt_key`] a renewal last failed for, its
/// class, and when.
static FAILED: Mutex<Option<HashMap<PathBuf, (String, &'static str, Instant)>>> = Mutex::new(None);

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
        commons_client(cfg)
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
    if let Some(session) = stored_session(shared, None).await? {
        return Ok(Some(session));
    }
    let Some(via) = via else {
        return Ok(None);
    };
    let turn = renewing(shared.store.dir());
    let _turn = turn.lock().await;
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

/// [`account_session_live`] for a read that must answer quickly (module
/// doc, "Never waited for by a read"): `(session, renewing)`, where
/// `renewing` says a renewal is still running after [`READ_WAIT`]. Without
/// a daemon runtime to run it on (a test or a one-shot command) the renewal
/// runs inline, as before.
pub(crate) async fn account_session_for_read(
    shared: &DaemonShared,
) -> anyhow::Result<(Option<LoadedAccountSession>, bool)> {
    // One budget for every wait below, so a read that collided with a
    // publication still answers within READ_WAIT.
    let deadline = tokio::time::Instant::now() + READ_WAIT;
    if let Some(session) = stored_session(shared, Some(deadline)).await? {
        return Ok((Some(session), false));
    }
    if !may_renew(shared) {
        // `may_renew` reads the record too, and a publication between the
        // two reads makes it refuse: read again once it is done.
        return after_renewal(shared, Some(deadline))
            .await
            .map(|session| (session, false));
    }
    let Some(task) = shared.spawn_detached(|shared| async move {
        let _ = account_session_live(&shared).await;
    }) else {
        return account_session_live(shared)
            .await
            .map(|session| (session, false));
    };
    // Dropping the handle on the deadline leaves the task running.
    let finished = tokio::time::timeout_at(deadline, task).await.is_ok();
    let session = super::withdraw::account_session(shared)?;
    Ok((session.clone(), session.is_none() && !finished))
}

/// The stored session, read again once any renewal in flight is done when
/// the first read fails.
///
/// A renewal stores its session off the runtime thread
/// ([`super::nearai_onboarding`] `store_renewed_session`), and a read that
/// loads the record while that publication replaces it is refused by the
/// credential store rather than answered from a stale snapshot. That is
/// not "signed out": every feature reading at that moment used to answer
/// `account-session-required` or an unknown figure. Renewals publish only
/// inside [`renew`], with the turn held, so the second read, taken under
/// the turn, sees what the renewal left. A store that cannot be read at
/// all fails both reads and is reported as before.
async fn stored_session(
    shared: &DaemonShared,
    deadline: Option<tokio::time::Instant>,
) -> anyhow::Result<Option<LoadedAccountSession>> {
    match super::withdraw::account_session(shared) {
        Ok(session) => Ok(session),
        Err(_) => after_renewal(shared, deadline).await,
    }
}

/// The stored session read under the turn, so no renewal is publishing,
/// waiting for the turn until `deadline` (a read) or for as long as it
/// takes (an action). A read past its deadline reads anyway.
async fn after_renewal(
    shared: &DaemonShared,
    deadline: Option<tokio::time::Instant>,
) -> anyhow::Result<Option<LoadedAccountSession>> {
    let turn = renewing(shared.store.dir());
    let _turn = match deadline {
        Some(deadline) => tokio::time::timeout_at(deadline, turn.lock()).await.ok(),
        None => Some(turn.lock().await),
    };
    super::withdraw::account_session(shared)
}

/// Whether an automatic renewal could be attempted at all, from local
/// reads only: a `nearai-` enrollment with a stored (expired) record, a
/// retained NEAR AI login, and no refusal for this record and login inside
/// the backoff. Checked so a read that cannot renew spawns nothing.
fn may_renew(shared: &DaemonShared) -> bool {
    !key_revoked(&shared.store)
        && near_ai_enrolled(shared)
        && (has_near_ai_login(shared) || super::device_codes::joined_by_code(&shared.store))
        && matches!(
            commons_credentials::load(&shared.store, Kind::Account),
            Ok(Some(raw)) if !recently_failed(shared.store.dir(), &attempt_key(shared, &raw))
        )
}

/// Whether a NEAR AI login is retained, which the NEAR AI renewal presents.
fn has_near_ai_login(shared: &DaemonShared) -> bool {
    shared
        .settings
        .lock()
        .map(|settings| settings.near_ai_session.is_some())
        .unwrap_or(false)
}

/// Whether this device's account signs in with a NEAR AI login (its
/// recorded identity kind, or a `nearai-` tenant when none is recorded; F10
/// decision 4): the accounts [`sign_in`] can sign in with NEAR AI.
pub(crate) fn near_ai_enrolled(shared: &DaemonShared) -> bool {
    matches!(
        shared.store.load_config(),
        Ok(Some(cfg)) if cfg.signs_in_with_near_ai()
    )
}

/// Whether [`sign_in`] and the automatic renewal may run for `cfg`: an
/// account that signs in with a NEAR AI login, or a device that joined an
/// account (in either namespace) by code and renews by its device key.
fn renews(store: &crate::config::ConfigStore, cfg: &crate::config::ContributorConfig) -> bool {
    cfg.signs_in_with_near_ai()
        || (cfg.signs_in_with().is_some() && super::device_codes::joined_by_code(store))
}

/// The class of the last refused renewal of the stored record and NEAR AI
/// login, while its backoff holds. `None` after a renewal, a new NEAR AI
/// sign-in, or the backoff.
pub(crate) fn last_failure(shared: &DaemonShared) -> Option<&'static str> {
    if key_revoked(&shared.store) {
        return Some(FAILURE_DEVICE_KEY_REVOKED);
    }
    let raw = commons_credentials::load(&shared.store, Kind::Account)
        .ok()
        .flatten()
        .unwrap_or_default();
    let key = attempt_key(shared, &raw);
    let failed = FAILED.lock().unwrap_or_else(PoisonError::into_inner);
    failed
        .as_ref()
        .and_then(|by_dir| by_dir.get(shared.store.dir()))
        .filter(|(digest, _, at)| *digest == key && at.elapsed() < RETRY_AFTER)
        .map(|(_, class, _)| *class)
}

/// Sign in again: the ceremony run because the person asked (module doc).
/// The backoff does not apply and an absent record is signed in. Refusals
/// are fixed labels, `near_ai_sign_in_required` among them.
pub(super) async fn sign_in(
    shared: &DaemonShared,
    via: &RenewVia<'_>,
) -> Result<LoadedAccountSession, &'static str> {
    let turn = renewing(shared.store.dir());
    let _turn = turn.lock().await;
    let store = &shared.store;
    let Ok(Some(cfg)) = store.load_config() else {
        return Err("near_ai_sign_in_not_enrolled");
    };
    if !renews(store, &cfg) {
        return Err("near_ai_sign_in_not_near_ai_account");
    }
    // Revoked for good: no ceremony can sign this key in again.
    if key_revoked(store) {
        return Err(FAILURE_DEVICE_KEY_REVOKED);
    }
    if let Ok(Some(session)) = super::withdraw::account_session(shared) {
        return Ok(session);
    }
    let Ok(Some(identity)) = DeviceIdentity::load(store) else {
        return Err("near_ai_sign_in_not_enrolled");
    };
    // A NEAR AI login is presented only for an account that signs in with
    // one; a wallet-kind account's code-joined device renews by its key.
    let near_ai_login = has_near_ai_login(shared) && cfg.signs_in_with_near_ai();
    let by_device_key = !near_ai_login && super::device_codes::joined_by_code(store);
    if !near_ai_login && !by_device_key {
        // Nothing to present: the shell signs in to NEAR AI first.
        return Err(FAILURE_NEAR_AI_SIGN_IN_REQUIRED);
    }
    let (raw, previous, expected) = match super::run_blocking(|| load_account_record(store)) {
        Err(AccountRecordError::Storage) => return Err("commons_credential_storage_unavailable"),
        Err(AccountRecordError::Malformed) => return Err("near_ai_sign_in_unavailable"),
        Ok(AccountRecord::Present {
            raw,
            previous,
            expected,
        }) => (raw, Some(previous), expected),
        Ok(AccountRecord::SignedOut { expected }) => (Vec::new(), None, expected),
    };
    let result = match via.commons(&cfg) {
        Ok(commons) if by_device_key => {
            super::nearai_onboarding::renew_account_session_by_device_key(
                shared, &commons, &cfg, &identity, previous, expected,
            )
            .await
        }
        Ok(commons) => {
            super::nearai_onboarding::renew_account_session(
                shared, via.api, &commons, &cfg, &identity, previous, expected,
            )
            .await
        }
        Err(_) => Err(anyhow::anyhow!("near_ai_enroll_endpoint_refused")),
    };
    match result {
        Ok(()) => {
            record(&shared.store, Trigger::SignIn, None);
            clear_failed(store.dir());
            super::commons_credit::forget(store.dir());
            super::withdraw::account_session(shared)
                .ok()
                .flatten()
                .ok_or("near_ai_sign_in_unavailable")
        }
        Err(error) => {
            let class = classify(&error);
            record(&shared.store, Trigger::SignIn, Some(class));
            failed(store.dir(), attempt_key(shared, &raw), class);
            remember_if_revoked(store, class, &identity.device_key_id);
            Err(match class {
                FAILURE_NEAR_AI_SIGN_IN_REQUIRED => FAILURE_NEAR_AI_SIGN_IN_REQUIRED,
                FAILURE_DEVICE_KEY_REVOKED => FAILURE_DEVICE_KEY_REVOKED,
                _ => "near_ai_sign_in_unavailable",
            })
        }
    }
}

// ---- A fresh NEAR AI sign-in, checked before it is stored ----

/// How long the commons may take to answer a fresh login's check. The
/// browser's sign-in tab waits on it.
const FRESH_LOGIN_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a fresh login's check waits for the renewal turn before it
/// gives up without asking the commons (`commons_not_asked`).
#[cfg(not(test))]
const TURN_WAIT: Duration = Duration::from_secs(10);
#[cfg(test)]
const TURN_WAIT: Duration = Duration::from_millis(300);

/// What the enrolled commons makes of a NEAR AI login a sign-in has just
/// obtained, before that login replaces the stored one (owner, 2026-11-02:
/// remember the account tied to Trace Commons and keep the contributor on
/// it).
pub(crate) enum FreshLogin {
    /// No Trace Commons account on this device signs in through NEAR AI:
    /// not enrolled (a first run), enrolled in another tenant, or a device
    /// key its account revoked. The login is stored as it always was.
    NotBound,
    /// The commons accepted the login for the enrolled account; storing it
    /// renews that account's session too ([`commit_fresh_login`]).
    Accepted(Box<AcceptedLogin>),
    /// Signed out of the account (no record) but still enrolled: the
    /// commons answered for the enrolled tenant and this device's key, so
    /// the login is this account's (owner, 2026-11-02: the connected
    /// credentials name one account). It is stored; the account session the
    /// answer carried is not, so the person is not signed back in here.
    ///
    /// With no account id to compare, the guarantee is the server's: a
    /// device key enrolled through one NEAR AI login refuses a finish with
    /// another (pinned by the ingest's
    /// `a_device_key_enrolled_by_one_near_ai_login_refuses_another`).
    Verified,
    /// The commons refused the login, or resolved it to another account:
    /// most often a different NEAR AI account from the one this device
    /// joined with. The stored login is kept.
    Refused,
    /// Signed in: the commons did not answer, or answered with something
    /// other than a refusal. Signed in or out: a successful answer this
    /// device could not verify as its account (a local check failed: a
    /// clock to correct, say), or a local step failed. Fail closed: the
    /// stored login is kept. (A signed-out device whose commons did not
    /// answer, or answered with something other than a refusal, is
    /// [`FreshLogin::Unchecked`] instead.)
    Unavailable,
    /// The account is bound, but its check gave no verdict: the account
    /// record is malformed, the allowlist refuses the enrolled address, or
    /// (signed out only) the commons did not answer, or answered with
    /// something unusable.
    /// The login is stored, so NEAR AI sign-in is not blocked for good, but
    /// it is not the account's verified login: nothing records it as the
    /// login the commons accepted. The reason is one of
    /// [`UNCHECKED_ACCOUNT_RECORD_MALFORMED`],
    /// [`UNCHECKED_COMMONS_ADDRESS_REFUSED`],
    /// [`UNCHECKED_COMMONS_UNREACHABLE`],
    /// [`UNCHECKED_COMMONS_ANSWER_UNUSABLE`],
    /// [`UNCHECKED_COMMONS_NO_VERDICT`] and [`UNCHECKED_COMMONS_NOT_ASKED`],
    /// the seam a "needs attention" state keys on (#1836); no shell shows
    /// one yet.
    Unchecked(&'static str),
}

/// [`FreshLogin::Unchecked`]: the stored account record is not JSON.
pub(crate) const UNCHECKED_ACCOUNT_RECORD_MALFORMED: &str = "account_record_malformed";
/// [`FreshLogin::Unchecked`]: the signup allowlist refuses the enrolled
/// commons address.
pub(crate) const UNCHECKED_COMMONS_ADDRESS_REFUSED: &str = "commons_address_refused";
/// [`FreshLogin::Unchecked`]: a signed-out device's commons did not answer
/// the start or the finish: no response (a failed connection, a TLS
/// handshake an interceptor broke, a timeout), a status that says it is busy
/// (a server error other than `511`, or `408`, `421`, `425`, `429`), or the
/// check's deadline.
pub(crate) const UNCHECKED_COMMONS_UNREACHABLE: &str = "commons_unreachable";
/// [`FreshLogin::Unchecked`]: a signed-out device got an answer the commons
/// does not send: any other status (a redirect, a proxy's or portal's `403`
/// or `404`, the `511` a captive portal is meant to send) or a body that
/// does not parse or is too large. Often a misrouted address, or a proxy
/// that intercepts TLS with a root this device trusts.
pub(crate) const UNCHECKED_COMMONS_ANSWER_UNUSABLE: &str = "commons_answer_unusable";
/// [`FreshLogin::Unchecked`]: a signed-out device's commons answered the
/// start itself, with its uniform `400` deny: reached, but no verdict (not
/// ready, a refused draw, a database error).
pub(crate) const UNCHECKED_COMMONS_NO_VERDICT: &str = "commons_no_verdict";
/// [`FreshLogin::Unchecked`]: a signed-out device's check never asked the
/// commons, because a renewal held the turn past its wait.
pub(crate) const UNCHECKED_COMMONS_NOT_ASKED: &str = "commons_not_asked";

/// A login the commons accepted, and the account session it earned, held
/// until the NEAR AI credentials are stored. No renewal turn is held with
/// it: the persist can wait on credential storage, and the commit is
/// checked against the record it was read with, so a renewal or sign-out
/// that lands in between wins.
pub(crate) struct AcceptedLogin {
    checked: super::nearai_onboarding::CheckedRenewal,
}

/// [`check_fresh_login`] against the enrolled commons (a test's stub under
/// `cfg(test)` when one is routed for this directory).
pub(crate) async fn check_fresh_login_live(
    store: &crate::config::ConfigStore,
    api: &CloudApi,
    access_token: &str,
) -> FreshLogin {
    #[cfg(test)]
    if let Some(origin) = test_route(store) {
        let Ok(commons) = super::native_identity::client(&origin, "unauthenticated") else {
            return FreshLogin::Unavailable;
        };
        return check_fresh_login(store, &RenewVia::at(api, commons), access_token).await;
    }
    check_fresh_login(store, &RenewVia::live(api), access_token).await
}

/// Ask the enrolled commons whether `access_token`, a fresh NEAR AI login
/// not yet stored, signs in this device's account. Nothing is written,
/// except the revoked-key marker [`sign_in`] and `renew` write too.
///
/// Not bound, and stored as before: not enrolled, another tenant, a
/// revoked device key, and a device that joined by one-time code (it renews
/// by device key and never took this ceremony). A signed-out account (no
/// record) is checked like a signed-in one, against the enrolled tenant and
/// this device's key, and is [`FreshLogin::Verified`] rather than accepted:
/// a NEAR AI sign-in for Private AI does not sign the person back in on this
/// device.
///
/// That check costs the commons one provisioning finish: it counts against
/// the commons's provisioning rate limit, and the account session it issues
/// on the server is dropped here unstored and expires on its own. One runs
/// per NEAR AI sign-in, never per retry of a renewal.
///
/// Only the finish can refuse a login, and the commons refuses one in
/// exactly two ways: its uniform `400` deny, and the revoked-key `409`. Any
/// other outcome gives no verdict, and does not block a signed-out device:
/// its login is stored [`FreshLogin::Unchecked`] with the kind of failure
/// ([`super::nearai_onboarding::FreshCheckFailure`]) --
/// `commons_unreachable` (no response, a busy status, the deadline),
/// `commons_answer_unusable` (an answer the commons does not send),
/// `commons_no_verdict` (the start's own uniform `400`: not ready, a refused
/// draw, a database error; with #1850 its rate limit and an unanswered
/// database are a `429` and a `503` instead), or `commons_not_asked` (a
/// renewal held the turn). A signed-in device fails closed on those.
///
/// A commons that answers its own trouble at the finish with the uniform
/// `400` cannot be told from a refusal here, and reads as
/// [`FreshLogin::Refused`]. #1850 answers the transient cases with a `429` or
/// `503`, but a commons that is not ready, cannot reach its account
/// database, has no account identity configured, or meets a non-transient
/// database error at the finish still answers the uniform `400`. That is a
/// server-side gap; this client cannot tell it apart. A check that issues
/// no session needs a commons endpoint that does not exist yet.
///
/// Unchecked, and stored but not recorded as the account's login: a
/// malformed record or an enrolled address the allowlist refuses, where no
/// retry can run the check and failing closed would block NEAR AI sign-in
/// for good, and a signed-out device whose commons gave no refusal and no
/// verdict. Unavailable, and failing closed: on a signed-in device anything
/// that may pass -- a read of configuration or credential storage, a
/// commons that gave no refusal and no verdict -- and, signed in or out, a
/// successful answer that fails a local check, where the commons spoke and
/// this device could not verify it as its account.
pub(super) async fn check_fresh_login(
    store: &crate::config::ConfigStore,
    via: &RenewVia<'_>,
    access_token: &str,
) -> FreshLogin {
    // F10 section 2.5: a wallet-origin account another device moved to a
    // NEAR AI login still records `wallet` here. Its NEAR AI sign-in is
    // asked about too, and only the commons accepting it as this account's
    // changes anything: the session is stored and the answer's `identity`
    // recorded, so this device renews by the login from then on. Anything
    // else is not bound, exactly as before: a wallet account's sign-in for
    // Private AI is stored as it always was.
    let probe = matches!(store.load_config(), Ok(Some(cfg)) if wallet_origin_probe(store, &cfg));
    let outcome = check_fresh_login_checked(store, via, access_token, probe).await;
    match outcome {
        FreshLogin::Accepted(accepted) => FreshLogin::Accepted(accepted),
        _ if probe => FreshLogin::NotBound,
        other => other,
    }
}

/// A wallet-origin account (recorded or by namespace `wallet`) whose NEAR AI
/// sign-in [`check_fresh_login`] asks about without binding it: a device key
/// not revoked, and a device that did not join by code.
fn wallet_origin_probe(
    store: &crate::config::ConfigStore,
    cfg: &crate::config::ContributorConfig,
) -> bool {
    cfg.signs_in_with() == Some(trace_commons_protocol::account_login_change::IdentityKind::Wallet)
        && !key_revoked_for(store, cfg)
        && !super::device_codes::joined_by_code(store)
}

async fn check_fresh_login_checked(
    store: &crate::config::ConfigStore,
    via: &RenewVia<'_>,
    access_token: &str,
    probe: bool,
) -> FreshLogin {
    let cfg = match store.load_config() {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return FreshLogin::NotBound,
        Err(_) => return FreshLogin::Unavailable,
    };
    if !checks_near_ai_login(store, &cfg) && !probe {
        return FreshLogin::NotBound;
    }
    let identity = match DeviceIdentity::load(store) {
        Ok(Some(identity)) => identity,
        Ok(None) => return FreshLogin::NotBound,
        Err(_) => return FreshLogin::Unavailable,
    };
    let (previous, expected) = match super::run_blocking(|| load_account_record(store)) {
        Ok(AccountRecord::Present {
            previous, expected, ..
        }) => (Some(previous), expected),
        // Signed out: no account to compare, so the answer is held to the
        // enrolled tenant and this device's key alone.
        Ok(AccountRecord::SignedOut { expected }) => (None, expected),
        Err(AccountRecordError::Malformed) => {
            return FreshLogin::Unchecked(UNCHECKED_ACCOUNT_RECORD_MALFORMED);
        }
        Err(AccountRecordError::Storage) => return FreshLogin::Unavailable,
    };
    let signed_out = previous.is_none();
    // A commons that gives no verdict fails a signed-in device closed. A
    // signed-out device stores the login unchecked instead, with the kind
    // of failure: the person left the account, and Private AI does not wait
    // on it.
    let no_verdict = |reason: &'static str| {
        if signed_out {
            FreshLogin::Unchecked(reason)
        } else {
            FreshLogin::Unavailable
        }
    };
    let Ok(commons) = via.commons(&cfg) else {
        return FreshLogin::Unchecked(UNCHECKED_COMMONS_ADDRESS_REFUSED);
    };
    // The renewal turn has its own, shorter wait: a renewal holding it is
    // not the commons failing to answer, and is named apart.
    let Ok(_turn) = tokio::time::timeout(TURN_WAIT, renewing(store.dir()).lock_owned()).await
    else {
        return no_verdict(UNCHECKED_COMMONS_NOT_ASKED);
    };
    let checked = tokio::time::timeout(FRESH_LOGIN_TIMEOUT, async {
        super::nearai_onboarding::check_fresh_login(
            &commons,
            &cfg,
            &identity,
            previous,
            expected,
            access_token,
        )
        .await
    })
    .await;
    let error = match checked {
        // The answer's session is dropped unstored when signed out.
        Ok(Ok(_)) if signed_out => return FreshLogin::Verified,
        Ok(Ok(checked)) => return FreshLogin::Accepted(Box::new(AcceptedLogin { checked })),
        Ok(Err(error)) => error,
        Err(_) => return no_verdict(UNCHECKED_COMMONS_UNREACHABLE),
    };
    // No start answer refuses a login, and only the finish's uniform deny
    // or revoked key does: anything else is a typed no-verdict.
    if let Some(failure) = error.downcast_ref::<super::nearai_onboarding::FreshCheckFailure>() {
        use super::nearai_onboarding::FreshCheckFailure;
        return match failure {
            FreshCheckFailure::Unanswered => no_verdict(UNCHECKED_COMMONS_UNREACHABLE),
            FreshCheckFailure::Unusable => no_verdict(UNCHECKED_COMMONS_ANSWER_UNUSABLE),
            FreshCheckFailure::NoVerdict => no_verdict(UNCHECKED_COMMONS_NO_VERDICT),
            FreshCheckFailure::Local => FreshLogin::Unavailable,
        };
    }
    let class = classify(&error);
    match class {
        "commons_refused" | "commons_answer_mismatch" => {
            // A signed-out device wanted no account session: nothing for
            // the renewal log.
            if !signed_out {
                record(store, Trigger::SignIn, Some(class));
            }
            FreshLogin::Refused
        }
        FAILURE_DEVICE_KEY_REVOKED => {
            remember_if_revoked(store, class, &identity.device_key_id);
            FreshLogin::NotBound
        }
        "commons_address_refused" => FreshLogin::Unchecked(UNCHECKED_COMMONS_ADDRESS_REFUSED),
        // Everything else fails closed, signed in or out: a successful
        // answer this device could not verify as its account (owner,
        // 2026-11-02), or a local failure.
        _ => FreshLogin::Unavailable,
    }
}

/// Store the account session an accepted login earned, once its NEAR AI
/// credentials are stored, and lift any wait a refused renewal left. A
/// record that changed meanwhile (a sign-out) wins.
pub(crate) async fn commit_fresh_login(
    store: &crate::config::ConfigStore,
    accepted: AcceptedLogin,
) -> anyhow::Result<()> {
    let AcceptedLogin { checked } = accepted;
    match super::nearai_onboarding::write_renewed_session(store, checked).await {
        Ok(()) => {
            record(store, Trigger::SignIn, None);
            clear_failed(store.dir());
            super::commons_credit::forget(store.dir());
            Ok(())
        }
        Err(error) => {
            record(store, Trigger::SignIn, Some(classify(&error)));
            Err(error)
        }
    }
}

/// `near_ai_account_sign_in` against the live NEAR AI service (a test's
/// stub under `cfg(test)`, as for [`account_session_live`]).
pub(crate) async fn sign_in_live(
    shared: &DaemonShared,
) -> Result<LoadedAccountSession, &'static str> {
    #[cfg(test)]
    {
        let Some(origin) = test_route(&shared.store) else {
            return Err("near_ai_sign_in_unavailable");
        };
        let api = CloudApi::for_test(&origin).map_err(|_| "near_ai_sign_in_unavailable")?;
        let commons = super::native_identity::client(&origin, "unauthenticated")
            .map_err(|_| "near_ai_sign_in_unavailable")?;
        sign_in(shared, &RenewVia::at(&api, commons)).await
    }
    #[cfg(not(test))]
    {
        let api = CloudApi::live().map_err(|_| "near_ai_sign_in_unavailable")?;
        sign_in(shared, &RenewVia::live(&api)).await
    }
}

/// A renewal failure's fixed class, from the control name the ceremony
/// failed with. Never the error's text: the errors underneath can quote a
/// remote body.
pub(crate) fn classify(error: &anyhow::Error) -> &'static str {
    match error.to_string().as_str() {
        "near_ai_enroll_near_ai_sign_in_required" | "near_ai_enroll_no_session" => {
            FAILURE_NEAR_AI_SIGN_IN_REQUIRED
        }
        // NEAR AI unreachable, timed out, or answering something other
        // than a refusal of the login.
        "near_ai_enroll_token_unavailable" => "near_ai_unavailable",
        "near_ai_enroll_start_failed" => "commons_start_failed",
        "near_ai_enroll_verification_failed" | "device_session_renewal_refused" => {
            "commons_refused"
        }
        super::nearai_onboarding::ENROLL_DEVICE_KEY_REVOKED => FAILURE_DEVICE_KEY_REVOKED,
        "near_ai_enroll_endpoint_refused" => "commons_address_refused",
        "account-enrollment-mismatch" => "commons_answer_mismatch",
        // The device proof (a ceremony window this clock refuses) or the
        // commons's answer failed a local check.
        "near_ai_enroll_invalid" => "local_check_failed",
        "commons_credential_worker_unavailable" => "storage_unavailable",
        _ => "unavailable",
    }
}

/// What started an attempt, for the log.
#[derive(Clone, Copy)]
enum Trigger {
    /// A feature or status read, or the supervisor's pass.
    Automatic,
    /// The person's Sign in again.
    SignIn,
}

/// Append one label-only line to [`RENEWAL_LOG_FILE`], and the same to
/// `tracing`. Best effort: a renewal is not refused for want of a log line.
fn record(store: &crate::config::ConfigStore, trigger: Trigger, failure: Option<&'static str>) {
    let trigger = match trigger {
        Trigger::Automatic => "automatic",
        Trigger::SignIn => "sign_in",
    };
    let outcome = if failure.is_some() {
        "failed"
    } else {
        "renewed"
    };
    match failure {
        None => tracing::info!(target: "account_session", trigger, "account_session_renewed"),
        Some(failure) => tracing::warn!(
            target: "account_session",
            trigger,
            failure,
            "account_session_renewal_failed"
        ),
    }
    let line = serde_json::json!({
        "at": chrono::Utc::now(),
        "trigger": trigger,
        "outcome": outcome,
        "failure": failure,
    });
    let mut lines: Vec<String> = store
        .read_daemon_file(RENEWAL_LOG_FILE)
        .ok()
        .flatten()
        .and_then(|body| String::from_utf8(body).ok())
        .map(|text| {
            text.lines()
                .filter(|line| !line.trim().is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    lines.push(line.to_string());
    if lines.len() > RENEWAL_LOG_LINES {
        lines.drain(..lines.len() - RENEWAL_LOG_LINES);
    }
    let mut body = lines.join("\n");
    body.push('\n');
    let _ = store.write_daemon_file(RENEWAL_LOG_FILE, body.as_bytes());
}

/// [`account_session`] against the live NEAR AI service: the entry point
/// for every daemon method that presents the account session. A usable
/// session is answered without building anything.
pub(crate) async fn account_session_live(
    shared: &DaemonShared,
) -> anyhow::Result<Option<LoadedAccountSession>> {
    if let Some(session) = stored_session(shared, None).await? {
        return Ok(Some(session));
    }
    #[cfg(test)]
    {
        // Never the live service from a test: the stub a fixture named for
        // this state directory, or no renewal at all.
        let Some(origin) = test_route(&shared.store) else {
            return account_session(shared, None).await;
        };
        let api = CloudApi::for_test(&origin)?;
        let commons = super::native_identity::client(&origin, "unauthenticated")?;
        account_session(shared, Some(&RenewVia::at(&api, commons))).await
    }
    #[cfg(not(test))]
    {
        let api = CloudApi::live().ok();
        let via = api.as_ref().map(RenewVia::live);
        account_session(shared, via.as_ref()).await
    }
}

/// Per state directory: when the supervisor's pass last started a renewal.
static OFF_PASS: Mutex<Option<HashMap<PathBuf, Instant>>> = Mutex::new(None);

/// Start a renewal off the supervisor's pass, for automatic upload's
/// admission read, which found no usable session. At most once per
/// [`RETRY_AFTER`] for this state directory, whatever the outcome: the pass
/// runs every minute, and each attempt that gets as far as the ceremony
/// rotates the NEAR AI refresh token. One that starts while a feature call
/// renews waits its turn and finds the renewed session. Every safeguard of [`account_session`] applies (signed out, not
/// a `nearai-` tenant, no NEAR AI login: nothing is asked).
pub(super) fn renew_off_pass(
    shared: &std::sync::Arc<DaemonShared>,
) -> Option<tokio::task::JoinHandle<()>> {
    {
        let mut started = OFF_PASS.lock().unwrap_or_else(PoisonError::into_inner);
        let by_dir = started.get_or_insert_with(HashMap::new);
        let dir = shared.store.dir().to_path_buf();
        if by_dir
            .get(&dir)
            .is_some_and(|at| at.elapsed() < RETRY_AFTER)
        {
            return None;
        }
        by_dir.insert(dir, Instant::now());
    }
    let shared = std::sync::Arc::clone(shared);
    Some(tokio::spawn(async move {
        let _ = account_session_live(&shared).await;
    }))
}

/// Per state directory: the stub a test renews through.
#[cfg(test)]
static TEST_ROUTES: Mutex<Option<HashMap<PathBuf, String>>> = Mutex::new(None);

/// Renew through the stub at `origin` for `store`'s state directory.
#[cfg(test)]
pub(super) fn route_for_test(store: &crate::config::ConfigStore, origin: &str) {
    TEST_ROUTES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(store.dir().to_path_buf(), origin.to_string());
}

#[cfg(test)]
pub(super) fn test_route(store: &crate::config::ConfigStore) -> Option<String> {
    TEST_ROUTES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()?
        .get(store.dir())
        .cloned()
}

/// Whether a session was renewed and stored. Every refusal is `false`.
async fn renew(shared: &DaemonShared, via: &RenewVia<'_>) -> bool {
    let store = &shared.store;
    let Ok(Some(cfg)) = store.load_config() else {
        return false;
    };
    if !renews(store, &cfg) {
        return false;
    }
    // Absent: signed out, never renewed.
    let Ok(AccountRecord::Present {
        raw,
        previous,
        expected,
    }) = super::run_blocking(|| load_account_record(store))
    else {
        return false;
    };
    // The stored session and the NEAR AI login, by digest: a renewal
    // stores a new session, and a new NEAR AI sign-in is a new login.
    if recently_failed(store.dir(), &attempt_key(shared, &raw)) {
        return false;
    }
    let Ok(Some(identity)) = DeviceIdentity::load(store) else {
        return false;
    };
    let near_ai_login = has_near_ai_login(shared) && cfg.signs_in_with_near_ai();
    let renewed = match (near_ai_login, via.commons(&cfg)) {
        (true, Ok(commons)) => {
            super::nearai_onboarding::renew_account_session(
                shared,
                via.api,
                &commons,
                &cfg,
                &identity,
                Some(previous),
                expected,
            )
            .await
        }
        // A device that joined by code has no NEAR AI login: it renews by
        // signing with its device key (owner decision 2026-10-27).
        (false, Ok(commons)) if super::device_codes::joined_by_code(store) => {
            super::nearai_onboarding::renew_account_session_by_device_key(
                shared,
                &commons,
                &cfg,
                &identity,
                Some(previous),
                expected,
            )
            .await
        }
        (false, _) => return false,
        (true, Err(_)) => Err(anyhow::anyhow!("near_ai_enroll_endpoint_refused")),
    };
    match renewed {
        Ok(()) => {
            record(&shared.store, Trigger::Automatic, None);
            super::commons_credit::forget(store.dir());
            true
        }
        Err(error) => {
            // Label only: the errors underneath can quote a remote body.
            let class = classify(&error);
            record(&shared.store, Trigger::Automatic, Some(class));
            // Keyed after the attempt, so the NEAR AI rotation it made is
            // the login a retry would present.
            failed(store.dir(), attempt_key(shared, &raw), class);
            remember_if_revoked(store, class, &identity.device_key_id);
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
        .is_some_and(|(digest, _, at)| digest == record && at.elapsed() < RETRY_AFTER)
}

fn failed(dir: &std::path::Path, record: String, class: &'static str) {
    FAILED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_or_insert_with(HashMap::new)
        .insert(dir.to_path_buf(), (record, class, Instant::now()));
}

fn clear_failed(dir: &std::path::Path) {
    if let Some(by_dir) = FAILED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_mut()
    {
        by_dir.remove(dir);
    }
}

#[cfg(test)]
#[path = "account_session_renewal_tests.rs"]
mod tests;

#[cfg(test)]
mod revoked_key_tests {
    use super::*;

    #[test]
    fn only_the_revoked_class_is_remembered_and_unenroll_removes_it() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        remember_if_revoked(&store, "commons_refused", "key-a");
        assert!(matches!(
            store.read_daemon_file(crate::config::DEVICE_KEY_REVOKED_FILE),
            Ok(None)
        ));
        remember_if_revoked(&store, FAILURE_DEVICE_KEY_REVOKED, "key-a");
        let marker = store
            .read_daemon_file(crate::config::DEVICE_KEY_REVOKED_FILE)
            .unwrap()
            .unwrap();
        assert!(
            !String::from_utf8_lossy(&marker).contains("key-a"),
            "the marker names the key by digest only"
        );
        assert!(!key_revoked(&store), "no enrollment, nothing to hold back");
        store.remove_enrollment_files().unwrap();
        assert!(matches!(
            store.read_daemon_file(crate::config::DEVICE_KEY_REVOKED_FILE),
            Ok(None)
        ));
    }
}
