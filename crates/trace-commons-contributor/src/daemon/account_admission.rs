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
//!   for them.
//! - **Cancelled by any admission refusal** from ingest on an upload, until
//!   the next affirmative answer.
//! - **A spent allowance is held.** `ready: false` with a
//!   `retry_after_seconds` is not asked again until that time has passed.
//! - **No answer is no.** No account session, a transport error, a status
//!   the server did not send, or a body this build cannot read all give
//!   `NotAdvertised`.
//!
//! Tenants outside the anchored namespaces are never asked: R3 does not
//! apply to them.
//!
//! The token is the account session from `account_auth`, the same one
//! withdrawal presents; the device key gains no new authority here. Log
//! lines are fixed labels.

use std::sync::Mutex;
use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::Deserialize;

use super::automatic_gate::AccountAdmission;
use super::ipc::DaemonShared;
use crate::config::{ContributorConfig, allowlist_for};

/// Bounded well under a poll interval, so an unresponsive ingest cannot
/// stall the pass that follows.
const STATUS_TIMEOUT: Duration = Duration::from_secs(10);

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

/// The last answer, in memory only.
#[derive(Debug, Default)]
pub struct AccountAdmissionState {
    answer: Mutex<Option<Answer>>,
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
                    .filter(|s| *s > 0)
                    .map(|s| now + chrono::Duration::seconds(s)),
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
}

/// Ask ingest again, before a full pass. Never fails the pass: every error
/// is an answer of `NotAdvertised`.
pub async fn refresh(shared: &DaemonShared, now: DateTime<Utc>) {
    let state = &shared.account_admission;
    let Ok(Some(cfg)) = shared.store.load_config() else {
        state.forget();
        return;
    };
    if !trace_commons_protocol::admission::is_anchored_tenant(&cfg.tenant_id) {
        state.forget();
        return;
    }
    let key = AnswerKey::of(&cfg);
    if !state.should_ask(&key, now) {
        return;
    }
    let token = match super::run_blocking(|| crate::account_auth::try_load_token(&shared.store)) {
        Ok(Some(token)) => token,
        _ => {
            state.record(key, None, now);
            return;
        }
    };
    let body = fetch_status(&cfg, &token).await;
    if body.is_none() {
        tracing::debug!("contribution status unavailable; R3 stays in force");
    }
    state.record(key, body, now);
}

async fn fetch_status(cfg: &ContributorConfig, token: &str) -> Option<StatusBody> {
    let client = trace_commons_operator_client::Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(token)
    .host_allowlist(allowlist_for(cfg.allowed_hosts.as_deref()))
    .timeout(STATUS_TIMEOUT)
    .build()
    .ok()?;
    client
        .call_json::<(), StatusBody>(Method::GET, "/v1/account/contribution-status", &[], None)
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    async fn serve(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    /// The request carries the account session, never the device key, and a
    /// yes from the server is read as one.
    #[tokio::test]
    async fn the_status_is_read_with_the_account_session() {
        use axum::{Json, Router, http::HeaderMap, routing::get};
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
        let body = fetch_status(&c, "acct-session-token").await.expect("read");
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
        use axum::{Json, Router, http::StatusCode, routing::get};
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
}
