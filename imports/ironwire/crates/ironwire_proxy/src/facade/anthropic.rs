//! The Anthropic façade — what `ANTHROPIC_BASE_URL` points at.
//!
//! Mounted at `/anthropic`; Claude Code appends `/v1/...` itself.
//! `docs/PROTOCOL.md` §1 lists what has to exist here and why —
//! `count_tokens` in particular is not optional, because Claude Code drives its
//! context budget and compaction trigger off it.
//!
//! Every route also exists under `/anthropic/account/{id}`, for a Claude Code
//! bound to one login. The account segment is IronWire's: it says whose request
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

const PROTOCOL: Protocol = Protocol::AnthropicMessages;

/// Routes for the Anthropic façade.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/v1/messages", post(messages))
        .route("/v1/messages/count_tokens", post(count_tokens))
        .route("/v1/models", get(models))
        .route("/account/{account}/v1/messages", post(bound_messages))
        .route(
            "/account/{account}/v1/messages/count_tokens",
            post(bound_count_tokens),
        )
        .route("/account/{account}/v1/models", get(bound_models))
}

async fn messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, FacadeError> {
    forward(state, headers, body, "/v1/messages", None).await
}

/// Claude Code calls this before every turn to decide whether to compact.
/// It goes through the same routing so the count comes from the model that
/// will actually serve the turn.
async fn count_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, FacadeError> {
    forward(state, headers, body, "/v1/messages/count_tokens", None).await
}

async fn bound_messages(
    State(state): State<AppState>,
    account: Result<Path<String>, PathRejection>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, FacadeError> {
    let login = bound(&state, &super::account_segment(account))?;
    forward(state, headers, body, "/v1/messages", Some(login)).await
}

async fn bound_count_tokens(
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
        "/v1/messages/count_tokens",
        Some(login),
    )
    .await
}

async fn bound_models(
    State(state): State<AppState>,
    account: Result<Path<String>, PathRejection>,
) -> Result<Response, FacadeError> {
    let login = bound(&state, &super::account_segment(account))?;
    Ok(list_models(state, Some(login)).await)
}

/// The Claude login a bound URL names, or the refusal for one that names none.
fn bound(state: &AppState, account: &str) -> Result<BackendId, FacadeError> {
    super::bound_login(state, account, SubscriptionProduct::Claude)
        .map_err(|message| FacadeError::unknown_login(message, false))
}

async fn models(State(state): State<AppState>) -> Response {
    list_models(state, None).await
}

/// Synthesized from the backends that are actually eligible, so a picker never
/// offers a model IronWire cannot currently serve.
async fn list_models(state: AppState, client_login: Option<BackendId>) -> Response {
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
                "type": "model",
                "id": model,
                "display_name": model,
            }));
        }
    }
    axum::Json(serde_json::json!({ "data": data, "has_more": false })).into_response()
}

async fn forward(
    state: AppState,
    headers: HeaderMap,
    body: Bytes,
    path: &str,
    client_login: Option<BackendId>,
) -> Result<Response, FacadeError> {
    let started_at = chrono::Utc::now();
    let started = std::time::Instant::now();
    // Parse once, for the peek only. The bytes we forward are the bytes we
    // received unless policy changes the model (`docs/PROTOCOL.md` §2).
    let parsed: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| FacadeError::invalid_request(format!("body is not valid JSON: {e}")))?;
    let markers = state.identity_markers();
    let mut peek = RequestPeek::inspect_with(PROTOCOL, &parsed, body.len(), &markers);
    // The system-block wording is prose and has already moved once; the
    // user-agent has not. Without this, Claude Code 2.1.226 is not recognised
    // as Claude Code and falls off its own subscription
    // (`ironwire_core::peek::user_agent_names_claude_code`).
    if peek.client_identity.is_none()
        && ironwire_core::peek::user_agent_names_claude_code(
            headers
                .get(axum::http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok()),
            &markers.claude_code_user_agent_prefix,
        )
    {
        peek.client_identity = Some(ClientIdentity::ClaudeCode);
    }
    peek.client_login = client_login;
    peek.client_session = client_session_id(&headers, PROTOCOL);
    let key = peek.client_session.as_deref().map_or_else(
        || super::key_for_login(conversation_key(&parsed), peek.client_login.as_ref()),
        |session| {
            ironwire_core::policy::ConversationKey::for_session(
                PROTOCOL,
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
        PROTOCOL,
        path,
        &peek,
        key.clone(),
        body.clone(),
        forwarded.clone(),
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "no route for request");
        let rendered =
            FacadeError::from_pipeline_naming(&e, &|id| super::connect_command(&state, id));
        if let Some(ledger) = state.ledger.as_ref() {
            pipeline::Refusal {
                started_at,
                started,
                facade: "anthropic",
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

    // The observation closure runs when the stream ends *or is dropped*, so
    // both quota accounting and the ledger entry survive a cancelled request.
    let ledger = state.ledger.clone();
    let spend = std::sync::Arc::clone(&state.spend);
    let entry = pipeline::LedgerContext {
        started_at,
        started,
        facade: "anthropic",
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
    let observed = pipeline::observe_boxed(
        response.body,
        dialect_for(PROTOCOL),
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

    // Keep a quiet stream alive, end a dead one honestly, and restart one that
    // failed before producing content — the three shapes of "Response stalled
    // mid-stream" (`docs/PROTOCOL.md` §5, `resilience`).
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

    let body: Body = if peek.stream {
        // A restarted stream carries the same placeholders, so it needs the
        // same map. Without this the reverser is bypassed on exactly the path
        // that runs when something already went wrong.
        let reconnect_map = applied.as_ref().map(|a| std::sync::Arc::clone(&a.map));
        let reconnect_state = state.clone();
        let reconnect_path = path.to_string();
        let reconnect_peek = peek.clone();
        let resilience = resilience::ResilienceConfig::for_turn(
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
            Box::pin(async move {
                match pipeline::dispatch(&state, PROTOCOL, &path, &peek, key, body, headers).await {
                    Ok((response, routed)) => {
                        tracing::info!(backend = %routed.decision.backend, "stream restarted");
                        // The restarted stream's own shape, not the original
                        // request's: a reconnect is a fresh response.
                        let streaming = pipeline::is_event_stream(&response.headers);
                        let restarted = pipeline::observe_boxed(
                            response.body,
                            dialect_for(PROTOCOL),
                            streaming,
                            |_| {},
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
            resilience,
            dialect_for(PROTOCOL),
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
    // Tell every intermediary — and any buffering layer of our own — to leave
    // an event stream alone. A buffered SSE stream is a hung coding agent.
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

/// Derive the conversation key from the parts that stay fixed as a
/// conversation grows (`docs/DESIGN.md` §3).
fn conversation_key(body: &serde_json::Value) -> ConversationKey {
    let system = match body.get("system") {
        Some(serde_json::Value::String(s)) => s.as_str(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .first()
            .and_then(|b| b.get("text"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default(),
        _ => "",
    };
    let tools: Vec<&str> = body
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t.get("name").and_then(serde_json::Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    ConversationKey::derive(PROTOCOL, system, &tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_key_is_stable_as_turns_accumulate() {
        let turn_1 = json!({
            "system": "You are Claude Code",
            "tools": [{"name": "Read"}, {"name": "Bash"}],
            "messages": [{"role": "user", "content": "hi"}],
        });
        let turn_40 = json!({
            "system": "You are Claude Code",
            "tools": [{"name": "Read"}, {"name": "Bash"}],
            "messages": (0..80).map(|i| json!({"role": "user", "content": format!("m{i}")})).collect::<Vec<_>>(),
        });
        assert_eq!(conversation_key(&turn_1), conversation_key(&turn_40));
    }

    #[test]
    fn a_different_session_gets_a_different_key() {
        let a = json!({"system": "You are Claude Code", "tools": [{"name": "Read"}]});
        let b = json!({"system": "You are Aider", "tools": [{"name": "Read"}]});
        assert_ne!(conversation_key(&a), conversation_key(&b));
    }

    #[test]
    fn a_body_with_no_system_or_tools_still_keys() {
        let _ = conversation_key(&json!({"messages": []}));
    }
}
