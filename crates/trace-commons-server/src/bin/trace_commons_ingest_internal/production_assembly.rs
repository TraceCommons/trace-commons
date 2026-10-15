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

/// Builds the production pipeline's gate components through the library's
/// one constructor, [`PipelineGateComponents::from_env`], and the legacy
/// `enclave_near_ai` gate over the same scorer and embedder (spec A-D3,
/// A-D6, A-D7, A-D8). Only that constructor names the concrete NEAR AI,
/// fastembed and usearch types.
#[cfg(feature = "near-ai-scorer")]
pub(crate) async fn build_near_ai_gate_service_with_pipeline_components(
    inputs: PipelineComponentInputs,
) -> anyhow::Result<(Arc<dyn TraceGateService>, Arc<PipelineGateComponents>)> {
    let (components, shared) = PipelineGateComponents::from_env(inputs).await?;
    let wrapper = near_ai_gate_key_wrapper_from_env().await?;
    Ok((
        near_ai_gate_service_from_parts(near_ai_gate_parts(wrapper, shared)?),
        components,
    ))
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

#[cfg(test)]
#[path = "production_assembly_tests.rs"]
pub(crate) mod tests;
