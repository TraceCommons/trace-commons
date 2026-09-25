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

/// Label-only readiness body. `reason` is present only when `status` is
/// `"not_ready"`, so a ready response serialises to exactly `{"status":
/// "ready"}` with no dangling null field.
#[derive(Debug, Serialize)]
pub(crate) struct PipelineReadinessResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
}

/// Answers `GET /v1/pipeline/readiness`. Unauthenticated and registered in
/// `app()` beside `/health`, outside every auth layer, for the same reason:
/// an orchestrator's readiness probe runs before any credential exists to
/// present. The body never carries a tenant id, run id, or error text --
/// only the worker's own boolean state, named by a fixed label.
pub(crate) async fn pipeline_readiness_handler(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<PipelineReadinessResponse>) {
    if state.pipeline_service.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(PipelineReadinessResponse {
                status: "not_ready",
                reason: Some("pipeline_runtime_absent"),
            }),
        );
    }
    if !state
        .pipeline_worker_ready
        .load(std::sync::atomic::Ordering::Relaxed)
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(PipelineReadinessResponse {
                status: "not_ready",
                reason: Some("pipeline_worker_not_ready"),
            }),
        );
    }
    (
        StatusCode::OK,
        Json(PipelineReadinessResponse {
            status: "ready",
            reason: None,
        }),
    )
}

/// Handle to the worker loop `spawn_pipeline_worker` starts. `stop` asks the
/// loop to exit at its next check (best-effort: the loop checks between
/// iterations, not mid-`process_one`); `join` is awaited -- bounded by the
/// shutdown grace period -- to confirm it actually did. `ready` is the same
/// `Arc<AtomicBool>` as `AppState::pipeline_worker_ready`, so the readiness
/// handler and the worker share one flag rather than needing to agree on two.
struct PipelineWorkerHandle {
    stop: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
    ready: Arc<std::sync::atomic::AtomicBool>,
}

/// How many runs of one tenant's pipeline queue the worker drains before
/// moving to the next rollout tenant, once per loop iteration. Bounds one
/// tenant's backlog from starving every other tenant sharing this loop --
/// the same tenant gets another turn on the very next iteration regardless.
const PIPELINE_WORKER_MAX_RUNS_PER_TENANT: usize = 32;

/// How long the worker sleeps between iterations when `stop` does not fire
/// first.
const PIPELINE_WORKER_POLL_INTERVAL: StdDuration = StdDuration::from_millis(200);

/// One worker pass: probe readiness, then drain each rollout tenant's queue
/// in turn with `drain`, checking `stop` between tenants.
///
/// Supervision (I5): the probe and every tenant's batch each run as their
/// own tokio task, so a panic in a policy, an adapter, or a row decode ends
/// only that task. The pass logs a fixed label (never the panic's own
/// text), reports not ready, and goes on to the next tenant; the loop
/// itself never ends on a panic.
///
/// `ready` goes false at once when the probe fails or a task does not
/// finish, and true at the end of a pass whose probe succeeded and whose
/// every task finished, unless `stop` has fired by then -- so a batch that
/// panics on every pass keeps the worker not ready rather than flapping.
pub(crate) async fn run_pipeline_worker_pass<Probe, Drain, DrainFuture>(
    probe: Probe,
    tenant_ids: impl IntoIterator<Item = String>,
    drain: Drain,
    ready: &std::sync::atomic::AtomicBool,
    stop: &tokio::sync::watch::Receiver<bool>,
) where
    Probe: std::future::Future<Output = anyhow::Result<()>> + Send + 'static,
    Drain: Fn(String) -> DrainFuture,
    DrainFuture: std::future::Future<Output = ()> + Send + 'static,
{
    let mut pass_ready = match tokio::spawn(probe).await {
        Ok(Ok(())) => true,
        Ok(Err(error)) => {
            ready.store(false, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!(
                error_class = "PipelineWorkerReadinessProbeFailed",
                error_hash = %safe_display_error_hash(&error),
                "pipeline worker readiness probe failed"
            );
            false
        }
        Err(join_error) => {
            ready.store(false, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!(
                error_class = pipeline_worker_task_failure_class(&join_error),
                "pipeline worker readiness probe did not finish"
            );
            false
        }
    };
    for tenant_id in tenant_ids {
        let storage_ref = tenant_storage_ref(&tenant_id);
        if let Err(join_error) = tokio::spawn(drain(tenant_id)).await {
            pass_ready = false;
            ready.store(false, std::sync::atomic::Ordering::Relaxed);
            tracing::warn!(
                error_class = pipeline_worker_task_failure_class(&join_error),
                tenant_storage_ref = %storage_ref,
                "pipeline worker tenant batch did not finish"
            );
        }
        if *stop.borrow() {
            break;
        }
    }
    if pass_ready && !*stop.borrow() {
        ready.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The label a supervised worker task that did not finish is logged under.
/// A `JoinError`'s panic payload is never read or logged.
fn pipeline_worker_task_failure_class(join_error: &tokio::task::JoinError) -> &'static str {
    if join_error.is_panic() {
        "PipelineWorkerTaskPanicked"
    } else {
        "PipelineWorkerTaskCancelled"
    }
}

/// Drains one tenant's own queue with its own `process_one` (never a
/// cross-tenant claim, D2), up to `PIPELINE_WORKER_MAX_RUNS_PER_TENANT` runs.
/// `process_one` claims through `claim_next`, so an expired lease from a
/// crashed run is claimable again without special handling here. A run
/// failure is logged with a label and the tenant's `tenant_storage_ref` --
/// never the tenant id or the error's own text -- and ends this tenant's
/// batch for the pass.
async fn drain_pipeline_tenant(service: Arc<PipelineService>, tenant_id: String) {
    for _ in 0..PIPELINE_WORKER_MAX_RUNS_PER_TENANT {
        match service.process_one(&tenant_id).await {
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(
                    error_class = "PipelineWorkerRunFailed",
                    tenant_storage_ref = %tenant_storage_ref(&tenant_id),
                    error_hash = %safe_display_error_hash(&error),
                    "pipeline worker run failed"
                );
                break;
            }
        }
    }
}

/// Starts the owned pipeline worker loop. `None` when no pipeline runtime is
/// injected -- there is nothing to drain, and the repository binary injects
/// none.
///
/// Each iteration runs one `run_pipeline_worker_pass` over the
/// `PipelineReceipts` rollout tenants, then sleeps
/// `PIPELINE_WORKER_POLL_INTERVAL` or until `stop` fires.
fn spawn_pipeline_worker(state: Arc<AppState>) -> Option<PipelineWorkerHandle> {
    let service = state.pipeline_service.clone()?;
    let gates = state.tenant_rollout_gates.clone();
    let ready = state.pipeline_worker_ready.clone();
    let worker_ready = ready.clone();
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        while !*stop_rx.borrow() {
            let probe_service = service.clone();
            let drain_service = service.clone();
            run_pipeline_worker_pass(
                async move { probe_service.readiness().await },
                gates.tenant_ids(TraceTenantRolloutFeature::PipelineReceipts),
                |tenant_id| drain_pipeline_tenant(drain_service.clone(), tenant_id),
                &worker_ready,
                &stop_rx,
            )
            .await;

            tokio::select! {
                _ = tokio::time::sleep(PIPELINE_WORKER_POLL_INTERVAL) => {},
                _ = stop_rx.changed() => {},
            }
        }
    });
    Some(PipelineWorkerHandle {
        stop: stop_tx,
        join,
        ready,
    })
}

/// The pipeline app: ingest's ordinary router, unchanged. A separate name
/// from `app()` because `run_pipeline_app` -- the ingest binary's actual
/// entry point -- is what owns the worker lifecycle; `app()` alone stays the
/// thing every existing handler-level test builds directly.
pub fn build_pipeline_app(state: Arc<AppState>) -> Router {
    app(state)
}

/// Serves ingest and, when a pipeline runtime is injected, runs the owned
/// worker loop alongside it. `shutdown` stops HTTP first, through
/// `serve_ingest_with_graceful_shutdown`'s own grace period; once that
/// resolves, the worker (if any) is asked to stop and given the same grace
/// period to confirm it did. A worker that does not stop in time is logged
/// with a label and left running rather than panicking -- shutdown still
/// completes.
pub async fn run_pipeline_app(
    state: Arc<AppState>,
    listener: TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    let worker = spawn_pipeline_worker(state.clone());
    let grace = parse_usize_env(
        TRACE_COMMONS_SHUTDOWN_GRACE_SECONDS,
        TRACE_COMMONS_DEFAULT_SHUTDOWN_GRACE_SECONDS,
    )? as u64;
    let result =
        serve_ingest_with_graceful_shutdown(listener, build_pipeline_app(state), grace, shutdown)
            .await;
    if let Some(worker) = worker {
        let _ = worker.stop.send(true);
        // Stop advertising ready the moment shutdown is requested, rather
        // than leaving the last readiness probe's result standing until the
        // loop wakes for its next (possibly final) iteration.
        worker
            .ready
            .store(false, std::sync::atomic::Ordering::Relaxed);
        if tokio::time::timeout(std::time::Duration::from_secs(grace), worker.join)
            .await
            .is_err()
        {
            tracing::warn!("pipeline worker did not stop within the shutdown grace period");
        }
    }
    result
}
