//! Client for the account's referral routes
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, section 6;
//! wire types in `trace_commons_protocol::referral`).
//!
//! Five routes, all on the **account session** from `crate::account_auth`,
//! never the device key (the server refuses a device bearer), as the
//! payout-link routes are (`crate::payout_link`):
//!
//! - `GET /v1/account/referral-codes` -- the account's codes, with how many
//!   people joined with each and the bonus they earned;
//! - `POST /v1/account/referral-codes` -- make a code;
//! - `POST /v1/account/referral-codes/{code_id}/disable` -- turn one off;
//! - `POST /v1/account/referral` -- apply a code to this account;
//! - `GET /v1/account/referral` -- whether one is applied, and until when
//!   one may be.
//!
//! Every success is validated before it is believed: the contract version,
//! every code's shape, the id a disable answers for. A code is sent only in
//! its canonical form ([`normalize_referral_code`]); a typed string that is
//! not a code never leaves the machine. Errors are fixed labels: never a URL,
//! a token, a code or a response body. A server refusal keeps the protocol's
//! own label, which is a closed set.

use reqwest::Method;
use serde::Serialize;
use uuid::Uuid;

use trace_commons_operator_client::{Client, Error as OcError};
use trace_commons_protocol::referral::{
    ApplyReferralRequest, INVALID_REFERRAL_REQUEST, MAX_LIVE_REFERRAL_CODES,
    REFERRAL_ACCOUNT_INELIGIBLE, REFERRAL_CODE_LIMIT_REACHED, REFERRAL_CODE_NOT_FOUND,
    REFERRAL_CONTRACT_VERSION, REFERRAL_NOT_APPLIED, REFERRAL_UNAVAILABLE, ReferralApplied,
    ReferralCodeCreated, ReferralCodeDisabled, ReferralCodesList, ReferralOutcome, ReferralStatus,
    format_referral_code, normalize_referral_code,
};
use trace_commons_protocol::{
    ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER, ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
};

use crate::config::allowlist_for;
pub use crate::payout_link::{Endpoint, REQUEST_TIMEOUT};

/// More codes than this, live or not, is not a list this client
/// understands. An account holds at most [`MAX_LIVE_REFERRAL_CODES`] live
/// ones; turned-off codes stay listed.
pub const MAX_LISTED_CODES: usize = 1024;

const CODES_PATH: &str = "/v1/account/referral-codes";
const REFERRAL_PATH: &str = "/v1/account/referral";

/// The protocol's refusal labels a server may answer and this client keeps
/// by name, because a shell routes on them.
const SERVER_LABELS: &[&str] = &[
    REFERRAL_NOT_APPLIED,
    REFERRAL_UNAVAILABLE,
    REFERRAL_CODE_LIMIT_REACHED,
    REFERRAL_CODE_NOT_FOUND,
    REFERRAL_ACCOUNT_INELIGIBLE,
    INVALID_REFERRAL_REQUEST,
];

/// Fixed-label outcomes. Never carries a response body, a URL, a code or a
/// token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReferralError {
    /// The session was rejected: expired, revoked, or not an account session.
    SessionInvalid,
    /// The server refused with one of the protocol's labels.
    Refused(&'static str),
    /// The request was malformed before it left the machine (a typed code
    /// that is not a code, a nil id).
    RequestInvalid,
    /// A success response that failed validation or did not match the
    /// request.
    ResponseInvalid,
    /// Transport failure, refused host, or an unrecognized refusal.
    Unavailable,
}

impl ReferralError {
    /// The IPC refusal label. A server refusal keeps the protocol's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::SessionInvalid => "account-session-required",
            Self::Refused(label) => label,
            Self::RequestInvalid => "referral-request-invalid",
            Self::ResponseInvalid => "referral-response-invalid",
            Self::Unavailable => "referral-commons-unavailable",
        }
    }
}

/// One call's result, and the rotated account session the server handed back
/// with it. The server rotates on a refusal as well as a success, so the
/// caller must persist `rotated_token` whatever `result` is.
pub struct ReferralCall<T> {
    pub result: Result<T, ReferralError>,
    pub rotated_token: Option<crate::account_auth::RotatedToken>,
}

impl<T> ReferralCall<T> {
    fn refused(error: ReferralError) -> Self {
        Self {
            result: Err(error),
            rotated_token: None,
        }
    }

    fn then<U>(self, check: impl FnOnce(T) -> Result<U, ReferralError>) -> ReferralCall<U> {
        ReferralCall {
            result: self.result.and_then(check),
            rotated_token: self.rotated_token,
        }
    }
}

async fn call<Req, Resp>(
    endpoint: &Endpoint<'_>,
    method: Method,
    path: &str,
    body: Option<&Req>,
) -> ReferralCall<Resp>
where
    Req: Serialize + ?Sized,
    Resp: serde::de::DeserializeOwned,
{
    let client = match Client::builder(
        endpoint.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(endpoint.account_session_token)
    .host_allowlist(allowlist_for(endpoint.allowed_hosts))
    .timeout(REQUEST_TIMEOUT)
    .build()
    {
        Ok(client) => client,
        Err(_) => return ReferralCall::refused(ReferralError::Unavailable),
    };
    let response = client
        .call_json_with_response_headers::<Req, Resp>(
            method,
            path,
            &[],
            body,
            ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
            ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
        )
        .await;
    ReferralCall {
        result: response.result.map_err(|error| classify(&error)),
        rotated_token: crate::account_auth::RotatedToken::from_headers(
            response.response_header,
            response.companion_header,
        ),
    }
}

/// A code as the server shows it: `TC-XXXXX-XXXXX`, canonical.
pub fn valid_shown_code(code: &str) -> bool {
    normalize_referral_code(code)
        .and_then(|normalized| format_referral_code(&normalized))
        .is_some_and(|shown| shown == code)
}

/// The account's codes.
pub async fn list_codes(endpoint: &Endpoint<'_>) -> ReferralCall<ReferralCodesList> {
    call::<(), ReferralCodesList>(endpoint, Method::GET, CODES_PATH, None)
        .await
        .then(|body| {
            let mut ids = std::collections::BTreeSet::new();
            let valid = body.contract_version == REFERRAL_CONTRACT_VERSION
                && body.codes.len() <= MAX_LISTED_CODES
                && body.max_live_codes <= MAX_LIVE_REFERRAL_CODES
                && body.bonus_sent_nano_usd <= body.bonus_owed_nano_usd
                && body.codes.iter().all(|code| {
                    !code.code_id.is_nil()
                        && ids.insert(code.code_id)
                        && code.code.as_deref().is_none_or(valid_shown_code)
                });
            if !valid {
                return Err(ReferralError::ResponseInvalid);
            }
            Ok(body)
        })
}

/// Make a code. Its text is answered once here, and by every later list.
pub async fn create_code(endpoint: &Endpoint<'_>) -> ReferralCall<ReferralCodeCreated> {
    call::<(), ReferralCodeCreated>(endpoint, Method::POST, CODES_PATH, None)
        .await
        .then(|created| {
            if created.contract_version != REFERRAL_CONTRACT_VERSION
                || created.code_id.is_nil()
                || !valid_shown_code(&created.code)
            {
                return Err(ReferralError::ResponseInvalid);
            }
            Ok(created)
        })
}

/// Turn `code_id` off. Existing referrals keep earning.
pub async fn disable_code(
    endpoint: &Endpoint<'_>,
    code_id: Uuid,
) -> ReferralCall<ReferralCodeDisabled> {
    if code_id.is_nil() {
        return ReferralCall::refused(ReferralError::RequestInvalid);
    }
    call::<(), ReferralCodeDisabled>(
        endpoint,
        Method::POST,
        &format!("{CODES_PATH}/{code_id}/disable"),
        None,
    )
    .await
    .then(|disabled| {
        if disabled.contract_version != REFERRAL_CONTRACT_VERSION || disabled.code_id != code_id {
            return Err(ReferralError::ResponseInvalid);
        }
        Ok(disabled)
    })
}

/// Apply `typed` to this account. Only its canonical form is sent; input
/// that is not a code is refused here with [`ReferralError::RequestInvalid`]
/// and never sent.
pub async fn apply(endpoint: &Endpoint<'_>, typed: &str) -> ReferralCall<ReferralApplied> {
    let Some(code) = normalize_referral_code(typed) else {
        return ReferralCall::refused(ReferralError::RequestInvalid);
    };
    let request = ApplyReferralRequest { code };
    call::<ApplyReferralRequest, ReferralApplied>(
        endpoint,
        Method::POST,
        REFERRAL_PATH,
        Some(&request),
    )
    .await
    .then(|applied| {
        if applied.contract_version != REFERRAL_CONTRACT_VERSION
            || applied.outcome != ReferralOutcome::Applied
            || applied.earn_until <= applied.applied_at
        {
            return Err(ReferralError::ResponseInvalid);
        }
        Ok(applied)
    })
}

/// Whether a code is applied to this account.
pub async fn status(endpoint: &Endpoint<'_>) -> ReferralCall<ReferralStatus> {
    call::<(), ReferralStatus>(endpoint, Method::GET, REFERRAL_PATH, None)
        .await
        .then(|status| {
            // An applied referral has its time; one that is not has none.
            let consistent = if status.applied {
                status.applied_at.is_some() && status.applied_via.is_some()
            } else {
                status.applied_at.is_none()
                    && status.applied_via.is_none()
                    && status.earn_until.is_none()
            };
            if status.contract_version != REFERRAL_CONTRACT_VERSION || !consistent {
                return Err(ReferralError::ResponseInvalid);
            }
            Ok(status)
        })
}

fn classify(error: &OcError) -> ReferralError {
    let status = match error {
        OcError::ServerLabel { status, .. } | OcError::HttpFailure { status, .. } => *status,
        OcError::MalformedResponse { .. } => return ReferralError::ResponseInvalid,
        _ => return ReferralError::Unavailable,
    };
    // A label from the protocol's closed set is kept by name, before the
    // session fallthrough: `referral_account_ineligible` is a 403 too.
    if let Some(label) = error
        .server_label()
        .and_then(|label| SERVER_LABELS.iter().find(|known| **known == label))
    {
        return ReferralError::Refused(label);
    }
    match status.as_u16() {
        // 401 is an expired or revoked session; 403 otherwise is a device
        // bearer or a cross-origin refusal. Either way the contributor needs
        // a fresh account sign-in, not a retry.
        401 | 403 => ReferralError::SessionInvalid,
        _ => ReferralError::Unavailable,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A stub of the five referral routes on 127.0.0.1, recording what each
    //! call presented and sent. Shared with the daemon's tests.

    use std::sync::{Arc, Mutex};

    use axum::{
        Json, Router,
        extract::Path,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use trace_commons_protocol::referral::REFERRAL_CONTRACT_VERSION;

    /// What one request carried.
    #[derive(Clone, Debug)]
    pub struct Seen {
        pub method: &'static str,
        pub path: String,
        pub auth: String,
        /// The raw body, exactly as it arrived.
        pub body: String,
    }

    pub struct Stub {
        /// The codes the list answers, as JSON summaries.
        pub codes: Vec<serde_json::Value>,
        pub bonus_owed: u64,
        pub bonus_sent: u64,
        /// The code text the next create answers.
        pub next_code: String,
        pub next_code_id: String,
        /// The canonical code an apply succeeds for; anything else is
        /// `referral_not_applied`.
        pub applicable: String,
        pub applied: bool,
        /// Overrides the answer of the route with this path suffix.
        pub reply: Option<(String, StatusCode, serde_json::Value)>,
        /// Set on every response, when present.
        pub rotate_to: Option<String>,
    }

    impl Default for Stub {
        fn default() -> Self {
            Self {
                codes: Vec::new(),
                bonus_owed: 0,
                bonus_sent: 0,
                next_code: "TC-7K3M9-QX2RD".into(),
                next_code_id: "11111111-2222-4333-8444-555555555555".into(),
                applicable: "ABCDE12345".into(),
                applied: false,
                reply: None,
                rotate_to: None,
            }
        }
    }

    pub struct Server {
        pub base: String,
        pub stub: Arc<Mutex<Stub>>,
        pub seen: Arc<Mutex<Vec<Seen>>>,
    }

    fn auth(headers: &HeaderMap) -> String {
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string()
    }

    fn reply(stub: &Stub, status: StatusCode, body: serde_json::Value) -> Response {
        let mut response = (status, Json(body)).into_response();
        if let Some(token) = &stub.rotate_to {
            response.headers_mut().insert(
                trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                token.parse().unwrap(),
            );
        }
        response
    }

    fn overridden(stub: &Stub, path: &str) -> Option<Response> {
        stub.reply
            .as_ref()
            .filter(|(suffix, _, _)| path.ends_with(suffix.as_str()))
            .map(|(_, status, value)| reply(stub, *status, value.clone()))
    }

    fn record(
        seen: &Mutex<Vec<Seen>>,
        method: &'static str,
        path: String,
        headers: &HeaderMap,
        body: String,
    ) {
        seen.lock().unwrap().push(Seen {
            method,
            path,
            auth: auth(headers),
            body,
        });
    }

    pub async fn spawn() -> Server {
        let stub = Arc::new(Mutex::new(Stub::default()));
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let codes_path = "/v1/account/referral-codes";
        let referral_path = "/v1/account/referral";
        let app = Router::new()
            .route(
                codes_path,
                get({
                    let (stub, seen) = (Arc::clone(&stub), Arc::clone(&seen));
                    move |headers: HeaderMap| async move {
                        record(&seen, "GET", codes_path.into(), &headers, String::new());
                        let stub = stub.lock().unwrap();
                        if let Some(response) = overridden(&stub, codes_path) {
                            return response;
                        }
                        let body = serde_json::json!({
                            "contract_version": REFERRAL_CONTRACT_VERSION,
                            "codes": stub.codes,
                            "max_live_codes": 5,
                            "bonus_owed_nano_usd": stub.bonus_owed,
                            "bonus_sent_nano_usd": stub.bonus_sent,
                        });
                        reply(&stub, StatusCode::OK, body)
                    }
                })
                .post({
                    let (stub, seen) = (Arc::clone(&stub), Arc::clone(&seen));
                    move |headers: HeaderMap, body: String| async move {
                        record(&seen, "POST", codes_path.into(), &headers, body);
                        let mut stub = stub.lock().unwrap();
                        if let Some(response) = overridden(&stub, codes_path) {
                            return response;
                        }
                        let created_at = chrono::Utc::now().to_rfc3339();
                        let summary = serde_json::json!({
                            "code_id": stub.next_code_id,
                            "code": stub.next_code,
                            "state": "live",
                            "created_at": created_at,
                            "referee_count": 0,
                            "bonus_owed_nano_usd": 0,
                        });
                        stub.codes.push(summary);
                        let value = serde_json::json!({
                            "contract_version": REFERRAL_CONTRACT_VERSION,
                            "code_id": stub.next_code_id,
                            "code": stub.next_code,
                            "created_at": created_at,
                        });
                        reply(&stub, StatusCode::OK, value)
                    }
                }),
            )
            .route(
                "/v1/account/referral-codes/{code_id}/disable",
                post({
                    let (stub, seen) = (Arc::clone(&stub), Arc::clone(&seen));
                    move |Path(code_id): Path<String>, headers: HeaderMap, body: String| async move {
                        let path = format!("{codes_path}/{code_id}/disable");
                        record(&seen, "POST", path.clone(), &headers, body);
                        let mut stub = stub.lock().unwrap();
                        if let Some(response) = overridden(&stub, &path) {
                            return response;
                        }
                        let Some(code) = stub
                            .codes
                            .iter_mut()
                            .find(|code| code["code_id"] == code_id.as_str())
                        else {
                            return reply(
                                &stub,
                                StatusCode::NOT_FOUND,
                                serde_json::json!({"error": "referral_code_not_found"}),
                            );
                        };
                        code["state"] = serde_json::json!("disabled");
                        let value = serde_json::json!({
                            "contract_version": REFERRAL_CONTRACT_VERSION,
                            "code_id": code_id,
                            "state": "disabled",
                        });
                        reply(&stub, StatusCode::OK, value)
                    }
                }),
            )
            .route(
                referral_path,
                get({
                    let (stub, seen) = (Arc::clone(&stub), Arc::clone(&seen));
                    move |headers: HeaderMap| async move {
                        record(&seen, "GET", referral_path.into(), &headers, String::new());
                        let stub = stub.lock().unwrap();
                        if let Some(response) = overridden(&stub, referral_path) {
                            return response;
                        }
                        let now = chrono::Utc::now();
                        let body = if stub.applied {
                            serde_json::json!({
                                "contract_version": REFERRAL_CONTRACT_VERSION,
                                "applied": true,
                                "applied_at": now.to_rfc3339(),
                                "applied_via": "later",
                                "earn_until": (now + chrono::TimeDelta::days(92)).to_rfc3339(),
                                "apply_until": null,
                            })
                        } else {
                            serde_json::json!({
                                "contract_version": REFERRAL_CONTRACT_VERSION,
                                "applied": false,
                                "apply_until": (now + chrono::TimeDelta::days(30)).to_rfc3339(),
                            })
                        };
                        reply(&stub, StatusCode::OK, body)
                    }
                })
                .post({
                    let (stub, seen) = (Arc::clone(&stub), Arc::clone(&seen));
                    move |headers: HeaderMap, body: String| async move {
                        record(&seen, "POST", referral_path.into(), &headers, body.clone());
                        let mut stub = stub.lock().unwrap();
                        if let Some(response) = overridden(&stub, referral_path) {
                            return response;
                        }
                        let code = serde_json::from_str::<serde_json::Value>(&body)
                            .ok()
                            .and_then(|value| value["code"].as_str().map(str::to_owned));
                        if stub.applied || code.as_deref() != Some(stub.applicable.as_str()) {
                            return reply(
                                &stub,
                                StatusCode::UNPROCESSABLE_ENTITY,
                                serde_json::json!({"error": "referral_not_applied"}),
                            );
                        }
                        stub.applied = true;
                        let now = chrono::Utc::now();
                        let value = serde_json::json!({
                            "contract_version": REFERRAL_CONTRACT_VERSION,
                            "outcome": "applied",
                            "applied_at": now.to_rfc3339(),
                            "earn_until": (now + chrono::TimeDelta::days(92)).to_rfc3339(),
                        });
                        reply(&stub, StatusCode::OK, value)
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Server { base, stub, seen }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use axum::http::StatusCode;

    const SESSION: &str = "tcn1_account.session";

    fn endpoint(base: &str) -> Endpoint<'_> {
        Endpoint {
            ingest_url: base,
            allowed_hosts: None,
            account_session_token: SESSION,
        }
    }

    #[tokio::test]
    async fn every_route_presents_the_account_session() {
        let server = spawn().await;
        let ep = endpoint(&server.base);
        let created = create_code(&ep).await.result.unwrap();
        assert_eq!(created.code, "TC-7K3M9-QX2RD");
        let listed = list_codes(&ep).await.result.unwrap();
        assert_eq!(listed.codes.len(), 1);
        let disabled = disable_code(&ep, created.code_id).await.result.unwrap();
        assert_eq!(disabled.code_id, created.code_id);
        assert!(!status(&ep).await.result.unwrap().applied);
        let applied = apply(&ep, "tc-abcde-12345").await.result.unwrap();
        assert_eq!(applied.outcome, ReferralOutcome::Applied);
        assert!(status(&ep).await.result.unwrap().applied);
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 6);
        assert!(seen.iter().all(|s| s.auth == format!("Bearer {SESSION}")));
        assert_eq!(
            seen.iter()
                .map(|s| format!("{} {}", s.method, s.path))
                .collect::<Vec<_>>(),
            [
                "POST /v1/account/referral-codes".to_string(),
                "GET /v1/account/referral-codes".to_string(),
                format!(
                    "POST /v1/account/referral-codes/{}/disable",
                    created.code_id
                ),
                "GET /v1/account/referral".to_string(),
                "POST /v1/account/referral".to_string(),
                "GET /v1/account/referral".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn only_the_canonical_code_is_sent_and_a_non_code_never_leaves() {
        let server = spawn().await;
        let ep = endpoint(&server.base);
        // I and O read as 1 and 0; the prefix and separators go.
        apply(&ep, " tc-abcde-i234s ").await.result.unwrap_err();
        let sent = server.seen.lock().unwrap().last().unwrap().body.clone();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&sent).unwrap(),
            serde_json::json!({"code": "ABCDE1234S"})
        );
        let before = server.seen.lock().unwrap().len();
        for typed in ["", "not a code", "TC-ABCDE-1234U", &"A".repeat(100)] {
            assert_eq!(
                apply(&ep, typed).await.result.unwrap_err(),
                ReferralError::RequestInvalid,
                "{typed:?}"
            );
        }
        assert_eq!(
            disable_code(&ep, Uuid::nil()).await.result.unwrap_err(),
            ReferralError::RequestInvalid
        );
        assert_eq!(server.seen.lock().unwrap().len(), before);
    }

    #[tokio::test]
    async fn refusals_keep_the_protocol_label_and_never_a_body() {
        let server = spawn().await;
        let ep = endpoint(&server.base);
        for (status_code, label, expected) in [
            (422, "referral_not_applied", "referral_not_applied"),
            (503, "referral_unavailable", "referral_unavailable"),
            (
                409,
                "referral_code_limit_reached",
                "referral_code_limit_reached",
            ),
            (
                403,
                "referral_account_ineligible",
                "referral_account_ineligible",
            ),
            (400, "invalid_referral_request", "invalid_referral_request"),
            (403, "account session required", "account-session-required"),
            (401, "unauthorized", "account-session-required"),
            (
                500,
                "boom at https://internal.example/x",
                "referral-commons-unavailable",
            ),
            (422, "referral_self", "referral-commons-unavailable"),
        ] {
            server.stub.lock().unwrap().reply = Some((
                "/v1/account/referral".into(),
                StatusCode::from_u16(status_code).unwrap(),
                serde_json::json!({ "error": label }),
            ));
            let error = apply(&ep, "ABCDE12345").await.result.unwrap_err();
            assert_eq!(error.label(), expected, "{status_code} {label}");
        }
    }

    #[tokio::test]
    async fn a_success_that_does_not_validate_is_refused() {
        let server = spawn().await;
        let ep = endpoint(&server.base);
        let id = "11111111-2222-4333-8444-555555555555";
        for (suffix, body) in [
            (
                "/v1/account/referral-codes",
                serde_json::json!({
                    "contract_version": "referral-v0", "codes": [], "max_live_codes": 5,
                    "bonus_owed_nano_usd": 0, "bonus_sent_nano_usd": 0,
                }),
            ),
            (
                "/v1/account/referral-codes",
                serde_json::json!({
                    "contract_version": REFERRAL_CONTRACT_VERSION,
                    "codes": [{
                        "code_id": id, "code": "7K3M9QX2RD", "state": "live",
                        "created_at": "2026-10-29T00:00:00Z", "referee_count": 0,
                        "bonus_owed_nano_usd": 0,
                    }],
                    "max_live_codes": 5, "bonus_owed_nano_usd": 0, "bonus_sent_nano_usd": 0,
                }),
            ),
            (
                "/v1/account/referral-codes",
                serde_json::json!({
                    "contract_version": REFERRAL_CONTRACT_VERSION, "codes": [],
                    "max_live_codes": 5, "bonus_owed_nano_usd": 1, "bonus_sent_nano_usd": 2,
                }),
            ),
        ] {
            server.stub.lock().unwrap().reply = Some((suffix.into(), StatusCode::OK, body));
            assert_eq!(
                list_codes(&ep).await.result.unwrap_err(),
                ReferralError::ResponseInvalid
            );
        }
        server.stub.lock().unwrap().reply = Some((
            "/disable".into(),
            StatusCode::OK,
            serde_json::json!({
                "contract_version": REFERRAL_CONTRACT_VERSION,
                "code_id": "99999999-2222-4333-8444-555555555555", "state": "disabled",
            }),
        ));
        assert_eq!(
            disable_code(&ep, Uuid::parse_str(id).unwrap())
                .await
                .result
                .unwrap_err(),
            ReferralError::ResponseInvalid,
            "a disable answering another code"
        );
        server.stub.lock().unwrap().reply = Some((
            "/v1/account/referral".into(),
            StatusCode::OK,
            serde_json::json!({
                "contract_version": REFERRAL_CONTRACT_VERSION, "applied": false,
                "earn_until": "2027-01-01T00:00:00Z",
            }),
        ));
        assert_eq!(
            status(&ep).await.result.unwrap_err(),
            ReferralError::ResponseInvalid,
            "an unapplied referral with an earning period"
        );
    }

    #[tokio::test]
    async fn the_rotated_session_is_handed_back_on_a_refusal_too() {
        let server = spawn().await;
        {
            let mut stub = server.stub.lock().unwrap();
            stub.rotate_to = Some("tcn1_rotated.session".into());
        }
        let call = apply(&endpoint(&server.base), "ZZZZZ99999").await;
        assert_eq!(
            call.result.unwrap_err(),
            ReferralError::Refused(REFERRAL_NOT_APPLIED)
        );
        assert_eq!(
            call.rotated_token.as_ref().map(|r| r.token.as_str()),
            Some("tcn1_rotated.session")
        );
    }

    #[test]
    fn a_shown_code_is_canonical() {
        assert!(valid_shown_code("TC-7K3M9-QX2RD"));
        for bad in [
            "7K3M9QX2RD",
            "tc-7k3m9-qx2rd",
            "TC-7K3M9-QX2R",
            "TC-7K3M9-QX2RU",
            "",
        ] {
            assert!(!valid_shown_code(bad), "{bad}");
        }
    }
}
