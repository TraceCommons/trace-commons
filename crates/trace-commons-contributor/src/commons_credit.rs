//! Client for the server-side commons credit-summary endpoint.
//!
//! `GET /v1/account/credit-summary`
//! (`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`,
//! `account_credit_summary_handler`). It reports two things the daemon has
//! never read before K7: the deployment's settlement posture (`posture`,
//! `trace_commons_server::credit_numbers::CreditPosture`) and this
//! contributor's own points on the commons' ledger.
//!
//! Like `withdraw`, this route is authenticated by an **account session**,
//! not the device key that authenticates every other call this crate makes.
//! See `daemon::withdraw`'s module doc for why: the session comes from
//! `crate::account_auth`'s browser login, and the daemon must keep whatever
//! rotated token the server hands back, on every response including a
//! refusal, or the contributor is signed out a few minutes later.
//!
//! # Naming: these are the COMMONS' figures
//!
//! #1118's open decision #1 is which ledger is of record for credit: the
//! commons (this route -- points, settlement posture) or near.ai (the cloud
//! balance in dollars, `near_ai_balance`). This client does not decide that
//! question, and neither does the daemon. Every field this module exposes is
//! named `commons_*` so a caller cannot mistake one ledger's figures for the
//! other's, and the daemon presents both without preferring either. See
//! `daemon::commons_credit`'s module doc for the IPC-side half of that rule.
//!
//! Never logs or returns a path, a token, or trace content: errors here are
//! a bare "did this work" (K7 only distinguishes a 200 from everything
//! else), matching every other boundary in this crate.

use std::time::Duration;

use reqwest::Method;
use serde::Deserialize;

use trace_commons_operator_client::Client;
use trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER;

use crate::config::allowlist_for;

/// Bounded well under a poll interval, matching `account_admission`'s own
/// read of a sibling account route.
const CREDIT_SUMMARY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct CreditSummaryBody {
    points: CreditSummaryPoints,
    #[serde(default)]
    currency: Option<CreditSummaryCurrency>,
    posture: CreditSummaryPosture,
    pending_review: i64,
}

#[derive(Debug, Deserialize)]
struct CreditSummaryPoints {
    earned_this_period: i64,
    lifetime_earned: i64,
}

#[derive(Debug, Deserialize)]
struct CreditSummaryCurrency {
    code: String,
    earned_this_period: String,
}

#[derive(Debug, Deserialize)]
struct CreditSummaryPosture {
    settlement: String,
    graded: bool,
    explanation: String,
}

/// The commons' own figures, read successfully. Every field is named
/// `commons_*` -- see this module's doc for why.
#[derive(Debug, Clone, PartialEq)]
pub struct CommonsCreditSummary {
    /// `posture.settlement`: the live `TRACE_COMMONS_NEAR_SETTLEMENT_MODE`
    /// label the server is running under (`"http"`, `"dry_run"`, or a
    /// fail-safe disabled label). This is what the design's "Settlement
    /// off" line is drawn from.
    pub commons_settlement: String,
    /// `posture.explanation`: the server's own sentence about what that
    /// posture means for this contributor's pending credit. Render this
    /// rather than composing a second sentence from `commons_settlement`;
    /// the server already accounts for the grading caveat in it.
    pub commons_settlement_explanation: String,
    /// `posture.graded`: whether quality, duplicate penalty and the
    /// per-contributor cap are authoritative yet. `false` means a figure
    /// here may still be revised.
    pub commons_graded: bool,
    pub commons_points_earned_this_period: i64,
    pub commons_points_lifetime_earned: i64,
    /// Submissions held and not yet counted in the two points figures
    /// above.
    pub commons_pending_review: i64,
    /// `currency.code` / `currency.earned_this_period`, when this deployment
    /// has a configured points-to-currency rate. `None` on both together --
    /// never one without the other -- when it does not: an absent rate
    /// makes no claim about what a point is worth, and must not be read as
    /// zero.
    pub commons_currency_code: Option<String>,
    pub commons_currency_earned_this_period: Option<String>,
}

/// One credit-summary call: its result (`None` for anything but an exactly
/// 200 response this build can read), and the rotated account session the
/// server handed back, if it rotated one.
pub struct CreditSummaryCall {
    pub result: Option<CommonsCreditSummary>,
    pub rotated_token: Option<String>,
}

/// Read the commons' own credit figures and settlement posture for the
/// signed-in account.
///
/// `ingest_url` / `allowed_hosts` are the same values every other ingest
/// call in this crate already loads from `ContributorConfig`.
/// `account_session_token` is the one input this function cannot obtain
/// itself -- see the module doc.
///
/// **Only exactly 200 counts.** Any other status, a transport failure, or a
/// body this build cannot read all produce `result: None` -- fail closed,
/// the same rule `account_admission::fetch_status` applies to its sibling
/// account route. A client renders `None` as "unknown", never as zero: a
/// contributor's settlement posture and points are either read from the
/// server or not stated at all.
pub async fn call_credit_summary(
    ingest_url: &str,
    allowed_hosts: Option<&str>,
    account_session_token: &str,
) -> CreditSummaryCall {
    let Ok(client) = Client::builder(ingest_url, "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV")
        .bearer_token(account_session_token)
        .host_allowlist(allowlist_for(allowed_hosts))
        .timeout(CREDIT_SUMMARY_TIMEOUT)
        .build()
    else {
        return CreditSummaryCall {
            result: None,
            rotated_token: None,
        };
    };
    let response = client
        .call_json_with_response_header::<(), CreditSummaryBody>(
            Method::GET,
            "/v1/account/credit-summary",
            &[],
            None,
            ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
        )
        .await;
    let result = match response.status {
        Some(reqwest::StatusCode::OK) => response.result.ok().map(|body| CommonsCreditSummary {
            commons_settlement: body.posture.settlement,
            commons_settlement_explanation: body.posture.explanation,
            commons_graded: body.posture.graded,
            commons_points_earned_this_period: body.points.earned_this_period,
            commons_points_lifetime_earned: body.points.lifetime_earned,
            commons_pending_review: body.pending_review,
            commons_currency_code: body.currency.as_ref().map(|c| c.code.clone()),
            commons_currency_earned_this_period: body
                .currency
                .as_ref()
                .map(|c| c.earned_this_period.clone()),
        }),
        _ => None,
    };
    CreditSummaryCall {
        result,
        rotated_token: response.response_header,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, http::HeaderMap, routing::get};
    use std::sync::{Arc, Mutex};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn stub_credit_summary(received_auth: Arc<Mutex<Vec<String>>>) -> Router {
        Router::new().route(
            "/v1/account/credit-summary",
            get(move |headers: HeaderMap| {
                let received_auth = received_auth.clone();
                async move {
                    if let Some(auth) = headers.get("authorization") {
                        received_auth
                            .lock()
                            .unwrap()
                            .push(auth.to_str().unwrap_or("").to_string());
                    }
                    Json(serde_json::json!({
                        "points": {"earned_this_period": 12, "lifetime_earned": 45},
                        "currency": {"code": "USD", "earned_this_period": "1.20"},
                        "posture": {
                            "settlement": "disabled",
                            "graded": false,
                            "explanation": "Credit is recorded but not settled.",
                        },
                        "period": {
                            "start": "2026-08-01T00:00:00Z",
                            "end": "2026-08-31T00:00:00Z",
                        },
                        "pending_review": 2,
                    }))
                }
            }),
        )
    }

    fn stub_credit_summary_with_rotation(rotated: &'static str) -> Router {
        Router::new().route(
            "/v1/account/credit-summary",
            get(move || async move {
                let mut headers = HeaderMap::new();
                headers.insert(
                    ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                    rotated.parse().unwrap(),
                );
                (
                    headers,
                    Json(serde_json::json!({
                        "points": {"earned_this_period": 0, "lifetime_earned": 0},
                        "currency": null,
                        "posture": {
                            "settlement": "http",
                            "graded": true,
                            "explanation": "Credit is queued for on-chain settlement.",
                        },
                        "period": {
                            "start": "2026-08-01T00:00:00Z",
                            "end": "2026-08-31T00:00:00Z",
                        },
                        "pending_review": 0,
                    })),
                )
            }),
        )
    }

    fn stub_credit_summary_status(status: axum::http::StatusCode) -> Router {
        Router::new().route(
            "/v1/account/credit-summary",
            get(move || async move { (status, Json(serde_json::json!({"error": "boom"}))) }),
        )
    }

    /// The bearer this call presents is the ACCOUNT session, never the
    /// device key -- the same distinction `withdraw`'s tests pin.
    #[tokio::test]
    async fn presents_the_account_session_as_the_bearer() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(stub_credit_summary(received.clone())).await;
        let call = call_credit_summary(&base, None, "acct-session-token").await;
        assert!(call.result.is_some());
        assert_eq!(received.lock().unwrap()[0], "Bearer acct-session-token");
    }

    #[tokio::test]
    async fn reports_the_commons_figures_labelled_by_source() {
        let base = spawn(stub_credit_summary(Arc::new(Mutex::new(Vec::new())))).await;
        let summary = call_credit_summary(&base, None, "acct-session-token")
            .await
            .result
            .unwrap();
        assert_eq!(summary.commons_settlement, "disabled");
        assert_eq!(
            summary.commons_settlement_explanation,
            "Credit is recorded but not settled."
        );
        assert!(!summary.commons_graded);
        assert_eq!(summary.commons_points_earned_this_period, 12);
        assert_eq!(summary.commons_points_lifetime_earned, 45);
        assert_eq!(summary.commons_pending_review, 2);
        assert_eq!(summary.commons_currency_code.as_deref(), Some("USD"));
        assert_eq!(
            summary.commons_currency_earned_this_period.as_deref(),
            Some("1.20")
        );
    }

    #[tokio::test]
    async fn a_rotated_session_token_is_returned_for_the_daemon_to_store() {
        let base = spawn(stub_credit_summary_with_rotation("tcn1_rotated.secret")).await;
        let call = call_credit_summary(&base, None, "acct-session-token").await;
        assert!(call.result.is_some());
        assert_eq!(call.rotated_token.as_deref(), Some("tcn1_rotated.secret"));
    }

    /// Only exactly 200 counts. A 401 must not be read as "no credit",
    /// which is a different claim than "this daemon does not know".
    #[tokio::test]
    async fn a_401_is_unknown_not_a_zero_figure() {
        let base = spawn(stub_credit_summary_status(
            axum::http::StatusCode::UNAUTHORIZED,
        ))
        .await;
        let call = call_credit_summary(&base, None, "stale-token").await;
        assert!(call.result.is_none());
    }

    #[tokio::test]
    async fn a_server_error_is_unknown() {
        let base = spawn(stub_credit_summary_status(
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        ))
        .await;
        let call = call_credit_summary(&base, None, "acct-session-token").await;
        assert!(call.result.is_none());
    }

    /// A 2xx that is not exactly 200 (e.g. `204`) is not an answer to act
    /// on either -- `account_admission::fetch_status` applies the same
    /// rule to its sibling account route, and K7's brief pins it here too.
    #[tokio::test]
    async fn a_204_is_unknown_even_though_it_is_a_success_status() {
        let base = spawn(stub_credit_summary_status(
            axum::http::StatusCode::NO_CONTENT,
        ))
        .await;
        let call = call_credit_summary(&base, None, "acct-session-token").await;
        assert!(call.result.is_none());
    }
}
