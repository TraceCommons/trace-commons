// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Spec section 3D acceptance: a receipt through the real HTTP router, auth,
//! and worker completes, survives a crash and restart, replays, conflicts on
//! changed content, refuses bad credentials and other tenants, and keeps
//! `/v1/source` public -- with one logical effect. This runs the exact
//! router and worker `trace-commons-ingest` boots in production
//! (`pipeline_runtime::build_pipeline_app`, `pipeline_runtime::run_pipeline_app`),
//! in-process on a loopback port, with test dependencies injected through
//! `IngestPipelineRuntimeAssembler` (P5) -- never a direct call into
//! `PipelineService`.
//!
//! Nested inside `tests` (via the same pattern as `admission_pg_tests` and
//! `nearai_ceremony_pg_tests`) rather than declared as a sibling module of
//! `trace-commons-ingest.rs`: `test_state_with_options`, `sample_envelope`,
//! `make_metadata_only_low_risk`, `auth_headers`, and `cleanup_pg_trace_tenant`
//! are private to `tests` and visible only to its descendants.

use super::*;

use std::collections::BTreeMap;
use std::sync::Arc;

use trace_commons_gate_api::pipeline::{
    AtomicUnits, InstrumentDescriptor, InstrumentId, InstrumentKind,
};
use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};
use trace_commons_server::versioned_pipeline::{
    PipelineCaps, PipelineCrashPoint, PipelineServiceBuilder,
};
use trace_commons_server::versioned_pipeline_bundle::{
    MinimalPolicyBundle, PipelineBundleConfig, PipelineInstrumentAwardConfig,
};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapter, SettlementAdapterRegistry,
};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;

/// This suite's own runtime role, distinct from `trace_pipeline_runtime_test`
/// (`tests/versioned_pipeline_runtime_pg.rs`, Task 8) so the two suites never
/// contend over the same role's grants even if a future job runs them
/// against databases on the same server.
const PIPELINE_HTTP_RUNTIME_ROLE: &str = "trace_pipeline_http_runtime_test";
static PIPELINE_HTTP_SETUP_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Connects as a `NOBYPASSRLS`, `NOSUPERUSER` runtime role, the way every
/// PostgreSQL pipeline test must (global constraints, "The pipeline service
/// connects as a `NOBYPASSRLS`, `NOSUPERUSER` runtime role"). Copied from the
/// Task 8 harness (`tests/versioned_pipeline_runtime_pg.rs::runtime_backend`)
/// rather than shared with it: that helper is private to its own
/// integration-test binary. Not `postgres_backend_for_ingest_test` (this
/// file's sibling in `tests.rs`): that helper also reads `DATABASE_URL` and
/// skips on any setup failure, which the controller ruled out here -- this
/// suite reads only `TRACE_COMMONS_PG_TEST_DATABASE_URL` and panics on any
/// failure once it is set. Returns `None` only when the variable is unset.
async fn runtime_backend(pool_size: usize) -> Option<Arc<PgBackend>> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").ok()?;
    let _guard = PIPELINE_HTTP_SETUP_LOCK.lock().await;
    let owner = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .expect("connect as migration owner");
    owner.run_migrations().await.expect("apply migrations");
    let client = owner
        .trace_pool_for_test()
        .get()
        .await
        .expect("owner client");
    client
        .batch_execute(&format!(
            "DO $$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{PIPELINE_HTTP_RUNTIME_ROLE}')
            THEN CREATE ROLE {PIPELINE_HTTP_RUNTIME_ROLE} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOINHERIT;
            END IF;
         END $$;
         GRANT USAGE ON SCHEMA public TO {PIPELINE_HTTP_RUNTIME_ROLE};
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO {PIPELINE_HTTP_RUNTIME_ROLE};
         GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO {PIPELINE_HTTP_RUNTIME_ROLE};"
        ))
        .await
        .expect("provision runtime role");
    let mut runtime_url = reqwest::Url::parse(&url).expect("parse test URL");
    runtime_url
        .set_username(PIPELINE_HTTP_RUNTIME_ROLE)
        .expect("set runtime user");
    let backend = PgBackend::new(&DatabaseConfig::from_postgres_url(
        runtime_url.as_str(),
        pool_size,
    ))
    .await
    .expect("connect as runtime role");
    let row = backend
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap();
    assert!(
        !row.get::<_, bool>(0) && !row.get::<_, bool>(1),
        "runtime role must not bypass RLS"
    );
    Some(Arc::new(backend))
}

/// Pinned per amendments-971 ruling A4: an off-chain credit account, whole
/// units only. The only instrument this test's minimal bundle awards.
fn storage_rebate_descriptor() -> InstrumentDescriptor {
    InstrumentDescriptor {
        kind: InstrumentKind::CreditAccount,
        network: "pipeline-test".to_string(),
        contract: "storage-rebate".to_string(),
        decimals: 0,
    }
}

/// A `LocalEncryptedTraceArtifactStore` rooted at `dir`, reused by both the
/// `AppState` the router reads objects through and the `TestAssembler` the
/// pipeline runtime reads and writes pipeline artifacts through. Delegates to
/// `test_artifact_store` (this file's parent, `tests.rs`) rather than
/// duplicating its crypto setup.
fn local_artifacts(dir: &tempfile::TempDir) -> Arc<LocalEncryptedTraceArtifactStore> {
    test_artifact_store(dir.path())
}

/// P5: this test's own `IngestPipelineRuntimeAssembler`. Holds the pieces
/// app 1 and app 2 must share across the restart -- the in-memory index and
/// the recording settlement adapters -- plus this instance's own crash
/// point, and builds a fresh `PipelineService` from them on every call. The
/// database backend and artifact store come from `IngestPipelineRuntimeContext`,
/// which `assemble_ingest_pipeline_runtime` resolves from the same
/// `TraceCorpusDbConnections` / `ConfiguredTraceArtifactStore` a real boot
/// would use -- this struct is the seam a proprietary production assembly
/// fills in, exercised here with reference dependencies instead.
struct TestAssembler {
    index: Arc<IsolatedPipelineIndex>,
    adapters: Vec<Arc<dyn SettlementAdapter>>,
    crash_point: Option<PipelineCrashPoint>,
}

impl IngestPipelineRuntimeAssembler for TestAssembler {
    fn assemble(
        &self,
        context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        let scorer = Arc::new(ReferencePerplexityScorer::new());
        let embedder = Arc::new(ReferenceEmbedder::new());
        let package = MinimalPolicyBundle::minimal_package(
            &PipelineBundleConfig {
                instrument_awards: vec![PipelineInstrumentAwardConfig {
                    instrument_id: "storage_rebate".into(),
                    atomic_units: AtomicUnits::from_raw(5),
                    descriptor: storage_rebate_descriptor(),
                }],
                include_index: true,
                variant: None,
            },
            scorer.as_ref(),
            embedder.as_ref(),
        )?;
        let registry = SettlementAdapterRegistry::new(self.adapters.clone())?;
        let caps = PipelineCaps {
            per_instrument_atomic_units: BTreeMap::from([
                (
                    "storage_rebate".to_string(),
                    AtomicUnits::from_raw(u128::MAX),
                ),
                (
                    InstrumentId::trace_credit().as_str().to_string(),
                    AtomicUnits::from_raw(u128::MAX),
                ),
            ]),
        };
        let mut builder = PipelineServiceBuilder::new(
            context.backend,
            context.artifact_store,
            package,
            self.index.clone(),
            self.index.clone(),
            registry,
            caps,
        )
        .with_scorer(scorer)
        .with_embedder(embedder)
        .with_object_store_name(context.object_store_name);
        if let Some(crash_point) = self.crash_point {
            builder = builder.with_crash_point(crash_point);
        }
        Ok(Arc::new(builder.build()?))
    }
}

/// Builds a `PipelineService` through the same injection seam ingest's real
/// boot uses (`assemble_ingest_pipeline_runtime`), rather than constructing
/// one directly -- the point of this test is that the router and worker run
/// against a service assembled exactly the way production assembles one.
fn assemble_test_pipeline_service(
    backend: Arc<PgBackend>,
    artifacts: Arc<LocalEncryptedTraceArtifactStore>,
    index: Arc<IsolatedPipelineIndex>,
    adapters: Vec<Arc<dyn SettlementAdapter>>,
    crash_point: Option<PipelineCrashPoint>,
) -> Arc<PipelineService> {
    let assembler = TestAssembler {
        index,
        adapters,
        crash_point,
    };
    let connections = TraceCorpusDbConnections {
        database: backend.clone() as Arc<dyn Database>,
        postgres: backend,
    };
    let configured_store = ConfiguredTraceArtifactStore::legacy(artifacts);
    assemble_ingest_pipeline_runtime(
        Some(&assembler),
        Some(&connections),
        Some(&configured_store),
        false,
    )
    .expect("assemble the injected pipeline runtime")
    .expect("an assembler was given, so a service is returned")
}

/// Opens a tenant-scoped transaction the way every raw-SQL helper below
/// needs one: `set_config('trace_commons.trace_tenant_id', ...)` first, so
/// RLS admits only `tenant_id`'s own rows.
async fn tenant_tx<'a>(
    client: &'a mut deadpool_postgres::Client,
    tenant_id: &str,
) -> deadpool_postgres::Transaction<'a> {
    let tx = client.transaction().await.expect("open tenant tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set tenant for tx");
    tx
}

/// P3: polls `pipeline_runs.settle_selection_hash` for `submission_id` every
/// 100 ms, up to 60 s, then panics with a clear message. This is how the
/// test learns app 1's worker reached and durably committed the Settle
/// selection -- immediately before the injected `AfterSettleSelection`
/// crash -- without calling the processor directly.
async fn wait_for_settle_selection(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: uuid::Uuid,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let mut client = backend
            .trace_pool_for_test()
            .get()
            .await
            .expect("client for wait_for_settle_selection");
        let tx = tenant_tx(&mut client, tenant_id).await;
        let hash: Option<String> = tx
            .query_one(
                "SELECT settle_selection_hash FROM pipeline_runs
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await
            .expect("the run row exists")
            .get(0);
        tx.commit().await.expect("commit wait_for_settle_selection");
        if hash.is_some() {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "timed out after 60s waiting for pipeline_runs.settle_selection_hash to be \
                 set for tenant-a's submission -- app 1's worker never reached (or never \
                 durably committed) the Settle selection"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// P3: polls `pipeline_runs.state` for `submission_id` every 100 ms, up to
/// 60 s, then panics with a clear message. This is how the test learns app
/// 2's worker reclaimed the expired lease and drove the run to completion on
/// its own -- no manual `process_run` call.
async fn wait_for_run_complete(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: uuid::Uuid,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let mut client = backend
            .trace_pool_for_test()
            .get()
            .await
            .expect("client for wait_for_run_complete");
        let tx = tenant_tx(&mut client, tenant_id).await;
        let state: String = tx
            .query_one(
                "SELECT state FROM pipeline_runs WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await
            .expect("the run row exists")
            .get(0);
        tx.commit().await.expect("commit wait_for_run_complete");
        if state == "complete" {
            return;
        }
        if std::time::Instant::now() >= deadline {
            panic!(
                "timed out after 60s waiting for pipeline_runs.state = 'complete' for \
                 tenant-a's submission -- app 2's worker never resumed the reclaimed run \
                 (last observed state: {state})"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// Polls `GET /v1/pipeline/readiness` until it reports `{"status":"ready"}`,
/// up to 10 s -- bounded so a broken worker fails the test instead of
/// hanging it, generous enough to absorb the gap between a freshly spawned
/// worker task and its first readiness probe.
async fn wait_for_pipeline_ready(client: &reqwest::Client, base: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let response = client
            .get(format!("{base}/v1/pipeline/readiness"))
            .send()
            .await
            .expect("readiness request");
        if response.status() == reqwest::StatusCode::OK {
            let body: serde_json::Value = response.json().await.expect("readiness body");
            if body == serde_json::json!({"status": "ready"}) {
                return;
            }
        }
        if std::time::Instant::now() >= deadline {
            panic!("pipeline readiness did not report {{\"status\":\"ready\"}} within 10s");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// P3's time shortcut, not a processor call: expires the crashed run's
/// lease directly, in a tenant-scoped transaction, only when it is still
/// `leased` -- exactly what a real lease does on its own once its duration
/// elapses, done immediately instead of waiting it out.
async fn expire_run_lease(backend: &Arc<PgBackend>, tenant_id: &str, submission_id: uuid::Uuid) {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for expire_run_lease");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let updated = tx
        .execute(
            "UPDATE pipeline_runs
                SET lease_expires_at = NOW() - INTERVAL '1 second'
              WHERE tenant_id = $1 AND submission_id = $2 AND state = 'leased'",
            &[&tenant_id, &submission_id],
        )
        .await
        .expect("expire the crashed run's lease");
    assert_eq!(
        updated, 1,
        "expected exactly one leased run for tenant-a's submission to expire"
    );
    tx.commit().await.expect("commit expire_run_lease");
}

/// `phase_outcomes` joined to `pipeline_runs` by submission, counted in a
/// tenant-scoped transaction. Proves "one logical effect across the
/// restart": exactly one outcome per phase (Admission, Review, Score,
/// Settle) survives the crash and resume, never a duplicate.
async fn count_outcomes(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: uuid::Uuid,
) -> i64 {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("client for count_outcomes");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let count: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM phase_outcomes o
              JOIN pipeline_runs r ON r.tenant_id = o.tenant_id AND r.run_id = o.run_id
             WHERE r.tenant_id = $1 AND r.submission_id = $2",
            &[&tenant_id, &submission_id],
        )
        .await
        .expect("count outcomes")
        .get(0);
    tx.commit().await.expect("commit count_outcomes");
    count
}

/// Converts an axum `HeaderMap` (what `auth_headers` builds) into a
/// `reqwest::header::HeaderMap`, so the same auth-header helper this file's
/// direct-call tests use can drive a real HTTP client too.
fn reqwest_headers(headers: axum::http::HeaderMap) -> reqwest::header::HeaderMap {
    let mut converted = reqwest::header::HeaderMap::new();
    for (name, value) in headers.iter() {
        converted.insert(
            reqwest::header::HeaderName::from_bytes(name.as_str().as_bytes())
                .expect("header name converts"),
            reqwest::header::HeaderValue::from_bytes(value.as_bytes())
                .expect("header value converts"),
        );
    }
    converted
}

/// Binds `127.0.0.1:0`, spawns `pipeline_runtime::run_pipeline_app` (the
/// binary's real entry point, worker and all -- never `build_pipeline_app`
/// alone), and returns the base URL, a shutdown sender, and the server's
/// join handle. `stop.send(())` and awaiting `server` is the live worker's
/// stop-and-join path: `run_pipeline_app` asks the worker to stop only
/// after HTTP has finished shutting down, and bounds the wait on the same
/// grace period ingest itself uses.
async fn serve_pipeline_app(
    state: Arc<AppState>,
) -> (
    String,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<anyhow::Result<()>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback listener");
    let addr = listener.local_addr().expect("listener local address");
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let shutdown = async move {
        let _ = stop_rx.await;
    };
    let server = tokio::spawn(pipeline_runtime::run_pipeline_app(
        state, listener, shutdown,
    ));
    (format!("http://{addr}"), stop_tx, server)
}

/// Awaits `server` within `timeout_secs`, panicking with a clear message on
/// either a timeout or a task panic, then unwraps the `anyhow::Result` the
/// serve future itself returned. Used to assert app 1's join returns `Ok`
/// within the shutdown grace period after the stop signal, and to shut app
/// 2 down cleanly at the end of the test.
async fn join_within(
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
    timeout_secs: u64,
    label: &str,
) {
    tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), server)
        .await
        .unwrap_or_else(|_| panic!("{label}'s server task did not join within {timeout_secs}s"))
        .unwrap_or_else(|_| panic!("{label}'s server task panicked"))
        .unwrap_or_else(|error| panic!("{label} did not shut down cleanly: {error}"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_receipt_completes_and_resumes_after_restart() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    cleanup_pg_trace_tenant(&backend, "tenant-a").await;
    cleanup_pg_trace_tenant(&backend, "tenant-b").await;

    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    // Shared between app 1 and app 2 (P5): the in-memory index and the
    // recording settlement adapters have no way to see each other's writes
    // unless the very same instances back both apps.
    let index = IsolatedPipelineIndex::new();
    let storage_rebate = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_http_test_only",
        "none",
    );
    let trace_credit = RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_http_test_only",
        "none",
    );
    let adapters: Vec<Arc<dyn SettlementAdapter>> = vec![
        storage_rebate as Arc<dyn SettlementAdapter>,
        trace_credit as Arc<dyn SettlementAdapter>,
    ];

    let start = |crash_point: Option<PipelineCrashPoint>| {
        let service = assemble_test_pipeline_service(
            backend.clone(),
            artifacts.clone(),
            index.clone(),
            adapters.clone(),
            crash_point,
        );
        let mut state = test_state_with_options(
            dir.path().to_path_buf(),
            Some(backend.clone() as Arc<dyn Database>),
            Some(artifacts.clone()),
            // db_contributor_reads = true: the cross-tenant check below goes
            // through the legacy `/v1/contributors/me/submission-status`
            // route, which the pipeline's Postgres-only writes are only
            // visible through when contributor reads come from the DB
            // mirror rather than the (unused, for a pipeline receipt) local
            // JSONL store.
            true,
            false,
            false,
            false,
        );
        let state_mut = Arc::make_mut(&mut state);
        state_mut.pipeline_service = Some(service);
        state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
            TraceTenantRolloutFeature::PipelineReceipts,
            &["tenant-a"],
        );
        state
    };

    // ---- App 1: crashes once, right after the Settle selection commits (P3) ----
    let (base, stop, server) =
        serve_pipeline_app(start(Some(PipelineCrashPoint::AfterSettleSelection))).await;
    let client = reqwest::Client::new();
    let mut envelope = sample_envelope().await;
    make_metadata_only_low_risk(&mut envelope);
    let body = serde_json::to_vec(&envelope).expect("envelope serialises");

    let response = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers("token-a")))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("submit over real HTTP");
    assert_eq!(response.status(), 200);
    let receipt: serde_json::Value = response.json().await.expect("receipt body");
    assert_eq!(receipt["status"], "processing");

    // App 1's live worker drains Admission -> Review -> Score -> Settle
    // selection on its own; wait for the durable marker instead of calling
    // the processor directly.
    wait_for_settle_selection(&backend, "tenant-a", envelope.submission_id).await;

    // Stop app 1: send the shutdown signal and await the join within the
    // shutdown grace period, exercising the live worker's stop-and-join path.
    stop.send(()).expect("send shutdown to app 1");
    join_within(server, 20, "app 1").await;

    // A time shortcut, not a processor call (P3): expire the crashed run's
    // lease directly, the way a real lease naturally expires.
    expire_run_lease(&backend, "tenant-a", envelope.submission_id).await;

    // ---- App 2: same database, artifact root, index, and adapters; no crash point ----
    let (base, stop, server) = serve_pipeline_app(start(None)).await;

    // Readiness is live while app 2 runs.
    wait_for_pipeline_ready(&client, &base).await;

    // App 2's live worker reclaims the expired lease and completes the run
    // on its own; no manual `process_run` call.
    wait_for_run_complete(&backend, "tenant-a", envelope.submission_id).await;

    // Readiness stays live after the run completes too.
    wait_for_pipeline_ready(&client, &base).await;

    // Replay: identical bytes under the same submission id return the same
    // run, not a new one.
    let replay = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers("token-a")))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("replay submission");
    assert_eq!(replay.status(), 200);

    // Conflict: changed content under the same submission id.
    let mut changed = envelope.clone();
    changed.privacy.warnings.push("changed-content".to_string());
    let conflict = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers("token-a")))
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&changed).expect("changed envelope serialises"))
        .send()
        .await
        .expect("conflicting submission");
    assert_eq!(conflict.status(), 409);

    // Bad credentials are refused. `authenticate` (the shared auth helper
    // every handler in `app()` goes through) answers an unrecognized-but-
    // present bearer token with 403 ("unknown tenant token"), not 401 --
    // 401 is reserved for a missing or malformed `Authorization` header.
    let denied = client
        .post(format!("{base}/v1/traces"))
        .header("authorization", "Bearer wrong")
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("submission with a bad credential");
    assert_eq!(denied.status(), 403);

    // /v1/source stays public (AGPL section 13), even on the pipeline path.
    let source = client
        .get(format!("{base}/v1/source"))
        .send()
        .await
        .expect("source request");
    assert_eq!(source.status(), 200);

    // Cross-tenant: the legacy `/v1/contributors/me/submission-status`
    // handler (`app()`'s `/v1/contributors/me/submission-status`, POST) is
    // the surface the controller asked this test to exercise for isolation.
    // It never refuses another tenant's submission id with an error status
    // -- it silently omits rows the caller's tenant cannot see -- so the
    // code both calls return is 200; the list contents carry the isolation
    // proof. Tenant-a's own credential sees its submission (positive
    // control, so the empty result below is isolation and not just an
    // always-empty response); tenant-b's does not.
    let own_status = client
        .post(format!("{base}/v1/contributors/me/submission-status"))
        .headers(reqwest_headers(auth_headers("token-a")))
        .header("content-type", "application/json")
        .body(
            serde_json::to_vec(&TraceSubmissionStatusRequest {
                submission_ids: vec![envelope.submission_id],
            })
            .unwrap(),
        )
        .send()
        .await
        .expect("tenant-a submission status");
    assert_eq!(own_status.status(), 200);
    let own_body: Vec<TraceSubmissionStatusUpdate> =
        own_status.json().await.expect("tenant-a status body");
    assert_eq!(
        own_body.len(),
        1,
        "tenant-a must see its own completed submission's status"
    );
    assert_eq!(own_body[0].submission_id, envelope.submission_id);

    let other_status = client
        .post(format!("{base}/v1/contributors/me/submission-status"))
        .headers(reqwest_headers(auth_headers("token-b")))
        .header("content-type", "application/json")
        .body(
            serde_json::to_vec(&TraceSubmissionStatusRequest {
                submission_ids: vec![envelope.submission_id],
            })
            .unwrap(),
        )
        .send()
        .await
        .expect("tenant-b submission status");
    assert_eq!(other_status.status(), 200);
    let other_body: Vec<TraceSubmissionStatusUpdate> =
        other_status.json().await.expect("tenant-b status body");
    assert!(
        other_body.is_empty(),
        "tenant-b's credential must not see tenant-a's submission status"
    );

    // One logical effect across the restart: exactly one phase outcome per
    // phase (Admission, Review, Score, Settle), never a duplicate Settle
    // outcome from the crash and resume.
    assert_eq!(
        count_outcomes(&backend, "tenant-a", envelope.submission_id).await,
        4
    );

    // M11: the submitted envelope's and the approved content's object refs
    // carry the configured store's name, as a legacy receipt's do.
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, "tenant-a").await;
    let stores: Vec<(String, String)> = tx
        .query(
            "SELECT artifact_kind, object_store FROM trace_object_refs
              WHERE tenant_id = $1 AND submission_id = $2
              ORDER BY artifact_kind",
            &[&"tenant-a", &envelope.submission_id],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
    tx.commit().await.unwrap();
    assert_eq!(
        stores,
        vec![
            (
                "review_snapshot".to_string(),
                TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE.to_string()
            ),
            (
                "submitted_envelope".to_string(),
                TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE.to_string()
            ),
        ]
    );

    stop.send(()).expect("send shutdown to app 2");
    join_within(server, 20, "app 2").await;
}
