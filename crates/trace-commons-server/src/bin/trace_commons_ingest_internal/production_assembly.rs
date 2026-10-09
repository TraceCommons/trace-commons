// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ingest binary's half of the production pipeline assembly (spec
//! `docs/superpowers/specs/2026-10-08-pipeline-production-assembly-design.md`,
//! Slice A): what reads the environment, and the seam ingest assembles
//! through. The descriptors, adapters, assembler and adapters check live in
//! the library (`trace_commons_server::versioned_pipeline_production`), so an
//! integration target can build the same assembly (PR #1295 review, Major 1).
//! Only the builder that constructs the NEAR AI scorer, the fastembed
//! embedder and the usearch index from the environment needs
//! `near-ai-scorer`. A default build therefore never constructs most of
//! what is here, and says so instead of warning about it.
#![cfg_attr(not(feature = "near-ai-scorer"), allow(dead_code))]

use super::*;

pub(crate) use trace_commons_server::versioned_pipeline_production::*;

/// Reads one variable through `lookup`, treating an empty or all-blank value
/// as unset.
fn lookup_trimmed(lookup: &dyn Fn(&str) -> Option<String>, var: &str) -> Option<String> {
    lookup(var)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn std_env_lookup(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

/// The NEAR AI scorer descriptor (spec A-D4) from `lookup`: the same parse
/// the legacy `enclave_near_ai` gate applies to the model and the tail
/// cutoff; `logprobs_top_k` is the constant both use.
pub(crate) fn near_ai_scorer_descriptor_from_lookup(
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
pub(crate) fn fastembed_descriptor_from_lookup(
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

/// `main`'s tenant submission policies as the authority provider reads
/// them: the two allowlists, nothing else.
pub(crate) fn tenant_policy_allowlists(
    policies: &BTreeMap<String, TenantSubmissionPolicy>,
) -> Arc<BTreeMap<String, trace_commons_server::trace_authority::SubmissionAllowlists>> {
    Arc::new(
        policies
            .iter()
            .map(|(tenant_id, policy)| {
                (
                    tenant_id.clone(),
                    trace_commons_server::trace_authority::SubmissionAllowlists {
                        allowed_consent_scopes: policy.allowed_consent_scopes.clone(),
                        allowed_uses: policy.allowed_uses.clone(),
                    },
                )
            })
            .collect(),
    )
}

/// Selects the pipeline runtime ingest starts (spec A-D2).
pub(crate) const TRACE_COMMONS_PIPELINE_RUNTIME: &str = "TRACE_COMMONS_PIPELINE_RUNTIME";
pub(crate) const PIPELINE_RUNTIME_SELECTION_UNKNOWN_LABEL: &str =
    "pipeline_runtime_selection_unknown";
pub(crate) const PIPELINE_RUNTIME_PRODUCTION_REQUIRES_NEAR_AI_SCORER_LABEL: &str =
    "pipeline_runtime_production_requires_near_ai_scorer";
pub(crate) const PIPELINE_RUNTIME_PRODUCTION_REQUIRES_ENCLAVE_NEAR_AI_LABEL: &str =
    "pipeline_runtime_production_requires_enclave_near_ai";

/// Which pipeline runtime this process starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipelineRuntimeSelection {
    /// No pipeline runtime: `run_ingest(None)`, exactly as before.
    None,
    /// [`ProductionPipelineAssembler`] over the shared NEAR AI gate
    /// components.
    Production,
}

impl PipelineRuntimeSelection {
    /// What `GET /v1/admin/config-status` reports.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Production => "production",
        }
    }

    /// The assembler `main` passes `run_ingest` for this selection.
    pub(crate) fn assembler(self) -> Option<&'static dyn IngestPipelineRuntimeAssembler> {
        match self {
            Self::None => None,
            Self::Production => Some(&ProductionPipelineAssembler),
        }
    }
}

/// Spec A-D2: unset (or blank) selects no runtime; `production` selects the
/// production assembly, and only on a `near-ai-scorer` build whose legacy
/// gate is `enclave_near_ai` (the components the pipeline reuses are that
/// gate's); any other value refuses the start.
pub(crate) fn pipeline_runtime_selection(
    raw: Option<&str>,
    gate_service: Option<&str>,
    near_ai_scorer_compiled: bool,
) -> anyhow::Result<PipelineRuntimeSelection> {
    match raw.map(str::trim).unwrap_or_default() {
        "" => Ok(PipelineRuntimeSelection::None),
        "production" => {
            anyhow::ensure!(
                near_ai_scorer_compiled,
                PIPELINE_RUNTIME_PRODUCTION_REQUIRES_NEAR_AI_SCORER_LABEL
            );
            anyhow::ensure!(
                gate_service.map(str::trim) == Some("enclave_near_ai"),
                PIPELINE_RUNTIME_PRODUCTION_REQUIRES_ENCLAVE_NEAR_AI_LABEL
            );
            Ok(PipelineRuntimeSelection::Production)
        }
        _ => anyhow::bail!(PIPELINE_RUNTIME_SELECTION_UNKNOWN_LABEL),
    }
}

pub(crate) fn pipeline_runtime_selection_from_env() -> anyhow::Result<PipelineRuntimeSelection> {
    pipeline_runtime_selection(
        std::env::var(TRACE_COMMONS_PIPELINE_RUNTIME)
            .ok()
            .as_deref(),
        std::env::var(TRACE_COMMONS_GATE_SERVICE).ok().as_deref(),
        cfg!(feature = "near-ai-scorer"),
    )
}

/// The production pipeline assembly behind ingest's assembler seam: maps
/// the context onto [`assemble_production_pipeline`], the library
/// constructor.
pub(crate) struct ProductionPipelineAssembler;

impl IngestPipelineRuntimeAssembler for ProductionPipelineAssembler {
    fn assemble(
        &self,
        context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        let components = context
            .components
            .clone()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_PRODUCTION_COMPONENTS_MISSING_LABEL))?;
        Ok(Arc::new(assemble_production_pipeline(
            ProductionPipelineInputs {
                backend: context.backend,
                artifact_store: context.artifact_store,
                object_store_name: context.object_store_name,
                lease_config: context.lease_config,
                novelty_utility_checks: context.novelty_utility_checks,
                unqualified_routing_allowed: context.unqualified_routing_allowed,
                main_gate: context.main_gate,
                components,
            },
        )?))
    }

    fn needs_gate_components(&self) -> bool {
        true
    }
}

/// What a production boot hands the components builder besides the
/// environment: `main`'s tenant policies, its require-policy flag, and
/// which tenants it reads policies for from the database.
pub(crate) struct PipelineComponentInputs {
    pub(crate) tenant_policies: Arc<BTreeMap<String, TenantSubmissionPolicy>>,
    pub(crate) require_tenant_submission_policy: bool,
    pub(crate) db_policy_reads: DbTenantPolicyReads,
}

/// The values the legacy gate was built with, which the pipeline's
/// descriptors must name: the two parse the same variables, and this
/// refuses a start where they would not agree, so the package can never
/// name a scorer or embedder other than the one that runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LegacyGatePins {
    pub(crate) model: String,
    pub(crate) tail_logprob_cutoff: f32,
    pub(crate) embedder_model_id: String,
    pub(crate) embedder_output_dim: usize,
    pub(crate) embedder_max_tokens: usize,
    pub(crate) embedder_matryoshka_dim: Option<usize>,
}

pub(crate) fn ensure_descriptors_match_gate(
    scorer: &NearAiScorerDescriptor,
    embedder: &FastEmbedDescriptor,
    gate: &LegacyGatePins,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        scorer.model == gate.model
            && scorer.tail_logprob_cutoff == gate.tail_logprob_cutoff
            && scorer.logprobs_top_k == TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K,
        "pipeline_scorer_descriptor_mismatch"
    );
    anyhow::ensure!(
        embedder.model_id == gate.embedder_model_id
            && embedder.output_dim == gate.embedder_output_dim
            && embedder.max_tokens == gate.embedder_max_tokens
            && embedder.matryoshka_dim == gate.embedder_matryoshka_dim,
        "pipeline_embedder_descriptor_mismatch"
    );
    Ok(())
}

/// The novelty and dedup roots the legacy gate opens, resolved exactly as
/// `build_dedup_vector_index_from_env` resolves the dedup default.
fn legacy_index_roots(lookup: &dyn Fn(&str) -> Option<String>) -> (PathBuf, PathBuf) {
    let novelty = lookup(TRACE_COMMONS_VECTOR_INDEX_ROOT)
        .unwrap_or_else(|| TRACE_COMMONS_VECTOR_INDEX_DEFAULT_ROOT.to_string());
    let dedup = lookup(TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT)
        .unwrap_or_else(|| format!("{novelty}/../dedup-index"));
    (PathBuf::from(novelty), PathBuf::from(dedup))
}

/// The pipeline index root: required, and never a legacy root.
pub(crate) fn pipeline_index_root_from(
    lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<PathBuf> {
    let root = lookup_trimmed(lookup, TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!(PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL))?;
    let (novelty, dedup) = legacy_index_roots(lookup);
    validate_pipeline_index_root(&root, &[&novelty, &dedup])?;
    Ok(root)
}

/// The pipeline's privacy boundary (spec A-D7): `main`'s classifier adapter
/// and policy, or `None` when no backend is configured.
fn pipeline_privacy_from_env() -> anyhow::Result<(
    Option<Arc<dyn trace_commons_server::versioned_pipeline_authority::PipelinePrivacyBoundary>>,
    Option<trace_commons_protocol::trace_contribution::PrivacyFilterBackendTag>,
)> {
    let Some((adapter, backend)) =
        trace_commons_protocol::trace_contribution::privacy_filter_adapter_from_env()
            .map_err(|_| anyhow::anyhow!("pipeline_privacy_filter_unavailable"))?
    else {
        return Ok((None, None));
    };
    let boundary =
        trace_commons_server::versioned_pipeline_authority::ClassifierRedactorPipelinePrivacyBoundary::new(
            adapter,
            backend,
            PiiClassifyPolicy::from_env(),
        );
    Ok((Some(Arc::new(boundary)), Some(backend)))
}

/// Builds the legacy `enclave_near_ai` gate and, from the same scorer,
/// embedder and settings, the components the production pipeline holds
/// (spec A-D3, A-D6, A-D7, A-D8). Only this builder names the concrete NEAR
/// AI, fastembed and usearch types.
#[cfg(feature = "near-ai-scorer")]
pub(crate) async fn build_near_ai_gate_service_with_pipeline_components(
    inputs: PipelineComponentInputs,
) -> anyhow::Result<(Arc<dyn TraceGateService>, Arc<PipelineGateComponents>)> {
    let scorer_descriptor = near_ai_scorer_descriptor_from_lookup(&std_env_lookup)?;
    let embedder_descriptor = fastembed_descriptor_from_lookup(&std_env_lookup)?;
    let pipeline_root = pipeline_index_root_from(&std_env_lookup)?;
    let parts = near_ai_gate_parts_from_env().await?;
    ensure_descriptors_match_gate(
        &scorer_descriptor,
        &embedder_descriptor,
        &LegacyGatePins {
            model: parts.model.clone(),
            tail_logprob_cutoff: parts.tail_logprob_cutoff,
            embedder_model_id: parts.embedder_model_id.clone(),
            embedder_output_dim: parts.embedder_output_dim,
            embedder_max_tokens: parts.embedder_max_tokens,
            embedder_matryoshka_dim: parts.embedder_matryoshka_dim,
        },
    )?;
    // Every pipeline write flushes, so no periodic flusher thread is needed.
    let mut index_config = parts.vector_index_config.clone();
    index_config.flush_interval = None;
    let index = Arc::new(UsearchPipelineIndex::open(&pipeline_root, index_config)?);
    let (privacy, privacy_backend) = pipeline_privacy_from_env()?;
    let tenant_policy_count = inputs.tenant_policies.len();
    let components = Arc::new(PipelineGateComponents {
        scorer: parts.scorer.clone(),
        scorer_descriptor,
        embedder: parts.embedder.clone(),
        embedder_descriptor,
        index_reader: index.clone(),
        index_writer: index,
        index_root_shared_with_legacy: false,
        authority: Arc::new(TenantPolicyPipelineAuthorityProvider::new(
            tenant_policy_allowlists(&inputs.tenant_policies),
            inputs.require_tenant_submission_policy,
            inputs.db_policy_reads,
        )),
        tenant_policy_count,
        privacy,
        privacy_backend,
    });
    Ok((near_ai_gate_service_from_parts(parts), components))
}

#[cfg(not(feature = "near-ai-scorer"))]
pub(crate) async fn build_near_ai_gate_service_with_pipeline_components(
    _inputs: PipelineComponentInputs,
) -> anyhow::Result<(Arc<dyn TraceGateService>, Arc<PipelineGateComponents>)> {
    anyhow::bail!(PIPELINE_RUNTIME_PRODUCTION_REQUIRES_NEAR_AI_SCORER_LABEL)
}

/// The three `TRACE_COMMONS_PIPELINE_CHECK_*` variables, as read: the
/// variables `PipelineCheckEmitter::from_env` reads.
pub(crate) fn pipeline_check_vars_from_env() -> PipelineCheckVars {
    PipelineCheckVars {
        dir: std::env::var("TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR").ok(),
        run_id: std::env::var("TRACE_COMMONS_PIPELINE_CHECK_RUN_ID").ok(),
        code_revision_hash: std::env::var("TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH").ok(),
    }
}

/// Where the pipeline's own vector index lives (spec A-D6). Required under
/// the production selection, and never the legacy novelty or dedup root.
pub(crate) const TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT: &str =
    "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT";
pub(crate) const PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL: &str =
    "pipeline_vector_index_root_missing";

#[cfg(test)]
#[path = "production_assembly_tests.rs"]
pub(crate) mod tests;
