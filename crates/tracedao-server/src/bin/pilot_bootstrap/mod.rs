//! Module root for the `tracedao-pilot-bootstrap` binary. Submodules light up
//! across the A.6 plan slices.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub mod hf_dataset;
pub mod sidecar;
pub mod submitter;
pub mod translators;

use hf_dataset::{list_parquet_shards, stream_parquet_rows};
use sidecar::{Sidecar, SidecarRecord};
use submitter::Submitter;
use translators::{translator_by_name, Translator};

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
        "tenant_token_len": config.tenant_token.len(),
    })
}

/// Resolve a translator name from `--translator` or auto-detect from
/// `--source`. Slice 5 fills in pi-mono + deepseek-agent auto-detection
/// alongside the swival default.
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
}

/// Pipeline entry point. Slice 4 wires the full dataset -> translator ->
/// submitter -> sidecar loop.
pub async fn run_pilot_bootstrap(config: PilotBootstrapConfig) -> Result<()> {
    tracing::info!(
        target: "tracedao_pilot_bootstrap",
        config = %config_summary(&config),
        "pilot-bootstrap starting"
    );

    if config.dry_run {
        tracing::info!(
            target: "tracedao_pilot_bootstrap",
            "dry-run requested; exiting before any submissions"
        );
        return Ok(());
    }

    let translator = resolve_translator(&config)?;
    tracing::info!(
        target: "tracedao_pilot_bootstrap",
        translator = translator.name(),
        "translator resolved"
    );

    let shards =
        list_parquet_shards(&config.source, config.cache_dir.as_deref()).await?;
    tracing::info!(
        target: "tracedao_pilot_bootstrap",
        shard_count = shards.len(),
        "parquet shards resolved"
    );

    let submitter = Submitter::new(
        config.target.clone(),
        config.tenant_token.clone(),
        config.rate,
    )?;
    let sidecar = Sidecar::open(&config.sidecar)?;

    let summary = drive_loop(&config, translator.as_ref(), &shards, &submitter, &sidecar)
        .await
        .context("submission loop")?;

    tracing::info!(
        target: "tracedao_pilot_bootstrap",
        summary = ?summary,
        "pilot-bootstrap finished"
    );
    Ok(())
}

async fn drive_loop(
    config: &PilotBootstrapConfig,
    translator: &dyn Translator,
    shards: &[PathBuf],
    submitter: &Submitter,
    sidecar: &Sidecar,
) -> Result<RunSummary> {
    let mut summary = RunSummary::default();
    'outer: for shard in shards {
        let iter = stream_parquet_rows(shard)
            .with_context(|| format!("open shard {}", shard.display()))?;
        for row_res in iter {
            if summary.total >= config.count {
                break 'outer;
            }
            let row = match row_res {
                Ok(r) => r,
                Err(err) => {
                    tracing::warn!(
                        target: "tracedao_pilot_bootstrap",
                        error = %err,
                        "parquet row decode failed; skipping"
                    );
                    summary.errors += 1;
                    summary.total += 1;
                    continue;
                }
            };
            let draft = match translator.translate(&row) {
                Ok(d) => d,
                Err(err) => {
                    tracing::warn!(
                        target: "tracedao_pilot_bootstrap",
                        error = %err,
                        "translator skipped row"
                    );
                    summary.errors += 1;
                    summary.total += 1;
                    continue;
                }
            };
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
            match outcome.gate_decision.as_str() {
                "accepted" => summary.accepted += 1,
                "quarantined" => summary.quarantined += 1,
                "rejected" => summary.rejected += 1,
                "duplicate" => summary.duplicate += 1,
                _ => summary.errors += 1,
            }
            summary.total += 1;
        }
    }
    Ok(summary)
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
}
