//! Rate-limited HTTP submitter. Wraps a [`SubmissionDraft`] into a full
//! `TraceContributionEnvelope` via the protocol crate's redactor and POSTs it
//! to the running `tracedao-ingest` binary's `/v1/traces` endpoint.
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
use tracedao_protocol::llm::recording::{TraceFile, TraceResponse, TraceStep};
use tracedao_protocol::trace_contribution::{
    DeterministicTraceRedactor, RawTraceContribution, RecordedTraceContributionOptions,
    TraceContributionEnvelope, TraceRedactor,
};
use uuid::Uuid;

use super::translators::SubmissionDraft;

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
                            target: "tracedao_pilot_bootstrap",
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
                            target: "tracedao_pilot_bootstrap",
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

/// Build a [`TraceContributionEnvelope`] from a translator draft. We construct
/// a synthetic single-turn `TraceFile` containing the body as a `UserInput`
/// step, run it through the deterministic redactor (the same pipeline real
/// clients use), then override the redactor-generated random ids with our
/// deterministic content-derived ones so reruns are idempotent.
pub async fn build_envelope_from_draft(
    draft: &SubmissionDraft,
) -> Result<TraceContributionEnvelope> {
    let trace = TraceFile {
        model_name: format!("pilot-bootstrap/{}", draft.source_dataset),
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps: vec![TraceStep {
            request_hint: None,
            response: TraceResponse::UserInput {
                content: draft.trace_body.clone(),
            },
            expected_tool_results: Vec::new(),
        }],
    };
    let options = RecordedTraceContributionOptions {
        include_message_text: true,
        pseudonymous_contributor_id: Some(format!(
            "sha256:pilot-bootstrap/{}",
            draft.source_dataset
        )),
        ..Default::default()
    };
    let raw = RawTraceContribution::from_recorded_trace(&trace, options);
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .context("redact pilot-bootstrap trace")?;
    let submission_uuid = submission_uuid(&draft.submission_id)?;
    envelope.submission_id = submission_uuid;
    // Trace id derived from the same bytes (so repeats hit the same record).
    envelope.trace_id = submission_uuid;
    Ok(envelope)
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
        };
        let a = build_envelope_from_draft(&draft).await.unwrap();
        let b = build_envelope_from_draft(&draft).await.unwrap();
        assert_eq!(a.submission_id, b.submission_id);
    }
}
