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
///
/// `novelty_utility_checks` is the configuration of `main`'s
/// `NoveltyUtility` credit checks (Rulings T15-6, T15-10, T15-11): `main`'s
/// central-issuer allowlist, the pipeline's issuer principal
/// (`TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`), and `main`'s
/// production-gate flag. An assembly passes it to
/// `PipelineServiceBuilder::with_novelty_utility_checks`, and
/// `assemble_ingest_pipeline_runtime` refuses one that does not. The tenant
/// policy those checks read comes from the assembly's own authority
/// provider, the source the receipt uses.
///
/// `near_payout_controls` are `main`'s NEAR payout controls: its settlement
/// mode (`TRACE_COMMONS_NEAR_SETTLEMENT_MODE`) and
/// `TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH`, resolved before
/// assembly. An assembly that enables payout passes them as
/// `PipelinePayoutConfig::controls`, and `assemble_ingest_pipeline_runtime`
/// refuses one that does not (Zaki review 1, round 2, finding 2).
pub struct IngestPipelineRuntimeContext {
    pub backend: Arc<PgBackend>,
    pub artifact_store: Arc<dyn TraceArtifactStore>,
    pub object_store_name: String,
    pub lease_config: PipelineLeaseConfig,
    pub near_contract_id: Option<String>,
    pub near_confirmation_interval: StdDuration,
    pub near_payout_controls: PipelineNearPayoutControls,
    pub novelty_utility_checks: PipelineNoveltyUtilityChecks,
    /// `main`'s gate configuration and `NoveltyUtility` delta, as ingest
    /// parsed them (multi-lens review L5-4, Zaki review 3, Z3-3; the
    /// index-insert threshold since Zaki review 1, round 2, finding 14). An
    /// assembly that binds the compatibility bundle builds its configuration
    /// from it (`CompatibilityBundleConfig::production_compatible`), and
    /// `assemble_ingest_pipeline_runtime` refuses one that does not hold it.
    pub main_gate: MainGateConfig,
}

/// Compile-time injection seam for a proprietary production pipeline
/// assembly. The stock binary intentionally has no implementation: the
/// scorer, embedder, vector index, settlement, and payout backends a
/// deployable pipeline needs do not live in this tree.
///
/// The index writer an assembly injects must return from every call well
/// within `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS` (60 s): Settle stops
/// starting index calls at its lease's end, and a withdrawal waits out that
/// margin before it removes the revision, so a call that outlives it could
/// write a withdrawn revision's entry after its removal (multi-lens review
/// L4-3).
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
/// whenever `tenants_processed` or `production_required` is true -- tenants
/// the worker processes (`pipeline_tenants_processed`: routed or drained) is
/// exactly the condition under which real receipts would otherwise be
/// scored, settled and paid by test doubles. This holds whether or not
/// `production_required` itself is set; `tenants_processed` alone is enough. `allow_test_dependencies` is the only way past that refusal, is
/// meant for tests and local development only, and never combines with
/// `production_required` -- both set refuses startup at once with
/// `pipeline_test_dependencies_not_allowed_when_required`, regardless of
/// qualification. The caller resolves both booleans from the environment (or
/// from the tenant rollout gates); this function reads neither directly.
/// `near_contract_id` is `main`'s configured NEAR credit contract,
/// `near_confirmation_interval` its NEAR outbox scheduler cadence,
/// `near_payout_controls` its NEAR settlement mode and adapter-auth
/// requirement, and `novelty_utility_checks` the configuration of `main`'s
/// `NoveltyUtility` credit checks, all handed to the assembly in its context.
#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_ingest_pipeline_runtime(
    assembler: Option<&dyn IngestPipelineRuntimeAssembler>,
    db_connections: Option<&TraceCorpusDbConnections>,
    artifact_store: Option<&ConfiguredTraceArtifactStore>,
    production_required: bool,
    lease_config: PipelineLeaseConfig,
    tenants_processed: bool,
    allow_test_dependencies: bool,
    near_contract_id: Option<&str>,
    near_confirmation_interval: StdDuration,
    near_payout_controls: PipelineNearPayoutControls,
    novelty_utility_checks: &PipelineNoveltyUtilityChecks,
    main_gate: MainGateConfig,
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
        near_payout_controls,
        novelty_utility_checks: novelty_utility_checks.clone(),
        main_gate,
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
    // Zaki review 1, round 2, finding 2: an enabled payout follows `main`'s
    // NEAR settlement mode and adapter-auth requirement, never ones the
    // assembly picked itself.
    anyhow::ensure!(
        !service.payout_enabled() || service.payout_controls() == Some(near_payout_controls),
        "pipeline_runtime_near_payout_controls_mismatch"
    );
    // Multi-lens review L5-4 and Zaki review 3, Z3-3 (and Zaki review 1,
    // round 2, finding 14): a compatibility bundle holds `main`'s gate
    // configuration -- floors, index-insert threshold, top-k, chunk knobs and
    // the `NoveltyUtility` delta -- never values the assembly picked.
    anyhow::ensure!(
        service
            .compatibility_config()
            .is_none_or(|config| config.matches_main_gate(&main_gate)),
        "pipeline_runtime_main_gate_config_mismatch"
    );
    // Ruling T15-12: a compatibility award applies `main`'s NoveltyUtility
    // credit checks with the configuration ingest was started with, never a
    // looser one an assembly picked itself or dropped.
    anyhow::ensure!(
        service.novelty_utility_checks() == novelty_utility_checks,
        "pipeline_runtime_novelty_utility_checks_mismatch"
    );
    // Zaki review 1, round 2, finding 15: a compatibility award's ledger
    // row names the pipeline's issuer, as `main`'s names its issuing gate
    // worker. A runtime that processes a tenant through the compatibility
    // bundle cannot run without one.
    anyhow::ensure!(
        !(tenants_processed
            && service.binds_compatibility_bundle()
            && novelty_utility_checks.issuer_principal_ref.is_none()),
        "pipeline_credit_issuer_principal_missing"
    );
    if (production_required || tenants_processed)
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

/// Refuses, with `pipeline_privacy_filter_required`, a pipeline runtime
/// whose privacy boundary does not classify prose PII while `main` requires
/// prose-PII filtering (`TRACE_COMMONS_REQUIRE_PRIVACY_FILTER`), as `main`
/// refuses to start with no filter backend. Judged by what the boundary does
/// (`PipelinePrivacyBoundary::classifies_prose_pii`), not by whether it
/// reports itself production-qualified (Zaki review 1, round 2, finding 21).
pub(crate) fn validate_pipeline_privacy_filter_requirement(
    require_privacy_filter: bool,
    service: &PipelineService,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !require_privacy_filter || service.privacy_classifies_prose_pii(),
        "pipeline_privacy_filter_required"
    );
    Ok(())
}

/// Whether every dependency the default bundle actually uses is
/// production-qualified (decision P4-D7).
///
/// Scoped to `PipelineService::bundle_qualification` for
/// `service.default_package()`, not every dependency the service holds: a
/// held-but-unnamed scorer, embedder, or settlement adapter the default
/// bundle never touches no longer blocks startup. What the default bundle
/// does use -- its named scorer and embedder, the held index reader and
/// writer, one settlement adapter per instrument it pins, authority, privacy
/// (Ruling T2-2) -- and the payout adapter whenever payout is enabled
/// (service-wide: payout pays every bundle's runs, final review M5) still
/// fail closed the same way, whenever tenants are routed; an invalid or
/// unresolvable default package (`bundle_package_invalid`,
/// `bundle_dependency_missing`) fails closed the same as an unqualified one.
/// A compatibility bundle's configuration must also be qualifiable (Zaki
/// review 1, round 2, finding 11), so the all-zero local reference never
/// binds for real tenants: that is the bundle qualification's own
/// configuration term (`PipelineBundleQualification::configuration_qualifiable`,
/// `bundle_configuration_not_qualifiable`), the same one `qualify_bundle`
/// sees, so startup reads no separate flag for it.
pub(crate) fn pipeline_runtime_is_production_qualified(service: &PipelineService) -> bool {
    service
        .bundle_qualification(service.default_package())
        .is_ok_and(|qualification| qualification.is_production_qualified())
}

/// Label-only readiness body. `reason` is present only when `status` is
/// `"not_ready"`, so a ready response has no dangling null field.
/// `drain_tenant_count` is the size of `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS`,
/// the tenants the worker drains without routing their receipts (Zaki
/// review 1, item 7): a count, never a tenant id.
#[derive(Debug, Serialize)]
pub(crate) struct PipelineReadinessResponse {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    drain_tenant_count: usize,
}

/// Answers `GET /v1/pipeline/readiness`. Unauthenticated and registered in
/// `app()` beside `/health`, outside every auth layer, for the same reason:
/// an orchestrator's readiness probe runs before any credential exists to
/// present. The body never carries a tenant id, run id, or error text --
/// only the worker's own boolean state, named by a fixed label.
pub(crate) async fn pipeline_readiness_handler(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<PipelineReadinessResponse>) {
    let drain_tenant_count = state.pipeline_drain_tenant_ids.len();
    if state.pipeline_service.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(PipelineReadinessResponse {
                status: "not_ready",
                reason: Some("pipeline_runtime_absent"),
                drain_tenant_count,
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
                drain_tenant_count,
            }),
        );
    }
    (
        StatusCode::OK,
        Json(PipelineReadinessResponse {
            status: "ready",
            reason: None,
            drain_tenant_count,
        }),
    )
}

/// `POST /v1/workers/pipeline/index-rebuild`: rebuilds the injected pipeline
/// runtime's vector index for the caller's tenant from the sealed index
/// commands Settle already committed -- no new outcome, no policy
/// evaluation, and no credit (`PipelineService::rebuild_index_from_authoritative_commands`).
/// Meant for after a restore, once the pipeline's rows are back but the
/// vector index is a fresh, empty store.
///
/// Sits behind the same vector worker credential as `vector_index_handler`
/// (`/v1/workers/vector-index`) -- an admin token or a bearer token scoped
/// `TokenRole::VectorWorker` -- and copies that route's authentication shape
/// exactly: `authenticate_with_tenant_access_grant` then
/// `require_vector_operator`. Without an injected pipeline runtime
/// (`state.pipeline_service`), it returns 404, the same refusal the
/// pipeline review routes use for the same reason: there is nothing to
/// rebuild.
///
/// The tenant is the authenticated credential's own tenant, never a request
/// field -- the same tenant-scoping rule every other pipeline route follows
/// (Envelope tenant fields are attribution only; auth derives the tenant
/// that is actually read and written).
///
/// A rebuild that completes appends one index maintenance audit row, as the
/// vector index worker route does (final review M4, ruling FR-7): `main`'s
/// `vector_index` event with the fixed purpose `pipeline_index_rebuild`
/// (hashed) and the report's counts under their own labels. Hash-only and
/// label-only: no run, submission, or command id. It needs no migration:
/// the event, its action, and its metadata shape are `main`'s, as the
/// pipeline's invalidation requeue route already uses them.
///
/// The rebuild and its audit row run in a task of their own, which this
/// handler awaits (review of the follow-up wave, m1). A client that
/// disconnects drops this handler's future, but not that task: each run's
/// writes still finish under the run and submission locks its transaction
/// holds, and the audit row is still appended. That task is not tracked: the
/// graceful shutdown drains open connections only, so it does not wait for a
/// rebuild whose client has gone, and the runtime drops that task when the
/// process exits. That exit, or a lost database session, can still release
/// a run's locks while its writes go on; see
/// `PipelineService::rebuild_index_run`.
pub(crate) async fn pipeline_index_rebuild_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<PipelineIndexRebuildReport>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_vector_operator(&tenant)?;
    require_pipeline_service(state.as_ref())?;
    tokio::spawn(rebuild_index_and_audit(state, tenant))
        .await
        .map_err(|_| internal_error("pipeline_index_rebuild_task_failed"))?
}

/// `pipeline_index_rebuild_handler`'s rebuild and audit row, for the
/// authenticated `tenant`, as one task.
async fn rebuild_index_and_audit(
    state: Arc<AppState>,
    tenant: TenantAuth,
) -> ApiResult<Json<PipelineIndexRebuildReport>> {
    let pipeline_service = require_pipeline_service(state.as_ref())?;
    let writer = pipeline_service.index_writer();
    let report = pipeline_service
        .rebuild_index_from_authoritative_commands(&tenant.tenant_id, writer)
        .await
        .map_err(pipeline_index_rebuild_error)?;
    let purpose = "pipeline_index_rebuild";
    let count = |value: usize| u32::try_from(value).unwrap_or(u32::MAX);
    let action_counts = BTreeMap::from([
        (
            "pipeline_index_commands_replayed".to_string(),
            count(report.command_count),
        ),
        (
            "pipeline_index_entries_written".to_string(),
            count(report.entry_count),
        ),
        (
            "pipeline_index_entries_unchanged".to_string(),
            count(report.unchanged_entry_count),
        ),
        (
            "pipeline_index_runs_skipped".to_string(),
            count(report.skipped_run_count),
        ),
    ]);
    append_audit_event_with_db_mirror(
        state.as_ref(),
        &tenant,
        TraceCommonsAuditEvent::vector_index(&tenant, false, Some(purpose), action_counts.clone()),
        StorageTraceAuditAction::VectorIndex,
        StorageTraceAuditSafeMetadata::Maintenance {
            surface: Some("vector_index".to_string()),
            purpose_hash: Some(sha256_prefixed(purpose)),
            dry_run: false,
            action_counts,
        },
    )
    .await
    .map_err(internal_error)?;
    tracing::info!(
        tenant_storage_ref = %tenant_storage_ref(&tenant.tenant_id),
        command_count = report.command_count,
        entry_count = report.entry_count,
        skipped_run_count = report.skipped_run_count,
        "pipeline index rebuilt from sealed commands"
    );
    Ok(Json(report))
}

/// Maps `rebuild_index_from_authoritative_commands`'s anyhow errors to their
/// HTTP shape. `index_command_invalid` -- a sealed command that failed
/// validation against its run or its own committed Score evidence -- is
/// surfaced as 409 Conflict, a state of the store rather than a transient
/// service fault, and `index_unavailable` -- a run's writes that passed
/// their deadline -- as 503. Everything else falls back to the generic
/// hash-only internal error.
fn pipeline_index_rebuild_error(error: anyhow::Error) -> (StatusCode, Json<ApiError>) {
    if error.to_string() == "index_command_invalid" {
        return api_error(StatusCode::CONFLICT, "index_command_invalid");
    }
    // Merge review I1: a run's writes passed their deadline (the index is
    // slow or down); the rows were freed and a rerun is safe.
    if error.to_string()
        == trace_commons_server::versioned_pipeline::PIPELINE_INDEX_UNAVAILABLE_LABEL
    {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            trace_commons_server::versioned_pipeline::PIPELINE_INDEX_UNAVAILABLE_LABEL,
        );
    }
    internal_error(error)
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

/// How many pipeline phase attempt artifacts the worker sweeps for one
/// tenant per pass (`PipelineService::sweep_attempt_artifacts_from`), right
/// after the receipt sweep, examining at most
/// `PIPELINE_ATTEMPT_SWEEP_EXAMINED_PER_REMOVAL` times as many due rows. The
/// rest wait for the next pass, which resumes where this one stopped.
pub(crate) const PIPELINE_WORKER_MAX_SWEPT_ATTEMPT_ARTIFACTS_PER_TENANT: usize = 32;

/// How many of one tenant's due index invalidations the worker processes
/// each time it runs the tenant's invalidation step
/// (`PipelineService::process_index_invalidations`), right after draining
/// its runs. A step that used the whole limit runs again on the next pass
/// (`PipelineFollowUpCadence::run_again`).
const PIPELINE_WORKER_MAX_INDEX_INVALIDATIONS_PER_TENANT: usize = 32;

/// How many of one tenant's complete runs the worker pays out each time it
/// runs the tenant's payout step (`PipelineService::process_payouts`), after
/// its index invalidations. A step that used the whole limit runs again on
/// the next pass.
const PIPELINE_WORKER_MAX_PAYOUTS_PER_TENANT: usize = 32;

/// How often the worker runs a tenant's index invalidation step when nothing
/// woke it (Zaki review 1, round 2, item 4). Invalidations are queued only
/// by a withdrawal, a cancelled index write, or a requeue, and each of those
/// wakes the step at once in the process that queued it; the interval
/// bounds the wait for one another replica queued, a retry's backoff, and a
/// claim whose lease passed.
const PIPELINE_WORKER_INDEX_INVALIDATION_INTERVAL: StdDuration = StdDuration::from_secs(10);

/// How often the worker runs a tenant's credit audit step when nothing woke
/// it (Zaki review 1, round 2, N-5). A Trace Credit leg that writes a
/// credit event wakes the step at once in the process that settled it; the
/// interval bounds the wait for one another replica settled, and for a pass
/// that failed to append.
const PIPELINE_WORKER_CREDIT_AUDIT_INTERVAL: StdDuration = StdDuration::from_secs(10);

/// The most `CreditMutate` audit events the worker appends for one tenant
/// in one pass.
const PIPELINE_WORKER_MAX_CREDIT_AUDITS_PER_TENANT: usize = 32;

/// A follow-up step of a tenant's drain, after its runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum PipelineFollowUpStep {
    IndexInvalidations,
    Payouts,
    CreditAudits,
}

/// When the worker last ran each tenant's follow-up steps, so an idle
/// tenant costs no invalidation or payout query on most passes (Zaki review
/// 1, round 2, item 4). One per worker loop; the run drain and the
/// staged-receipt sweep are not scheduled here and run on every pass. It
/// also keeps where each tenant's attempt sweep stopped, so the next pass
/// resumes there (wave 2; follow-up review, m2).
#[derive(Debug, Default)]
pub(crate) struct PipelineFollowUpCadence {
    last_run: HashMap<(String, PipelineFollowUpStep), std::time::Instant>,
    attempt_sweep_resume: HashMap<String, AttemptSweepCursor>,
}

impl PipelineFollowUpCadence {
    /// Where `tenant_id`'s next attempt sweep pass resumes; `None` to start
    /// at its oldest due row.
    pub(crate) fn attempt_sweep_resume_after(&self, tenant_id: &str) -> Option<AttemptSweepCursor> {
        self.attempt_sweep_resume.get(tenant_id).cloned()
    }

    /// Records where `tenant_id`'s last attempt sweep pass said the next
    /// one resumes.
    pub(crate) fn record_attempt_sweep(
        &mut self,
        tenant_id: &str,
        resume_after: Option<AttemptSweepCursor>,
    ) {
        match resume_after {
            Some(cursor) => {
                self.attempt_sweep_resume
                    .insert(tenant_id.to_string(), cursor);
            }
            None => {
                self.attempt_sweep_resume.remove(tenant_id);
            }
        }
    }

    /// The follow-up steps that run for `tenant_id` on the pass at `now`,
    /// each recorded as run at `now`. A step runs when `woken` names it
    /// (this process queued work for it: `PipelineService::take_follow_ups`),
    /// on the tenant's first pass, and once its interval has passed since it
    /// last ran: `PIPELINE_WORKER_INDEX_INVALIDATION_INTERVAL` for the
    /// invalidation step, and `payout_interval` -- the payout's confirmation
    /// interval, `main`'s NEAR cadence -- for the payout step, which never
    /// runs while `payout_interval` is `None` (payout disabled).
    pub(crate) fn due_steps(
        &mut self,
        tenant_id: &str,
        woken: PipelineFollowUps,
        payout_interval: Option<StdDuration>,
        now: std::time::Instant,
    ) -> PipelineFollowUps {
        PipelineFollowUps {
            index_invalidations: self.take_due(
                tenant_id,
                PipelineFollowUpStep::IndexInvalidations,
                woken.index_invalidations,
                PIPELINE_WORKER_INDEX_INVALIDATION_INTERVAL,
                now,
            ),
            payouts: payout_interval.is_some_and(|interval| {
                self.take_due(
                    tenant_id,
                    PipelineFollowUpStep::Payouts,
                    woken.payouts,
                    interval,
                    now,
                )
            }),
            credit_audits: self.take_due(
                tenant_id,
                PipelineFollowUpStep::CreditAudits,
                woken.credit_audits,
                PIPELINE_WORKER_CREDIT_AUDIT_INTERVAL,
                now,
            ),
        }
    }

    fn take_due(
        &mut self,
        tenant_id: &str,
        step: PipelineFollowUpStep,
        woken: bool,
        interval: StdDuration,
        now: std::time::Instant,
    ) -> bool {
        let key = (tenant_id.to_string(), step);
        let due = woken
            || self
                .last_run
                .get(&key)
                .is_none_or(|last_run| now.saturating_duration_since(*last_run) >= interval);
        if due {
            self.last_run.insert(key, now);
        }
        due
    }

    /// `step` ran for `tenant_id` and used its whole limit, so work may be
    /// left: it runs again on the next pass.
    pub(crate) fn run_again(&mut self, tenant_id: &str, step: PipelineFollowUpStep) {
        self.last_run.remove(&(tenant_id.to_string(), step));
    }
}

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
/// Then, whatever the runs did, it runs the follow-up steps `cadence` finds
/// due (`PipelineFollowUpCadence::due_steps`, with the steps this service
/// woke): it releases the tenant's parked runs whose submission is no
/// longer operable (`release_inoperable_parked_runs`), processes up to
/// `PIPELINE_WORKER_MAX_INDEX_INVALIDATIONS_PER_TENANT` of the tenant's due
/// index invalidations (`process_index_invalidations`, which removes a
/// withdrawn or cancelled revision from the index), and pays out up to
/// `PIPELINE_WORKER_MAX_PAYOUTS_PER_TENANT` of the tenant's complete runs
/// (`process_payouts`; Ruling S7 puts the invalidations right after the runs
/// and the payouts after them). Last, on every pass, it sweeps up to
/// `PIPELINE_WORKER_MAX_SWEPT_RECEIPTS_PER_TENANT` of the tenant's receipt
/// attempts that never committed: each staged object whose
/// row's `cleanup_after` has passed is deleted with its row. Then, also on
/// every pass, it sweeps up to
/// `PIPELINE_WORKER_MAX_SWEPT_ATTEMPT_ARTIFACTS_PER_TENANT` of the tenant's
/// pipeline phase attempt objects
/// (`PipelineService::sweep_attempt_artifacts`): the same shape, for the
/// Review and Score objects a phase attempt writes under its own lease
/// token and never commits. A committed attempt's objects are object refs
/// of the submission, which a withdrawal queues for `main`'s
/// revocation-propagation worker to delete. An invalidation, payout, or
/// sweep failure is logged the same way, and the drain goes on to the next
/// step. All of it runs in the pass's supervised task for the tenant
/// (`run_pipeline_worker_pass`).
pub(crate) async fn drain_pipeline_tenant(
    state: Arc<AppState>,
    service: Arc<PipelineService>,
    tenant_id: String,
    cadence: Arc<std::sync::Mutex<PipelineFollowUpCadence>>,
) {
    for _ in 0..PIPELINE_WORKER_MAX_RUNS_PER_TENANT {
        match service.process_one(&tenant_id).await {
            // Multi-lens review L3-2: another Score holds this tenant's Score
            // lock, so this replica moves on to its other tenants for the pass.
            Ok(Some(run))
                if run.last_error_label.as_deref()
                    == Some(
                        trace_commons_server::versioned_pipeline::PIPELINE_SCORE_LOCK_BUSY_LABEL,
                    ) =>
            {
                break;
            }
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
    let lock_cadence = || {
        cadence
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    let due = lock_cadence().due_steps(
        &tenant_id,
        service.take_follow_ups(&tenant_id),
        service.payout_confirmation_interval(),
        std::time::Instant::now(),
    );
    if due.index_invalidations {
        // A run parked for review whose submission expired, was purged, or
        // was revoked or withdrawn without the pipeline's follow-up is
        // reached by nothing else (Zaki review 1, round 2, finding 9).
        if let Err(error) = service.release_inoperable_parked_runs(&tenant_id).await {
            tracing::warn!(
                error_class = "pipeline_worker_parked_run_release_failed",
                tenant_storage_ref = %tenant_storage_ref(&tenant_id),
                error_hash = %safe_display_error_hash(&error),
                "pipeline worker parked run release failed"
            );
        }
        match service
            .process_index_invalidations(
                &tenant_id,
                PIPELINE_WORKER_MAX_INDEX_INVALIDATIONS_PER_TENANT,
            )
            .await
        {
            Ok(processed) => {
                if processed >= PIPELINE_WORKER_MAX_INDEX_INVALIDATIONS_PER_TENANT {
                    lock_cadence().run_again(&tenant_id, PipelineFollowUpStep::IndexInvalidations);
                }
            }
            Err(error) => {
                tracing::warn!(
                    error_class = "pipeline_worker_index_invalidation_failed",
                    tenant_storage_ref = %tenant_storage_ref(&tenant_id),
                    error_hash = %safe_display_error_hash(&error),
                    "pipeline worker index invalidation failed"
                );
            }
        }
    }
    if due.credit_audits {
        match append_pipeline_credit_audit_events(
            state.as_ref(),
            service.as_ref(),
            &tenant_id,
            PIPELINE_WORKER_MAX_CREDIT_AUDITS_PER_TENANT,
        )
        .await
        {
            Ok(appended) => {
                if appended >= PIPELINE_WORKER_MAX_CREDIT_AUDITS_PER_TENANT {
                    lock_cadence().run_again(&tenant_id, PipelineFollowUpStep::CreditAudits);
                }
            }
            Err(error) => {
                tracing::warn!(
                    error_class = "pipeline_worker_credit_audit_failed",
                    tenant_storage_ref = %tenant_storage_ref(&tenant_id),
                    error_hash = %safe_display_error_hash(&error),
                    "pipeline worker credit audit failed"
                );
            }
        }
    }
    if due.payouts {
        match service
            .process_payouts(&tenant_id, PIPELINE_WORKER_MAX_PAYOUTS_PER_TENANT)
            .await
        {
            Ok(processed) => {
                if processed >= PIPELINE_WORKER_MAX_PAYOUTS_PER_TENANT {
                    lock_cadence().run_again(&tenant_id, PipelineFollowUpStep::Payouts);
                }
            }
            Err(error) => {
                tracing::warn!(
                    error_class = "pipeline_worker_payout_failed",
                    tenant_storage_ref = %tenant_storage_ref(&tenant_id),
                    error_hash = %safe_display_error_hash(&error),
                    "pipeline worker payout failed"
                );
            }
        }
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
    let resume_after = lock_cadence().attempt_sweep_resume_after(&tenant_id);
    match service
        .sweep_attempt_artifacts_from(
            &tenant_id,
            PIPELINE_WORKER_MAX_SWEPT_ATTEMPT_ARTIFACTS_PER_TENANT,
            resume_after,
        )
        .await
    {
        Ok(pass) => lock_cadence().record_attempt_sweep(&tenant_id, pass.resume_after),
        Err(error) => {
            tracing::warn!(
                error_class = "pipeline_worker_attempt_sweep_failed",
                tenant_storage_ref = %tenant_storage_ref(&tenant_id),
                error_hash = %safe_display_error_hash(&error),
                "pipeline worker attempt sweep failed"
            );
        }
    }
}

/// Zaki review 1, round 2, N-5: appends `main`'s hash-only `CreditMutate`
/// audit event for each credit event the tenant's Trace Credit legs wrote
/// to `main`'s ledger (`PgPipelineStore::list_unaudited_credit_events`), up
/// to `limit`, through `main`'s mirrored audit log, as `main`'s credit paths
/// append one after each credit event they write
/// (`append_automatic_utility_credit_events_once_with_counts`), and then
/// marks each leg audited. The event carries the credit event's id, so a
/// pass that stopped after the append and before the mark finds the event
/// already in the database and only marks the leg. Its actor is the ledger
/// row's: for a `NoveltyUtility` event the pipeline's issuer, in the role
/// `main` records for its gate worker; an actor whose role is not a token
/// role (a minimal-family `accepted` event's `pipeline_worker`) is recorded
/// as the in-process pipeline worker, role `system`. Returns how many legs
/// it handled.
pub(crate) async fn append_pipeline_credit_audit_events(
    state: &AppState,
    service: &PipelineService,
    tenant_id: &str,
    limit: usize,
) -> anyhow::Result<usize> {
    let items = service
        .store()
        .list_unaudited_credit_events(tenant_id, limit)
        .await?;
    for item in &items {
        let already_appended = match state.db_mirror.as_ref() {
            Some(db) => db
                .get_trace_audit_event_by_id(tenant_id, item.credit_event_id)
                .await?
                .is_some(),
            None => false,
        };
        if !already_appended {
            let points = item
                .points_delta
                .parse::<f32>()
                .map_err(|_| anyhow::anyhow!("pipeline_credit_points_invalid"))?;
            let (actor, actor_role_label) = match serde_json::from_value::<TokenRole>(
                serde_json::Value::String(item.actor_role.clone()),
            ) {
                Ok(role) => (
                    TenantAuth {
                        role,
                        principal_ref: item.actor_principal_ref.clone(),
                        ..system_audit_tenant(tenant_id, PIPELINE_WORKER_AUDIT_ACTOR_REF)
                    },
                    None,
                ),
                Err(_) => (
                    system_audit_tenant(tenant_id, PIPELINE_WORKER_AUDIT_ACTOR_REF),
                    Some("system"),
                ),
            };
            let mut event = TraceCommonsAuditEvent::credit_mutation(
                &actor,
                item.submission_id,
                points,
                item.reason.as_deref(),
            );
            event.event_id = item.credit_event_id;
            append_audit_event_mirrored(
                state,
                &actor,
                event,
                AuditRowMirror {
                    action: StorageTraceAuditAction::CreditMutate,
                    metadata: StorageTraceAuditSafeMetadata::CreditMutation {
                        event_type: item.event_type,
                        credit_points_delta_micros: credit_delta_micros(points),
                        reason_hash: sha256_prefixed(item.reason.as_deref().unwrap_or_default()),
                        external_ref_hash: item.external_ref.as_deref().map(sha256_prefixed),
                    },
                    object_ref_id: None,
                    actor_role_label,
                },
                "pipeline credit audit event",
            )
            .await?;
        }
        service.store().mark_credit_audited(tenant_id, item).await?;
    }
    Ok(items.len())
}

/// The actor label of an audit event the pipeline worker appends as an
/// in-process driver (`system_audit_tenant`).
const PIPELINE_WORKER_AUDIT_ACTOR_REF: &str = "pipeline_worker";

/// Whether the pipeline worker processes any tenant: one routed to the
/// pipeline (`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS`) or on the drain
/// list (`TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS`). The worker drains a
/// drain tenant's runs, ledger credit and payouts through the runtime's own
/// dependencies, so the fail-closed qualification gate counts both lists
/// (Zaki review 1, round 2, finding 3).
pub(crate) fn pipeline_tenants_processed(
    tenant_rollout_gates: &TraceTenantRolloutGates,
    drain_tenant_ids: &BTreeSet<String>,
) -> bool {
    tenant_rollout_gates.tenant_count(TraceTenantRolloutFeature::PipelineReceipts) > 0
        || !drain_tenant_ids.is_empty()
}

/// Multi-lens review L5-2 and Zaki review 3, Z3-2: before ingest serves,
/// runs the default package's startup checks on every bundle a worker may
/// run for a routed or drained tenant -- its active bundle and the bundle of
/// each run in flight (`PipelineService::check_tenant_bundles`) -- and
/// refuses to start on the first failure, under its label. A tenant keeps
/// its active bundle when the default package changes, so the assembly's
/// checks of the default package alone do not cover it.
pub(crate) async fn validate_pipeline_tenant_bundles(
    service: &PipelineService,
    tenant_rollout_gates: &TraceTenantRolloutGates,
    drain_tenant_ids: &BTreeSet<String>,
    main_gate: &MainGateConfig,
    allow_test_dependencies: bool,
) -> anyhow::Result<()> {
    let mut tenant_ids =
        tenant_rollout_gates.tenant_ids(TraceTenantRolloutFeature::PipelineReceipts);
    tenant_ids.extend(drain_tenant_ids.iter().cloned());
    for tenant_id in tenant_ids {
        service
            .check_tenant_bundles(&tenant_id, main_gate, !allow_test_dependencies)
            .await?;
    }
    Ok(())
}

/// The tenants the pipeline worker drains on each pass, each once, in order:
/// the tenants whose receipts are routed to the pipeline
/// (`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS`) and the drain list
/// (`TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS`). A tenant rolled back off the
/// first list onto the second keeps its runs in flight, index
/// invalidations, payouts and confirmations, and staged receipt sweeps
/// processed, while no receipt of its is routed (Zaki review 1, item 7).
pub(crate) fn pipeline_worker_tenant_ids(state: &AppState) -> Vec<String> {
    let mut tenant_ids = state
        .tenant_rollout_gates
        .tenant_ids(TraceTenantRolloutFeature::PipelineReceipts);
    tenant_ids.extend(state.pipeline_drain_tenant_ids.iter().cloned());
    tenant_ids.into_iter().collect()
}

/// Starts the owned pipeline worker loop. `None` when no pipeline runtime is
/// injected -- there is nothing to drain, and the repository binary injects
/// none.
///
/// Each iteration runs one `run_pipeline_worker_pass` over
/// `pipeline_worker_tenant_ids` (the `PipelineReceipts` rollout tenants and
/// the drain list, read once at start), then sleeps
/// `PIPELINE_WORKER_POLL_INTERVAL` or until `stop` fires.
fn spawn_pipeline_worker(state: Arc<AppState>) -> Option<PipelineWorkerHandle> {
    let service = state.pipeline_service.clone()?;
    let tenant_ids = pipeline_worker_tenant_ids(&state);
    let cadence = Arc::new(std::sync::Mutex::new(PipelineFollowUpCadence::default()));
    let ready = state.pipeline_worker_ready.clone();
    let worker_ready = ready.clone();
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        while !*stop_rx.borrow() {
            let probe_service = service.clone();
            let drain_service = service.clone();
            run_pipeline_worker_pass(
                async move { probe_service.readiness().await },
                tenant_ids.clone(),
                |tenant_id| {
                    drain_pipeline_tenant(
                        state.clone(),
                        drain_service.clone(),
                        tenant_id,
                        cadence.clone(),
                    )
                },
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
