//! A handler panic reaches the client as an error, not a dropped socket.
//!
//! Without this, a panic inside a façade handler unwinds to the connection
//! task, hyper drops the socket, and the client sees `ECONNRESET` with no
//! status, no body and no ledger row. Coding agents treat a reset as a
//! transport fault and retry the same request indefinitely, so one request
//! that always panics leaves the session stuck with nothing on screen but
//! the reset.
//!
//! Only the handler future is guarded: a panic while a streamed body is
//! being polled, after the status line has gone out, still ends the
//! connection. And only an unwinding build can catch anything here; under
//! `panic = "abort"` (this workspace's own release profile) the process ends
//! either way. An application embedding the proxy builds it under its own
//! profile, which is where this matters.
//!
//! The same layer makes sure every exchange leaves exactly one ledger row.
//! A handler that routes a request arranges its own row (the refusal, or the
//! row the response body writes when it ends or is dropped) and says so on the
//! request's [`RowClaim`]. When nothing claimed the row, the guard writes it,
//! with a status and one of the fixed codes below and nothing else:
//!
//! - [`PANIC`]: the handler panicked (status 500).
//! - [`BODY_REJECTED`]: the body was refused before the handler ran for being
//!   over the size limit (413).
//! - [`INVALID_JSON`] and [`UNKNOWN_LOGIN`]: the handler refused the request
//!   before routing it (400, 404).
//! - [`CLIENT_DISCONNECTED`]: the client went away before the response began,
//!   either mid-upload (the body read failed) or after the request was in, so
//!   the handler's future was dropped (status 499, which no client saw).
//! - [`UNRECORDED`]: a handler answered without arranging a row. No handler
//!   does this today; the code keeps such a bug visible instead of silent.
//!
//! Only `POST` requests to a route that exists are exchanges. A model listing,
//! and the router's own 404 for an unknown path or 405 for a wrong method,
//! leave no row, as before. (A 404 a handler answers, for a login-bound URL
//! naming no login, is claimed under [`UNKNOWN_LOGIN`] and is recorded.)

use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures_util::FutureExt;
use ironwire_ledger::Ledger;

use crate::facade::error::FacadeError;
use crate::pipeline::Refusal;

/// The handler panicked.
pub(crate) const PANIC: &str = "panic";
/// The body was refused before the handler ran, over the size limit.
pub(crate) const BODY_REJECTED: &str = "body_rejected";
/// The body was read but is not JSON.
pub(crate) const INVALID_JSON: &str = "invalid_json";
/// A login-bound URL names no login of the façade's product.
pub(crate) const UNKNOWN_LOGIN: &str = "unknown_login";
/// The client went away before the response began.
pub(crate) const CLIENT_DISCONNECTED: &str = "client_disconnected";
/// A handler answered without arranging a row.
pub(crate) const UNRECORDED: &str = "unrecorded";

/// The status recorded for [`CLIENT_DISCONNECTED`]: the common "client closed
/// request" convention. Nothing was sent, so no client ever saw it.
const CLOSED_BY_CLIENT: u16 = 499;

/// What a façade handler has done about its request's ledger row, shared with
/// the guard through the request's extensions.
///
/// A handler reached without the guard (and so without a ledger to write to)
/// gets a default claim nobody reads, which is harmless.
#[derive(Debug, Clone, Default)]
pub(crate) struct RowClaim(Arc<Mutex<Claim>>);

#[derive(Debug, Default)]
struct Claim {
    /// A row is written, or the response body will write it when it ends or
    /// is dropped.
    arranged: bool,
    /// The code the handler refused the request under.
    failure: Option<&'static str>,
    conversation: Option<String>,
    client_session_id: Option<String>,
    requested_model: Option<String>,
}

impl RowClaim {
    /// The claim on `claim`, or a detached one when the guard did not run.
    pub(crate) fn from_extension(claim: Option<axum::Extension<Self>>) -> Self {
        claim
            .map(|axum::Extension(claim)| claim)
            .unwrap_or_default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Claim> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The handler has written this request's row or handed it to the
    /// response body. The guard writes nothing.
    pub(crate) fn arranged(&self) {
        self.lock().arranged = true;
    }

    /// The handler refused the request under `failure` without writing a row.
    pub(crate) fn failed(&self, failure: &'static str) {
        self.lock().failure = Some(failure);
    }

    /// The client went away while the request was still being read. Its status
    /// is [`CLOSED_BY_CLIENT`] whatever the extractor answered, since nothing
    /// that was answered reached anyone.
    pub(crate) fn disconnected(&self) {
        self.lock().failure = Some(CLIENT_DISCONNECTED);
    }

    /// What the handler has learned about the request, kept for the row the
    /// guard writes if the handler never finishes. Identifiers only.
    pub(crate) fn describe(
        &self,
        conversation: String,
        client_session_id: Option<String>,
        requested_model: Option<String>,
    ) {
        let mut claim = self.lock();
        claim.conversation = Some(conversation);
        claim.client_session_id = client_session_id;
        claim.requested_model = requested_model;
    }
}

/// Which façade's error envelope a caught panic is rendered in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Envelope {
    /// `{"type":"error","error":{...}}`.
    Anthropic,
    /// `{"error":{...}}`.
    OpenAi,
}

/// The message a client is shown. It never includes the panic's own text,
/// which can quote request content (a slicing panic prints the string it
/// was slicing).
pub(crate) const PANIC_MESSAGE: &str = "IronWire failed internally while handling this request. \
     This is an IronWire bug, not a provider error; repeating the same request is \
     unlikely to succeed.";

impl Envelope {
    /// The façade name the ledger records.
    fn facade(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
        }
    }
}

/// `router`, with every handler's panic turned into a 500 in `envelope`, and
/// every exchange that would otherwise leave no row recorded in `ledger`.
pub(crate) fn catch_panics<S>(
    router: Router<S>,
    envelope: Envelope,
    ledger: Option<Ledger>,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    crate::panic_hook::chain_for_guarded_handlers();
    router.layer(axum::middleware::from_fn(
        move |request: Request, next: Next| guard(envelope, ledger.clone(), request, next),
    ))
}

/// The row a request will leave if its handler does not arrange one.
///
/// Held by the guard's own future, so it is dropped when that future is: by
/// hyper, when the client closes the connection before the response. Its
/// `Drop` is the only code that runs then.
struct PendingRow {
    claim: RowClaim,
    ledger: Ledger,
    facade: &'static str,
    path: String,
    started_at: chrono::DateTime<chrono::Utc>,
    started: std::time::Instant,
    settled: bool,
}

impl PendingRow {
    /// A pending row for `request`, with its claim put where the handler can
    /// reach it, or `None` for a request that is not an exchange.
    fn arm(request: &mut Request, envelope: Envelope, ledger: Option<Ledger>) -> Option<Self> {
        let ledger = ledger?;
        if request.method() != Method::POST {
            return None;
        }
        let facade = envelope.facade();
        let path = recorded_path(request.uri().path(), facade);
        let claim = RowClaim::default();
        request.extensions_mut().insert(claim.clone());
        Some(Self {
            claim,
            ledger,
            facade,
            path,
            started_at: chrono::Utc::now(),
            started: std::time::Instant::now(),
            settled: false,
        })
    }

    /// The handler finished with `status`, or panicked.
    fn settle(mut self, status: StatusCode, panicked: bool) {
        self.settled = true;
        let failure = {
            let claim = self.claim.lock();
            if claim.arranged {
                return;
            }
            if panicked {
                PANIC
            } else if let Some(failure) = claim.failure {
                failure
            } else if matches!(
                status,
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ) {
                // The router's own answer: no route took the request, so
                // there was no exchange.
                return;
            } else if status.is_client_error() {
                // The body extractor is the one fallible extractor that
                // answers before a façade handler runs, and a read failure
                // there is claimed as a disconnect, so what is left is 413.
                BODY_REJECTED
            } else {
                UNRECORDED
            }
        };
        let status = if failure == CLIENT_DISCONNECTED {
            CLOSED_BY_CLIENT
        } else {
            status.as_u16()
        };
        self.write(status, failure);
    }

    fn write(&self, status: u16, failure: &'static str) {
        let (conversation, client_session_id, requested_model) = {
            let claim = self.claim.lock();
            (
                claim.conversation.clone(),
                claim.client_session_id.clone(),
                claim.requested_model.clone(),
            )
        };
        Refusal {
            started_at: self.started_at,
            started: self.started,
            facade: self.facade,
            path: self.path.clone(),
            conversation: conversation.unwrap_or_else(|| Refusal::NO_CONVERSATION.to_string()),
            client_session_id,
            requested_model,
            status,
        }
        .write_failure(&self.ledger, failure);
    }
}

impl Drop for PendingRow {
    fn drop(&mut self) {
        if self.settled || self.claim.lock().arranged {
            return;
        }
        self.write(CLOSED_BY_CLIENT, CLIENT_DISCONNECTED);
    }
}

/// The path a row records: beneath the façade and without the account
/// segment, as the handlers record it. The account is the one segment of a
/// façade route the client chooses, so it never reaches the row.
fn recorded_path(path: &str, facade: &str) -> String {
    // A nested router sees the path beneath its prefix; this also accepts the
    // full path, so the answer does not depend on how the router is mounted.
    let beneath = path
        .strip_prefix('/')
        .and_then(|rest| rest.strip_prefix(facade))
        .filter(|rest| rest.starts_with('/'))
        .unwrap_or(path);
    match beneath.strip_prefix("/account/") {
        Some(rest) => rest
            .find('/')
            .map_or_else(String::new, |at| rest[at..].to_string()),
        None => beneath.to_string(),
    }
}

fn caught(envelope: Envelope) -> Response {
    let error = FacadeError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "api_error",
        PANIC_MESSAGE,
    );
    match envelope {
        Envelope::Anthropic => error.into_response(),
        Envelope::OpenAi => error.on_openai_facade(false).into_response(),
    }
}

async fn guard(
    envelope: Envelope,
    ledger: Option<Ledger>,
    mut request: Request,
    next: Next,
) -> Response {
    let pending = PendingRow::arm(&mut request, envelope, ledger);
    // Before this guard, the connection task caught the same unwind and the
    // daemon carried on serving other requests; this changes only what the
    // client is told, not whether the daemon continues.
    // `Guarded` keeps the panic's message out of the log: the hook runs
    // before this catch, and std's default one prints the message.
    match AssertUnwindSafe(crate::panic_hook::Guarded::new(next.run(request)))
        .catch_unwind()
        .await
    {
        Ok(response) => {
            if let Some(pending) = pending {
                pending.settle(response.status(), false);
            }
            response
        }
        Err(_payload) => {
            // The payload is dropped unread: its text can quote the request.
            // The hook has already reported the location, and only that
            // (`crate::panic_hook`).
            tracing::error!("a façade handler panicked; answered with a 500");
            if let Some(pending) = pending {
                pending.settle(StatusCode::INTERNAL_SERVER_ERROR, true);
            }
            caught(envelope)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::post;
    use tower::ServiceExt;

    /// The text a handler panics with: it stands for request content, which
    /// must not reach the client or the log. These tests hold the client to
    /// that; the log is the panic hook's, and `crate::panic_hook`'s tests
    /// hold it there in a child process, since a hook is process-wide.
    const SECRET: &str = "prompt text that must not leak";

    async fn boom() -> StatusCode {
        panic!("{SECRET}")
    }

    fn panicking() -> Router {
        Router::new()
            .route("/boom", post(boom))
            .route("/fine", post(|| async { (StatusCode::CREATED, "served") }))
    }

    async fn call(router: Router, path: &str) -> (StatusCode, String) {
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("served");
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        (status, String::from_utf8_lossy(&body).into_owned())
    }

    #[tokio::test]
    async fn a_panic_on_the_openai_facade_is_an_openai_shaped_500() {
        let (status, body) = call(catch_panics(panicking(), Envelope::OpenAi, None), "/boom").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["error"]["type"], "api_error");
        assert_eq!(value["error"]["message"], PANIC_MESSAGE);
        assert!(
            value.get("type").is_none(),
            "Anthropic envelope on OpenAI: {body}"
        );
    }

    #[tokio::test]
    async fn a_panic_on_the_anthropic_facade_is_an_anthropic_shaped_500() {
        let (status, body) = call(
            catch_panics(panicking(), Envelope::Anthropic, None),
            "/boom",
        )
        .await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["type"], "error");
        assert_eq!(value["error"]["type"], "api_error");
        assert_eq!(value["error"]["message"], PANIC_MESSAGE);
    }

    #[tokio::test]
    async fn the_panic_text_never_reaches_the_client() {
        for envelope in [Envelope::OpenAi, Envelope::Anthropic] {
            let (_, body) = call(catch_panics(panicking(), envelope, None), "/boom").await;
            assert!(!body.contains(SECRET), "panic text leaked: {body}");
        }
    }

    #[tokio::test]
    async fn a_handler_that_does_not_panic_is_untouched() {
        let (status, body) = call(catch_panics(panicking(), Envelope::OpenAi, None), "/fine").await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body, "served");
    }

    /// A handler that has learned the request's model, then panics.
    async fn boom_after_describing(claim: Option<axum::Extension<RowClaim>>) -> StatusCode {
        let claim = RowClaim::from_extension(claim);
        claim.describe(
            "c-1".into(),
            Some("session-1".into()),
            Some("claude-opus-4-6".into()),
        );
        panic!("{SECRET}")
    }

    /// A handler that arranged its row (as a routed response does), then
    /// panics. The arranged row is the request's one row.
    async fn boom_after_arranging(claim: Option<axum::Extension<RowClaim>>) -> StatusCode {
        RowClaim::from_extension(claim).arranged();
        panic!("{SECRET}")
    }

    fn recording() -> Router {
        panicking()
            .route("/v1/messages", post(boom_after_describing))
            .route("/account/{account}/v1/messages", post(boom))
            .route("/arranged", post(boom_after_arranging))
    }

    #[tokio::test]
    async fn a_panic_leaves_one_row_with_its_code_and_no_content() {
        let ledger = Ledger::in_memory().expect("ledger");
        let router = catch_panics(recording(), Envelope::Anthropic, Some(ledger.clone()));
        let (status, _) = call(router, "/v1/messages").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let rows = ledger.recent(10).expect("reads");
        assert_eq!(rows.len(), 1, "{rows:#?}");
        let row = &rows[0];
        assert_eq!(row.status, 500);
        assert_eq!(row.failure.as_deref(), Some(PANIC));
        assert_eq!(row.error, None);
        assert_eq!(row.facade, "anthropic");
        assert_eq!(row.path, "/v1/messages");
        assert_eq!(row.backend, "none");
        assert_eq!(row.requested_model.as_deref(), Some("claude-opus-4-6"));
        assert_eq!(row.client_session_id.as_deref(), Some("session-1"));
        let rendered = serde_json::to_string(row).expect("serializes");
        assert!(!rendered.contains(SECRET), "panic text reached the ledger");
    }

    #[tokio::test]
    async fn a_panic_before_anything_is_known_still_leaves_a_row() {
        let ledger = Ledger::in_memory().expect("ledger");
        let router = catch_panics(recording(), Envelope::OpenAi, Some(ledger.clone()));
        let (status, _) = call(router, "/account/someone/v1/messages").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let rows = ledger.recent(10).expect("reads");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].failure.as_deref(), Some(PANIC));
        assert_eq!(rows[0].facade, "openai");
        assert_eq!(rows[0].path, "/v1/messages", "the account is not recorded");
        assert_eq!(rows[0].conversation, Refusal::NO_CONVERSATION);
        assert_eq!(rows[0].requested_model, None);
    }

    #[tokio::test]
    async fn an_arranged_row_is_not_written_twice() {
        let ledger = Ledger::in_memory().expect("ledger");
        let router = catch_panics(recording(), Envelope::OpenAi, Some(ledger.clone()));
        let (status, _) = call(router, "/arranged").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(
            ledger.recent(10).expect("reads").is_empty(),
            "the handler's own row is the one; the guard adds none"
        );
    }

    #[test]
    fn the_recorded_path_drops_the_facade_prefix_and_the_account() {
        assert_eq!(recorded_path("/v1/messages", "anthropic"), "/v1/messages");
        assert_eq!(
            recorded_path("/anthropic/v1/messages", "anthropic"),
            "/v1/messages"
        );
        assert_eq!(
            recorded_path("/account/claude-work/v1/messages/count_tokens", "anthropic"),
            "/v1/messages/count_tokens"
        );
        assert_eq!(
            recorded_path("/openai/account/x/v1/responses", "openai"),
            "/v1/responses"
        );
        assert_eq!(recorded_path("/account/only", "openai"), "");
    }

    #[tokio::test]
    async fn the_router_keeps_serving_after_a_panic() {
        let router = catch_panics(panicking(), Envelope::OpenAi, None);
        let (first, _) = call(router.clone(), "/boom").await;
        assert_eq!(first, StatusCode::INTERNAL_SERVER_ERROR);
        let (second, _) = call(router, "/fine").await;
        assert_eq!(second, StatusCode::CREATED);
    }
}
