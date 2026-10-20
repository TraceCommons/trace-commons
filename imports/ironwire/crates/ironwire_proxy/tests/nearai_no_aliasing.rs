//! Model aliasing on NEAR AI: observed by default, refused only on request.
//!
//! The gateway resolves a requested model name against every catalogue entry's
//! alias list. On a hit it serves the *canonical* model and rewrites the
//! response body to add a top-level `warning` field. cloud-api states the
//! consequence itself, on `inject_warning_field`: the rewritten body "no longer
//! byte-matches what the backend TD signed, so response-hash verification will
//! not pass for aliased responses ... strict clients can avoid it entirely with
//! `x-no-aliasing`."
//!
//! Two different things follow from that, and they are deliberately not the
//! same setting.
//!
//! **Observing it is unconditional.** The gateway announces the substitution in
//! `x-model-alias-resolved` on every aliased response, so the exchange can be
//! recorded as one whose receipt will not verify. Reading a response header
//! costs the caller nothing and refuses nothing.
//!
//! **Refusing it is opt-in.** `x-no-aliasing` does not make the gateway serve
//! what was asked for -- it makes the gateway answer 400. Most traffic through
//! this proxy is somebody getting work done, and turning their working call
//! into a hard failure to protect an evidence property they may not be using is
//! not a trade IronWire makes for them. An operator who would rather have no
//! answer than an unverifiable one sets `refuse_model_aliases`.

use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_core::protocol::{BackendId, ModelTier};
use ironwire_creds::ConsentLedger;
use ironwire_ledger::Ledger;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::observe::MODEL_ALIAS_RESOLVED_HEADER;
use ironwire_upstream::openai_chat::{ChatCompletionsBackend, NO_ALIASING_HEADER};
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

const RESPONSES_SSE: &str = concat!(
    "event: response.created\n",
    r#"data: {"type":"response.created","response":{"id":"resp_1","model":"qwen3-coder"}}"#,
    "\n\n",
    "event: response.completed\n",
    r#"data: {"type":"response.completed","response":{"id":"resp_1","usage":{"input_tokens":4,"output_tokens":1}}}"#,
    "\n\n",
);

const CHAT_SSE: &str = concat!(
    r#"data: {"id":"c1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"content":"hi"}}]}"#,
    "\n\n",
    "data: [DONE]\n\n",
);

/// What an aliased response announces, in cloud-api's own spelling.
const ALIAS_NOTICE: &str = "openai/gpt-5 -> Qwen/Qwen3.8-27B";

/// An upstream that records the request head verbatim and replies with the
/// headers it is given.
///
/// Hand-rolled rather than mounted on a framework because the claim is about
/// the exact set of headers on the wire; a framework that normalised or added
/// one would make the assertion mean something else.
async fn spawn_upstream(
    sse: &'static str,
    extra_response_headers: &'static str,
) -> (String, Arc<Mutex<Option<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let head_seen = Arc::new(Mutex::new(None));
    let sink = Arc::clone(&head_seen);

    tokio::spawn(async move {
        let Ok((mut socket, _)) = listener.accept().await else {
            return;
        };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let Ok(n) = socket.read(&mut chunk).await else {
                return;
            };
            if n == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..n]);
            let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4) else {
                continue;
            };
            let head = String::from_utf8_lossy(&buf[..split]).to_string();
            let length = head
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                .and_then(|l| l.split(':').nth(1))
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if buf.len() - split >= length {
                *sink.lock().expect("lock") = Some(head);
                break;
            }
        }

        let response = format!(
            "HTTP/1.1 200 OK\r\n\
             content-type: text/event-stream\r\n\
             {extra_response_headers}\
             content-length: {}\r\n\r\n",
            sse.len()
        );
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.write_all(sse.as_bytes()).await;
        let _ = socket.flush().await;
    });

    // `/v1` included: that is the shape of `NEARAI_DEFAULT_BASE_URL`, and
    // `endpoint_url` composes against it.
    (format!("http://{addr}/v1"), head_seen)
}

fn state_with(backend: ChatCompletionsBackend) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(backend));
    AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(Ledger::in_memory().expect("ledger opens")))
}

fn nearai_at(base_url: &str) -> ChatCompletionsBackend {
    ChatCompletionsBackend::nearai(
        Some(SecretString::from("near-key")),
        Some(base_url.to_string()),
        vec![("qwen3-coder".to_string(), ModelTier::Frontier)],
        30,
    )
    .expect("client builds")
}

/// Every value the upstream saw under `name`.
fn header_values(head: &str, name: &str) -> Vec<String> {
    head.lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.trim().eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_string())
        .collect()
}

async fn head_for(
    state: &AppState,
    request: Request<Body>,
    seen: &Mutex<Option<String>>,
) -> String {
    let response = app(state.clone())
        .oneshot(request)
        .await
        .expect("the proxy answers");
    assert_eq!(response.status(), StatusCode::OK, "the request was served");
    let _ = axum::body::to_bytes(response.into_body(), 1 << 20).await;
    seen.lock()
        .expect("lock")
        .clone()
        .expect("the upstream saw a request")
}

fn responses_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/openai/v1/responses")
        .header("content-type", "application/json")
        .header("originator", "codex_cli_rs")
        .body(Body::from(
            r#"{"model":"qwen3-coder","stream":true,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}]}"#,
        ))
        .expect("request builds")
}

fn chat_request() -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/openai/v1/chat/completions")
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"qwen3-coder","stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
        ))
        .expect("request builds")
}

/// The first ledger row, once the pipeline has written it.
async fn recorded(state: &AppState) -> ironwire_ledger::Exchange {
    let ledger = state.ledger.clone().expect("ledger");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(row) = ledger.recent(1).expect("reads").first() {
            return row.clone();
        }
        assert!(tokio::time::Instant::now() < deadline, "no ledger entry");
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

// -- Detection, which is the default ----------------------------------------

/// The whole point of the default: the caller's turn succeeds, and the row
/// still says the answer came from a model they did not name.
#[tokio::test]
async fn an_aliased_response_is_recorded_without_refusing_the_turn() {
    let alias_header = "x-model-alias-resolved: openai/gpt-5 -> Qwen/Qwen3.8-27B\r\n";
    let (base, seen) = spawn_upstream(CHAT_SSE, alias_header).await;
    let state = state_with(nearai_at(&base));

    let head = head_for(&state, chat_request(), &seen).await;
    assert!(
        header_values(&head, NO_ALIASING_HEADER).is_empty(),
        "the turn must not have been refused to get this row: {head}"
    );

    let row = recorded(&state).await;
    assert_eq!(
        row.model_alias_resolved.as_deref(),
        Some(ALIAS_NOTICE),
        "the gateway said it substituted a model and the row lost it"
    );
}

/// The control that makes the case above mean something.
///
/// A row that carried a value whether or not the gateway sent the header would
/// be a constant wearing a test's clothes. The only difference between this
/// case and the one above is the response header.
#[tokio::test]
async fn an_ordinary_response_records_no_substitution() {
    let (base, seen) = spawn_upstream(CHAT_SSE, "").await;
    let state = state_with(nearai_at(&base));
    let _ = head_for(&state, chat_request(), &seen).await;

    let row = recorded(&state).await;
    assert_eq!(
        row.model_alias_resolved, None,
        "a substitution was recorded that the gateway never announced"
    );
}

// -- Refusal, which is not ---------------------------------------------------

/// Default off, on Codex's wire.
#[tokio::test]
async fn a_responses_request_does_not_refuse_aliases_by_default() {
    let (base, seen) = spawn_upstream(RESPONSES_SSE, "").await;
    let state = state_with(nearai_at(&base));
    let head = head_for(&state, responses_request(), &seen).await;

    assert!(
        head.starts_with("POST /v1/responses "),
        "the request went to the wrong endpoint: {head}"
    );
    assert!(
        header_values(&head, NO_ALIASING_HEADER).is_empty(),
        "a caller's inference was made to fail closed without them asking: {head}"
    );
}

/// Default off, on the wire every third-party OpenAI client uses.
#[tokio::test]
async fn a_chat_completions_request_does_not_refuse_aliases_by_default() {
    let (base, seen) = spawn_upstream(CHAT_SSE, "").await;
    let state = state_with(nearai_at(&base));
    let head = head_for(&state, chat_request(), &seen).await;

    assert!(
        head.starts_with("POST /v1/chat/completions "),
        "the request went to the wrong endpoint: {head}"
    );
    assert!(
        header_values(&head, NO_ALIASING_HEADER).is_empty(),
        "a caller's inference was made to fail closed without them asking: {head}"
    );
}

/// Opted in, on Codex's wire.
#[tokio::test]
async fn a_responses_request_refuses_aliases_when_asked_to() {
    let (base, seen) = spawn_upstream(RESPONSES_SSE, "").await;
    let state = state_with(nearai_at(&base).refusing_model_aliases(true));
    let head = head_for(&state, responses_request(), &seen).await;

    assert_eq!(
        header_values(&head, NO_ALIASING_HEADER),
        vec!["true".to_string()],
        "an operator asked for refusal and the Responses lane did not deliver it"
    );
}

/// Opted in, on the Chat Completions wire.
///
/// Both of NEAR AI's wires reach one `send`, and a change that covered only the
/// lane it was tested on would leave whichever agent landed on the other one
/// unprotected. That mistake has been made in this repo before.
#[tokio::test]
async fn a_chat_completions_request_refuses_aliases_when_asked_to() {
    let (base, seen) = spawn_upstream(CHAT_SSE, "").await;
    let state = state_with(nearai_at(&base).refusing_model_aliases(true));
    let head = head_for(&state, chat_request(), &seen).await;

    assert_eq!(
        header_values(&head, NO_ALIASING_HEADER),
        vec!["true".to_string()],
        "both of NEAR AI's wires need the option, not whichever one was tested"
    );
}

/// `x-no-aliasing` is NEAR AI's, and no other backend is told about it.
///
/// Another OpenAI-compatible endpoint is somebody else's server with somebody
/// else's header vocabulary. Sending it there is at best noise and at worst a
/// rejection, and IronWire does not invent a provider's protocol.
#[tokio::test]
async fn an_openai_compatible_backend_is_not_sent_near_ais_header() {
    let (base, seen) = spawn_upstream(CHAT_SSE, "").await;
    let local = ChatCompletionsBackend::local(
        BackendId::from("lmstudio"),
        "LM Studio",
        base.clone(),
        None,
        vec![("qwen3-coder".to_string(), ModelTier::Balanced)],
        30,
    )
    .expect("client builds");
    let state = state_with(local);
    let head = head_for(&state, chat_request(), &seen).await;

    assert!(
        header_values(&head, NO_ALIASING_HEADER).is_empty(),
        "a header was invented for a provider that never asked for it: {head}"
    );
}

/// When an operator has asked for refusal, the guarantee is theirs, not the
/// caller's.
///
/// cloud-api reads an explicit `false` or `0` as "alias away"
/// (`no_aliasing_requested`). Forwarding a client's copy would either send the
/// header twice or let the client switch off the property the operator turned
/// on, so the client's copy is dropped and ours is sent.
#[tokio::test]
async fn a_client_cannot_turn_alias_resolution_back_on() {
    let (base, seen) = spawn_upstream(CHAT_SSE, "").await;
    let state = state_with(nearai_at(&base).refusing_model_aliases(true));
    let mut request = chat_request();
    request
        .headers_mut()
        .insert(NO_ALIASING_HEADER, "false".parse().expect("header value"));
    let head = head_for(&state, request, &seen).await;

    assert_eq!(
        header_values(&head, NO_ALIASING_HEADER),
        vec!["true".to_string()],
        "the client's value reached the gateway, or was sent alongside ours"
    );
}

/// The observation is not conditional on the refusal.
///
/// With refusal on, an alias should never be served -- so a header saying one
/// was is the gateway telling us the guarantee did not hold, which is exactly
/// the row worth having.
#[tokio::test]
async fn a_substitution_is_recorded_even_when_refusal_was_requested() {
    let alias_header = "x-model-alias-resolved: openai/gpt-5 -> Qwen/Qwen3.8-27B\r\n";
    let (base, seen) = spawn_upstream(CHAT_SSE, alias_header).await;
    let state = state_with(nearai_at(&base).refusing_model_aliases(true));
    let head = head_for(&state, chat_request(), &seen).await;
    assert_eq!(header_values(&head, NO_ALIASING_HEADER), vec!["true"]);

    let row = recorded(&state).await;
    assert_eq!(
        row.model_alias_resolved.as_deref(),
        Some(ALIAS_NOTICE),
        "refusal was on, the gateway aliased anyway, and the row did not say so"
    );
}

/// The header name is cloud-api's, not ours.
#[tokio::test]
async fn the_observed_header_is_the_one_the_gateway_sends() {
    assert_eq!(MODEL_ALIAS_RESOLVED_HEADER, "x-model-alias-resolved");
    assert_eq!(NO_ALIASING_HEADER, "x-no-aliasing");
}
