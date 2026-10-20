//! Keeping a streamed response alive, and ending it honestly when it dies.
//!
//! Three failures produce "API Error: Response stalled mid-stream. The response
//! above may be incomplete." in Claude Code, and a proxy can do something about
//! each one.
//!
//! **1. The upstream is alive but quiet.** Anthropic streams `message_start`,
//! then often a long silence while the model thinks, then content. If nothing
//! crosses the wire for longer than the client's patience, the client gives up
//! on a request that was going to succeed. We emit `ping` events during silence
//! — they are part of the Anthropic event stream, carry nothing, and say the
//! true thing: still connected, still waiting. We stop the moment the socket
//! actually breaks, so this keeps a live request alive without ever masking a
//! dead one, and we give up ourselves at [`ResilienceConfig::stall_timeout`]
//! rather than pinging into the void.
//!
//! **2. The upstream dies mid-stream.** Today the connection simply ends: no
//! terminal event, so the client infers a stall. An `error` event costs nothing
//! and turns "something may be missing" into a stated failure the agent can
//! surface and the user can act on.
//!
//! **3. The upstream dies during the thinking gap.** This is the common one,
//! and it is fully recoverable — see below.
//!
//! # The point of no return is the first *content* byte
//!
//! `docs/PROTOCOL.md` §5 originally said retry ends at the first byte of the
//! response body. That was too strict, and it gave away the most recoverable
//! failure there is.
//!
//! `message_start` and `ping` carry no content: they are an envelope and a
//! heartbeat. A stream that has emitted only those has told the client nothing
//! it would have to un-tell. So we **hold them** and retry freely until the
//! first content-bearing event, which is precisely where the thinking-gap
//! failures live. Past that point nothing is retried, because replaying would
//! duplicate text the client has already committed to its transcript — that
//! rule is unchanged and it is the one that matters.

use std::time::Duration;

use bytes::Bytes;
use futures_util::stream::BoxStream;
use futures_util::{Stream, StreamExt};
use ironwire_upstream::backend::UpstreamError;
use ironwire_upstream::sse::Dialect;

/// How aggressively to keep a quiet stream alive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResilienceConfig {
    /// Emit a `ping` after this much silence. Well under any plausible client
    /// stall timeout, because the cost of an extra ping is nothing and the cost
    /// of being one second too slow is a failed turn.
    pub keepalive: Duration,
    /// Give up after this much *continuous* silence. Without a cap we would
    /// ping forever at a hung upstream, which is a worse failure than a clean
    /// error: the user waits instead of retrying.
    pub stall_timeout: Duration,
    /// How many times to transparently restart a stream that died **before**
    /// committing any content.
    pub max_reconnects: usize,
}

impl From<&ironwire_core::config::ResilienceConfig> for ResilienceConfig {
    fn from(config: &ironwire_core::config::ResilienceConfig) -> Self {
        Self::for_turn(config, false, false)
    }
}

impl ResilienceConfig {
    /// Settings for one turn.
    ///
    /// A compaction turn sends the whole conversation and thinks for far longer
    /// before its first token, so it gets a longer stall timeout
    /// (`docs/PROTOCOL.md` §8), and so does a turn served by a local model,
    /// which is the slowest thing IronWire routes to. Everything else is
    /// unchanged: the keepalive cadence is about the *client's* patience, which
    /// does not vary by turn.
    #[must_use]
    pub fn for_turn(
        config: &ironwire_core::config::ResilienceConfig,
        likely_compaction: bool,
        is_local: bool,
    ) -> Self {
        Self {
            keepalive: Duration::from_secs(config.keepalive_secs.max(1)),
            stall_timeout: Duration::from_secs(
                config
                    .stall_timeout_for_backend(likely_compaction, is_local)
                    .max(1),
            ),
            max_reconnects: config.max_reconnects,
        }
    }
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            keepalive: Duration::from_secs(15),
            stall_timeout: Duration::from_secs(180),
            max_reconnects: 2,
        }
    }
}

/// A content-free heartbeat in the dialect the client is speaking.
///
/// Anthropic has a `ping` event; the OpenAI dialects do not, so we use an SSE
/// **comment**, which every conforming parser ignores. Inventing an event type
/// the client has never seen would be a worse kind of help.
fn keepalive(dialect: Dialect) -> Bytes {
    match dialect {
        Dialect::Anthropic => Bytes::from_static(b"event: ping\ndata: {\"type\": \"ping\"}\n\n"),
        Dialect::OpenAiResponses | Dialect::OpenAiChat => {
            Bytes::from_static(b": ironwire-keepalive\n\n")
        }
    }
}

/// A terminal failure in the shape the client already handles.
fn error_event(dialect: Dialect, message: &str) -> Bytes {
    match dialect {
        Dialect::Anthropic => {
            let payload = serde_json::json!({
                "type": "error",
                "error": {"type": "api_error", "message": message},
            });
            Bytes::from(format!("event: error\ndata: {payload}\n\n"))
        }
        Dialect::OpenAiResponses => {
            let payload = serde_json::json!({
                "type": "response.failed",
                "response": {
                    "status": "failed",
                    "error": {"code": "api_error", "message": message},
                },
            });
            Bytes::from(format!("event: response.failed\ndata: {payload}\n\n"))
        }
        Dialect::OpenAiChat => {
            let payload = serde_json::json!({
                "error": {"type": "api_error", "message": message},
            });
            Bytes::from(format!("data: {payload}\n\n"))
        }
    }
}

/// What a pre-commitment frame should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Disposition {
    /// Content the client cannot be asked to un-see. Forwarding it ends the
    /// retry window.
    Commits,
    /// The message envelope. Held, so a restart before any content is
    /// invisible — and replaced rather than duplicated if a restart happens.
    HoldAsEnvelope,
    /// A keepalive. Forwarded immediately and never held: delaying a heartbeat
    /// defeats the only thing it is for.
    ForwardAsKeepalive,
}

fn disposition(dialect: Dialect, frame: &str) -> Disposition {
    // An SSE comment is a keepalive by definition, in every dialect.
    if frame
        .lines()
        .all(|line| line.trim().is_empty() || line.starts_with(':'))
    {
        return Disposition::ForwardAsKeepalive;
    }
    let event = frame
        .lines()
        .find_map(|line| line.strip_prefix("event:"))
        .map(str::trim);
    match dialect {
        Dialect::Anthropic => match event {
            Some("message_start") => Disposition::HoldAsEnvelope,
            Some("ping") => Disposition::ForwardAsKeepalive,
            Some(_) => Disposition::Commits,
            // A `data:`-only frame: read the payload, and treat anything we do
            // not recognise as content — assuming otherwise risks replaying it.
            None => {
                if frame.contains("message_start") {
                    Disposition::HoldAsEnvelope
                } else if frame.contains("\"ping\"") {
                    Disposition::ForwardAsKeepalive
                } else {
                    Disposition::Commits
                }
            }
        },
        // The Responses envelope: the response object exists but has produced
        // no output items yet — exactly the thinking window.
        Dialect::OpenAiResponses => {
            let name = event.map_or_else(|| frame.to_string(), str::to_string);
            if name.contains("response.created") || name.contains("response.in_progress") {
                Disposition::HoldAsEnvelope
            } else {
                Disposition::Commits
            }
        }
        // Chat Completions has no event names: its envelope is the first chunk,
        // which announces a role and carries nothing else.
        Dialect::OpenAiChat => {
            if !frame.contains("[DONE]") && is_chat_role_only(frame) {
                Disposition::HoldAsEnvelope
            } else {
                Disposition::Commits
            }
        }
    }
}

/// A Chat Completions chunk whose delta announces the role and nothing else.
fn is_chat_role_only(frame: &str) -> bool {
    let Some(payload) = frame.lines().find_map(|line| line.strip_prefix("data:")) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload.trim()) else {
        return false;
    };
    let Some(delta) = value
        .pointer("/choices/0/delta")
        .and_then(|d| d.as_object())
    else {
        return false;
    };
    delta.contains_key("role")
        && !delta.contains_key("tool_calls")
        && delta
            .get("content")
            .is_none_or(|c| c.as_str().is_none_or(str::is_empty))
}

/// Whether a frame ends the message cleanly.
fn is_terminal(dialect: Dialect, frame: &str) -> bool {
    match dialect {
        Dialect::Anthropic => {
            frame.contains("message_stop") || frame.contains("\"type\":\"error\"")
        }
        Dialect::OpenAiResponses => {
            frame.contains("response.completed")
                || frame.contains("response.failed")
                || frame.contains("response.incomplete")
        }
        Dialect::OpenAiChat => frame.contains("[DONE]"),
    }
}

/// Split complete SSE frames out of `buffer`, leaving any partial tail behind.
fn take_frames(buffer: &mut Vec<u8>) -> Vec<String> {
    let mut frames = Vec::new();
    loop {
        let lf = buffer.windows(2).position(|w| w == b"\n\n").map(|p| p + 2);
        let crlf = buffer
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .map(|p| p + 4);
        let Some(end) = (match (lf, crlf) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }) else {
            return frames;
        };
        let raw: Vec<u8> = buffer.drain(..end).collect();
        frames.push(String::from_utf8_lossy(&raw).to_string());
    }
}

/// The upstream body plus a way to start it over.
pub type Reconnect = Box<
    dyn Fn() -> futures_util::future::BoxFuture<
            'static,
            Option<BoxStream<'static, Result<Bytes, UpstreamError>>>,
        > + Send
        + Sync,
>;

/// Wrap an upstream event stream so a quiet upstream stays alive, a dead one
/// ends honestly, and one that dies before producing content is restarted
/// invisibly.
///
/// `reconnect` produces a fresh upstream stream; returning `None` means no
/// capacity is left to try.
pub fn guard(
    initial: BoxStream<'static, Result<Bytes, UpstreamError>>,
    reconnect: Reconnect,
    config: ResilienceConfig,
    dialect: Dialect,
) -> impl Stream<Item = Result<Bytes, UpstreamError>> + Send {
    async_stream::stream! {
        let mut upstream = initial;
        let mut buffer: Vec<u8> = Vec::new();
        // The message envelope, held until content arrives. Exactly one frame,
        // replaced on a restart — which is what makes the restart invisible.
        let mut envelope: Option<String> = None;
        let mut committed = false;
        let mut saw_terminal = false;
        let mut silence = Duration::ZERO;
        let mut reconnects = 0usize;

        loop {
            match tokio::time::timeout(config.keepalive, upstream.next()).await {
                // Quiet, but the connection is up. Say so.
                Err(_) => {
                    silence += config.keepalive;
                    if silence >= config.stall_timeout {
                        tracing::warn!(
                            silent_for = ?silence,
                            committed,
                            "upstream went silent; ending the stream rather than pinging into the void"
                        );
                        yield Ok(error_event(dialect, &format!(
                            "upstream produced nothing for {}s; IronWire ended the stream. \
                             Retry — the request may simply have been dropped.",
                            silence.as_secs()
                        )));
                        return;
                    }
                    yield Ok(keepalive(dialect));
                }

                Ok(Some(Ok(chunk))) => {
                    silence = Duration::ZERO;
                    if committed {
                        // Past the point of no return: forward verbatim, and
                        // still watch for the terminal frame so a clean end is
                        // told apart from a truncation.
                        buffer.extend_from_slice(&chunk);
                        for frame in take_frames(&mut buffer) {
                            if is_terminal(dialect, &frame) {
                                saw_terminal = true;
                            }
                            yield Ok(Bytes::from(frame));
                        }
                        continue;
                    }

                    buffer.extend_from_slice(&chunk);
                    for frame in take_frames(&mut buffer) {
                        match disposition(dialect, &frame) {
                            // Straight through: a held heartbeat is not a
                            // heartbeat, and holding them would also let the
                            // buffer grow without bound.
                            Disposition::ForwardAsKeepalive => yield Ok(Bytes::from(frame)),
                            Disposition::HoldAsEnvelope => envelope = Some(frame),
                            Disposition::Commits => {
                                committed = true;
                                if let Some(envelope) = envelope.take() {
                                    yield Ok(Bytes::from(envelope));
                                }
                                if is_terminal(dialect, &frame) {
                                    saw_terminal = true;
                                }
                                yield Ok(Bytes::from(frame));
                            }
                        }
                    }
                }

                // The upstream ended or failed.
                Ok(Some(Err(error))) => {
                    if committed {
                        tracing::warn!(%error, "upstream failed after committing content");
                        yield Ok(error_event(dialect, &format!(
                            "the upstream failed partway through this response ({error}). \
                             IronWire does not retry once text has reached you, because \
                             replaying would duplicate it."
                        )));
                        return;
                    }
                    match restart(&reconnect, &mut reconnects, config.max_reconnects, &error).await {
                        Some(fresh) => {
                            upstream = fresh;
                            envelope = None;
                            buffer.clear();
                            silence = Duration::ZERO;
                        }
                        None => {
                            yield Ok(error_event(dialect, &format!(
                                "no capacity could serve this request ({error})."
                            )));
                            return;
                        }
                    }
                }

                Ok(None) => {
                    if saw_terminal {
                        return;
                    }
                    if committed {
                        // Truncated. Saying so beats letting the client guess.
                        tracing::warn!("upstream closed mid-message without a terminal event");
                        yield Ok(error_event(
                            dialect,
                            "the upstream closed this response before finishing it. \
                             The text above is incomplete.",
                        ));
                        return;
                    }
                    let reason = UpstreamError::Transport {
                        backend: ironwire_core::protocol::BackendId::from("upstream"),
                        detail: "closed before producing any content".to_string(),
                    };
                    match restart(&reconnect, &mut reconnects, config.max_reconnects, &reason).await
                    {
                        Some(fresh) => {
                            upstream = fresh;
                            envelope = None;
                            buffer.clear();
                            silence = Duration::ZERO;
                        }
                        None => {
                            yield Ok(error_event(
                                dialect,
                                "the upstream closed without producing a response, and no \
                                 other capacity could serve it.",
                            ));
                            return;
                        }
                    }
                }
            }
        }
    }
}

/// Try to start the stream over. Only ever called before any content has been
/// forwarded, so the client cannot tell it happened.
async fn restart(
    reconnect: &Reconnect,
    attempts: &mut usize,
    max: usize,
    reason: &UpstreamError,
) -> Option<BoxStream<'static, Result<Bytes, UpstreamError>>> {
    if *attempts >= max {
        return None;
    }
    *attempts += 1;
    tracing::info!(
        attempt = *attempts,
        %reason,
        "stream failed before producing content; restarting invisibly"
    );
    reconnect().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    fn cfg() -> ResilienceConfig {
        ResilienceConfig {
            keepalive: Duration::from_millis(20),
            stall_timeout: Duration::from_millis(200),
            max_reconnects: 2,
        }
    }

    fn no_reconnect() -> Reconnect {
        Box::new(|| Box::pin(async { None }))
    }

    fn frame(event: &str, payload: &str) -> Bytes {
        Bytes::from(format!("event: {event}\ndata: {payload}\n\n"))
    }

    fn message_start() -> Bytes {
        frame(
            "message_start",
            r#"{"type":"message_start","message":{"model":"claude-opus-4-6"}}"#,
        )
    }

    fn text_delta(text: &str) -> Bytes {
        frame(
            "content_block_delta",
            &format!(r#"{{"type":"content_block_delta","delta":{{"text":"{text}"}}}}"#),
        )
    }

    fn message_stop() -> Bytes {
        frame("message_stop", r#"{"type":"message_stop"}"#)
    }

    async fn collect(s: impl Stream<Item = Result<Bytes, UpstreamError>>) -> String {
        let chunks: Vec<Bytes> = Box::pin(s).filter_map(|c| async { c.ok() }).collect().await;
        chunks
            .iter()
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect()
    }

    #[tokio::test]
    async fn a_healthy_stream_passes_through_unchanged() {
        let source = vec![
            Ok(message_start()),
            Ok(text_delta("hi")),
            Ok(message_stop()),
        ];
        let expected: String = source
            .iter()
            .filter_map(|c| c.as_ref().ok())
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect();
        let out = collect(guard(
            stream::iter(source).boxed(),
            no_reconnect(),
            cfg(),
            Dialect::Anthropic,
        ))
        .await;
        assert_eq!(out, expected, "a working stream must not be altered");
    }

    #[tokio::test]
    async fn silence_produces_pings_rather_than_a_client_side_stall() {
        // The fix for "Response stalled mid-stream" when the model is simply
        // thinking: the connection is alive, so say so.
        let slow = stream::once(async {
            tokio::time::sleep(Duration::from_millis(70)).await;
            Ok(message_start())
        })
        .chain(stream::iter(vec![
            Ok(text_delta("done")),
            Ok(message_stop()),
        ]));

        let out = collect(guard(
            slow.boxed(),
            no_reconnect(),
            cfg(),
            Dialect::Anthropic,
        ))
        .await;
        assert!(out.contains("event: ping"), "no keepalive emitted: {out}");
        assert!(out.contains("done"));
        assert!(out.contains("message_stop"));
    }

    #[tokio::test]
    async fn a_permanently_silent_upstream_ends_with_an_error_not_endless_pings() {
        // Pinging forever at a hung upstream is worse than a clean failure:
        // the user waits instead of retrying.
        let hung = stream::once(async {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(Bytes::new())
        });
        let out = collect(guard(
            hung.boxed(),
            no_reconnect(),
            cfg(),
            Dialect::Anthropic,
        ))
        .await;
        assert!(out.contains("event: error"), "{out}");
        assert!(out.contains("produced nothing"), "{out}");
    }

    #[tokio::test]
    async fn a_failure_during_the_thinking_gap_is_retried_invisibly() {
        // The common stall, and the one the corrected point-of-no-return
        // recovers: message_start arrived, nothing else, then the socket died.
        let first = stream::iter(vec![
            Ok(message_start()),
            Err(UpstreamError::Transport {
                backend: ironwire_core::protocol::BackendId::from("claude-sub"),
                detail: "connection reset".into(),
            }),
        ]);
        let second = || {
            Box::pin(async {
                Some(
                    stream::iter(vec![
                        Ok(message_start()),
                        Ok(text_delta("recovered")),
                        Ok(message_stop()),
                    ])
                    .boxed(),
                )
            }) as futures_util::future::BoxFuture<'static, _>
        };

        let out = collect(guard(
            first.boxed(),
            Box::new(second),
            cfg(),
            Dialect::Anthropic,
        ))
        .await;
        assert!(out.contains("recovered"));
        assert!(
            !out.contains("event: error"),
            "retry should be silent: {out}"
        );
        // Exactly one envelope — the discarded attempt's must not leak through.
        assert_eq!(out.matches("event: message_start").count(), 1, "{out}");
    }

    #[tokio::test]
    async fn content_is_never_replayed_once_it_has_reached_the_client() {
        // The rule that has not changed, and the one that matters: a retry here
        // would duplicate text already in the agent's transcript.
        let source = stream::iter(vec![
            Ok(message_start()),
            Ok(text_delta("partial answer")),
            Err(UpstreamError::Transport {
                backend: ironwire_core::protocol::BackendId::from("claude-sub"),
                detail: "reset".into(),
            }),
        ]);
        let reconnect: Reconnect = Box::new(|| {
            Box::pin(async {
                panic!("must not reconnect after content was forwarded");
            })
        });
        let out = collect(guard(source.boxed(), reconnect, cfg(), Dialect::Anthropic)).await;
        assert_eq!(out.matches("partial answer").count(), 1);
        assert!(out.contains("event: error"));
        assert!(
            out.contains("does not retry once text has reached you"),
            "{out}"
        );
    }

    #[tokio::test]
    async fn a_truncated_stream_ends_with_a_stated_error_not_a_dangling_socket() {
        // This is literally the "may be incomplete" case: the client should be
        // told, not left to infer it.
        let source = stream::iter(vec![Ok(message_start()), Ok(text_delta("half"))]);
        let out = collect(guard(
            source.boxed(),
            no_reconnect(),
            cfg(),
            Dialect::Anthropic,
        ))
        .await;
        assert!(out.contains("event: error"), "{out}");
        assert!(out.contains("incomplete"), "{out}");
    }

    #[tokio::test]
    async fn a_clean_end_adds_no_error() {
        let source = stream::iter(vec![
            Ok(message_start()),
            Ok(text_delta("x")),
            Ok(message_stop()),
        ]);
        let out = collect(guard(
            source.boxed(),
            no_reconnect(),
            cfg(),
            Dialect::Anthropic,
        ))
        .await;
        assert!(!out.contains("event: error"), "{out}");
    }

    #[tokio::test]
    async fn reconnects_are_capped_so_a_dead_provider_does_not_loop() {
        let dead = || {
            Box::pin(async {
                Some(
                    stream::iter(vec![Err(UpstreamError::Transport {
                        backend: ironwire_core::protocol::BackendId::from("x"),
                        detail: "still dead".into(),
                    })])
                    .boxed(),
                )
            }) as futures_util::future::BoxFuture<'static, _>
        };
        let source = stream::iter(vec![Err(UpstreamError::Transport {
            backend: ironwire_core::protocol::BackendId::from("x"),
            detail: "dead".into(),
        })]);
        let out = collect(guard(
            source.boxed(),
            Box::new(dead),
            cfg(),
            Dialect::Anthropic,
        ))
        .await;
        assert!(out.contains("event: error"), "{out}");
        assert!(out.contains("no capacity"), "{out}");
    }

    #[test]
    fn frames_are_dispositioned_by_what_the_client_would_have_to_un_see() {
        use Disposition::{Commits, ForwardAsKeepalive, HoldAsEnvelope};
        let cases = [
            ("event: message_start\ndata: {}\n\n", HoldAsEnvelope),
            // Held heartbeats are not heartbeats.
            ("event: ping\ndata: {}\n\n", ForwardAsKeepalive),
            ("event: content_block_start\ndata: {}\n\n", Commits),
            ("event: content_block_delta\ndata: {}\n\n", Commits),
            // Terminal frames must reach the client even with no text.
            ("event: message_delta\ndata: {}\n\n", Commits),
            ("event: message_stop\ndata: {}\n\n", Commits),
            // An unknown event is content: assuming otherwise risks replaying
            // something the client has already seen.
            ("event: something_new\ndata: {}\n\n", Commits),
            (r#"data: {"type":"ping"}"#, ForwardAsKeepalive),
            (r#"data: {"type":"message_start"}"#, HoldAsEnvelope),
        ];
        for (frame, expected) in cases {
            assert_eq!(
                disposition(Dialect::Anthropic, frame),
                expected,
                "{frame:?}"
            );
        }
    }

    #[tokio::test]
    async fn upstream_keepalives_are_forwarded_immediately_not_buffered() {
        // Delaying the provider's own heartbeat would defeat it — and holding
        // them would let the pre-commit buffer grow without bound.
        let source = stream::iter(vec![
            Ok(message_start()),
            Ok(frame("ping", r#"{"type":"ping"}"#)),
            Ok(frame("ping", r#"{"type":"ping"}"#)),
        ]);
        let mut guarded = Box::pin(guard(
            source.boxed(),
            no_reconnect(),
            cfg(),
            Dialect::Anthropic,
        ));
        let first = guarded.next().await.expect("a chunk").expect("ok");
        assert!(
            String::from_utf8_lossy(&first).contains("event: ping"),
            "the provider's keepalive was withheld"
        );
    }

    #[test]
    fn frames_are_split_on_boundaries_and_partials_are_kept() {
        let mut buffer = b"event: a\ndata: 1\n\nevent: b\ndata: 2\n\nevent: partial".to_vec();
        let frames = take_frames(&mut buffer);
        assert_eq!(frames.len(), 2);
        assert!(frames[0].contains("event: a"));
        assert_eq!(buffer, b"event: partial");
    }
}
