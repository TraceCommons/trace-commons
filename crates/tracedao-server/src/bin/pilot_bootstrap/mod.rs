//! Module root for the `tracedao-pilot-bootstrap` binary. Submodules light up
//! across the A.6 plan slices.

use std::path::PathBuf;

pub mod hf_dataset;

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

/// Slice 1 entry point — logs parsed config and exits. Slice 4 replaces this
/// with the full dataset -> translator -> submitter -> sidecar pipeline.
pub async fn run_pilot_bootstrap(config: PilotBootstrapConfig) -> anyhow::Result<()> {
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

    anyhow::bail!("pilot-bootstrap pipeline not yet wired (filled in by Slice 4)")
}
