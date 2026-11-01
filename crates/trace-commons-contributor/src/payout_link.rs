//! Client for the account's payout-link routes
//! (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
//! section 5; wire types in `trace_commons_protocol::payout_link`).
//!
//! Three routes, all on the **account session** from `crate::account_auth`,
//! never the device key (the server refuses a device bearer):
//!
//! - `GET /v1/account/payout-rails` -- each enabled rail's link state and
//!   version, never a recipient or a name;
//! - `POST /v1/account/payout-rails/{rail_id}/link-requests` -- start a link
//!   or an unlink. Nothing changes on the server until it is confirmed;
//! - `POST /v1/account/payout-rails/{rail_id}/link-requests/{handle}/confirm`
//!   -- confirm it from the same account.
//!
//! The signed-in session is the authority, so the daemon calls confirm itself once the person confirms in
//! the app. There is no step-up page.
//!
//! Every success is validated before it is believed. A started link must
//! answer the recipient ref this client recomputes from the idempotency key
//! and the org it chose ([`near_ai_recipient_ref`]): a server that would
//! store another org is refused before anything is confirmed. Errors are
//! fixed labels: never a URL, a token, an org, or a response body. A server
//! refusal keeps the protocol's own outcome label, which is a closed set.

use std::time::Duration;

use reqwest::Method;
use serde::Serialize;
use uuid::Uuid;

use trace_commons_operator_client::{Client, Error as OcError};
use trace_commons_protocol::payout_link::{
    PAYOUT_LINK_CONTRACT_VERSION, PayoutLinkAction, PayoutLinkConfirmed, PayoutLinkOutcome,
    PayoutLinkProof, PayoutLinkRequest, PayoutLinkStarted, PayoutRailsStatus,
    near_ai_recipient_ref,
};
use trace_commons_protocol::{
    ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER, ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
};

use crate::config::allowlist_for;

/// Bounded: a start waits on the server's own NEAR AI checks, and a hung
/// commons must not hang the shell with it.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// More rails than this is not a status this client understands.
pub const MAX_RAILS: usize = 16;

const STATUS_PATH: &str = "/v1/account/payout-rails";

/// A rail id as the server spells them: lowercase, digits and `_`, short.
/// Checked before an id becomes a path segment.
pub fn valid_rail_id(rail_id: &str) -> bool {
    (1..=32).contains(&rail_id.len())
        && rail_id.as_bytes()[0].is_ascii_lowercase()
        && rail_id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// A confirm handle: 64 lowercase hex characters.
pub fn valid_handle(handle: &str) -> bool {
    handle.len() == 64
        && handle
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// `sha256:` and 64 lowercase hex characters.
pub fn valid_recipient_ref(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && valid_handle(hex))
}

/// Fixed-label outcomes. Never carries a response body, a URL, an org or a
/// token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayoutLinkError {
    /// The session was rejected: expired, revoked, or not an account session.
    SessionInvalid,
    /// The server refused with one of the protocol's outcome labels.
    Refused(PayoutLinkOutcome),
    /// The request was malformed before it left the machine.
    RequestInvalid,
    /// A success response that failed validation or did not match the
    /// request.
    ResponseInvalid,
    /// A started link answered a recipient ref other than the one for the
    /// org this client chose. Nothing is confirmed.
    RecipientMismatch,
    /// Transport failure, refused host, or an unrecognized refusal.
    Unavailable,
}

impl PayoutLinkError {
    /// The IPC refusal label. A server refusal keeps the protocol's label,
    /// because a shell routes on it.
    pub fn label(self) -> &'static str {
        match self {
            Self::SessionInvalid => "account-session-required",
            Self::Refused(outcome) => outcome.as_str(),
            Self::RequestInvalid => "payout-link-request-invalid",
            Self::ResponseInvalid => "payout-link-response-invalid",
            Self::RecipientMismatch => "payout-link-recipient-mismatch",
            Self::Unavailable => "payout-link-unavailable",
        }
    }
}

/// One call's result, and the rotated account session the server handed back
/// with it. The server rotates on a refusal as well as a success, so the
/// caller must persist `rotated_token` whatever `result` is.
pub struct PayoutCall<T> {
    pub result: Result<T, PayoutLinkError>,
    pub rotated_token: Option<crate::account_auth::RotatedToken>,
}

impl<T> PayoutCall<T> {
    fn refused(error: PayoutLinkError) -> Self {
        Self {
            result: Err(error),
            rotated_token: None,
        }
    }
}

/// Where a call goes and what it presents. Built from the same enrolled
/// `ingest_url` / `allowed_hosts` every other ingest call uses.
pub struct Endpoint<'a> {
    pub ingest_url: &'a str,
    pub allowed_hosts: Option<&'a str>,
    pub account_session_token: &'a str,
}

impl Endpoint<'_> {
    async fn call<Req, Resp>(
        &self,
        method: Method,
        path: &str,
        body: Option<&Req>,
    ) -> PayoutCall<Resp>
    where
        Req: Serialize + ?Sized,
        Resp: serde::de::DeserializeOwned,
    {
        let client = match Client::builder(
            self.ingest_url,
            "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
        )
        .bearer_token(self.account_session_token)
        .host_allowlist(allowlist_for(self.allowed_hosts))
        .timeout(REQUEST_TIMEOUT)
        .build()
        {
            Ok(client) => client,
            Err(_) => return PayoutCall::refused(PayoutLinkError::Unavailable),
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
        PayoutCall {
            result: response.result.map_err(|error| classify(&error)),
            rotated_token: crate::account_auth::RotatedToken::from_headers(
                response.response_header,
                response.companion_header,
            ),
        }
    }
}

fn then<T, U>(
    call: PayoutCall<T>,
    check: impl FnOnce(T) -> Result<U, PayoutLinkError>,
) -> PayoutCall<U> {
    PayoutCall {
        result: call.result.and_then(check),
        rotated_token: call.rotated_token,
    }
}

/// Every enabled rail's link state for this account.
pub async fn status(endpoint: &Endpoint<'_>) -> PayoutCall<PayoutRailsStatus> {
    let call = endpoint
        .call::<(), PayoutRailsStatus>(Method::GET, STATUS_PATH, None)
        .await;
    then(call, |body| {
        let mut ids = std::collections::BTreeSet::new();
        let valid = body.contract_version == PAYOUT_LINK_CONTRACT_VERSION
            && body.rails.len() <= MAX_RAILS
            && body.rails.iter().all(|rail| {
                valid_rail_id(&rail.rail_id)
                    && ids.insert(rail.rail_id.clone())
                    && rail.version >= 0
                    && matches!(rail.mode.as_str(), "dry_run" | "live")
                    && rail
                        .recipient_ref
                        .as_deref()
                        .is_none_or(valid_recipient_ref)
            });
        if !valid {
            return Err(PayoutLinkError::ResponseInvalid);
        }
        Ok(body)
    })
}

fn start_path(rail_id: &str) -> String {
    format!("{STATUS_PATH}/{rail_id}/link-requests")
}

fn check_started(
    started: &PayoutLinkStarted,
    rail_id: &str,
    action: PayoutLinkAction,
) -> Result<(), PayoutLinkError> {
    if started.contract_version != PAYOUT_LINK_CONTRACT_VERSION
        || started.rail_id != rail_id
        || started.action != action
        || started.outcome != PayoutLinkOutcome::PendingConfirmation
        || !valid_handle(&started.handle)
    {
        return Err(PayoutLinkError::ResponseInvalid);
    }
    Ok(())
}

/// Start linking `organization_id` on the `near_ai` rail. `access_token` is
/// a fresh NEAR AI access token the server spends once on `/users/me`; it is
/// sent in this one request body and nowhere else. The caller drops it after.
///
/// The answer must carry the recipient ref for `idempotency_key` and
/// `organization_id`, or it is refused as [`PayoutLinkError::RecipientMismatch`]
/// and must not be confirmed.
pub async fn start_near_ai_link(
    endpoint: &Endpoint<'_>,
    organization_id: Uuid,
    access_token: String,
    idempotency_key: Uuid,
    expected_current_version: Option<i64>,
) -> PayoutCall<PayoutLinkStarted> {
    let rail_id = trace_commons_protocol::payout_link::NEAR_AI_PAYOUT_RAIL_ID;
    let request = PayoutLinkRequest {
        action: PayoutLinkAction::Link,
        idempotency_key,
        expected_current_version,
        proof: Some(PayoutLinkProof::NearAi {
            organization_id,
            near_ai_access_token: access_token,
        }),
    };
    if request.validate(rail_id).is_err() {
        return PayoutCall::refused(PayoutLinkError::RequestInvalid);
    }
    let call = endpoint
        .call::<PayoutLinkRequest, PayoutLinkStarted>(
            Method::POST,
            &start_path(rail_id),
            Some(&request),
        )
        .await;
    drop(request);
    then(call, |started| {
        check_started(&started, rail_id, PayoutLinkAction::Link)?;
        match started.recipient_ref.as_deref() {
            Some(reference)
                if reference == near_ai_recipient_ref(idempotency_key, organization_id) =>
            {
                Ok(started)
            }
            Some(reference) if valid_recipient_ref(reference) => {
                Err(PayoutLinkError::RecipientMismatch)
            }
            _ => Err(PayoutLinkError::ResponseInvalid),
        }
    })
}

/// Start stopping payouts on `rail_id`. Carries no proof and no token.
pub async fn start_unlink(
    endpoint: &Endpoint<'_>,
    rail_id: &str,
    idempotency_key: Uuid,
    expected_current_version: Option<i64>,
) -> PayoutCall<PayoutLinkStarted> {
    let request = PayoutLinkRequest {
        action: PayoutLinkAction::Unlink,
        idempotency_key,
        expected_current_version,
        proof: None,
    };
    if !valid_rail_id(rail_id) || request.validate(rail_id).is_err() {
        return PayoutCall::refused(PayoutLinkError::RequestInvalid);
    }
    let call = endpoint
        .call::<PayoutLinkRequest, PayoutLinkStarted>(
            Method::POST,
            &start_path(rail_id),
            Some(&request),
        )
        .await;
    then(call, |started| {
        check_started(&started, rail_id, PayoutLinkAction::Unlink)?;
        Ok(started)
    })
}

/// Confirm the started request `handle` from the same account session.
pub async fn confirm(
    endpoint: &Endpoint<'_>,
    rail_id: &str,
    handle: &str,
    action: PayoutLinkAction,
) -> PayoutCall<PayoutLinkConfirmed> {
    if !valid_rail_id(rail_id) || !valid_handle(handle) {
        return PayoutCall::refused(PayoutLinkError::RequestInvalid);
    }
    let call = endpoint
        .call::<(), PayoutLinkConfirmed>(
            Method::POST,
            &format!("{}/{handle}/confirm", start_path(rail_id)),
            None,
        )
        .await;
    then(call, |confirmed| {
        if confirmed.contract_version != PAYOUT_LINK_CONTRACT_VERSION
            || confirmed.rail_id != rail_id
            || confirmed.action != action
            || confirmed.outcome != PayoutLinkOutcome::Confirmed
            || confirmed.version < 1
        {
            return Err(PayoutLinkError::ResponseInvalid);
        }
        Ok(confirmed)
    })
}

fn classify(error: &OcError) -> PayoutLinkError {
    let status = match error {
        OcError::ServerLabel { status, .. } | OcError::HttpFailure { status, .. } => *status,
        OcError::MalformedResponse { .. } => return PayoutLinkError::ResponseInvalid,
        _ => return PayoutLinkError::Unavailable,
    };
    // A refusal label from the protocol's closed set is kept by name, before
    // the session fallthrough: the verifier's refusals are 403s too.
    if let Some(outcome) = error
        .server_label()
        .and_then(|label| {
            serde_json::from_value::<PayoutLinkOutcome>(serde_json::Value::String(label.to_owned()))
                .ok()
        })
        .filter(|outcome| {
            !matches!(
                outcome,
                PayoutLinkOutcome::PendingConfirmation | PayoutLinkOutcome::Confirmed
            )
        })
    {
        return PayoutLinkError::Refused(outcome);
    }
    match status.as_u16() {
        // 401 is an expired or revoked session; 403 otherwise is a device
        // bearer or a cross-origin refusal. Either way the contributor needs
        // a fresh account sign-in, not a retry.
        401 | 403 => PayoutLinkError::SessionInvalid,
        _ => PayoutLinkError::Unavailable,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    //! A stub of the three payout-link routes on 127.0.0.1, recording what
    //! each call presented and sent. Shared with the daemon's tests.

    use std::sync::{Arc, Mutex};

    use axum::{
        Json, Router,
        extract::Path,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use trace_commons_protocol::payout_link::{
        PAYOUT_LINK_CONTRACT_VERSION, PayoutLinkProof, PayoutLinkRequest, near_ai_recipient_ref,
    };

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
        /// The account's link version on `near_ai`.
        pub version: i64,
        /// The newest link's recipient ref, as status reports it.
        pub recipient_ref: Option<String>,
        pub linked: bool,
        /// Overrides the start answer.
        pub start_reply: Option<(StatusCode, serde_json::Value)>,
        /// Overrides the confirm answer.
        pub confirm_reply: Option<(StatusCode, serde_json::Value)>,
        /// The handle the next start answers.
        pub handle: String,
        /// The action and recipient ref of the last started request.
        pub started: Option<(String, Option<String>)>,
        /// Set on every response, when present.
        pub rotate_to: Option<String>,
        /// The `near_ai` rail's `streams`, when the stub reports any.
        pub streams: Option<serde_json::Value>,
    }

    impl Default for Stub {
        fn default() -> Self {
            Self {
                version: 0,
                recipient_ref: None,
                linked: false,
                start_reply: None,
                confirm_reply: None,
                handle: "ab".repeat(32),
                started: None,
                rotate_to: None,
                streams: None,
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

    pub async fn spawn() -> Server {
        let stub = Arc::new(Mutex::new(Stub::default()));
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new()
            .route(
                "/v1/account/payout-rails",
                get({
                    let (stub, seen) = (Arc::clone(&stub), Arc::clone(&seen));
                    move |headers: HeaderMap| async move {
                        seen.lock().unwrap().push(Seen {
                            method: "GET",
                            path: "/v1/account/payout-rails".into(),
                            auth: auth(&headers),
                            body: String::new(),
                        });
                        let stub = stub.lock().unwrap();
                        let mut body = serde_json::json!({
                            "contract_version": PAYOUT_LINK_CONTRACT_VERSION,
                            "rails": [{
                                "rail_id": "near_ai",
                                "mode": "live",
                                "linked": stub.linked,
                                "recipient_ref": stub.recipient_ref,
                                "state": stub.linked.then_some("waiting"),
                                "version": stub.version,
                                "request_state": null,
                                "unlink_requested": false,
                                "recipient_unavailable": false,
                            }],
                        });
                        if let Some(streams) = &stub.streams {
                            body["rails"][0]["streams"] = streams.clone();
                        }
                        reply(&stub, StatusCode::OK, body)
                    }
                }),
            )
            .route(
                "/v1/account/payout-rails/{rail}/link-requests",
                post({
                    let (stub, seen) = (Arc::clone(&stub), Arc::clone(&seen));
                    move |Path(rail): Path<String>, headers: HeaderMap, body: String| async move {
                        seen.lock().unwrap().push(Seen {
                            method: "POST",
                            path: format!("/v1/account/payout-rails/{rail}/link-requests"),
                            auth: auth(&headers),
                            body: body.clone(),
                        });
                        let mut stub = stub.lock().unwrap();
                        if let Some((status, value)) = stub.start_reply.clone() {
                            return reply(&stub, status, value);
                        }
                        let Ok(request) = serde_json::from_str::<PayoutLinkRequest>(&body) else {
                            return reply(
                                &stub,
                                StatusCode::BAD_REQUEST,
                                serde_json::json!({"error": "invalid_payout_link_request"}),
                            );
                        };
                        let action = serde_json::to_value(request.action).unwrap();
                        let reference = match &request.proof {
                            Some(PayoutLinkProof::NearAi {
                                organization_id, ..
                            }) => Some(near_ai_recipient_ref(
                                request.idempotency_key,
                                *organization_id,
                            )),
                            None => None,
                        };
                        stub.started = Some((
                            action.as_str().unwrap().to_owned(),
                            reference.clone(),
                        ));
                        let value = serde_json::json!({
                            "contract_version": PAYOUT_LINK_CONTRACT_VERSION,
                            "rail_id": rail,
                            "action": action,
                            "outcome": "pending_confirmation",
                            "handle": stub.handle,
                            "expires_at": (chrono::Utc::now() + chrono::TimeDelta::minutes(5)).to_rfc3339(),
                            "recipient_ref": reference,
                        });
                        reply(&stub, StatusCode::OK, value)
                    }
                }),
            )
            .route(
                "/v1/account/payout-rails/{rail}/link-requests/{handle}/confirm",
                post({
                    let (stub, seen) = (Arc::clone(&stub), Arc::clone(&seen));
                    move |Path((rail, handle)): Path<(String, String)>,
                          headers: HeaderMap,
                          body: String| async move {
                        seen.lock().unwrap().push(Seen {
                            method: "POST",
                            path: format!(
                                "/v1/account/payout-rails/{rail}/link-requests/{handle}/confirm"
                            ),
                            auth: auth(&headers),
                            body,
                        });
                        let mut stub = stub.lock().unwrap();
                        if let Some((status, value)) = stub.confirm_reply.clone() {
                            return reply(&stub, status, value);
                        }
                        let Some((action, reference)) = stub.started.take() else {
                            return reply(
                                &stub,
                                StatusCode::NOT_FOUND,
                                serde_json::json!({"error": "not_found"}),
                            );
                        };
                        if handle != stub.handle {
                            return reply(
                                &stub,
                                StatusCode::NOT_FOUND,
                                serde_json::json!({"error": "not_found"}),
                            );
                        }
                        stub.version += 1;
                        if action == "link" {
                            stub.linked = true;
                            stub.recipient_ref = reference;
                        }
                        let value = serde_json::json!({
                            "contract_version": PAYOUT_LINK_CONTRACT_VERSION,
                            "rail_id": rail,
                            "action": action,
                            "outcome": "confirmed",
                            "version": stub.version,
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
    const TOKEN: &str = "synthetic-near-ai-access-SENTINEL";

    fn endpoint(base: &str) -> Endpoint<'_> {
        Endpoint {
            ingest_url: base,
            allowed_hosts: None,
            account_session_token: SESSION,
        }
    }

    fn org() -> Uuid {
        Uuid::parse_str("66666666-7777-4888-9999-aaaaaaaaaaaa").unwrap()
    }

    #[tokio::test]
    async fn every_route_presents_the_account_session_and_the_token_goes_only_to_start() {
        let server = spawn().await;
        let ep = endpoint(&server.base);
        assert_eq!(status(&ep).await.result.unwrap().rails[0].version, 0);
        let key = Uuid::new_v4();
        let started = start_near_ai_link(&ep, org(), TOKEN.into(), key, None)
            .await
            .result
            .unwrap();
        assert_eq!(
            started.recipient_ref.as_deref(),
            Some(near_ai_recipient_ref(key, org()).as_str())
        );
        let confirmed = confirm(&ep, "near_ai", &started.handle, PayoutLinkAction::Link)
            .await
            .result
            .unwrap();
        assert_eq!(confirmed.version, 1);
        let seen = server.seen.lock().unwrap();
        assert_eq!(seen.len(), 3);
        assert_eq!(
            seen.iter().map(|s| s.method).collect::<Vec<_>>(),
            ["GET", "POST", "POST"]
        );
        assert!(seen.iter().all(|s| s.auth == format!("Bearer {SESSION}")));
        let carrying: Vec<_> = seen.iter().filter(|s| s.body.contains(TOKEN)).collect();
        assert_eq!(carrying.len(), 1);
        assert!(carrying[0].path.ends_with("/link-requests"));
    }

    #[tokio::test]
    async fn a_start_answering_another_orgs_ref_is_refused() {
        let server = spawn().await;
        let other = near_ai_recipient_ref(Uuid::new_v4(), Uuid::new_v4());
        server.stub.lock().unwrap().start_reply = Some((
            StatusCode::OK,
            serde_json::json!({
                "contract_version": PAYOUT_LINK_CONTRACT_VERSION,
                "rail_id": "near_ai",
                "action": "link",
                "outcome": "pending_confirmation",
                "handle": "cd".repeat(32),
                "expires_at": chrono::Utc::now().to_rfc3339(),
                "recipient_ref": other,
            }),
        ));
        let ep = endpoint(&server.base);
        let result = start_near_ai_link(&ep, org(), TOKEN.into(), Uuid::new_v4(), None)
            .await
            .result;
        assert_eq!(result.unwrap_err(), PayoutLinkError::RecipientMismatch);
    }

    #[tokio::test]
    async fn refusals_keep_the_protocol_label_and_never_a_body() {
        let server = spawn().await;
        let ep = endpoint(&server.base);
        for (status_code, label, expected) in [
            (403, "org_not_owned", "org_not_owned"),
            (403, "subject_mismatch", "subject_mismatch"),
            (
                403,
                "payout_account_ineligible",
                "payout_account_ineligible",
            ),
            (409, "version_conflict", "version_conflict"),
            (404, "payout_rail_unknown", "payout_rail_unknown"),
            (503, "payout_unavailable", "payout_unavailable"),
            (403, "account session required", "account-session-required"),
            (
                403,
                "cross-origin account mutation",
                "account-session-required",
            ),
            (401, "unauthorized", "account-session-required"),
            (
                500,
                "boom at https://internal.example/x",
                "payout-link-unavailable",
            ),
            (403, "confirmed", "account-session-required"),
        ] {
            server.stub.lock().unwrap().start_reply = Some((
                StatusCode::from_u16(status_code).unwrap(),
                serde_json::json!({ "error": label }),
            ));
            let error = start_near_ai_link(&ep, org(), TOKEN.into(), Uuid::new_v4(), Some(2))
                .await
                .result
                .unwrap_err();
            assert_eq!(error.label(), expected, "{status_code} {label}");
        }
    }

    #[tokio::test]
    async fn a_malformed_request_never_leaves_the_machine() {
        let server = spawn().await;
        let ep = endpoint(&server.base);
        for call in [
            start_near_ai_link(&ep, Uuid::nil(), TOKEN.into(), Uuid::new_v4(), None)
                .await
                .result
                .map(|_| ()),
            start_near_ai_link(&ep, org(), "  ".into(), Uuid::new_v4(), None)
                .await
                .result
                .map(|_| ()),
            start_unlink(&ep, "../admin", Uuid::new_v4(), None)
                .await
                .result
                .map(|_| ()),
            confirm(&ep, "near_ai", "../../x", PayoutLinkAction::Link)
                .await
                .result
                .map(|_| ()),
        ] {
            assert_eq!(call.unwrap_err(), PayoutLinkError::RequestInvalid);
        }
        assert!(server.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_rotated_session_is_handed_back_on_a_refusal_too() {
        let server = spawn().await;
        {
            let mut stub = server.stub.lock().unwrap();
            stub.rotate_to = Some("tcn1_rotated.session".into());
            stub.start_reply = Some((
                StatusCode::CONFLICT,
                serde_json::json!({"error": "version_conflict"}),
            ));
        }
        let call = start_unlink(&endpoint(&server.base), "near_ai", Uuid::new_v4(), Some(1)).await;
        assert!(call.result.is_err());
        assert_eq!(
            call.rotated_token.as_ref().map(|r| r.token.as_str()),
            Some("tcn1_rotated.session")
        );
    }

    #[test]
    fn identifiers_are_checked_before_they_become_paths() {
        assert!(valid_rail_id("near_ai"));
        for bad in ["", "Near", "near-ai", "../x", "1rail", &"a".repeat(33)] {
            assert!(!valid_rail_id(bad), "{bad}");
        }
        assert!(valid_handle(&"ab".repeat(32)));
        assert!(!valid_handle(&"AB".repeat(32)));
        assert!(valid_recipient_ref(&format!("sha256:{}", "0".repeat(64))));
        assert!(!valid_recipient_ref(&"0".repeat(64)));
    }
}
