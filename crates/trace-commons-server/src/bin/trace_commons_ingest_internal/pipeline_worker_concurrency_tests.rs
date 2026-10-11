// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `TRACE_COMMONS_PIPELINE_WORKER_CONCURRENCY`: the worker's concurrent
//! supervisor and tenant lanes (`pipeline_runtime::run_concurrent_pipeline_worker`,
//! `pipeline_runtime::drain_pipeline_tenant_runs_concurrently`), first with
//! test doubles for the steps and drains, then end to end against
//! PostgreSQL through `pipeline_runtime::spawn_pipeline_worker`.

use super::*;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use super::pipeline_http_pg_tests::{
    account_owner_backend, assemble_compatibility_pipeline_service_with, count_outcomes,
    credit_mutate_audit_rows, local_artifacts, mains_database, model_training_envelope,
    runtime_backend, tenant_tx, wait_for_run_state,
};
use crate::pipeline_default_routing::PipelineWorkerPassStats;
use crate::pipeline_runtime::{
    PIPELINE_WORKER_CONCURRENCY_INVALID_LABEL, PipelineWorkerStep,
    drain_pipeline_tenant_runs_concurrently, parse_pipeline_worker_concurrency,
    run_concurrent_pipeline_worker,
};
use trace_commons_gate_api::SettlementAdapter;
use trace_commons_gate_api::pipeline::InstrumentId;
use trace_commons_server::versioned_pipeline_authority::PipelinePrivacyBoundary;
use trace_commons_server::versioned_pipeline_bundle::MINIMAL_INDEX_ID;
use trace_commons_server::versioned_pipeline_credit::RecordingSettlementAdapter;
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;

// ---------------------------------------------------------------------------
// The setting.
// ---------------------------------------------------------------------------

/// Unset or blank is 1 (the sequential pass); 1 to 16 is itself; anything
/// else refuses startup under its label, never echoing the value.
#[test]
fn the_worker_concurrency_is_one_to_sixteen_and_refuses_anything_else() {
    assert_eq!(parse_pipeline_worker_concurrency(None).unwrap(), 1);
    assert_eq!(parse_pipeline_worker_concurrency(Some("")).unwrap(), 1);
    assert_eq!(parse_pipeline_worker_concurrency(Some("  ")).unwrap(), 1);
    assert_eq!(parse_pipeline_worker_concurrency(Some("1")).unwrap(), 1);
    assert_eq!(parse_pipeline_worker_concurrency(Some(" 4 ")).unwrap(), 4);
    assert_eq!(parse_pipeline_worker_concurrency(Some("16")).unwrap(), 16);
    for invalid in [
        "0",
        "17",
        "-1",
        "abc",
        "2.5",
        "4 workers",
        "99999999999999999999999",
    ] {
        let error = parse_pipeline_worker_concurrency(Some(invalid))
            .expect_err("an invalid concurrency refuses startup");
        assert_eq!(
            error.to_string(),
            PIPELINE_WORKER_CONCURRENCY_INVALID_LABEL,
            "{invalid:?}"
        );
    }
}

/// The env reader refuses startup the same way, and config-status reports
/// the configured value.
#[test]
fn an_invalid_worker_concurrency_variable_refuses_startup() {
    // SAFETY: env mutation in tests is OK here -- this variable is read
    // only by `parse_pipeline_worker_concurrency_from_env`, which nothing
    // else in this suite calls.
    unsafe { std::env::set_var(TRACE_COMMONS_PIPELINE_WORKER_CONCURRENCY, "0") };
    let refused = parse_pipeline_worker_concurrency_from_env();
    unsafe { std::env::set_var(TRACE_COMMONS_PIPELINE_WORKER_CONCURRENCY, "3") };
    let accepted = parse_pipeline_worker_concurrency_from_env();
    unsafe { std::env::remove_var(TRACE_COMMONS_PIPELINE_WORKER_CONCURRENCY) };
    let unset = parse_pipeline_worker_concurrency_from_env();
    assert_eq!(unset.unwrap(), 1);
    assert_eq!(
        refused.expect_err("0 refuses startup").to_string(),
        PIPELINE_WORKER_CONCURRENCY_INVALID_LABEL
    );
    assert_eq!(accepted.unwrap(), 3);

    let dir = tempfile::tempdir().unwrap();
    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        None,
        None,
        false,
        false,
        false,
        false,
    );
    Arc::make_mut(&mut state).pipeline_worker_concurrency = 3;
    let status = trace_commons_config_status_response(state.as_ref());
    assert_eq!(status.pipeline_worker_concurrency, 3);
}

// ---------------------------------------------------------------------------
// Tenant lanes, with a test double for `pipeline_worker_step`.
// ---------------------------------------------------------------------------

/// How many test-double steps are running, and the most that ever ran at
/// once.
#[derive(Default)]
struct InFlight {
    now: AtomicUsize,
    max: AtomicUsize,
    calls: AtomicUsize,
}

impl InFlight {
    fn enter(&self) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let now = self.now.fetch_add(1, Ordering::SeqCst) + 1;
        self.max.fetch_max(now, Ordering::SeqCst);
    }

    fn leave(&self) {
        self.now.fetch_sub(1, Ordering::SeqCst);
    }
}

/// A tenant with a backlog runs as many steps at once as the permit pool
/// allows, and its lanes together claim at most the per-tenant budget of 32
/// (`PIPELINE_WORKER_MAX_RUNS_PER_TENANT`), as the sequential drain does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tenant_lanes_share_the_permit_pool_and_one_budget() {
    let in_flight = Arc::new(InFlight::default());
    let permits = Arc::new(tokio::sync::Semaphore::new(3));
    let (_stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let step_in_flight = in_flight.clone();
    let advanced = drain_pipeline_tenant_runs_concurrently(
        "tenant-a".to_string(),
        8,
        permits,
        stop_rx,
        move |_tenant_id| {
            let in_flight = step_in_flight.clone();
            async move {
                in_flight.enter();
                tokio::time::sleep(StdDuration::from_millis(20)).await;
                in_flight.leave();
                PipelineWorkerStep::Advanced
            }
        },
    )
    .await;
    assert_eq!(advanced, 32, "the lanes stop at the tenant's budget");
    assert_eq!(in_flight.calls.load(Ordering::SeqCst), 32);
    assert_eq!(
        in_flight.max.load(Ordering::SeqCst),
        3,
        "eight lanes run at most the pool's three permits at once"
    );
}

/// An idle tenant costs one claim: the first lane finds nothing due and the
/// waiting lanes never claim.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_idle_tenant_costs_one_claim() {
    let in_flight = Arc::new(InFlight::default());
    let (_stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let step_in_flight = in_flight.clone();
    let advanced = drain_pipeline_tenant_runs_concurrently(
        "tenant-a".to_string(),
        4,
        Arc::new(tokio::sync::Semaphore::new(4)),
        stop_rx,
        move |_tenant_id| {
            let in_flight = step_in_flight.clone();
            async move {
                in_flight.enter();
                in_flight.leave();
                PipelineWorkerStep::Idle
            }
        },
    )
    .await;
    assert_eq!(advanced, 0);
    assert_eq!(in_flight.calls.load(Ordering::SeqCst), 1);
}

/// A lane whose run found the Score lock busy ends alone; the other lane
/// drains the rest of the queue.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_busy_score_lock_ends_only_its_own_lane() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (_stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let step_calls = calls.clone();
    let advanced = drain_pipeline_tenant_runs_concurrently(
        "tenant-a".to_string(),
        2,
        Arc::new(tokio::sync::Semaphore::new(2)),
        stop_rx,
        move |_tenant_id| {
            let calls = step_calls.clone();
            async move {
                match calls.fetch_add(1, Ordering::SeqCst) + 1 {
                    1 => PipelineWorkerStep::ScoreLockBusy,
                    2..=10 => PipelineWorkerStep::Advanced,
                    _ => PipelineWorkerStep::Idle,
                }
            }
        },
    )
    .await;
    assert_eq!(
        advanced, 9,
        "the other lane went on after the busy one ended"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 11);
}

/// `stop` is honoured before every claim: a drain whose stop has fired
/// claims nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tenant_lanes_claim_nothing_once_stop_has_fired() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    stop_tx.send(true).unwrap();
    let step_calls = calls.clone();
    let advanced = drain_pipeline_tenant_runs_concurrently(
        "tenant-a".to_string(),
        4,
        Arc::new(tokio::sync::Semaphore::new(4)),
        stop_rx,
        move |_tenant_id| {
            let calls = step_calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                PipelineWorkerStep::Advanced
            }
        },
    )
    .await;
    assert_eq!(advanced, 0);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------------
// The supervisor, with test-double drains.
// ---------------------------------------------------------------------------

/// Polls `condition` every 10 ms for up to 10 s.
async fn eventually(what: &str, condition: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + StdDuration::from_secs(10);
    while !condition() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting until {what}"
        );
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
}

/// No pass barrier: while tenant A's drain is stuck, tenant B is drained
/// again and again. A tenant whose drain panics holds the worker not ready
/// on every tick until it is no longer listed, and `stop` ends the loop once
/// the drains in flight have finished.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_supervisor_drains_other_tenants_while_one_is_stuck() {
    let release_a = Arc::new(tokio::sync::Semaphore::new(0));
    let a_drains = Arc::new(AtomicUsize::new(0));
    let b_drains = Arc::new(AtomicUsize::new(0));
    let panicking = Arc::new(AtomicBool::new(true));
    let tenants = Arc::new(std::sync::Mutex::new(vec![
        "tenant-a".to_string(),
        "tenant-b".to_string(),
        "tenant-c".to_string(),
    ]));
    let ready = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(PipelineWorkerPassStats::default());
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

    let supervisor = {
        let (release_a, a_drains, b_drains, panicking, tenants) = (
            release_a.clone(),
            a_drains.clone(),
            b_drains.clone(),
            panicking.clone(),
            tenants.clone(),
        );
        let (ready, stats) = (ready.clone(), stats.clone());
        tokio::spawn(async move {
            run_concurrent_pipeline_worker(
                2,
                || async { Ok(()) },
                move || tenants.lock().unwrap().clone(),
                move |tenant_id, _permits| {
                    let (release_a, a_drains, b_drains, panicking) = (
                        release_a.clone(),
                        a_drains.clone(),
                        b_drains.clone(),
                        panicking.clone(),
                    );
                    async move {
                        match tenant_id.as_str() {
                            "tenant-a" => {
                                a_drains.fetch_add(1, Ordering::SeqCst);
                                let _ = release_a.acquire().await;
                            }
                            "tenant-b" => {
                                b_drains.fetch_add(1, Ordering::SeqCst);
                            }
                            _ => {
                                if panicking.load(Ordering::SeqCst) {
                                    panic!("test-only panic in a tenant drain");
                                }
                            }
                        }
                    }
                },
                ready,
                stats,
                stop_rx,
            )
            .await;
        })
    };

    eventually("tenant B was drained three times", || {
        b_drains.load(Ordering::SeqCst) >= 3
    })
    .await;
    assert_eq!(
        a_drains.load(Ordering::SeqCst),
        1,
        "a tenant never has two drains in flight"
    );
    // Tenant C panics on every tick: the worker stays not ready.
    for _ in 0..5 {
        assert!(
            !ready.load(Ordering::SeqCst),
            "a failing tenant holds ready"
        );
        tokio::time::sleep(StdDuration::from_millis(100)).await;
    }
    tenants
        .lock()
        .unwrap()
        .retain(|tenant| tenant != "tenant-c");
    eventually("the worker reports ready", || ready.load(Ordering::SeqCst)).await;
    // The tick stores `ready` before it records its figures.
    eventually("the tick's tenant count is 2", || {
        stats.tenant_count.load(Ordering::Relaxed) == 2
    })
    .await;
    eventually("the stuck drain's age shows in the duration", || {
        stats.duration_ms.load(Ordering::Relaxed) >= 500
    })
    .await;

    stop_tx.send(true).unwrap();
    tokio::time::sleep(StdDuration::from_millis(300)).await;
    assert!(
        !supervisor.is_finished(),
        "the supervisor waits for the drain in flight"
    );
    release_a.add_permits(1);
    tokio::time::timeout(StdDuration::from_secs(5), supervisor)
        .await
        .expect("the supervisor stops once its drains have")
        .unwrap();
}

/// A failing readiness probe reports not ready.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_supervisor_reports_not_ready_when_its_probe_fails() {
    let ready = Arc::new(AtomicBool::new(true));
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
    let supervisor = {
        let ready = ready.clone();
        tokio::spawn(run_concurrent_pipeline_worker(
            2,
            || async { Err(anyhow::anyhow!("probe failed (test double)")) },
            Vec::new,
            |_tenant_id, _permits| async {},
            ready,
            Arc::new(PipelineWorkerPassStats::default()),
            stop_rx,
        ))
    };
    eventually("the worker reports not ready", || {
        !ready.load(Ordering::SeqCst)
    })
    .await;
    stop_tx.send(true).unwrap();
    tokio::time::timeout(StdDuration::from_secs(5), supervisor)
        .await
        .unwrap()
        .unwrap();
}

// ---------------------------------------------------------------------------
// End to end, against PostgreSQL.
// ---------------------------------------------------------------------------

/// The text a test envelope carries (as a tool name, which survives the
/// metadata-only fixture) to make the privacy pass block until released.
const BLOCK_MARKER: &str = "worker_concurrency_block_marker";

/// A privacy boundary whose classifier half sleeps `delay`, or, for an
/// envelope that carries `BLOCK_MARKER`, waits for a permit of `release`;
/// it counts its calls and the most that ever ran at once.
struct SlowPrivacyBoundary {
    delay: StdDuration,
    release: Arc<tokio::sync::Semaphore>,
    in_flight: InFlight,
}

impl SlowPrivacyBoundary {
    fn new(delay: StdDuration) -> Arc<Self> {
        Arc::new(Self {
            delay,
            release: Arc::new(tokio::sync::Semaphore::new(0)),
            in_flight: InFlight::default(),
        })
    }
}

#[async_trait::async_trait]
impl PipelinePrivacyBoundary for SlowPrivacyBoundary {
    async fn rescrub_deterministic(
        &self,
        _envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        Ok(Vec::new())
    }

    async fn rescrub_classifier(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        self.in_flight.enter();
        if serde_json::to_string(envelope)?.contains(BLOCK_MARKER) {
            let _ = self.release.acquire().await;
        } else {
            tokio::time::sleep(self.delay).await;
        }
        self.in_flight.leave();
        Ok(Vec::new())
    }
}

/// One worker under test: the compatibility service with `privacy`, its
/// state, and the tenants it drains.
struct WorkerFixture {
    runtime: Arc<PgBackend>,
    service: Arc<PipelineService>,
    state: Arc<AppState>,
    index: Arc<IsolatedPipelineIndex>,
    trace_credit: Arc<RecordingSettlementAdapter>,
    _dir: tempfile::TempDir,
}

impl WorkerFixture {
    async fn new(
        privacy: Arc<dyn PipelinePrivacyBoundary>,
        tenants: &[&str],
        concurrency: usize,
    ) -> Option<Self> {
        let runtime = runtime_backend(16).await?;
        let dir = tempfile::tempdir().expect("temp dir");
        let artifacts = local_artifacts(&dir);
        let index = IsolatedPipelineIndex::new();
        let trace_credit = RecordingSettlementAdapter::new(
            InstrumentId::trace_credit(),
            "recording_trace_credit_worker_concurrency_test_only",
            "none",
        );
        let service = assemble_compatibility_pipeline_service_with(
            runtime.clone(),
            &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
            index.clone(),
            2_500_000,
            privacy,
            vec![trace_credit.clone() as Arc<dyn SettlementAdapter>],
            None,
        );
        let mut state = test_state_with_options(
            dir.path().to_path_buf(),
            Some(mains_database().await),
            Some(artifacts),
            false,
            false,
            false,
            false,
        );
        let state_mut = Arc::make_mut(&mut state);
        state_mut.require_db_mirror_writes = true;
        state_mut.pipeline_service = Some(service.clone());
        state_mut.pipeline_drain_tenant_ids =
            Arc::new(tenants.iter().map(|tenant| tenant.to_string()).collect());
        state_mut.pipeline_worker_concurrency = concurrency;
        for tenant in tenants {
            service
                .register_default_bundle(tenant)
                .await
                .expect("register the bundle");
        }
        Some(Self {
            runtime,
            service,
            state,
            index,
            trace_credit,
            _dir: dir,
        })
    }

    /// Receives a distinct metadata-only envelope for `tenant`, carrying
    /// `BLOCK_MARKER` when `blocking`, and returns its submission id. The
    /// worker moves the run.
    async fn submit(&self, tenant: &str, blocking: bool) -> Uuid {
        let mut envelope = model_training_envelope().await;
        envelope.submission_id = Uuid::new_v4();
        let tool_name = if blocking {
            format!("{BLOCK_MARKER}_{}", envelope.submission_id.simple())
        } else {
            format!("tool_{}", envelope.submission_id.simple())
        };
        set_metadata_only_tool_name(&mut envelope, &tool_name);
        let raw = serde_json::to_vec(&envelope).unwrap();
        let key = envelope.submission_id.to_string();
        let principal = static_token_principal_ref(&format!("token-worker-conc-{tenant}"));
        let PipelineReceiptResult::Created(_) = self
            .service
            .submit(PipelineReceiptRequest {
                source_session: None,
                tenant_id: tenant,
                actor_principal_ref: &principal,
                counts_toward_quota: true,
                request_idempotency_key: &key,
                request_bytes: &raw,
                server_envelope: &envelope,
                residual_risk_basis: &[],
                limits: PipelineAdmissionLimits {
                    max_per_tenant_per_hour: 0,
                    max_per_principal_per_hour: 0,
                },
            })
            .await
            .expect("the receipt succeeds")
        else {
            panic!("the receipt creates a run")
        };
        envelope.submission_id
    }

    /// `(state, count)` of the tenant's runs.
    async fn run_states(&self, tenant: &str) -> BTreeMap<String, i64> {
        let mut client = self.runtime.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let rows = tx
            .query(
                "SELECT state, COUNT(*) FROM pipeline_runs WHERE tenant_id = $1 GROUP BY state",
                &[&tenant],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        rows.iter()
            .map(|row| (row.get::<_, String>(0), row.get::<_, i64>(1)))
            .collect()
    }

    /// One `i64` from a tenant-scoped query with the tenant as `$1`.
    async fn count(&self, tenant: &str, sql: &str) -> i64 {
        let mut client = self.runtime.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let count = tx.query_one(sql, &[&tenant]).await.unwrap().get(0);
        tx.commit().await.unwrap();
        count
    }
}

/// Stops `worker` and waits for it, as `run_pipeline_app` does.
async fn stop_worker(worker: pipeline_runtime::PipelineWorkerHandle) {
    let _ = worker.stop.send(true);
    pipeline_runtime::join_or_abort(worker.join, StdDuration::from_secs(30)).await;
}

/// Test (1): with a worker concurrency of 2, two runs whose Review-start
/// privacy pass is slow are in it at the same time, and both finish in
/// about one pass's time rather than two. Red on the sequential pass: one
/// pass at a time.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_slow_reviews_run_at_once_with_a_worker_concurrency_of_two() {
    const DELAY: StdDuration = StdDuration::from_secs(3);
    let boundary = SlowPrivacyBoundary::new(DELAY);
    let tenant = format!("tenant-worker-conc-two-{}", Uuid::new_v4().simple());
    let Some(fixture) = WorkerFixture::new(boundary.clone(), &[&tenant], 2).await else {
        return;
    };
    let first = fixture.submit(&tenant, false).await;
    let second = fixture.submit(&tenant, false).await;

    let worker = pipeline_runtime::spawn_pipeline_worker(fixture.state.clone())
        .expect("a runtime, so a worker");
    wait_for_run_state(&fixture.runtime, &tenant, first, "complete").await;
    wait_for_run_state(&fixture.runtime, &tenant, second, "complete").await;
    stop_worker(worker).await;

    assert_eq!(boundary.in_flight.calls.load(Ordering::SeqCst), 2);
    // Counted, not timed: the second pass started before the first ended,
    // so the two together took about one pass's time. (A wall-clock bound
    // flakes on a loaded runner.)
    assert_eq!(
        boundary.in_flight.max.load(Ordering::SeqCst),
        2,
        "both privacy passes ran at once"
    );
}

/// Test (2): a run of a second tenant, received after the worker is already
/// held in a first tenant's slow run, completes while that run is still
/// held. Red on the sequential pass, which never reaches the second tenant.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_tenants_run_is_not_held_behind_a_slow_run_of_the_first() {
    let boundary = SlowPrivacyBoundary::new(StdDuration::from_millis(10));
    let suffix = Uuid::new_v4().simple().to_string();
    // The drain list is ordered: tenant A comes first.
    let tenant_a = format!("tenant-worker-conc-a-{suffix}");
    let tenant_b = format!("tenant-worker-conc-b-{suffix}");
    let Some(fixture) = WorkerFixture::new(boundary.clone(), &[&tenant_a, &tenant_b], 2).await
    else {
        return;
    };
    let held = fixture.submit(&tenant_a, true).await;
    let worker = pipeline_runtime::spawn_pipeline_worker(fixture.state.clone())
        .expect("a runtime, so a worker");
    eventually("tenant A's run is in its privacy pass", || {
        boundary.in_flight.now.load(Ordering::SeqCst) == 1
    })
    .await;

    let second = fixture.submit(&tenant_b, false).await;
    let reached = tokio::time::timeout(
        StdDuration::from_secs(20),
        wait_for_run_state(&fixture.runtime, &tenant_b, second, "complete"),
    )
    .await;
    let a_states = fixture.run_states(&tenant_a).await;
    boundary.release.add_permits(1);
    wait_for_run_state(&fixture.runtime, &tenant_a, held, "complete").await;
    stop_worker(worker).await;

    assert!(
        reached.is_ok(),
        "tenant B's run did not complete while tenant A's run was held"
    );
    assert_eq!(
        a_states,
        BTreeMap::from([("leased".to_string(), 1)]),
        "tenant A's run was still held when tenant B's completed"
    );
}

/// Test (3): forty runs across three tenants, with a worker concurrency of
/// 4, each complete exactly once: one outcome per phase, one settlement
/// call and one settlement row per run and instrument, at most one credit
/// ledger row and exactly one `CreditMutate` audit row per credit event,
/// and one index revision per run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forty_runs_across_three_tenants_each_complete_once() {
    let boundary = SlowPrivacyBoundary::new(StdDuration::from_millis(100));
    let suffix = Uuid::new_v4().simple().to_string();
    let tenants = [
        (format!("tenant-worker-conc-x-{suffix}"), 20),
        (format!("tenant-worker-conc-y-{suffix}"), 19),
        (format!("tenant-worker-conc-z-{suffix}"), 1),
    ];
    let names = tenants
        .iter()
        .map(|(tenant, _)| tenant.as_str())
        .collect::<Vec<_>>();
    let Some(fixture) = WorkerFixture::new(boundary.clone(), &names, 4).await else {
        return;
    };
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let mut submissions = Vec::new();
    for (tenant, runs) in &tenants {
        for _ in 0..*runs {
            submissions.push((tenant.clone(), fixture.submit(tenant, false).await));
        }
    }

    let worker = pipeline_runtime::spawn_pipeline_worker(fixture.state.clone())
        .expect("a runtime, so a worker");
    for (tenant, submission_id) in &submissions {
        wait_for_run_state(&fixture.runtime, tenant, *submission_id, "complete").await;
    }
    // The credit audit step runs after the runs; wait for every leg.
    let deadline = std::time::Instant::now() + StdDuration::from_secs(60);
    for (tenant, _) in &tenants {
        while fixture
            .count(
                tenant,
                "SELECT COUNT(*) FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND credit_event_id IS NOT NULL
                    AND credit_audited_at IS NULL",
            )
            .await
            > 0
        {
            assert!(
                std::time::Instant::now() < deadline,
                "credit audits pending"
            );
            tokio::time::sleep(StdDuration::from_millis(100)).await;
        }
    }
    stop_worker(worker).await;

    assert_eq!(boundary.in_flight.calls.load(Ordering::SeqCst), 40);
    assert!(
        boundary.in_flight.max.load(Ordering::SeqCst) >= 2,
        "the runs overlapped"
    );
    let requests = fixture.trace_credit.requests();
    let settled_runs = requests
        .iter()
        .map(|request| request.run_id())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        requests.len(),
        settled_runs.len(),
        "no run was settled twice"
    );
    for (tenant, runs) in &tenants {
        let runs = *runs as i64;
        assert_eq!(
            fixture.run_states(tenant).await,
            BTreeMap::from([("complete".to_string(), runs)]),
        );
        assert_eq!(
            fixture
                .count(
                    tenant,
                    "SELECT COUNT(*) FROM (SELECT run_id FROM phase_outcomes
                      WHERE tenant_id = $1 GROUP BY run_id, phase HAVING COUNT(*) > 1) d",
                )
                .await,
            0
        );
        let settlement_rows = fixture
            .count(
                tenant,
                "SELECT COUNT(*) FROM pipeline_run_settlements WHERE tenant_id = $1",
            )
            .await;
        assert_eq!(settlement_rows, runs, "one settlement row per run");
        let credit_events = fixture
            .count(
                tenant,
                "SELECT COUNT(*) FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND credit_event_id IS NOT NULL",
            )
            .await;
        assert_eq!(
            fixture
                .count(
                    tenant,
                    "SELECT COUNT(*) FROM trace_credit_ledger l
                       JOIN pipeline_run_settlements s
                         ON s.tenant_id = l.tenant_id AND s.credit_event_id = l.credit_event_id
                      WHERE l.tenant_id = $1",
                )
                .await,
            credit_events,
            "one ledger row per credit event"
        );
        assert_eq!(
            fixture
                .count(
                    tenant,
                    "SELECT COUNT(*) FROM (SELECT submission_id FROM trace_credit_ledger
                      WHERE tenant_id = $1 AND submission_id IS NOT NULL
                      GROUP BY submission_id, event_type HAVING COUNT(*) > 1) d",
                )
                .await,
            0,
            "no submission has two ledger rows of one type"
        );
        for (submission_tenant, submission_id) in &submissions {
            if submission_tenant == tenant {
                assert_eq!(
                    count_outcomes(&fixture.runtime, tenant, *submission_id).await,
                    4,
                    "one outcome per phase"
                );
            }
        }
        let audit_rows = {
            let mut total = 0;
            for (submission_tenant, submission_id) in &submissions {
                if submission_tenant == tenant {
                    let rows = credit_mutate_audit_rows(&owner, tenant, *submission_id).await;
                    assert!(rows <= 1, "at most one CreditMutate row per submission");
                    total += rows;
                }
            }
            total
        };
        assert_eq!(
            audit_rows, credit_events,
            "one CreditMutate row per credit event"
        );
        let tenant_ref =
            trace_commons_server::versioned_pipeline::pipeline_tenant_storage_ref(tenant);
        // Whether a run is indexed depends on what the index held when its
        // Score ran (a chunk too close to an earlier run's is not
        // inserted), so the index is checked against each Settle's own
        // membership decision: one revision per included run, and exactly
        // the entries those decisions name.
        let included_runs = fixture
            .count(
                tenant,
                "SELECT COUNT(*) FROM pipeline_runs
                  WHERE tenant_id = $1 AND index_write_state = 'complete'",
            )
            .await;
        assert_eq!(
            fixture
                .count(
                    tenant,
                    "SELECT COUNT(*) FROM phase_outcomes
                      WHERE tenant_id = $1 AND phase = 'settle'
                        AND decision->'index_membership'->>'kind' = 'include'",
                )
                .await,
            included_runs
        );
        assert_eq!(
            fixture.index.revision_count(&tenant_ref, MINIMAL_INDEX_ID) as i64,
            included_runs,
            "one index revision per included run"
        );
        assert_eq!(
            fixture.index.entry_count(&tenant_ref, MINIMAL_INDEX_ID) as i64,
            fixture
                .count(
                    tenant,
                    "SELECT COALESCE(SUM((decision->'index_membership'->>'entry_count')::BIGINT), 0)::BIGINT
                       FROM phase_outcomes
                      WHERE tenant_id = $1 AND phase = 'settle'
                        AND decision->'index_membership'->>'kind' = 'include'",
                )
                .await,
            "each included run's entries, once"
        );
    }
}
