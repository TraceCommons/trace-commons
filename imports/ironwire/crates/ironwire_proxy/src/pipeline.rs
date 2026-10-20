//! The request pipeline: peek, decide, send, fail over, stream.
//!
//! Two rules govern this file and both come from `docs/PROTOCOL.md`:
//!
//! 1. **Mutate only what §2 enumerates.** URL, auth headers, hop-by-hop
//!    headers, and — only when policy chose a different model — the `model`
//!    key, plus separately consented admission metadata. Nothing else is touched,
//!    which is why provider features we have
//!    never heard of keep working.
//! 2. **Failover ends at the first byte.** Once a byte of the response has
//!    reached the client, replaying the request would duplicate content the
//!    client has already committed to its transcript. After that point an
//!    upstream failure is surfaced on the open stream, never retried (§5).

use std::collections::HashSet;
use std::sync::Arc;

use bytes::Bytes;
use chrono::Utc;
use futures_util::{Stream, StreamExt};
use ironwire_core::confidence::ConfidenceAggregates;
use ironwire_core::peek::RequestPeek;
use ironwire_core::policy::{ConversationKey, NoRoute, RouteDecision};
use ironwire_core::protocol::Protocol;
use ironwire_ledger::{Exchange, Ledger};
use ironwire_upstream::backend::{Backend, UpstreamError, UpstreamRequest, UpstreamResponse};
use ironwire_upstream::observe::Observation;
use ironwire_upstream::sse::{Dialect, SseObserver};

use crate::state::AppState;

/// The upshot of routing one request.
#[derive(Debug)]
pub struct Routed {
    /// What the router decided.
    pub decision: RouteDecision,
    /// How many backends were tried before this one succeeded.
    pub attempts: usize,
    /// Errors from backends tried and rejected, for the log and for the error
    /// we return if everything fails.
    pub rejected: Vec<(String, String)>,
    /// The bodies this exchange put on the wire, when `capture.bodies` is on.
    ///
    /// `None` when capture is off -- which is the default, because bodies are
    /// the user's source code (`docs/TRUST.md` §4).
    pub capture: Option<Capture>,
    /// Where the translated response will leave its confidence aggregate.
    ///
    /// Filled asynchronously, when the response finishes, which is after this
    /// struct is handed back — so the façade carries the handle into the ledger
    /// entry rather than a value. Stays empty on the native lane and whenever
    /// `capture.logprobs` is off, which is nearly always.
    pub confidence: ConfidenceSink,
}

/// A slot one translated response writes its confidence aggregate into.
///
/// The aggregate is only known when the last frame has been translated, and the
/// ledger entry is only written when the response body ends or is dropped —
/// which is strictly later. A shared slot is what connects the two without
/// making the stream carry the ledger or the ledger poll the stream.
#[derive(Clone, Debug, Default)]
pub struct ConfidenceSink(Arc<std::sync::Mutex<Option<ConfidenceAggregates>>>);

impl ConfidenceSink {
    /// Record the aggregate for the response that just finished.
    ///
    /// Reduces from log-probabilities and stores nothing when there were none,
    /// so an absent aggregate never becomes a measured zero.
    fn record(&self, logprobs: &[f64]) {
        let Some(aggregates) = ironwire_core::confidence::reduce_token_logprobs(logprobs) else {
            return;
        };
        match self.0.lock() {
            Ok(mut slot) => *slot = Some(aggregates),
            Err(poisoned) => *poisoned.into_inner() = Some(aggregates),
        }
    }

    /// What was recorded, if anything.
    #[must_use]
    pub fn get(&self) -> Option<ConfidenceAggregates> {
        match self.0.lock() {
            Ok(slot) => *slot,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }
}

/// Failure of the whole pipeline.
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    /// Explicit metadata consent cannot be honored on this request.
    #[error("{0}; renew or revoke this session's admission binding in onboarding")]
    Admission(ironwire_core::admission::AdmissionError),
    /// The router found nowhere to send this.
    #[error("no route: {0}")]
    NoRoute(NoRoute),
    /// Every candidate backend failed.
    #[error("all backends failed")]
    AllFailed {
        /// The last error, which is the most informative one to surface.
        last: UpstreamError,
        /// Everything tried, for diagnostics.
        rejected: Vec<(String, String)>,
    },
    /// The upstream returned a non-retryable error we pass through.
    #[error(transparent)]
    Upstream(UpstreamError),
}

/// Build the candidate list with any breached spend cap applied.
///
/// A cap is expressed as `Headroom::CapReached` rather than as a filter of its
/// own, so every existing consumer — `usable`, the sort key, the status
/// renderer, the balance view — behaves correctly without a second exclusion
/// mechanism that would drift from the first.
fn capped_candidates(
    state: &AppState,
    statuses: &[ironwire_upstream::backend::BackendStatus],
    consent: &ironwire_creds::ConsentLedger,
    now: chrono::DateTime<Utc>,
) -> (
    Vec<ironwire_core::policy::Candidate>,
    Option<(String, f64, f64)>,
) {
    let limits = &state.config.limits;
    if !limits.any_cap() {
        return (state.backends.candidates(statuses, consent), None);
    }
    let mut tracker = match state.spend.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut breached: Vec<(String, f64, f64)> = Vec::new();
    let mut any_capped: Option<(String, f64, f64)> = None;
    let candidates = state
        .backends
        .candidates_capped(statuses, consent, &mut |backend, quota| {
            // Only metered money. A subscription is already paid for and its
            // recorded cost is "what this would have cost on the meter", not
            // money anyone was billed; capping on it would cap capacity the
            // user bought. Prepaid credits are excluded for the same reason.
            if !backend.kind().is_metered() {
                return quota;
            }
            let Some(cap) = limits.cap_for(backend.id().as_str()) else {
                return quota;
            };
            // The global cap is measured against the metered total, the
            // per-backend one against that backend — whichever binds first.
            let spent = tracker.spent(backend.id(), now).max(
                if limits.daily_spend_usd.is_some_and(|c| c > 0.0) {
                    tracker.total(now)
                } else {
                    0.0
                },
            );
            if spent < cap {
                return quota;
            }
            any_capped.get_or_insert_with(|| (backend.id().to_string(), spent, cap));
            if tracker.announce_once(backend.id(), now) {
                breached.push((backend.id().to_string(), spent, cap));
            }
            let mut capped = quota;
            capped.primary = ironwire_core::quota::Headroom::CapReached {
                spent_usd: spent,
                cap_usd: cap,
                resets_at: crate::spend::window_start(now) + chrono::Duration::days(1),
            };
            capped
        });
    drop(tracker);
    // Published after the lock is released: this is the moment the user most
    // needs to know, and it happens while nobody is watching `status`.
    for (backend, spent_usd, cap_usd) in breached {
        tracing::warn!(%backend, spent_usd, cap_usd, "spend cap reached; not routing here");
        state.events.publish(crate::events::Event::CapReached {
            at: now,
            backend,
            spent_usd,
            cap_usd,
        });
    }
    let refused = (limits.on_breach == ironwire_core::config::BreachAction::Refuse)
        .then_some(any_capped)
        .flatten();
    (candidates, refused)
}

/// A per-request route override from the `X-IronWire-Route` header.
///
/// Distinct from `ironwire pin`, which is a daemon-wide mode a user turns on
/// and forgets. This is one request, named by the caller — the escape hatch for
/// a script that wants a specific backend without disturbing anyone else's
/// session (`docs/DESIGN.md` §3).
///
/// Like a pin, it overrides *preference* but never *eligibility*: a route that
/// would corrupt the request is still refused. Obeying a caller into producing
/// a broken response is not obedience, it is a bug.
pub const ROUTE_OVERRIDE_HEADER: &str = "x-ironwire-route";

/// Read and remove the route override from the forwarded header list.
///
/// Removed, not just read: it is IronWire's own header and forwarding it to a
/// provider would leak our routing vocabulary into someone else's API.
#[must_use]
pub fn take_route_override(
    headers: &mut Vec<(String, String)>,
) -> Option<(String, Option<String>)> {
    let index = headers
        .iter()
        .position(|(name, _)| name.eq_ignore_ascii_case(ROUTE_OVERRIDE_HEADER))?;
    let (_, value) = headers.remove(index);
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    // `backend` or `backend:model`.
    Some(match value.split_once(':') {
        Some((backend, model)) => (backend.trim().to_string(), Some(model.trim().to_string())),
        None => (value.to_string(), None),
    })
}

/// One exchange that never reached a backend, for the ledger.
///
/// A refusal is an exchange. The ledger used to hold rows whose `error` column
/// was always `NULL` — nothing anywhere ever set it — so a user looking at a
/// failed turn saw a status and no reason, which is the state that made the
/// Codex-on-NEAR-AI failure take a live bisect to understand. Nothing here is
/// hashed or elided: the detail is what the row is for.
pub struct Refusal {
    /// When the request arrived.
    pub started_at: chrono::DateTime<Utc>,
    /// When work on it started, for the elapsed time.
    pub started: std::time::Instant,
    /// Which façade took it.
    pub facade: &'static str,
    /// Path as the client asked for it.
    pub path: String,
    /// Conversation key.
    pub conversation: String,
    /// The agent's own session id, where it sent one.
    pub client_session_id: Option<String>,
    /// Model the client asked for.
    pub requested_model: Option<String>,
    /// Status IronWire is about to return.
    pub status: u16,
}

impl Refusal {
    /// Write the row. Never propagates: a ledger problem must not change what
    /// the client is told.
    pub fn write(self, ledger: &Ledger, error: &PipelineError) {
        let backend = match error {
            PipelineError::Upstream(upstream) | PipelineError::AllFailed { last: upstream, .. } => {
                upstream.backend_id().map(ToString::to_string)
            }
            _ => None,
        };
        let exchange = Exchange {
            id: None,
            started_at: self.started_at,
            ttfb_ms: None,
            total_ms: i64::try_from(self.started.elapsed().as_millis()).ok(),
            facade: self.facade.to_string(),
            path: self.path,
            conversation: self.conversation,
            client_session_id: self.client_session_id,
            backend: backend.unwrap_or_else(|| "none".to_string()),
            requested_model: self.requested_model,
            served_model: None,
            upstream_id: None,
            // No response reached us, so there were no headers to read it from.
            model_alias_resolved: None,
            request_sha256: None,
            response_sha256: None,
            body_ref: None,
            rung: "none".to_string(),
            attempts: 0,
            input_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: None,
            cost_usd: None,
            substitutions: None,
            status: i64::from(self.status),
            error: Some(error.to_string()),
            confidence: None,
            // No backend answered, so there is no answer to prove -- and no
            // backend kind in hand to say which side of the routed/outside line
            // it would have fallen on. Absent, not `outside`: the rollup keeps
            // these with the rows it cannot place rather than guessing.
            proof: None,
        };
        if let Err(error) = ledger.record(&exchange) {
            tracing::debug!(%error, "could not write the refusal to the trace ledger");
        }
    }
}

/// Route and dispatch one request, failing over while it is still safe to.
///
/// # Errors
///
/// [`PipelineError`] when no backend could serve the request.
pub async fn dispatch(
    state: &AppState,
    inbound: Protocol,
    path: &str,
    peek: &RequestPeek,
    key: ConversationKey,
    body: Bytes,
    headers: Vec<(String, String)>,
) -> Result<(UpstreamResponse, Routed), PipelineError> {
    let conversation = key.0.to_string();
    let result = dispatch_inner(state, inbound, path, peek, key, body, headers).await;
    // Every failure exit publishes, not just the one at the bottom: `dispatch`
    // returns early from inside the failover loop too, and a channel that
    // reports only some failures is worse than one that reports none — a user
    // would learn to read silence as success.
    let result = result.map_err(|error| match error {
        // The router knows a trusted backend could not serve this; only the
        // registry knows which trusted ids do not exist at all. A typo there
        // refuses everything, so it has to reach the message.
        PipelineError::NoRoute(NoRoute::NoTrustedBackendAvailable { tried, .. }) => {
            PipelineError::NoRoute(NoRoute::NoTrustedBackendAvailable {
                tried,
                missing: state.backends.missing_trusted(),
            })
        }
        other => other,
    });
    if let Err(error) = &result {
        state.events.publish(crate::events::Event::Failed {
            at: Utc::now(),
            conversation,
            detail: error.to_string(),
        });
    }
    result
}

/// The neutral header stays inside the pipeline and is stripped before sending.
fn admission_session(headers: &[(String, String)], protocol: Protocol) -> Option<String> {
    let neutral = ironwire_upstream::headers::NEUTRAL_SESSION_HEADER;
    let native = ironwire_upstream::headers::client_session_header(protocol);
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(neutral))
        .or_else(|| {
            headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(native))
        })
        .map(|(_, value)| value)
        .filter(|value| ironwire_core::admission::valid_session(value))
        .cloned()
}

async fn dispatch_inner(
    state: &AppState,
    inbound: Protocol,
    path: &str,
    peek: &RequestPeek,
    key: ConversationKey,
    body: Bytes,
    headers: Vec<(String, String)>,
) -> Result<(UpstreamResponse, Routed), PipelineError> {
    let session = admission_session(&headers, inbound);
    // Sessionless traffic does not touch this opt-in registry. A poisoned plain
    // map remains usable; one panicking control request cannot brick the proxy.
    let binding = match session.as_deref() {
        None => None,
        Some(session) => state
            .admission_bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot(session, Utc::now().timestamp())
            .map_err(PipelineError::Admission)?,
    };
    let mut peek = peek.clone();
    peek.requirements.admission_backend = binding.as_ref().map(|b| b.backend.clone());
    let peek = &peek;
    let mut headers = headers;
    headers.retain(|(name, _)| {
        !name.eq_ignore_ascii_case(ironwire_upstream::headers::NEUTRAL_SESSION_HEADER)
    });
    let route_override = take_route_override(&mut headers).map(|(backend, model)| {
        (
            ironwire_core::protocol::BackendId::from(backend.as_str()),
            model,
        )
    });

    let cross_account_fallback = state
        .policy
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .cross_account_fallback();
    let statuses = state
        .backends
        .routing_statuses(peek, cross_account_fallback)
        .await;
    let consent = state.consent_snapshot();
    let (candidates, refused) = capped_candidates(state, &statuses, &consent, Utc::now());
    // `on_breach = "refuse"` is a hard stop, so it is answered before routing
    // rather than after the ladder has quietly found somewhere else to go.
    // `descend` — the default — never reaches here: the capped backend is
    // simply skipped, which is the whole point of expressing a cap as
    // `Headroom`.
    if let Some((backend, spent_usd, cap_usd)) = refused {
        return Err(PipelineError::NoRoute(NoRoute::SpendCapReached {
            backend,
            spent_usd,
            cap_usd,
        }));
    }

    // Skip backends whose circuit is open, so a known-dead one is not
    // rediscovered — at the cost of a round trip — on every single turn.
    //
    // Unless that would leave nothing. A breaker exists to spend less time on a
    // backend that is failing, not to turn a degraded proxy into a dead one:
    // when every circuit is open, the honest move is to try anyway and report
    // the real upstream error rather than a 503 of our own invention.
    //
    // Skipped by marking it unhealthy, as the loop below marks one that just
    // failed, and for the same reasons: the rung can see a preferred backend
    // was passed over, and a refusal can see the client's own login is there.
    // Dropped, it read as missing, and a client on its own login's URL was
    // told to rebind itself for as long as that circuit stayed open.
    let now = Utc::now();
    let open: HashSet<_> = candidates
        .iter()
        .filter(|c| !state.breakers.allows(&c.id, now))
        .map(|c| c.id.clone())
        .collect();
    let candidates = if open.len() == candidates.len() {
        if !candidates.is_empty() {
            tracing::warn!("every backend's circuit is open; trying anyway");
        }
        candidates
    } else {
        candidates
            .into_iter()
            .map(|mut candidate| {
                if open.contains(&candidate.id) {
                    candidate.healthy = false;
                }
                candidate
            })
            .collect()
    };

    // Where this conversation was before we decided anything, so a genuine
    // route change can be told apart from a request that stayed put.
    let previous = match state.policy.lock() {
        Ok(policy) => policy.current_backend(&key),
        Err(poisoned) => poisoned.into_inner().current_backend(&key),
    };

    let confidence = ConfidenceSink::default();
    let mut rejected: Vec<(String, String)> = Vec::new();
    let mut attempts = 0usize;
    let mut last_error: Option<UpstreamError> = None;

    // Bounded: each iteration marks the failed backend unavailable, so the
    // candidate set strictly shrinks. The cap is belt and braces against a
    // backend that reports itself healthy after failing.
    let max_attempts = candidates
        .len()
        .max(1)
        .saturating_mul(1 + MAX_SAME_BACKEND_RETRIES);
    let mut excluded: HashSet<ironwire_core::protocol::BackendId> = HashSet::new();
    let mut same_backend_attempts = 0usize;

    while attempts < max_attempts {
        // Marked unhealthy rather than removed. `usable()` refuses an unhealthy
        // candidate exactly as it refused an absent one, so routing is
        // unchanged — but the rung calculation can still see that a preferred
        // backend was passed over. Dropping them from the list made a live
        // failover record `Preferred` while the same fall, predicted from
        // quota, recorded rung 2: the same event, two different answers,
        // depending only on whether we knew in advance.
        let available: Vec<_> = candidates
            .iter()
            .cloned()
            .map(|mut candidate| {
                if excluded.contains(&candidate.id) {
                    candidate.healthy = false;
                }
                candidate
            })
            .collect();

        let decision = {
            let mut policy = match state.policy.lock() {
                Ok(p) => p,
                Err(poisoned) => poisoned.into_inner(),
            };
            match policy.decide_with_override(
                key.clone(),
                inbound,
                peek,
                &available,
                Utc::now(),
                route_override.clone(),
            ) {
                Ok(decision) => decision,
                Err(no_route) => {
                    return Err(last_error.map_or(PipelineError::NoRoute(no_route), |last| {
                        PipelineError::AllFailed { last, rejected }
                    }));
                }
            }
        };

        let Some(backend) = state.backends.get(&decision.backend) else {
            excluded.insert(decision.backend.clone());
            continue;
        };

        attempts += 1;
        // The wire this backend prefers is the one a translated route targets.
        // The router already established that it does not speak `inbound`, or
        // `translated` would be false.
        let target = backend.capabilities().wires.primary();
        let mut request = if decision.translated {
            match translate_request(
                &body,
                path,
                inbound,
                target,
                &decision,
                peek,
                capture_logprobs(&state.config.capture),
            ) {
                Ok(request) => request,
                Err(reason) => {
                    // Refusing beats sending a body the target cannot parse.
                    rejected.push((decision.backend.to_string(), reason.clone()));
                    excluded.insert(decision.backend.clone());
                    if let Ok(mut policy) = state.policy.lock() {
                        policy.forget(&key);
                    }
                    continue;
                }
            }
        } else {
            UpstreamRequest {
                path: path.to_string(),
                body: strip_unsupported_tools(
                    &apply_model(&body, decision.model.as_deref()),
                    inbound,
                    &backend.capabilities().unsupported_responses_tools,
                    &decision.backend,
                ),
                headers: headers.clone(),
                stream: peek.stream,
                expected_account_scope: None,
            }
        };
        request.expected_account_scope = available
            .iter()
            .find(|candidate| candidate.id == decision.backend)
            .and_then(|candidate| candidate.account_scope.clone());

        let upstream_protocol = if decision.translated { target } else { inbound };
        let token_target = state
            .config
            .capture
            .token_capture
            .as_ref()
            .filter(|_| {
                state.config.capture.enabled
                    && state.ledger.is_some()
                    && session.is_some()
                    && upstream_protocol == Protocol::OpenAiChat
            })
            .filter(|capture| {
                capture.selects(
                    decision.backend.as_str(),
                    decision
                        .model
                        .as_deref()
                        .or(peek.requested_model.as_deref())
                        .unwrap_or(""),
                )
            })
            .zip(state.token_spool.as_ref())
            .and_then(|(config, store)| {
                if store
                    .can_accept_capture(session.as_deref().unwrap_or(""), request.body.len() as u64)
                    .ok()
                    != Some(true)
                {
                    if let Some(session) = session.as_ref() {
                        let _ = store.note_absence(
                            session,
                            &ironwire_ledger::token_spool::SpoolError::Capacity,
                            Utc::now().timestamp(),
                        );
                    }
                    return None;
                }
                match ironwire_core::token_capture::augment_chat(&request.body, config.top_k) {
                    Some(bytes) => {
                        request.body = Bytes::from(bytes);
                        Some((store.clone(), upstream_protocol.to_string(), peek.stream))
                    }
                    None => {
                        if let Some(session) = session.as_ref() {
                            let _ = store.note_absence(
                                session,
                                &ironwire_ledger::token_spool::SpoolError::Invalid,
                                Utc::now().timestamp(),
                            );
                        }
                        None
                    }
                }
            });

        if binding
            .as_ref()
            .is_some_and(|b| b.expires_at <= Utc::now().timestamp())
        {
            return Err(PipelineError::Admission(
                ironwire_core::admission::AdmissionError::Expired,
            ));
        }
        if let Some(binding) = &binding {
            request.body = ironwire_core::admission::insert_binding(&request.body, &binding.value)
                .map(Bytes::from)
                .map_err(PipelineError::Admission)?;
        }

        // Cloned before the request moves into the backend, because these are
        // the bytes the upstream will hash: `send` puts `request.body` on the
        // wire unchanged. Cheap -- `Bytes` is refcounted.
        //
        // With `capture.receipts` on and bodies off, only the digests are
        // taken -- and only for a backend that signs its answers, since a
        // digest is worth nothing without a receipt to check it against.
        let mut capture = if state.bodies.is_some() || token_target.is_some() {
            let mut capture = Capture::of_request(request.body.clone());
            capture.token_target = token_target;
            Some(capture)
        } else if state.ledger.is_some()
            && state.config.capture.receipts
            && backend.offers_receipts()
        {
            Some(Capture::digest_only(&request.body))
        } else {
            None
        };

        match backend.send(request).await {
            Ok(mut response) => {
                // Teed before translation, for the same reason. On a translated
                // route the bytes the client eventually sees are ours, not the
                // provider's, and only the provider's are in its receipt.
                if let Some(capture) = capture.as_mut() {
                    if let Some((_, _, streaming)) = capture.token_target.as_mut() {
                        *streaming = is_event_stream(&response.headers);
                    }
                    response.body = capture_stream(response.body, capture).boxed();
                }
                let response = if decision.translated {
                    translate_response(
                        response,
                        inbound,
                        target,
                        peek.requested_model.as_deref().unwrap_or("unknown"),
                        peek.stream,
                        confidence.clone(),
                    )
                } else {
                    response
                };
                // Headers are in hand, so the backend answered. Whether the
                // *stream* then stalls is a different failure, handled on the
                // open stream by `resilience` — a breaker cannot help there,
                // because by then the client is already committed.
                state.breakers.record_success(&decision.backend);
                // Announce only a real change. A sticky conversation producing
                // one "routed" line per turn would bury the one line that
                // means something (`crate::events`).
                state.set_last_route(crate::state::LastRoute {
                    backend: decision.backend.to_string(),
                    model: decision.model.clone(),
                    rung: decision.rung,
                    from: previous
                        .as_ref()
                        .filter(|p| *p != &decision.backend)
                        .map(ToString::to_string),
                    at: Utc::now(),
                });
                if previous.as_ref() != Some(&decision.backend) {
                    state.events.publish(crate::events::Event::Routed {
                        at: Utc::now(),
                        conversation: key.0.to_string(),
                        from: previous.as_ref().map(ToString::to_string),
                        to: decision.backend.to_string(),
                        rung: decision.rung,
                        translated: decision.translated,
                        reason: decision.reason.clone(),
                    });
                }
                return Ok((
                    response,
                    Routed {
                        decision,
                        attempts,
                        rejected,
                        confidence,
                        capture,
                    },
                ));
            }
            Err(error) => {
                tracing::warn!(
                    backend = %decision.backend,
                    error = %error,
                    retryable = error.is_retryable(),
                    "backend failed before first byte"
                );
                rejected.push((decision.backend.to_string(), error.to_string()));
                state
                    .breakers
                    .record_failure(&decision.backend, &error, Utc::now());

                // A rate limit teaches us something; fold it in so the next
                // decision does not pick the same backend again.
                if let Some(secs) = error.retry_after_secs() {
                    backend.record(&Observation {
                        retry_after_secs: Some(secs),
                        ..Observation::default()
                    });
                }

                if !error.is_retryable() {
                    return Err(PipelineError::Upstream(error));
                }

                // A 529/overload or a dropped connection is usually momentary.
                // Descending the ladder on one would move a warm conversation
                // off its subscription — and onto a metered key the user pays
                // for — over a blip. Try the same backend again first.
                if should_retry_same_backend(&error)
                    && same_backend_attempts < MAX_SAME_BACKEND_RETRIES
                {
                    same_backend_attempts += 1;
                    let backoff = backoff_for(same_backend_attempts);
                    tracing::info!(
                        backend = %decision.backend,
                        attempt = same_backend_attempts,
                        ?backoff,
                        "transient upstream failure; retrying the same backend before descending"
                    );
                    tokio::time::sleep(backoff).await;
                    last_error = Some(error);
                    continue;
                }

                same_backend_attempts = 0;
                // Drop this conversation's affinity: it is pinned to a backend
                // that just failed, and keeping it would re-select it.
                if let Ok(mut policy) = state.policy.lock() {
                    policy.forget(&key);
                }
                excluded.insert(decision.backend.clone());
                last_error = Some(error);
            }
        }
    }

    Err(match last_error {
        Some(last) => PipelineError::AllFailed { last, rejected },
        None => PipelineError::NoRoute(NoRoute::AllExhausted),
    })
}

/// How many times to retry the *same* backend on a transient failure before
/// descending the fidelity ladder. Small on purpose: the point is to ride out a
/// blip, not to wait out a real outage.
pub const MAX_SAME_BACKEND_RETRIES: usize = 2;

/// Whether this failure is worth another go at the same backend.
///
/// A 429 the provider attached a real wait to is *not* — it told us how long,
/// and sleeping through it would stall the agent when other capacity exists.
/// Everything else transient is: an overload or a reset usually clears.
#[must_use]
pub fn should_retry_same_backend(error: &UpstreamError) -> bool {
    match error {
        UpstreamError::RateLimited {
            retry_after_secs, ..
        } => retry_after_secs.is_none_or(|secs| secs <= 2),
        UpstreamError::Transport { .. } => true,
        UpstreamError::Upstream { status, .. } => status.is_server_error(),
        // A missing credential will not fix itself, and a host mismatch is our
        // own bug — retrying either just delays the real answer.
        UpstreamError::NeedsAuth { .. } | UpstreamError::CredentialHostMismatch { .. } => false,
    }
}

/// Exponential backoff for same-backend retries.
#[must_use]
pub fn backoff_for(attempt: usize) -> std::time::Duration {
    std::time::Duration::from_millis(250 * (1 << attempt.min(6)) as u64)
}

/// Re-express a request that arrived on `inbound` for a backend speaking
/// `target`.
///
/// Everything goes through the pivot IR (`docs/TRANSLATION.md`): parse on the
/// wire it came in on, emit on the wire it is going out on. Which pair that is
/// stops mattering here — the one thing this function still decides is what to
/// do about content the target cannot express.
///
/// # Errors
///
/// A human-readable reason when this request cannot be translated.
fn translate_request(
    body: &Bytes,
    path: &str,
    inbound: Protocol,
    target: Protocol,
    decision: &RouteDecision,
    peek: &RequestPeek,
    logprobs: bool,
) -> Result<UpstreamRequest, String> {
    // Only the completion endpoints translate. `count_tokens` has no equivalent
    // on the other wires, and answering it with a guess would corrupt the
    // client's context accounting — so it is refused rather than approximated.
    if !is_completion_path(path) {
        return Err(format!("{path} has no cross-family equivalent"));
    }
    let parsed: serde_json::Value = serde_json::from_slice(body)
        .map_err(|e| format!("could not parse the request body for translation: {e}"))?;
    let model = decision
        .model
        .as_deref()
        .or(peek.requested_model.as_deref())
        .unwrap_or("default");

    let mut conversation = ironwire_translate::parse_request(inbound, &parsed);
    // OR rather than assignment: a client that asked for log-probabilities
    // itself keeps them whatever the capture setting says. This is the only
    // place the setting is applied, and it is on the path that already builds a
    // fresh body — the native lane is never reached from here, so its
    // byte-identity claim (`docs/PROTOCOL.md` §2) is untouched. Which wires can
    // actually express it is the emitters' business, not this function's.
    conversation.params.logprobs |= logprobs;
    let (translated, dropped) = ironwire_translate::emit_request(target, &conversation, model);

    // A block type this build does not model makes the whole cross-wire route
    // ineligible. We cannot tell whether it was load-bearing — a `document` the
    // user asked a question about looks exactly like one that was decorative —
    // and answering about content the model never received is the silent
    // degradation `docs/PROTOCOL.md` §6 refuses. The native lane carries it
    // perfectly, so the cost is waiting for same-wire capacity.
    if !dropped.unknown_blocks.is_empty() {
        return Err(format!(
            "the request contains content this build cannot translate \
             ({}); routing it to a different API family would silently drop it",
            dropped.unknown_blocks.join(", ")
        ));
    }

    if !dropped.is_empty() {
        tracing::info!(
            backend = %decision.backend,
            %inbound,
            %target,
            reasoning_blocks = dropped.reasoning_blocks,
            cache_breakpoints = dropped.cache_breakpoints,
            images = dropped.images,
            "translated across wires; these did not survive"
        );
    }
    let body = serde_json::to_vec(&translated)
        .map_err(|e| format!("could not serialise the translated request: {e}"))?;

    Ok(UpstreamRequest {
        path: ironwire_translate::endpoint_path(target).to_string(),
        body: Bytes::from(body),
        // The inbound headers describe the protocol the request arrived on;
        // none of them mean anything to a backend speaking a different one.
        headers: Vec::new(),
        stream: peek.stream,
        expected_account_scope: None,
    })
}

/// Whether to ask a translated request for per-token log-probabilities.
///
/// Both switches, not either. `capture.logprobs` says the user wants the
/// signal; `capture.enabled` is what decides whether there is a ledger to write
/// the aggregate into. With capture off, asking would inflate every response on
/// the cross-family lane and be read by nobody, which is the one combination
/// worth refusing outright rather than honouring literally.
fn capture_logprobs(capture: &ironwire_core::config::CaptureConfig) -> bool {
    capture.enabled && capture.logprobs
}

/// Whether this path is a completion endpoint on some wire.
fn is_completion_path(path: &str) -> bool {
    path.ends_with("/v1/messages")
        || path.ends_with("/v1/responses")
        || path.ends_with("/v1/chat/completions")
}

/// Map an answer that arrived on `target` back into the shape the client asked
/// for on `inbound`.
fn translate_response(
    response: UpstreamResponse,
    inbound: Protocol,
    target: Protocol,
    requested_model: &str,
    stream: bool,
    confidence: ConfidenceSink,
) -> UpstreamResponse {
    let mut headers: Vec<(String, String)> = response
        .headers
        .into_iter()
        // Length and encoding describe the pre-translation body.
        .filter(|(name, _)| name != "content-length" && name != "content-encoding")
        .collect();
    headers.retain(|(name, _)| name != "content-type");
    headers.push((
        "content-type".to_string(),
        if stream {
            "text/event-stream".to_string()
        } else {
            "application/json".to_string()
        },
    ));

    let body = if stream {
        translated_stream(
            response.body,
            inbound,
            target,
            requested_model.to_string(),
            confidence,
        )
        .boxed()
    } else {
        translated_body(
            response.body,
            inbound,
            target,
            requested_model.to_string(),
            confidence,
        )
        .boxed()
    };

    UpstreamResponse {
        status: response.status,
        headers,
        body,
    }
}

/// Buffer a non-streaming response and re-emit it in the client's shape.
fn translated_body(
    inner: futures_util::stream::BoxStream<'static, Result<Bytes, UpstreamError>>,
    inbound: Protocol,
    target: Protocol,
    requested_model: String,
    confidence: ConfidenceSink,
) -> impl Stream<Item = Result<Bytes, UpstreamError>> + Send {
    futures_util::stream::once(async move {
        let collected: Vec<Bytes> = inner.filter_map(|c| async move { c.ok() }).collect().await;
        let mut raw = Vec::new();
        for chunk in collected {
            raw.extend_from_slice(&chunk);
        }
        let parsed: serde_json::Value =
            serde_json::from_slice(&raw).unwrap_or(serde_json::Value::Null);
        // A `stream: false` request paid for the inflated response too, so it
        // is read here rather than only on the streaming path. Capturing on one
        // path and not the other is the worst of both: cost with no signal.
        if target == Protocol::OpenAiChat {
            confidence.record(&ironwire_translate::chat::completion_token_logprobs(
                &parsed,
            ));
        }
        let completion = ironwire_translate::parse_completion(target, &parsed);
        let (translated, _) =
            ironwire_translate::emit_completion(inbound, &completion, &requested_model);
        Ok(Bytes::from(
            serde_json::to_vec(&translated).unwrap_or_else(|_| b"{}".to_vec()),
        ))
    })
}

/// Translate the event stream chunk by chunk, so text still arrives live.
fn translated_stream(
    inner: futures_util::stream::BoxStream<'static, Result<Bytes, UpstreamError>>,
    inbound: Protocol,
    target: Protocol,
    requested_model: String,
    confidence: ConfidenceSink,
) -> impl Stream<Item = Result<Bytes, UpstreamError>> + Send {
    let state = (
        inner,
        Some(ironwire_translate::Translator::new(
            target,
            inbound,
            requested_model,
        )),
        confidence,
    );
    futures_util::stream::unfold(
        state,
        |(mut inner, mut translator, confidence)| async move {
            let mut active = translator.take()?;
            loop {
                match inner.next().await {
                    Some(Ok(chunk)) => {
                        let out = active.push(&chunk);
                        if out.is_empty() {
                            // Nothing to forward yet — keep reading rather than
                            // emitting an empty frame.
                            continue;
                        }
                        translator = Some(active);
                        return Some((Ok(Bytes::from(out)), (inner, translator, confidence)));
                    }
                    // The upstream failed mid-stream. We are past the point of no
                    // return (PROTOCOL.md §5), so close the client's stream
                    // properly instead of leaving it hanging.
                    Some(Err(_)) | None => {
                        let out = active.finish();
                        // The last place the translator is whole. A client that
                        // disconnected early still leaves the aggregate over what
                        // it did receive, which is the same rule the observation
                        // path follows.
                        confidence.record(active.token_logprobs());
                        return Some((Ok(Bytes::from(out)), (inner, None, confidence)));
                    }
                }
            }
        },
    )
}

/// Rewrite the `model` key, and only that key.
///
/// Returns the original bytes untouched when policy did not change the model —
/// which is the common case, and the safest possible request: no parse, no
/// re-encode, no chance of a fidelity bug. `serde_json`'s `preserve_order`
/// feature keeps field order stable when an edit *is* needed.
#[must_use]
pub fn apply_model(body: &Bytes, model: Option<&str>) -> Bytes {
    let Some(model) = model else {
        return body.clone();
    };
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return body.clone();
    };
    let Some(object) = value.as_object_mut() else {
        return body.clone();
    };
    object.insert(
        "model".to_string(),
        serde_json::Value::String(model.to_string()),
    );
    serde_json::to_vec(&value).map_or_else(|_| body.clone(), Bytes::from)
}

/// Remove Responses `tools` entries this backend is known to reject.
///
/// The native lane forwards a body untouched, which is its whole point and is
/// right for every field a provider merely does not recognise. A tool `type`
/// is not one of those: NEAR AI deserialises `tools` strictly and answers a
/// `namespace` entry with
///
/// ```text
/// tools[8].type: unknown variant `namespace`, expected one of `function`,
/// `web_search`, `web_context_search`, `file_search`, `code_interpreter`,
/// `computer`, `mcp`
/// ```
///
/// — refusing the entire turn. Codex emits one `namespace` per connected MCP
/// server, so before this, any Codex user with a single MCP server configured
/// could not reach NEAR AI at all.
///
/// Only types the backend is *known* to reject are removed
/// ([`Capabilities::unsupported_responses_tools`]). Everything else, built-ins
/// included, goes through: `web_search` is on NEAR AI's own accepted list, and
/// stripping a tool the provider would have taken is a capability the user
/// silently loses.
///
/// The removal is logged with what went and how much, because the model not
/// having the user's MCP tools is a thing they need to be able to find out.
#[must_use]
pub fn strip_unsupported_tools(
    body: &Bytes,
    inbound: Protocol,
    unsupported: &[String],
    backend: &ironwire_core::protocol::BackendId,
) -> Bytes {
    // Only this wire has tool entries that are not functions, and the common
    // case is an empty list — neither is worth reparsing a body for.
    if inbound != Protocol::OpenAiResponses || unsupported.is_empty() {
        return body.clone();
    }
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return body.clone();
    };
    let Some(tools) = value
        .get_mut("tools")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return body.clone();
    };

    let mut removed: Vec<String> = Vec::new();
    tools.retain(|tool| {
        let Some(kind) = tool.get("type").and_then(serde_json::Value::as_str) else {
            return true;
        };
        if unsupported.iter().any(|rejected| rejected == kind) {
            removed.push(kind.to_string());
            return false;
        }
        true
    });

    if removed.is_empty() {
        return body.clone();
    }
    let mut kinds: Vec<&str> = removed.iter().map(String::as_str).collect();
    kinds.sort_unstable();
    kinds.dedup();
    tracing::warn!(
        %backend,
        count = removed.len(),
        types = kinds.join(", "),
        "removed tool declarations this backend rejects; the model will not \
         have those tools for this turn"
    );
    serde_json::to_vec(&value).map_or_else(|_| body.clone(), Bytes::from)
}

/// Largest body we will hold in memory to keep it.
///
/// Above this we keep nothing rather than a prefix: a stored body is only
/// worth having whole. The *digest* is not bounded by this -- it is taken as
/// the bytes stream past, so an oversized body still gets an honest digest and
/// simply has no stored copy.
const MAX_CAPTURE_BYTES: usize = 32 * 1024 * 1024;

/// The bodies one exchange put on the wire, held -- or only hashed -- for the
/// ledger.
///
/// Both halves are the *upstream* bytes, which is the only pair a receipt is
/// about: the request after any model override, privacy substitution or
/// translation, and the response before translation back and before the
/// privacy reverser. On a translated route the client's own bytes are a
/// different document entirely, and hashing those would fail against every
/// receipt while looking exactly like tampering.
///
/// The digest is SHA-256 over exactly those bytes, lowercase hex -- for a
/// streamed response, over the raw concatenated event stream. That is the
/// definition a NEAR AI receipt's `text` uses, pinned against a live capture in
/// `tests/proof_status.rs`.
///
/// Two modes. [`Self::of_request`] keeps the bytes as well, for
/// `capture.bodies` and token capture. [`Self::digest_only`] keeps nothing but
/// the two digests, for `capture.receipts` with bodies off: the response is
/// hashed as it streams and never accumulated, so nothing of the user's
/// content outlives the exchange in this process or on disk.
#[derive(Clone)]
pub struct Capture {
    /// Exactly the bytes handed to the backend, when bodies are kept.
    request: Option<Bytes>,
    /// Digest of those bytes, taken up front.
    request_sha256: String,
    /// Whether the response bytes are accumulated as well as hashed.
    keep_bodies: bool,
    /// The response, once its stream has been read cleanly to its end.
    ///
    /// A cancelled, restarted or failed response leaves this empty on
    /// purpose. There is no honest digest of a response that did not finish.
    response: Arc<std::sync::Mutex<Option<CapturedResponse>>>,
    token_target: Option<(Arc<ironwire_ledger::token_spool::TokenSpool>, String, bool)>,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture")
            .field("complete", &self.response_sha256().is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct CapturedResponse {
    sha256: String,
    /// `None` in digest-only mode, and when the body outgrew
    /// [`MAX_CAPTURE_BYTES`].
    bytes: Option<Bytes>,
}

impl Capture {
    /// Keep the bodies and their digests, given the request bytes about to be
    /// sent.
    #[must_use]
    pub fn of_request(request: Bytes) -> Self {
        Self {
            request_sha256: ironwire_ledger::bodies::sha256_hex(&request),
            request: Some(request),
            keep_bodies: true,
            response: Arc::new(std::sync::Mutex::new(None)),
            token_target: None,
        }
    }

    /// Keep only the digests. The request bytes are hashed here and not held.
    #[must_use]
    pub fn digest_only(request: &[u8]) -> Self {
        Self {
            request_sha256: ironwire_ledger::bodies::sha256_hex(request),
            request: None,
            keep_bodies: false,
            response: Arc::new(std::sync::Mutex::new(None)),
            token_target: None,
        }
    }

    /// The request bytes, when this capture keeps bodies.
    #[must_use]
    pub fn request(&self) -> Option<&Bytes> {
        self.request.as_ref()
    }

    /// Digest of the request bytes.
    #[must_use]
    pub fn request_sha256(&self) -> &str {
        &self.request_sha256
    }

    fn finished(&self) -> Option<CapturedResponse> {
        match self.response.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// The response bytes, once the stream finished cleanly and when they
    /// were kept.
    #[must_use]
    pub fn response(&self) -> Option<Bytes> {
        self.finished()?.bytes
    }

    /// Digest of the response bytes, once the stream finished cleanly.
    #[must_use]
    pub fn response_sha256(&self) -> Option<String> {
        Some(self.finished()?.sha256)
    }
}

/// Tee a response stream into a [`Capture`], forwarding every byte untouched.
///
/// Structurally the same rule as `observe_body`: the copy can only ever learn
/// less, and can never stall, alter or fail the forwarded bytes. It differs in
/// one way that matters -- it deliberately does **not** flush on `Drop`. A
/// dropped stream is a response the client never saw whole, and recording its
/// digest would claim we hashed a complete body we did not have.
pub fn capture_stream<S>(
    inner: S,
    capture: &Capture,
) -> impl Stream<Item = Result<Bytes, UpstreamError>> + Send + use<S>
where
    S: Stream<Item = Result<Bytes, UpstreamError>> + Send + Unpin + 'static,
{
    struct Tee<S> {
        inner: S,
        digest: ironwire_ledger::bodies::StreamingSha256,
        /// The bytes so far, while they are being kept at all.
        buffer: Option<Vec<u8>>,
        /// Set once the stream yielded an error; from then on nothing will be
        /// recorded.
        ///
        /// Needs its own latch because a failed stream still ends in
        /// `Ready(None)` afterwards, which would otherwise look exactly like a
        /// clean finish and put the digest of half a response on the row.
        spoiled: bool,
        sink: Arc<std::sync::Mutex<Option<CapturedResponse>>>,
    }

    impl<S> Stream for Tee<S>
    where
        S: Stream<Item = Result<Bytes, UpstreamError>> + Unpin,
    {
        type Item = Result<Bytes, UpstreamError>;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            let this = self.get_mut();
            let polled = std::pin::Pin::new(&mut this.inner).poll_next(cx);
            match &polled {
                std::task::Poll::Ready(Some(Ok(chunk))) if !this.spoiled => {
                    this.digest.update(chunk);
                    if let Some(buffer) = this.buffer.as_mut() {
                        if buffer.len().saturating_add(chunk.len()) > MAX_CAPTURE_BYTES {
                            // Too big to keep; still hashed.
                            this.buffer = None;
                        } else {
                            buffer.extend_from_slice(chunk);
                        }
                    }
                }
                std::task::Poll::Ready(Some(Err(_))) => {
                    this.spoiled = true;
                    this.buffer = None;
                }
                // A clean end, and only a clean end. An error mid-stream leaves
                // the capture empty for the same reason a drop does.
                std::task::Poll::Ready(None) if !this.spoiled => {
                    // Latch, so a stream polled again after its end cannot
                    // record a second, different answer.
                    this.spoiled = true;
                    let finished = CapturedResponse {
                        sha256: std::mem::take(&mut this.digest).finish_hex(),
                        bytes: this.buffer.take().map(Bytes::from),
                    };
                    match this.sink.lock() {
                        Ok(mut guard) => *guard = Some(finished),
                        Err(poisoned) => *poisoned.into_inner() = Some(finished),
                    }
                }
                _ => {}
            }
            polled
        }
    }

    Tee {
        inner,
        digest: ironwire_ledger::bodies::StreamingSha256::new(),
        buffer: capture.keep_bodies.then(Vec::new),
        spoiled: false,
        sink: Arc::clone(&capture.response),
    }
}

/// SSE dialect matching an inbound protocol.
#[must_use]
pub fn dialect_for(protocol: Protocol) -> Dialect {
    match protocol {
        Protocol::AnthropicMessages => Dialect::Anthropic,
        Protocol::OpenAiResponses => Dialect::OpenAiResponses,
        Protocol::OpenAiChat => Dialect::OpenAiChat,
    }
}

/// Wrap a response body so it is observed on the way past.
///
/// The observer reads a copy and can only ever learn less; it has no way to
/// stall, alter or fail the forwarded bytes. `on_finish` runs when the stream
/// ends — including when the client disconnects early, so an abandoned request
/// still records what it consumed.
pub fn observe_body<S>(
    inner: S,
    dialect: Dialect,
    streaming: bool,
    on_finish: impl FnOnce(Observation) + Send + 'static,
) -> impl Stream<Item = Result<Bytes, UpstreamError>> + Send
where
    S: Stream<Item = Result<Bytes, UpstreamError>> + Send + Unpin + 'static,
{
    // The callback is boxed rather than generic so `Tee` is unconditionally
    // `Unpin`: a `Drop` impl cannot carry bounds the struct does not, and we
    // need `Drop` to be the thing that guarantees the flush.
    struct Tee<S> {
        inner: S,
        observer: Option<SseObserver>,
        on_finish: Option<Box<dyn FnOnce(Observation) + Send>>,
    }

    impl<S> Tee<S> {
        fn flush(&mut self) {
            if let (Some(observer), Some(on_finish)) = (self.observer.take(), self.on_finish.take())
            {
                on_finish(observer.finish());
            }
        }
    }

    // Firing on Drop rather than only on stream end is what makes cancellation
    // accounting correct: a client that hits Esc mid-response still leaves us
    // with whatever the provider had already reported.
    impl<S> Drop for Tee<S> {
        fn drop(&mut self) {
            self.flush();
        }
    }

    impl<S> Stream for Tee<S>
    where
        S: Stream<Item = Result<Bytes, UpstreamError>> + Unpin,
    {
        type Item = Result<Bytes, UpstreamError>;

        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            let this = self.get_mut();
            let polled = std::pin::Pin::new(&mut this.inner).poll_next(cx);
            match &polled {
                std::task::Poll::Ready(Some(Ok(chunk))) => {
                    if let Some(observer) = this.observer.as_mut() {
                        observer.push(chunk);
                    }
                }
                std::task::Poll::Ready(_) => this.flush(),
                std::task::Poll::Pending => {}
            }
            polled
        }
    }

    Tee {
        inner,
        observer: Some(if streaming {
            SseObserver::new(dialect)
        } else {
            SseObserver::for_document(dialect)
        }),
        on_finish: Some(Box::new(on_finish)),
    }
}

/// Fold an observation into a backend's quota state.
pub fn record(backend: &Arc<dyn Backend>, observation: &Observation) {
    if !observation.is_empty() {
        backend.record(observation);
    }
}

/// Everything the ledger needs that the observation itself does not carry.
///
/// Assembled before the response streams and consumed when it ends — including
/// when the client disconnects early, so an abandoned request is still on the
/// record with whatever the provider had already reported.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LedgerContext {
    /// When the request arrived.
    pub started_at: chrono::DateTime<chrono::Utc>,
    /// Monotonic start, for durations.
    pub started: std::time::Instant,
    /// Façade that received it.
    pub facade: &'static str,
    /// Path beneath the façade.
    pub path: String,
    /// Opaque conversation key.
    pub conversation: String,
    /// The session id the client sent, when it sent one. Read from the header
    /// named by `ironwire_upstream::headers::client_session_header`.
    pub client_session_id: Option<String>,
    /// Backend that served it.
    pub backend: String,
    /// Whether that backend is metered, so its cost counts against a cap.
    pub backend_is_metered: bool,
    /// Whether that backend is local capacity.
    ///
    /// Carried rather than inferred from the id, because the id is
    /// user-supplied. It decides one thing: whether this exchange gets a price
    /// at all (see [`Self::write`]).
    pub backend_is_local: bool,
    /// Whether that backend signs its answers, so the row starts `pending` a
    /// proof check rather than `outside` (`crate::proof`).
    pub backend_offers_receipts: bool,
    /// Model the client asked for.
    pub requested_model: Option<String>,
    /// Fidelity rung, lowercased.
    pub rung: String,
    /// Backends tried before this one succeeded.
    pub attempts: usize,
    /// Distinct values the privacy filter substituted, or `None` when the
    /// filter was off. Not conflated with zero: a user reading the log to
    /// decide whether the filter is working needs to tell "off" from "on and
    /// found nothing" (`docs/PRIVACY.md` §7).
    pub substitutions: Option<i64>,
    /// Status returned to the client.
    pub status: u16,
    /// Where the translated response leaves its confidence aggregate.
    ///
    /// A handle rather than a value: this context is assembled before the
    /// response streams, and the aggregate is only known once it has finished.
    pub confidence: ConfidenceSink,
    /// The bodies this exchange put on the wire, when capture is on.
    pub capture: Option<Capture>,
    /// Where captured bodies are written. `None` when `capture.bodies` is off.
    pub bodies: Option<Arc<ironwire_ledger::bodies::BodyStore>>,
}

impl LedgerContext {
    /// Write one exchange.
    ///
    /// Never propagates: a ledger problem must not fail a user's inference
    /// request, and by the time this runs the response has already been
    /// delivered anyway.
    pub fn write(
        self,
        ledger: &Ledger,
        spend: &std::sync::Mutex<crate::spend::SpendTracker>,
        observation: &Observation,
    ) {
        let usage = observation.usage;
        // A local model has no price, and must not be given one. The price
        // table matches on the slug, so a local `llama3.3:70b` — or any slug
        // colliding with a hosted name — would be costed against a cloud rate
        // and reported as money the user did not spend. `None`, not `Some(0.0)`:
        // a measured zero sums into `Summary::cost_usd` as though it were an
        // observation, and the ledger's rule throughout is that an absent
        // number beats a fabricated one.
        let cost_usd = if self.backend_is_local {
            None
        } else {
            // Priced against whichever model actually served it, so a fallback
            // to a cheaper model shows up as a cheaper turn.
            ironwire_ledger::price(
                observation
                    .served_model
                    .as_deref()
                    .or(self.requested_model.as_deref())
                    .unwrap_or("unknown"),
                usage.and_then(|u| {
                    Some((
                        u32::try_from(u.input_tokens).ok()?,
                        u32::try_from(u.output_tokens).ok()?,
                        u32::try_from(u.cache_read_tokens).ok()?,
                        u32::try_from(u.cache_creation_tokens).ok()?,
                    ))
                }),
            )
        };
        // Both halves or neither. A request digest on a row with no response
        // digest invites the reader to check half a receipt against a body
        // that finished somewhere we did not see.
        let digests = self.capture.as_ref().and_then(|capture| {
            Some((
                capture.request_sha256().to_string(),
                capture.response_sha256()?,
            ))
        });
        // Bodies only when both halves were kept and there is a store to put
        // them in. A digest-only capture has neither, by design.
        let kept = self
            .capture
            .as_ref()
            .and_then(|capture| Some((capture.request()?.clone(), capture.response()?)));
        let body_ref = match (&digests, &kept, &self.bodies) {
            (Some(_), Some((request, response)), Some(store)) => {
                match store.store(request, response) {
                    Ok(reference) => Some(reference),
                    Err(error) => {
                        // The digests are still true, and are what a receipt is
                        // checked against; losing the bodies costs the trace,
                        // not the verification.
                        tracing::debug!(%error, "could not write the captured bodies");
                        None
                    }
                }
            }
            _ => None,
        };
        let (request_sha256, response_sha256) = match digests {
            Some((request, response)) => (Some(request), Some(response)),
            None => (None, None),
        };
        let exchange = Exchange {
            // Assigned by SQLite on insert; an exchange on its way in has none.
            id: None,
            started_at: self.started_at,
            ttfb_ms: None,
            total_ms: i64::try_from(self.started.elapsed().as_millis()).ok(),
            facade: self.facade.to_string(),
            path: self.path,
            conversation: self.conversation,
            client_session_id: self.client_session_id,
            backend: self.backend,
            requested_model: self.requested_model,
            served_model: observation.served_model.clone(),
            upstream_id: observation.upstream_id.clone(),
            model_alias_resolved: observation.model_alias_resolved.clone(),
            request_sha256,
            response_sha256,
            body_ref,
            rung: self.rung,
            attempts: i64::try_from(self.attempts).unwrap_or(i64::MAX),
            // `None`, not `0`, when the provider reported nothing: a fabricated
            // zero would silently understate the user's spend.
            input_tokens: usage.and_then(|u| i64::try_from(u.input_tokens).ok()),
            cache_read_tokens: usage.and_then(|u| i64::try_from(u.cache_read_tokens).ok()),
            cache_write_tokens: usage.and_then(|u| i64::try_from(u.cache_creation_tokens).ok()),
            output_tokens: usage.and_then(|u| i64::try_from(u.output_tokens).ok()),
            cost_usd,
            substitutions: self.substitutions,
            status: i64::from(self.status),
            error: None,
            // Read here rather than earlier: this runs when the body ends or is
            // dropped, which is the first moment the whole response has been
            // translated. `None` on every native-lane row, and on every
            // cross-family one where nothing asked for log-probabilities.
            confidence: self.confidence.get(),
            // Settled later, off this path, by `crate::proof`.
            proof: Some(if self.backend_offers_receipts {
                ironwire_ledger::ProofStatus::Pending
            } else {
                ironwire_ledger::ProofStatus::Outside
            }),
        };
        // Metered spend only, and recorded even when the exchange failed:
        // tokens burned by a request that 500'd were still billed.
        if self.backend_is_metered
            && let Some(cost) = exchange.cost_usd
        {
            match spend.lock() {
                Ok(mut tracker) => tracker.record(
                    &ironwire_core::protocol::BackendId::from(exchange.backend.as_str()),
                    cost,
                    exchange.started_at,
                ),
                Err(poisoned) => poisoned.into_inner().record(
                    &ironwire_core::protocol::BackendId::from(exchange.backend.as_str()),
                    cost,
                    exchange.started_at,
                ),
            }
        }
        let recorded = match ledger.record(&exchange) {
            Ok(id) => Some(id),
            Err(error) => {
                tracing::debug!(%error, "could not write the trace ledger entry");
                None
            }
        };
        if let (Some(capture), Some(session)) =
            (self.capture.as_ref(), exchange.client_session_id.as_ref())
            && let Some((spool, protocol, streaming)) = &capture.token_target
        {
            let result = match (recorded, kept.as_ref()) {
                (Some(id), Some((request, response))) => spool
                    .record(
                        session,
                        id,
                        protocol,
                        *streaming,
                        (request, response),
                        chrono::Utc::now().timestamp(),
                    )
                    .map(|_| ()),
                _ => Err(ironwire_ledger::token_spool::SpoolError::Unavailable),
            };
            if let Err(error) = result {
                let _ = spool.note_absence(session, &error, chrono::Utc::now().timestamp());
                tracing::debug!(%error,"detailed capture unavailable for completed exchange");
            }
        }
        // The rolling window. Only ever rotates rows that are already in the
        // ledger, which means exchanges whose response finished -- an in-flight
        // one has no row and no files yet, so this cannot delete a body that is
        // still streaming. Ordered by row id, i.e. by completion: two turns of
        // one session can share a timestamp, and the later *finisher* is the
        // one worth keeping.
        if let (Some(id), Some(store)) = (recorded, &self.bodies)
            && exchange.body_ref.is_some()
        {
            match ledger.supersede_bodies(exchange.retention_key(), id) {
                Ok(superseded) => {
                    for reference in superseded {
                        if let Err(error) = store.remove(&reference) {
                            // The reference is already gone from the row, so
                            // the file is an orphan the startup sweep collects.
                            tracing::debug!(%error, "could not release a superseded body");
                        }
                    }
                }
                Err(error) => tracing::debug!(%error, "could not roll the body window"),
            }
        }
    }
}

/// Convenience wrapper so a boxed upstream body can be observed.
pub fn observe_boxed(
    body: futures_util::stream::BoxStream<'static, Result<Bytes, UpstreamError>>,
    dialect: Dialect,
    streaming: bool,
    on_finish: impl FnOnce(Observation) + Send + 'static,
) -> futures_util::stream::BoxStream<'static, Result<Bytes, UpstreamError>> {
    observe_body(body, dialect, streaming, on_finish).boxed()
}

/// Whether a response is an event stream, from what it says it is.
///
/// Read from the response rather than the request: a provider may answer a
/// streaming request with a plain body when something goes wrong, and it is the
/// shape that came back that decides how to read it.
#[must_use]
pub fn is_event_stream(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value.to_ascii_lowercase().contains("text/event-stream")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;
    use std::sync::Mutex;

    /// Reducing log-probabilities and then dropping the result on the floor is
    /// the failure this whole path exists to avoid, so the assertion is that
    /// the aggregate reaches a ledger row a person can query — not that some
    /// intermediate held it.
    #[tokio::test]
    async fn a_streamed_confidence_aggregate_reaches_the_ledger() {
        let frames = [
            r#"data: {"id":"c","choices":[{"index":0,"delta":{"content":"Hi"},"logprobs":{"content":[{"token":"Hi","logprob":-0.6931471805599453}]}}]}"#,
            r#"data: {"id":"c","choices":[{"index":0,"finish_reason":"stop","logprobs":{"content":[{"token":"!","logprob":-0.6931471805599453}]}}]}"#,
            "data: [DONE]",
        ]
        .join("\n\n");

        let confidence = ConfidenceSink::default();
        let body = stream::iter(vec![Ok(Bytes::from(frames))]).boxed();
        let translated = translated_stream(
            body,
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            "claude-opus-4-6".to_string(),
            confidence.clone(),
        );
        let _: Vec<_> = translated.collect().await;

        let ledger = ironwire_ledger::Ledger::in_memory().expect("ledger");
        let spend = Mutex::new(crate::spend::SpendTracker::default());
        ledger_context(confidence).write(&ledger, &spend, &Observation::default());

        let row = ledger.recent(1).expect("reads");
        let aggregate = row[0].confidence.expect("the aggregate reached the row");
        assert_eq!(aggregate.token_count, 2);
        assert!((aggregate.mean_confidence - 0.5).abs() < 1e-6);
    }

    /// `stream: false` pays for the same inflated response. Capturing only on
    /// the streaming path would mean the cost with none of the signal.
    #[tokio::test]
    async fn a_non_streamed_answer_is_reduced_too() {
        let body = Bytes::from_static(
            br#"{"id":"c","choices":[{"index":0,"message":{"content":"Hi"},"finish_reason":"stop",
                 "logprobs":{"content":[{"token":"Hi","logprob":-0.6931471805599453}]}}]}"#,
        );
        let confidence = ConfidenceSink::default();
        let translated = translated_body(
            stream::iter(vec![Ok(body)]).boxed(),
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            "claude-opus-4-6".to_string(),
            confidence.clone(),
        );
        let _: Vec<_> = translated.collect().await;

        let aggregate = confidence.get().expect("an aggregate");
        assert_eq!(aggregate.token_count, 1);
        assert!((aggregate.mean_confidence - 0.5).abs() < 1e-6);
    }

    /// The overwhelmingly common row: capture off, nothing captured, and no
    /// measured-looking zero written where there was no measurement.
    #[tokio::test]
    async fn a_response_with_no_logprobs_writes_no_confidence() {
        let frames = r#"data: {"id":"c","choices":[{"index":0,"delta":{"content":"Hi"},"finish_reason":"stop"}]}"#;
        let confidence = ConfidenceSink::default();
        let translated = translated_stream(
            stream::iter(vec![Ok(Bytes::from(frames))]).boxed(),
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            "claude-opus-4-6".to_string(),
            confidence.clone(),
        );
        let _: Vec<_> = translated.collect().await;

        let ledger = ironwire_ledger::Ledger::in_memory().expect("ledger");
        let spend = Mutex::new(crate::spend::SpendTracker::default());
        ledger_context(confidence).write(&ledger, &spend, &Observation::default());
        assert!(ledger.recent(1).expect("reads")[0].confidence.is_none());
    }

    /// A backend that signs nothing is outside from the moment it is
    /// recorded; one that does starts `pending`, never anything stronger.
    #[test]
    fn the_first_proof_label_is_decided_by_whether_the_backend_signs() {
        let ledger = ironwire_ledger::Ledger::in_memory().expect("ledger");
        let spend = Mutex::new(crate::spend::SpendTracker::default());
        ledger_context(ConfidenceSink::default()).write(&ledger, &spend, &Observation::default());
        let mut signing = ledger_context(ConfidenceSink::default());
        signing.backend_offers_receipts = true;
        signing.write(&ledger, &spend, &Observation::default());

        let rows = ledger.recent(2).expect("reads");
        assert_eq!(rows[0].proof, Some(ironwire_ledger::ProofStatus::Pending));
        assert_eq!(rows[1].proof, Some(ironwire_ledger::ProofStatus::Outside));
    }

    fn ledger_context(confidence: ConfidenceSink) -> LedgerContext {
        LedgerContext {
            started_at: Utc::now(),
            started: std::time::Instant::now(),
            facade: "anthropic",
            path: "/v1/messages".to_string(),
            conversation: "c-1".to_string(),
            client_session_id: None,
            backend: "near-ai".to_string(),
            backend_is_metered: false,
            backend_is_local: false,
            backend_offers_receipts: false,
            requested_model: Some("claude-opus-4-6".to_string()),
            rung: "translated".to_string(),
            attempts: 1,
            substitutions: None,
            status: 200,
            confidence,
            capture: None,
            bodies: None,
        }
    }

    #[test]
    fn an_unchanged_model_forwards_the_original_bytes_exactly() {
        // The safest request is the one we did not re-encode.
        let body = Bytes::from_static(br#"{"model":"claude-opus-4-6","weird":[1,2,3]}"#);
        let out = apply_model(&body, None);
        assert_eq!(out, body);
    }

    #[test]
    fn a_model_edit_changes_only_the_model() {
        let body = Bytes::from_static(
            br#"{"model":"claude-opus-4-6","stream":true,"future_field":{"a":1}}"#,
        );
        let out = apply_model(&body, Some("claude-sonnet-4-6"));
        let value: serde_json::Value = serde_json::from_slice(&out).expect("valid json");
        assert_eq!(value["model"], "claude-sonnet-4-6");
        assert_eq!(value["stream"], true);
        assert_eq!(
            value["future_field"]["a"], 1,
            "fields we do not model must survive an edit"
        );
    }

    #[test]
    fn a_model_edit_preserves_field_order() {
        let body = Bytes::from_static(br#"{"model":"a","zzz":1,"aaa":2}"#);
        let out = apply_model(&body, Some("b"));
        let text = String::from_utf8(out.to_vec()).expect("utf8");
        assert!(
            text.find("zzz").unwrap() < text.find("aaa").unwrap(),
            "preserve_order must keep the client's field order: {text}"
        );
    }

    #[test]
    fn an_unparseable_body_is_forwarded_rather_than_mangled() {
        // If we cannot understand it, passing it through unchanged is strictly
        // better than guessing.
        let body = Bytes::from_static(b"not json");
        assert_eq!(apply_model(&body, Some("x")), body);
    }

    #[tokio::test]
    async fn the_tee_forwards_bytes_verbatim_and_reports_usage() {
        let frames = vec![
            Ok(Bytes::from_static(
                br#"data: {"type":"message_start","message":{"model":"claude-opus-4-6","usage":{"input_tokens":7}}}"#,
            )),
            Ok(Bytes::from_static(b"\n\n")),
            Ok(Bytes::from_static(
                br#"data: {"type":"message_delta","usage":{"output_tokens":42}}"#,
            )),
            Ok(Bytes::from_static(b"\n\n")),
        ];
        let expected: Vec<u8> = frames
            .iter()
            .filter_map(|f| f.as_ref().ok())
            .flat_map(|b| b.to_vec())
            .collect();

        let seen = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        let observed = observe_body(stream::iter(frames), Dialect::Anthropic, true, move |obs| {
            *sink.lock().expect("lock") = Some(obs);
        });

        let collected: Vec<u8> = observed
            .filter_map(|r| async move { r.ok() })
            .flat_map(|b| stream::iter(b.to_vec()))
            .collect()
            .await;

        assert_eq!(collected, expected, "forwarded bytes must be verbatim");
        let obs = seen.lock().expect("lock").clone().expect("observed");
        let usage = obs.usage.expect("usage");
        assert_eq!(usage.input_tokens, 7);
        assert_eq!(usage.output_tokens, 42);
    }

    #[tokio::test]
    async fn abandoning_the_stream_still_records_what_was_consumed() {
        // A user hitting Esc must not lose the quota accounting for the tokens
        // the provider already generated.
        let frames = vec![
            Ok(Bytes::from_static(
                br#"data: {"type":"message_start","message":{"model":"m","usage":{"input_tokens":9}}}"#,
            )),
            Ok(Bytes::from_static(b"\n\n")),
            Ok(Bytes::from_static(b"data: never read\n\n")),
        ];
        let seen = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        let mut observed = Box::pin(observe_body(
            stream::iter(frames),
            Dialect::Anthropic,
            true,
            move |obs| {
                *sink.lock().expect("lock") = Some(obs);
            },
        ));

        // Read two chunks, then drop mid-stream.
        let _ = observed.next().await;
        let _ = observed.next().await;
        drop(observed);

        let obs = seen
            .lock()
            .expect("lock")
            .clone()
            .expect("recorded on drop");
        assert_eq!(obs.usage.expect("usage").input_tokens, 9);
    }

    #[tokio::test]
    async fn a_stream_error_still_flushes_the_observation() {
        let frames = vec![
            Ok(Bytes::from_static(
                br#"data: {"type":"message_start","message":{"model":"m","usage":{"input_tokens":3}}}"#,
            )),
            Ok(Bytes::from_static(b"\n\n")),
            Err(UpstreamError::Transport {
                backend: ironwire_core::protocol::BackendId::from("x"),
                detail: "reset".into(),
            }),
        ];
        let seen = Arc::new(Mutex::new(None));
        let sink = Arc::clone(&seen);
        let observed = observe_body(stream::iter(frames), Dialect::Anthropic, true, move |obs| {
            *sink.lock().expect("lock") = Some(obs);
        });
        let _: Vec<_> = observed.collect().await;
        assert!(seen.lock().expect("lock").is_some());
    }
}
