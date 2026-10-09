// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the production pipeline assembly reads from the environment, and,
//! under `near-ai-scorer`, its one constructor: [`PipelineGateComponents::from_env`]
//! (PR #1295 review round 2, Major 1). The ingest binary and any integration
//! target build the NEAR AI scorer, the fastembed embedder and the usearch
//! pipeline index through it, so there is a single place that constructs
//! them and checks the descriptors against them. The variable names the
//! legacy `enclave_near_ai` gate shares are defined here; the ingest binary
//! imports them.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;

use super::{
    DbTenantPolicyReads, FastEmbedDescriptor, NearAiScorerDescriptor, validate_pipeline_index_root,
};
use crate::trace_authority::SubmissionAllowlists;

pub const TRACE_COMMONS_GATE_SERVICE: &str = "TRACE_COMMONS_GATE_SERVICE";

pub const TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF: &str =
    "TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF";
pub const TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF: f32 = -8.0;

// NEAR AI Cloud-backed perplexity scorer (pilot deployment path). Read only when
// `TRACE_COMMONS_GATE_SERVICE=enclave_near_ai` AND the `near-ai-scorer` cargo
// feature is compiled in. API key intentionally only read from env (never CLI)
// so it never appears in process listings.
pub const TRACE_COMMONS_NEAR_AI_BASE_URL: &str = "TRACE_COMMONS_NEAR_AI_BASE_URL";
pub const TRACE_COMMONS_NEAR_AI_MODEL: &str = "TRACE_COMMONS_NEAR_AI_MODEL";
pub const TRACE_COMMONS_NEAR_AI_API_KEY: &str = "TRACE_COMMONS_NEAR_AI_API_KEY";
pub const TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS: &str = "TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS";
pub const TRACE_COMMONS_NEAR_AI_DEFAULT_TIMEOUT_SECONDS: u64 = 60;
// Chunked scoring sends one bounded request per chunk; perplexity needs
// only the realized token's logprob, so k=1 cuts TEE backend memory and
// response size ~5x vs the OpenAI-canonical 5 (large-trace OOM root cause).
pub const TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K: u32 = 1;

// fastembed embedder (Phase A3), shared by the `enclave_local_gpu` and
// `enclave_near_ai` gates.
pub const TRACE_COMMONS_EMBEDDER_MODEL_ID: &str = "TRACE_COMMONS_EMBEDDER_MODEL_ID";
pub const TRACE_COMMONS_EMBEDDER_CACHE_DIR: &str = "TRACE_COMMONS_EMBEDDER_CACHE_DIR";
pub const TRACE_COMMONS_EMBEDDER_MAX_TOKENS: &str = "TRACE_COMMONS_EMBEDDER_MAX_TOKENS";
pub const TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM: &str = "TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM";
pub const TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID: &str = "BAAI/bge-large-en-v1.5";
pub const TRACE_COMMONS_EMBEDDER_DEFAULT_CACHE_DIR: &str = "/var/cache/trace-commons-embedder";
pub const TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS: usize = 512;

// usearch-backed vector index (Phase A4): the legacy novelty index, and the
// HNSW parameters the pipeline index shares with it.
pub const TRACE_COMMONS_VECTOR_INDEX_ROOT: &str = "TRACE_COMMONS_VECTOR_INDEX_ROOT";
pub const TRACE_COMMONS_VECTOR_INDEX_DIM: &str = "TRACE_COMMONS_VECTOR_INDEX_DIM";
pub const TRACE_COMMONS_VECTOR_INDEX_MAX_OPEN: &str = "TRACE_COMMONS_VECTOR_INDEX_MAX_OPEN";
pub const TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY: &str = "TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY";
pub const TRACE_COMMONS_VECTOR_INDEX_HNSW_M: &str = "TRACE_COMMONS_VECTOR_INDEX_HNSW_M";
pub const TRACE_COMMONS_VECTOR_INDEX_EF_CONSTRUCTION: &str =
    "TRACE_COMMONS_VECTOR_INDEX_EF_CONSTRUCTION";
pub const TRACE_COMMONS_VECTOR_INDEX_EF_SEARCH: &str = "TRACE_COMMONS_VECTOR_INDEX_EF_SEARCH";
pub const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_ROOT: &str = "/var/lib/trace-commons-vector-index";
pub const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_DIM: usize = 1024;
pub const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_MAX_OPEN: usize = 32;
pub const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_FLUSH_EVERY: usize = 32;
pub const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_HNSW_M: usize = 16;
pub const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_EF_CONSTRUCTION: usize = 200;
pub const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_EF_SEARCH: usize = 50;
// Cross-trace dedup (shadow-only): a SEPARATE `UsearchVectorIndex` instance
// from the novelty index above, so dedup lookups never pollute novelty's
// nearest-neighbor results. Same dim/hnsw/ef params as the novelty index
// (`TRACE_COMMONS_VECTOR_INDEX_*`); only the root path is independently
// configurable.
pub const TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT: &str = "TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT";

/// Where the pipeline's own vector index lives (spec A-D6). Required under
/// the production selection, and never the legacy novelty or dedup root.
pub const TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT: &str =
    "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT";
pub const PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL: &str = "pipeline_vector_index_root_missing";
pub const PIPELINE_SCORER_DESCRIPTOR_MISMATCH_LABEL: &str = "pipeline_scorer_descriptor_mismatch";
pub const PIPELINE_EMBEDDER_DESCRIPTOR_MISMATCH_LABEL: &str =
    "pipeline_embedder_descriptor_mismatch";

/// Parse `T = usize` from an env var with a default fallback. Trim + strict
/// integer parse; empty / unset → default; malformed → fail-closed.
///
/// Not feature-gated (unlike its sibling gate-config parsers): the chunk-knob
/// parser that reuses this reads env unconditionally so its defaults are
/// exercised by the plain `cargo test` CI path regardless of which optional
/// gate-service feature (if any) is compiled in.
pub fn parse_usize_env(var: &'static str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(var) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Ok(default)
            } else {
                trimmed
                    .parse::<usize>()
                    .with_context(|| format!("{var} must be a non-negative integer"))
            }
        }
        Err(_) => Ok(default),
    }
}

/// Reads one variable through `lookup`, treating an empty or all-blank value
/// as unset.
fn lookup_trimmed(lookup: &dyn Fn(&str) -> Option<String>, var: &str) -> Option<String> {
    lookup(var)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// The process environment as a `lookup`.
pub fn std_env_lookup(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

/// The NEAR AI scorer descriptor (spec A-D4) from `lookup`: the same parse
/// the legacy `enclave_near_ai` gate applies to the model and the tail
/// cutoff; `logprobs_top_k` is the constant both use.
pub fn near_ai_scorer_descriptor_from_lookup(
    lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<NearAiScorerDescriptor> {
    let model = lookup_trimmed(lookup, TRACE_COMMONS_NEAR_AI_MODEL)
        .ok_or_else(|| anyhow::anyhow!("pipeline_scorer_model_missing"))?;
    let tail_logprob_cutoff =
        match lookup_trimmed(lookup, TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF) {
            Some(raw) => raw
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| anyhow::anyhow!("pipeline_scorer_tail_cutoff_invalid"))?,
            None => TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF,
        };
    Ok(NearAiScorerDescriptor {
        model,
        tail_logprob_cutoff,
        logprobs_top_k: TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K,
    })
}

/// The fastembed embedder descriptor (spec A-D5) from `lookup`.
pub fn fastembed_descriptor_from_lookup(
    lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<FastEmbedDescriptor> {
    let positive = |var: &str| -> anyhow::Result<Option<usize>> {
        lookup_trimmed(lookup, var)
            .map(|raw| {
                raw.parse::<usize>()
                    .ok()
                    .filter(|value| *value > 0)
                    .ok_or_else(|| anyhow::anyhow!("pipeline_embedder_descriptor_invalid"))
            })
            .transpose()
    };
    Ok(FastEmbedDescriptor {
        model_id: lookup_trimmed(lookup, TRACE_COMMONS_EMBEDDER_MODEL_ID)
            .unwrap_or_else(|| TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID.to_string()),
        output_dim: positive(TRACE_COMMONS_VECTOR_INDEX_DIM)?
            .unwrap_or(TRACE_COMMONS_VECTOR_INDEX_DEFAULT_DIM),
        max_tokens: positive(TRACE_COMMONS_EMBEDDER_MAX_TOKENS)?
            .unwrap_or(TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS),
        matryoshka_dim: positive(TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM)?,
    })
}

/// The values the legacy gate was built with, which the pipeline's
/// descriptors must name: the two parse the same variables, and this
/// refuses a start where they would not agree, so the package can never
/// name a scorer or embedder other than the one that runs.
#[derive(Debug, Clone, PartialEq)]
pub struct LegacyGatePins {
    pub model: String,
    pub tail_logprob_cutoff: f32,
    pub embedder_model_id: String,
    pub embedder_output_dim: usize,
    pub embedder_max_tokens: usize,
    pub embedder_matryoshka_dim: Option<usize>,
}

pub fn ensure_descriptors_match_gate(
    scorer: &NearAiScorerDescriptor,
    embedder: &FastEmbedDescriptor,
    gate: &LegacyGatePins,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        scorer.model == gate.model
            && scorer.tail_logprob_cutoff == gate.tail_logprob_cutoff
            && scorer.logprobs_top_k == TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K,
        PIPELINE_SCORER_DESCRIPTOR_MISMATCH_LABEL
    );
    anyhow::ensure!(
        embedder.model_id == gate.embedder_model_id
            && embedder.output_dim == gate.embedder_output_dim
            && embedder.max_tokens == gate.embedder_max_tokens
            && embedder.matryoshka_dim == gate.embedder_matryoshka_dim,
        PIPELINE_EMBEDDER_DESCRIPTOR_MISMATCH_LABEL
    );
    Ok(())
}

/// The novelty and dedup roots the legacy gate opens, resolved exactly as
/// the ingest binary's `build_dedup_vector_index_from_env` resolves the
/// dedup default.
fn legacy_index_roots(lookup: &dyn Fn(&str) -> Option<String>) -> (PathBuf, PathBuf) {
    let novelty = lookup(TRACE_COMMONS_VECTOR_INDEX_ROOT)
        .unwrap_or_else(|| TRACE_COMMONS_VECTOR_INDEX_DEFAULT_ROOT.to_string());
    let dedup = lookup(TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT)
        .unwrap_or_else(|| format!("{novelty}/../dedup-index"));
    (PathBuf::from(novelty), PathBuf::from(dedup))
}

/// The pipeline index root: required, and never a legacy root
/// (`pipeline_vector_index_root_shared`).
pub fn pipeline_index_root_from(
    lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<PathBuf> {
    let root = lookup_trimmed(lookup, TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!(PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL))?;
    let (novelty, dedup) = legacy_index_roots(lookup);
    validate_pipeline_index_root(&root, &[&novelty, &dedup])?;
    Ok(root)
}

/// What a production boot hands `PipelineGateComponents::from_env`
/// (`near-ai-scorer`) besides the environment: `main`'s tenant policies (as
/// the authority provider reads them), its require-policy flag, and which
/// tenants it reads policies for from the database.
pub struct PipelineComponentInputs {
    pub tenant_policies: Arc<BTreeMap<String, SubmissionAllowlists>>,
    pub require_tenant_submission_policy: bool,
    pub db_policy_reads: DbTenantPolicyReads,
}

#[cfg(feature = "near-ai-scorer")]
pub use near_ai::*;

#[cfg(feature = "near-ai-scorer")]
mod near_ai {
    use std::sync::Arc;

    use anyhow::Context;
    use trace_commons_gate_api::{Embedder, PerplexityScorer};
    use trace_commons_gate_enclave::vector_index_usearch::UsearchVectorIndexConfig;

    use super::*;
    use crate::versioned_pipeline_authority::PipelinePrivacyBoundary;
    use crate::versioned_pipeline_production::{
        PipelineGateComponentParts, PipelineGateComponents, TenantPolicyPipelineAuthorityProvider,
        UsearchPipelineIndex,
    };

    /// The NEAR AI scorer and the fastembed embedder, built once from the
    /// environment and held as shared trait objects, with the values they
    /// were built from. The legacy `enclave_near_ai` gate holds these same
    /// two (spec A-D3), so the embedder is loaded once.
    pub struct NearAiGateSharedComponents {
        pub scorer: Arc<dyn PerplexityScorer>,
        pub embedder: Arc<dyn Embedder>,
        pub pins: LegacyGatePins,
    }

    impl NearAiGateSharedComponents {
        /// The legacy gate's construction of the scorer and the embedder,
        /// unchanged: the same variables, the same parse, the same refusals.
        pub async fn from_env() -> anyhow::Result<Self> {
            use std::time::Duration as StdDuration;
            use trace_commons_gate_enclave::embedder_fastembed::FastEmbedTextEmbedder;
            use trace_commons_gate_enclave::{NearAiPerplexityScorer, NearAiScorerConfig};

            let base_url = std::env::var(TRACE_COMMONS_NEAR_AI_BASE_URL).with_context(|| {
                format!(
                    "{TRACE_COMMONS_NEAR_AI_BASE_URL} must be set when {TRACE_COMMONS_GATE_SERVICE}=\"enclave_near_ai\""
                )
            })?;
            let model = std::env::var(TRACE_COMMONS_NEAR_AI_MODEL).with_context(|| {
                format!(
                    "{TRACE_COMMONS_NEAR_AI_MODEL} must be set when {TRACE_COMMONS_GATE_SERVICE}=\"enclave_near_ai\""
                )
            })?;
            let api_key = std::env::var(TRACE_COMMONS_NEAR_AI_API_KEY).with_context(|| {
                format!(
                    "{TRACE_COMMONS_NEAR_AI_API_KEY} must be set when {TRACE_COMMONS_GATE_SERVICE}=\"enclave_near_ai\""
                )
            })?;
            let timeout_seconds = match std::env::var(TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS) {
                Ok(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        TRACE_COMMONS_NEAR_AI_DEFAULT_TIMEOUT_SECONDS
                    } else {
                        trimmed.parse::<u64>().with_context(|| {
                            format!(
                                "{TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS} must be a positive integer"
                            )
                        })?
                    }
                }
                Err(_) => TRACE_COMMONS_NEAR_AI_DEFAULT_TIMEOUT_SECONDS,
            };
            anyhow::ensure!(
                timeout_seconds > 0,
                "{TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS} must be greater than zero"
            );
            let tail_cutoff = match std::env::var(TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF) {
                Ok(raw) => raw.trim().parse::<f32>().with_context(|| {
                    format!(
                        "{TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF} must be a floating-point number"
                    )
                })?,
                Err(_) => TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF,
            };
            anyhow::ensure!(
                tail_cutoff.is_finite(),
                "{TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF} must be finite"
            );

            let scorer_cfg = NearAiScorerConfig {
                base_url,
                model: model.clone(),
                api_key,
                tail_logprob_cutoff: tail_cutoff,
                logprobs_top_k: TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K,
                timeout: StdDuration::from_secs(timeout_seconds),
            };
            // reqwest's blocking client owns an internal Tokio runtime and must not be
            // constructed from this async startup task. Build it on the blocking pool,
            // which is also where synchronous gate evaluation runs.
            let scorer =
                tokio::task::spawn_blocking(move || NearAiPerplexityScorer::try_new(scorer_cfg))
                    .await
                    .context("NearAiPerplexityScorerInitJoinFailed")?
                    .context("NearAiPerplexityScorerInitFailed")?;

            // fastembed-rs embedder — same configuration surface as the local-GPU
            // path. Runs locally on CPU; no GPU required.
            let embedder_model_id = std::env::var(TRACE_COMMONS_EMBEDDER_MODEL_ID)
                .unwrap_or_else(|_| TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID.to_string());
            let embedder_cache_dir = std::env::var(TRACE_COMMONS_EMBEDDER_CACHE_DIR)
                .unwrap_or_else(|_| TRACE_COMMONS_EMBEDDER_DEFAULT_CACHE_DIR.to_string());
            let embedder_max_tokens = match std::env::var(TRACE_COMMONS_EMBEDDER_MAX_TOKENS) {
                Ok(raw) => raw.trim().parse::<usize>().with_context(|| {
                    format!("{TRACE_COMMONS_EMBEDDER_MAX_TOKENS} must be a positive integer")
                })?,
                Err(_) => TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS,
            };
            anyhow::ensure!(
                embedder_max_tokens > 0,
                "{TRACE_COMMONS_EMBEDDER_MAX_TOKENS} must be greater than zero"
            );
            let embedder_matryoshka_dim = match std::env::var(TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM)
            {
                Ok(raw) => {
                    let trimmed = raw.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.parse::<usize>().with_context(|| {
                            format!(
                                "{TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM} must be a positive integer"
                            )
                        })?)
                    }
                }
                Err(_) => None,
            };
            if let Some(d) = embedder_matryoshka_dim {
                anyhow::ensure!(
                    d > 0,
                    "{TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM} must be greater than zero"
                );
            }
            let embedder = FastEmbedTextEmbedder::try_new(
                embedder_model_id.clone(),
                &embedder_cache_dir,
                embedder_matryoshka_dim,
                embedder_max_tokens,
            )
            .await
            .context("FastEmbedTextEmbedderInitFailed")?;
            let embedder_output_dim = embedder.output_dim();
            Ok(Self {
                scorer: Arc::new(scorer),
                embedder: Arc::new(embedder),
                pins: LegacyGatePins {
                    model,
                    tail_logprob_cutoff: tail_cutoff,
                    embedder_model_id,
                    embedder_output_dim,
                    embedder_max_tokens,
                    embedder_matryoshka_dim,
                },
            })
        }
    }

    /// The usearch parameters the legacy novelty index is opened with, in
    /// the order and with the parse it reads them. The periodic flush
    /// interval is not among them: the caller sets it (the pipeline index
    /// runs with none).
    pub fn usearch_index_config_from_env() -> anyhow::Result<UsearchVectorIndexConfig> {
        let dim = parse_usize_env(
            TRACE_COMMONS_VECTOR_INDEX_DIM,
            TRACE_COMMONS_VECTOR_INDEX_DEFAULT_DIM,
        )?;
        let max_open = parse_usize_env(
            TRACE_COMMONS_VECTOR_INDEX_MAX_OPEN,
            TRACE_COMMONS_VECTOR_INDEX_DEFAULT_MAX_OPEN,
        )?;
        let flush_every = parse_usize_env(
            TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY,
            TRACE_COMMONS_VECTOR_INDEX_DEFAULT_FLUSH_EVERY,
        )?;
        let hnsw_m = parse_usize_env(
            TRACE_COMMONS_VECTOR_INDEX_HNSW_M,
            TRACE_COMMONS_VECTOR_INDEX_DEFAULT_HNSW_M,
        )?;
        let ef_construction = parse_usize_env(
            TRACE_COMMONS_VECTOR_INDEX_EF_CONSTRUCTION,
            TRACE_COMMONS_VECTOR_INDEX_DEFAULT_EF_CONSTRUCTION,
        )?;
        let ef_search = parse_usize_env(
            TRACE_COMMONS_VECTOR_INDEX_EF_SEARCH,
            TRACE_COMMONS_VECTOR_INDEX_DEFAULT_EF_SEARCH,
        )?;
        Ok(UsearchVectorIndexConfig {
            dim,
            hnsw_m,
            ef_construction,
            ef_search,
            max_open,
            flush_every,
            flush_interval: None,
        })
    }

    /// The pipeline's privacy boundary (spec A-D7): `main`'s classifier
    /// adapter and policy, or `None` when no backend is configured.
    fn pipeline_privacy_from_env() -> anyhow::Result<(
        Option<Arc<dyn PipelinePrivacyBoundary>>,
        Option<trace_commons_protocol::trace_contribution::PrivacyFilterBackendTag>,
    )> {
        let Some((adapter, backend)) =
            trace_commons_protocol::trace_contribution::privacy_filter_adapter_from_env()
                .map_err(|_| anyhow::anyhow!("pipeline_privacy_filter_unavailable"))?
        else {
            return Ok((None, None));
        };
        let boundary =
            crate::versioned_pipeline_authority::ClassifierRedactorPipelinePrivacyBoundary::new(
                adapter,
                backend,
                trace_commons_protocol::trace_contribution::PiiClassifyPolicy::from_env(),
            );
        Ok((Some(Arc::new(boundary)), Some(backend)))
    }

    impl PipelineGateComponents {
        /// The production pipeline's gate components from the environment
        /// (spec A-D3 to A-D8): the only constructor whose scorer and
        /// embedder adapters report themselves production-qualified.
        ///
        /// In order: the scorer and embedder descriptors and the pipeline
        /// index root are parsed (no network, no model load); the NEAR AI
        /// scorer and the fastembed embedder are built
        /// ([`NearAiGateSharedComponents::from_env`]); the descriptors must
        /// agree with what was built (`pipeline_scorer_descriptor_mismatch`,
        /// `pipeline_embedder_descriptor_mismatch`); the usearch pipeline
        /// index is opened on its own root; then the privacy boundary and
        /// the tenant-policy authority. Returns the scorer and embedder too,
        /// so the legacy gate holds the same two.
        pub async fn from_env(
            inputs: PipelineComponentInputs,
        ) -> anyhow::Result<(Arc<Self>, NearAiGateSharedComponents)> {
            let scorer_descriptor = near_ai_scorer_descriptor_from_lookup(&std_env_lookup)?;
            let embedder_descriptor = fastembed_descriptor_from_lookup(&std_env_lookup)?;
            let pipeline_root = pipeline_index_root_from(&std_env_lookup)?;
            let shared = NearAiGateSharedComponents::from_env().await?;
            ensure_descriptors_match_gate(&scorer_descriptor, &embedder_descriptor, &shared.pins)?;
            // Every pipeline write flushes, so no periodic flusher thread is
            // needed.
            let index = Arc::new(UsearchPipelineIndex::open(
                &pipeline_root,
                usearch_index_config_from_env()?,
            )?);
            let (privacy, privacy_backend) = pipeline_privacy_from_env()?;
            let tenant_policy_count = inputs.tenant_policies.len();
            let components = Self::production(PipelineGateComponentParts {
                scorer: shared.scorer.clone(),
                scorer_descriptor,
                embedder: shared.embedder.clone(),
                embedder_descriptor,
                index_reader: index.clone(),
                index_writer: index,
                index_root_shared_with_legacy: false,
                authority: Arc::new(TenantPolicyPipelineAuthorityProvider::new(
                    inputs.tenant_policies,
                    inputs.require_tenant_submission_policy,
                    inputs.db_policy_reads,
                )),
                tenant_policy_count,
                privacy,
                privacy_backend,
            });
            Ok((Arc::new(components), shared))
        }
    }
}
