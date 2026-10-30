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
//!   signs them in.
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
    let Ok(Some(marker)) = store.read_daemon_file(crate::config::DEVICE_KEY_REVOKED_FILE) else {
        return false;
    };
    let Ok(Some(cfg)) = store.load_config() else {
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

/// Whether this device is enrolled in a `nearai-` tenant: the accounts
/// [`sign_in`] can sign in.
pub(crate) fn near_ai_enrolled(shared: &DaemonShared) -> bool {
    matches!(
        shared.store.load_config(),
        Ok(Some(cfg)) if super::nearai_onboarding::is_near_ai_tenant_id(&cfg.tenant_id)
    )
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
    if !super::nearai_onboarding::is_near_ai_tenant_id(&cfg.tenant_id) {
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
    let by_device_key = !has_near_ai_login(shared) && super::device_codes::joined_by_code(store);
    if !has_near_ai_login(shared) && !by_device_key {
        // Nothing to present: the shell signs in to NEAR AI first.
        return Err(FAILURE_NEAR_AI_SIGN_IN_REQUIRED);
    }
    let Ok(loaded) =
        super::run_blocking(|| commons_credentials::load_with_snapshot(store, Kind::Account))
    else {
        return Err("commons_credential_storage_unavailable");
    };
    let (raw, previous, expected) = match loaded {
        Some((raw, expected)) => {
            let Ok(previous) = serde_json::from_slice::<serde_json::Value>(&raw) else {
                return Err("near_ai_sign_in_unavailable");
            };
            (raw, Some(previous), expected)
        }
        None => {
            let Ok(expected) = commons_credentials::snapshot(store, Kind::Account) else {
                return Err("commons_credential_storage_unavailable");
            };
            (Vec::new(), None, expected)
        }
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
            record(shared, Trigger::SignIn, None);
            clear_failed(store.dir());
            super::commons_credit::forget(store.dir());
            super::withdraw::account_session(shared)
                .ok()
                .flatten()
                .ok_or("near_ai_sign_in_unavailable")
        }
        Err(error) => {
            let class = classify(&error);
            record(shared, Trigger::SignIn, Some(class));
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
fn record(shared: &DaemonShared, trigger: Trigger, failure: Option<&'static str>) {
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
    let store = &shared.store;
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
fn test_route(store: &crate::config::ConfigStore) -> Option<String> {
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
    let renewed = match (has_near_ai_login(shared), via.commons(&cfg)) {
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
            record(shared, Trigger::Automatic, None);
            super::commons_credit::forget(store.dir());
            true
        }
        Err(error) => {
            // Label only: the errors underneath can quote a remote body.
            let class = classify(&error);
            record(shared, Trigger::Automatic, Some(class));
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
