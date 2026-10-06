//! Clients for the commons' credit figures, from two routes.
//!
//! - `GET /v1/contributors/me/settlement-posture` (#1119,
//!   `settlement_posture_handler`): the deployment's settlement posture
//!   (`trace_commons_server::credit_numbers::CreditPosture`). A DEVICE route,
//!   authenticated by the same device-signed status claim as
//!   `submit::status`, so a device-only contributor with no account session
//!   still learns "Settlement off". [`fetch_settlement_posture`].
//! - `GET /v1/account/credit-summary` (`account_credit_summary_handler`):
//!   this contributor's own points on the commons' ledger, which only the
//!   account can see. [`call_credit_summary`]. Its posture block is used
//!   only to decide whether the currency figure may be relayed.
//!
//! Like `withdraw`, the credit-summary route is authenticated by an
//! **account session**,
//! not the device key that authenticates every other call this crate makes.
//! See `daemon::withdraw`'s module doc for why: the session comes from
//! `crate::account_auth`'s browser login, and the daemon must keep whatever
//! rotated token the server hands back, on every response including a
//! refusal, or the contributor is signed out a few minutes later.
//!
//! # Naming: these are the COMMONS' figures
//!
//! #1118's decision makes the commons record the source of pending credit;
//! dollars stay on near.ai as a link-out. Every field exposed here is named
//! `commons_*` so a caller preserves provenance rather than converting points
//! into a near.ai dollar balance. See `daemon::commons_credit` for the IPC half.
//!
//! Never logs or returns a path, a token, or trace content: errors here are
//! a bare "did this work" (only an exactly-200 response counts), matching
//! every other boundary in this crate.

use std::time::Duration;

use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::Deserialize;

use trace_commons_operator_client::Client;
use trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER;

use crate::config::{ConfigStore, ContributorConfig, allowlist_for, config_allowlist};
use crate::identity::DeviceIdentity;
use crate::issuer_client::IssuerClient;

/// The settlement label under which a currency figure means money that can
/// actually move. Anything else -- `dry_run`, `disabled` -- and a dollar
/// figure would be one the app cannot verify.
const SETTLEMENT_HTTP: &str = "http";

/// Bounded well under a poll interval, matching `account_admission`'s own
/// read of a sibling account route.
const CREDIT_SUMMARY_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize)]
struct CreditSummaryBody {
    points: CreditSummaryPoints,
    #[serde(default)]
    currency: Option<CreditSummaryCurrency>,
    posture: CreditSummaryPosture,
    #[serde(default)]
    period: Option<CreditSummaryPeriod>,
    pending_review: i64,
}

#[derive(Debug, Deserialize)]
struct CreditSummaryPeriod {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
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

/// The deployment's settlement posture, from the device route. Every field
/// is named `commons_*` -- see this module's doc for why.
#[derive(Debug, Clone, PartialEq)]
pub struct CommonsSettlementPosture {
    /// `settlement`: the live `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` label the
    /// server is running under (`"http"`, `"dry_run"`, or a fail-safe
    /// disabled label). This is what the design's "Settlement off" line is
    /// drawn from.
    pub commons_settlement: String,
    /// `explanation`: the server's own sentence about what that posture
    /// means for pending credit. Render this rather than composing a second
    /// sentence from `commons_settlement`.
    pub commons_settlement_explanation: String,
    /// `graded`: whether quality, duplicate penalty and the per-contributor
    /// cap are authoritative yet. `false` means a figure may still be
    /// revised.
    pub commons_graded: bool,
}

/// This contributor's own figures on the commons' ledger, from the account
/// route, read successfully. Every field is named `commons_*`.
#[derive(Debug, Clone, PartialEq)]
pub struct CommonsCreditSummary {
    pub commons_points_earned_this_period: i64,
    pub commons_points_lifetime_earned: i64,
    /// Submissions held and not yet counted in the two points figures
    /// above.
    pub commons_pending_review: i64,
    /// `currency.code` / `currency.earned_this_period`, relayed ONLY when
    /// the same response's posture says the figures are graded and
    /// settlement is live (`graded && settlement == "http"`). The app never
    /// shows a dollar figure it cannot verify, and a currency amount beside
    /// ungraded points, or under a settlement mode that moves no money, is
    /// exactly that. `None` on both together -- never one without the other
    /// -- otherwise, and `None` makes no claim about what a point is worth:
    /// it must not be read as zero.
    pub commons_currency_code: Option<String>,
    pub commons_currency_earned_this_period: Option<String>,
    /// `period.start` / `period.end`: the window `earned_this_period`
    /// covers. `None` when the server sent no period.
    pub commons_period_start: Option<DateTime<Utc>>,
    pub commons_period_end: Option<DateTime<Utc>>,
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
        Some(reqwest::StatusCode::OK) => response.result.ok().map(|body| {
            let verifiable = body.posture.graded && body.posture.settlement == SETTLEMENT_HTTP;
            let currency = body.currency.filter(|_| verifiable);
            CommonsCreditSummary {
                commons_points_earned_this_period: body.points.earned_this_period,
                commons_points_lifetime_earned: body.points.lifetime_earned,
                commons_pending_review: body.pending_review,
                commons_currency_code: currency.as_ref().map(|c| c.code.clone()),
                commons_currency_earned_this_period: currency
                    .as_ref()
                    .map(|c| c.earned_this_period.clone()),
                commons_period_start: body.period.as_ref().map(|p| p.start),
                commons_period_end: body.period.as_ref().map(|p| p.end),
            }
        }),
        _ => None,
    };
    CreditSummaryCall {
        result,
        rotated_token: response.response_header,
    }
}

/// The path of the device-authenticated posture route (#1119).
const SETTLEMENT_POSTURE_PATH: &str = "/v1/contributors/me/settlement-posture";

/// Read the deployment's settlement posture over the DEVICE route, so a
/// contributor with no account session can still be told "Settlement off".
///
/// Authenticated like `submit::status`: a device-signed status claim from
/// the issuer, presented as the bearer. `None` for anything but an exactly
/// 200 response with a readable body -- a missing identity, a mint failure,
/// a transport failure, any other status -- so a caller renders "unknown",
/// never a guessed posture.
pub async fn fetch_settlement_posture(
    store: &ConfigStore,
    cfg: &ContributorConfig,
) -> Option<CommonsSettlementPosture> {
    let device = DeviceIdentity::load_or_generate_async(store).await.ok()?;
    let issuer = IssuerClient::new(config_allowlist(cfg)).ok()?;
    let token = crate::submit::mint_status_claim(&issuer, cfg, &device, Utc::now())
        .await
        .ok()?;
    let client = crate::submit::build_ingest_client(cfg, &token).ok()?;
    // `call_json_with_response_header` is the operator client's only call
    // that reports the exact status; the header itself is unused here.
    let response = client
        .call_json_with_response_header::<(), CreditSummaryPosture>(
            Method::GET,
            SETTLEMENT_POSTURE_PATH,
            &[],
            None,
            reqwest::header::CONTENT_TYPE.as_str(),
        )
        .await;
    match response.status {
        Some(reqwest::StatusCode::OK) => {
            response
                .result
                .ok()
                .map(|posture| CommonsSettlementPosture {
                    commons_settlement: posture.settlement,
                    commons_settlement_explanation: posture.explanation,
                    commons_graded: posture.graded,
                })
        }
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode},
        routing::{get, post},
    };
    use std::sync::{Arc, Mutex};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn summary_body(settlement: &str, graded: bool) -> serde_json::Value {
        serde_json::json!({
            "points": {"earned_this_period": 12, "lifetime_earned": 45},
            "currency": {"code": "USD", "earned_this_period": "1.20"},
            "posture": {
                "settlement": settlement,
                "graded": graded,
                "explanation": "Credit is recorded but not settled.",
            },
            "period": {
                "start": "2026-08-01T00:00:00Z",
                "end": "2026-08-31T00:00:00Z",
            },
            "pending_review": 2,
        })
    }

    fn stub_credit_summary(received_auth: Arc<Mutex<Vec<String>>>) -> Router {
        stub_credit_summary_with(
            received_auth,
            StatusCode::OK,
            summary_body("disabled", false),
        )
    }

    fn stub_credit_summary_with(
        received_auth: Arc<Mutex<Vec<String>>>,
        status: StatusCode,
        body: serde_json::Value,
    ) -> Router {
        Router::new().route(
            "/v1/account/credit-summary",
            get(move |headers: HeaderMap| {
                let received_auth = received_auth.clone();
                let body = body.clone();
                async move {
                    if let Some(auth) = headers.get("authorization") {
                        received_auth
                            .lock()
                            .unwrap()
                            .push(auth.to_str().unwrap_or("").to_string());
                    }
                    (status, Json(body))
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
                (headers, Json(summary_body("http", true)))
            }),
        )
    }

    fn stub_credit_summary_status(status: StatusCode) -> Router {
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
    async fn reports_the_commons_points_and_period() {
        let base = spawn(stub_credit_summary(Arc::new(Mutex::new(Vec::new())))).await;
        let summary = call_credit_summary(&base, None, "acct-session-token")
            .await
            .result
            .unwrap();
        assert_eq!(summary.commons_points_earned_this_period, 12);
        assert_eq!(summary.commons_points_lifetime_earned, 45);
        assert_eq!(summary.commons_pending_review, 2);
        assert_eq!(
            summary.commons_period_start,
            Some("2026-08-01T00:00:00Z".parse().unwrap())
        );
        assert_eq!(
            summary.commons_period_end,
            Some("2026-08-31T00:00:00Z".parse().unwrap())
        );
    }

    /// The design rule: the app never shows a dollar figure it cannot
    /// verify. The server hard-codes `graded: false` today, so a currency
    /// block beside it is withheld; so is one under a settlement mode that
    /// moves no money, even when graded.
    #[tokio::test]
    async fn currency_is_relayed_only_when_graded_and_settlement_is_live() {
        for (settlement, graded, relayed) in [
            ("disabled", false, false),
            ("http", false, false),
            ("dry_run", true, false),
            ("disabled", true, false),
            ("http", true, true),
        ] {
            let base = spawn(stub_credit_summary_with(
                Arc::new(Mutex::new(Vec::new())),
                StatusCode::OK,
                summary_body(settlement, graded),
            ))
            .await;
            let summary = call_credit_summary(&base, None, "acct-session-token")
                .await
                .result
                .unwrap();
            if relayed {
                assert_eq!(summary.commons_currency_code.as_deref(), Some("USD"));
                assert_eq!(
                    summary.commons_currency_earned_this_period.as_deref(),
                    Some("1.20")
                );
            } else {
                assert_eq!(
                    summary.commons_currency_code, None,
                    "{settlement} graded={graded}"
                );
                assert_eq!(summary.commons_currency_earned_this_period, None);
            }
        }
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
        let base = spawn(stub_credit_summary_status(StatusCode::UNAUTHORIZED)).await;
        let call = call_credit_summary(&base, None, "stale-token").await;
        assert!(call.result.is_none());
    }

    #[tokio::test]
    async fn a_server_error_is_unknown() {
        let base = spawn(stub_credit_summary_status(
            StatusCode::INTERNAL_SERVER_ERROR,
        ))
        .await;
        let call = call_credit_summary(&base, None, "acct-session-token").await;
        assert!(call.result.is_none());
    }

    /// The test that pins the exactly-200 gate. A 201 or 203 with a body
    /// that would otherwise parse is still not an answer to act on --
    /// `account_admission::fetch_status` applies the same rule to its
    /// sibling account route. Unlike a 204 or a 401, nothing upstream of
    /// the gate refuses these, so replacing the gate with `_ =>` fails here.
    #[tokio::test]
    async fn a_non_200_success_with_a_valid_body_is_unknown() {
        for status in [
            StatusCode::CREATED,
            StatusCode::NON_AUTHORITATIVE_INFORMATION,
        ] {
            let base = spawn(stub_credit_summary_with(
                Arc::new(Mutex::new(Vec::new())),
                status,
                summary_body("http", true),
            ))
            .await;
            let call = call_credit_summary(&base, None, "acct-session-token").await;
            assert!(call.result.is_none(), "{status} must not count");
        }
    }

    fn stub_issuer() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": [],
                    "allowed_uses": [],
                }))
            }),
        )
    }

    /// A device route plus the issuer that mints its bearer, recording the
    /// bearer the posture call presented.
    pub(crate) fn stub_posture_route(
        status: StatusCode,
        settlement: &'static str,
        seen: Arc<Mutex<Vec<String>>>,
    ) -> Router {
        stub_issuer().route(
            SETTLEMENT_POSTURE_PATH,
            get(move |headers: HeaderMap| {
                let seen = seen.clone();
                async move {
                    if let Some(auth) = headers.get("authorization") {
                        seen.lock()
                            .unwrap()
                            .push(auth.to_str().unwrap_or("").to_string());
                    }
                    (
                        status,
                        Json(serde_json::json!({
                            "settlement": settlement,
                            "graded": false,
                            "explanation": "Settlement is off on this deployment.",
                        })),
                    )
                }
            }),
        )
    }

    fn device_cfg(store: &ConfigStore, base: &str) -> ContributorConfig {
        let device = DeviceIdentity::load_or_generate(store).unwrap();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.issuer_url = base.to_string();
        cfg.ingest_url = base.to_string();
        cfg.device_key_id = device.device_key_id;
        cfg
    }

    /// Posture comes from the DEVICE route with a device-minted claim, so a
    /// contributor with no account session still reads "Settlement off".
    #[tokio::test]
    async fn posture_is_read_over_the_device_route_with_a_device_claim() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let base = spawn(stub_posture_route(StatusCode::OK, "disabled", seen.clone())).await;
        let (_d, store) = crate::config::tests_support::temp_store();
        let cfg = device_cfg(&store, &base);
        let posture = fetch_settlement_posture(&store, &cfg).await.unwrap();
        assert_eq!(posture.commons_settlement, "disabled");
        assert!(!posture.commons_graded);
        assert_eq!(
            posture.commons_settlement_explanation,
            "Settlement is off on this deployment."
        );
        assert_eq!(seen.lock().unwrap()[0], "Bearer stub-claim-jwt");
    }

    #[tokio::test]
    async fn posture_from_a_non_200_success_is_unknown() {
        let base = spawn(stub_posture_route(
            StatusCode::NON_AUTHORITATIVE_INFORMATION,
            "disabled",
            Arc::new(Mutex::new(Vec::new())),
        ))
        .await;
        let (_d, store) = crate::config::tests_support::temp_store();
        let cfg = device_cfg(&store, &base);
        assert_eq!(fetch_settlement_posture(&store, &cfg).await, None);
    }
}
