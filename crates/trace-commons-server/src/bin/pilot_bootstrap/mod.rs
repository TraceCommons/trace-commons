//! Module root for the `trace-commons-pilot-bootstrap` binary.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub mod hf_dataset;
pub mod sidecar;
pub mod submitter;
pub mod translators;

use hf_dataset::{SessionFile, list_jsonl_sessions, read_session_bytes};
use sidecar::{Sidecar, SidecarRecord};
use submitter::Submitter;
use translators::{Translator, passes_word_filter, translator_by_name};

/// Default word-count bounds for the session-level filter. Mirrors the
/// corpus builder defaults (`scripts/operator/build-agent-traces-corpus.py`).
pub const DEFAULT_MIN_WORDS: usize = 200;
pub const DEFAULT_MAX_WORDS: usize = 2000;

/// Parsed configuration handed to [`run_pilot_bootstrap`].
#[derive(Debug, Clone)]
pub struct PilotBootstrapConfig {
    pub source: String,
    pub translator: Option<String>,
    pub count: usize,
    pub target: String,
    pub tenant_token: String,
    pub rate: f64,
    pub sidecar: PathBuf,
    pub seed: u64,
    pub cache_dir: Option<PathBuf>,
    pub dry_run: bool,
    pub min_words: usize,
    pub max_words: usize,
}

/// Hash-only summary of a [`PilotBootstrapConfig`], for diagnostic logging.
/// Never includes raw bearer tokens or other secret material.
pub fn config_summary(config: &PilotBootstrapConfig) -> serde_json::Value {
    serde_json::json!({
        "source": config.source,
        "translator": config.translator,
        "count": config.count,
        "target": config.target,
        "rate": config.rate,
        "sidecar": config.sidecar,
        "seed": config.seed,
        "cache_dir": config.cache_dir,
        "dry_run": config.dry_run,
        "min_words": config.min_words,
        "max_words": config.max_words,
        "tenant_token_len": config.tenant_token.len(),
    })
}

/// Resolve a translator name from `--translator` or auto-detect from
/// `--source`.
pub fn resolve_translator(config: &PilotBootstrapConfig) -> Result<Box<dyn Translator>> {
    if let Some(name) = config.translator.as_ref() {
        return translator_by_name(name);
    }
    let detected = auto_detect_translator(&config.source).ok_or_else(|| {
        anyhow::anyhow!(
            "could not auto-detect translator for source dataset {}; pass --translator",
            config.source
        )
    })?;
    translator_by_name(detected)
}

/// Map a dataset id to a translator short name. Returns `None` for unknown
/// sources so the caller can require an explicit `--translator`.
pub fn auto_detect_translator(source: &str) -> Option<&'static str> {
    let lower = source.to_ascii_lowercase();
    if lower.starts_with("jedisct1/agent-traces-swival") {
        Some("swival")
    } else if lower.starts_with("badlogicgames/pi-mono") || lower.starts_with("cfahlgren1/pi-mono")
    {
        Some("pi-mono")
    } else if lower.starts_with("teichai/deepseek-v4-pro-agent") {
        Some("deepseek-agent")
    } else {
        None
    }
}

#[derive(Debug, Default, Clone)]
pub struct RunSummary {
    pub total: usize,
    pub accepted: usize,
    pub quarantined: usize,
    pub rejected: usize,
    pub duplicate: usize,
    pub errors: usize,
    pub filtered_out: usize,
}

/// Pipeline entry point: dataset -> translator -> submitter -> sidecar loop.
pub async fn run_pilot_bootstrap(config: PilotBootstrapConfig) -> Result<()> {
    tracing::info!(
        target: "trace_commons_pilot_bootstrap",
        config = %config_summary(&config),
        "pilot-bootstrap starting"
    );

    if config.dry_run {
        tracing::info!(
            target: "trace_commons_pilot_bootstrap",
            "dry-run requested; exiting before any submissions"
        );
        return Ok(());
    }

    if config.min_words > config.max_words {
        anyhow::bail!(
            "min_words ({}) must be <= max_words ({})",
            config.min_words,
            config.max_words
        );
    }

    let translator = resolve_translator(&config)?;
    tracing::info!(
        target: "trace_commons_pilot_bootstrap",
        translator = translator.name(),
        "translator resolved"
    );

    let sessions = list_jsonl_sessions(&config.source, config.cache_dir.as_deref()).await?;
    tracing::info!(
        target: "trace_commons_pilot_bootstrap",
        session_count = sessions.len(),
        "jsonl sessions resolved"
    );

    let submitter = Submitter::new(
        config.target.clone(),
        config.tenant_token.clone(),
        config.rate,
    )?;
    let sidecar = Sidecar::open(&config.sidecar)?;

    let summary = drive_loop(
        &config,
        translator.as_ref(),
        &sessions,
        &submitter,
        &sidecar,
    )
    .await
    .context("submission loop")?;

    tracing::info!(
        target: "trace_commons_pilot_bootstrap",
        summary = ?summary,
        "pilot-bootstrap finished"
    );
    Ok(())
}

async fn drive_loop(
    config: &PilotBootstrapConfig,
    translator: &dyn Translator,
    sessions: &[SessionFile],
    submitter: &Submitter,
    sidecar: &Sidecar,
) -> Result<RunSummary> {
    let mut summary = RunSummary::default();
    for session in sessions {
        if summary.total >= config.count {
            break;
        }
        let bytes = match read_session_bytes(&session.local_path) {
            Ok(b) => b,
            Err(err) => {
                let sibling_hash = label_sha256_12(&session.sibling_name);
                tracing::warn!(
                    target: "trace_commons_pilot_bootstrap",
                    error = %err,
                    sibling_hash = %sibling_hash,
                    "session read failed; skipping"
                );
                summary.errors += 1;
                continue;
            }
        };
        let draft = match translator.translate(&session.sibling_name, &bytes) {
            Ok(d) => d,
            Err(err) => {
                let sibling_hash = label_sha256_12(&session.sibling_name);
                tracing::warn!(
                    target: "trace_commons_pilot_bootstrap",
                    error = %err,
                    sibling_hash = %sibling_hash,
                    "translator skipped session"
                );
                summary.filtered_out += 1;
                continue;
            }
        };
        if !passes_word_filter(&draft.trace_body, config.min_words, config.max_words) {
            tracing::debug!(
                target: "trace_commons_pilot_bootstrap",
                sibling_hash = %label_sha256_12(&session.sibling_name),
                submission_id = %draft.submission_id,
                "session outside word-count bounds; skipping"
            );
            summary.filtered_out += 1;
            continue;
        }
        let outcome = submitter.submit(&draft).await?;
        let record = SidecarRecord {
            submission_id: draft.submission_id.clone(),
            source_dataset: draft.source_dataset.clone(),
            source_row_id: draft.source_row_id.clone(),
            source_domain_tag: draft.source_domain_tag.clone(),
            http_status: outcome.http_status,
            gate_decision: outcome.gate_decision.clone(),
            elapsed_ms: outcome.elapsed_ms,
            timestamp: chrono::Utc::now(),
        };
        sidecar.write(&record)?;
        tracing::info!(
            target: "trace_commons_pilot_bootstrap",
            sibling_hash = %label_sha256_12(&session.sibling_name),
            submission_id = %draft.submission_id,
            decision = %outcome.gate_decision,
            "session submitted"
        );
        match outcome.gate_decision.as_str() {
            "accepted" => summary.accepted += 1,
            "quarantined" => summary.quarantined += 1,
            "rejected" => summary.rejected += 1,
            "duplicate" => summary.duplicate += 1,
            _ => summary.errors += 1,
        }
        summary.total += 1;
    }
    Ok(summary)
}

/// Short hash label for hash-only logging. Used to identify sessions in
/// operator logs without echoing file paths or contributor identity.
fn label_sha256_12(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    let digest = h.finalize();
    format!("sha256:{}", &hex::encode(digest)[..12])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_detects_known_sources() {
        assert_eq!(
            auto_detect_translator("jedisct1/agent-traces-swival"),
            Some("swival")
        );
        assert_eq!(
            auto_detect_translator("badlogicgames/pi-mono"),
            Some("pi-mono")
        );
        assert_eq!(
            auto_detect_translator("cfahlgren1/pi-mono-traces"),
            Some("pi-mono")
        );
        assert_eq!(
            auto_detect_translator("TeichAI/DeepSeek-v4-Pro-Agent"),
            Some("deepseek-agent")
        );
        assert_eq!(auto_detect_translator("unknown/dataset"), None);
    }

    #[test]
    fn label_sha256_12_is_stable_and_short() {
        let a = label_sha256_12("session-1.jsonl");
        let b = label_sha256_12("session-1.jsonl");
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
        assert_eq!(a.len(), "sha256:".len() + 12);
    }
}
