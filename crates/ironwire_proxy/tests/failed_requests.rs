//! Every request that reaches a façade leaves exactly one ledger row.
//!
//! The ledger feeds every surface that shows a user what their agent did, so a
//! request that fails without a row fails invisibly. These are the paths that
//! used to leave none: a body rejected before the handler ran, a body that is
//! not JSON, a bound URL naming no login, and a client that hung up before the
//! response began. A handler panic is covered beside the guard itself
//! (`facade::guard`).
//!
//! Each row carries a status and a fixed reason code, and never any request
//! content or error text.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::{Exchange, Ledger};
use ironwire_proxy::facade::MAX_REQUEST_BODY_BYTES;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use tower::ServiceExt;

/// Text that stands for the user's prompt: it must not reach any row.
const SECRET: &str = "prompt text that must not reach the ledger";

fn state_for(base_url: &str, ledger: Ledger) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test-key"),
            Some(base_url.to_string()),
            30,
        )
        .expect("client builds"),
    ));
    AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger))
}

/// No upstream listens here; the paths under test never reach one.
const NOWHERE: &str = "http://127.0.0.1:9";

fn post(uri: &str, body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.into())
        .expect("request builds")
}

/// The one row the ledger holds, with the assertions every failure row shares.
fn only_row(ledger: &Ledger) -> Exchange {
    let rows = ledger.recent(10).expect("reads");
    assert_eq!(rows.len(), 1, "expected exactly one row, got {rows:#?}");
    let row = rows.into_iter().next().expect("one row");
    assert_eq!(
        row.error, None,
        "a failure row carries a code, not error text"
    );
    assert_eq!(row.backend, "none");
    assert_eq!(row.attempts, 0);
    assert_eq!(row.input_tokens, None);
    assert_eq!(row.proof, None);
    let rendered = serde_json::to_string(&row).expect("serializes");
    assert!(
        !rendered.contains(SECRET),
        "request content reached the ledger: {rendered}"
    );
    row
}

/// The four inference routes, one per façade lane.
const PATHS: [&str; 4] = [
    "/anthropic/v1/messages",
    "/anthropic/v1/messages/count_tokens",
    "/openai/v1/chat/completions",
    "/openai/v1/responses",
];

#[tokio::test]
async fn a_body_over_the_limit_is_recorded_as_rejected() {
    // Over the façades' own limit, refused by `FacadeBody` before the handler
    // runs, in the façade's JSON envelope.
    let mut body =
        format!(r#"{{"model":"claude-opus-4-6","messages":[{{"role":"user","content":"{SECRET}"#);
    body.push_str(&"x".repeat(MAX_REQUEST_BODY_BYTES));
    body.push_str(r#""}]}"#);
    let body = bytes::Bytes::from(body);

    for path in PATHS {
        let ledger = Ledger::in_memory().expect("ledger opens");
        let response = app(state_for(NOWHERE, ledger.clone()))
            .oneshot(post(path, body.clone()))
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE, "{path}");
        let answer = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let error: serde_json::Value = serde_json::from_slice(&answer)
            .unwrap_or_else(|_| panic!("{path}: not JSON: {}", String::from_utf8_lossy(&answer)));
        assert_eq!(error["error"]["type"], "invalid_request_error", "{path}");

        let row = only_row(&ledger);
        assert_eq!(row.status, 413, "{path}");
        assert_eq!(row.failure.as_deref(), Some("body_rejected"), "{path}");
        let (facade, beneath) = path[1..].split_once('/').expect("a façade path");
        assert_eq!(row.facade, facade);
        assert_eq!(row.path, format!("/{beneath}"));
        assert_eq!(
            row.requested_model, None,
            "the body was never read, so no model is known"
        );
    }
}

#[tokio::test]
async fn a_client_that_hangs_up_mid_body_is_recorded_as_disconnected() {
    for path in PATHS {
        let ledger = Ledger::in_memory().expect("ledger opens");
        let broken = Body::from_stream(futures_util::stream::iter([
            Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"{\"model\":")),
            Err(std::io::Error::other("the client's stream broke")),
        ]));
        let response = app(state_for(NOWHERE, ledger.clone()))
            .oneshot(post(path, broken))
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");

        let row = only_row(&ledger);
        assert_eq!(
            row.status, 499,
            "{path}: no client saw the 400 the extractor answered"
        );
        assert_eq!(
            row.failure.as_deref(),
            Some("client_disconnected"),
            "{path}: not `body_rejected` or the `unrecorded` backstop"
        );
        assert!(
            !serde_json::to_string(&row)
                .expect("serializes")
                .contains("broke"),
            "{path}: the read error's text reached the row"
        );
    }
}

#[tokio::test]
async fn a_body_that_is_not_json_is_recorded() {
    let ledger = Ledger::in_memory().expect("ledger opens");
    let response = app(state_for(NOWHERE, ledger.clone()))
        .oneshot(post("/openai/v1/responses", format!("{{{SECRET}")))
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let row = only_row(&ledger);
    assert_eq!(row.status, 400);
    assert_eq!(row.failure.as_deref(), Some("invalid_json"));
    assert_eq!(row.facade, "openai");
    assert_eq!(row.path, "/v1/responses");
}

#[tokio::test]
async fn a_bound_url_naming_no_login_is_recorded() {
    let ledger = Ledger::in_memory().expect("ledger opens");
    let response = app(state_for(NOWHERE, ledger.clone()))
        .oneshot(post(
            "/anthropic/account/nobody/v1/messages",
            r#"{"model":"claude-opus-4-6","messages":[]}"#,
        ))
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let row = only_row(&ledger);
    assert_eq!(row.status, 404);
    assert_eq!(row.failure.as_deref(), Some("unknown_login"));
    assert_eq!(
        row.path, "/v1/messages",
        "the account segment is not part of the recorded path"
    );
}

/// An upstream that accepts the connection and never answers, so the request
/// is still waiting on dispatch when the client goes away.
async fn spawn_silent_upstream() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            held.push(socket);
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn a_client_that_hangs_up_before_the_response_is_recorded() {
    let base_url = spawn_silent_upstream().await;
    let ledger = Ledger::in_memory().expect("ledger opens");
    let router = app(state_for(&base_url, ledger.clone()));
    let body = format!(
        r#"{{"model":"claude-opus-4-6","messages":[{{"role":"user","content":"{SECRET}"}}]}}"#
    );

    let request = tokio::spawn(router.oneshot(post("/anthropic/v1/messages", body)));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        ledger.recent(10).expect("reads").is_empty(),
        "still in flight"
    );
    // What hyper does to a handler whose client closed the socket.
    request.abort();
    let _ = request.await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while ledger.recent(10).expect("reads").is_empty() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a request abandoned before its response left no row"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let row = only_row(&ledger);
    assert_eq!(row.status, 499);
    assert_eq!(row.failure.as_deref(), Some("client_disconnected"));
    assert_eq!(row.path, "/v1/messages");
    assert_eq!(
        row.requested_model.as_deref(),
        Some("claude-opus-4-6"),
        "what the handler had already read is kept"
    );
}

#[tokio::test]
async fn a_routing_refusal_still_writes_its_one_row() {
    // No backend at all: the existing 503 refusal path.
    let ledger = Ledger::in_memory().expect("ledger opens");
    let state = AppState::new(
        BackendRegistry::new(),
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger.clone()));
    let response = app(state)
        .oneshot(post(
            "/anthropic/v1/messages",
            r#"{"model":"claude-opus-4-6","messages":[]}"#,
        ))
        .await
        .expect("served");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let rows = ledger.recent(10).expect("reads");
    assert_eq!(rows.len(), 1, "a refusal is one row, not two: {rows:#?}");
    assert_eq!(rows[0].status, 503);
    assert!(rows[0].error.is_some(), "a refusal keeps its reason text");
    assert_eq!(rows[0].failure, None);
}

#[tokio::test]
async fn listings_and_unknown_paths_are_not_exchanges() {
    let ledger = Ledger::in_memory().expect("ledger opens");
    let router = app(state_for(NOWHERE, ledger.clone()));
    for request in [
        Request::builder()
            .uri("/anthropic/v1/models")
            .body(Body::empty())
            .expect("request builds"),
        Request::builder()
            .uri("/openai/v1/models")
            .body(Body::empty())
            .expect("request builds"),
        post("/anthropic/v1/nothing-here", "{}"),
        Request::builder()
            .method("PUT")
            .uri("/anthropic/v1/messages")
            .body(Body::empty())
            .expect("request builds"),
    ] {
        let _ = router.clone().oneshot(request).await.expect("served");
    }
    assert!(
        ledger.recent(10).expect("reads").is_empty(),
        "a model listing, a 404 or a 405 is not an exchange"
    );
}
