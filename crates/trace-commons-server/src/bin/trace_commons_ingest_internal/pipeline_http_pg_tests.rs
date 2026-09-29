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

#[path = "../../../tests/support/pilot_runtime_grants.rs"]
mod pilot_runtime_grants;
#[path = "../../../tests/support/pilot_runtime_login.rs"]
mod pilot_runtime_login;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::State;
use trace_commons_gate_api::pipeline::{
    AtomicUnits, InstrumentDescriptor, InstrumentId, InstrumentKind,
};
use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer, SettlementAdapter};
use trace_commons_protocol::admission::{AdmissionBinding, REQUEST_METADATA_KEY, hash_hex};
use trace_commons_protocol::trace_contribution::{
    RawTraceCaptureTurn, RawTraceContribution, TraceContributionEventType,
};
use trace_commons_server::admission_evidence::AdmissionProviderTrust;
use trace_commons_server::admission_ledger::AdmissionLimits;
use trace_commons_server::versioned_pipeline::{
    PipelineCaps, PipelineCrashPoint, PipelineServiceBuilder,
};
use trace_commons_server::versioned_pipeline_bundle::{
    MinimalPolicyBundle, PipelineBundleConfig, PipelineInstrumentAwardConfig,
};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapterRegistry,
};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::witness_service;

/// This suite's own runtime login, distinct from `trace_pipeline_runtime_test`
/// (`tests/versioned_pipeline_runtime_pg.rs`). Like that one, its only
/// privilege source is membership in `trace_ingest_runtime`, the ingest
/// runtime group V90 names.
const PIPELINE_HTTP_RUNTIME_ROLE: &str = "trace_pipeline_http_runtime_test";
static PIPELINE_HTTP_DATABASE: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

/// The database this suite runs in, created once per process: a sibling of
/// `TRACE_COMMONS_PG_TEST_DATABASE_URL`'s database named
/// `<that database>_pilot`, migrated the way the pilot's was
/// (`migrate_like_the_pilot`). Not the configured database itself: the other
/// ingest tests share that one and migrate it straight through, and the
/// pilot's V62 grants can only be given to a database that has stopped at V62.
/// Returns `None` only when the variable is unset; every failure after that
/// panics. Everything this suite does in PostgreSQL goes through this URL,
/// the fixture rows its owner connections write included.
///
/// The database is force-dropped (`WITH (FORCE)`, which disconnects any
/// other session on it) and recreated once, the first time this process
/// calls this function; every later call in the same process reuses it.
/// Two test processes pointed at the same `TRACE_COMMONS_PG_TEST_DATABASE_URL`
/// at once would each force-drop the sibling database the other is mid-setup
/// on or already running against -- do not run this suite that way.
async fn pipeline_http_database_url() -> Option<String> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").ok()?;
    Some(
        PIPELINE_HTTP_DATABASE
            .get_or_init(|| async {
                let mut pilot_url = reqwest::Url::parse(&url).expect("parse test URL");
                let database = format!("{}_pilot", pilot_url.path().trim_start_matches('/'));
                assert!(
                    database
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                    "the test database name must be lowercase letters, digits and underscores"
                );
                pilot_url.set_path(&format!("/{database}"));
                let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
                    .await
                    .expect("connect to the configured test database");
                let connection = tokio::spawn(async move {
                    let _ = connection.await;
                });
                // Separate statements: neither may run inside a transaction.
                client
                    .execute(
                        &format!("DROP DATABASE IF EXISTS {database} WITH (FORCE)"),
                        &[],
                    )
                    .await
                    .expect("drop the pilot-shaped database an earlier run left");
                client
                    .execute(&format!("CREATE DATABASE {database}"), &[])
                    .await
                    .expect("create the pilot-shaped database");
                drop(client);
                let _ = connection.await;
                let pilot_url = pilot_url.to_string();
                pilot_runtime_login::migrate_like_the_pilot(&pilot_url).await;
                pilot_runtime_login::provision_member_only_login(
                    &pilot_url,
                    PIPELINE_HTTP_RUNTIME_ROLE,
                )
                .await;
                pilot_url
            })
            .await
            .clone(),
    )
}

/// Connects to `pipeline_http_database_url` as this suite's runtime login, a
/// `NOBYPASSRLS`, `NOSUPERUSER` role, the way every PostgreSQL pipeline test
/// must. Not `postgres_backend_for_ingest_test` (this file's sibling in
/// `tests.rs`): that helper also reads `DATABASE_URL` and skips on any setup
/// failure; this suite reads only `TRACE_COMMONS_PG_TEST_DATABASE_URL` and
/// panics on any failure once it is set. Returns `None` only when the
/// variable is unset.
///
/// The login's privileges are the pilot's: a real HTTP submission runs the
/// legacy ingest path (`state.db_mirror`, the NEAR admission functions) and
/// the pipeline as this login, so both are held to what
/// `trace_ingest_runtime` is granted.
async fn runtime_backend(pool_size: usize) -> Option<Arc<PgBackend>> {
    let url = pipeline_http_database_url().await?;
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
///
/// This suite's `TestAssembler` builds reference dependencies
/// (`ReferencePerplexityScorer`, `ReferenceEmbedder`, `IsolatedPipelineIndex`,
/// `RecordingSettlementAdapter`) that are not production-qualified, and more
/// than one test in this file routes `tenant-a` to the pipeline (see
/// `real_http_receipt_completes_and_resumes_after_restart` and
/// `real_http_pipeline_receipt_checks_ownership_on_replay`), so both new
/// fail-closed arguments are set here explicitly: `tenants_routed = true`
/// reflects that reality rather than hiding it, and `allow_test_dependencies
/// = true` is the test-only opt-in that lets the unqualified runtime start
/// anyway.
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
        trace_commons_server::versioned_pipeline::PipelineLeaseConfig::default(),
        true,
        true,
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

/// `run_pipeline_app` -- this suite's own entry point, the one `main` calls
/// too -- registers and activates the default bundle for every
/// `PipelineReceipts` rollout tenant before it starts the worker or the
/// HTTP listener. A tenant that has never submitted anything still has an
/// active bundle the moment the server comes up, with no receipt required
/// to put one there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_registers_the_default_bundle_for_rollout_tenants() {
    let Some(backend) = runtime_backend(2).await else {
        return;
    };
    let tenant = format!("startup-bundle-{}", uuid::Uuid::new_v4());
    cleanup_pg_trace_tenant(&backend, &tenant).await;

    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let index = IsolatedPipelineIndex::new();
    let storage_rebate = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_startup_test_only",
        "none",
    );
    let trace_credit = RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_startup_test_only",
        "none",
    );
    let adapters: Vec<Arc<dyn SettlementAdapter>> = vec![
        storage_rebate as Arc<dyn SettlementAdapter>,
        trace_credit as Arc<dyn SettlementAdapter>,
    ];
    let service =
        assemble_test_pipeline_service(backend.clone(), artifacts.clone(), index, adapters, None);

    assert!(
        service
            .store()
            .active_bundle_id(&tenant)
            .await
            .expect("query the tenant's active bundle before startup")
            .is_none(),
        "a tenant `run_pipeline_app` has not started for yet has no active bundle"
    );

    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        Some(backend.clone() as Arc<dyn Database>),
        Some(artifacts.clone()),
        false,
        false,
        false,
        false,
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.pipeline_service = Some(service.clone());
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[tenant.as_str()],
    );

    let (_base, stop, server) = serve_pipeline_app(state).await;

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if service
            .store()
            .active_bundle_id(&tenant)
            .await
            .expect("query the tenant's active bundle after startup")
            .is_some()
        {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "startup never registered and activated the default bundle for the rollout tenant"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let active = service
        .store()
        .active_bundle_id(&tenant)
        .await
        .unwrap()
        .expect("checked above");
    assert_eq!(
        active,
        service.bundle_id(),
        "the tenant's active bundle is the service's own default bundle"
    );

    stop.send(()).unwrap();
    join_within(server, 15, "startup bundle registration test").await;
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

// ----------------------------------------------------------------------------
// A retried upload for a pipeline-routed tenant whose admission ledger
// already marked it `completed` must replay the pipeline receipt instead of
// 500ing on the legacy file record the pipeline never writes, and must not
// hand that receipt to a principal other than the one who made the original
// submission. `admission::reserve`'s completed-lookup only ever answers
// `true` for a NEAR-namespaced tenant (`near-`/`nearai-`) with a provisioned
// admission anchor (`admission::anchor`) -- every other tenant gets `Ok(None)`
// from `reserve` and never reaches the branch these bugs are in. Reaching
// that combination for real needs the same NEAR evidence/witness
// verification chain
// `admission_pg_tests::actual_postgres_challenge_witness_ingest_and_terminal_retry`
// exercises, reused here through its `pub(super)` exports
// (`FixtureSigner`, `FixtureEnclave`, `provision_synthetic_near_account`,
// `provision_second_device_on_the_same_account`) rather than a second copy.
// ----------------------------------------------------------------------------

/// The same `AdmissionLimits` value for the two tests in this file that
/// reserve admission for real, `real_http_pipeline_receipt_replays_on_retry`
/// and `real_http_pipeline_receipt_refuses_a_different_devices_retry`: the
/// global budget row (`trace_admission_global_budget`) is a `PostgreSQL`
/// singleton, not tenant-scoped, so two tests reserving in the same
/// database must agree on `global_cost_limit` or the second one's
/// reservation answers `configuration_changed` instead of `reserved`.
fn o1_admission_limits() -> AdmissionLimits {
    AdmissionLimits {
        window_attempts: 1,
        account_cost_limit: 100,
        global_cost_limit: 1000,
        processing_cost_bound: 10,
        lease_seconds: 60,
        challenge_ttl_seconds: 60,
    }
}

/// A `PipelineService` with no crash point, built through the production
/// injection seam (`assemble_test_pipeline_service`) every test in this file
/// must use.
fn o1_pipeline_service(backend: Arc<PgBackend>, dir: &tempfile::TempDir) -> Arc<PipelineService> {
    let index = IsolatedPipelineIndex::new();
    let storage_rebate = RecordingSettlementAdapter::new(
        InstrumentId::new("storage_rebate").unwrap(),
        "recording_storage_rebate_o1_test_only",
        "none",
    );
    let trace_credit = RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_o1_test_only",
        "none",
    );
    let adapters: Vec<Arc<dyn SettlementAdapter>> = vec![
        storage_rebate as Arc<dyn SettlementAdapter>,
        trace_credit as Arc<dyn SettlementAdapter>,
    ];
    assemble_test_pipeline_service(backend, local_artifacts(dir), index, adapters, None)
}

/// The principal a static token resolves to, independent of tenant (static
/// token principal refs are a function of the token alone --
/// `static_token_principal_refs`) -- computed through a throwaway tenant
/// before the real one is known, the same trick
/// `admission_pg_tests::principal_for` uses (not shared with it: this is
/// six mechanical lines, not the larger fixture logic that must not be
/// duplicated).
fn o1_principal_for(token: &str) -> String {
    let mut throwaway = BTreeMap::new();
    insert_token(
        &mut throwaway,
        "near-o1-placeholder-tenant",
        token,
        TokenRole::Contributor,
    );
    throwaway.get(token).unwrap().principal_ref.clone()
}

/// One witness-verified admission upload: issues a fresh challenge directly
/// through `admission::challenge_handler` (a plain function call, not an HTTP
/// round trip -- the challenge nonce itself is not what a caller's test is
/// about), witnesses a fresh contribution against it, and signs a matching
/// provider-TEE receipt. This is the same construction
/// `admission_pg_tests::actual_postgres_challenge_witness_ingest_and_terminal_retry`
/// does inline for its own accepted submission, built here from its shared
/// `pub(super)` `FixtureSigner`/`FixtureEnclave` and provisioning fixtures
/// -- those are shared, not duplicated, but this upload construction is
/// its own copy of that inline one.
///
/// Returns the envelope bytes and the evidence/witness request headers ready
/// to POST to `/v1/traces` (the caller still adds its own auth header), and
/// the evidence's `redaction_policy_version`, which `state.witness_bypass`
/// must allow before any of these headers can verify.
///
/// `submission_id`, when given, overrides the witnessed envelope's own id.
/// `RawTraceContribution::submission_id` is settable before witnessing, so
/// the resulting certificate legitimately covers a chosen id -- for a second
/// upload that must collide with a first one's submission id while
/// genuinely differing in content (not a same-bytes replay).
///
/// Callable both before and after the real server starts: it only reads
/// `state` (`admission::challenge_handler`) and does not touch
/// `state.witness_bypass` itself, so it is safe to call again on a cloned
/// `Arc` handle once the server already holds its own clone -- unlike
/// mutating `state.witness_bypass`, which must happen before the server
/// starts (`Arc::make_mut` would otherwise silently mutate an orphaned copy).
#[allow(clippy::too_many_arguments)]
async fn evidenced_upload(
    state: &Arc<AppState>,
    token: &str,
    anchor: &str,
    signer: &Arc<admission_pg_tests::FixtureSigner>,
    provider: &ring::signature::Ed25519KeyPair,
    provider_key: &str,
    trust: AdmissionProviderTrust,
    submission_id: Option<Uuid>,
    content_seed: &str,
) -> (Vec<u8>, HeaderMap, String) {
    let challenge_response =
        admission::challenge_handler(State(state.clone()), auth_headers(token))
            .await
            .expect("admission challenge succeeds");
    let challenge_body = to_bytes(challenge_response.into_body(), 1024 * 1024)
        .await
        .expect("challenge response body bytes");
    let challenge_value: serde_json::Value =
        serde_json::from_slice(&challenge_body).expect("challenge response body json");
    let binding = AdmissionBinding::parse(challenge_value["binding"].as_str().unwrap()).unwrap();
    assert_eq!(binding.account_anchor_sha256, anchor);

    let request_body = serde_json::json!({
        "model": "synthetic-model",
        "metadata": {REQUEST_METADATA_KEY: binding.encode().unwrap()},
        "messages": [{"role": "user", "content": format!("please summarize {content_seed}")}],
    })
    .to_string();
    let response_body = format!("{{\"answer\":\"{content_seed} succeeded\"}}");
    let receipt_text = format!(
        "synthetic-model:{}:{}",
        hash_hex(request_body.as_bytes()),
        hash_hex(response_body.as_bytes())
    );
    let receipt = trace_commons_server::near_attestation::receipt::ReceiptPayload {
        text: receipt_text.clone(),
        signature: hex::encode(provider.sign(receipt_text.as_bytes()).as_ref()),
        signing_address: provider_key.to_string(),
        signing_algo: trace_commons_server::near_attestation::receipt::ReceiptAlgo::Ed25519,
        signature_kind:
            trace_commons_server::near_attestation::receipt::ReceiptSignatureKind::ProviderTee,
    };

    let mut raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: format!("summarize {content_seed}"),
            response: None,
            tool_calls: Vec::new(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            state: Some("Completed".into()),
        }],
        RecordedTraceContributionOptions {
            include_message_text: true,
            pseudonymous_contributor_id: Some("sha256:synthetic-admission".into()),
            ..Default::default()
        },
    );
    if let Some(submission_id) = submission_id {
        raw.submission_id = submission_id;
    }
    raw.ironclaw
        .feature_flags
        .insert("agent".into(), "opencode".into());
    let mut event = raw.events.last().unwrap().clone();
    event.event_id = Uuid::new_v4();
    event.event_type = TraceContributionEventType::HttpExchange;
    event.content = Some(response_body.clone());
    event.structured_payload = serde_json::json!({
        "request": {"method": "POST", "body": request_body},
        "response": {"status": 200},
    });
    raw.events = vec![event];

    let witness = witness_service::surface::WitnessService::new(
        Arc::new(witness_service::DeterministicRedaction::new(Vec::new())),
        signer.clone(),
        Arc::new(admission_pg_tests::FixtureEnclave(signer.address())),
        1024 * 1024,
    )
    .with_contribution_redactor(Arc::new(
        witness_service::PipelineContributionRedaction::deterministic_only(Vec::new()),
    ))
    .with_admission_provider_trust(trust);
    let (witnessed, evidence, evidence_signature) = witness
        .witness_admission_contribution(witness_service::WitnessContributionRequest {
            raw_contribution: raw,
            granted: witness_service::GrantedConsent {
                scopes: vec![
                    ConsentScope::DebuggingEvaluation,
                    ConsentScope::ModelTraining,
                ],
                uses: vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation],
            },
            offered_receipt: Some(receipt),
        })
        .await
        .expect("witness admission contribution succeeds");

    let mut headers = HeaderMap::new();
    headers.insert(
        trace_commons_server::redaction_witness::request::CERTIFICATE_HEADER,
        serde_json::to_string(&witness_service::http::certificate_json(
            &witnessed.certificate,
            witnessed.residual_risk_verdict(),
        ))
        .unwrap()
        .parse()
        .unwrap(),
    );
    headers.insert(
        trace_commons_server::redaction_witness::request::SIGNATURE_HEADER,
        witnessed.signature_hex.parse().unwrap(),
    );
    headers.insert(
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        serde_json::to_string(&evidence).unwrap().parse().unwrap(),
    );
    headers.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        evidence_signature.parse().unwrap(),
    );

    (
        witnessed.envelope_bytes,
        headers,
        evidence.redaction_policy_version,
    )
}

/// Builds the fresh Ed25519 provider keypair, its hex-encoded public key,
/// the `FixtureSigner`, and the `AdmissionProviderTrust` that trusts it --
/// the same fixture identity
/// `admission_pg_tests::actual_postgres_challenge_witness_ingest_and_terminal_retry`
/// builds for itself, constructed fresh per test in this file so the tests
/// stay independent of each other.
fn o1_evidence_identity() -> (
    ring::signature::Ed25519KeyPair,
    String,
    Arc<admission_pg_tests::FixtureSigner>,
    AdmissionProviderTrust,
) {
    use ring::signature::KeyPair as _;
    let provider = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let provider_key = hex::encode(provider.public_key().as_ref());
    let signer = Arc::new(admission_pg_tests::FixtureSigner::new(
        "o1-pipeline-replay-witness",
    ));
    let trust = AdmissionProviderTrust::new(
        [provider_key.clone()],
        Vec::new(),
        ["synthetic-model".into()],
        1,
    )
    .unwrap();
    (provider, provider_key, signer, trust)
}

/// The rule this test proves: the same upload posted twice must return the
/// same receipt. The first POST is a real upload through the HTTP handler
/// (it takes `route_pipeline_receipt` and creates the run, via a real,
/// evidence-verified admission reservation, not a direct
/// `PipelineService::submit` call), and the second POST of the same bytes
/// takes the completed-admission branch.
///
/// Also covers the same key with different content after completion
/// returning 409: a third POST, with fresh evidence for a genuinely
/// different envelope forced to the same submission id, is refused by
/// `admission::reserve` itself before it can ever reach the pipeline's own
/// `ContentConflict` branch -- see the comment on that branch in
/// `submit_trace_handler`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_pipeline_receipt_replays_on_retry() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let url = pipeline_http_database_url()
        .await
        .expect("checked by runtime_backend, which already returned Some");
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .expect("connect as the migration owner for the near-account fixture rows");

    let token = "near-retry-fixture-token";
    let principal = o1_principal_for(token);
    let (tenant, anchor, _device) =
        admission_pg_tests::provision_synthetic_near_account(&admin, &principal).await;
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, token, TokenRole::Contributor);

    let dir = tempfile::tempdir().expect("temp dir");
    let service = o1_pipeline_service(backend.clone(), &dir);
    let (provider, provider_key, signer, trust) = o1_evidence_identity();

    let mut state = test_state_with_tokens(dir.path().to_path_buf(), tokens);
    let state_mut = Arc::make_mut(&mut state);
    state_mut.db_mirror = Some(backend.clone() as Arc<dyn Database>);
    state_mut.require_db_mirror_writes = true;
    state_mut.accept_medium_risk_submissions = true;
    state_mut.pipeline_service = Some(service);
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[tenant.as_str()],
    );
    state_mut.admission = Some(admission::AdmissionConfig {
        limits: o1_admission_limits(),
        providers: trust.clone(),
    });

    // Build the first, accepted upload's bytes and evidence headers, then set
    // `state.witness_bypass` from its `redaction_policy_version` -- both
    // before the server starts (`evidenced_upload` mutates nothing, but
    // `witness_bypass` below does, and `Arc::make_mut` must still see a
    // refcount of one).
    let (body_1, evidence_headers_1, policy_version) = evidenced_upload(
        &state,
        token,
        &anchor,
        &signer,
        &provider,
        &provider_key,
        trust.clone(),
        None,
        "the first upload",
    )
    .await;
    Arc::make_mut(&mut state).witness_bypass =
        trace_commons_server::redaction_witness::config::witness_bypass_config_from_values(
            Some("true"),
            Some(&signer.address()),
            Some("synthetic-admission-measurement"),
            Some(&policy_version),
            None,
        )
        .unwrap();
    // A second handle to call `admission::challenge_handler` again for the
    // third POST below, once the server already owns its own clone.
    let state_handle = state.clone();

    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();

    // First POST: a real upload, with real evidence. Takes
    // `route_pipeline_receipt` and creates the pipeline run.
    let first = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token)))
        .headers(reqwest_headers(evidence_headers_1))
        .header("content-type", "application/json")
        .body(body_1.clone())
        .send()
        .await
        .expect("first, evidence-verified upload over real HTTP");
    assert_eq!(first.status(), 200, "first upload must be accepted");
    let first_receipt: serde_json::Value = first.json().await.expect("first receipt body");
    assert_eq!(first_receipt["status"], "processing");

    // Second POST: the exact same bytes, no evidence headers -- a terminal
    // retry. `admission::reserve` finds the ledger already `completed` for
    // this exact body hash, so the completed-admission branch must replay
    // the pipeline receipt rather than reading the legacy file record the
    // pipeline never wrote.
    let second = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token)))
        .header("content-type", "application/json")
        .body(body_1.clone())
        .send()
        .await
        .expect("second retry over real HTTP");
    assert_eq!(second.status(), 200, "second retry must replay, not 500");
    let second_receipt: serde_json::Value = second.json().await.expect("second receipt body");
    assert_eq!(
        first_receipt, second_receipt,
        "both retries must return the exact same pipeline receipt"
    );

    // Third POST: a genuinely different, independently witnessed
    // envelope, forced to the same submission id, with its own fresh
    // evidence. `admission::reserve`'s own completed-lookup is keyed on
    // body_hash, so this never reads as a terminal retry of a `completed`
    // submission; `reserve_submission_admission`'s conflict check
    // (`prior.body_hash <> p_body`) answers it directly.
    let (body_2, evidence_headers_2, _) = evidenced_upload(
        &state_handle,
        token,
        &anchor,
        &signer,
        &provider,
        &provider_key,
        trust,
        Some(first_receipt_submission_id(&body_1)),
        "a different upload",
    )
    .await;
    let conflict = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token)))
        .headers(reqwest_headers(evidence_headers_2))
        .header("content-type", "application/json")
        .body(body_2)
        .send()
        .await
        .expect("conflicting upload over real HTTP");
    assert_eq!(conflict.status(), 409, "changed content must be refused");
    let conflict_body: serde_json::Value = conflict.json().await.expect("conflict body");
    assert_eq!(conflict_body["error"], "admission_identity_conflict");

    let run_count: i64 = admin
        .trace_pool_for_test()
        .get()
        .await
        .expect("admin client to count runs")
        .query_one(
            "SELECT COUNT(*) FROM pipeline_runs WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .expect("count pipeline_runs")
        .get(0);
    assert_eq!(
        run_count, 1,
        "neither retry nor the refused conflict may create a second pipeline run"
    );

    stop.send(()).expect("send shutdown");
    join_within(server, 20, "replay-on-retry test server").await;
}

/// The `submission_id` a `TraceContributionEnvelope`'s raw JSON bytes carry,
/// read back out rather than threaded through as a separate value --
/// `evidenced_upload`'s first call does not return the envelope it built,
/// only its bytes and headers.
fn first_receipt_submission_id(envelope_bytes: &[u8]) -> Uuid {
    let envelope: TraceContributionEnvelope =
        serde_json::from_slice(envelope_bytes).expect("envelope bytes parse");
    envelope.submission_id
}

/// A second device on the same NEAR account shares its admission anchor
/// (V58 keys `trace_near_provisioned_devices` on `(tenant_id,
/// principal_ref)`; nothing makes `anchor_hash` unique per principal), so
/// `admission::reserve`'s completed-lookup -- keyed on `(tenant, anchor,
/// submission, body_hash)`, not on principal -- answers a retry from either
/// device's credential the same way. `admission::reserve`'s own comment says
/// the handler still checks ownership before returning the existing receipt,
/// and the completed-admission branch's pipeline lookup must enforce that
/// same check before it returns either pipeline outcome.
/// A second device retrying the first device's submission must get the
/// legacy branch's own 409 `admission_identity_conflict`, not the pipeline
/// receipt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_pipeline_receipt_refuses_a_different_devices_retry() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let url = pipeline_http_database_url()
        .await
        .expect("checked by runtime_backend, which already returned Some");
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .expect("connect as the migration owner for the near-account fixture rows");

    let token_a = "near-device-a-fixture-token";
    let token_b = "near-device-b-fixture-token";
    let principal_a = o1_principal_for(token_a);
    let principal_b = o1_principal_for(token_b);
    let (tenant, anchor, _device_a) =
        admission_pg_tests::provision_synthetic_near_account(&admin, &principal_a).await;
    let _device_b = admission_pg_tests::provision_second_device_on_the_same_account(
        &admin,
        &tenant,
        &anchor,
        &principal_b,
    )
    .await;
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, token_a, TokenRole::Contributor);
    insert_token(&mut tokens, &tenant, token_b, TokenRole::Contributor);

    let dir = tempfile::tempdir().expect("temp dir");
    let service = o1_pipeline_service(backend.clone(), &dir);
    let (provider, provider_key, signer, trust) = o1_evidence_identity();

    let mut state = test_state_with_tokens(dir.path().to_path_buf(), tokens);
    let state_mut = Arc::make_mut(&mut state);
    state_mut.db_mirror = Some(backend.clone() as Arc<dyn Database>);
    state_mut.require_db_mirror_writes = true;
    state_mut.accept_medium_risk_submissions = true;
    state_mut.pipeline_service = Some(service);
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[tenant.as_str()],
    );
    state_mut.admission = Some(admission::AdmissionConfig {
        limits: o1_admission_limits(),
        providers: trust.clone(),
    });

    let (body, evidence_headers, policy_version) = evidenced_upload(
        &state,
        token_a,
        &anchor,
        &signer,
        &provider,
        &provider_key,
        trust,
        None,
        "device a's upload",
    )
    .await;
    Arc::make_mut(&mut state).witness_bypass =
        trace_commons_server::redaction_witness::config::witness_bypass_config_from_values(
            Some("true"),
            Some(&signer.address()),
            Some("synthetic-admission-measurement"),
            Some(&policy_version),
            None,
        )
        .unwrap();

    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();

    let created = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token_a)))
        .headers(reqwest_headers(evidence_headers))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("device a's evidence-verified upload over real HTTP");
    assert_eq!(created.status(), 200, "device a's upload must be accepted");

    // Positive control: device A retrying its own submission still replays.
    let own_retry = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token_a)))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("device a's own retry over real HTTP");
    assert_eq!(
        own_retry.status(),
        200,
        "the owning device must still replay"
    );

    // Device B -- a different principal, same account/anchor -- retries
    // device A's submission id with the exact same bytes and no evidence.
    // `admission::reserve`'s completed-lookup matches (same tenant, same
    // anchor, same submission id, same body hash), so device B's retry also
    // reaches the completed-admission branch; the ownership check inside it
    // must refuse it.
    let other_device_retry = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token_b)))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("device b's retry over real HTTP");
    assert_eq!(
        other_device_retry.status(),
        409,
        "a different device on the same account must not get another device's receipt"
    );
    let other_device_body: serde_json::Value = other_device_retry
        .json()
        .await
        .expect("device b's response body");
    assert_eq!(other_device_body["error"], "admission_identity_conflict");

    stop.send(()).expect("send shutdown");
    join_within(server, 20, "different-device-retry test server").await;
}

/// `route_pipeline_receipt` (the plain-receipt branch a fresh submission or
/// its ordinary retry takes) must apply the same ownership check the
/// completed-admission branch of `submit_trace_handler` already applies. For
/// a pipeline-routed tenant, `read_submission_record` finds no legacy file
/// record -- the pipeline never writes one -- so the legacy
/// `can_access_submission` check never runs, and without an ownership check
/// of its own, `submit`'s `Replayed`/`ContentConflict` outcome would reach
/// the caller unchecked: a second principal posting the first principal's
/// submission id would get the first principal's receipt back on identical
/// bytes, or the generic "receipt id reused with different content" 409 on
/// changed bytes, never the ownership refusal.
///
/// Exercised with two static contributor tokens on one pipeline-routed
/// tenant (`token-a` and `token-a-2`, both already configured for
/// `tenant-a` by the default test fixture) and no NEAR admission
/// configured, so the plain-receipt branch is the one under test -- not the
/// completed-admission branch `real_http_pipeline_receipt_refuses_a_different_devices_retry`
/// already covers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_pipeline_receipt_checks_ownership_on_replay() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    cleanup_pg_trace_tenant(&backend, "tenant-a").await;

    let dir = tempfile::tempdir().expect("temp dir");
    let service = o1_pipeline_service(backend.clone(), &dir);

    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        Some(backend.clone() as Arc<dyn Database>),
        Some(local_artifacts(&dir)),
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

    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();
    let mut envelope = sample_envelope().await;
    make_metadata_only_low_risk(&mut envelope);
    let body = serde_json::to_vec(&envelope).expect("envelope serialises");

    // Principal A (token-a) uploads and creates the run.
    let first = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers("token-a")))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("first principal's upload over real HTTP");
    assert_eq!(first.status(), 200, "the first upload must be accepted");
    let first_receipt: serde_json::Value = first.json().await.expect("first receipt body");
    assert_eq!(first_receipt["status"], "processing");

    // Principal B (token-a-2, a different principal on the same tenant)
    // posts the same submission id with the exact same bytes: this must be
    // refused as an ownership conflict, not replayed as if B owned the run.
    let same_bytes_from_b = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers("token-a-2")))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("second principal's same-bytes retry over real HTTP");
    assert_eq!(
        same_bytes_from_b.status(),
        409,
        "a different principal replaying someone else's submission id must be refused"
    );
    let same_bytes_body: serde_json::Value = same_bytes_from_b
        .json()
        .await
        .expect("same-bytes conflict body");
    assert_eq!(
        same_bytes_body["error"],
        "submission id already belongs to another principal"
    );

    // Principal B, different bytes under the same submission id: still the
    // ownership refusal, not the generic content-conflict message.
    let mut changed = envelope.clone();
    changed.privacy.warnings.push("changed-by-b".to_string());
    let different_bytes_from_b = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers("token-a-2")))
        .header("content-type", "application/json")
        .body(serde_json::to_vec(&changed).expect("changed envelope serialises"))
        .send()
        .await
        .expect("second principal's different-bytes retry over real HTTP");
    assert_eq!(
        different_bytes_from_b.status(),
        409,
        "a different principal posting different bytes under someone else's \
         submission id must also be refused as an ownership conflict"
    );
    let different_bytes_body: serde_json::Value = different_bytes_from_b
        .json()
        .await
        .expect("different-bytes conflict body");
    assert_eq!(
        different_bytes_body["error"],
        "submission id already belongs to another principal"
    );

    // Principal A's own retry is unaffected: still 200 with the same
    // receipt as the first upload.
    let owner_retry = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers("token-a")))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("owning principal's retry over real HTTP");
    assert_eq!(
        owner_retry.status(),
        200,
        "the owner's retry must still replay"
    );
    let owner_retry_receipt: serde_json::Value =
        owner_retry.json().await.expect("owner retry receipt body");
    assert_eq!(
        first_receipt, owner_retry_receipt,
        "the owner's retry must return the exact same pipeline receipt"
    );

    stop.send(()).expect("send shutdown");
    join_within(server, 20, "ownership-on-replay test server").await;
}

/// Requirement in the completed-admission branch when the tenant is routed
/// but no pipeline run exists yet: falls back to the legacy record read,
/// unchanged. Models a submission that completed admission on the legacy
/// path before this tenant was ever routed to the pipeline: the legacy file
/// record exists, no pipeline run does, and the retry must still replay the
/// legacy receipt rather than erroring or trying the pipeline's own fixed
/// receipt.
///
/// No direct `PipelineService` call: the admission-ledger `completed` row is
/// stood up directly (see `mark_admission_completed`'s own doc comment for
/// why -- reaching this exact combination, an already-completed submission
/// with no pipeline run, through the real evidence-verified `/v1/traces`
/// route is not possible for a tenant this test also routes to the
/// pipeline, since a real upload would create the run this test needs
/// absent).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_http_pipeline_receipt_falls_back_to_legacy_record_without_a_run() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let url = pipeline_http_database_url()
        .await
        .expect("checked by runtime_backend, which already returned Some");
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .expect("connect as the migration owner for the near-account fixture rows");

    let token = "near-legacy-fallback-fixture-token";
    let principal = o1_principal_for(token);
    let (tenant, anchor, _device) =
        admission_pg_tests::provision_synthetic_near_account(&admin, &principal).await;
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, token, TokenRole::Contributor);

    let mut envelope = sample_envelope().await;
    make_metadata_only_low_risk(&mut envelope);
    let body = serde_json::to_vec(&envelope).expect("envelope serialises");
    mark_admission_completed(&admin, &tenant, &anchor, envelope.submission_id, &body).await;

    let dir = tempfile::tempdir().expect("temp dir");
    // No pipeline run is seeded for this key -- `replay_receipt` must answer
    // `None`. A legacy file record stands in for one written before this
    // tenant was routed.
    let record: TraceCommonsSubmissionRecord = serde_json::from_value(serde_json::json!({
        "tenant_id": tenant,
        "tenant_storage_ref": tenant_storage_ref(&tenant),
        "auth_principal_ref": principal,
        "submission_id": envelope.submission_id,
        "trace_id": envelope.trace_id,
        "status": "accepted",
        "privacy_risk": "low",
        "submission_score": 0.0,
        "credit_points_pending": 0.0,
        "consent_scopes": [],
        "received_at": chrono::Utc::now().to_rfc3339(),
        "object_key": "obj/o1-legacy-fallback-fixture",
    }))
    .expect("legacy submission record fixture deserialises");
    write_submission_record(dir.path(), &record).expect("write the legacy record fixture");

    let service = o1_pipeline_service(backend.clone(), &dir);

    let mut state = test_state_with_tokens(dir.path().to_path_buf(), tokens);
    let state_mut = Arc::make_mut(&mut state);
    state_mut.db_mirror = Some(backend.clone() as Arc<dyn Database>);
    state_mut.require_db_mirror_writes = true;
    state_mut.accept_medium_risk_submissions = true;
    state_mut.pipeline_service = Some(service);
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[tenant.as_str()],
    );
    state_mut.admission = Some(admission::AdmissionConfig {
        limits: o1_admission_limits(),
        providers: AdmissionProviderTrust::new(
            [hash_hex(Uuid::new_v4().as_bytes())],
            Vec::new(),
            ["synthetic-model".into()],
            1,
        )
        .expect("provider trust fixture value (never exercised by this terminal retry)"),
    });

    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token)))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("retry over real HTTP");
    assert_eq!(response.status(), 200);
    let receipt: serde_json::Value = response.json().await.expect("receipt body");
    assert_eq!(receipt["status"], "accepted");

    stop.send(()).expect("send shutdown");
    join_within(server, 20, "legacy-fallback test server").await;
}

/// Marks `submission_id` `completed` in the admission ledger directly, by
/// inserting the row `admission::reserve`'s terminal-retry read
/// (`lookup_completed_submission_admission`, a plain `SELECT`) expects to
/// find -- never through `trace_reserve_admission`/`trace_transition_admission`
/// (the real reservation path, exercised for real by the other two tests in
/// this section through `evidenced_upload`). This one models an admission
/// row that reached `completed` before this tenant was ever pipeline-routed,
/// with no pipeline run to match -- a state a real upload cannot produce for
/// a tenant this test also routes to the pipeline, since the upload itself
/// would create the run. `kind = 'window'` here for the same reason: no
/// evidence was verified.
async fn mark_admission_completed(
    admin: &PgBackend,
    tenant: &str,
    anchor: &str,
    submission_id: Uuid,
    body: &[u8],
) {
    let client = admin
        .trace_pool_for_test()
        .get()
        .await
        .expect("admin client for the completed-admission fixture row");
    client
        .execute(
            "INSERT INTO trace_admission_submissions
               (tenant_id, submission_id, anchor_hash, body_hash, kind, receipt_hash, challenge_hash,
                status, lease_id, lease_expires_at, last_cost_bound, attempt_held, ever_processed)
             VALUES ($1, $2, $3, $4, 'window', NULL, NULL, 'completed', $5, NOW(), $6, FALSE, TRUE)",
            &[
                &tenant,
                &submission_id,
                &anchor,
                &hash_hex(body),
                &Uuid::new_v4(),
                &10i64,
            ],
        )
        .await
        .expect("insert fixture trace_admission_submissions row");
}
