//! What the configured ingest says about account admission (#1020), for the
//! automatic gate's R3.
//!
//! R3 keeps an armed folder in a `near-`/`nearai-` namespace from sending
//! sessions that ingest would refuse for want of per-session admission
//! evidence. It lifts only while ingest says it admits by account instead,
//! which `/v1/account/contribution-status` reports as its `authority` and
//! `ready` (see `automatic_gate::AccountAdmission::from_status`). That
//! answer comes from one ingest replica and can revert on any redeploy, so
//! it is treated as provisional:
//!
//! - **Read before every full pass**, never persisted. A restart starts from
//!   `NotAdvertised`, which keeps R3.
//! - **Tied to the ingest URL and account it was read for.** A config naming
//!   another ingest, tenant or subject gets `NotAdvertised` until it is read
//!   for them. The server answers for the tenant the session token names, so
//!   a session naming any tenant but the configured one is not asked at all.
//! - **Cancelled by a rule-3 admission refusal** from ingest on an upload
//!   (`admission_refused`, `account_limit_reached`,
//!   `account_identity_unlinked`), until the next affirmative answer.
//! - **A spent allowance is held.** `ready: false` with a
//!   `retry_after_seconds` is not asked again until that time has passed, or
//!   for [`MAX_HOLD`] at most, whatever the server sends.
//! - **No answer is no.** No account session, a transport error, any status
//!   but 200, or a body this build cannot read all give `NotAdvertised`.
//!
//! Nothing is asked, and any earlier answer is dropped, when the gate has
//! nothing to decide: a tenant outside the anchored namespaces (R3 does not
//! apply to it), a dry run (which sends nothing), or no armed folder and no
//! automatic grant that could arm one. Each read keeps the account session
//! alive on the server, so asking every minute for nothing would also defeat
//! its idle expiry.
//!
//! The token is the account session from `account_auth`, the same one
//! withdrawal presents; the device key gains no new authority here. The
//! server rotates that session on this route like any other account route,
//! so a rotated token is stored whatever the status, as `daemon::public_run`
//! does. The session is held in memory between passes and read from the OS
//! credential store again only when the local credential snapshot changes
//! (sign-in, sign-out, a rotation, a configuration change). That read runs
//! off the pass, bounded by [`CREDENTIAL_READ_TIMEOUT`], so a locked or
//! prompting keychain cannot stall it. Log lines are fixed labels.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use reqwest::Method;
use serde::Deserialize;

use super::automatic_gate::AccountAdmission;
use super::ipc::DaemonShared;
use crate::account_auth::LoadedAccountSession;
use crate::config::{ContributorConfig, allowlist_for};

/// Bounded well under a poll interval, so an unresponsive ingest cannot
/// stall the pass that follows.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a pass waits for the OS credential store. A read still running
/// then carries on off the pass and fills the held session for a later one;
/// this pass keeps R3.
#[cfg(not(test))]
const CREDENTIAL_READ_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(test)]
const CREDENTIAL_READ_TIMEOUT: Duration = Duration::from_millis(300);

/// The longest a spent allowance is held without asking again, whatever
/// `retry_after_seconds` says. Also what keeps a broken or hostile value
/// from overflowing the clock.
const MAX_HOLD: TimeDelta = TimeDelta::days(1);

/// The prefix of a native account session: `tcn1_<b64url tenant>.<secret>`.
const NATIVE_TOKEN_PREFIX: &str = "tcn1_";

/// Who an answer was read for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AnswerKey {
    ingest_url: String,
    tenant_id: String,
    user_subject: String,
}

impl AnswerKey {
    fn of(cfg: &ContributorConfig) -> Self {
        Self {
            ingest_url: cfg.ingest_url.clone(),
            tenant_id: cfg.tenant_id.clone(),
            user_subject: cfg.user_subject.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct Answer {
    key: AnswerKey,
    admission: AccountAdmission,
    /// Not asked again before this, for a spent allowance.
    hold_until: Option<DateTime<Utc>>,
}

/// The account session between passes, and whether a credential-store read
/// is still out. Shared with that read, which may outlive the pass that
/// started it.
#[derive(Default)]
struct SessionCache {
    loaded: Mutex<Option<LoadedAccountSession>>,
    reading: AtomicBool,
}

/// Clears `reading` however the read ends, a panic included, so one failed
/// read cannot stop every later pass from trying again.
struct ReadingGuard(Arc<SessionCache>);

impl Drop for ReadingGuard {
    fn drop(&mut self) {
        self.0.reading.store(false, Ordering::SeqCst);
    }
}

/// The last answer, in memory only.
#[derive(Default)]
pub struct AccountAdmissionState {
    answer: Mutex<Option<Answer>>,
    session: Arc<SessionCache>,
}

/// The fields of the status body this client reads. Everything else is
/// ignored, so a server that adds fields does not stop R3 lifting.
#[derive(Debug, Deserialize)]
struct StatusBody {
    authority: String,
    ready: bool,
    #[serde(default)]
    retry_after_seconds: Option<i64>,
}

/// What one status read came back with.
struct StatusReply {
    /// The body, for exactly HTTP 200 and nothing else.
    body: Option<StatusBody>,
    /// A rotated account session, whatever the status.
    rotated_token: Option<String>,
}

/// When a spent allowance may be asked about again: `retry_after_seconds`
/// from `now`, capped at [`MAX_HOLD`]. `None` for no positive value, and for
/// a time the clock cannot represent; R3 holds either way, because the
/// admission recorded beside it is still `AllowanceSpent`.
fn hold_until(now: DateTime<Utc>, retry_after_seconds: i64) -> Option<DateTime<Utc>> {
    if retry_after_seconds <= 0 {
        return None;
    }
    let wait = TimeDelta::try_seconds(retry_after_seconds)
        .unwrap_or(MAX_HOLD)
        .min(MAX_HOLD);
    now.checked_add_signed(wait)
}

/// The tenant a native account session names, or `None` for anything that
/// is not one.
fn session_tenant(token: &str) -> Option<String> {
    use base64::Engine;
    let body = token.strip_prefix(NATIVE_TOKEN_PREFIX)?;
    let (tenant, secret) = body.split_once('.')?;
    if secret.is_empty() {
        return None;
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(tenant)
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Whether anything could be approved unattended: an armed folder, or an
/// automatic grant, which can arm a folder during the very pass that finds
/// it.
fn anything_armed(shared: &DaemonShared) -> bool {
    let policy = shared.policy.lock().expect("policy lock");
    policy.automatic_grant.is_some()
        || policy
            .projects
            .values()
            .any(|entry| entry.mode == super::policy::ProjectMode::AutoUpload)
}

impl AccountAdmissionState {
    /// What the gate should use for `cfg` now: the last answer if it was read
    /// for this ingest and account, and `NotAdvertised` otherwise.
    pub fn current(&self, cfg: Option<&ContributorConfig>) -> AccountAdmission {
        let Some(cfg) = cfg else {
            return AccountAdmission::NotAdvertised;
        };
        let key = AnswerKey::of(cfg);
        match &*self.answer.lock().expect("account admission lock") {
            Some(answer) if answer.key == key => answer.admission,
            _ => AccountAdmission::NotAdvertised,
        }
    }

    /// An admission refusal from ingest: whatever it said before, it does not
    /// admit this account by account now.
    pub fn refused(&self) {
        if let Some(answer) = self.answer.lock().expect("account admission lock").as_mut() {
            answer.admission = AccountAdmission::NotAdvertised;
        }
    }

    fn should_ask(&self, key: &AnswerKey, now: DateTime<Utc>) -> bool {
        match &*self.answer.lock().expect("account admission lock") {
            Some(Answer {
                key: held,
                hold_until: Some(until),
                ..
            }) if held == key => now >= *until,
            _ => true,
        }
    }

    fn record(&self, key: AnswerKey, body: Option<StatusBody>, now: DateTime<Utc>) {
        let (admission, hold_until) = match body {
            Some(body) => (
                AccountAdmission::from_status(&body.authority, body.ready),
                (!body.ready)
                    .then_some(body.retry_after_seconds)
                    .flatten()
                    .and_then(|s| hold_until(now, s)),
            ),
            None => (AccountAdmission::NotAdvertised, None),
        };
        *self.answer.lock().expect("account admission lock") = Some(Answer {
            key,
            admission,
            hold_until,
        });
    }

    /// Record an answer for `cfg`, tests only.
    #[cfg(test)]
    pub(crate) fn record_for_test(&self, cfg: &ContributorConfig, authority: &str, ready: bool) {
        self.record(
            AnswerKey::of(cfg),
            Some(StatusBody {
                authority: authority.to_string(),
                ready,
                retry_after_seconds: None,
            }),
            Utc::now(),
        );
    }

    fn forget(&self) {
        *self.answer.lock().expect("account admission lock") = None;
    }

    /// The account session for this pass: the one held since the last read,
    /// while the local credential snapshot still matches it, and a fresh read
    /// of the OS store otherwise. `None` for no usable session, a store that
    /// cannot be read, or one that did not answer in time.
    async fn session(&self, shared: &DaemonShared) -> Option<LoadedAccountSession> {
        let store = shared.store.clone();
        // Local files only, never the OS store.
        let current = super::run_blocking(|| {
            super::commons_credentials::snapshot(&store, super::commons_credentials::Kind::Account)
        })
        .ok()?;
        if let Some(held) = self.session.loaded.lock().expect("session lock").as_ref() {
            if held.snapshot == current && crate::account_auth::session_is_usable(&held.session) {
                return Some(held.clone());
            }
        }
        if self.session.reading.swap(true, Ordering::SeqCst) {
            // An earlier pass's read is still out; it fills the held session
            // when it finishes.
            return None;
        }
        let cache = Arc::clone(&self.session);
        let read = tokio::task::spawn_blocking(move || {
            let _guard = ReadingGuard(Arc::clone(&cache));
            let loaded = crate::account_auth::try_load_session_with_snapshot(&store);
            let mut held = cache.loaded.lock().expect("session lock");
            match loaded {
                Ok(Some(loaded)) => {
                    *held = Some(loaded.clone());
                    Some(loaded)
                }
                Ok(None) => {
                    *held = None;
                    None
                }
                Err(_) => None,
            }
        });
        match tokio::time::timeout(CREDENTIAL_READ_TIMEOUT, read).await {
            Ok(Ok(loaded)) => loaded,
            _ => {
                tracing::debug!("account session read did not finish; R3 stays in force");
                None
            }
        }
    }

    fn drop_session(&self) {
        *self.session.loaded.lock().expect("session lock") = None;
    }
}

/// Ask ingest again, before a full pass. Never fails the pass: every error
/// is an answer of `NotAdvertised`.
///
/// Only the supervisor's full pass calls this. A scoped pass uses the last
/// answer and never asks.
pub async fn refresh(shared: &DaemonShared, now: DateTime<Utc>, dry_run: bool) {
    let state = &shared.account_admission;
    let Ok(Some(cfg)) = shared.store.load_config() else {
        state.forget();
        return;
    };
    if !trace_commons_protocol::admission::is_anchored_tenant(&cfg.tenant_id)
        || dry_run
        || !anything_armed(shared)
    {
        state.forget();
        return;
    }
    let key = AnswerKey::of(&cfg);
    if !state.should_ask(&key, now) {
        return;
    }
    let Some(session) = state.session(shared).await else {
        state.record(key, None, now);
        return;
    };
    if session_tenant(&session.session.access_token).as_deref() != Some(cfg.tenant_id.as_str()) {
        // The server would answer for the token's tenant, not this one.
        tracing::debug!("account session names another tenant; R3 stays in force");
        state.record(key, None, now);
        return;
    }
    let reply = fetch_status(&cfg, &session.session.access_token).await;
    if let Some(rotated) = reply.rotated_token {
        // Stored against the snapshot the session was read under, so a
        // sign-out in the meantime is not overwritten. Either way the held
        // session is stale now, and the next pass reads the store again.
        if super::run_blocking(|| {
            crate::account_auth::store_rotated_token(&shared.store, &session, rotated)
        })
        .is_err()
        {
            tracing::debug!("rotated account session could not be stored");
        }
        state.drop_session();
    }
    if reply.body.is_none() {
        tracing::debug!("contribution status unavailable; R3 stays in force");
    }
    state.record(key, reply.body, now);
}

async fn fetch_status(cfg: &ContributorConfig, token: &str) -> StatusReply {
    let Ok(client) = trace_commons_operator_client::Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(token)
    .host_allowlist(allowlist_for(cfg.allowed_hosts.as_deref()))
    .timeout(STATUS_TIMEOUT)
    .build() else {
        return StatusReply {
            body: None,
            rotated_token: None,
        };
    };
    let response = client
        .call_json_with_response_header::<(), StatusBody>(
            Method::GET,
            "/v1/account/contribution-status",
            &[],
            None,
            trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
        )
        .await;
    StatusReply {
        // Exactly 200. The client decodes any 2xx, and #1020 answers 200;
        // anything else is not an answer to act on.
        body: match response.status {
            Some(reqwest::StatusCode::OK) => response.result.ok(),
            _ => None,
        },
        rotated_token: response.response_header,
    }
}

/// A native account session naming `tenant`, as the server mints them.
#[cfg(test)]
pub(crate) fn native_token_for_test(tenant: &str, secret: &str) -> String {
    use base64::Engine;
    format!(
        "{NATIVE_TOKEN_PREFIX}{}.{secret}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tenant.as_bytes())
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::StatusCode, response::IntoResponse, routing::get};
    use std::sync::atomic::{AtomicU64, AtomicUsize};

    fn cfg(ingest: &str, tenant: &str) -> ContributorConfig {
        let mut c = crate::commands::unenrolled_preview_config();
        c.ingest_url = ingest.to_string();
        c.tenant_id = tenant.to_string();
        c
    }

    fn body(authority: &str, ready: bool, retry: Option<i64>) -> Option<StatusBody> {
        Some(StatusBody {
            authority: authority.to_string(),
            ready,
            retry_after_seconds: retry,
        })
    }

    fn now() -> DateTime<Utc> {
        "2026-09-26T12:00:00Z".parse().unwrap()
    }

    /// Nothing read is no; a yes counts only for the ingest and account it
    /// was read for.
    #[test]
    fn a_yes_belongs_to_the_ingest_and_account_it_was_read_for() {
        let s = AccountAdmissionState::default();
        let a = cfg("https://a.invalid", "nearai-1");
        assert_eq!(s.current(Some(&a)), AccountAdmission::NotAdvertised);
        assert_eq!(s.current(None), AccountAdmission::NotAdvertised);

        s.record(AnswerKey::of(&a), body("bounded", true, None), now());
        assert_eq!(s.current(Some(&a)), AccountAdmission::Advertised);
        assert_eq!(
            s.current(Some(&cfg("https://b.invalid", "nearai-1"))),
            AccountAdmission::NotAdvertised,
            "another ingest"
        );
        assert_eq!(
            s.current(Some(&cfg("https://a.invalid", "nearai-2"))),
            AccountAdmission::NotAdvertised,
            "another account"
        );
    }

    /// An admission refusal on an upload cancels the yes until the next one.
    #[test]
    fn a_refusal_cancels_the_yes_until_ingest_says_it_again() {
        let s = AccountAdmissionState::default();
        let a = cfg("https://a.invalid", "nearai-1");
        s.record(AnswerKey::of(&a), body("invited", true, None), now());
        s.refused();
        assert_eq!(s.current(Some(&a)), AccountAdmission::NotAdvertised);
        assert!(
            s.should_ask(&AnswerKey::of(&a), now()),
            "asked again next pass"
        );
        s.record(AnswerKey::of(&a), body("invited", true, None), now());
        assert_eq!(s.current(Some(&a)), AccountAdmission::Advertised);
    }

    /// A spent allowance keeps R3 and is not asked again before its
    /// `retry_after_seconds`; no answer at all is no.
    #[test]
    fn a_spent_allowance_is_held_until_its_retry_time() {
        let s = AccountAdmissionState::default();
        let a = cfg("https://a.invalid", "nearai-1");
        let key = AnswerKey::of(&a);
        s.record(key.clone(), body("bounded", false, Some(600)), now());
        assert_eq!(s.current(Some(&a)), AccountAdmission::AllowanceSpent);
        assert!(!s.should_ask(&key, now() + chrono::Duration::seconds(599)));
        assert!(s.should_ask(&key, now() + chrono::Duration::seconds(600)));
        assert!(
            s.should_ask(&AnswerKey::of(&cfg("https://b.invalid", "nearai-1")), now()),
            "the hold is for that account only"
        );

        s.record(key.clone(), None, now());
        assert_eq!(s.current(Some(&a)), AccountAdmission::NotAdvertised);
        assert!(s.should_ask(&key, now()), "no answer is asked again");
        s.record(key.clone(), body("legacy_evidence", false, None), now());
        assert_eq!(s.current(Some(&a)), AccountAdmission::NotAdvertised);
    }

    /// A `retry_after_seconds` out of the clock's range is held for a day,
    /// not a panic in the supervisor loop, and R3 stays.
    #[test]
    fn an_out_of_range_retry_after_is_held_for_a_day() {
        let s = AccountAdmissionState::default();
        let a = cfg("https://a.invalid", "nearai-1");
        let key = AnswerKey::of(&a);
        for retry in [i64::MAX, 10_000_000_000_000, 86_401] {
            s.record(key.clone(), body("bounded", false, Some(retry)), now());
            assert_eq!(s.current(Some(&a)), AccountAdmission::AllowanceSpent);
            assert!(!s.should_ask(&key, now() + TimeDelta::hours(23)), "{retry}");
            assert!(s.should_ask(&key, now() + TimeDelta::days(1)), "{retry}");
        }
        assert_eq!(hold_until(now(), 0), None);
        assert_eq!(hold_until(now(), -5), None);
        assert_eq!(
            hold_until(DateTime::<Utc>::MAX_UTC, 60),
            None,
            "a clock that cannot move is no hold, not a panic"
        );
    }

    #[test]
    fn a_session_names_its_tenant() {
        let tenant = format!("nearai-{}", "a".repeat(64));
        assert_eq!(
            session_tenant(&native_token_for_test(&tenant, "secret")).as_deref(),
            Some(tenant.as_str())
        );
        for other in [
            "tcn1_nodot",
            "tcn1_dGVuYW50.",
            "tcn1_!!!.secret",
            "acct-session-token",
            "",
        ] {
            assert_eq!(session_tenant(other), None, "{other}");
        }
    }

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    /// The request carries the account session, never the device key, and a
    /// yes from the server is read as one.
    #[tokio::test]
    async fn the_status_is_read_with_the_account_session() {
        use axum::http::HeaderMap;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_in = seen.clone();
        let base = serve(Router::new().route(
            "/v1/account/contribution-status",
            get(move |headers: HeaderMap| {
                let seen = seen_in.clone();
                async move {
                    if let Some(auth) = headers.get("authorization") {
                        seen.lock()
                            .unwrap()
                            .push(auth.to_str().unwrap().to_string());
                    }
                    Json(serde_json::json!({
                        "authority": "invited", "policy_version": null, "ready": true,
                        "refusal_label": null, "retry_after_seconds": null,
                    }))
                }
            }),
        ))
        .await;
        let c = cfg(&base, "nearai-1");
        let body = fetch_status(&c, "acct-session-token")
            .await
            .body
            .expect("read");
        assert_eq!(
            AccountAdmission::from_status(&body.authority, body.ready),
            AccountAdmission::Advertised
        );
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            ["Bearer acct-session-token"]
        );
    }

    /// A refusal status, or a body this build cannot read, is no answer.
    #[tokio::test]
    async fn an_error_or_an_unreadable_body_is_no_answer() {
        let refused = serve(Router::new().route(
            "/v1/account/contribution-status",
            get(|| async {
                (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "account_identity_unlinked"})),
                )
            }),
        ))
        .await;
        assert!(
            fetch_status(&cfg(&refused, "nearai-1"), "t")
                .await
                .body
                .is_none()
        );
        let garbled = serve(Router::new().route(
            "/v1/account/contribution-status",
            get(|| async { Json(serde_json::json!({"authority": 7})) }),
        ))
        .await;
        assert!(
            fetch_status(&cfg(&garbled, "nearai-1"), "t")
                .await
                .body
                .is_none()
        );
    }

    /// The fields read from the body are the ones #1020 sends, and extra
    /// fields do not stop it parsing.
    #[test]
    fn the_status_body_parses_what_the_server_sends() {
        let sent = serde_json::json!({
            "authority": "bounded",
            "policy_version": "v1",
            "ready": false,
            "refusal_label": "account_limit_reached",
            "retry_after_seconds": 30,
        });
        let parsed: StatusBody = serde_json::from_value(sent).unwrap();
        assert_eq!(parsed.authority, "bounded");
        assert!(!parsed.ready);
        assert_eq!(parsed.retry_after_seconds, Some(30));
        let legacy: StatusBody = serde_json::from_value(serde_json::json!({
            "authority": "legacy_evidence", "policy_version": null, "ready": false,
            "refusal_label": null, "retry_after_seconds": null,
        }))
        .unwrap();
        assert_eq!(legacy.retry_after_seconds, None);
    }

    // `refresh` end to end: a daemon with a stored account session, an armed
    // folder, and a mock ingest.

    const TENANT: &str = "nearai-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn token(secret: &str) -> String {
        native_token_for_test(TENANT, secret)
    }

    /// A mock contribution-status route, counting the reads it answers.
    fn status_router(
        code: StatusCode,
        body: serde_json::Value,
        rotated: Option<String>,
        hits: Arc<AtomicUsize>,
    ) -> Router {
        Router::new().route(
            "/v1/account/contribution-status",
            get(move || {
                let body = body.clone();
                let rotated = rotated.clone();
                hits.fetch_add(1, Ordering::SeqCst);
                async move {
                    let mut resp = (code, Json(body)).into_response();
                    if let Some(t) = rotated {
                        resp.headers_mut().insert(
                            trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                            t.parse().unwrap(),
                        );
                    }
                    resp
                }
            }),
        )
    }

    fn yes() -> serde_json::Value {
        serde_json::json!({"authority":"bounded","policy_version":"v1","ready":true,
            "refusal_label":null,"retry_after_seconds":null})
    }

    /// The OS credential store as the tests see it, counting reads and able
    /// to stall them, the way a locked or prompting keychain does.
    #[derive(Default)]
    struct SlowStore {
        inner: super::super::cloud_credential_test_support::MemoryBackend,
        reads: AtomicUsize,
        delay_ms: AtomicU64,
    }

    impl super::super::credential_store::SecretBackend for SlowStore {
        fn read(
            &self,
            reference: &super::super::credential_store::CredentialReference,
        ) -> Result<Vec<u8>, super::super::credential_store::CredentialError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(self.delay_ms.load(Ordering::SeqCst)));
            self.inner.read(reference)
        }
        fn write(
            &self,
            reference: &super::super::credential_store::CredentialReference,
            bytes: &[u8],
        ) -> Result<(), super::super::credential_store::CredentialError> {
            self.inner.write(reference, bytes)
        }
        fn delete(
            &self,
            reference: &super::super::credential_store::CredentialReference,
        ) -> Result<(), super::super::credential_store::CredentialError> {
            self.inner.delete(reference)
        }
    }

    struct Daemon {
        _dir: tempfile::TempDir,
        shared: DaemonShared,
        cfg: ContributorConfig,
        os_store: Arc<SlowStore>,
    }

    impl Daemon {
        /// Enrolled in [`TENANT`] against `base`, signed in with `access_token`,
        /// and with one folder armed.
        fn new(base: &str, access_token: &str) -> Self {
            let (dir, store) = crate::config::tests_support::temp_store();
            let os_store = Arc::new(SlowStore::default());
            super::super::commons_credentials::install_test_backend(&store, os_store.clone());
            let mut cfg = crate::commands::unenrolled_preview_config();
            cfg.ingest_url = base.to_string();
            cfg.tenant_id = TENANT.to_string();
            store.save_config(&cfg).unwrap();
            let session = crate::account_auth::AccountSession {
                access_token: access_token.to_string(),
                expires_at: Utc::now() + chrono::TimeDelta::hours(6),
                account_id: "synthetic-account".to_string(),
            };
            store
                .write_daemon_file(
                    crate::config::ACCOUNT_SESSION_FILE,
                    &serde_json::to_vec(&session).unwrap(),
                )
                .unwrap();
            // The first load moves the session into the OS store; count the
            // reads after that.
            crate::account_auth::try_load_token(&store)
                .unwrap()
                .unwrap();
            os_store.reads.store(0, Ordering::SeqCst);
            let shared = DaemonShared::load(store).unwrap();
            let daemon = Self {
                _dir: dir,
                shared,
                cfg,
                os_store,
            };
            daemon.arm(true);
            daemon
        }

        fn arm(&self, armed: bool) {
            let key = super::super::policy::project_key_for(Some("/tmp/armed-project"));
            let mode = if armed {
                super::super::policy::ProjectMode::AutoUpload
            } else {
                super::super::policy::ProjectMode::NotifyOnly
            };
            self.shared
                .policy
                .lock()
                .unwrap()
                .set_mode(&key, mode, Utc::now())
                .unwrap();
        }

        async fn refresh(&self) {
            refresh(&self.shared, Utc::now(), false).await;
        }

        fn current(&self) -> AccountAdmission {
            self.shared.account_admission.current(Some(&self.cfg))
        }

        fn stored_token(&self) -> Option<String> {
            crate::account_auth::try_load_token(&self.shared.store).unwrap()
        }
    }

    /// Refresh, then read: what ingest said is what the gate sees.
    #[tokio::test]
    async fn refresh_reads_what_ingest_says() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = serve(status_router(StatusCode::OK, yes(), None, hits.clone())).await;
        let d = Daemon::new(&base, &token("original"));
        assert_eq!(d.current(), AccountAdmission::NotAdvertised);
        d.refresh().await;
        assert_eq!(d.current(), AccountAdmission::Advertised);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    /// The server rotates the session on this route. The new token is kept,
    /// or the old one dies after the server's grace and signs the
    /// contributor out.
    #[tokio::test]
    async fn a_rotated_session_from_the_status_read_is_kept() {
        let rotated = token("rotated");
        let base = serve(status_router(
            StatusCode::OK,
            yes(),
            Some(rotated.clone()),
            Default::default(),
        ))
        .await;
        let d = Daemon::new(&base, &token("original"));
        d.refresh().await;
        assert_eq!(d.stored_token(), Some(rotated));
        assert_eq!(d.current(), AccountAdmission::Advertised);
    }

    /// The middleware rotates before the handler runs, so a refusal can
    /// carry a rotation too, and it is kept all the same.
    #[tokio::test]
    async fn a_rotated_session_is_kept_on_a_refusal_too() {
        let rotated = token("rotated");
        let base = serve(status_router(
            StatusCode::FORBIDDEN,
            serde_json::json!({"error":"account_identity_unlinked"}),
            Some(rotated.clone()),
            Default::default(),
        ))
        .await;
        let d = Daemon::new(&base, &token("original"));
        d.refresh().await;
        assert_eq!(d.stored_token(), Some(rotated));
        assert_eq!(d.current(), AccountAdmission::NotAdvertised);
    }

    /// Rule 1 says 200. A 202 with the same body is not an answer.
    #[tokio::test]
    async fn only_a_200_lifts_r3() {
        let base = serve(status_router(
            StatusCode::ACCEPTED,
            yes(),
            None,
            Default::default(),
        ))
        .await;
        let d = Daemon::new(&base, &token("original"));
        d.refresh().await;
        assert_eq!(
            d.current(),
            AccountAdmission::NotAdvertised,
            "a 202 lifted R3"
        );
    }

    /// A `retry_after_seconds` the clock cannot hold, served for real:
    /// no panic, and R3 stays under its own reason.
    #[tokio::test]
    async fn an_out_of_range_retry_after_from_ingest_does_not_panic() {
        for retry in [i64::MAX, 10_000_000_000_000_i64] {
            let base = serve(status_router(
                StatusCode::OK,
                serde_json::json!({"authority":"bounded","ready":false,"retry_after_seconds":retry}),
                None,
                Default::default(),
            ))
            .await;
            let d = Daemon::new(&base, &token("original"));
            d.refresh().await;
            assert_eq!(d.current(), AccountAdmission::AllowanceSpent, "{retry}");
        }
    }

    /// The server answers for the tenant the token names. A session for
    /// another tenant is not asked, and lifts nothing for the configured one.
    #[tokio::test]
    async fn a_session_for_another_tenant_lifts_nothing() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = serve(status_router(StatusCode::OK, yes(), None, hits.clone())).await;
        let other = native_token_for_test(&format!("nearai-{}", "b".repeat(64)), "secret");
        for session in [other.as_str(), "acct-session-token"] {
            let d = Daemon::new(&base, session);
            d.refresh().await;
            assert_eq!(
                d.current(),
                AccountAdmission::NotAdvertised,
                "a yes read with a session for another tenant lifted R3 for the configured one"
            );
        }
        assert_eq!(hits.load(Ordering::SeqCst), 0, "never asked");
    }

    /// A refusal replaces an earlier yes.
    #[tokio::test]
    async fn a_refused_status_read_replaces_an_earlier_yes() {
        let base = serve(status_router(
            StatusCode::FORBIDDEN,
            serde_json::json!({"error":"admission_refused"}),
            None,
            Default::default(),
        ))
        .await;
        let d = Daemon::new(&base, &token("original"));
        d.shared
            .account_admission
            .record_for_test(&d.cfg, "bounded", true);
        d.refresh().await;
        assert_eq!(d.current(), AccountAdmission::NotAdvertised);
    }

    /// A yes read for one ingest is not applied once the config names
    /// another.
    #[tokio::test]
    async fn a_yes_is_not_applied_after_the_ingest_changes() {
        let base = serve(status_router(
            StatusCode::OK,
            yes(),
            None,
            Default::default(),
        ))
        .await;
        let mut d = Daemon::new(&base, &token("original"));
        d.refresh().await;
        assert_eq!(d.current(), AccountAdmission::Advertised);
        d.cfg.ingest_url = "http://127.0.0.1:1".into();
        d.shared.store.save_config(&d.cfg).unwrap();
        let loaded = d.shared.store.load_config().unwrap().unwrap();
        assert_eq!(
            d.shared.account_admission.current(Some(&loaded)),
            AccountAdmission::NotAdvertised
        );
    }

    /// Nothing armed, no grant: the gate decides nothing, so ingest is not
    /// asked and an earlier answer is dropped.
    #[tokio::test]
    async fn nothing_is_asked_with_nothing_armed() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = serve(status_router(StatusCode::OK, yes(), None, hits.clone())).await;
        let d = Daemon::new(&base, &token("original"));
        d.arm(false);
        d.shared
            .account_admission
            .record_for_test(&d.cfg, "bounded", true);
        d.refresh().await;
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert_eq!(d.current(), AccountAdmission::NotAdvertised);

        d.arm(true);
        d.refresh().await;
        assert_eq!(
            hits.load(Ordering::SeqCst),
            1,
            "asked once something is armed"
        );
    }

    /// A dry run sends nothing, so it asks nothing either.
    #[tokio::test]
    async fn nothing_is_asked_in_a_dry_run() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = serve(status_router(StatusCode::OK, yes(), None, hits.clone())).await;
        let d = Daemon::new(&base, &token("original"));
        d.shared
            .account_admission
            .record_for_test(&d.cfg, "bounded", true);
        refresh(&d.shared, Utc::now(), true).await;
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        assert_eq!(d.current(), AccountAdmission::NotAdvertised);
    }

    /// The OS store is read once, not every pass, and read again once the
    /// credential changes under it.
    #[tokio::test]
    async fn the_session_is_read_from_the_os_store_once_until_it_changes() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = serve(status_router(StatusCode::OK, yes(), None, hits.clone())).await;
        let d = Daemon::new(&base, &token("original"));
        for _ in 0..3 {
            d.refresh().await;
        }
        assert_eq!(hits.load(Ordering::SeqCst), 3, "asked every pass");
        assert_eq!(d.os_store.reads.load(Ordering::SeqCst), 1, "read once");
        assert_eq!(d.current(), AccountAdmission::Advertised);

        // Signing out changes the snapshot: the held session is not used.
        crate::account_auth::clear_token(&d.shared.store).unwrap();
        d.refresh().await;
        assert_eq!(hits.load(Ordering::SeqCst), 3, "not asked once signed out");
        assert_eq!(d.current(), AccountAdmission::NotAdvertised);
    }

    /// A rotation changes the stored credential, and the next pass presents
    /// the rotated token rather than the one it held.
    #[tokio::test]
    async fn the_pass_after_a_rotation_presents_the_rotated_token() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let rotated = token("rotated");
        let base = serve(Router::new().route(
            "/v1/account/contribution-status",
            get({
                let seen = seen.clone();
                let rotated = rotated.clone();
                move |headers: axum::http::HeaderMap| {
                    let seen = seen.clone();
                    let rotated = rotated.clone();
                    async move {
                        let auth = headers["authorization"].to_str().unwrap().to_string();
                        let first = seen.lock().unwrap().is_empty();
                        seen.lock().unwrap().push(auth);
                        let mut resp = Json(yes()).into_response();
                        if first {
                            resp.headers_mut().insert(
                                trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                                rotated.parse().unwrap(),
                            );
                        }
                        resp
                    }
                }
            }),
        ))
        .await;
        let d = Daemon::new(&base, &token("original"));
        d.refresh().await;
        d.refresh().await;
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [
                format!("Bearer {}", token("original")),
                format!("Bearer {rotated}")
            ]
        );
    }

    /// A keychain that does not answer holds R3 for this pass instead of
    /// stalling it, and its read, once it finishes, serves a later pass.
    #[tokio::test]
    async fn a_stalled_credential_store_does_not_stall_the_pass() {
        let hits = Arc::new(AtomicUsize::new(0));
        let base = serve(status_router(StatusCode::OK, yes(), None, hits.clone())).await;
        let d = Daemon::new(&base, &token("original"));
        d.os_store.delay_ms.store(1_500, Ordering::SeqCst);

        let started = std::time::Instant::now();
        d.refresh().await;
        assert!(
            started.elapsed() < Duration::from_millis(1_200),
            "the pass waited {:?} for the credential store",
            started.elapsed()
        );
        assert_eq!(d.current(), AccountAdmission::NotAdvertised);
        assert_eq!(hits.load(Ordering::SeqCst), 0);

        // While that read is out, a pass does not start another.
        d.refresh().await;
        assert_eq!(d.os_store.reads.load(Ordering::SeqCst), 1);

        tokio::time::timeout(Duration::from_secs(10), async {
            while d
                .shared
                .account_admission
                .session
                .reading
                .load(Ordering::SeqCst)
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the stalled read finishes");
        d.refresh().await;
        assert_eq!(d.current(), AccountAdmission::Advertised);
        assert_eq!(
            d.os_store.reads.load(Ordering::SeqCst),
            1,
            "its read was used"
        );
    }
}
