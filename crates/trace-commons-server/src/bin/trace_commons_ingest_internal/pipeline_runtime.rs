// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

/// What a proprietary production pipeline assembly needs from ingest: the
/// PostgreSQL backend the pipeline's own tables live on, the artifact store
/// envelopes and pipeline artifacts are written to, that store's name, and
/// the per-phase claim lease. All four come from the
/// connections and configuration
/// `AppState::from_env_with_pipeline_runtime_assembler` already holds.
/// An assembly passes `object_store_name` to
/// `PipelineServiceBuilder::with_object_store_name` and `lease_config` to
/// `PipelineServiceBuilder::with_lease_config`, so the assembled service
/// carries the configured store's label and lease lengths, the same way M11
/// pins the store name; `assemble_ingest_pipeline_runtime` refuses a service
/// that does not.
///
/// `near_contract_id` is the NEAR credit contract `main`'s legacy NEAR path
/// is configured with (`TRACE_COMMONS_CREDIT_SETTLEMENT_NEAR_CONTRACT_ID`).
/// An assembly that enables payout passes it as
/// `PipelinePayoutConfig::near_contract_id` (Ruling T10-4);
/// `assemble_ingest_pipeline_runtime` refuses an enabled payout that names
/// another contract, or any contract when none is configured.
///
/// `near_confirmation_interval` is `main`'s NEAR outbox scheduler cadence
/// (`TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_INTERVAL_SECONDS`, 60 seconds
/// unless configured). An assembly that enables payout passes it as
/// `PipelinePayoutConfig::confirmation_interval` (Ruling T10-10), and
/// `assemble_ingest_pipeline_runtime` refuses one that does not.
pub struct IngestPipelineRuntimeContext {
    pub backend: Arc<PgBackend>,
    pub artifact_store: Arc<dyn TraceArtifactStore>,
    pub object_store_name: String,
    pub lease_config: PipelineLeaseConfig,
    pub near_contract_id: Option<String>,
    pub near_confirmation_interval: StdDuration,
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
/// DB-mirror and artifact-store configuration ingest already loaded.
///
/// Fail-closed dependency qualification: a
/// non-production-qualified dependency (the Reference scorer, the in-memory
/// `IsolatedPipelineIndex`, `RecordingSettlementAdapter`, or the like) refuses
/// startup with `pipeline_runtime_dependencies_not_production_qualified`
/// whenever `tenants_routed` or `production_required` is true -- tenants
/// routed to the pipeline is exactly the condition under which real receipts
/// would otherwise be scored and settled by test doubles. This holds whether
/// or not `production_required` itself is set; `tenants_routed` alone is
/// enough. `allow_test_dependencies` is the only way past that refusal, is
/// meant for tests and local development only, and never combines with
/// `production_required` -- both set refuses startup at once with
/// `pipeline_test_dependencies_not_allowed_when_required`, regardless of
/// qualification. The caller resolves both booleans from the environment (or
/// from the tenant rollout gates); this function reads neither directly.
/// `near_contract_id` is `main`'s configured NEAR credit contract and
/// `near_confirmation_interval` its NEAR outbox scheduler cadence, both
/// handed to the assembly in its context.
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_ingest_pipeline_runtime(
    assembler: Option<&dyn IngestPipelineRuntimeAssembler>,
    db_connections: Option<&TraceCorpusDbConnections>,
    artifact_store: Option<&ConfiguredTraceArtifactStore>,
    production_required: bool,
    lease_config: PipelineLeaseConfig,
    tenants_routed: bool,
    allow_test_dependencies: bool,
    near_contract_id: Option<&str>,
    near_confirmation_interval: StdDuration,
) -> anyhow::Result<Option<Arc<PipelineService>>> {
    anyhow::ensure!(
        !(allow_test_dependencies && production_required),
        "pipeline_test_dependencies_not_allowed_when_required"
    );
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
    let configured_store = artifact_store
        .ok_or_else(|| anyhow::anyhow!("pipeline_runtime_artifact_store_unavailable"))?;
    let object_store_name = configured_store.object_store_name().to_string();
    let service = assembler.assemble(IngestPipelineRuntimeContext {
        backend,
        artifact_store: configured_store.store.clone(),
        object_store_name: object_store_name.clone(),
        lease_config,
        near_contract_id: near_contract_id.map(str::to_string),
        near_confirmation_interval,
    })?;
    // M11: every object ref the pipeline commits names the store it was
    // written to, exactly as a legacy receipt's does.
    anyhow::ensure!(
        service.object_store_name() == object_store_name,
        "pipeline_runtime_object_store_mismatch"
    );
    // The same shape as the store-name check above -- an assembly that
    // ignores the configured lease lengths would silently run every phase
    // under whatever lease lengths its own construction happened to pick.
    anyhow::ensure!(
        service.lease_config() == lease_config,
        "pipeline_runtime_lease_config_mismatch"
    );
    // Ruling T10-4: an enabled payout pays through the NEAR credit contract
    // `main` is configured with, never one the assembly picked itself, and
    // not at all when `main` has none.
    anyhow::ensure!(
        !service.payout_enabled() || service.payout_near_contract_id() == near_contract_id,
        "pipeline_runtime_near_contract_mismatch"
    );
    // Ruling T10-10: an enabled payout polls a submitted payout at `main`'s
    // NEAR outbox scheduler cadence.
    anyhow::ensure!(
        !service.payout_enabled()
            || service.payout_confirmation_interval() == Some(near_confirmation_interval),
        "pipeline_runtime_near_confirmation_interval_mismatch"
    );
    if (production_required || tenants_routed)
        && !pipeline_runtime_is_production_qualified(&service)
    {
        anyhow::ensure!(
            allow_test_dependencies,
            "pipeline_runtime_dependencies_not_production_qualified"
        );
        // The `ensure!` above already refused the combination of
        // `allow_test_dependencies` with `production_required`, so reaching
        // here means the opt-in is what let this specific, otherwise-refused
        // runtime start.
        tracing::warn!("pipeline_runtime_test_dependencies_allowed");
    }
    Ok(Some(service))
}

/// Whether every dependency an injected pipeline runtime holds is
/// production-qualified.
///
/// Checks `scorer`, `embedder`, `index_reader`, `index_writer`, every
/// registered settlement adapter (decision P4's
/// `PipelineDependencyQualification`), and now `authority` and `privacy`
/// (Ruling T2-2): an unqualified authority provider or privacy boundary
/// fails closed the same way an unqualified scorer or index does, whenever
/// tenants are routed. The NEAR payout adapter counts only when payout is
/// enabled (`PipelineService::payout_enabled`): a service that pays nothing
/// out holds no payout dependency to qualify.
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
        && qualification.authority
        && qualification.privacy
        && (!service.payout_enabled() || qualification.payout)
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

/// How many due staged receipt attempts the worker sweeps for one tenant per
/// pass (`PipelineService::sweep_staged_receipts`), after draining its runs.
/// The rest wait for the next pass.
const PIPELINE_WORKER_MAX_SWEPT_RECEIPTS_PER_TENANT: usize = 32;

/// How many of one tenant's due index invalidations the worker processes
/// per pass (`PipelineService::process_index_invalidations`), right after
/// draining its runs. The rest wait for the next pass.
const PIPELINE_WORKER_MAX_INDEX_INVALIDATIONS_PER_TENANT: usize = 32;

/// How many of one tenant's complete runs the worker pays out per pass
/// (`PipelineService::process_payouts`), after its index invalidations. The
/// rest wait for the next pass.
const PIPELINE_WORKER_MAX_PAYOUTS_PER_TENANT: usize = 32;

/// How long the worker sleeps between iterations when `stop` does not fire
/// first.
const PIPELINE_WORKER_POLL_INTERVAL: StdDuration = StdDuration::from_millis(200);

/// One worker pass: probe readiness, then drain each rollout tenant's queue
/// in turn with `drain`, checking `stop` between tenants.
///
/// Supervision: the probe and every tenant's batch each run as their
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
///
/// Then, whatever the runs did, it processes up to
/// `PIPELINE_WORKER_MAX_INDEX_INVALIDATIONS_PER_TENANT` of the tenant's due
/// index invalidations (`process_index_invalidations`, which removes a
/// withdrawn or cancelled revision from the index), pays out up to
/// `PIPELINE_WORKER_MAX_PAYOUTS_PER_TENANT` of the tenant's complete runs
/// (`process_payouts`, which does nothing unless payout is enabled; Ruling
/// S7 puts the invalidations right after the runs and the payouts after
/// them), and sweeps up to
/// `PIPELINE_WORKER_MAX_SWEPT_RECEIPTS_PER_TENANT` of the tenant's receipt
/// attempts that never committed: each staged object whose
/// row's `cleanup_after` has passed is deleted with its row. An
/// invalidation, payout, or sweep failure is logged the same way, and the
/// drain goes on to the next step. All of it runs in the pass's supervised
/// task for the tenant (`run_pipeline_worker_pass`).
pub(crate) async fn drain_pipeline_tenant(service: Arc<PipelineService>, tenant_id: String) {
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
    if let Err(error) = service
        .process_index_invalidations(
            &tenant_id,
            PIPELINE_WORKER_MAX_INDEX_INVALIDATIONS_PER_TENANT,
        )
        .await
    {
        tracing::warn!(
            error_class = "pipeline_worker_index_invalidation_failed",
            tenant_storage_ref = %tenant_storage_ref(&tenant_id),
            error_hash = %safe_display_error_hash(&error),
            "pipeline worker index invalidation failed"
        );
    }
    if let Err(error) = service
        .process_payouts(&tenant_id, PIPELINE_WORKER_MAX_PAYOUTS_PER_TENANT)
        .await
    {
        tracing::warn!(
            error_class = "pipeline_worker_payout_failed",
            tenant_storage_ref = %tenant_storage_ref(&tenant_id),
            error_hash = %safe_display_error_hash(&error),
            "pipeline worker payout failed"
        );
    }
    if let Err(error) = service
        .sweep_staged_receipts(&tenant_id, PIPELINE_WORKER_MAX_SWEPT_RECEIPTS_PER_TENANT)
        .await
    {
        tracing::warn!(
            error_class = "pipeline_worker_receipt_sweep_failed",
            tenant_storage_ref = %tenant_storage_ref(&tenant_id),
            error_hash = %safe_display_error_hash(&error),
            "pipeline worker receipt sweep failed"
        );
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

/// Registers and activates the injected pipeline runtime's default bundle
/// once for every `PipelineReceipts` rollout tenant, so none of them reaches
/// its first receipt without an active bundle already on file. `None` when
/// no runtime is injected -- there is nothing to register.
///
/// Called from `run_pipeline_app`, after the runtime this process holds was
/// assembled and checked (`assemble_ingest_pipeline_runtime`) and before the
/// worker or the HTTP listener starts: a receipt no longer registers
/// anything itself (`PipelineService::submit` assumes an active bundle
/// already exists), so a tenant this call never reaches would refuse every
/// receipt with a missing-bundle error instead. A registration failure
/// refuses startup rather than let that happen silently.
async fn register_default_bundles_for_rollout_tenants(state: &AppState) -> anyhow::Result<()> {
    let Some(service) = state.pipeline_service.as_ref() else {
        return Ok(());
    };
    for tenant_id in state
        .tenant_rollout_gates
        .tenant_ids(TraceTenantRolloutFeature::PipelineReceipts)
    {
        service
            .register_default_bundle(&tenant_id)
            .await
            .map_err(|_| anyhow::anyhow!("pipeline_default_bundle_registration_failed"))?;
    }
    Ok(())
}

/// Waits up to `grace` for a spawned task to finish on its own; if it has
/// not, aborts it. Mirrors the wait-then-abort rule
/// `serve_ingest_with_graceful_shutdown`'s watchdog already applies to the
/// HTTP server task: a task that does not notice its own stop signal within
/// the shutdown grace period is a defect (a stuck drain, a wedged
/// connection), and letting it keep running after shutdown has already
/// logged as complete is exactly how it would go on touching a database a
/// later caller assumes is now quiescent. `handle` is taken by value and
/// consumed either way, so a caller cannot accidentally await or drop it
/// again afterward.
pub(crate) async fn join_or_abort<T>(mut handle: tokio::task::JoinHandle<T>, grace: StdDuration) {
    if tokio::time::timeout(grace, &mut handle).await.is_err() {
        tracing::warn!(
            "pipeline worker did not stop within the shutdown grace period; aborting it"
        );
        handle.abort();
    }
}

/// Serves ingest and, when a pipeline runtime is injected, runs the owned
/// worker loop alongside it. `shutdown` stops HTTP first, through
/// `serve_ingest_with_graceful_shutdown`'s own grace period; once that
/// resolves, the worker (if any) is asked to stop and given the same grace
/// period to confirm it did. A worker that does not stop in time is aborted
/// (`join_or_abort`) rather than left running -- shutdown still completes
/// either way.
pub async fn run_pipeline_app(
    state: Arc<AppState>,
    listener: TcpListener,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> anyhow::Result<()> {
    register_default_bundles_for_rollout_tenants(&state).await?;
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
        join_or_abort(worker.join, StdDuration::from_secs(grace)).await;
    }
    result
}
