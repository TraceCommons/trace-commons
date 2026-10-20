//! The OpenAI façade — what a Codex custom provider points at.
//!
//! Mounted at `/openai`; Codex appends `/v1/responses` itself, and third-party
//! clients append `/v1/chat/completions`.
//!
//! Two native lanes live here, not one. Responses and Chat Completions are the
//! same *family* but different wires, so each gets a backend that speaks it
//! natively and neither is translated into the other.
//!
//! Every route also exists under `/openai/account/{id}`, for a Codex bound to
//! one ChatGPT login. The account segment is IronWire's: it says whose request
//! this is, and is never forwarded.

use super::client_session_id;
use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::rejection::PathRejection;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use ironwire_core::peek::RequestPeek;
use ironwire_core::policy::ConversationKey;
use ironwire_core::protocol::{BackendId, ClientIdentity, Protocol};
use ironwire_creds::SubscriptionProduct;
use ironwire_upstream::headers::{forward_request_header, forward_response_header};

use crate::facade::error::FacadeError;
use crate::pipeline::{self, dialect_for};
use crate::resilience;
use crate::state::AppState;

/// Routes for the OpenAI façade.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/responses", post(responses))
        .route("/v1/chat/completions", post(chat_completions))
        .route("/v1/models", get(models))
        .route("/account/{account}/v1/responses", post(bound_responses))
        .route(
            "/account/{account}/v1/chat/completions",
            post(bound_chat_completions),
        )
        .route("/account/{account}/v1/models", get(bound_models))
}

async fn responses(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, FacadeError> {
    forward(
        state,
        headers,
        body,
        "/v1/responses",
        Protocol::OpenAiResponses,
        None,
    )
    .await
}

async fn chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, FacadeError> {
    forward(
        state,
        headers,
        body,
        "/v1/chat/completions",
        Protocol::OpenAiChat,
        None,
    )
    .await
}

async fn bound_responses(
    State(state): State<AppState>,
    account: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, FacadeError> {
    let login = bound(&state, &super::account_segment(account))?;
    forward(
        state,
        headers,
        body,
        "/v1/responses",
        Protocol::OpenAiResponses,
        Some(login),
    )
    .await
}

async fn bound_chat_completions(
    State(state): State<AppState>,
    account: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, FacadeError> {
    let login = bound(&state, &super::account_segment(account))?;
    forward(
        state,
        headers,
        body,
        "/v1/chat/completions",
        Protocol::OpenAiChat,
        Some(login),
    )
    .await
}

async fn bound_models(
    State(state): State<AppState>,
    account: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
) -> Result<Response, FacadeError> {
    let login = bound(&state, &super::account_segment(account))?;
    Ok(list_models(state, headers, Some(login)).await)
}

/// The ChatGPT login a bound URL names, or the refusal for one that names none.
fn bound(state: &AppState, account: &str) -> Result<BackendId, FacadeError> {
    super::bound_login(state, account, SubscriptionProduct::ChatGpt)
        .map_err(|message| FacadeError::unknown_login(message, true))
}

async fn models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    list_models(state, headers, None).await
}

/// `/v1/models`, in whichever of its two shapes the caller is actually asking
/// for.
///
/// Codex does not ask the public OpenAI endpoint. It asks
/// `chatgpt.com/backend-api/codex/models`, which answers with `{"models":[…]}`
/// — a client-configuration document carrying each model's context window,
/// truncation policy, reasoning levels and instructions template. So this is
/// not "OpenAI disagreeing with OpenAI": it is a different product surface, and
/// a list synthesized in the public shape is both unparseable to Codex and
/// missing everything it came for.
///
/// For Codex we therefore forward the provider's own document, by the same rule
/// as the native lane. For every other OpenAI-compatible client we synthesize
/// the public shape from the backends that are actually eligible.
async fn list_models(
    state: AppState,
    headers: HeaderMap,
    client_login: Option<BackendId>,
) -> Response {
    let markers = state.identity_markers();
    if ironwire_core::peek::originator_names_codex(
        headers.get("originator").and_then(|v| v.to_str().ok()),
        &markers.codex_originator_prefix,
    ) && let Some(document) = codex_models_document(&state, client_login.as_ref()).await
    {
        return (
            [(axum::http::header::CONTENT_TYPE, "application/json")],
            document,
        )
            .into_response();
    }
    openai_shaped_models(state, client_login).await
}

/// The models document from a consented ChatGPT subscription, if there is one.
///
/// The client's own login's: two ChatGPT plans can carry different models, and
/// a Codex shown another account's catalogue would pick models its own
/// requests are then refused. Another login's only with cross-account fallback
/// on, and after the client's own — the order the router would serve it in.
async fn codex_models_document(
    state: &AppState,
    client_login: Option<&BackendId>,
) -> Option<Vec<u8>> {
    let consent = state.consent_snapshot();
    let fallback = state.cross_account_fallback();
    let mut offered: Vec<_> = state
        .backends
        .all()
        .iter()
        .filter(|backend| crate::state::consents_to(&consent, backend.as_ref()))
        .filter(|backend| super::offered_to(backend.as_ref(), client_login, fallback))
        .collect();
    offered.sort_by_key(|backend| match backend.login() {
        Some(_) => u8::from(!super::offered_to(backend.as_ref(), client_login, false)),
        None => 2,
    });
    for backend in offered {
        if let Some(document) = backend.models_document().await {
            return Some(document);
        }
    }
    None
}

async fn openai_shaped_models(state: AppState, client_login: Option<BackendId>) -> Response {
    let statuses = state.backends.statuses().await;
    let consent = state.consent_snapshot();
    let fallback = state.cross_account_fallback();
    let mut data = Vec::new();
    for status in &statuses {
        if !status.authenticated {
            continue;
        }
        if !state.backends.consented(&status.id, &consent) {
            continue;
        }
        if !state.backends.get(&status.id).is_none_or(|backend| {
            super::offered_to(backend.as_ref(), client_login.as_ref(), fallback)
        }) {
            continue;
        }
        for (model, _) in &status.models {
            if data.iter().any(|m: &serde_json::Value| m["id"] == *model) {
                continue;
            }
            data.push(serde_json::json!({
                "id": model,
                "object": "model",
                "owned_by": "ironwire",
            }));
        }
    }
    axum::Json(serde_json::json!({ "object": "list", "data": data })).into_response()
}

async fn forward(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
    path: &str,
    protocol: Protocol,
    client_login: Option<BackendId>,
) -> Result<Response, FacadeError> {
    let started_at = chrono::Utc::now();
    let started = std::time::Instant::now();
    // Parsed once, for the peek only. The bytes we forward are the bytes we
    // received unless policy changes the model (`docs/PROTOCOL.md` §2).
    let parsed: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| FacadeError::invalid_request(format!("body is not valid JSON: {e}")))?;
    let markers = state.identity_markers();
    let mut peek = RequestPeek::inspect_with(protocol, &parsed, body.len(), &markers);
    // The body-side marker is only half the signal, and it is the half that
    // Codex stopped sending: since 0.145 there is no `instructions` field, so
    // the `originator` header is what identifies the client. Without this the
    // ChatGPT subscription is unreachable from Codex itself
    // (`ironwire_core::peek::originator_names_codex`).
    if peek.client_identity.is_none()
        && ironwire_core::peek::originator_names_codex(
            headers.get("originator").and_then(|v| v.to_str().ok()),
            &markers.codex_originator_prefix,
        )
    {
        peek.client_identity = Some(ClientIdentity::Codex);
    }
    peek.client_login = client_login;
    // Which ChatGPT account minted a session's encrypted reasoning is recorded
    // per session: every Codex session of one version shares a conversation
    // key, and one of them moving to another login moves no one else's.
    peek.client_session = client_session_id(&headers, protocol);
    let key = peek.client_session.as_deref().map_or_else(
        || {
            super::key_for_login(
                conversation_key(protocol, &parsed),
                peek.client_login.as_ref(),
            )
        },
        |session| {
            ironwire_core::policy::ConversationKey::for_session(
                protocol,
                session,
                peek.client_login.as_ref(),
            )
        },
    );
    let peek = peek;
    let conversation = key.0;

    // The privacy filter is the one documented exception to byte-identical
    // forwarding (`docs/PROTOCOL.md` §2), and it is opt-in. With it off,
    // `body` below is still exactly the bytes the client sent.
    let applied = state.privacy().map(|filter| filter.apply(&key, &parsed));
    let body = match &applied {
        Some(applied) => {
            bytes::Bytes::from(serde_json::to_vec(&applied.body).unwrap_or_else(|_| body.to_vec()))
        }
        None => body,
    };

    let forwarded: Vec<(String, String)> = headers
        .iter()
        .filter(|(name, _)| {
            forward_request_header(name.as_str())
                || name
                    .as_str()
                    .eq_ignore_ascii_case(ironwire_upstream::headers::NEUTRAL_SESSION_HEADER)
        })
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_ascii_lowercase(), v.to_string()))
        })
        .collect();

    let (response, routed) = pipeline::dispatch(
        &state,
        protocol,
        path,
        &peek,
        key.clone(),
        body.clone(),
        forwarded.clone(),
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "no route for request");
        // Codex renders a line from us on a limit response, and this is the
        // moment it is worth using: the user has just been stopped and has no
        // way to tell whether IronWire had somewhere else to go.
        let rendered =
            FacadeError::from_pipeline_naming(&e, &|id| super::connect_command(&state, id))
                .on_openai_facade(peek.client_identity == Some(ClientIdentity::Codex));
        if let Some(ledger) = state.ledger.as_ref() {
            pipeline::Refusal {
                started_at,
                started,
                facade: "openai",
                path: path.to_string(),
                conversation: conversation.to_string(),
                client_session_id: peek.client_session.clone(),
                requested_model: peek.requested_model.clone(),
                status: rendered.status().as_u16(),
            }
            .write(ledger, &e);
        }
        rendered
    })?;

    tracing::info!(
        backend = %routed.decision.backend,
        model = routed.decision.model.as_deref().unwrap_or("<client's>"),
        rung = ?routed.decision.rung,
        attempts = routed.attempts,
        reason = %routed.decision.reason,
        "routed"
    );

    let backend = state
        .backends
        .get(&routed.decision.backend)
        .cloned()
        .expect("dispatch returned a registered backend");
    // Read before the `Arc` moves into the observation closure below. Local
    // capacity gets a longer stall timeout and no price.
    let backend_is_local = backend.kind() == ironwire_core::protocol::BackendKind::Local;
    let backend_is_metered = backend.kind().is_metered();
    let backend_offers_receipts = backend.offers_receipts();

    let ledger = state.ledger.clone();
    let spend = std::sync::Arc::clone(&state.spend);
    let entry = pipeline::LedgerContext {
        started_at,
        started,
        facade: "openai",
        path: path.to_string(),
        conversation: conversation.to_string(),
        client_session_id: peek.client_session.clone(),
        backend: routed.decision.backend.to_string(),
        backend_is_local,
        backend_is_metered,
        backend_offers_receipts,
        requested_model: peek.requested_model.clone(),
        rung: format!("{:?}", routed.decision.rung).to_lowercase(),
        attempts: routed.attempts,
        substitutions: applied
            .as_ref()
            .map(|a| i64::try_from(a.substitutions).unwrap_or(i64::MAX)),
        status: response.status.as_u16(),
        confidence: routed.confidence.clone(),
        capture: routed.capture.clone(),
        bodies: state.bodies.clone(),
    };
    let streaming = pipeline::is_event_stream(&response.headers);
    // The gateway announces a model substitution in a response *header*, and
    // the observation the ledger gets is folded out of the response *body*, so
    // the fact has to be carried across. Read here rather than in the backend:
    // the backend's own observation feeds quota state and never reaches a row.
    let model_alias_resolved = ironwire_upstream::observe::model_alias_resolved(&response.headers);
    let reconnect_entry = entry.clone();
    let observed = pipeline::observe_boxed(
        response.body,
        dialect_for(protocol),
        streaming,
        move |obs| {
            let mut obs = obs;
            obs.model_alias_resolved = model_alias_resolved.clone();
            pipeline::record(&backend, &obs);
            if let Some(ledger) = ledger.as_ref() {
                entry.write(ledger, &spend, &obs);
            }
        },
    );

    // Put the real values back before anything reaches the client. This runs
    // inside the resilience guard's input, so a reconnect gets its own
    // reverser and cannot inherit half a placeholder.
    let observed: std::pin::Pin<
        Box<
            dyn futures_util::Stream<
                    Item = Result<bytes::Bytes, ironwire_upstream::backend::UpstreamError>,
                > + Send,
        >,
    > = match applied.as_ref().map(|a| std::sync::Arc::clone(&a.map)) {
        Some(map) => Box::pin(crate::privacy::reverse_stream(observed, map)),
        None => Box::pin(observed),
    };

    let body: Body = if streaming {
        // A restarted stream carries the same placeholders, so it needs the
        // same map. Without this the reverser is bypassed on exactly the path
        // that runs when something already went wrong.
        let reconnect_map = applied.as_ref().map(|a| std::sync::Arc::clone(&a.map));
        let reconnect_state = state.clone();
        let reconnect_path = path.to_string();
        let reconnect_peek = peek.clone();
        let settings = resilience::ResilienceConfig::for_turn(
            &state.config.resilience,
            peek.likely_compaction,
            backend_is_local,
        );
        let reconnect: resilience::Reconnect = Box::new(move || {
            let state = reconnect_state.clone();
            let path = reconnect_path.clone();
            let peek = reconnect_peek.clone();
            let key = key.clone();
            let body = body.clone();
            let headers = forwarded.clone();
            let map = reconnect_map.clone();
            let mut entry = reconnect_entry.clone();
            Box::pin(async move {
                entry.started_at = chrono::Utc::now();
                entry.started = std::time::Instant::now();
                match pipeline::dispatch(&state, protocol, &path, &peek, key, body, headers).await {
                    Ok((response, routed)) => {
                        tracing::info!(backend = %routed.decision.backend, "stream restarted");
                        // The restarted stream's own shape, not the original
                        // request's: a reconnect is a fresh response.
                        let streaming = pipeline::is_event_stream(&response.headers);
                        let backend = state.backends.get(&routed.decision.backend)?.clone();
                        entry.backend = routed.decision.backend.to_string();
                        entry.backend_is_local =
                            backend.kind() == ironwire_core::protocol::BackendKind::Local;
                        entry.backend_is_metered = backend.kind().is_metered();
                        entry.rung = format!("{:?}", routed.decision.rung).to_lowercase();
                        entry.attempts = routed.attempts;
                        entry.status = response.status.as_u16();
                        entry.confidence = routed.confidence;
                        entry.capture = routed.capture;
                        let alias =
                            ironwire_upstream::observe::model_alias_resolved(&response.headers);
                        let restarted = pipeline::observe_boxed(
                            response.body,
                            dialect_for(protocol),
                            streaming,
                            move |mut obs| {
                                obs.model_alias_resolved = alias;
                                pipeline::record(&backend, &obs);
                                if let Some(ledger) = state.ledger.as_ref() {
                                    entry.write(ledger, &state.spend, &obs);
                                }
                            },
                        );
                        Some(match map {
                            Some(map) => Box::pin(crate::privacy::reverse_stream(restarted, map))
                                as futures_util::stream::BoxStream<
                                    'static,
                                    Result<bytes::Bytes, ironwire_upstream::backend::UpstreamError>,
                                >,
                            None => restarted,
                        })
                    }
                    Err(error) => {
                        tracing::warn!(%error, "no capacity left to restart the stream");
                        None
                    }
                }
            })
        });
        Body::from_stream(resilience::guard(
            observed,
            reconnect,
            settings,
            dialect_for(protocol),
        ))
    } else {
        Body::from_stream(observed)
    };

    let mut builder = Response::builder()
        .status(StatusCode::from_u16(response.status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY));
    for (name, value) in &response.headers {
        if forward_response_header(name) {
            builder = builder.header(name, value);
        }
    }
    if peek.stream {
        builder = builder
            .header("cache-control", "no-cache")
            .header("x-accel-buffering", "no");
    }
    builder = builder.header("x-ironwire-backend", routed.decision.backend.to_string());

    builder
        .body(body)
        .map_err(|e| FacadeError::invalid_request(format!("could not build response: {e}")))
}

/// Derive the conversation key from the parts that stay fixed as a conversation
/// grows (`docs/DESIGN.md` §3).
///
/// Codex carries its persona in `instructions`; Chat Completions carries it in
/// the first `system` message. Both are stable across a session, which is what
/// affinity needs.
fn conversation_key(protocol: Protocol, body: &serde_json::Value) -> ConversationKey {
    let preamble = body
        .get("instructions")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            body.get("messages")
                .and_then(serde_json::Value::as_array)
                .and_then(|messages| messages.first())
                .filter(|first| {
                    first.get("role").and_then(serde_json::Value::as_str) == Some("system")
                })
                .and_then(|first| first.get("content"))
                .and_then(serde_json::Value::as_str)
        })
        .unwrap_or_default();

    let tools: Vec<&str> = body
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| {
                    t.get("name")
                        .or_else(|| t.pointer("/function/name"))
                        .and_then(serde_json::Value::as_str)
                })
                .collect()
        })
        .unwrap_or_default();

    ConversationKey::derive(protocol, preamble, &tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_codex_session_keeps_one_key_as_it_grows() {
        let turn_1 = json!({
            "instructions": "You are Codex, a coding agent.",
            "tools": [{"type": "function", "name": "shell"}],
            "input": [{"role": "user", "content": "hi"}],
        });
        let turn_40 = json!({
            "instructions": "You are Codex, a coding agent.",
            "tools": [{"type": "function", "name": "shell"}],
            "input": (0..80).map(|i| json!({"role": "user", "content": format!("m{i}")})).collect::<Vec<_>>(),
        });
        assert_eq!(
            conversation_key(Protocol::OpenAiResponses, &turn_1),
            conversation_key(Protocol::OpenAiResponses, &turn_40)
        );
    }

    #[test]
    fn chat_completions_keys_off_its_system_message() {
        let a = json!({"messages": [{"role": "system", "content": "You are Aider."}]});
        let b = json!({"messages": [{"role": "system", "content": "You are something else."}]});
        assert_ne!(
            conversation_key(Protocol::OpenAiChat, &a),
            conversation_key(Protocol::OpenAiChat, &b)
        );
    }

    #[test]
    fn the_two_openai_wires_share_one_key() {
        // Affinity is keyed on the *family*, not the wire (`ConversationKey::
        // derive`), and that is the behaviour we want here: what affinity
        // protects is the provider-side prompt cache, which is shared across
        // both wires. A client that sends the same preamble and tools to both is
        // one conversation, and splitting it would throw the cache away.
        let body = json!({"instructions": "same", "tools": []});
        assert_eq!(
            conversation_key(Protocol::OpenAiResponses, &body),
            conversation_key(Protocol::OpenAiChat, &body)
        );
        // The Anthropic family is a different pool and must not collide.
        assert_ne!(
            conversation_key(Protocol::OpenAiResponses, &body),
            ConversationKey::derive(Protocol::AnthropicMessages, "same", &[])
        );
    }

    #[test]
    fn tool_names_are_read_from_both_shapes() {
        // Responses puts `name` at the top level; Chat Completions nests it
        // under `function`. Missing one would split a conversation in half.
        let responses =
            json!({"instructions": "x", "tools": [{"type": "function", "name": "shell"}]});
        let chat = json!({"instructions": "x", "tools": [{"type": "function", "function": {"name": "shell"}}]});
        assert_eq!(
            conversation_key(Protocol::OpenAiResponses, &responses),
            conversation_key(Protocol::OpenAiResponses, &chat)
        );
    }

    #[test]
    fn a_body_with_nothing_recognisable_still_keys() {
        let _ = conversation_key(Protocol::OpenAiResponses, &json!({}));
    }
}
