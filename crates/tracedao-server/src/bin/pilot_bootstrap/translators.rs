//! Per-dataset translators that map an HF dataset row to a
//! [`SubmissionDraft`]. Slice 3 ships the `SwivalTranslator` plus stubs for
//! `PiMonoTranslator` and `DeepSeekAgentTranslator`; Slice 5 fills the stubs in
//! and wires auto-detection from the `--source` argument.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use super::hf_dataset::Row;

/// Maximum number of `source_code` characters folded into the swival trace
/// body. The spec calls out a 2000-char cap so the resulting envelope stays
/// well under the gate-service's per-request body size.
pub const SWIVAL_SOURCE_CODE_CAP: usize = 2000;

/// Translator-neutral hand-off to the submitter. The deterministic
/// `submission_id` is the idempotency anchor — re-running against the same
/// dataset yields the same id, so the ingest server's
/// `read_submission_record` path collapses the retry to a no-op.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionDraft {
    pub submission_id: String,
    pub trace_body: String,
    pub source_dataset: String,
    pub source_row_id: String,
    pub source_domain_tag: String,
}

/// Per-dataset translator contract. Translators are intentionally small and
/// stateless so the submitter loop can swap them on a single CLI flag.
pub trait Translator: Send + Sync {
    fn name(&self) -> &str;
    fn translate(&self, row: &Row) -> Result<SubmissionDraft>;
}

/// Compute a deterministic submission id from the trace body. The first 32 hex
/// chars (128 bits) of SHA-256 fit comfortably into a UUID v4 wire format and
/// give us idempotency without leaking the body.
pub fn submission_id_from_body(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

/// Swival = `jedisct1/agent-traces-swival`. The format is one row per security
/// audit finding with `title`, `severity`, `finding_type`, a `proof` array of
/// reasoning strings, a `fix_outline`, and a (potentially long) `source_code`
/// excerpt. We concatenate them into a body suitable for the gate-service.
pub struct SwivalTranslator;

impl SwivalTranslator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SwivalTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for SwivalTranslator {
    fn name(&self) -> &str {
        "swival"
    }

    fn translate(&self, row: &Row) -> Result<SubmissionDraft> {
        let title = row.get_str("title").unwrap_or_default();
        let severity = row.get_str("severity").unwrap_or_default();
        let finding_type = row.get_str("finding_type").unwrap_or_default();
        let proof = row.get_array_strs("proof").join("\n");
        let fix_outline = row.get_str("fix_outline").unwrap_or_default();
        let source_code = row.get_str("source_code").unwrap_or_default();

        // Char-safe truncation (parquet rows are UTF-8).
        let mut truncated = String::new();
        for ch in source_code.chars() {
            if truncated.len() + ch.len_utf8() > SWIVAL_SOURCE_CODE_CAP {
                break;
            }
            truncated.push(ch);
        }

        let body = format!(
            "{title}\n\n{severity} {finding_type}\n\n{proof}\n\n{fix_outline}\n\n{truncated}"
        );
        let id = submission_id_from_body(&body);
        Ok(SubmissionDraft {
            submission_id: id,
            trace_body: body,
            source_dataset: "jedisct1/agent-traces-swival".into(),
            source_row_id: title.to_string(),
            source_domain_tag: format!("security-audit/{finding_type}"),
        })
    }
}

/// `badlogicgames/pi-mono` and the `cfahlgren1/pi-mono-*` mirrors expose a
/// tree-structured message log per row. Slice 3 stubs this; Slice 5 implements
/// the longest-session concatenation per the spec.
pub struct PiMonoTranslator;

impl PiMonoTranslator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PiMonoTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for PiMonoTranslator {
    fn name(&self) -> &str {
        "pi-mono"
    }

    fn translate(&self, _row: &Row) -> Result<SubmissionDraft> {
        anyhow::bail!("pi-mono translator is not implemented yet (filled in by Slice 5)")
    }
}

/// `TeichAI/DeepSeek-v4-Pro-Agent` exposes message-stream rows with
/// `role`/`content` arrays. Slice 5 implements the assistant-text concatenation
/// per the spec.
pub struct DeepSeekAgentTranslator;

impl DeepSeekAgentTranslator {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DeepSeekAgentTranslator {
    fn default() -> Self {
        Self::new()
    }
}

impl Translator for DeepSeekAgentTranslator {
    fn name(&self) -> &str {
        "deepseek-agent"
    }

    fn translate(&self, _row: &Row) -> Result<SubmissionDraft> {
        anyhow::bail!("deepseek-agent translator is not implemented yet (filled in by Slice 5)")
    }
}

/// Construct a translator by short name. Slice 3 wires the swival path; Slice
/// 5 fills the stubs and adds dataset-id auto-detection.
pub fn translator_by_name(name: &str) -> Result<Box<dyn Translator>> {
    match name {
        "swival" => Ok(Box::new(SwivalTranslator::new())),
        "pi-mono" => Ok(Box::new(PiMonoTranslator::new())),
        "deepseek-agent" => Ok(Box::new(DeepSeekAgentTranslator::new())),
        other => Err(anyhow::anyhow!("unknown translator: {other}"))
            .context("translator_by_name"),
    }
}
