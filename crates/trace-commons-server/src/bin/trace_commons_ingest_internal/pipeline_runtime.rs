// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

/// What a proprietary production pipeline assembly needs from ingest: the
/// PostgreSQL backend the pipeline's own tables live on, and the artifact
/// store envelopes and pipeline artifacts are written to. Both are the same
/// connections `AppState::from_env_with_pipeline_runtime_assembler` already
/// holds from its own DB-mirror and artifact-store configuration.
pub struct IngestPipelineRuntimeContext {
    pub backend: Arc<PgBackend>,
    pub artifact_store: Arc<dyn TraceArtifactStore>,
}

/// Compile-time injection seam for a proprietary production pipeline
/// assembly. The stock binary intentionally has no implementation: the
/// scorer, embedder, vector index, settlement, and payout backends a
/// deployable pipeline needs do not live in this tree.
pub trait IngestPipelineRuntimeAssembler: Send + Sync {
    fn assemble(
        &self,
        context: IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>>;
}

/// Assembles the optional pipeline runtime.
///
/// `assembler: None` returns `Ok(None)` unless `production_required`, in
/// which case it fails closed with `pipeline_runtime_required_but_not_injected`
/// rather than letting ingest boot without a pipeline. Given an assembler,
/// this resolves the PostgreSQL backend and artifact store it needs from the
/// DB-mirror and artifact-store configuration ingest already loaded, and --
/// when `production_required` -- refuses to boot with a dependency that is
/// not production-qualified.
pub(crate) fn assemble_ingest_pipeline_runtime(
    assembler: Option<&dyn IngestPipelineRuntimeAssembler>,
    db_connections: Option<&TraceCorpusDbConnections>,
    artifact_store: Option<&ConfiguredTraceArtifactStore>,
    production_required: bool,
) -> anyhow::Result<Option<Arc<PipelineService>>> {
    let Some(assembler) = assembler else {
        anyhow::ensure!(
            !production_required,
            "pipeline_runtime_required_but_not_injected"
        );
        return Ok(None);
    };
    let backend = db_connections
        .map(|connections| connections.postgres.clone())
        .ok_or_else(|| anyhow::anyhow!("pipeline_runtime_database_unavailable"))?;
    let artifact_store = artifact_store
        .map(|configured| configured.store.clone())
        .ok_or_else(|| anyhow::anyhow!("pipeline_runtime_artifact_store_unavailable"))?;
    let service = assembler.assemble(IngestPipelineRuntimeContext {
        backend,
        artifact_store,
    })?;
    if production_required {
        anyhow::ensure!(
            pipeline_runtime_is_production_qualified(&service),
            "pipeline_runtime_dependencies_not_production_qualified"
        );
    }
    Ok(Some(service))
}

/// Whether every dependency an injected pipeline runtime holds is
/// production-qualified.
///
/// Checks only `scorer`, `embedder`, `index_reader`, `index_writer`, and
/// every registered settlement adapter (decision P4's
/// `PipelineDependencyQualification`). Authority, privacy, and payout
/// qualification are PR 3 and are not part of this bundle-runtime
/// dependency set.
pub(crate) fn pipeline_runtime_is_production_qualified(service: &PipelineService) -> bool {
    let qualification = service.dependency_qualification();
    qualification.scorer
        && qualification.embedder
        && qualification.index_reader
        && qualification.index_writer
        && !qualification.settlement_adapters.is_empty()
        && qualification
            .settlement_adapters
            .values()
            .all(|ready| *ready)
}
