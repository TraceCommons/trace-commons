//! Rate-limited HTTP submitter. Wraps a [`SubmissionDraft`] into a full
//! `TraceContributionEnvelope` via the protocol crate's redactor and POSTs it
//! to the running `trace-commons-ingest` binary's `/v1/traces` endpoint.
//!
//! Idempotency is server-side: the deterministic submission id makes
//! re-running the harness against the same dataset collapse to no-op
//! submissions at the ingest server (it reads the existing record and
//! returns its receipt). We therefore do not gate POSTs on a local seen-list;
//! the spec's "rerunning the same dataset doesn't double-submit" property
//! comes from the deterministic id, not from extra round-trips.

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use tokio::time::Instant;
use trace_commons_protocol::llm::recording::{
    ExpectedToolResult, TraceFile, TraceResponse, TraceStep, TraceToolCall,
};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceContribution, RecordedTraceContributionOptions,
    TraceContributionEnvelope, TraceRedactor,
};
use uuid::Uuid;

use super::translators::{SessionEvent, SessionEventRole, SessionEventTool, SubmissionDraft};

/// Outcome of a single submission attempt. Hash-only fields, no body content.
#[derive(Debug, Clone)]
pub struct SubmissionOutcome {
    // Carried for Debug/tracing parity with the sidecar record; the main loop
    // reads `draft.submission_id` directly, so this field is currently unread.
    #[allow(dead_code)]
    pub submission_id: String,
    pub http_status: Option<u16>,
    pub gate_decision: String,
    pub elapsed_ms: u64,
}

/// HTTP submitter. Sleeps `1/rate` seconds between attempts; retries 5xx with
/// exponential backoff up to `max_retries`; treats 4xx as terminal.
pub struct Submitter {
    client: Client,
    target: String,
    tenant_token: String,
    min_interval: Duration,
    max_retries: u32,
    last_send: tokio::sync::Mutex<Option<Instant>>,
}

#[derive(Debug, Deserialize)]
struct ReceiptWire {
    status: String,
}

impl Submitter {
    pub fn new(target: String, tenant_token: String, rate_per_sec: f64) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("build reqwest client")?;
        let min_interval = if rate_per_sec > 0.0 {
            Duration::from_secs_f64(1.0 / rate_per_sec)
        } else {
            Duration::ZERO
        };
        Ok(Self {
            client,
            target: target.trim_end_matches('/').to_string(),
            tenant_token,
            min_interval,
            max_retries: 3,
            last_send: tokio::sync::Mutex::new(None),
        })
    }

    async fn wait_for_slot(&self) {
        if self.min_interval.is_zero() {
            return;
        }
        let mut guard = self.last_send.lock().await;
        if let Some(prev) = *guard {
            let elapsed = prev.elapsed();
            if elapsed < self.min_interval {
                tokio::time::sleep(self.min_interval - elapsed).await;
            }
        }
        *guard = Some(Instant::now());
    }

    pub async fn submit(&self, draft: &SubmissionDraft) -> Result<SubmissionOutcome> {
        let envelope = build_envelope_from_draft(draft).await?;
        let url = format!("{}/v1/traces", self.target);
        let started = Instant::now();
        let mut attempt = 0u32;
        loop {
            self.wait_for_slot().await;
            let response = self
                .client
                .post(&url)
                .bearer_auth(&self.tenant_token)
                .json(&envelope)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let receipt: ReceiptWire = resp.json().await.unwrap_or(ReceiptWire {
                            status: "accepted".into(),
                        });
                        return Ok(SubmissionOutcome {
                            submission_id: draft.submission_id.clone(),
                            http_status: Some(status.as_u16()),
                            gate_decision: receipt.status,
                            elapsed_ms: started.elapsed().as_millis() as u64,
                        });
                    }
                    if status.is_server_error() && attempt < self.max_retries {
                        let backoff = Duration::from_millis(250u64 << attempt);
                        tracing::warn!(
                            target: "trace_commons_pilot_bootstrap",
                            attempt,
                            backoff_ms = backoff.as_millis() as u64,
                            http_status = status.as_u16(),
                            "submission failed; retrying"
                        );
                        attempt += 1;
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Ok(SubmissionOutcome {
                        submission_id: draft.submission_id.clone(),
                        http_status: Some(status.as_u16()),
                        gate_decision: if status == StatusCode::CONFLICT {
                            "duplicate".into()
                        } else if status.is_client_error() {
                            "rejected".into()
                        } else {
                            "error".into()
                        },
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    });
                }
                Err(err) => {
                    if attempt < self.max_retries {
                        let backoff = Duration::from_millis(250u64 << attempt);
                        tracing::warn!(
                            target: "trace_commons_pilot_bootstrap",
                            attempt,
                            backoff_ms = backoff.as_millis() as u64,
                            error = %err,
                            "submission transport error; retrying"
                        );
                        attempt += 1;
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                    return Ok(SubmissionOutcome {
                        submission_id: draft.submission_id.clone(),
                        http_status: None,
                        gate_decision: "error".into(),
                        elapsed_ms: started.elapsed().as_millis() as u64,
                    });
                }
            }
        }
    }
}

/// Convert the draft id (hex prefix of SHA-256) into a deterministic
/// [`Uuid`]. The id is 32 hex chars = 16 bytes; `Uuid::from_bytes` consumes
/// exactly that. Any deviation is a programmer error in the translator.
fn submission_uuid(draft_id: &str) -> Result<Uuid> {
    let bytes =
        hex::decode(draft_id).with_context(|| format!("decode draft submission id {draft_id}"))?;
    let arr: [u8; 16] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("draft submission id must be 16 bytes / 32 hex chars"))?;
    Ok(Uuid::from_bytes(arr))
}

/// Build a [`TraceContributionEnvelope`] from a translator draft. We
/// construct a synthetic multi-step `TraceFile`, one step per session
/// event, run it through the deterministic redactor (the same pipeline real
/// clients use), then override the redactor-generated random ids with our
/// deterministic content-derived ones so reruns are idempotent.
pub async fn build_envelope_from_draft(
    draft: &SubmissionDraft,
) -> Result<TraceContributionEnvelope> {
    let steps: Vec<TraceStep> = draft.session_events.iter().flat_map(steps_for).collect();
    let trace = TraceFile {
        model_name: format!("pilot-bootstrap/{}", draft.source_dataset),
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps,
    };
    let options = RecordedTraceContributionOptions {
        include_message_text: true,
        // Tool arguments and tool-result content are the point of mapping
        // tool records at all -- withheld, a `ToolCall` event says only that
        // some tool ran. The consent flag this sets on the envelope is
        // descriptive: it follows the payload, and the payload is here.
        include_tool_payloads: true,
        pseudonymous_contributor_id: Some(format!(
            "sha256:pilot-bootstrap/{}",
            draft.source_dataset
        )),
        ..Default::default()
    };
    let raw = RawTraceContribution::from_recorded_trace(&trace, options);
    let mut envelope = DeterministicTraceRedactor::try_default()
        .map_err(|err| anyhow::anyhow!("privacy filter config invalid: {err}"))?
        .redact_trace(raw)
        .await
        .context("redact pilot-bootstrap trace")?;
    let submission_uuid = submission_uuid(&draft.submission_id)?;
    envelope.submission_id = submission_uuid;
    // Trace id derived from the same bytes (so repeats hit the same record).
    envelope.trace_id = submission_uuid;
    Ok(envelope)
}

/// Turn one session event into the trace steps it needs.
///
/// Most records map to one step. A record that carried prose *and* tool
/// calls -- the dominant assistant shape in pi-mono and DeepSeek -- maps to
/// two, both stamped with that record's own real time, because a `TraceStep`
/// holds exactly one response and neither half may be dropped.
///
/// A tool result rides a step whose response is an empty `ToolCalls`: the
/// result itself lives in `expected_tool_results`, and an empty call list
/// emits no event of its own, so the result keeps its own real timestamp
/// instead of borrowing the timestamp of whichever step it was attached to.
/// `from_recorded_trace` pairs it back to its call by `tool_call_id`, not by
/// position, so the pairing survives the result being its own step.
fn steps_for(event: &SessionEvent) -> Vec<TraceStep> {
    let mut steps = Vec::new();

    // A pi-mono/DeepSeek tool-result record's extracted text *is* its result
    // content. Emitting it as prose too would put the same bytes in the
    // envelope twice.
    let text_is_the_result = matches!(
        &event.tool,
        Some(SessionEventTool::Results(results))
            if results.iter().any(|r| r.content == event.text)
    );

    if !event.text.is_empty() && !text_is_the_result {
        steps.push(TraceStep {
            request_hint: None,
            response: match event.role {
                SessionEventRole::User | SessionEventRole::Other => TraceResponse::UserInput {
                    content: event.text.clone(),
                },
                SessionEventRole::Assistant => TraceResponse::Text {
                    content: event.text.clone(),
                    // These corpus-building datasets do not carry real
                    // per-event token counts. 0 is an explicit "not
                    // measured", not a fabricated value -- unlike
                    // `timestamp`, `TraceResponse::Text` has no absent-value
                    // representation for this field.
                    input_tokens: 0,
                    output_tokens: 0,
                },
            },
            expected_tool_results: Vec::new(),
            timestamp: event.timestamp,
        });
    }

    match &event.tool {
        Some(SessionEventTool::Calls(calls)) => steps.push(TraceStep {
            request_hint: None,
            response: TraceResponse::ToolCalls {
                tool_calls: calls
                    .iter()
                    .map(|call| TraceToolCall {
                        id: call.id.clone(),
                        name: call.name.clone(),
                        arguments: call.arguments.clone(),
                    })
                    .collect(),
                input_tokens: 0,
                output_tokens: 0,
            },
            expected_tool_results: Vec::new(),
            timestamp: event.timestamp,
        }),
        Some(SessionEventTool::Results(results)) => steps.push(TraceStep {
            request_hint: None,
            response: TraceResponse::ToolCalls {
                tool_calls: Vec::new(),
                input_tokens: 0,
                output_tokens: 0,
            },
            expected_tool_results: results
                .iter()
                .map(|result| ExpectedToolResult {
                    tool_call_id: result.tool_call_id.clone(),
                    name: result.name.clone(),
                    content: result.content.clone(),
                })
                .collect(),
            timestamp: event.timestamp,
        }),
        None => {}
    }

    steps
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn build_envelope_is_deterministic_for_same_draft() {
        let draft = SubmissionDraft {
            submission_id: "0123456789abcdef0123456789abcdef".into(),
            trace_body: "hello world".into(),
            source_dataset: "test/dataset".into(),
            source_row_id: "row-1".into(),
            source_domain_tag: "test".into(),
            session_events: vec![super::super::translators::SessionEvent {
                text: "hello world".into(),
                timestamp: None,
                role: SessionEventRole::User,
                tool: None,
            }],
        };
        let a = build_envelope_from_draft(&draft).await.unwrap();
        let b = build_envelope_from_draft(&draft).await.unwrap();
        assert_eq!(a.submission_id, b.submission_id);
    }

    /// End to end: a session's tool records reach the envelope as real tool
    /// events -- name, arguments, result content -- with the result pointing
    /// at the call it answers by id rather than by position.
    #[tokio::test]
    async fn tool_events_reach_the_envelope_as_tool_calls_and_results() {
        use chrono::{TimeZone, Utc};
        use trace_commons_protocol::trace_contribution::TraceContributionEventType;

        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 7).unwrap();
        let draft = SubmissionDraft {
            submission_id: "00112233445566778899aabbccddeeff".into(),
            trace_body: "let me look".into(),
            source_dataset: "test/dataset".into(),
            source_row_id: "row-3".into(),
            source_domain_tag: "test".into(),
            session_events: vec![
                SessionEvent {
                    text: "let me look".into(),
                    timestamp: Some(t0),
                    role: SessionEventRole::Assistant,
                    tool: Some(SessionEventTool::Calls(vec![
                        super::super::translators::SessionToolCall {
                            id: "call_1".into(),
                            name: "bash".into(),
                            arguments: serde_json::json!({"command": "ls"}),
                        },
                    ])),
                },
                SessionEvent {
                    text: "a.txt".into(),
                    timestamp: Some(t1),
                    role: SessionEventRole::Other,
                    tool: Some(SessionEventTool::Results(vec![
                        super::super::translators::SessionToolResult {
                            tool_call_id: "call_1".into(),
                            name: "bash".into(),
                            content: "a.txt".into(),
                        },
                    ])),
                },
            ],
        };
        let envelope = build_envelope_from_draft(&draft).await.unwrap();

        let call = envelope
            .events
            .iter()
            .find(|e| e.event_type == TraceContributionEventType::ToolCall)
            .expect("the tool call must reach the envelope as a tool call");
        assert_eq!(call.tool_name.as_deref(), Some("bash"));
        assert_eq!(call.tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(call.timestamp, t0);
        assert_eq!(
            call.structured_payload["arguments"]["command"],
            serde_json::json!("ls"),
            "arguments must survive, not just the fact that some tool ran"
        );

        let result = envelope
            .events
            .iter()
            .find(|e| e.event_type == TraceContributionEventType::ToolResult)
            .expect("the tool result must reach the envelope as a tool result");
        assert_eq!(result.tool_name.as_deref(), Some("bash"));
        assert_eq!(result.timestamp, t1, "the result keeps its own real time");
        assert_eq!(
            result.parent_event_id,
            Some(call.event_id),
            "the result must be paired to its call by id"
        );

        // The result's text is its content; it must not also appear as prose.
        assert_eq!(
            envelope
                .events
                .iter()
                .filter(|e| e.event_type == TraceContributionEventType::UserMessage)
                .count(),
            0
        );
        assert!(envelope.consent.tool_payloads_included);
    }

    #[tokio::test]
    async fn a_multi_event_draft_produces_a_multi_event_envelope_with_distinct_timestamps() {
        // The end-to-end point of the restructure: a session that used to
        // collapse into one flattened `UserInput` step now reaches the
        // envelope as multiple events, each keeping its own real time.
        use chrono::{TimeZone, Utc};
        let t0 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
        let t1 = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 5).unwrap();
        let draft = SubmissionDraft {
            submission_id: "fedcba9876543210fedcba9876543210".into(),
            trace_body: "first turn\n\nfirst reply".into(),
            source_dataset: "test/dataset".into(),
            source_row_id: "row-2".into(),
            source_domain_tag: "test".into(),
            session_events: vec![
                super::super::translators::SessionEvent {
                    text: "first turn".into(),
                    timestamp: Some(t0),
                    role: SessionEventRole::User,
                    tool: None,
                },
                super::super::translators::SessionEvent {
                    text: "first reply".into(),
                    timestamp: Some(t1),
                    role: SessionEventRole::Assistant,
                    tool: None,
                },
            ],
        };
        let envelope = build_envelope_from_draft(&draft).await.unwrap();
        assert_eq!(envelope.events.len(), 2);
        let stamps: std::collections::BTreeSet<_> =
            envelope.events.iter().map(|e| e.timestamp).collect();
        assert_eq!(stamps.len(), 2, "each event must keep its own timestamp");
        assert!(stamps.contains(&t0));
        assert!(stamps.contains(&t1));
    }
}
