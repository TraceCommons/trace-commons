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
pub(super) mod pilot_runtime_login;

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::State;
use trace_commons_gate_api::pipeline::{
    AtomicUnits, InstrumentDescriptor, InstrumentId, InstrumentKind, Microcredits,
};
use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer, SettlementAdapter};
use trace_commons_protocol::admission::{AdmissionBinding, REQUEST_METADATA_KEY, hash_hex};
use trace_commons_protocol::trace_contribution::{
    RawTraceCaptureTurn, RawTraceContribution, TraceContributionEventType,
};
use trace_commons_server::admission_evidence::AdmissionProviderTrust;
use trace_commons_server::admission_ledger::AdmissionLimits;
use trace_commons_server::trace_authority::{SubmissionAllowlists, SubmissionAuthority};
use trace_commons_server::versioned_pipeline::{
    PipelineCaps, PipelineCrashPoint, PipelinePayoutConfig, PipelineServiceBuilder,
};
use trace_commons_server::versioned_pipeline_authority::{
    PipelinePrivacyBoundary, StaticPipelineAuthorityProvider,
};
use trace_commons_server::versioned_pipeline_bundle::{
    MINIMAL_INDEX_ID, MinimalPolicyBundle, PipelineBundleConfig, PipelineInstrumentAwardConfig,
};
use trace_commons_server::versioned_pipeline_compat::{
    COMPATIBILITY_SCORE_RULE, CompatibilityBundleConfig,
};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingNearAdapter, RecordingSettlementAdapter, SettlementAdapterRegistry,
};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_qualification::PipelineCheckEmitter;
use trace_commons_server::witness_service;

/// This suite's own runtime login, distinct from `trace_pipeline_runtime_test`
/// (`tests/versioned_pipeline_runtime_pg.rs`). Like that one, its only
/// privilege sources are membership in `trace_ingest_runtime`, the ingest
/// runtime group V90 names, and in `trace_account_admission_runtime` (V77).
pub(super) const PIPELINE_HTTP_RUNTIME_ROLE: &str = "trace_pipeline_http_runtime_test";
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
                // legacy_withdrawal_route_uses_the_pipeline_for_a_session_with_a_run
                // and pipeline_withdrawal_route_withdraws_through_the_account_session
                // withdraw through a mapped source session: main's withdrawal
                // path reads trace_account_admission_submissions, which only
                // trace_account_admission_runtime may read (owner ruling RB-11).
                // Granted before the login's membership check runs.
                pilot_runtime_login::provision_runtime_login(
                    &pilot_url,
                    PIPELINE_HTTP_RUNTIME_ROLE,
                    &["trace_account_admission_runtime"],
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
pub(super) async fn runtime_backend(pool_size: usize) -> Option<Arc<PgBackend>> {
    let url = pipeline_http_database_url().await?;
    Some(runtime_backend_at(&url, pool_size).await)
}

/// `runtime_backend` for a database the caller names: connects to `url` as
/// this suite's runtime login and checks that the login is neither
/// `SUPERUSER` nor `BYPASSRLS`. The restore drill's resume
/// (`pipeline_restore_pg_tests`) connects to the restored database this way.
pub(super) async fn runtime_backend_at(url: &str, pool_size: usize) -> Arc<PgBackend> {
    let mut runtime_url = reqwest::Url::parse(url).expect("parse test URL");
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
    Arc::new(backend)
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

/// A privacy boundary that transforms nothing and finds nothing -- Ruling
/// T2-4's harness default, the same test double as
/// `versioned_pipeline_runtime_pg.rs`'s `PassThroughPipelinePrivacyBoundary`
/// (test doubles live in the test files, so this file holds its own copy
/// rather than sharing one).
pub(super) struct PassThroughPipelinePrivacyBoundary;

#[async_trait::async_trait]
impl PipelinePrivacyBoundary for PassThroughPipelinePrivacyBoundary {
    async fn rescrub(
        &self,
        _envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        Ok(Vec::new())
    }

    fn dependency_identity(&self) -> &str {
        "pass_through_privacy_test_only"
    }

    fn is_production_compatible(&self) -> bool {
        false
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
        let package = minimal_storage_rebate_package(scorer.as_ref(), embedder.as_ref())?;
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
        .with_object_store_name(context.object_store_name)
        .with_novelty_utility_checks(context.novelty_utility_checks)
        .with_authority(allow_all_test_authority())
        .with_privacy(Arc::new(PassThroughPipelinePrivacyBoundary));
        if let Some(crash_point) = self.crash_point {
            builder = builder.with_crash_point(crash_point);
        }
        Ok(Arc::new(builder.build()?))
    }
}

/// PR 2's minimal package with the `storage_rebate` award (5 whole units),
/// naming `scorer` and `embedder`: the package `TestAssembler` serves, and
/// the `minimal` bundle of `pipeline.py run` (`pipeline_corpus_pg_tests`).
pub(super) fn minimal_storage_rebate_package(
    scorer: &ReferencePerplexityScorer,
    embedder: &ReferenceEmbedder,
) -> anyhow::Result<trace_commons_gate_api::pipeline::BundlePackage> {
    MinimalPolicyBundle::minimal_package(
        &PipelineBundleConfig {
            instrument_awards: vec![PipelineInstrumentAwardConfig {
                instrument_id: "storage_rebate".into(),
                atomic_units: AtomicUnits::from_raw(5),
                descriptor: storage_rebate_descriptor(),
            }],
            include_index: true,
            variant: None,
        },
        scorer,
        embedder,
    )
}

/// The compatibility package over `CompatibilityBundleConfig::local_reference()`
/// with the given `NoveltyUtility` delta, naming `scorer` and `embedder`: the
/// package `CompatibilityTestAssembler` serves, and (with 2_500_000) the
/// `compatibility` bundle of `pipeline.py run` (`pipeline_corpus_pg_tests`).
pub(super) fn compatibility_reference_package(
    novelty_utility_microcredits: u64,
    scorer: &ReferencePerplexityScorer,
    embedder: &ReferenceEmbedder,
) -> anyhow::Result<trace_commons_gate_api::pipeline::BundlePackage> {
    let mut config = CompatibilityBundleConfig::local_reference();
    config.novelty_utility_microcredits = novelty_utility_microcredits;
    MinimalPolicyBundle::compatibility_package(&config, scorer, embedder)
}

/// The authority every test service in this file holds: each tenant gets
/// empty allowlists, which restrict nothing, and no tenant policy.
pub(super) fn allow_all_test_authority() -> Arc<StaticPipelineAuthorityProvider> {
    Arc::new(StaticPipelineAuthorityProvider::test_only(
        SubmissionAuthority {
            tenant: SubmissionAllowlists::default(),
            policy: None,
            require_policy: false,
        },
    ))
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
        None,
        TEST_NEAR_CONFIRMATION_INTERVAL,
        TEST_NEAR_PAYOUT_CONTROLS,
        &PipelineNoveltyUtilityChecks::default(),
        TEST_EMBED_INSERT_NOVELTY_MICROS,
    )
    .expect("assemble the injected pipeline runtime")
    .expect("an assembler was given, so a service is returned")
}

/// Opens a tenant-scoped transaction the way every raw-SQL helper below
/// needs one: `set_config('trace_commons.trace_tenant_id', ...)` first, so
/// RLS admits only `tenant_id`'s own rows.
pub(super) async fn tenant_tx<'a>(
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
pub(super) async fn wait_for_settle_selection(
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
pub(super) async fn wait_for_run_complete(
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
pub(super) async fn wait_for_pipeline_ready(client: &reqwest::Client, base: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let response = client
            .get(format!("{base}/v1/pipeline/readiness"))
            .send()
            .await
            .expect("readiness request");
        if response.status() == reqwest::StatusCode::OK {
            let body: serde_json::Value = response.json().await.expect("readiness body");
            if body == serde_json::json!({"status": "ready", "drain_tenant_count": 0}) {
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
pub(super) async fn expire_run_lease(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: uuid::Uuid,
) {
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
pub(super) async fn serve_pipeline_app(
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
pub(super) async fn join_within(
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
    let resumed_state = start(None);
    let package = resumed_state
        .pipeline_service
        .as_ref()
        .expect("app 2 serves the injected pipeline service")
        .default_package()
        .clone();
    let (base, stop, server) = serve_pipeline_app(resumed_state).await;

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
    // carry the configured store's name, as a legacy receipt's do, and so
    // does the ref of the index command Score stored (Zaki review 1, item 1:
    // the minimal bundle stores no neighbour set).
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
            (
                "worker_intermediate".to_string(),
                TRACE_COMMONS_LEGACY_ENCRYPTED_OBJECT_STORE.to_string()
            ),
        ]
    );

    stop.send(()).expect("send shutdown to app 2");
    join_within(server, 20, "app 2").await;

    PipelineCheckEmitter::emit_pass_from_env(
        "pipeline_http_restart_recovery",
        Some(&package),
        serde_json::json!({
            "phase_outcomes": 4,
            "replay_status": 200,
            "conflict_status": 409,
            "bad_credential_status": 403,
            "other_tenant_rows": 0,
        }),
    );
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

/// Zaki review 1, fix round, item 5: a tenant rolled back from
/// `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` to
/// `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` still replays a pipeline receipt
/// for a retried upload that completed admission on the pipeline path: the
/// replay lookup reads the drain list as well as the receipts list. Without
/// it the retry falls through to the legacy record the pipeline never wrote
/// and answers 500 (`admission_record_unavailable`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drain_tenant_retry_replays_its_pipeline_receipt() {
    let Some(backend) = runtime_backend(4).await else {
        return;
    };
    let url = pipeline_http_database_url()
        .await
        .expect("checked by runtime_backend, which already returned Some");
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .expect("connect as the migration owner for the near-account fixture rows");

    let token = "near-drain-retry-fixture-token";
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
    let (body, evidence_headers, policy_version) = evidenced_upload(
        &state,
        token,
        &anchor,
        &signer,
        &provider,
        &provider_key,
        trust,
        None,
        "the drain tenant's upload",
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
    let mut rolled_back = state.clone();

    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();
    let first = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token)))
        .headers(reqwest_headers(evidence_headers))
        .header("content-type", "application/json")
        .body(body.clone())
        .send()
        .await
        .expect("first, evidence-verified upload over real HTTP");
    assert_eq!(first.status(), 200, "the routed upload is accepted");
    let first_receipt: serde_json::Value = first.json().await.expect("first receipt body");
    stop.send(()).expect("send shutdown");
    join_within(server, 20, "the routed app").await;

    // The rollback: the tenant leaves the receipts list for the drain list.
    let rolled_back_mut = Arc::make_mut(&mut rolled_back);
    rolled_back_mut.tenant_rollout_gates = TraceTenantRolloutGates::default();
    rolled_back_mut.pipeline_drain_tenant_ids = Arc::new(BTreeSet::from([tenant.clone()]));
    let (base, stop, server) = serve_pipeline_app(rolled_back).await;
    let retry = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token)))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("the retry over real HTTP");
    assert_eq!(
        retry.status(),
        200,
        "a drain tenant's retry replays, not 500"
    );
    let retry_receipt: serde_json::Value = retry.json().await.expect("retry receipt body");
    assert_eq!(retry_receipt, first_receipt);
    stop.send(()).expect("send shutdown");
    join_within(server, 20, "the rolled-back app").await;
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
    let package = service.default_package().clone();

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

    PipelineCheckEmitter::emit_pass_from_env(
        "pipeline_http_receipt_ownership",
        Some(&package),
        serde_json::json!({
            "other_principal_refusals": 2,
            "refusal_status": 409,
            "owner_replay_status": 200,
            "owner_replay_same_receipt": true,
        }),
    );
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

// ---------------------------------------------------------------------------
// Withdrawal through the account session: the pipeline route and the legacy
// route on a submission with a pipeline run (Rulings T7-4, T7-7, T7-8).
//
// These tests drive the run to completion through the service built at the
// P5 seam (`assemble_test_pipeline_service`) and then exercise the handler
// or the router: what they test is the withdrawal routes, not the receipt.
// ---------------------------------------------------------------------------

/// The migration owner, with the login-resolver pool, for the account side
/// of the withdrawal tests: minting and redeeming an account session needs
/// that pool (`docs/operator/login-resolver-role.md`), which the runtime-role
/// backend above does not configure. The pipeline service itself still runs
/// on the runtime role. Both connect to `pipeline_http_database_url`, the
/// database the runtime login runs in, and so does the resolver pool: the
/// resolver URL the database configuration reads names the configured test
/// database, so its database is replaced with that one. Panics on any setup
/// failure once `TRACE_COMMONS_PG_TEST_DATABASE_URL` is set.
pub(super) async fn account_owner_backend() -> Option<Arc<PgBackend>> {
    let url = pipeline_http_database_url().await?;
    let login_resolver_url = DatabaseConfig::login_resolver_url_from_env().map(|resolver| {
        let mut resolver_url =
            reqwest::Url::parse(resolver.expose_secret()).expect("parse the resolver URL");
        resolver_url.set_path(
            reqwest::Url::parse(&url)
                .expect("parse the suite's database URL")
                .path(),
        );
        SecretString::from(resolver_url.to_string())
    });
    let config = DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    let backend = PgBackend::new(&config)
        .await
        .expect("connect as the migration owner");
    backend.run_migrations().await.expect("apply migrations");
    reset_account_rate_limiter_for_db_test().await;
    Some(Arc::new(backend))
}

/// `main`'s database connection for the `AppState` these tests build: the
/// suite's runtime login (`PIPELINE_HTTP_RUNTIME_ROLE`, `NOBYPASSRLS`, the
/// pilot ingest login's groups only) with the login resolver pool the
/// account routes use, so `main`'s half of every parity and HTTP test runs
/// under the grants and row-level security the pilot's ingest runs under,
/// never as the owner superuser (Zaki review 1, round 2, finding 17).
/// Fixture writes that are not the ingest runtime's go through
/// `account_owner_backend` instead.
pub(super) async fn mains_database() -> Arc<dyn Database> {
    let url = pipeline_http_database_url()
        .await
        .expect("the same variable runtime_backend read is set");
    mains_database_at(&url).await
}

/// `mains_database` for a database the caller names: the restore drill's
/// resume serves `main` from the restored database this way.
pub(super) async fn mains_database_at(url: &str) -> Arc<dyn Database> {
    let mut runtime_url = reqwest::Url::parse(url).expect("parse test URL");
    runtime_url
        .set_username(PIPELINE_HTTP_RUNTIME_ROLE)
        .expect("set runtime user");
    let login_resolver_url = DatabaseConfig::login_resolver_url_from_env().map(|resolver| {
        let mut resolver_url =
            reqwest::Url::parse(resolver.expose_secret()).expect("parse the resolver URL");
        resolver_url.set_path(runtime_url.path());
        SecretString::from(resolver_url.to_string())
    });
    let config = DatabaseConfig {
        url: SecretString::from(runtime_url.to_string()),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    let backend = PgBackend::new(&config)
        .await
        .expect("connect as the runtime login");
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
        "main's database connection must not bypass RLS"
    );
    Arc::new(backend)
}

/// One tenant with two contributor tokens (an owner and another account), an
/// `AppState` whose account side runs on the migration owner, and a pipeline
/// service on the runtime role, injected into the state.
struct WithdrawalFixture {
    state: Arc<AppState>,
    service: Arc<PipelineService>,
    owner: Arc<PgBackend>,
    runtime: Arc<PgBackend>,
    tenant: String,
    token: String,
    other_token: String,
    _dir: tempfile::TempDir,
}

async fn withdrawal_fixture() -> Option<WithdrawalFixture> {
    withdrawal_fixture_with(
        |runtime, artifacts| {
            assemble_test_pipeline_service(
                runtime,
                artifacts,
                IsolatedPipelineIndex::new(),
                vec![RecordingSettlementAdapter::new(
                    InstrumentId::new("storage_rebate").unwrap(),
                    "recording_storage_rebate_withdrawal_test_only",
                    "none",
                ) as Arc<dyn SettlementAdapter>],
                None,
            )
        },
        false,
    )
    .await
}

/// `withdrawal_fixture`, with the pipeline service `service` builds over the
/// runtime backend and the fixture's artifact store, and `main`'s reviewer
/// reads (credit events and settlement batches among them) from the
/// database when `db_reviewer_reads`.
async fn withdrawal_fixture_with(
    service: impl FnOnce(Arc<PgBackend>, Arc<LocalEncryptedTraceArtifactStore>) -> Arc<PipelineService>,
    db_reviewer_reads: bool,
) -> Option<WithdrawalFixture> {
    let runtime = runtime_backend(4).await?;
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-withdraw-{suffix}");
    let token = format!("token-withdraw-{suffix}");
    let other_token = format!("token-withdraw-other-{suffix}");
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, &token, TokenRole::Contributor);
    insert_token(&mut tokens, &tenant, &other_token, TokenRole::Contributor);
    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let service = service(runtime.clone(), artifacts.clone());
    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        Some(mains_database().await),
        Some(artifacts),
        false,
        db_reviewer_reads,
        false,
        false,
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.tokens = Arc::new(tokens);
    state_mut.pipeline_service = Some(service.clone());
    Some(WithdrawalFixture {
        state,
        service,
        owner,
        runtime,
        tenant,
        token,
        other_token,
        _dir: dir,
    })
}

/// A Low-risk receipt by `principal`, run through Review, Score, and Settle:
/// the run is complete and its index write is `complete`.
async fn completed_pipeline_run(
    service: &PipelineService,
    tenant: &str,
    principal: &str,
) -> trace_commons_server::versioned_pipeline::PipelineRunRecord {
    let mut envelope = sample_envelope().await;
    envelope.submission_id = Uuid::new_v4();
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
    let run = completed_run_of(service, tenant, principal, &envelope).await;
    assert_eq!(run.index_write_state, "complete");
    run
}

/// `envelope`, received from `principal` and run through Review, Score, and
/// Settle: the run is complete.
async fn completed_run_of(
    service: &PipelineService,
    tenant: &str,
    principal: &str,
    envelope: &TraceContributionEnvelope,
) -> trace_commons_server::versioned_pipeline::PipelineRunRecord {
    service
        .register_default_bundle(tenant)
        .await
        .expect("register the bundle");
    let raw = serde_json::to_vec(envelope).unwrap();
    let key = envelope.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(PipelineReceiptRequest {
            source_session: None,
            tenant_id: tenant,
            actor_principal_ref: principal,
            counts_toward_quota: true,
            request_idempotency_key: &key,
            request_bytes: &raw,
            server_envelope: envelope,
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
    for _ in 0..3 {
        service
            .process_run(tenant, created.run_id)
            .await
            .expect("the phase runs");
    }
    let run = service
        .store()
        .get_run(tenant, created.run_id)
        .await
        .unwrap()
        .expect("the run exists");
    assert_eq!(run.state, PipelineRunState::Complete);
    run
}

/// `(queued invalidations for the run, its index_invalidation_state)`.
async fn queued_index_invalidation(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    run_id: Uuid,
) -> (i64, String) {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let row = tx
        .query_one(
            "SELECT (SELECT COUNT(*) FROM pipeline_index_invalidations i
                      WHERE i.tenant_id = r.tenant_id AND i.run_id = r.run_id
                        AND i.state = 'pending' AND i.reason_code = 'withdrawn'),
                    r.index_invalidation_state
               FROM pipeline_runs r WHERE r.tenant_id = $1 AND r.run_id = $2",
            &[&tenant_id, &run_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    (row.get(0), row.get(1))
}

/// Owner ruling T7-8: `main`'s legacy route `POST /v1/account/traces/{id}/withdraw`
/// uses the pipeline withdrawal when the pipeline runtime is present and the
/// withdrawal reaches a pipeline run: the requested submission's own, or one
/// of another submission of its source session. Here the session holds a
/// pipeline submission whose index write is `complete` and a legacy sibling
/// with no run, and the legacy route is called once on each (a fresh tenant
/// each time). Either way it returns its own response shape, queues the index
/// invalidation of the pipeline revision, withdraws both submissions and the
/// session, and still does `main`'s file-side cleanup (the pipeline's objects
/// are deleted) and one hash-only revoke audit event per withdrawn
/// submission.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_withdrawal_route_uses_the_pipeline_for_a_session_with_a_run() {
    for request_the_pipeline_submission in [true, false] {
        let Some(fixture) = withdrawal_fixture().await else {
            return;
        };
        let state = &fixture.state;
        let tenant = fixture.tenant.as_str();
        let principal = static_token_principal_ref(&fixture.token);
        let session = account_session_headers(state, &fixture.token).await;
        let run = completed_pipeline_run(&fixture.service, tenant, &principal).await;
        let sibling =
            insert_account_test_submission(fixture.owner.as_ref(), tenant, &principal).await;
        let ext = account_ctx_ext(state, &session).await;
        let account_id = ext.0.account_id.as_uuid();
        let digest: [u8; 32] = Sha256::digest(tenant.as_bytes())
            .as_slice()
            .try_into()
            .unwrap();
        for submission_id in [run.submission_id, sibling] {
            assert_eq!(
                fixture
                    .owner
                    .claim_trace_source_session(tenant, account_id, &digest, submission_id)
                    .await
                    .unwrap(),
                StorageTraceSourceSessionStatus::Active
            );
        }
        let live_pipeline_objects = fixture
            .owner
            .list_trace_object_refs(tenant, run.submission_id)
            .await
            .unwrap();
        assert!(!live_pipeline_objects.is_empty());
        let requested = if request_the_pipeline_submission {
            run.submission_id
        } else {
            sibling
        };

        let Json(response) =
            account_trace_withdraw_handler(State(state.clone()), ext, AxumPath(requested))
                .await
                .expect("the owner withdraws a submission of the session");

        assert_eq!(response.submission_id, requested);
        assert_eq!(response.prior_status, "accepted");
        assert_eq!(response.distribution_reach, "commons_not_distributed");
        assert!(!response.already_distributed);
        assert!(response.credit_retained);
        assert_eq!(
            queued_index_invalidation(&fixture.runtime, tenant, run.run_id).await,
            (1, "pending".to_string()),
            "the index invalidation is queued (requesting the pipeline submission: \
             {request_the_pipeline_submission})"
        );
        assert_eq!(
            fixture
                .owner
                .get_trace_source_session_status(tenant, account_id, &digest)
                .await
                .unwrap(),
            StorageTraceSourceSessionStatus::Withdrawn
        );
        let audit_client = fixture.owner.trace_pool_for_test().get().await.unwrap();
        for submission_id in [run.submission_id, sibling] {
            assert!(
                fixture
                    .owner
                    .get_trace_withdrawal(tenant, submission_id)
                    .await
                    .unwrap()
                    .is_some(),
                "every submission of the session is withdrawn"
            );
            let revoke_events: i64 = audit_client
                .query_one(
                    "SELECT count(*) FROM trace_audit_events
                      WHERE submission_id = $1 AND action = 'revoke'",
                    &[&submission_id],
                )
                .await
                .unwrap()
                .get(0);
            assert_eq!(
                revoke_events, 1,
                "one hash-only revoke event per submission"
            );
        }
        for object_ref in fixture
            .owner
            .list_trace_object_refs(tenant, run.submission_id)
            .await
            .unwrap()
        {
            assert!(
                object_ref.deleted_at.is_some(),
                "main's file-side cleanup deleted every pipeline object"
            );
        }
    }
}

/// Owner ruling T7-8, the other side: with the pipeline runtime present, the
/// legacy route on a submission whose withdrawal reaches no pipeline run
/// takes `main`'s path unchanged -- even when the tenant has a pipeline run
/// of its own on another submission. The submission here has no source
/// session, is `quarantined`, and has an export membership. `main`'s path
/// for a submission with no session reports a submission that is not
/// `accepted` as `not_distributed` whatever its exports; the pipeline
/// withdrawal applies the session rule, under which any export makes it
/// `commons_distributed`. So the tier tells the two paths apart, and no
/// index invalidation is queued.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_withdrawal_route_keeps_mains_path_without_a_pipeline_run() {
    let Some(fixture) = withdrawal_fixture().await else {
        return;
    };
    let state = &fixture.state;
    let tenant = fixture.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.token);
    let session = account_session_headers(state, &fixture.token).await;
    let unrelated_run = completed_pipeline_run(&fixture.service, tenant, &principal).await;
    let legacy = insert_account_test_submission_with_status(
        fixture.owner.as_ref(),
        tenant,
        &principal,
        StorageTraceCorpusStatus::Quarantined,
    )
    .await;
    let legacy_record = fixture
        .owner
        .get_trace_submission(tenant, legacy)
        .await
        .unwrap()
        .expect("the legacy submission exists");
    let manifest_id = Uuid::new_v4();
    fixture
        .owner
        .upsert_trace_export_manifest(StorageTraceExportManifestWrite {
            tenant_id: tenant.to_string(),
            export_manifest_id: manifest_id,
            artifact_kind: StorageTraceObjectArtifactKind::ExportArtifact,
            purpose_code: None,
            audit_event_id: None,
            source_submission_ids: vec![legacy],
            source_submission_ids_hash: "sha256:manifest".to_string(),
            item_count: 1,
            generated_at: Utc::now(),
        })
        .await
        .expect("manifest writes");
    fixture
        .owner
        .upsert_trace_export_manifest_item(StorageTraceExportManifestItemWrite {
            tenant_id: tenant.to_string(),
            export_manifest_id: manifest_id,
            submission_id: legacy,
            trace_id: legacy_record.trace_id,
            derived_id: None,
            object_ref_id: None,
            vector_entry_id: None,
            source_status_at_export: StorageTraceCorpusStatus::Quarantined,
            source_hash_at_export: "sha256:source".to_string(),
        })
        .await
        .expect("manifest item writes");
    let ext = account_ctx_ext(state, &session).await;

    let Json(response) =
        account_trace_withdraw_handler(State(state.clone()), ext, AxumPath(legacy))
            .await
            .expect("the owner withdraws the legacy submission");

    assert_eq!(response.submission_id, legacy);
    assert_eq!(response.prior_status, "quarantined");
    assert_eq!(
        response.distribution_reach, "not_distributed",
        "main's rule for a submission with no session"
    );
    assert!(!response.already_distributed);
    let tombstone = fixture
        .owner
        .get_trace_withdrawal(tenant, legacy)
        .await
        .unwrap()
        .expect("the withdrawal row is written");
    assert_eq!(tombstone.prior_status, "quarantined");
    assert_eq!(tombstone.distribution_reach, "not_distributed");
    assert_eq!(tombstone.withdrawn_at, response.withdrawn_at);
    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant).await;
    let invalidations: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM pipeline_index_invalidations WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    assert_eq!(invalidations, 0, "no pipeline index invalidation is queued");
    assert_eq!(
        queued_index_invalidation(&fixture.runtime, tenant, unrelated_run.run_id).await,
        (0, "none".to_string()),
        "the tenant's unrelated pipeline run is untouched"
    );
}

/// Ruling T7-4: `POST /v1/contributors/me/pipeline-submissions/{id}/withdraw`
/// authenticates as `main`'s legacy withdrawal route does. A device bearer is
/// refused; another account's session gets the same 404 as a missing id; the
/// owner's session withdraws and gets the legacy response shape plus the
/// pipeline follow-up states. Without a pipeline runtime the route is 404.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_withdrawal_route_withdraws_through_the_account_session() {
    use tower::ServiceExt;

    let Some(fixture) = withdrawal_fixture().await else {
        return;
    };
    let state = &fixture.state;
    let tenant = fixture.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.token);
    let session = account_session_headers(state, &fixture.token).await;
    let other_session = account_session_headers(state, &fixture.other_token).await;
    let run = completed_pipeline_run(&fixture.service, tenant, &principal).await;
    let post = |state: Arc<AppState>, submission_id: Uuid, headers: HeaderMap| async move {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/contributors/me/pipeline-submissions/{submission_id}/withdraw"
            ))
            .body(axum::body::Body::empty())
            .unwrap();
        request.headers_mut().extend(headers);
        let response = app(state).oneshot(request).await.unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1 << 16).await.unwrap();
        (status, body)
    };

    let (status, _) = post(
        state.clone(),
        run.submission_id,
        auth_headers(&fixture.token),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "a device key cannot withdraw"
    );
    let (not_owned, not_owned_body) =
        post(state.clone(), run.submission_id, other_session.clone()).await;
    let (missing, missing_body) = post(state.clone(), Uuid::new_v4(), other_session).await;
    assert_eq!(not_owned, StatusCode::NOT_FOUND);
    assert_eq!(missing, StatusCode::NOT_FOUND);
    assert_eq!(
        not_owned_body, missing_body,
        "not owned and not found are indistinguishable"
    );
    let mut without_runtime = state.clone();
    Arc::make_mut(&mut without_runtime).pipeline_service = None;
    let (status, _) = post(without_runtime, run.submission_id, session.clone()).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no pipeline runtime, no route"
    );
    assert_eq!(
        queued_index_invalidation(&fixture.runtime, tenant, run.run_id).await,
        (0, "none".to_string()),
        "no refused request withdrew anything"
    );

    let (status, body) = post(state.clone(), run.submission_id, session).await;
    assert_eq!(status, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["submission_id"], run.submission_id.to_string());
    assert_eq!(body["prior_status"], "accepted");
    assert_eq!(body["distribution_reach"], "commons_not_distributed");
    assert_eq!(body["already_distributed"], false);
    assert_eq!(body["credit_retained"], true);
    assert_eq!(body["index_invalidation"], "pending");
    assert_eq!(body["revocation_propagation"], "pending");
    assert!(
        !body.to_string().contains(tenant),
        "the response carries no tenant id"
    );
    assert_eq!(
        queued_index_invalidation(&fixture.runtime, tenant, run.run_id).await,
        (1, "pending".to_string())
    );
}

/// Enrols `near_account_id` as the designated NEAR payout account of the
/// account `principal` is linked to, through an owner connection (a fixture
/// write, as `main`'s NEAR enrolment makes).
async fn designate_near_account(
    owner: &Arc<PgBackend>,
    tenant: &str,
    principal: &str,
    near_account_id: &str,
) {
    let account_id = *owner
        .resolve_principals_to_accounts(tenant, &[principal.to_string()])
        .await
        .unwrap()
        .get(principal)
        .expect("the principal is linked to an account");
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant).await;
    tx.execute(
        "INSERT INTO trace_near_identities (
            tenant_id, public_key, near_account_id, account_id, payout_designated_at
         ) VALUES ($1, $2, $3, $4, NOW())",
        &[
            &tenant,
            &format!("ed25519:{}", Uuid::new_v4().simple()),
            &near_account_id,
            &account_id,
        ],
    )
    .await
    .expect("enrol the NEAR account");
    tx.commit().await.unwrap();
}

/// Owner decision (Zaki review 1, item S), through the route: a withdrawal
/// after Settle completed the Trace Credit leg keeps that credit, so the
/// response says `credit_retained` (with `main`'s reviewer reads on the
/// database, which see the pipeline's credit event and its finalized
/// batch), and the payout pass then submits and confirms its line, as
/// `main`'s submitter pays finalized credit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_withdrawal_after_settle_keeps_and_pays_the_settled_trace_credit() {
    use tower::ServiceExt;

    let near = Arc::new(RecordingNearAdapter::new());
    let Some(fixture) = withdrawal_fixture_with(
        |runtime, artifacts| trace_credit_payout_service(runtime, artifacts, near.clone()),
        true,
    )
    .await
    else {
        return;
    };
    let state = &fixture.state;
    let tenant = fixture.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.token);
    let session = account_session_headers(state, &fixture.token).await;
    // The session links the principal to an account; the account's payout
    // goes to its NEAR account (item 4), so it enrols one.
    designate_near_account(&fixture.owner, tenant, &principal, "contributor.testnet").await;
    let run = completed_pipeline_run(&fixture.service, tenant, &principal).await;

    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri(format!(
            "/v1/contributors/me/pipeline-submissions/{}/withdraw",
            run.submission_id
        ))
        .body(axum::body::Body::empty())
        .unwrap();
    request.headers_mut().extend(session);
    let response = app(state.clone()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1 << 16).await.unwrap()).unwrap();
    assert_eq!(body["credit_retained"], true, "{body}");

    assert_eq!(
        fixture.service.process_payouts(tenant, 32).await.unwrap(),
        1
    );
    let requests = near.requests();
    assert_eq!(requests.len(), 1, "the settled credit is submitted");
    near.record_confirmation(
        &requests[0].idempotency_key,
        format!("sha256:{}", "a".repeat(64)),
        format!("sha256:{}", "b".repeat(64)),
    )
    .expect("confirm the submitted request");
    assert_eq!(
        fixture.service.process_payouts(tenant, 32).await.unwrap(),
        1
    );
    let settlements = fixture
        .service
        .store()
        .list_settlements(tenant, run.run_id)
        .await
        .unwrap();
    assert_eq!(settlements.len(), 1);
    assert_eq!(settlements[0].payout_state, "confirmed");
    assert_eq!(settlements[0].last_error_label, None);
}

/// Zaki review 1, item 3: `main`'s admin NEAR outbox routes do not reach a
/// pipeline payout line (a non-NULL `instrument_id`). The pipeline submits
/// and confirms those through its own adapter, so the manual status route
/// answers a pipeline line as it answers a line it does not have (`404`),
/// and leaves it unchanged, and the admin listing leaves it out. A legacy
/// line (`instrument_id` NULL) is listed and moved as before. `main`'s
/// reads run on the database here, the only store that holds pipeline
/// lines.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mains_admin_outbox_routes_leave_pipeline_payout_lines_alone() {
    let near = Arc::new(RecordingNearAdapter::new());
    let Some(fixture) = withdrawal_fixture_with(
        |runtime, artifacts| trace_credit_payout_service(runtime, artifacts, near.clone()),
        true,
    )
    .await
    else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    let admin_token = format!("{}-admin", fixture.token);
    let utility_token = format!("{}-utility", fixture.token);
    let mut state = fixture.state.clone();
    let mut tokens = (*state.tokens).clone();
    insert_token(&mut tokens, tenant, &admin_token, TokenRole::Admin);
    insert_token(
        &mut tokens,
        tenant,
        &utility_token,
        TokenRole::UtilityWorker,
    );
    Arc::make_mut(&mut state).tokens = Arc::new(tokens);

    let principal = static_token_principal_ref(&fixture.token);
    let run = completed_pipeline_run(&fixture.service, tenant, &principal).await;
    assert_eq!(
        fixture.service.process_payouts(tenant, 32).await.unwrap(),
        1
    );
    let pipeline_line = fixture
        .owner
        .list_trace_near_credit_outbox_items(tenant)
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.instrument_id.is_some())
        .expect("the pipeline wrote its payout line");
    assert!(
        fixture
            .service
            .store()
            .list_settlements(tenant, run.run_id)
            .await
            .unwrap()[0]
            .settlement_batch_id
            .is_some(),
        "the leg has a batch"
    );

    // A legacy line of `main`'s own, on a legacy batch (no instrument).
    let legacy_id = Uuid::new_v4();
    let mut legacy = submitted_near_credit_outbox_item(legacy_id, TEST_NEAR_TX_HASH_1, 1_000_000);
    legacy.tenant_id = tenant.to_string();
    legacy.tenant_storage_ref = tenant_storage_ref(tenant);
    let legacy_list_hash = sha256_prefixed(&format!("legacy-sources:{legacy_id}"));
    fixture
        .owner
        .upsert_trace_credit_settlement_batch(StorageTraceCreditSettlementBatchWrite {
            tenant_id: tenant.to_string(),
            settlement_batch_id: legacy.settlement_batch_id,
            policy_version: "trace-credit-policy-v1".to_string(),
            status: StorageTraceCreditSettlementBatchStatus::Finalized,
            reason_hash: legacy_list_hash.clone(),
            issuer_approval_evidence_hash: None,
            source_credit_event_ids: Vec::new(),
            source_submission_ids: Vec::new(),
            source_list_hash: legacy_list_hash,
            settled_credit_points: "1".to_string(),
            settled_credit_micros: 1_000_000,
            line_items: Vec::new(),
            near_contract_id: Some("trace-credits.testnet".to_string()),
            ranking_model_version: None,
            ranking_target_use: None,
            ranking_calibration_run_id: None,
            ranking_calibration_report_hash: None,
            ranking_calibration_joined_evidence_hash: None,
            ranking_credit_events_excluded_count: 0,
            ranking_credit_events_excluded_reason_counts: BTreeMap::new(),
            actor_principal_ref: principal.clone(),
        })
        .await
        .expect("write the legacy batch");
    append_near_credit_outbox_item_with_db_mirror(
        state.as_ref(),
        &revocation_worker_tenant_auth(tenant),
        &legacy,
    )
    .await
    .expect("write the legacy line");
    mirror_near_credit_outbox_item_to_db(state.as_ref(), &legacy)
        .await
        .expect("the legacy line is in the database");

    let Json(listed) = near_credit_outbox_handler(State(state.clone()), auth_headers(&admin_token))
        .await
        .expect("the admin lists the outbox");
    let listed_ids = listed
        .iter()
        .map(|line| line.near_outbox_id)
        .collect::<Vec<_>>();
    assert_eq!(
        listed_ids,
        vec![legacy_id],
        "only the legacy line is listed"
    );

    let refused = mark_near_credit_outbox_status_handler(
        State(state.clone()),
        auth_headers(&utility_token),
        Json(TraceNearCreditOutboxStatusRequest {
            near_outbox_id: pipeline_line.near_outbox_id,
            status: StorageTraceCreditSettlementNearStatus::Failed,
            near_transaction_hash: None,
            error_detail: Some("manual".to_string()),
        }),
    )
    .await
    .expect_err("a pipeline line cannot be moved by hand");
    assert_eq!(refused.0, StatusCode::NOT_FOUND);
    assert_eq!(refused.1.0.error, "NEAR credit outbox item not found");
    let unchanged = fixture
        .owner
        .list_trace_near_credit_outbox_items(tenant)
        .await
        .unwrap()
        .into_iter()
        .find(|line| line.near_outbox_id == pipeline_line.near_outbox_id)
        .unwrap();
    assert_eq!(
        unchanged.status, pipeline_line.status,
        "the line is unchanged"
    );

    let Json(moved) = mark_near_credit_outbox_status_handler(
        State(state.clone()),
        auth_headers(&utility_token),
        Json(TraceNearCreditOutboxStatusRequest {
            near_outbox_id: legacy_id,
            status: StorageTraceCreditSettlementNearStatus::Failed,
            near_transaction_hash: None,
            error_detail: Some("manual".to_string()),
        }),
    )
    .await
    .expect("a legacy line moves as before");
    assert_eq!(moved.status, StorageTraceCreditSettlementNearStatus::Failed);
}

/// Review Focus 1 through the worker: the worker's tenant drain
/// (`drain_pipeline_tenant`, one tenant's share of a worker pass) processes
/// the index invalidation a withdrawal queued, with no direct call into the
/// invalidation pass, so the withdrawn revision's entries leave the index.
/// The drain before the withdrawal ran the tenant's invalidation step, so
/// its 10-second interval has not passed: the withdrawal's wakeup is what
/// runs it again at once (Zaki review 1, round 2, item 4).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_worker_drain_removes_a_withdrawn_revision_from_the_index() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    let dir = tempfile::tempdir().expect("temp dir");
    let index = IsolatedPipelineIndex::new();
    let service = assemble_test_pipeline_service(
        runtime.clone(),
        local_artifacts(&dir),
        index.clone(),
        vec![RecordingSettlementAdapter::new(
            InstrumentId::new("storage_rebate").unwrap(),
            "recording_storage_rebate_worker_drain_test_only",
            "none",
        ) as Arc<dyn SettlementAdapter>],
        None,
    );
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-worker-invalidation-{suffix}");
    let principal = static_token_principal_ref(&format!("token-worker-invalidation-{suffix}"));
    let tenant_ref = trace_commons_server::versioned_pipeline::pipeline_tenant_storage_ref(&tenant);
    let run = completed_pipeline_run(&service, &tenant, &principal).await;
    assert!(
        index.entry_count(&tenant_ref, MINIMAL_INDEX_ID) > 0,
        "Settle wrote the revision's entries"
    );
    let cadence = Arc::new(std::sync::Mutex::new(
        pipeline_runtime::PipelineFollowUpCadence::default(),
    ));
    pipeline_runtime::drain_pipeline_tenant(service.clone(), tenant.clone(), cadence.clone()).await;
    service
        .withdraw_submission(&tenant, run.submission_id, &principal, None)
        .await
        .expect("the owner withdraws the submission");
    assert_eq!(
        queued_index_invalidation(&runtime, &tenant, run.run_id).await,
        (1, "pending".to_string())
    );

    pipeline_runtime::drain_pipeline_tenant(service.clone(), tenant.clone(), cadence).await;

    assert_eq!(
        index.entry_count(&tenant_ref, MINIMAL_INDEX_ID),
        0,
        "no entry of the withdrawn revision stays in the index"
    );
    assert_eq!(
        queued_index_invalidation(&runtime, &tenant, run.run_id)
            .await
            .1,
        "complete"
    );
}

/// Zaki review 1, item 1: Score stores the index command (embeddings and
/// content hashes) and the neighbour set as objects of their own. A run of
/// the compatibility bundle has both. The run is withdrawn after Score and
/// before Settle, through the pipeline withdrawal alone (none of the route's
/// own content cleanup), so the only thing that deletes the two objects is
/// the follow-up the withdrawal queued: `main`'s revocation-propagation
/// worker. After one worker pass both objects are gone from the artifact
/// store; the neighbour set was already gone before the pass, which is not
/// an error. A second withdrawal queues nothing and a second pass has
/// nothing to do. Settle then completes the withdrawn run without the
/// deleted command.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_revocation_worker_deletes_the_score_objects_of_a_withdrawn_run() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-score-objects-{suffix}");
    let principal = static_token_principal_ref(&format!("token-score-objects-{suffix}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    // The service-owned local store: the worker deletes payloads only from a
    // service-owned store, and the pipeline records this store's name.
    let configured_store = || {
        ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifacts.clone(),
        )
    };
    let service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &configured_store(),
        IsolatedPipelineIndex::new(),
        2_500_000,
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        Some(mains_database().await),
        None,
        false,
        false,
        false,
        false,
    );
    Arc::make_mut(&mut state).artifact_store = Some(configured_store());

    service
        .register_default_bundle(&tenant)
        .await
        .expect("register the bundle");
    let mut envelope = model_training_envelope().await;
    envelope.submission_id = Uuid::new_v4();
    let raw = serde_json::to_vec(&envelope).unwrap();
    let key = envelope.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(PipelineReceiptRequest {
            source_session: None,
            tenant_id: &tenant,
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
    for _ in 0..2 {
        service
            .process_run(&tenant, created.run_id)
            .await
            .expect("the phase runs");
    }
    let scored = service
        .store()
        .get_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the run exists");
    assert_eq!(scored.next_phase, Some(Phase::Settle));
    let tenant_ref = tenant_storage_ref(&tenant);
    let score_objects = [
        scored
            .index_command_ref
            .clone()
            .expect("Score stored an index command"),
        scored
            .score_neighbor_ref
            .clone()
            .expect("Score stored a neighbour set"),
    ]
    .map(|stored| {
        let (object_key, ciphertext_sha256) = stored.rsplit_once('#').unwrap();
        (object_key.to_string(), ciphertext_sha256.to_string())
    });
    let present = |object_key: &str| {
        artifacts
            .artifact_present_by_object_key(
                &tenant_ref,
                TraceArtifactKind::VectorPayload,
                object_key,
                "",
            )
            .expect("the store answers")
    };
    for (object_key, _) in &score_objects {
        assert_eq!(present(object_key), Some(true), "Score's object is stored");
    }

    let outcome = service
        .withdraw_submission(&tenant, scored.submission_id, &principal, None)
        .await
        .expect("the owner withdraws the submission");
    assert_eq!(
        outcome.revocation_propagation,
        PipelineWithdrawalFollowUpState::Pending
    );
    let (neighbor_key, neighbor_sha256) = &score_objects[1];
    artifacts
        .delete_artifact(
            &tenant_ref,
            &EncryptedTraceArtifactReceipt {
                tenant_storage_ref: tenant_ref.clone(),
                artifact_kind: TraceArtifactKind::VectorPayload,
                object_key: neighbor_key.clone(),
                ciphertext_sha256: neighbor_sha256.clone(),
                encrypted_at: Utc::now(),
            },
        )
        .expect("delete the neighbour set ahead of the worker");

    let auth = revocation_worker_tenant_auth(&tenant);
    let worker_request = || TraceRevocationPropagationWorkerRequest {
        purpose: Some("score object deletion".to_string()),
        dry_run: false,
        limit: 100,
    };
    let first_pass = run_revocation_propagation_worker(state.as_ref(), &auth, worker_request())
        .await
        .expect("the worker runs");
    assert_eq!(
        first_pass.checked, 4,
        "one deletion per object: the source, the approved revision, and the two Score objects"
    );
    assert_eq!(
        (first_pass.failed, first_pass.skipped),
        (0, 0),
        "every queued deletion completes"
    );
    assert_eq!(first_pass.completed, first_pass.checked);
    for (object_key, _) in &score_objects {
        assert_eq!(
            present(object_key),
            Some(false),
            "the withdrawn run's Score object is deleted"
        );
    }
    let object_refs = owner
        .list_trace_object_refs(&tenant, scored.submission_id)
        .await
        .unwrap();
    for (object_key, _) in &score_objects {
        let object_ref = object_refs
            .iter()
            .find(|object_ref| &object_ref.object_key == object_key)
            .expect("the Score object is an object ref of the submission");
        assert!(object_ref.deleted_at.is_some(), "marked deleted");
    }

    let again = service
        .withdraw_submission(&tenant, scored.submission_id, &principal, None)
        .await
        .expect("a second withdrawal succeeds");
    assert_eq!(
        again.revocation_propagation,
        PipelineWithdrawalFollowUpState::Complete
    );
    let second_pass = run_revocation_propagation_worker(state.as_ref(), &auth, worker_request())
        .await
        .expect("the worker runs again");
    assert_eq!(second_pass.checked, 0, "nothing more to do");

    let settled = service
        .process_run(&tenant, scored.run_id)
        .await
        .expect("Settle runs")
        .expect("the run is claimed");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert_eq!(settled.index_membership, "excluded");
}

/// Zaki review 1, item 7: a tenant taken off
/// `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` and put on
/// `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` still has its follow-up work
/// processed. Here no receipt of the tenant is routed; its withdrawal queued
/// an index invalidation; the real worker (`run_pipeline_app`) processes it
/// within a few passes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_worker_drains_a_tenant_on_the_drain_list() {
    let Some(fixture) = withdrawal_fixture().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let principal = static_token_principal_ref(&fixture.token);
    let run = completed_pipeline_run(&fixture.service, &tenant, &principal).await;
    fixture
        .service
        .withdraw_submission(&tenant, run.submission_id, &principal, None)
        .await
        .expect("the owner withdraws the submission");
    assert_eq!(
        queued_index_invalidation(&fixture.runtime, &tenant, run.run_id).await,
        (1, "pending".to_string())
    );
    let mut state = fixture.state.clone();
    assert_eq!(
        state
            .tenant_rollout_gates
            .tenant_count(TraceTenantRolloutFeature::PipelineReceipts),
        0,
        "no receipt of the tenant is routed"
    );
    Arc::make_mut(&mut state).pipeline_drain_tenant_ids =
        Arc::new(BTreeSet::from([tenant.clone()]));

    let (_base, stop, server) = serve_pipeline_app(state).await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let (_, state) = queued_index_invalidation(&fixture.runtime, &tenant, run.run_id).await;
        if state == "complete" {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the worker never drained the tenant on the drain list (invalidation {state})"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    stop.send(()).expect("send shutdown");
    join_within(server, 20, "the drain app").await;
}

/// Controller ruling R2-2: every object a complete pipeline run stored is an
/// object ref of its submission -- the receipt's source envelope, Review's
/// approved revision, and Score's index command and neighbour set -- so
/// once the owner withdraws the submission, one pass of `main`'s
/// revocation-propagation worker deletes all four from the service-owned
/// store and marks each ref deleted. The pipeline's own attempt sweep
/// (`sweep_attempt_artifacts`) never runs here and is not needed: it sweeps
/// only attempts that never committed (ruling R2-1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_revocation_worker_deletes_every_object_of_a_withdrawn_complete_run() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-complete-objects-{suffix}");
    let principal = static_token_principal_ref(&format!("token-complete-objects-{suffix}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let configured_store = || {
        ConfiguredTraceArtifactStore::new(
            TRACE_COMMONS_SERVICE_LOCAL_ENCRYPTED_OBJECT_STORE,
            artifacts.clone(),
        )
    };
    let service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &configured_store(),
        IsolatedPipelineIndex::new(),
        2_500_000,
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        Some(owner.clone() as Arc<dyn Database>),
        None,
        false,
        false,
        false,
        false,
    );
    Arc::make_mut(&mut state).artifact_store = Some(configured_store());

    service
        .register_default_bundle(&tenant)
        .await
        .expect("register the bundle");
    let mut envelope = model_training_envelope().await;
    envelope.submission_id = Uuid::new_v4();
    let raw = serde_json::to_vec(&envelope).unwrap();
    let key = envelope.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(PipelineReceiptRequest {
            source_session: None,
            tenant_id: &tenant,
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
    for _ in 0..3 {
        service
            .process_run(&tenant, created.run_id)
            .await
            .expect("the phase runs");
    }
    let settled = service
        .store()
        .get_run(&tenant, created.run_id)
        .await
        .unwrap()
        .expect("the run exists");
    assert_eq!(settled.state, PipelineRunState::Complete);
    assert!(
        settled.index_command_ref.is_some() && settled.score_neighbor_ref.is_some(),
        "Score stored both of its objects"
    );

    let object_refs = owner
        .list_trace_object_refs(&tenant, settled.submission_id)
        .await
        .unwrap();
    let mut kinds: Vec<StorageTraceObjectArtifactKind> = object_refs
        .iter()
        .map(|object_ref| object_ref.artifact_kind)
        .collect();
    kinds.sort_by_key(|kind| format!("{kind:?}"));
    assert_eq!(
        kinds,
        vec![
            StorageTraceObjectArtifactKind::ReviewSnapshot,
            StorageTraceObjectArtifactKind::SubmittedEnvelope,
            StorageTraceObjectArtifactKind::WorkerIntermediate,
            StorageTraceObjectArtifactKind::WorkerIntermediate,
        ],
        "the source, the approved revision, and the two Score objects are object refs"
    );
    let tenant_ref = tenant_storage_ref(&tenant);
    // The local store answers presence from the object key alone.
    let present = |object_key: &str| {
        artifacts
            .artifact_present_by_object_key(
                &tenant_ref,
                TraceArtifactKind::VectorPayload,
                object_key,
                "",
            )
            .expect("the store answers")
    };
    for object_ref in &object_refs {
        assert_eq!(
            present(&object_ref.object_key),
            Some(true),
            "{:?} is stored before the withdrawal",
            object_ref.artifact_kind
        );
    }

    let outcome = service
        .withdraw_submission(&tenant, settled.submission_id, &principal, None)
        .await
        .expect("the owner withdraws the submission");
    assert_eq!(
        outcome.revocation_propagation,
        PipelineWithdrawalFollowUpState::Pending
    );
    let auth = revocation_worker_tenant_auth(&tenant);
    let pass = run_revocation_propagation_worker(
        state.as_ref(),
        &auth,
        TraceRevocationPropagationWorkerRequest {
            purpose: Some("complete run object deletion".to_string()),
            dry_run: false,
            limit: 100,
        },
    )
    .await
    .expect("the worker runs");
    assert_eq!(
        (pass.checked, pass.completed, pass.failed, pass.skipped),
        (4, 4, 0, 0),
        "one completed deletion per object"
    );

    let after = owner
        .list_trace_object_refs(&tenant, settled.submission_id)
        .await
        .unwrap();
    assert_eq!(after.len(), object_refs.len());
    for object_ref in &after {
        assert_eq!(
            present(&object_ref.object_key),
            Some(false),
            "{:?} is deleted from the store",
            object_ref.artifact_kind
        );
        assert!(
            object_ref.deleted_at.is_some(),
            "{:?} is marked deleted",
            object_ref.artifact_kind
        );
    }
}

// ---------------------------------------------------------------------------
// Pipeline product routes through the router: the status block, the score
// attestation, exports, and the administrator reads, with the product store
// on the runtime role.
// ---------------------------------------------------------------------------

/// `withdrawal_fixture`, with the product store on the runtime role, an
/// export-worker and an admin token for the fixture's tenant, and an admin
/// and an export-worker token for another tenant.
struct ProductFixture {
    base: WithdrawalFixture,
    export_token: String,
    admin_token: String,
    other_tenant_admin_token: String,
    other_tenant_export_token: String,
}

async fn product_fixture() -> Option<ProductFixture> {
    let mut base = withdrawal_fixture().await?;
    let suffix = Uuid::new_v4().simple().to_string();
    let other_tenant = format!("tenant-product-other-{suffix}");
    let export_token = format!("token-export-{suffix}");
    let admin_token = format!("token-admin-{suffix}");
    let other_tenant_admin_token = format!("token-admin-other-{suffix}");
    let other_tenant_export_token = format!("token-export-other-{suffix}");
    let mut tokens = (*base.state.tokens).clone();
    insert_token(
        &mut tokens,
        &base.tenant,
        &export_token,
        TokenRole::ExportWorker,
    );
    insert_token(&mut tokens, &base.tenant, &admin_token, TokenRole::Admin);
    insert_token(
        &mut tokens,
        &other_tenant,
        &other_tenant_admin_token,
        TokenRole::Admin,
    );
    insert_token(
        &mut tokens,
        &other_tenant,
        &other_tenant_export_token,
        TokenRole::ExportWorker,
    );
    let runtime = base.runtime.clone();
    let tenant = base.tenant.clone();
    let state = Arc::make_mut(&mut base.state);
    state.tokens = Arc::new(tokens);
    state.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime)));
    // The status route reads the pipeline's view only for a tenant on the
    // receipts or the drain list (Zaki review 1, round 2, item 6). The
    // drain list routes no receipt.
    state.pipeline_drain_tenant_ids = Arc::new(BTreeSet::from([tenant]));
    Some(ProductFixture {
        base,
        export_token,
        admin_token,
        other_tenant_admin_token,
        other_tenant_export_token,
    })
}

/// `(state, attempt_count, the run's index_invalidation_state)` of the run's
/// index invalidation.
async fn invalidation_state(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    run_id: Uuid,
) -> (String, i32, String) {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let row = tx
        .query_one(
            "SELECT i.state, i.attempt_count, r.index_invalidation_state
               FROM pipeline_index_invalidations i
               JOIN pipeline_runs r ON r.tenant_id = i.tenant_id AND r.run_id = i.run_id
              WHERE i.tenant_id = $1 AND i.run_id = $2",
            &[&tenant_id, &run_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    (row.get(0), row.get(1), row.get(2))
}

/// Zaki review 1, item 6: `POST /v1/admin/pipeline/index-invalidations/requeue-failed`
/// re-enqueues every `failed` index invalidation of the caller's tenant and
/// answers how many (a count, nothing else). It needs `main`'s admin
/// credential: a contributor is refused and nothing changes. Another
/// tenant's admin re-enqueues none of this tenant's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_operator_route_requeues_the_tenants_failed_invalidations() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = &fixture.base.state;
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let run = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    fixture
        .base
        .service
        .withdraw_submission(tenant, run.submission_id, &principal, None)
        .await
        .expect("the owner withdraws the submission");
    let mut owner = fixture
        .base
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut owner, tenant).await;
    tx.execute(
        "UPDATE pipeline_index_invalidations
            SET state = 'failed', attempt_count = max_attempts,
                last_error_label = 'index_invalidation_failed'
          WHERE tenant_id = $1 AND run_id = $2",
        &[&tenant, &run.run_id],
    )
    .await
    .unwrap();
    tx.execute(
        "UPDATE pipeline_runs SET index_invalidation_state = 'failed'
          WHERE tenant_id = $1 AND run_id = $2",
        &[&tenant, &run.run_id],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let uri = "/v1/admin/pipeline/index-invalidations/requeue-failed";

    let (status, _) = route_request(
        state.clone(),
        "POST",
        uri,
        auth_headers(&fixture.base.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a contributor is refused");
    let (status, body) = route_request(
        state.clone(),
        "POST",
        uri,
        auth_headers(&fixture.other_tenant_admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, serde_json::json!({"requeued": 0}));
    assert_eq!(
        invalidation_state(&fixture.base.runtime, tenant, run.run_id).await,
        ("failed".to_string(), 5, "failed".to_string()),
        "no refused or other-tenant request changed it"
    );

    let (status, body) = route_request(
        state.clone(),
        "POST",
        uri,
        auth_headers(&fixture.admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body, serde_json::json!({"requeued": 1}));
    assert_eq!(
        invalidation_state(&fixture.base.runtime, tenant, run.run_id).await,
        ("pending".to_string(), 0, "pending".to_string())
    );
    let (_, again) = route_request(
        state.clone(),
        "POST",
        uri,
        auth_headers(&fixture.admin_token),
        None,
    )
    .await;
    assert_eq!(again, serde_json::json!({"requeued": 0}));

    // Zaki review 1, fix round, item 4: each call the route answered
    // appends a hash-only, label-only audit row (a count, no ids).
    let mut client = fixture
        .base
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut client, tenant).await;
    let rows = tx
        .query(
            "SELECT action, metadata_json FROM trace_audit_events
              WHERE tenant_id = $1
                AND metadata_json->'action_counts' ? 'pipeline_index_invalidations_requeued'
              ORDER BY audit_sequence",
            &[&tenant],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(rows.len(), 2, "one row per admitted call of this tenant");
    assert_eq!(rows[0].get::<_, String>("action"), "vector_index");
    let metadata: serde_json::Value = rows[0].get("metadata_json");
    assert_eq!(
        metadata["action_counts"]["pipeline_index_invalidations_requeued"], 1,
        "{metadata}"
    );
    assert_eq!(
        metadata["purpose_hash"],
        sha256_prefixed("pipeline_index_invalidation_requeue"),
        "{metadata}"
    );
    assert!(!metadata.to_string().contains(&run.run_id.to_string()));
}

/// Final review M4 (ruling FR-7): `POST /v1/workers/pipeline/index-rebuild`
/// rebuilds the caller's tenant's index from its sealed commands and appends
/// one hash-only, label-only index maintenance audit row: `main`'s
/// `vector_index` action, the fixed purpose `pipeline_index_rebuild` as a
/// hash, and the report's counts, with no run or submission id. A
/// contributor is refused and appends nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_index_rebuild_route_appends_an_audit_row() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = &fixture.base.state;
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let run = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let uri = "/v1/workers/pipeline/index-rebuild";

    let (status, _) = route_request(
        state.clone(),
        "POST",
        uri,
        auth_headers(&fixture.base.token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a contributor is refused");

    let (status, body) = route_request(
        state.clone(),
        "POST",
        uri,
        auth_headers(&fixture.admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["command_count"], 1, "{body}");
    assert_eq!(body["skipped_run_count"], 0, "{body}");
    let entry_count = body["entry_count"].as_u64().expect("an entry count");
    assert!(entry_count > 0, "{body}");
    assert_eq!(
        body["unchanged_entry_count"].as_u64(),
        Some(entry_count),
        "the live index already holds every entry Settle wrote"
    );

    let mut client = fixture
        .base
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut client, tenant).await;
    let rows = tx
        .query(
            "SELECT action, metadata_json FROM trace_audit_events
              WHERE tenant_id = $1
                AND metadata_json->'action_counts' ? 'pipeline_index_commands_replayed'
              ORDER BY audit_sequence",
            &[&tenant],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(rows.len(), 1, "one row for the one admitted call");
    assert_eq!(rows[0].get::<_, String>("action"), "vector_index");
    let metadata: serde_json::Value = rows[0].get("metadata_json");
    let counts = &metadata["action_counts"];
    assert_eq!(counts["pipeline_index_commands_replayed"], 1, "{metadata}");
    assert_eq!(
        counts["pipeline_index_entries_written"], entry_count,
        "{metadata}"
    );
    assert_eq!(
        counts["pipeline_index_entries_unchanged"], entry_count,
        "{metadata}"
    );
    assert_eq!(counts["pipeline_index_runs_skipped"], 0, "{metadata}");
    assert_eq!(
        metadata["purpose_hash"],
        sha256_prefixed("pipeline_index_rebuild"),
        "{metadata}"
    );
    let text = metadata.to_string();
    assert!(!text.contains(&run.run_id.to_string()));
    assert!(!text.contains(&run.submission_id.to_string()));
}

/// `POST /v1/pipeline/exports` for `use`, with `limit`, keyed by `key`.
async fn create_pipeline_export(
    state: &Arc<AppState>,
    token: &str,
    key: &str,
    limit: usize,
) -> (StatusCode, serde_json::Value) {
    pipeline_product_request(
        state.clone(),
        "POST",
        "/v1/pipeline/exports",
        Some(token),
        Some(key),
        Some(serde_json::json!({
            "allowed_use": "evaluation",
            "purpose": "benchmark refresh",
            "limit": limit,
        })),
    )
    .await
}

/// `POST /v1/pipeline/exports/{snapshot_id}/complete`.
async fn complete_pipeline_export(
    state: &Arc<AppState>,
    token: &str,
    snapshot_id: &serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let snapshot_id = snapshot_id.as_str().expect("a snapshot id");
    pipeline_product_request(
        state.clone(),
        "POST",
        &format!("/v1/pipeline/exports/{snapshot_id}/complete"),
        Some(token),
        None,
        None,
    )
    .await
}

/// The tenant's `export` audit events, oldest first, as `main`'s mirrored
/// audit helper writes them to the database: each one's reason, export
/// manifest id, decision-inputs hash, and metadata.
async fn export_audit_events(owner: &Arc<PgBackend>, tenant_id: &str) -> Vec<serde_json::Value> {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let rows = tx
        .query(
            "SELECT reason, export_manifest_id, decision_inputs_hash, metadata_json
               FROM trace_audit_events
              WHERE tenant_id = $1 AND action = 'export'
              ORDER BY audit_sequence",
            &[&tenant_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    rows.iter()
        .map(|row| {
            serde_json::json!({
                "reason": row.get::<_, Option<String>>(0),
                "export_manifest_id": row.get::<_, Option<Uuid>>(1),
                "decision_inputs_hash": row.get::<_, Option<String>>(2),
                "metadata": row.get::<_, serde_json::Value>(3),
            })
        })
        .collect()
}

/// Checks the newest of `events`: an export event of `stage` for
/// `snapshot`, with the use label, the purpose hash, the source-list hash,
/// and the item count, and nothing else. A `delivered` event's purpose code
/// is the manifest's (Ruling T14-11); a `created` event's is the use label.
fn assert_pipeline_export_audit(
    events: &[serde_json::Value],
    stage: &str,
    snapshot: &serde_json::Value,
) {
    let event = events.last().expect("an export audit event");
    assert_eq!(
        event["reason"],
        format!(
            "pipeline_export={stage};allowed_use=evaluation;purpose_hash={};source_list_hash={}",
            snapshot["purpose_hash"].as_str().unwrap(),
            snapshot["source_list_hash"].as_str().unwrap(),
        ),
        "{event}"
    );
    assert_eq!(event["export_manifest_id"], snapshot["snapshot_id"]);
    assert_eq!(event["decision_inputs_hash"], snapshot["source_list_hash"]);
    let purpose_code = if stage == "delivered" {
        "pipeline_export:evaluation"
    } else {
        "evaluation"
    };
    assert_eq!(event["metadata"]["purpose_code"], purpose_code, "{event}");
    assert_eq!(
        event["metadata"]["item_count"],
        snapshot["items"].as_array().unwrap().len(),
        "{event}"
    );
}

/// The export routes, with the export credential. Creation snapshots the
/// tenant's approved revision; the same request key with the same body
/// returns that snapshot, and with another item limit is refused (the limit
/// is part of the request the key names). Another tenant's export
/// credential does not find the snapshot. Completion delivers it, and
/// again returns it unchanged. The response never carries the tenant id.
/// Each call that succeeds appends one `export` audit event (`created` or
/// `delivered`), hash- and label-only; a refused call appends none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_export_routes_snapshot_and_deliver_with_the_export_credential() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = &fixture.base.state;
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let run = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;

    let (status, created) = create_pipeline_export(state, &fixture.export_token, "key-1", 10).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(created["state"], "ready");
    assert_eq!(created["allowed_use"], "evaluation");
    let items = created["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "{created}");
    assert_eq!(items[0]["run_id"], run.run_id.to_string());
    assert_eq!(items[0]["submission_id"], run.submission_id.to_string());
    assert_eq!(
        items[0]["source_content_hash"],
        run.approved_content_hash.clone().expect("an approved hash")
    );
    assert!(created.get("tenant_id").is_none());
    assert!(!created.to_string().contains(tenant), "{created}");
    let owner = &fixture.base.owner;
    let audit = export_audit_events(owner, tenant).await;
    assert_eq!(audit.len(), 1, "creation writes one audit event: {audit:?}");
    assert_pipeline_export_audit(&audit, "created", &created);
    let exporter = static_token_principal_ref(&fixture.export_token);
    let audit_text = serde_json::to_string(&audit).unwrap();
    assert!(
        !audit_text.contains(tenant) && !audit_text.contains(&exporter),
        "{audit_text}"
    );

    let (status, again) = create_pipeline_export(state, &fixture.export_token, "key-1", 10).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(again, created, "the same request returns the same snapshot");
    let audit = export_audit_events(owner, tenant).await;
    assert_eq!(audit.len(), 2, "{audit:?}");
    assert_pipeline_export_audit(&audit, "created", &created);
    let (status, conflict) =
        create_pipeline_export(state, &fixture.export_token, "key-1", 11).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"], "export_idempotency_conflict");
    assert_eq!(
        export_audit_events(owner, tenant).await.len(),
        2,
        "a refused creation writes no audit event"
    );

    let snapshot_id = &created["snapshot_id"];
    let (status, other) =
        complete_pipeline_export(state, &fixture.other_tenant_export_token, snapshot_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{other}");
    assert_eq!(other["error"], "export_snapshot_not_found");
    assert_eq!(export_audit_events(owner, tenant).await.len(), 2);
    let other_tenant = fixture
        .base
        .state
        .tokens
        .get(&fixture.other_tenant_export_token)
        .expect("the other tenant's token")
        .tenant_id
        .clone();
    assert!(
        export_audit_events(owner, &other_tenant).await.is_empty(),
        "a refused completion writes no audit event"
    );
    let (status, completed) =
        complete_pipeline_export(state, &fixture.export_token, snapshot_id).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["state"], "complete");
    assert_eq!(&completed["export_manifest_id"], snapshot_id);
    assert_eq!(completed["items"], created["items"]);
    let audit = export_audit_events(owner, tenant).await;
    assert_eq!(audit.len(), 3, "delivery writes one audit event: {audit:?}");
    assert_pipeline_export_audit(&audit, "delivered", &completed);
    let (status, completed_again) =
        complete_pipeline_export(state, &fixture.export_token, snapshot_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(completed_again, completed);
    let audit = export_audit_events(owner, tenant).await;
    assert_eq!(audit.len(), 4, "{audit:?}");
    assert_pipeline_export_audit(&audit, "delivered", &completed);

    // `main`'s own check accepts the mirrored audit rows: each row's columns
    // match its canonical payload, whose kind is `dataset_export`, the kind
    // an `Export` row projects to.
    let rows = owner
        .list_trace_audit_events(tenant)
        .await
        .expect("main lists the tenant's audit events");
    let failures = collect_db_audit_canonical_projection_failures(&rows);
    assert!(failures.is_empty(), "{failures:?}");
}

/// `POST /v1/pipeline/exports` with `body`, keyed by `key`.
async fn create_pipeline_export_with(
    state: &Arc<AppState>,
    token: &str,
    key: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    pipeline_product_request(
        state.clone(),
        "POST",
        "/v1/pipeline/exports",
        Some(token),
        Some(key),
        Some(body),
    )
    .await
}

/// Ruling T14-8: the export route applies `main`'s export rules for a
/// request. Two low-risk submissions that consent to debugging and
/// evaluation complete.
/// - Guardrails off: an empty purpose and no consent scope are accepted.
/// - The item limit is clamped to `max_export_items_per_request`.
/// - A named consent scope narrows the export: model training holds
///   nothing, debugging and evaluation holds both. The scope is part of the
///   request the key names, so the same key with another scope is refused.
/// - Guardrails on (purpose and consent scope given): the export holds only
///   low-risk submissions, as `main`'s guardrailed replay export does; the
///   second submission, stored as medium risk, is left out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_export_route_applies_mains_guardrails_limit_and_consent_scope() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = &fixture.base.state;
    let token = fixture.export_token.as_str();
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let low = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let medium = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let mut both = vec![low.run_id.to_string(), medium.run_id.to_string()];
    both.sort();
    let item_runs = |snapshot: &serde_json::Value| {
        let mut runs = snapshot["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["run_id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        runs.sort();
        runs
    };

    let (status, unguarded) = create_pipeline_export_with(
        state,
        token,
        "unguarded",
        serde_json::json!({"allowed_use": "evaluation", "purpose": "", "limit": 10}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unguarded}");
    assert_eq!(item_runs(&unguarded), both);

    let mut capped = state.clone();
    Arc::make_mut(&mut capped).max_export_items_per_request = 1;
    let (status, clamped) = create_pipeline_export_with(
        &capped,
        token,
        "clamped",
        serde_json::json!({"allowed_use": "evaluation", "purpose": "p", "limit": 10}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{clamped}");
    assert_eq!(
        clamped["items"].as_array().unwrap().len(),
        1,
        "the limit is clamped to the configured maximum: {clamped}"
    );

    let scoped = |scope: &str| {
        serde_json::json!({
            "allowed_use": "evaluation",
            "purpose": "p",
            "limit": 10,
            "consent_scope": scope,
        })
    };
    let (status, training) =
        create_pipeline_export_with(state, token, "training", scoped("model_training")).await;
    assert_eq!(status, StatusCode::OK, "{training}");
    assert!(item_runs(&training).is_empty(), "{training}");
    let (status, debugging) =
        create_pipeline_export_with(state, token, "debugging", scoped("debugging_evaluation"))
            .await;
    assert_eq!(status, StatusCode::OK, "{debugging}");
    assert_eq!(item_runs(&debugging), both);
    let (status, conflict) =
        create_pipeline_export_with(state, token, "debugging", scoped("model_training")).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"], "export_idempotency_conflict");

    let mut owner = fixture
        .base
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut owner, tenant).await;
    tx.execute(
        "UPDATE trace_submissions SET privacy_risk = 'medium'
          WHERE tenant_id = $1 AND submission_id = $2",
        &[&tenant, &medium.submission_id],
    )
    .await
    .expect("store the second submission as medium risk");
    tx.commit().await.unwrap();
    drop(owner);
    let mut guarded = state.clone();
    Arc::make_mut(&mut guarded).require_export_guardrails = true;
    let (status, low_only) =
        create_pipeline_export_with(&guarded, token, "guarded", scoped("debugging_evaluation"))
            .await;
    assert_eq!(status, StatusCode::OK, "{low_only}");
    assert_eq!(item_runs(&low_only), vec![low.run_id.to_string()]);
    let (status, unguarded_again) = create_pipeline_export_with(
        state,
        token,
        "unguarded-again",
        scoped("debugging_evaluation"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{unguarded_again}");
    assert_eq!(item_runs(&unguarded_again), both);
}

/// Owner ruling T14-7: a delivered pipeline snapshot's export manifest keeps
/// kind `export_artifact`, but it stays out of `main`'s replay-dataset
/// manifest list (`GET /v1/datasets/replay/manifests`) and out of the DB
/// reconciliation's replay manifest count, so a replay worker never takes a
/// pipeline export for a replay dataset. A `main` replay export in the same
/// tenant, made through `main`'s worker route, is still listed and counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_exports_stay_out_of_mains_replay_dataset_list_and_count() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = &fixture.base.state;
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let run = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let (status, created) =
        create_pipeline_export(state, &fixture.export_token, "replay-list", 10).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    let (status, completed) =
        complete_pipeline_export(state, &fixture.export_token, &created["snapshot_id"]).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    let pipeline_manifest_id: Uuid =
        serde_json::from_value(completed["export_manifest_id"].clone())
            .expect("the delivered snapshot names its manifest");

    let (status, replay) = pipeline_product_request(
        state.clone(),
        "POST",
        "/v1/workers/replay-export",
        Some(fixture.export_token.as_str()),
        None,
        Some(serde_json::json!({"purpose": "trace_commons_worker_replay_dataset"})),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "main's replay export runs: {replay}"
    );
    let replay_manifest_id: Uuid =
        serde_json::from_value(replay["export_id"].clone()).expect("main's replay export id");

    let mut stored = fixture
        .base
        .owner
        .list_trace_export_manifests(tenant)
        .await
        .expect("main reads the tenant's export manifests");
    stored.sort_by_key(|manifest| manifest.export_manifest_id != pipeline_manifest_id);
    assert_eq!(stored.len(), 2, "{stored:?}");
    assert_eq!(stored[0].export_manifest_id, pipeline_manifest_id);
    assert_eq!(
        stored[0].artifact_kind,
        StorageTraceObjectArtifactKind::ExportArtifact
    );
    assert_eq!(
        stored[0].purpose_code.as_deref(),
        Some("pipeline_export:evaluation"),
        "the marker main's replay filters read"
    );
    assert_eq!(stored[0].source_submission_ids, vec![run.submission_id]);
    assert_eq!(stored[1].export_manifest_id, replay_manifest_id);

    let (status, listed) = pipeline_product_request(
        state.clone(),
        "GET",
        "/v1/datasets/replay/manifests",
        Some(fixture.export_token.as_str()),
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let listed = listed
        .as_array()
        .expect("a manifest list")
        .iter()
        .map(|manifest| manifest["export_manifest_id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        listed,
        vec![serde_json::json!(replay_manifest_id)],
        "main's replay list holds main's replay export and not the pipeline export"
    );

    let caller = state
        .tokens
        .get(&fixture.export_token)
        .expect("the export credential")
        .clone();
    let report = reconcile_db_mirror(state.as_ref(), &caller, &[], &[], true, None)
        .await
        .expect("main reconciles the tenant's DB mirror")
        .expect("a reconciliation report");
    assert_eq!(report.db_export_manifest_count, 2);
    assert_eq!(
        report.db_replay_export_manifest_count, 1,
        "main's replay export is counted and the pipeline export is not"
    );
}

/// The export route passes the caller's consent-scope allowlists to the
/// store, as `main`'s export path filters records by them: the scoped
/// export credential's and the tenant policy's. The completed submission
/// consents to debugging and evaluation only. A credential scoped to model
/// training gets an empty snapshot, and an unscoped one gets the
/// submission; a tenant policy that allows only model training empties the
/// snapshot again, and one that allows debugging and evaluation does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_export_route_applies_the_callers_consent_scope_allowlists() {
    let Some(mut fixture) = product_fixture().await else {
        return;
    };
    let tenant = fixture.base.tenant.clone();
    let principal = static_token_principal_ref(&fixture.base.token);
    let run = completed_pipeline_run(&fixture.base.service, &tenant, &principal).await;
    let scoped_token = format!("{}-scoped", fixture.export_token);
    let mut tokens = (*fixture.base.state.tokens).clone();
    insert_token(&mut tokens, &tenant, &scoped_token, TokenRole::ExportWorker);
    tokens
        .get_mut(&scoped_token)
        .unwrap()
        .allowed_consent_scopes = BTreeSet::from([ConsentScope::ModelTraining]);
    Arc::make_mut(&mut fixture.base.state).tokens = Arc::new(tokens);
    let state = fixture.base.state.clone();
    let with_policy = |scope: ConsentScope| {
        let mut state = state.clone();
        Arc::make_mut(&mut state).tenant_policies = Arc::new(BTreeMap::from([(
            tenant.clone(),
            TenantSubmissionPolicy {
                allowed_consent_scopes: BTreeSet::from([scope]),
                allowed_uses: BTreeSet::new(),
            },
        )]));
        state
    };
    let item_runs = |snapshot: &serde_json::Value| {
        snapshot["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|item| item["run_id"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };

    let (status, scoped) = create_pipeline_export(&state, &scoped_token, "scoped", 10).await;
    assert_eq!(status, StatusCode::OK, "{scoped}");
    assert!(item_runs(&scoped).is_empty(), "{scoped}");
    let (status, unscoped) =
        create_pipeline_export(&state, &fixture.export_token, "unscoped", 10).await;
    assert_eq!(status, StatusCode::OK, "{unscoped}");
    assert_eq!(item_runs(&unscoped), vec![run.run_id.to_string()]);
    assert_eq!(
        unscoped["items"][0]["consent_scopes"],
        serde_json::json!(["debugging_evaluation"])
    );
    let (status, training_policy) = create_pipeline_export(
        &with_policy(ConsentScope::ModelTraining),
        &fixture.export_token,
        "training-policy",
        10,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{training_policy}");
    assert!(item_runs(&training_policy).is_empty(), "{training_policy}");
    let (status, debugging_policy) = create_pipeline_export(
        &with_policy(ConsentScope::DebuggingEvaluation),
        &fixture.export_token,
        "debugging-policy",
        10,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{debugging_policy}");
    assert_eq!(item_runs(&debugging_policy), vec![run.run_id.to_string()]);
}

/// A snapshot that can no longer be delivered stays as it is, and the
/// complete route says to create a new one. A submission that expired
/// after the snapshot was taken leaves the snapshot `ready` but not
/// deliverable; a withdrawal invalidates the snapshot. Each answer is a
/// label and writes no audit event, and a new snapshot leaves the expired
/// submission out.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_export_complete_route_tells_the_caller_to_create_a_new_snapshot() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = &fixture.base.state;
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let expired = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let withdrawn = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;

    let (status, first) = create_pipeline_export(state, &fixture.export_token, "first", 10).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    let mut owner = fixture
        .base
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = tenant_tx(&mut owner, tenant).await;
    tx.execute(
        "UPDATE trace_submissions SET expires_at = NOW() - INTERVAL '1 second'
          WHERE tenant_id = $1 AND submission_id = $2",
        &[&tenant, &expired.submission_id],
    )
    .await
    .expect("expire the submission");
    tx.commit().await.unwrap();
    drop(owner);
    let owner = &fixture.base.owner;
    assert_eq!(export_audit_events(owner, tenant).await.len(), 1);
    let (status, stale) =
        complete_pipeline_export(state, &fixture.export_token, &first["snapshot_id"]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    assert_eq!(stale["error"], "export_snapshot_stale_create_new_snapshot");
    assert_eq!(
        export_audit_events(owner, tenant).await.len(),
        1,
        "a refused completion writes no audit event"
    );

    let (status, second) = create_pipeline_export(state, &fixture.export_token, "second", 10).await;
    assert_eq!(status, StatusCode::OK, "{second}");
    let items = second["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{second}");
    assert_eq!(items[0]["run_id"], withdrawn.run_id.to_string());
    fixture
        .base
        .service
        .withdraw_submission(tenant, withdrawn.submission_id, &principal, None)
        .await
        .expect("the owner withdraws the submission");
    let (status, invalidated) =
        complete_pipeline_export(state, &fixture.export_token, &second["snapshot_id"]).await;
    assert_eq!(status, StatusCode::CONFLICT, "{invalidated}");
    assert_eq!(
        invalidated["error"],
        "export_snapshot_invalidated_create_new_snapshot"
    );
    assert_eq!(
        export_audit_events(owner, tenant).await.len(),
        2,
        "two creations, and no audit event for either refused completion"
    );
}

/// `POST /v1/contributors/me/submission-status` reports a pipeline
/// submission from its run: the owner gets a document with the pipeline
/// block, whose amounts are decimal strings; another contributor of the
/// tenant gets nothing for that id; without the product store the
/// submission is unknown to the route; and when `main`'s own view has the
/// submission, its document carries the pipeline block.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_status_route_reports_the_pipeline_block_to_the_owner() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = &fixture.base.state;
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let run = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let request = serde_json::json!({"submission_ids": [run.submission_id, Uuid::new_v4()]});
    let status_for = |state: Arc<AppState>, token: String| {
        let request = request.clone();
        async move {
            pipeline_product_request(
                state,
                "POST",
                "/v1/contributors/me/submission-status",
                Some(token.as_str()),
                None,
                Some(request),
            )
            .await
        }
    };

    let (status, documents) = status_for(state.clone(), fixture.base.token.clone()).await;
    assert_eq!(status, StatusCode::OK, "{documents}");
    let documents = documents.as_array().expect("a document list").clone();
    assert_eq!(documents.len(), 1, "{documents:?}");
    let document = &documents[0];
    assert_eq!(document["submission_id"], run.submission_id.to_string());
    assert_eq!(
        document["status"], "accepted",
        "`main`'s vocabulary (Zaki review 1, minor item M-c)"
    );
    let pipeline = &document["pipeline"];
    assert_eq!(pipeline["run_id"], run.run_id.to_string());
    assert_eq!(pipeline["processing_state"], "complete");
    assert_eq!(
        pipeline["instruments"],
        serde_json::json!([{
            "instrument_id": "storage_rebate",
            "atomic_units": "5",
            "operation_state": "complete",
            "internal_settlement_state": "not_applicable",
            "payout_rail": "none",
            "payout_state": "disabled",
            "reason_label": null,
        }])
    );
    let document: TraceSubmissionStatusUpdate =
        serde_json::from_value(document.clone()).expect("the protocol type reads the document");
    assert_eq!(document.pipeline.unwrap().instruments[0].atomic_units, "5");

    let (status, other) = status_for(state.clone(), fixture.base.other_token.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(other, serde_json::json!([]), "not the other contributor's");
    let mut without_product = state.clone();
    Arc::make_mut(&mut without_product).pipeline_product = None;
    let (status, legacy) = status_for(without_product, fixture.base.token.clone()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(legacy, serde_json::json!([]));

    // With contributor reads from the database, `main`'s own view also has
    // the submission: the document is `main`'s, and it carries the pipeline
    // block.
    let mut database_reads = state.clone();
    Arc::make_mut(&mut database_reads).db_contributor_reads = true;
    let (status, documents) = status_for(database_reads, fixture.base.token.clone()).await;
    assert_eq!(status, StatusCode::OK, "{documents}");
    let documents = documents.as_array().expect("a document list").clone();
    assert_eq!(documents.len(), 1, "{documents:?}");
    assert_eq!(documents[0]["status"], "accepted", "main's own status");
    assert_eq!(documents[0]["pipeline"]["run_id"], run.run_id.to_string());
    assert_eq!(documents[0]["pipeline"]["processing_state"], "complete");
}

/// The administrator reads and the score attestation, each for its own
/// caller. The operational summary counts the tenant's complete run, and
/// another tenant's summary does not; the forensic trace of the run is
/// hash-only, a run id the tenant does not have is 404, and so is the run
/// for another tenant's admin. The score attestation, signed with the
/// versioned-pipeline schema, holds the owner's run, and another
/// contributor's attestation holds nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_product_admin_reads_and_score_attestation_are_scoped_to_their_caller() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = test_state_with_attestation_signing(fixture.base.state.clone());
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let run = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let get = |token: String, uri: String| {
        let state = state.clone();
        async move {
            pipeline_product_request(state, "GET", &uri, Some(token.as_str()), None, None).await
        }
    };

    let summary_uri = "/v1/admin/pipeline/operational-summary".to_string();
    let (status, summary) = get(fixture.admin_token.clone(), summary_uri.clone()).await;
    assert_eq!(status, StatusCode::OK, "{summary}");
    let work = summary["work"].as_array().expect("work buckets");
    assert!(
        work.iter()
            .any(|bucket| bucket["state"] == "complete" && bucket["count"] == 1),
        "{summary}"
    );
    assert!(!summary.to_string().contains(tenant));
    let (status, other_summary) = get(fixture.other_tenant_admin_token.clone(), summary_uri).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(other_summary["work"], serde_json::json!([]));

    let forensic_uri = format!("/v1/admin/pipeline/runs/{}/forensic", run.run_id);
    let (status, forensic) = get(fixture.admin_token.clone(), forensic_uri.clone()).await;
    assert_eq!(status, StatusCode::OK, "{forensic}");
    assert_eq!(forensic["run_id"], run.run_id.to_string());
    assert_eq!(forensic["submission_id"], run.submission_id.to_string());
    let phases = forensic["phases"].as_array().expect("phases");
    assert_eq!(phases.len(), 4, "{forensic}");
    assert!(phases.iter().all(|phase| {
        phase["decision_hash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    }));
    let text = forensic.to_string();
    assert!(
        !text.contains(tenant) && !text.contains(&principal),
        "{text}"
    );
    let (status, missing) = get(
        fixture.admin_token.clone(),
        format!("/v1/admin/pipeline/runs/{}/forensic", Uuid::new_v4()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(missing["error"], "pipeline_run_not_found");
    let (status, other_forensic) =
        get(fixture.other_tenant_admin_token.clone(), forensic_uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(other_forensic, missing);

    let attestation_uri = "/v1/contributors/me/pipeline-score-attestation".to_string();
    let decode = |attestation: &serde_json::Value| {
        let decoding_key = DecodingKey::from_ed_pem(TEST_EDDSA_PUBLIC_KEY_PEM.as_bytes())
            .expect("test public key parses");
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();
        jsonwebtoken::decode::<serde_json::Value>(
            attestation["attestation"].as_str().expect("an attestation"),
            &decoding_key,
            &validation,
        )
        .expect("the attestation verifies against the published key")
        .claims
    };
    let (status, own) = get(fixture.base.token.clone(), attestation_uri.clone()).await;
    assert_eq!(status, StatusCode::OK, "{own}");
    let claims = decode(&own);
    assert_eq!(
        claims["schema_version"],
        trace_commons_server::trace_score_attestation::VERSIONED_SCORE_ATTESTATION_SCHEMA_VERSION
    );
    assert_eq!(claims["auth_principal_ref"], principal);
    let submissions = claims["submissions"].as_array().expect("submissions");
    assert_eq!(submissions.len(), 1, "{claims}");
    assert_eq!(submissions[0]["run_id"], run.run_id.to_string());
    assert_eq!(submissions[0]["scored_microcredits"], "0");
    let (status, other) = get(fixture.base.other_token.clone(), attestation_uri).await;
    assert_eq!(status, StatusCode::OK, "{other}");
    assert_eq!(decode(&other)["submissions"], serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// The pipeline review routes' refusals (Ruling T3-9(b), Ruling F-M1).
// ---------------------------------------------------------------------------

/// A Medium-risk receipt by `principal`, not yet run through Review: the run
/// is `pending` at Review as a quarantine, with no assessment, so a reviewer
/// may claim it.
async fn quarantined_pipeline_run(
    service: &PipelineService,
    tenant: &str,
    principal: &str,
) -> trace_commons_server::versioned_pipeline::PipelineRunRecord {
    service
        .register_default_bundle(tenant)
        .await
        .expect("register the bundle");
    let mut envelope = sample_envelope().await;
    envelope.submission_id = Uuid::new_v4();
    make_metadata_only_low_risk(&mut envelope);
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
    let raw = serde_json::to_vec(&envelope).unwrap();
    let key = envelope.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(PipelineReceiptRequest {
            source_session: None,
            tenant_id: tenant,
            actor_principal_ref: principal,
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
    assert_eq!(created.admission_decision, "quarantine");
    assert_eq!(created.state, PipelineRunState::Pending);
    created
}

/// The review routes through the router, against a real runtime:
///
/// - Another reviewer's live claim answers a second reviewer's claim `409`.
/// - An approval that does not resolve the run's quarantine reason answers
///   `422`.
/// - A claim on a run whose submission is no longer operable (withdrawn
///   after the receipt, while the run still waits at Review) answers `404`,
///   "not waiting for review", as for any run a reviewer cannot claim
///   (Ruling T3-9(b)): `409` means only that another reviewer holds it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_review_routes_answer_409_422_and_404_for_an_inoperable_run() {
    let Some(mut fixture) = withdrawal_fixture().await else {
        return;
    };
    let suffix = Uuid::new_v4().simple().to_string();
    let reviewer = format!("token-review-{suffix}");
    let other_reviewer = format!("token-review-other-{suffix}");
    let mut tokens = (*fixture.state.tokens).clone();
    insert_token(&mut tokens, &fixture.tenant, &reviewer, TokenRole::Reviewer);
    insert_token(
        &mut tokens,
        &fixture.tenant,
        &other_reviewer,
        TokenRole::Reviewer,
    );
    Arc::make_mut(&mut fixture.state).tokens = Arc::new(tokens);
    let principal = "principal_sha256:review-routes";
    let claim_uri = |run_id: Uuid| format!("/v1/review/pipeline/runs/{run_id}/claim");

    let held = quarantined_pipeline_run(&fixture.service, &fixture.tenant, principal).await;
    let (status, claim) = route_request(
        fixture.state.clone(),
        "POST",
        &claim_uri(held.run_id),
        auth_headers(&reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let (status, refused) = route_request(
        fixture.state.clone(),
        "POST",
        &claim_uri(held.run_id),
        auth_headers(&other_reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(
        refused["error"],
        "pipeline review claim is held by another reviewer"
    );
    let (status, refused) = route_request(
        fixture.state.clone(),
        "POST",
        &format!("/v1/review/pipeline/runs/{}/assessment", held.run_id),
        auth_headers(&reviewer),
        Some(serde_json::json!({
            "lease_token": claim["lease_token"],
            "recommendation": "approve",
            "reason": "privacy_review_required",
            "resolved_quarantine_reasons": [],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{refused}");
    assert_eq!(refused["error"], "quarantine reason is unresolved");

    let inoperable = quarantined_pipeline_run(&fixture.service, &fixture.tenant, principal).await;
    let mut client = fixture.runtime.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &fixture.tenant).await;
    tx.execute(
        "INSERT INTO trace_withdrawals (
            tenant_id, submission_id, withdrawn_at, prior_status, distribution_reach
         ) VALUES ($1, $2, NOW(), 'accepted', 'not_distributed')",
        &[&fixture.tenant, &inoperable.submission_id],
    )
    .await
    .expect("withdraw the submission after its receipt");
    tx.commit().await.unwrap();
    drop(client);
    let (status, refused) = route_request(
        fixture.state.clone(),
        "POST",
        &claim_uri(inoperable.run_id),
        auth_headers(&reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{refused}");
    assert_eq!(refused["error"], "pipeline run is not waiting for review");
}

/// Zaki review 1, minor item M-a: the pipeline review claim and assessment
/// routes append `trace_audit_events` rows as `main`'s review routes do: a
/// `review_lease` claim row with the lease's expiry, and a `review_decision`
/// row whose reason is the hash of the reviewer's reason and whose metadata
/// carries only labels (the decision, the resulting status, the reason code).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pipeline_review_routes_append_hash_only_audit_rows() {
    let Some(mut fixture) = withdrawal_fixture().await else {
        return;
    };
    let reviewer = format!("token-review-audit-{}", Uuid::new_v4().simple());
    let mut tokens = (*fixture.state.tokens).clone();
    insert_token(&mut tokens, &fixture.tenant, &reviewer, TokenRole::Reviewer);
    Arc::make_mut(&mut fixture.state).tokens = Arc::new(tokens);
    let principal = "principal_sha256:review-audit";
    let run = quarantined_pipeline_run(&fixture.service, &fixture.tenant, principal).await;

    let (status, claim) = route_request(
        fixture.state.clone(),
        "POST",
        &format!("/v1/review/pipeline/runs/{}/claim", run.run_id),
        auth_headers(&reviewer),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let (status, assessed) = route_request(
        fixture.state.clone(),
        "POST",
        &format!("/v1/review/pipeline/runs/{}/assessment", run.run_id),
        auth_headers(&reviewer),
        Some(serde_json::json!({
            "lease_token": claim["lease_token"],
            "recommendation": "reject",
            "reason": "privacy_review_required",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{assessed}");

    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &fixture.tenant).await;
    let rows = tx
        .query(
            "SELECT action, reason, metadata_json, actor_role FROM trace_audit_events
              WHERE tenant_id = $1 AND submission_id = $2
              ORDER BY audit_sequence",
            &[&fixture.tenant, &run.submission_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(rows.len(), 2, "one row per route");
    let (claim_row, decision_row) = (&rows[0], &rows[1]);
    assert_eq!(claim_row.get::<_, String>("action"), "review");
    assert_eq!(claim_row.get::<_, String>("actor_role"), "reviewer");
    let claim_metadata: serde_json::Value = claim_row.get("metadata_json");
    assert_eq!(claim_metadata["action"], "claim", "{claim_metadata}");
    assert_eq!(decision_row.get::<_, String>("action"), "review");
    assert_eq!(
        decision_row.get::<_, Option<String>>("reason").as_deref(),
        Some(format!("reason_hash={}", sha256_prefixed("privacy_review_required")).as_str())
    );
    let decision_metadata: serde_json::Value = decision_row.get("metadata_json");
    assert_eq!(
        decision_metadata["decision"], "rejected",
        "the resulting status, as `main` records it: {decision_metadata}"
    );
    assert_eq!(
        decision_metadata["resulting_status"], "rejected",
        "{decision_metadata}"
    );
    assert_eq!(
        decision_metadata["reason_code"], "privacy_review_required",
        "{decision_metadata}"
    );
}

/// Zaki review 1, minor item M-f: `main`'s gate evaluate route
/// (`POST /v1/workers/gate/evaluate`) refuses a submission that has a
/// pipeline run with a label-only `409`, before it scores anything, so the
/// gate path cannot award a second `NoveltyUtility` credit for a trace the
/// pipeline credits. No gate decision is written. A submission with no run
/// is not refused by this check.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mains_gate_evaluate_route_refuses_a_submission_with_a_pipeline_run() {
    let Some(mut fixture) = withdrawal_fixture().await else {
        return;
    };
    let vector_token = format!("token-vector-{}", Uuid::new_v4().simple());
    let mut tokens = (*fixture.state.tokens).clone();
    insert_token(
        &mut tokens,
        &fixture.tenant,
        &vector_token,
        TokenRole::VectorWorker,
    );
    Arc::make_mut(&mut fixture.state).tokens = Arc::new(tokens);
    let principal = static_token_principal_ref(&fixture.token);
    let run = completed_pipeline_run(&fixture.service, &fixture.tenant, &principal).await;

    let (status, body) = route_request(
        fixture.state.clone(),
        "POST",
        "/v1/workers/gate/evaluate",
        auth_headers(&vector_token),
        Some(serde_json::json!({"submission_id": run.submission_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "pipeline_run_owns_submission");
    let decisions: i64 = fixture
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT COUNT(*) FROM trace_gate_decisions WHERE submission_id = $1",
            &[&run.submission_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(decisions, 0, "nothing was scored");

    let legacy =
        insert_account_test_submission(fixture.owner.as_ref(), &fixture.tenant, &principal).await;
    let (status, body) = route_request(
        fixture.state.clone(),
        "POST",
        "/v1/workers/gate/evaluate",
        auth_headers(&vector_token),
        Some(serde_json::json!({"submission_id": legacy})),
    )
    .await;
    assert_ne!(
        body["error"], "pipeline_run_owns_submission",
        "a submission with no run is not refused by this check ({status})"
    );
}

// ---------------------------------------------------------------------------
// `main`'s DB/file reconciliation of a tenant with pipeline rows (Ruling F-M10).
// ---------------------------------------------------------------------------

/// A minimal-family service that awards Trace Credit on the `near` rail and
/// pays it out through `near`, on `artifacts`: its runs write a pipeline
/// credit event, its finalized batch, and, once paid, a NEAR outbox line.
fn trace_credit_payout_service(
    backend: Arc<PgBackend>,
    artifacts: Arc<LocalEncryptedTraceArtifactStore>,
    near: Arc<RecordingNearAdapter>,
) -> Arc<PipelineService> {
    let scorer = Arc::new(ReferencePerplexityScorer::new());
    let embedder = Arc::new(ReferenceEmbedder::new());
    let package = MinimalPolicyBundle::minimal_package(
        &PipelineBundleConfig {
            instrument_awards: vec![PipelineInstrumentAwardConfig {
                instrument_id: InstrumentId::trace_credit().as_str().to_string(),
                atomic_units: AtomicUnits::from_raw(1_000_000),
                descriptor: InstrumentDescriptor {
                    kind: InstrumentKind::Nep141,
                    network: "testnet".to_string(),
                    contract: "trace-credit.testnet".to_string(),
                    decimals: trace_commons_gate_api::pipeline::TRACE_CREDIT_DECIMALS,
                },
            }],
            include_index: true,
            variant: None,
        },
        scorer.as_ref(),
        embedder.as_ref(),
    )
    .expect("build the minimal package");
    let index = IsolatedPipelineIndex::new();
    let registry = SettlementAdapterRegistry::new(vec![RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_reconciliation_test_only",
        "near",
    ) as Arc<dyn SettlementAdapter>])
    .expect("build the adapter registry");
    let caps = PipelineCaps {
        per_instrument_atomic_units: BTreeMap::from([(
            InstrumentId::trace_credit().as_str().to_string(),
            AtomicUnits::from_raw(u128::MAX),
        )]),
    };
    Arc::new(
        PipelineServiceBuilder::new(
            backend,
            artifacts,
            package,
            index.clone(),
            index,
            registry,
            caps,
        )
        .with_scorer(scorer)
        .with_embedder(embedder)
        .with_authority(allow_all_test_authority())
        .with_privacy(Arc::new(PassThroughPipelinePrivacyBoundary))
        .with_payout(
            near,
            PipelinePayoutConfig {
                enabled: true,
                require_confirmation_evidence: true,
                near_contract_id: Some("trace-credits.testnet".to_string()),
                confirmation_interval: std::time::Duration::ZERO,
                controls: TEST_NEAR_PAYOUT_CONTROLS,
            },
        )
        .build()
        .expect("build the payout service"),
    )
}

/// `POST /v1/admin/db-reconciliation-drill` for the tenant of `admin_token`:
/// `(ready, blocking_gaps)`.
async fn db_reconciliation_drill(state: &Arc<AppState>, admin_token: &str) -> (bool, Vec<String>) {
    let (status, body) = route_request(
        state.clone(),
        "POST",
        "/v1/admin/db-reconciliation-drill",
        auth_headers(admin_token),
        Some(serde_json::json!({ "purpose": "pipeline reconciliation test" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let gaps = body["blocking_gaps"]
        .as_array()
        .expect("blocking gaps")
        .iter()
        .map(|gap| gap.as_str().expect("a gap label").to_string())
        .collect();
    (body["ready"].as_bool().expect("ready"), gaps)
}

/// `main`'s DB/file reconciliation compares its file mirror with the
/// database. The pipeline writes database rows only (Ruling T7-10, RB-14),
/// so each of them is identified by its pipeline run and left out of every
/// file-versus-database comparison it would fail; `main`'s own rows keep
/// every check.
///
/// - Tenant A has a legacy submission (accepted and vector-indexed, as a
///   deployment's worker leaves it), and two minimal-family pipeline runs
///   that each settled Trace Credit into a finalized batch and were paid
///   out through a NEAR outbox line; both were exported, and one was then
///   withdrawn. The drill is ready, with no blocking gap.
/// - Tenant B has a compatibility run, whose `NoveltyUtility` credit event
///   has no batch. The drill is ready too.
/// - A legacy submission row in tenant A's database with no file record is
///   still a blocking gap.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn db_reconciliation_leaves_pipeline_rows_out_of_the_file_comparison() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-reconcile-{suffix}");
    let compat_tenant = format!("tenant-reconcile-compat-{suffix}");
    let token = format!("token-reconcile-{suffix}");
    let admin_token = format!("token-reconcile-admin-{suffix}");
    let vector_token = format!("token-reconcile-vector-{suffix}");
    let export_token = format!("token-reconcile-export-{suffix}");
    let compat_admin_token = format!("token-reconcile-compat-admin-{suffix}");
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, &token, TokenRole::Contributor);
    insert_token(&mut tokens, &tenant, &admin_token, TokenRole::Admin);
    insert_token(&mut tokens, &tenant, &vector_token, TokenRole::VectorWorker);
    insert_token(&mut tokens, &tenant, &export_token, TokenRole::ExportWorker);
    insert_token(
        &mut tokens,
        &compat_tenant,
        &compat_admin_token,
        TokenRole::Admin,
    );
    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        Some(mains_database().await),
        Some(artifacts.clone()),
        false,
        false,
        false,
        false,
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.tokens = Arc::new(tokens);
    state_mut.require_db_mirror_writes = true;
    state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));

    // ---- A legacy submission, with its file record and its DB mirror ----
    let mut legacy = sample_envelope().await;
    make_metadata_only_low_risk(&mut legacy);
    let _ = submit_trace_handler(
        State(state.clone()),
        auth_headers(&token),
        submit_body(legacy),
    )
    .await
    .expect("the legacy submission mirrors to the database");
    let _ = vector_index_handler(
        State(state.clone()),
        auth_headers(&vector_token),
        Json(TraceVectorIndexRequest {
            purpose: Some("reconciliation test vector index".to_string()),
            dry_run: false,
            limit: None,
        }),
    )
    .await
    .expect("the vector worker indexes the legacy submission");
    let (ready, gaps) = db_reconciliation_drill(&state, &admin_token).await;
    assert!(ready && gaps.is_empty(), "legacy only: {gaps:?}");

    // ---- Pipeline rows: two paid runs, exported, one withdrawn ----
    let near = Arc::new(RecordingNearAdapter::new());
    let service = trace_credit_payout_service(runtime.clone(), artifacts.clone(), near.clone());
    let principal = "principal_sha256:reconcile";
    let mut runs = Vec::new();
    for _ in 0..2 {
        let mut envelope = sample_envelope().await;
        envelope.submission_id = Uuid::new_v4();
        make_metadata_only_low_risk(&mut envelope);
        runs.push(completed_run_of(&service, &tenant, principal, &envelope).await);
    }
    assert_eq!(service.process_payouts(&tenant, 32).await.unwrap(), 2);
    assert_eq!(near.requests().len(), 2, "both runs were paid out");
    let (status, created) = create_pipeline_export(&state, &export_token, "reconcile", 10).await;
    assert_eq!(status, StatusCode::OK, "{created}");
    assert_eq!(
        created["items"].as_array().map(Vec::len),
        Some(2),
        "{created}"
    );
    let (status, completed) =
        complete_pipeline_export(&state, &export_token, &created["snapshot_id"]).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    service
        .withdraw_submission(&tenant, runs[1].submission_id, principal, None)
        .await
        .expect("the owner withdraws the second run");
    let (ready, gaps) = db_reconciliation_drill(&state, &admin_token).await;
    assert!(
        gaps.is_empty(),
        "pipeline rows are not file-versus-database gaps: {gaps:?}"
    );
    assert!(ready);

    // ---- A compatibility run: a NoveltyUtility event with no batch ----
    let compat = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
        IsolatedPipelineIndex::new(),
        2_500_000,
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    let mut envelope = model_training_envelope().await;
    envelope.submission_id = Uuid::new_v4();
    completed_run_of(&compat, &compat_tenant, principal, &envelope).await;
    let (ready, gaps) = db_reconciliation_drill(&state, &compat_admin_token).await;
    assert!(
        gaps.is_empty(),
        "a compatibility run's rows are not gaps: {gaps:?}"
    );
    assert!(ready);

    // ---- A legacy row with no file record is still a gap ----
    let db_only = Uuid::new_v4();
    owner
        .upsert_trace_submission(StorageTraceSubmissionWrite {
            tenant_id: tenant.clone(),
            submission_id: db_only,
            trace_id: Uuid::new_v4(),
            auth_principal_ref: principal_storage_ref(&token),
            contributor_pseudonym: None,
            submitted_tenant_scope_ref: Some(tenant_storage_ref(&tenant)),
            schema_version: "trace_contribution.v1".to_string(),
            consent_policy_version: "trace-consent-v1".to_string(),
            consent_scopes: vec!["debugging_evaluation".to_string()],
            allowed_uses: vec!["debugging".to_string()],
            retention_policy_id: "retention-debugging-evaluation-v1".to_string(),
            status: StorageTraceCorpusStatus::Quarantined,
            privacy_risk: "medium".to_string(),
            redaction_pipeline_version: "test-redactor-v1".to_string(),
            redaction_counts: BTreeMap::new(),
            redaction_hash: sha256_prefixed(&format!("{tenant}:{db_only}")),
            canonical_summary_hash: None,
            submission_score: None,
            credit_points_pending: None,
            credit_points_final: None,
            expires_at: None,
            residual_risk_basis: None,
        })
        .await
        .expect("write a legacy submission row with no file record");
    let (ready, gaps) = db_reconciliation_drill(&state, &admin_token).await;
    assert!(!ready);
    assert!(
        gaps.iter()
            .any(|gap| gap == "missing_submission_ids_in_files=1"),
        "a legacy database row with no file record is a gap: {gaps:?}"
    );
}

// ---------------------------------------------------------------------------
// The compatibility bundle's Trace Credit, read by `main`'s credit readers.
// ---------------------------------------------------------------------------

/// P5: `TestAssembler` with the compatibility bundle instead of the minimal
/// one: `CompatibilityBundleConfig::local_reference()` with the given
/// `NoveltyUtility` delta, the reference scorer and embedder, the isolated
/// index, the given settlement adapters (one `trace_credit` recording
/// adapter on payout rail `none` unless a caller shares its own), the given
/// privacy boundary, and an optional crash point. Payout stays disabled
/// (the default).
struct CompatibilityTestAssembler {
    index: Arc<IsolatedPipelineIndex>,
    novelty_utility_microcredits: u64,
    privacy: Arc<dyn PipelinePrivacyBoundary>,
    adapters: Vec<Arc<dyn SettlementAdapter>>,
    crash_point: Option<PipelineCrashPoint>,
}

impl IngestPipelineRuntimeAssembler for CompatibilityTestAssembler {
    fn assemble(
        &self,
        context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        let scorer = Arc::new(ReferencePerplexityScorer::new());
        let embedder = Arc::new(ReferenceEmbedder::new());
        let package = compatibility_reference_package(
            self.novelty_utility_microcredits,
            scorer.as_ref(),
            embedder.as_ref(),
        )?;
        let registry = SettlementAdapterRegistry::new(self.adapters.clone())?;
        let caps = PipelineCaps {
            per_instrument_atomic_units: BTreeMap::from([(
                InstrumentId::trace_credit().as_str().to_string(),
                AtomicUnits::from_raw(u128::MAX),
            )]),
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
        .with_object_store_name(context.object_store_name)
        .with_novelty_utility_checks(context.novelty_utility_checks)
        .with_authority(allow_all_test_authority())
        .with_privacy(self.privacy.clone());
        if let Some(crash_point) = self.crash_point {
            builder = builder.with_crash_point(crash_point);
        }
        Ok(Arc::new(builder.build()?))
    }
}

/// The pipeline credit issuer the compatibility HTTP tests configure
/// (`TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`).
const TEST_PIPELINE_CREDIT_ISSUER: &str =
    "principal_sha256:1111111111111111111111111111111111111111111111111111111111111111";

/// `assemble_test_pipeline_service` for `CompatibilityTestAssembler`: the
/// service comes out of `assemble_ingest_pipeline_runtime`, the seam ingest's
/// real boot uses, over `configured_store`, with its own `trace_credit`
/// recording adapter and no crash point.
fn assemble_compatibility_pipeline_service(
    backend: Arc<PgBackend>,
    configured_store: &ConfiguredTraceArtifactStore,
    index: Arc<IsolatedPipelineIndex>,
    novelty_utility_microcredits: u64,
    privacy: Arc<dyn PipelinePrivacyBoundary>,
) -> Arc<PipelineService> {
    let trace_credit: Arc<dyn SettlementAdapter> = RecordingSettlementAdapter::new(
        InstrumentId::trace_credit(),
        "recording_trace_credit_compatibility_http_test_only",
        "none",
    );
    assemble_compatibility_pipeline_service_with(
        backend,
        configured_store,
        index,
        novelty_utility_microcredits,
        privacy,
        vec![trace_credit],
        None,
    )
}

/// `assemble_compatibility_pipeline_service` with the caller's settlement
/// adapters and crash point: the restore drill's seed shares one recording
/// adapter across two app lifetimes and crashes the second one
/// (`pipeline_restore_pg_tests`), as `TestAssembler` does for the minimal
/// bundle.
pub(super) fn assemble_compatibility_pipeline_service_with(
    backend: Arc<PgBackend>,
    configured_store: &ConfiguredTraceArtifactStore,
    index: Arc<IsolatedPipelineIndex>,
    novelty_utility_microcredits: u64,
    privacy: Arc<dyn PipelinePrivacyBoundary>,
    adapters: Vec<Arc<dyn SettlementAdapter>>,
    crash_point: Option<PipelineCrashPoint>,
) -> Arc<PipelineService> {
    let assembler = CompatibilityTestAssembler {
        index,
        novelty_utility_microcredits,
        privacy,
        adapters,
        crash_point,
    };
    let connections = TraceCorpusDbConnections {
        database: backend.clone() as Arc<dyn Database>,
        postgres: backend,
    };
    assemble_ingest_pipeline_runtime(
        Some(&assembler),
        Some(&connections),
        Some(configured_store),
        false,
        trace_commons_server::versioned_pipeline::PipelineLeaseConfig::default(),
        true,
        true,
        None,
        TEST_NEAR_CONFIRMATION_INTERVAL,
        TEST_NEAR_PAYOUT_CONTROLS,
        // A runtime that routes the compatibility bundle needs the
        // pipeline's credit issuer (Zaki review 1, round 2, finding 15).
        &PipelineNoveltyUtilityChecks {
            issuer_principal_ref: Some(TEST_PIPELINE_CREDIT_ISSUER.to_string()),
            ..PipelineNoveltyUtilityChecks::default()
        },
        TEST_EMBED_INSERT_NOVELTY_MICROS,
    )
    .expect("assemble the injected compatibility pipeline runtime")
    .expect("an assembler was given, so a service is returned")
}

/// A metadata-only, Low-risk envelope whose consent allows model training,
/// the allowed use `main`'s `NoveltyUtility` credit requires.
async fn model_training_envelope() -> TraceContributionEnvelope {
    let mut envelope = sample_envelope().await;
    make_metadata_only_low_risk(&mut envelope);
    envelope.consent.scopes = vec![ConsentScope::ModelTraining];
    envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
    envelope
}

/// `method uri` through `app()` with `headers` (and a JSON `body`), returning
/// the status and the body, parsed as JSON when it is JSON.
async fn route_request(
    state: Arc<AppState>,
    method: &str,
    uri: &str,
    headers: HeaderMap,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    use tower::ServiceExt;

    let builder = axum::http::Request::builder().method(method).uri(uri);
    let mut request = match body {
        Some(body) => builder
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string())),
        None => builder.body(axum::body::Body::empty()),
    }
    .unwrap();
    request.headers_mut().extend(headers);
    let response = app(state).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    let text = String::from_utf8_lossy(&bytes).to_string();
    let body = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
    (status, body)
}

/// Ruling T15-1: a compatibility run's `NoveltyUtility` ledger row carries
/// the actor role `main`'s gate path records for that event
/// (`vector_worker`). `main`'s database credit readers parse the role of
/// every event type they map, and refuse the whole read on a role that is
/// not a token role. With contributor and reviewer reads from PostgreSQL, as
/// the pilot runs, a tenant with compatibility runs with a positive delta
/// gets 200 from the submission-status route, the credit route, and both
/// withdrawal routes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compatibility_credit_rows_stay_readable_by_mains_database_reads() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-compat-reads-{suffix}");
    let token = format!("token-compat-reads-{suffix}");
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, &token, TokenRole::Contributor);
    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
        IsolatedPipelineIndex::new(),
        2_500_000,
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        Some(mains_database().await),
        Some(artifacts),
        true,
        true,
        false,
        false,
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.tokens = Arc::new(tokens);
    state_mut.pipeline_service = Some(service.clone());
    state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
    let principal = static_token_principal_ref(&token);
    let first = completed_run_of(
        &service,
        &tenant,
        &principal,
        &model_training_envelope().await,
    )
    .await;
    let second = completed_run_of(
        &service,
        &tenant,
        &principal,
        &model_training_envelope().await,
    )
    .await;

    let (status, documents) = route_request(
        state.clone(),
        "POST",
        "/v1/contributors/me/submission-status",
        auth_headers(&token),
        Some(serde_json::json!({
            "submission_ids": [first.submission_id, second.submission_id],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{documents}");
    assert_eq!(documents.as_array().map(Vec::len), Some(2), "{documents}");
    let (status, credit) = route_request(
        state.clone(),
        "GET",
        "/v1/contributors/me/credit",
        auth_headers(&token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{credit}");
    assert_eq!(
        credit["credit_points_ledger"].as_f64(),
        Some(5.0),
        "both runs' events count: {credit}"
    );
    let session = account_session_headers(&state, &token).await;
    let (status, withdrawal) = route_request(
        state.clone(),
        "POST",
        &format!(
            "/v1/contributors/me/pipeline-submissions/{}/withdraw",
            first.submission_id
        ),
        session.clone(),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{withdrawal}");
    let (status, withdrawal) = route_request(
        state.clone(),
        "POST",
        &format!("/v1/account/traces/{}/withdraw", second.submission_id),
        session,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{withdrawal}");

    // The rows the routes above read.
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &tenant).await;
    let rows = tx
        .query(
            "SELECT event_type, actor_role FROM trace_credit_ledger WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get::<_, String>(0), row.get::<_, String>(1)))
        .collect::<Vec<_>>();
    tx.commit().await.unwrap();
    assert_eq!(
        rows,
        vec![("novelty_utility".to_string(), "vector_worker".to_string()); 2]
    );
}

/// The shadow credit quality a run's committed Score outcome recorded, as
/// the gate-decision row `main` builds its credit figure and explanation
/// from.
async fn score_shadow_credit_decision(
    runtime: &Arc<PgBackend>,
    tenant_id: &str,
    run: &trace_commons_server::versioned_pipeline::PipelineRunRecord,
) -> StorageTraceGateCreditDecisionRow {
    let mut client = runtime.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let evidence: serde_json::Value = tx
        .query_one(
            "SELECT evidence FROM phase_outcomes
              WHERE tenant_id = $1 AND run_id = $2 AND phase = 'score'",
            &[&tenant_id, &run.run_id],
        )
        .await
        .expect("the run's Score outcome")
        .get(0);
    tx.commit().await.unwrap();
    let int = |field: &str| evidence[field].as_i64();
    StorageTraceGateCreditDecisionRow {
        submission_id: run.submission_id,
        credit_quality_micros: int("credit_quality_micros"),
        credit_quality_calibration_version: int("credit_quality_version").map(|v| v as i32),
        credit_withheld_reason: None,
        chunk_count: int("chunk_count").map(|v| v as i32),
        total_chunk_count: int("total_chunk_count").map(|v| v as i32),
        chunks_capped: evidence["chunks_capped"].as_bool(),
    }
}

/// Ruling T15-7: a compatibility run's contributor status reads as `main`'s
/// document for the same credit, with contributor reads from the database
/// and from files alike: status `accepted`; its `NoveltyUtility` event
/// counted as ledger credit (ledger and total 2.5, with `main`'s delayed
/// credit line); and, where `main` shows its gate's credit-quality figure,
/// the Score evidence's shadow credit quality with `main`'s explanation
/// line, not "scoring in progress". The pipeline block comes with it. The
/// minimal family keeps its own document under file reads (Ruling T14-9;
/// `pipeline_status_route_reports_the_pipeline_block_to_the_owner`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compatibility_status_reads_as_mains_under_both_read_modes() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    // Migrates the suite's database and resets the account rate limiter.
    account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-compat-status-{suffix}");
    let token = format!("token-compat-status-{suffix}");
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, &token, TokenRole::Contributor);
    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
        IsolatedPipelineIndex::new(),
        2_500_000,
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    let principal = static_token_principal_ref(&token);
    let run = completed_run_of(
        &service,
        &tenant,
        &principal,
        &model_training_envelope().await,
    )
    .await;
    let shadow = score_shadow_credit_decision(&runtime, &tenant, &run).await;
    let shadow_quality = shadow
        .credit_quality_micros
        .expect("a compatibility Score records a shadow credit quality");
    let expected_pending = f64::from(credit_points_from_quality_micros(shadow_quality));
    let expected_basis = gate_credit_basis_line(&shadow);
    assert!(
        expected_basis.starts_with("Credit reflects the gate's scoring (calibration V"),
        "{expected_basis}"
    );

    for database_reads in [true, false] {
        let mut state = test_state_with_options(
            dir.path().to_path_buf(),
            Some(mains_database().await),
            Some(artifacts.clone()),
            database_reads,
            database_reads,
            false,
            false,
        );
        let state_mut = Arc::make_mut(&mut state);
        state_mut.tokens = Arc::new(tokens.clone());
        state_mut.pipeline_service = Some(service.clone());
        state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
        state_mut.pipeline_drain_tenant_ids = Arc::new(BTreeSet::from([tenant.clone()]));
        let (status, documents) = route_request(
            state,
            "POST",
            "/v1/contributors/me/submission-status",
            auth_headers(&token),
            Some(serde_json::json!({ "submission_ids": [run.submission_id] })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{documents}");
        let documents = documents.as_array().expect("a document list").clone();
        assert_eq!(documents.len(), 1, "{documents:?}");
        let document = &documents[0];
        let mode = if database_reads { "database" } else { "file" };
        assert_eq!(document["status"], "accepted", "{mode} reads: {document}");
        let pending = document["credit_points_pending"]
            .as_f64()
            .expect("a pending figure");
        assert!(
            (pending - expected_pending).abs() < 1e-4,
            "{mode} reads: the shadow credit quality is the pending figure: {document}"
        );
        assert!(
            document.get("credit_points_final").is_none(),
            "{mode} reads: {document}"
        );
        assert_eq!(
            document["credit_points_ledger"].as_f64(),
            Some(2.5),
            "{mode} reads: {document}"
        );
        assert_eq!(
            document["credit_points_total"].as_f64(),
            Some(2.5),
            "{mode} reads: {document}"
        );
        let explanation = document["explanation"]
            .as_array()
            .expect("explanation lines")
            .iter()
            .map(|line| line.as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(
            explanation.contains(&expected_basis),
            "{mode} reads: {explanation:?}"
        );
        assert!(
            !explanation
                .iter()
                .any(|line| line == SCORING_IN_PROGRESS_LINE),
            "{mode} reads: {explanation:?}"
        );
        assert_eq!(
            document["delayed_credit_explanations"],
            serde_json::json!([
                "NoveltyUtility: +2.50 (novelty_utility:compatibility_quality_novelty_v1)"
            ]),
            "{mode} reads: {document}"
        );
        assert_eq!(
            document["consent_scopes"],
            serde_json::json!(["model_training"]),
            "{mode} reads: {document}"
        );
        assert_eq!(document["pipeline"]["run_id"], run.run_id.to_string());
    }
}

/// Zaki review 1, round 2, item 1: one status request for several
/// compatibility runs in different states -- scored and settled, past Review
/// with Score to come, withdrawn, and waiting for review (a `received`
/// submission, which falls back to the pipeline's document) -- answers with
/// the same documents, in the asked order, as one request per id, under file
/// and database contributor reads alike. The batched path reads those
/// submissions and their credit events once for the request.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_status_request_for_several_compatibility_runs_matches_one_request_per_id() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    // Migrates the suite's database and resets the account rate limiter.
    account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-compat-batch-{suffix}");
    let token = format!("token-compat-batch-{suffix}");
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, &token, TokenRole::Contributor);
    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
        IsolatedPipelineIndex::new(),
        2_500_000,
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    let principal = static_token_principal_ref(&token);

    let settled = completed_run_of(
        &service,
        &tenant,
        &principal,
        &model_training_envelope().await,
    )
    .await;
    let withdrawn = completed_run_of(
        &service,
        &tenant,
        &principal,
        &model_training_envelope().await,
    )
    .await;
    service
        .withdraw_submission(&tenant, withdrawn.submission_id, &principal, None)
        .await
        .expect("the owner withdraws a run");
    let mut envelope = model_training_envelope().await;
    envelope.submission_id = Uuid::new_v4();
    let raw = serde_json::to_vec(&envelope).unwrap();
    let key = envelope.submission_id.to_string();
    let PipelineReceiptResult::Created(created) = service
        .submit(PipelineReceiptRequest {
            source_session: None,
            tenant_id: &tenant,
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
    service
        .process_run(&tenant, created.run_id)
        .await
        .expect("Review runs");
    let quarantined = quarantined_pipeline_run(&service, &tenant, &principal).await;
    let ids = [
        settled.submission_id,
        created.submission_id,
        withdrawn.submission_id,
        quarantined.submission_id,
    ];

    for database_reads in [false, true] {
        let mut state = test_state_with_options(
            dir.path().to_path_buf(),
            Some(mains_database().await),
            Some(artifacts.clone()),
            database_reads,
            database_reads,
            false,
            false,
        );
        let state_mut = Arc::make_mut(&mut state);
        state_mut.tokens = Arc::new(tokens.clone());
        state_mut.pipeline_service = Some(service.clone());
        state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
        state_mut.pipeline_drain_tenant_ids = Arc::new(BTreeSet::from([tenant.clone()]));
        let ask = |submission_ids: Vec<Uuid>| {
            let state = state.clone();
            let token = token.clone();
            async move {
                let (status, documents) = route_request(
                    state,
                    "POST",
                    "/v1/contributors/me/submission-status",
                    auth_headers(&token),
                    Some(serde_json::json!({ "submission_ids": submission_ids })),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{documents}");
                documents.as_array().expect("a document list").clone()
            }
        };
        let batched = ask(ids.to_vec()).await;
        let mut one_by_one = Vec::new();
        for id in ids {
            one_by_one.extend(ask(vec![id]).await);
        }
        assert_eq!(batched.len(), ids.len(), "{batched:?}");
        assert_eq!(
            batched, one_by_one,
            "database reads {database_reads}: the batched documents are the per-id ones"
        );
        let statuses = batched
            .iter()
            .map(|document| document["status"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            statuses,
            ["accepted", "accepted", "revoked", "quarantined"],
            "database reads {database_reads}"
        );
    }
}

/// Zaki review 1, round 2, item 6: the status route reads the pipeline's
/// view only for a tenant on the receipts or the drain list, the tenants
/// whose retried uploads replay pipeline receipts
/// (`pipeline_runtime_for_replay`). With a runtime injected, a tenant on
/// neither list gets `main`'s answer alone, so its pipeline-only submission
/// is not described; the same request describes it once the tenant is on
/// either list.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_status_route_reads_the_pipeline_only_for_a_routed_or_drained_tenant() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    // Migrates the suite's database and resets the account rate limiter.
    account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-status-lists-{suffix}");
    let token = format!("token-status-lists-{suffix}");
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, &token, TokenRole::Contributor);
    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
        IsolatedPipelineIndex::new(),
        2_500_000,
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    let principal = static_token_principal_ref(&token);
    let settled = completed_run_of(
        &service,
        &tenant,
        &principal,
        &model_training_envelope().await,
    )
    .await;

    for (receipts, drained, described) in [
        (false, false, false),
        (false, true, true),
        (true, false, true),
    ] {
        let mut state = test_state_with_options(
            dir.path().to_path_buf(),
            Some(mains_database().await),
            Some(artifacts.clone()),
            false,
            false,
            false,
            false,
        );
        let state_mut = Arc::make_mut(&mut state);
        state_mut.tokens = Arc::new(tokens.clone());
        state_mut.pipeline_service = Some(service.clone());
        state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
        if receipts {
            state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
                TraceTenantRolloutFeature::PipelineReceipts,
                &[tenant.as_str()],
            );
        }
        if drained {
            state_mut.pipeline_drain_tenant_ids = Arc::new(BTreeSet::from([tenant.clone()]));
        }
        let (status, documents) = route_request(
            state,
            "POST",
            "/v1/contributors/me/submission-status",
            auth_headers(&token),
            Some(serde_json::json!({ "submission_ids": [settled.submission_id] })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{documents}");
        let documents = documents.as_array().expect("a document list").clone();
        if described {
            assert_eq!(documents.len(), 1, "receipts {receipts}, drained {drained}");
            assert_eq!(documents[0]["status"], "accepted");
        } else {
            assert_eq!(
                documents,
                Vec::<serde_json::Value>::new(),
                "a tenant on neither list gets main's answer alone"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// The compatibility bundle through the real router and worker, and parity
// with the legacy path under the same configuration.
// ---------------------------------------------------------------------------

/// Replaces a marker token in the envelope, as a redacting classifier would.
/// The same test double as `versioned_pipeline_runtime_pg.rs`'s
/// `MarkerRedactingBoundary` (test doubles live in the test files, P5).
struct MarkerRedactingBoundary;

#[async_trait::async_trait]
impl PipelinePrivacyBoundary for MarkerRedactingBoundary {
    async fn rescrub(
        &self,
        envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        let text = serde_json::to_string(envelope)?;
        *envelope = serde_json::from_str(&text.replace("MARKER_SECRET", "[redacted]"))?;
        Ok(Vec::new())
    }

    fn dependency_identity(&self) -> &str {
        "marker_redacting_boundary_test_only"
    }

    fn is_production_compatible(&self) -> bool {
        false
    }
}

/// A privacy boundary whose rescrub always fails: the classifier outage a
/// receipt must fail closed on. The same test double as
/// `versioned_pipeline_runtime_pg.rs`'s `FailingPrivacyBoundary`.
struct FailingPrivacyBoundary;

#[async_trait::async_trait]
impl PipelinePrivacyBoundary for FailingPrivacyBoundary {
    async fn rescrub(
        &self,
        _envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        anyhow::bail!("privacy classifier unavailable (test double)")
    }

    fn dependency_identity(&self) -> &str {
        "failing_privacy_boundary_test_only"
    }

    fn is_production_compatible(&self) -> bool {
        false
    }
}

/// Polls the state of `submission_id`'s run in `tenant_id` every 100 ms, up
/// to 60 s, until it is `expected`; the live worker moves the run, never this
/// test. Panics with the last state it saw when the bound passes.
async fn wait_for_run_state(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: Uuid,
    expected: &str,
) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let mut client = backend.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant_id).await;
        let state: Option<String> = tx
            .query_opt(
                "SELECT state FROM pipeline_runs WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant_id, &submission_id],
            )
            .await
            .unwrap()
            .map(|row| row.get(0));
        tx.commit().await.unwrap();
        if state.as_deref() == Some(expected) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out after 60s waiting for the worker to move the run to {expected} \
             (last observed state: {state:?})"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// The run the pipeline created for `submission_id`.
async fn run_of_submission(
    service: &PipelineService,
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: Uuid,
) -> trace_commons_server::versioned_pipeline::PipelineRunRecord {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let run_id: Uuid = tx
        .query_one(
            "SELECT run_id FROM pipeline_runs WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant_id, &submission_id],
        )
        .await
        .expect("the submission has a run")
        .get(0);
    tx.commit().await.unwrap();
    drop(client);
    service
        .store()
        .get_run(tenant_id, run_id)
        .await
        .unwrap()
        .expect("the run exists")
}

/// Sends `body` (JSON, when given) to `url` over real HTTP with `headers`,
/// and returns the status and the response body, parsed as JSON when it is
/// JSON and as a string otherwise.
pub(super) async fn send_http(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    headers: HeaderMap,
    body: Option<serde_json::Value>,
) -> (reqwest::StatusCode, serde_json::Value) {
    let mut request = client
        .request(method, url)
        .headers(reqwest_headers(headers));
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(body.to_string());
    }
    let response = request
        .send()
        .await
        .expect("the request reaches the server");
    let status = response.status();
    let text = response.text().await.expect("a response body");
    let body = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
    (status, body)
}

/// `POST /v1/traces` of `body` with `token`.
pub(super) async fn post_trace(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    body: &[u8],
) -> (reqwest::StatusCode, serde_json::Value) {
    let response = client
        .post(format!("{base}/v1/traces"))
        .headers(reqwest_headers(auth_headers(token)))
        .header("content-type", "application/json")
        .body(body.to_vec())
        .send()
        .await
        .expect("submit over real HTTP");
    let status = response.status();
    let text = response.text().await.expect("a receipt body");
    let body = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
    (status, body)
}

/// `POST /v1/contributors/me/submission-status` for `submission_ids` with
/// `token`.
pub(super) async fn post_submission_status(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    submission_ids: &[Uuid],
) -> (reqwest::StatusCode, serde_json::Value) {
    send_http(
        client,
        reqwest::Method::POST,
        format!("{base}/v1/contributors/me/submission-status"),
        auth_headers(token),
        Some(serde_json::json!({ "submission_ids": submission_ids })),
    )
    .await
}

/// `headers` with `Bearer token` and an `idempotency-key`.
fn bearer_with_idempotency_key(token: &str, key: &str) -> HeaderMap {
    let mut headers = auth_headers(token);
    headers.insert("idempotency-key", key.parse().unwrap());
    headers
}

/// The compatibility bundle through the router and worker ingest boots
/// (`run_pipeline_app`), with no direct processor call. Tenant A is routed to
/// a service holding the compatibility bundle (`local_reference()`, delta
/// 2_500_000) and `MarkerRedactingBoundary`:
///
/// - `POST /v1/traces` answers `processing`, and the worker completes the
///   run. The approved revision is the transformed content: its hash is not
///   the request's, and it holds the boundary's replacement, not the marker.
/// - `POST /v1/contributors/me/submission-status` returns the pipeline block,
///   whose `trace_credit` leg is `"2500000"` and not settlement eligible.
/// - `POST /v1/pipeline/exports` and `.../complete`, with the export
///   credential, return one item whose content hash is the approved hash.
/// - A Medium-risk receipt parks in `awaiting_review`; the three review routes,
///   with the review credential, list, claim, and approve it, and the worker
///   completes the run.
/// - The pipeline withdrawal route, with the owner's account session, answers
///   `credit_retained` and a pending `index_invalidation`, and the worker then
///   removes the withdrawn revision's index entries.
/// - Tenant B's credentials: the status route has no document for tenant A's
///   submission (the route omits what the caller cannot see, as `main`'s
///   does, rather than answering 403 or 404), and completing tenant A's
///   export is 404.
/// - A service whose privacy boundary fails answers `POST /v1/traces` with a
///   label that carries no trace text, and creates no run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compatibility_bundle_through_http_with_review_privacy_withdrawal_and_export() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    // Migrates the suite's database and resets the account rate limiter.
    account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let tenant = format!("tenant-compat-http-{suffix}");
    let other_tenant = format!("tenant-compat-http-other-{suffix}");
    let failing_tenant = format!("tenant-compat-http-failing-{suffix}");
    let token = format!("token-compat-{suffix}");
    let reviewer_token = format!("token-compat-reviewer-{suffix}");
    let export_token = format!("token-compat-export-{suffix}");
    let other_token = format!("token-compat-other-{suffix}");
    let other_export_token = format!("token-compat-other-export-{suffix}");
    let failing_token = format!("token-compat-failing-{suffix}");
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, &tenant, &token, TokenRole::Contributor);
    insert_token(&mut tokens, &tenant, &reviewer_token, TokenRole::Reviewer);
    insert_token(&mut tokens, &tenant, &export_token, TokenRole::ExportWorker);
    insert_token(
        &mut tokens,
        &other_tenant,
        &other_token,
        TokenRole::Contributor,
    );
    insert_token(
        &mut tokens,
        &other_tenant,
        &other_export_token,
        TokenRole::ExportWorker,
    );
    insert_token(
        &mut tokens,
        &failing_tenant,
        &failing_token,
        TokenRole::Contributor,
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let artifacts = local_artifacts(&dir);
    let index = IsolatedPipelineIndex::new();
    let service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
        index.clone(),
        2_500_000,
        Arc::new(MarkerRedactingBoundary),
    );
    // The account side (sessions) runs on the migration owner with the
    // login-resolver pool, as `withdrawal_fixture` does; the pipeline service
    // and the product store run on the runtime login.
    let mut state = test_state_with_options(
        dir.path().to_path_buf(),
        Some(mains_database().await),
        Some(artifacts.clone()),
        false,
        false,
        false,
        false,
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.tokens = Arc::new(tokens);
    state_mut.pipeline_service = Some(service.clone());
    state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[tenant.as_str()],
    );
    let state_handle = state.clone();
    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();
    let tenant_ref = trace_commons_server::versioned_pipeline::pipeline_tenant_storage_ref(&tenant);

    // ---- A receipt whose content the privacy boundary transforms ----
    // Its consent allows model training, which `main`'s NoveltyUtility
    // credit requires (Ruling T15-8), and evaluation, the use the export
    // below selects.
    let mut envelope = sample_envelope().await;
    make_metadata_only_low_risk(&mut envelope);
    envelope.consent.scopes = vec![ConsentScope::ModelTraining];
    envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
    envelope.trace_card.allowed_uses =
        vec![TraceAllowedUse::Evaluation, TraceAllowedUse::ModelTraining];
    envelope
        .privacy
        .warnings
        .push("MARKER_SECRET in a free-text field".to_string());
    let body = serde_json::to_vec(&envelope).expect("envelope serialises");
    assert!(String::from_utf8_lossy(&body).contains("MARKER_SECRET"));
    let (status, receipt) = post_trace(&client, &base, &token, &body).await;
    assert_eq!(status, 200, "{receipt}");
    assert_eq!(receipt["status"], "processing", "{receipt}");

    wait_for_run_state(&runtime, &tenant, envelope.submission_id, "complete").await;
    let run = run_of_submission(&service, &runtime, &tenant, envelope.submission_id).await;
    assert_eq!(run.admission_decision, "admit");
    assert_eq!(run.index_write_state, "complete");
    let approved_hash = run
        .approved_content_hash
        .clone()
        .expect("Review approved a revision");
    assert_ne!(
        approved_hash, run.request_content_hash,
        "the approved revision is the transformed content, not the raw request"
    );
    let approved = String::from_utf8(service.load_approved_bytes(&run).await.unwrap()).unwrap();
    assert!(!approved.contains("MARKER_SECRET"), "{approved}");
    assert!(approved.contains("[redacted]"), "{approved}");
    let revision_entries = index.entry_count(&tenant_ref, MINIMAL_INDEX_ID);
    assert!(revision_entries > 0, "Settle wrote the revision's entries");

    let (status, documents) =
        post_submission_status(&client, &base, &token, &[envelope.submission_id]).await;
    assert_eq!(status, 200, "{documents}");
    let documents = documents.as_array().expect("a document list").clone();
    assert_eq!(documents.len(), 1, "{documents:?}");
    assert_eq!(
        documents[0]["submission_id"],
        envelope.submission_id.to_string()
    );
    assert_eq!(documents[0]["pipeline"]["run_id"], run.run_id.to_string());
    assert_eq!(documents[0]["pipeline"]["processing_state"], "complete");
    assert_eq!(
        documents[0]["pipeline"]["instruments"],
        serde_json::json!([{
            "instrument_id": "trace_credit",
            "atomic_units": "2500000",
            "operation_state": "complete",
            "internal_settlement_state": "not_settlement_eligible",
            "payout_rail": "none",
            "payout_state": "disabled",
            "reason_label": null,
        }])
    );
    let (status, other_documents) =
        post_submission_status(&client, &base, &other_token, &[envelope.submission_id]).await;
    assert_eq!(status, 200, "{other_documents}");
    assert_eq!(
        other_documents,
        serde_json::json!([]),
        "tenant B's credential gets no document for tenant A's submission"
    );

    // ---- Export of the approved revision ----
    let (status, created) = send_http(
        &client,
        reqwest::Method::POST,
        format!("{base}/v1/pipeline/exports"),
        bearer_with_idempotency_key(&export_token, "compatibility-export"),
        Some(serde_json::json!({
            "allowed_use": "evaluation",
            "purpose": "benchmark refresh",
            "limit": 10,
        })),
    )
    .await;
    assert_eq!(status, 200, "{created}");
    assert_eq!(created["state"], "ready", "{created}");
    let items = created["items"].as_array().expect("items").clone();
    assert_eq!(items.len(), 1, "{created}");
    assert_eq!(items[0]["run_id"], run.run_id.to_string());
    assert_eq!(items[0]["source_content_hash"], approved_hash.as_str());
    assert!(!created.to_string().contains(&tenant), "{created}");
    let snapshot_id = created["snapshot_id"]
        .as_str()
        .expect("a snapshot id")
        .to_string();
    let (status, refused) = send_http(
        &client,
        reqwest::Method::POST,
        format!("{base}/v1/pipeline/exports/{snapshot_id}/complete"),
        auth_headers(&other_export_token),
        None,
    )
    .await;
    assert_eq!(status, 404, "{refused}");
    assert_eq!(refused["error"], "export_snapshot_not_found");
    let (status, completed) = send_http(
        &client,
        reqwest::Method::POST,
        format!("{base}/v1/pipeline/exports/{snapshot_id}/complete"),
        auth_headers(&export_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{completed}");
    assert_eq!(completed["state"], "complete", "{completed}");
    assert_eq!(completed["items"], created["items"]);
    assert_eq!(
        completed["items"][0]["source_content_hash"],
        approved_hash.as_str()
    );

    // ---- A Medium-risk receipt waits for a human reviewer ----
    let mut medium = sample_envelope().await;
    make_metadata_only_low_risk(&mut medium);
    medium.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
    let (status, receipt) = post_trace(
        &client,
        &base,
        &token,
        &serde_json::to_vec(&medium).expect("envelope serialises"),
    )
    .await;
    assert_eq!(status, 200, "{receipt}");
    assert_eq!(receipt["status"], "processing", "{receipt}");
    wait_for_run_state(&runtime, &tenant, medium.submission_id, "awaiting_review").await;
    let medium_run = run_of_submission(&service, &runtime, &tenant, medium.submission_id).await;
    assert_eq!(medium_run.admission_decision, "quarantine");
    let (status, queue) = send_http(
        &client,
        reqwest::Method::GET,
        format!("{base}/v1/review/pipeline/quarantine"),
        auth_headers(&reviewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{queue}");
    let queue = queue.as_array().expect("a review queue").clone();
    assert_eq!(queue.len(), 1, "{queue:?}");
    assert_eq!(queue[0]["run_id"], medium_run.run_id.to_string());
    let reason = queue[0]["admission_reason"]
        .as_str()
        .expect("the quarantine reason")
        .to_string();
    let (status, refused) = send_http(
        &client,
        reqwest::Method::POST,
        format!("{base}/v1/review/pipeline/runs/{}/claim", medium_run.run_id),
        auth_headers(&token),
        None,
    )
    .await;
    assert_eq!(status, 403, "a contributor cannot claim: {refused}");
    let (status, claim) = send_http(
        &client,
        reqwest::Method::POST,
        format!("{base}/v1/review/pipeline/runs/{}/claim", medium_run.run_id),
        auth_headers(&reviewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200, "{claim}");
    let (status, assessment) = send_http(
        &client,
        reqwest::Method::POST,
        format!(
            "{base}/v1/review/pipeline/runs/{}/assessment",
            medium_run.run_id
        ),
        auth_headers(&reviewer_token),
        Some(serde_json::json!({
            "lease_token": claim["lease_token"],
            "recommendation": "approve",
            "reason": reason,
            "resolved_quarantine_reasons": [reason],
        })),
    )
    .await;
    assert_eq!(status, 200, "{assessment}");
    assert!(assessment["assessment_id"].is_string(), "{assessment}");
    wait_for_run_state(&runtime, &tenant, medium.submission_id, "complete").await;
    let reviewed = run_of_submission(&service, &runtime, &tenant, medium.submission_id).await;
    assert!(
        reviewed.approved_content_hash.is_some(),
        "the approved assessment let Review approve the revision"
    );
    let (status, queue) = send_http(
        &client,
        reqwest::Method::GET,
        format!("{base}/v1/review/pipeline/quarantine"),
        auth_headers(&reviewer_token),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(queue, serde_json::json!([]), "the queue is empty again");

    // ---- Withdrawal through the account session ----
    let entries_before_withdrawal = index.entry_count(&tenant_ref, MINIMAL_INDEX_ID);
    let session = account_session_headers(&state_handle, &token).await;
    let (status, withdrawal) = send_http(
        &client,
        reqwest::Method::POST,
        format!(
            "{base}/v1/contributors/me/pipeline-submissions/{}/withdraw",
            envelope.submission_id
        ),
        session,
        None,
    )
    .await;
    assert_eq!(status, 200, "{withdrawal}");
    assert_eq!(
        withdrawal["submission_id"],
        envelope.submission_id.to_string()
    );
    assert_eq!(
        withdrawal["credit_retained"], true,
        "a complete NoveltyUtility leg is not forfeited: {withdrawal}"
    );
    assert_eq!(withdrawal["index_invalidation"], "pending", "{withdrawal}");
    assert!(!withdrawal.to_string().contains(&tenant), "{withdrawal}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while queued_index_invalidation(&runtime, &tenant, run.run_id)
        .await
        .1
        != "complete"
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the worker never completed the withdrawn revision's index invalidation"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(
        index.entry_count(&tenant_ref, MINIMAL_INDEX_ID),
        entries_before_withdrawal - revision_entries,
        "the withdrawn revision's entries left the index"
    );

    stop.send(()).expect("send shutdown");
    join_within(server, 20, "compatibility HTTP test server").await;

    // ---- A privacy boundary that fails ----
    let failing_service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
        IsolatedPipelineIndex::new(),
        2_500_000,
        Arc::new(FailingPrivacyBoundary),
    );
    let mut failing_state = state_handle;
    let failing_mut = Arc::make_mut(&mut failing_state);
    failing_mut.pipeline_service = Some(failing_service);
    failing_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[failing_tenant.as_str()],
    );
    let (base, stop, server) = serve_pipeline_app(failing_state).await;
    let mut failing = sample_envelope_with_user_input("Summarise the failing-boundary notes").await;
    failing.privacy.residual_pii_risk = ResidualPiiRisk::Low;
    let (status, refused) = post_trace(
        &client,
        &base,
        &failing_token,
        &serde_json::to_vec(&failing).expect("envelope serialises"),
    )
    .await;
    assert_eq!(status, 500, "{refused}");
    assert_eq!(
        refused,
        serde_json::json!({"error": "trace commons operation failed"}),
        "the refusal is a label"
    );
    assert!(
        !refused.to_string().contains("failing-boundary"),
        "{refused}"
    );
    let mut client_pg = runtime.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client_pg, &failing_tenant).await;
    let runs: i64 = tx
        .query_one(
            "SELECT COUNT(*) FROM pipeline_runs WHERE tenant_id = $1",
            &[&failing_tenant],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    assert_eq!(runs, 0, "a failed rescrub creates no run");
    stop.send(()).expect("send shutdown");
    join_within(server, 20, "failing boundary test server").await;
}

/// The legacy `NoveltyUtility` points delta the parity test configures
/// (`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA`).
const LEGACY_NOVELTY_UTILITY_CREDIT_POINTS_DELTA: f32 = 2.5;

/// The gate policy version of the parity test's in-memory gate, which
/// `main`'s `NoveltyUtility` reason carries.
const PARITY_GATE_POLICY_VERSION: &str = "parity_gate_v1";

/// Collects every legacy-versus-pipeline difference, so one run reports all
/// of them rather than the first.
#[derive(Default)]
struct ParityReport {
    mismatches: Vec<String>,
}

impl ParityReport {
    /// The two paths must agree.
    fn compare(&mut self, what: &str, legacy: serde_json::Value, pipeline: serde_json::Value) {
        if legacy != pipeline {
            self.mismatches
                .push(format!("{what}: legacy {legacy}, pipeline {pipeline}"));
        }
    }

    /// The two paths differ by design (`ruling`): each side must hold its
    /// own pinned value.
    fn pin(
        &mut self,
        what: &str,
        ruling: &str,
        (legacy, legacy_expected): (serde_json::Value, serde_json::Value),
        (pipeline, pipeline_expected): (serde_json::Value, serde_json::Value),
    ) {
        if legacy != legacy_expected || pipeline != pipeline_expected {
            self.mismatches.push(format!(
                "{what} ({ruling}): legacy {legacy} (pinned {legacy_expected}), \
                 pipeline {pipeline} (pinned {pipeline_expected})"
            ));
        }
    }
}

/// The sorted top-level keys of a JSON object.
fn json_keys(value: &serde_json::Value) -> serde_json::Value {
    let mut keys = value
        .as_object()
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    serde_json::json!(keys)
}

/// `main`'s `NoveltyUtility` reason shape, `novelty_utility:<version>`, with
/// the version replaced: each path names its own version (Ruling T15-2).
fn novelty_utility_reason_shape(text: &str) -> String {
    match text.split_once("novelty_utility:") {
        Some((before, after)) => {
            let rest = after
                .find(')')
                .map(|close| &after[close..])
                .unwrap_or_default();
            format!("{before}novelty_utility:<version>{rest}")
        }
        None => text.to_string(),
    }
}

/// Every `trace_credit_ledger` row of `submission_id`, read as the migration
/// owner: event type, amount in microcredits, settlement state, actor role,
/// and the reason with its version replaced.
async fn ledger_rows(
    owner: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: Uuid,
) -> serde_json::Value {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let rows = tx
        .query(
            "SELECT event_type, points_delta, settlement_state, actor_role, reason
               FROM trace_credit_ledger
              WHERE tenant_id = $1 AND submission_id = $2
              ORDER BY occurred_at",
            &[&tenant_id, &submission_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    serde_json::json!(
        rows.iter()
            .map(|row| {
                let points: String = row.get(1);
                serde_json::json!({
                    "event_type": row.get::<_, String>(0),
                    "microcredits": Microcredits::from_credit_decimal(&points)
                        .expect("points_delta is a credit decimal")
                        .get(),
                    "settlement_state": row.get::<_, String>(2),
                    "actor_role": row.get::<_, String>(3),
                    "reason": novelty_utility_reason_shape(&row.get::<_, String>(4)),
                })
            })
            .collect::<Vec<_>>()
    )
}

/// The raw `reason` of every `trace_credit_ledger` row of `submission_id`.
async fn ledger_reasons(
    owner: &Arc<PgBackend>,
    tenant_id: &str,
    submission_id: Uuid,
) -> serde_json::Value {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let reasons = tx
        .query(
            "SELECT reason FROM trace_credit_ledger
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant_id, &submission_id],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get::<_, String>(0))
        .collect::<Vec<_>>();
    tx.commit().await.unwrap();
    serde_json::json!(reasons)
}

/// The fields of a status document the two paths must share outright.
fn status_fields(document: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "status": document["status"],
        "credit_points_final": document["credit_points_final"],
        "credit_points_ledger": document["credit_points_ledger"],
        "credit_points_total": document["credit_points_total"],
        "consent_scopes": document["consent_scopes"],
        "delayed_credit_explanations": document["delayed_credit_explanations"]
            .as_array()
            .map(|lines| {
                lines
                    .iter()
                    .map(|line| novelty_utility_reason_shape(line.as_str().unwrap_or_default()))
                    .collect::<Vec<_>>()
            }),
    })
}

/// How a status document presents its credit figure, against the gate
/// decision it should come from: the legacy path's own gate decision, or the
/// pipeline's Score evidence (Ruling T15-7). The figures themselves differ
/// (two scorers read two traces); `main`'s rule for turning a decision into
/// a figure and an explanation must be the same on both paths.
fn credit_presentation(
    document: &serde_json::Value,
    decision: Option<&StorageTraceGateCreditDecisionRow>,
) -> serde_json::Value {
    let Some(decision) = decision else {
        return serde_json::json!("no gate decision");
    };
    let quality = decision
        .credit_quality_micros
        .expect("the decision has a credit quality");
    let pending = document["credit_points_pending"].as_f64().unwrap_or(-1.0);
    let basis = gate_credit_basis_line(decision);
    let explanation = document["explanation"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|line| {
            let line = line.as_str().unwrap_or_default();
            if line == basis {
                "<the decision's basis line>".to_string()
            } else if line.starts_with("Attributed to tenant ") {
                "Attributed to tenant <tenant>".to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>();
    serde_json::json!({
        "pending_is_the_decision_figure":
            (pending - f64::from(credit_points_from_quality_micros(quality))).abs() < 1e-4,
        "explanation": explanation,
    })
}

/// Brief section 3C acceptance: receipt, status, withdrawal, and credit and
/// dedup behaviour match `main` under equivalent configuration. Tenant L
/// takes `main`'s path; tenant P is routed to a pipeline whose compatibility
/// bundle pins `novelty_utility_microcredits` equal to L's
/// `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` in microcredits. Both
/// share one `AppState` with the pilot's database reads
/// (`deploy/pilot-gcp/ingest.env.template`: contributor and reviewer reads
/// from PostgreSQL), and both submit equivalent envelopes, a pair allowing
/// model training and a pair that does not (the default consent scope).
///
/// L's credit comes from `main`'s own gate path: `POST
/// /v1/workers/gate/evaluate` with an in-memory gate service whose floors
/// pass, which runs `main`'s `NoveltyUtility` checks and append
/// (`attempt_emit_novelty_utility_credit`). P's comes from the live worker.
///
/// Differences by design are pinned per side with their ruling (T15-2,
/// T15-3); everything else must be equal. The status document is also read
/// with contributor reads from files (Ruling T15-7).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn legacy_and_pipeline_tenants_match_under_equivalent_configuration() {
    let Some(runtime) = runtime_backend(4).await else {
        return;
    };
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let suffix = Uuid::new_v4().simple().to_string();
    let legacy_tenant = format!("tenant-parity-legacy-{suffix}");
    let pipeline_tenant = format!("tenant-parity-pipeline-{suffix}");
    let legacy_token = format!("token-parity-legacy-{suffix}");
    let legacy_gate_token = format!("token-parity-legacy-gate-{suffix}");
    let pipeline_token = format!("token-parity-pipeline-{suffix}");
    let mut tokens = BTreeMap::new();
    insert_token(
        &mut tokens,
        &legacy_tenant,
        &legacy_token,
        TokenRole::Contributor,
    );
    insert_token(
        &mut tokens,
        &legacy_tenant,
        &legacy_gate_token,
        TokenRole::VectorWorker,
    );
    insert_token(
        &mut tokens,
        &pipeline_tenant,
        &pipeline_token,
        TokenRole::Contributor,
    );

    // The gate worker reads only KEK-wrapped (v2) envelopes, so both paths
    // store into the service-owned store the gate-worker tests use.
    let dir = tempfile::tempdir().expect("temp dir");
    let artifact_dir = tempfile::tempdir().expect("artifact dir");
    let (configured_store, _) = fixture_gate_worker_artifact_store(artifact_dir.path());
    let mut state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
        dir.path().to_path_buf(),
        Some(mains_database().await),
        Some(configured_store),
        true,
        true,
        false,
        false,
        false,
        false,
        BTreeMap::new(),
        false,
        false,
    );
    let pipeline_delta_microcredits =
        (f64::from(LEGACY_NOVELTY_UTILITY_CREDIT_POINTS_DELTA) * 1_000_000.0).round() as u64;
    let service = assemble_compatibility_pipeline_service(
        runtime.clone(),
        state.artifact_store.as_ref().expect("a configured store"),
        IsolatedPipelineIndex::new(),
        pipeline_delta_microcredits,
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.tokens = Arc::new(tokens);
    state_mut.gate_service = Arc::new(InMemoryGateService::new(
        PARITY_GATE_POLICY_VERSION,
        "sha256:parity_gate_v1",
    ));
    state_mut.novelty_utility_credit_points_delta = LEGACY_NOVELTY_UTILITY_CREDIT_POINTS_DELTA;
    state_mut.pipeline_service = Some(service.clone());
    state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[pipeline_tenant.as_str()],
    );
    let state_handle = state.clone();
    // The same state with contributor and reviewer reads from files, for the
    // file-read status (Ruling T15-7). `make_mut` copies the state, which
    // keeps the same database, store, and pipeline service.
    let mut file_reads = state.clone();
    let file_reads_mut = Arc::make_mut(&mut file_reads);
    file_reads_mut.db_contributor_reads = false;
    file_reads_mut.db_reviewer_reads = false;
    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();
    let mut parity = ParityReport::default();

    // Each case submits its own content. The first case withdraws its legacy
    // submission, and `main`'s account withdrawal records the withdrawn
    // content on the file side (#1112), so the same content again is
    // refused as previously revoked. The tool name is what tells the two
    // metadata-only envelopes apart; both sides of a case share it.
    let envelope = |model_training: bool| async move {
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        set_metadata_only_tool_name(
            &mut envelope,
            if model_training {
                "parity_model_training_case"
            } else {
                "parity_default_consent_case"
            },
        );
        if model_training {
            envelope.consent.scopes = vec![ConsentScope::ModelTraining];
            envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
            envelope.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
        }
        envelope
    };

    for model_training in [true, false] {
        let case = if model_training {
            "model training allowed"
        } else {
            "model training not allowed"
        };
        let legacy_envelope = envelope(model_training).await;
        let pipeline_envelope = envelope(model_training).await;
        let legacy_body = serde_json::to_vec(&legacy_envelope).unwrap();
        let pipeline_body = serde_json::to_vec(&pipeline_envelope).unwrap();

        // ---- Receipt ----
        let (legacy_code, legacy_receipt) =
            post_trace(&client, &base, &legacy_token, &legacy_body).await;
        // The legacy side is `main`'s path, the reference every comparison
        // below reads: it must succeed first, or two failures compare equal.
        assert_eq!(
            legacy_code, 200,
            "{case}: main's receipt succeeds: {legacy_receipt}"
        );
        let (pipeline_code, pipeline_receipt) =
            post_trace(&client, &base, &pipeline_token, &pipeline_body).await;
        parity.compare(
            &format!("{case}: receipt code"),
            serde_json::json!(legacy_code.as_u16()),
            serde_json::json!(pipeline_code.as_u16()),
        );
        // Ruling T15-3: the pipeline's receipt is asynchronous (PR 2's
        // `pipeline_processing_receipt`): `processing`, and no credit figure
        // until a run has one.
        parity.pin(
            &format!("{case}: receipt status and keys"),
            "T15-3",
            (
                serde_json::json!([legacy_receipt["status"], json_keys(&legacy_receipt)]),
                serde_json::json!([
                    "accepted",
                    ["credit_points_pending", "explanation", "status"]
                ]),
            ),
            (
                serde_json::json!([pipeline_receipt["status"], json_keys(&pipeline_receipt)]),
                serde_json::json!(["processing", ["explanation", "status"]]),
            ),
        );

        // ---- Processing: main's gate path, and the pipeline's worker ----
        let (gate_code, gate) = send_http(
            &client,
            reqwest::Method::POST,
            format!("{base}/v1/workers/gate/evaluate"),
            auth_headers(&legacy_gate_token),
            Some(serde_json::json!({ "submission_id": legacy_envelope.submission_id })),
        )
        .await;
        assert_eq!(gate_code, 200, "main's gate path runs: {gate}");
        assert_eq!(gate["perplexity_passed"], true, "{gate}");
        assert_eq!(gate["novelty_passed"], true, "{gate}");
        wait_for_run_state(
            &runtime,
            &pipeline_tenant,
            pipeline_envelope.submission_id,
            "complete",
        )
        .await;
        let pipeline_run = run_of_submission(
            &service,
            &runtime,
            &pipeline_tenant,
            pipeline_envelope.submission_id,
        )
        .await;
        let legacy_decision = owner
            .list_latest_gate_credit_decisions(&legacy_tenant, &[legacy_envelope.submission_id])
            .await
            .unwrap()
            .pop();
        let pipeline_decision =
            score_shadow_credit_decision(&runtime, &pipeline_tenant, &pipeline_run).await;

        // ---- Credit: the ledger event each path appends ----
        let legacy_ledger =
            ledger_rows(&owner, &legacy_tenant, legacy_envelope.submission_id).await;
        let pipeline_ledger =
            ledger_rows(&owner, &pipeline_tenant, pipeline_envelope.submission_id).await;
        parity.compare(
            &format!("{case}: ledger events"),
            legacy_ledger.clone(),
            pipeline_ledger.clone(),
        );
        // Ruling T15-2: `main`'s reason names its gate policy version; the
        // pipeline's names the compatibility Score rule.
        let (legacy_reasons, pipeline_reasons) = if model_training {
            (
                serde_json::json!([format!("novelty_utility:{PARITY_GATE_POLICY_VERSION}")]),
                serde_json::json!([format!("novelty_utility:{COMPATIBILITY_SCORE_RULE}")]),
            )
        } else {
            (serde_json::json!([]), serde_json::json!([]))
        };
        parity.pin(
            &format!("{case}: ledger reasons"),
            "T15-2",
            (
                ledger_reasons(&owner, &legacy_tenant, legacy_envelope.submission_id).await,
                legacy_reasons,
            ),
            (
                ledger_reasons(&owner, &pipeline_tenant, pipeline_envelope.submission_id).await,
                pipeline_reasons,
            ),
        );

        // ---- Status after processing: database reads, then file reads ----
        for (reads, legacy_state, pipeline_state) in [
            ("database reads", None, None),
            (
                "file reads",
                Some(file_reads.clone()),
                Some(file_reads.clone()),
            ),
        ] {
            let status_of = |state: Option<Arc<AppState>>, token: String, id: Uuid| {
                let client = client.clone();
                let base = base.clone();
                async move {
                    match state {
                        None => post_submission_status(&client, &base, &token, &[id]).await,
                        Some(state) => {
                            let (code, body) = route_request(
                                state,
                                "POST",
                                "/v1/contributors/me/submission-status",
                                auth_headers(&token),
                                Some(serde_json::json!({ "submission_ids": [id] })),
                            )
                            .await;
                            (reqwest::StatusCode::from_u16(code.as_u16()).unwrap(), body)
                        }
                    }
                }
            };
            let (legacy_code, legacy_documents) = status_of(
                legacy_state,
                legacy_token.clone(),
                legacy_envelope.submission_id,
            )
            .await;
            assert_eq!(
                legacy_code, 200,
                "{case}, {reads}: main's status read succeeds: {legacy_documents}"
            );
            assert_eq!(
                legacy_documents.as_array().map(Vec::len),
                Some(1),
                "{case}, {reads}: main's status read has the submission's document: \
                 {legacy_documents}"
            );
            let (pipeline_code, pipeline_documents) = status_of(
                pipeline_state,
                pipeline_token.clone(),
                pipeline_envelope.submission_id,
            )
            .await;
            parity.compare(
                &format!("{case}, {reads}: status route code"),
                serde_json::json!(legacy_code.as_u16()),
                serde_json::json!(pipeline_code.as_u16()),
            );
            parity.compare(
                &format!("{case}, {reads}: status document"),
                status_fields(&legacy_documents[0]),
                status_fields(&pipeline_documents[0]),
            );
            parity.compare(
                &format!("{case}, {reads}: credit figure and explanation"),
                credit_presentation(&legacy_documents[0], legacy_decision.as_ref()),
                credit_presentation(&pipeline_documents[0], Some(&pipeline_decision)),
            );
        }

        // ---- Credit summary (database reads) ----
        let credit_of = |token: String| {
            let client = client.clone();
            let base = base.clone();
            async move {
                let (code, body) = send_http(
                    &client,
                    reqwest::Method::GET,
                    format!("{base}/v1/contributors/me/credit"),
                    auth_headers(&token),
                    None,
                )
                .await;
                serde_json::json!({
                    "code": code.as_u16(),
                    "accepted": body["accepted"],
                    "credit_points_ledger": body["credit_points_ledger"],
                    "credit_points_pending_ledger": body["credit_points_pending_ledger"],
                    "credit_points_total": body["credit_points_total"],
                    "credit_points_settled": body["credit_points_settled"],
                })
            }
        };
        parity.compare(
            &format!("{case}: credit summary"),
            credit_of(legacy_token.clone()).await,
            credit_of(pipeline_token.clone()).await,
        );

        // ---- Another tenant's credential (Ruling T15-4) ----
        // `main`'s status route omits what the caller cannot see: 200 and an
        // empty list, never 403 or 404.
        let (legacy_code, legacy_other) = post_submission_status(
            &client,
            &base,
            &pipeline_token,
            &[legacy_envelope.submission_id],
        )
        .await;
        let (pipeline_code, pipeline_other) = post_submission_status(
            &client,
            &base,
            &legacy_token,
            &[pipeline_envelope.submission_id],
        )
        .await;
        parity.pin(
            &format!("{case}: another tenant's status read"),
            "T15-4",
            (
                serde_json::json!([legacy_code.as_u16(), legacy_other]),
                serde_json::json!([200, []]),
            ),
            (
                serde_json::json!([pipeline_code.as_u16(), pipeline_other]),
                serde_json::json!([200, []]),
            ),
        );

        // ---- Dedup: the same bytes again, and changed bytes under the id ----
        let (legacy_code, legacy_retry) =
            post_trace(&client, &base, &legacy_token, &legacy_body).await;
        let (pipeline_code, pipeline_retry) =
            post_trace(&client, &base, &pipeline_token, &pipeline_body).await;
        parity.compare(
            &format!("{case}: retry code"),
            serde_json::json!(legacy_code.as_u16()),
            serde_json::json!(pipeline_code.as_u16()),
        );
        parity.pin(
            &format!("{case}: retry status"),
            "T15-3",
            (
                legacy_retry["status"].clone(),
                serde_json::json!("accepted"),
            ),
            (
                pipeline_retry["status"].clone(),
                serde_json::json!("processing"),
            ),
        );
        parity.compare(
            &format!("{case}: ledger events after a retry"),
            ledger_rows(&owner, &legacy_tenant, legacy_envelope.submission_id).await,
            ledger_rows(&owner, &pipeline_tenant, pipeline_envelope.submission_id).await,
        );
        parity.compare(
            &format!("{case}: a retry adds no ledger event"),
            serde_json::json!([
                legacy_ledger,
                ledger_rows(&owner, &legacy_tenant, legacy_envelope.submission_id).await
            ]),
            serde_json::json!([
                pipeline_ledger,
                ledger_rows(&owner, &pipeline_tenant, pipeline_envelope.submission_id).await
            ]),
        );
        let changed = |mut envelope: TraceContributionEnvelope| {
            envelope
                .privacy
                .warnings
                .push("changed content".to_string());
            serde_json::to_vec(&envelope).unwrap()
        };
        let (legacy_code, legacy_changed) = post_trace(
            &client,
            &base,
            &legacy_token,
            &changed(legacy_envelope.clone()),
        )
        .await;
        let (pipeline_code, pipeline_changed) = post_trace(
            &client,
            &base,
            &pipeline_token,
            &changed(pipeline_envelope.clone()),
        )
        .await;
        // Ruling T15-3: spec section 3D -- the pipeline's replay identity is
        // the request's bytes, so changed content under the same id is a
        // conflict; `main` returns the existing receipt.
        parity.pin(
            &format!("{case}: changed content under the same id"),
            "T15-3",
            (
                serde_json::json!([legacy_code.as_u16(), legacy_changed["error"]]),
                serde_json::json!([200, null]),
            ),
            (
                serde_json::json!([pipeline_code.as_u16(), pipeline_changed["error"]]),
                serde_json::json!([409, "receipt id reused with different content"]),
            ),
        );

        // ---- Withdrawal through the account session ----
        let legacy_session = account_session_headers(&state_handle, &legacy_token).await;
        let pipeline_session = account_session_headers(&state_handle, &pipeline_token).await;
        let (legacy_code, legacy_withdrawal) = send_http(
            &client,
            reqwest::Method::POST,
            format!(
                "{base}/v1/account/traces/{}/withdraw",
                legacy_envelope.submission_id
            ),
            legacy_session,
            None,
        )
        .await;
        let (pipeline_code, pipeline_withdrawal) = send_http(
            &client,
            reqwest::Method::POST,
            format!(
                "{base}/v1/contributors/me/pipeline-submissions/{}/withdraw",
                pipeline_envelope.submission_id
            ),
            pipeline_session,
            None,
        )
        .await;
        assert_eq!(
            legacy_code, 200,
            "{case}: main's withdrawal succeeds: {legacy_withdrawal}"
        );
        parity.compare(
            &format!("{case}: withdrawal code"),
            serde_json::json!(legacy_code.as_u16()),
            serde_json::json!(pipeline_code.as_u16()),
        );
        for field in [
            "credit_retained",
            "prior_status",
            "distribution_reach",
            "already_distributed",
            "token_deletion_state",
        ] {
            parity.compare(
                &format!("{case}: withdrawal {field}"),
                legacy_withdrawal[field].clone(),
                pipeline_withdrawal[field].clone(),
            );
        }
        let (_, legacy_after) = post_submission_status(
            &client,
            &base,
            &legacy_token,
            &[legacy_envelope.submission_id],
        )
        .await;
        let (_, pipeline_after) = post_submission_status(
            &client,
            &base,
            &pipeline_token,
            &[pipeline_envelope.submission_id],
        )
        .await;
        parity.compare(
            &format!("{case}: status after the withdrawal"),
            serde_json::json!([
                status_fields(&legacy_after[0]),
                legacy_after[0]["credit_points_pending"]
            ]),
            serde_json::json!([
                status_fields(&pipeline_after[0]),
                pipeline_after[0]["credit_points_pending"]
            ]),
        );
    }

    stop.send(()).expect("send shutdown");
    join_within(server, 20, "parity test server").await;
    assert!(
        parity.mismatches.is_empty(),
        "the legacy and pipeline paths differ:\n{}",
        parity.mismatches.join("\n")
    );
}

/// `(pending invalidations of the run, the run's index_invalidation_state,
/// pending payload deletions the pipeline queued for the submission, live
/// pipeline export snapshot items of the submission)`, for the revocation
/// tests below.
async fn pipeline_revocation_follow_up(
    backend: &Arc<PgBackend>,
    tenant_id: &str,
    run: &trace_commons_server::versioned_pipeline::PipelineRunRecord,
) -> (i64, String, i64) {
    let mut client = backend.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let row = tx
        .query_one(
            "SELECT (SELECT COUNT(*) FROM pipeline_index_invalidations i
                      WHERE i.tenant_id = r.tenant_id AND i.run_id = r.run_id
                        AND i.state = 'pending' AND i.reason_code = 'revoked'),
                    r.index_invalidation_state,
                    (SELECT COUNT(*) FROM trace_revocation_propagation_items p
                      WHERE p.tenant_id = r.tenant_id AND p.source_submission_id = r.submission_id
                        AND p.action = 'delete_object_payload' AND p.status = 'pending'
                        AND p.reason = 'pipeline_withdrawal')
               FROM pipeline_runs r WHERE r.tenant_id = $1 AND r.run_id = $2",
            &[&tenant_id, &run.run_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    (row.get(0), row.get(1), row.get(2))
}

/// Zaki review 1, round 2, finding 5: `main`'s revocation routes -- `DELETE
/// /v1/traces/{id}`, `POST /v1/traces/{id}/revoke` and `DELETE /v1/traces`
/// -- run the pipeline's follow-up for a submission with a pipeline run, as
/// the pipeline withdrawal does: the complete run's revision is queued for
/// removal from the pipeline index (reason `revoked`), a payload deletion is
/// queued for each of its live objects, and the run's worker wakes to it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mains_revocation_routes_queue_the_pipeline_follow_up() {
    let Some(fixture) = withdrawal_fixture().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let principal = static_token_principal_ref(&fixture.token);
    for route in [
        "DELETE /v1/traces/{id}",
        "POST /v1/traces/{id}/revoke",
        "DELETE /v1/traces",
    ] {
        let run = completed_pipeline_run(&fixture.service, &tenant, &principal).await;
        let _ = fixture.service.take_follow_ups(&tenant);
        let id = run.submission_id.to_string();
        let (method, uri, body) = match route {
            "DELETE /v1/traces/{id}" => ("DELETE", format!("/v1/traces/{id}"), None),
            "POST /v1/traces/{id}/revoke" => ("POST", format!("/v1/traces/{id}/revoke"), None),
            _ => (
                "DELETE",
                "/v1/traces".to_string(),
                Some(serde_json::json!({ "submission_id": id })),
            ),
        };
        let (status, response) = route_request(
            fixture.state.clone(),
            method,
            &uri,
            auth_headers(&fixture.token),
            body,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{route}: {response}");
        let (invalidations, run_state, deletions) =
            pipeline_revocation_follow_up(&fixture.runtime, &tenant, &run).await;
        assert_eq!(
            (invalidations, run_state.as_str()),
            (1, "pending"),
            "{route}: the revision is queued for removal"
        );
        assert!(
            deletions >= 2,
            "{route}: a payload deletion per live object ({deletions})"
        );
        assert!(
            fixture.service.take_follow_ups(&tenant).index_invalidations,
            "{route}: the worker's invalidation step is woken"
        );
    }
}

/// `main`'s #1155 completes a source-session withdrawal from actual state:
/// an account merge can join a live version to a session the other account
/// had withdrawn, and the merge confirm (scoped to the survivor) and the
/// revocation-propagation worker's reconciler (the whole tenant) both run
/// `reconcile_source_session_withdrawals` over every such version. For a
/// version with a pipeline run, that completion also makes the pipeline's
/// follow-up -- its revision queued for removal from the pipeline index
/// (reason `withdrawn`), its payload deletions, its runs' work ended -- and
/// the version then drops off the incomplete list, though the pipeline
/// keeps part of its cleanup in its own tables. Once as the merge's scoped
/// call, once as the worker's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_withdrawal_completion_runs_the_pipeline_follow_up_and_stops_listing_the_version() {
    for scoped_to_the_survivor in [true, false] {
        let Some(fixture) = withdrawal_fixture().await else {
            return;
        };
        let state = &fixture.state;
        let tenant = fixture.tenant.as_str();
        let principal = static_token_principal_ref(&fixture.token);
        let session = account_session_headers(state, &fixture.token).await;
        let run = completed_pipeline_run(&fixture.service, tenant, &principal).await;
        let ext = account_ctx_ext(state, &session).await;
        let account_id = ext.0.account_id.as_uuid();
        let digest: [u8; 32] = Sha256::digest(tenant.as_bytes())
            .as_slice()
            .try_into()
            .unwrap();
        assert_eq!(
            fixture
                .owner
                .claim_trace_source_session(tenant, account_id, &digest, run.submission_id)
                .await
                .unwrap(),
            StorageTraceSourceSessionStatus::Active
        );
        // The session is withdrawn while this version is not: what a merge
        // leaves when the other account had withdrawn the session.
        {
            let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
            let tx = tenant_tx(&mut client, tenant).await;
            tx.execute(
                "UPDATE trace_source_sessions SET withdrawn_at = NOW()
                  WHERE tenant_id = $1 AND account_id = $2",
                &[&tenant, &account_id],
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        let db = state.db_mirror.clone().expect("main's database");
        let incomplete = || {
            let db = db.clone();
            async move {
                db.list_incomplete_source_session_withdrawals(tenant, None, &[], 100)
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|version| version.submission_id)
                    .collect::<Vec<_>>()
            }
        };
        assert!(incomplete().await.contains(&run.submission_id));
        let _ = fixture.service.take_follow_ups(tenant);

        let summary = reconcile_source_session_withdrawals(
            state.as_ref(),
            &db,
            tenant,
            scoped_to_the_survivor.then_some(account_id),
            &account_audit_tenant(&ext.0),
            100,
            false,
        )
        .await
        .expect("the completion runs");
        assert_eq!(
            (summary.completed, summary.failed),
            (1, 0),
            "scoped: {scoped_to_the_survivor}"
        );
        assert_eq!(
            queued_index_invalidation(&fixture.runtime, tenant, run.run_id).await,
            (1, "pending".to_string()),
            "the pipeline invalidation is queued (scoped: {scoped_to_the_survivor})"
        );
        assert!(fixture.service.take_follow_ups(tenant).index_invalidations);
        assert!(
            !incomplete().await.contains(&run.submission_id),
            "the completed pipeline version is no longer listed (scoped: {scoped_to_the_survivor})"
        );
    }
}

/// The pipeline HTTP harness's own setup and nothing else: the
/// `<database>_pilot` database created and migrated the way the pilot's was,
/// the runtime login provisioned, and the owner and runtime connections
/// opened, as every test here opens them. CI runs it alone against a
/// database of its own, and the transactions it commits there are the
/// baseline its no-activity guard holds each real pipeline HTTP step above:
/// the harness setup by itself commits more than a fixed floor would allow
/// for, so only commits beyond the baseline show that a step ran its test
/// (Zaki review 1, round 2, finding 22).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pipeline_http_harness_sets_up_its_database() {
    let Some(runtime) = runtime_backend(1).await else {
        return;
    };
    account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let _ = mains_database().await;
    drop(runtime);
}

// ---------------------------------------------------------------------------
// `main`'s side paths and submissions with a pipeline run (Zaki review 1,
// round 2, finding 18).
// ---------------------------------------------------------------------------

/// Moves `submission_id`'s expiry date two days into the past, as time
/// would.
async fn age_submission_past_expiry(owner: &Arc<PgBackend>, tenant_id: &str, submission_id: Uuid) {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let updated = tx
        .execute(
            "UPDATE trace_submissions SET expires_at = NOW() - INTERVAL '2 days'
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant_id, &submission_id],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(updated, 1);
}

/// The pipeline follow-up queued for a retention change of `run`'s
/// submission: `(index invalidations of the run under reason_code,
/// delete_object_payload items for the submission under reason, the
/// submission's object refs not yet invalidated)`.
async fn retention_follow_up(
    owner: &Arc<PgBackend>,
    tenant_id: &str,
    run: &trace_commons_server::versioned_pipeline::PipelineRunRecord,
    reason_code: &str,
    reason: &str,
) -> (i64, i64, i64) {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    let row = tx
        .query_one(
            "SELECT (SELECT COUNT(*) FROM pipeline_index_invalidations
                      WHERE tenant_id = $1 AND run_id = $2 AND reason_code = $4),
                    (SELECT COUNT(*) FROM trace_revocation_propagation_items
                      WHERE tenant_id = $1 AND source_submission_id = $3
                        AND action = 'delete_object_payload' AND reason = $5),
                    (SELECT COUNT(*) FROM trace_object_refs
                      WHERE tenant_id = $1 AND submission_id = $3 AND invalidated_at IS NULL)",
            &[
                &tenant_id,
                &run.run_id,
                &run.submission_id,
                &reason_code,
                &reason,
            ],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    (row.get(0), row.get(1), row.get(2))
}

/// Ruling RB-30 (Zaki review 1, round 2, finding 18): `main`'s retention
/// maintenance expires and purges a submission with a pipeline run, which
/// is in the database only, by its own rules and request, with no pipeline
/// runtime injected. A legal hold on its retention policy keeps it; a dry
/// run counts the expiry and changes nothing; the expiry marks the row
/// `expired` through `main`'s mirror (which invalidates its object refs, as
/// for a legacy row) and queues the index invalidation of the run's
/// revision, deleting no payload; the purge, under the request's
/// cutoff, marks it `purged` and queues one payload deletion per live
/// object ref for `main`'s revocation-propagation worker.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mains_retention_maintenance_expires_and_purges_a_pipeline_submission() {
    let Some(fixture) = withdrawal_fixture().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    let admin_token = format!("{}-admin", fixture.token);
    let principal = static_token_principal_ref(&fixture.token);
    let run = completed_pipeline_run(&fixture.service, tenant, &principal).await;
    let mut state = fixture.state.clone();
    {
        let state = Arc::make_mut(&mut state);
        let mut tokens = (*state.tokens).clone();
        insert_token(&mut tokens, tenant, &admin_token, TokenRole::Admin);
        state.tokens = Arc::new(tokens);
        state.pipeline_service = None;
        state.pipeline_store = Some(Arc::new(PgPipelineStore::new(fixture.runtime.clone())));
    }
    age_submission_past_expiry(&fixture.owner, tenant, run.submission_id).await;
    let submission = || async {
        fixture
            .owner
            .get_trace_submission(tenant, run.submission_id)
            .await
            .unwrap()
            .expect("the submission row")
    };
    let live_object_refs = retention_follow_up(&fixture.owner, tenant, &run, "-", "-")
        .await
        .2;
    assert!(live_object_refs > 0, "the run stored objects");
    let maintain = |state: Arc<AppState>, dry_run: bool, purge: bool| {
        let admin_token = admin_token.clone();
        async move {
            let Json(response) = maintenance_handler(
                State(state),
                auth_headers(&admin_token),
                Json(TraceMaintenanceRequest {
                    purpose: Some("pipeline retention test".to_string()),
                    dry_run,
                    prune_export_cache: false,
                    purge_expired_before: purge.then(Utc::now),
                    ..test_maintenance_request()
                }),
            )
            .await
            .expect("the maintenance runs");
            (
                response.records_marked_expired,
                response.records_marked_purged,
            )
        }
    };

    let mut held = state.clone();
    Arc::make_mut(&mut held).legal_hold_retention_policy_ids =
        Arc::new(BTreeSet::from([submission().await.retention_policy_id]));
    assert_eq!(
        maintain(held, false, true).await,
        (0, 0),
        "a legal hold keeps it"
    );
    assert_eq!(
        submission().await.status,
        StorageTraceCorpusStatus::Accepted
    );

    assert_eq!(
        maintain(state.clone(), true, true).await,
        (1, 0),
        "a dry run counts"
    );
    assert_eq!(
        submission().await.status,
        StorageTraceCorpusStatus::Accepted
    );
    assert_eq!(
        retention_follow_up(
            &fixture.owner,
            tenant,
            &run,
            "retention_expired",
            "pipeline_retention_purge"
        )
        .await,
        (0, 0, live_object_refs),
        "a dry run queues nothing"
    );

    assert_eq!(maintain(state.clone(), false, false).await, (1, 0));
    assert_eq!(submission().await.status, StorageTraceCorpusStatus::Expired);
    assert_eq!(
        retention_follow_up(
            &fixture.owner,
            tenant,
            &run,
            "retention_expired",
            "pipeline_retention_purge"
        )
        .await,
        (1, 0, 0),
        "the expiry queues the revision's invalidation, and main's expiry \
         invalidates the object refs and deletes no payload"
    );

    assert_eq!(maintain(state.clone(), false, true).await, (0, 1));
    let purged = submission().await;
    assert_eq!(purged.status, StorageTraceCorpusStatus::Purged);
    assert!(purged.purged_at.is_some());
    assert_eq!(
        retention_follow_up(
            &fixture.owner,
            tenant,
            &run,
            "retention_expired",
            "pipeline_retention_purge"
        )
        .await,
        (1, live_object_refs, 0),
        "the purge queues one payload deletion per live object ref"
    );
    assert_eq!(
        maintain(state, false, true).await,
        (0, 0),
        "a purged submission is done"
    );
}

/// Finding 18: `main`'s rollback-flag drill leaves the database-only rows
/// of submissions with a pipeline run (the submissions and their
/// tombstones) out of its blocking gaps, as the DB reconciliation does, and
/// counts them on their own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_rollback_drill_counts_database_only_pipeline_rows_apart() {
    let Some(fixture) = product_fixture().await else {
        return;
    };
    let state = &fixture.base.state;
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let withdrawn = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    fixture
        .base
        .service
        .withdraw_submission(tenant, withdrawn.submission_id, &principal, None)
        .await
        .expect("the owner withdraws the second run");
    let (status, body) = route_request(
        state.clone(),
        "POST",
        "/v1/admin/rollback-drill",
        auth_headers(&fixture.admin_token),
        Some(serde_json::json!({ "purpose": "pipeline rollback drill test" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["blocking_gaps"], serde_json::json!([]), "{body}");
    assert_eq!(body["ready"], true);
    assert_eq!(body["pipeline_db_only_submission_count"], 2);
    assert_eq!(body["pipeline_db_only_tombstone_count"], 1);
}

/// Finding 18 and its addendum: `main`'s operational summary reads `main`'s
/// NEAR outbox lines only, as `main`'s outbox listing does, so a pipeline
/// payout line that failed for good neither counts in its NEAR summary nor
/// holds its promotion gates (and the rollout-smoke readiness built from
/// them). `main`'s reads run on the database, the only store that holds
/// pipeline lines.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mains_operational_summary_leaves_a_failed_pipeline_payout_line_out() {
    let near = Arc::new(RecordingNearAdapter::new());
    let Some(fixture) = withdrawal_fixture_with(
        |runtime, artifacts| trace_credit_payout_service(runtime, artifacts, near.clone()),
        true,
    )
    .await
    else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    let admin_token = format!("{}-admin", fixture.token);
    let mut state = fixture.state.clone();
    let mut tokens = (*state.tokens).clone();
    insert_token(&mut tokens, tenant, &admin_token, TokenRole::Admin);
    Arc::make_mut(&mut state).tokens = Arc::new(tokens);
    let principal = static_token_principal_ref(&fixture.token);
    completed_pipeline_run(&fixture.service, tenant, &principal).await;
    assert_eq!(
        fixture.service.process_payouts(tenant, 32).await.unwrap(),
        1
    );
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant).await;
    let failed = tx
        .execute(
            "UPDATE trace_near_credit_outbox SET status = 'failed'
              WHERE tenant_id = $1 AND instrument_id IS NOT NULL",
            &[&tenant],
        )
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(failed, 1, "the pipeline's payout line fails for good");

    let (status, body) = route_request(
        state,
        "GET",
        "/v1/admin/operational-summary",
        auth_headers(&admin_token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["near_credit"]["item_count"], 0, "{body}");
    assert_eq!(body["near_credit"]["failed_count"], 0, "{body}");
    let gates = body["promotion_gates"]["blocking_gates"]
        .as_array()
        .expect("the blocking gates")
        .iter()
        .map(|gate| gate.as_str().expect("a gate label").to_string())
        .collect::<Vec<_>>();
    assert!(
        gates
            .iter()
            .all(|gate| !gate.starts_with("near_credit_outbox")),
        "{gates:?}"
    );
}

/// Finding 18: `main`'s replay export with database replay reads leaves the
/// submissions with a pipeline run out of its source selection (they are
/// exported through pipeline snapshots, and their stored bodies are
/// pipeline artifacts): the export runs, and a legacy submission is still
/// exported.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mains_database_replay_export_leaves_pipeline_submissions_out() {
    let Some(mut fixture) = product_fixture().await else {
        return;
    };
    {
        let state = Arc::make_mut(&mut fixture.base.state);
        state.db_replay_export_reads = true;
        state.pipeline_store = Some(Arc::new(PgPipelineStore::new(fixture.base.runtime.clone())));
    }
    let state = fixture.base.state.clone();
    let tenant = fixture.base.tenant.as_str();
    let principal = static_token_principal_ref(&fixture.base.token);
    let run = completed_pipeline_run(&fixture.base.service, tenant, &principal).await;
    let mut legacy = sample_envelope().await;
    make_metadata_only_low_risk(&mut legacy);
    let legacy_id = legacy.submission_id;
    let _ = submit_trace_handler(
        State(state.clone()),
        auth_headers(&fixture.base.token),
        submit_body(legacy),
    )
    .await
    .expect("the legacy submission mirrors to the database");

    let (status, replay) = pipeline_product_request(
        state,
        "POST",
        "/v1/workers/replay-export",
        Some(fixture.export_token.as_str()),
        None,
        Some(serde_json::json!({"purpose": "trace_commons_worker_replay_dataset"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    let exported = replay["items"]
        .as_array()
        .expect("the exported items")
        .iter()
        .map(|item| item["submission_id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(exported, vec![serde_json::json!(legacy_id)], "{replay}");
    assert_ne!(exported, vec![serde_json::json!(run.submission_id)]);
}

/// Zaki review 1, round 2, N-2: the pipeline assessment route applies the
/// privileged-action consent check `main`'s review decision route applies
/// (`ensure_record_matches_privileged_action_policy_abac`): a reviewer whose
/// claims allow no consent scope of the submission, and a reviewer of a
/// tenant whose policy was narrowed after the receipt, each get `403`, and
/// no assessment is recorded. The same reviewer under the receipt's policy
/// records it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_pipeline_assessment_route_applies_mains_privileged_action_consent_check() {
    let Some(mut fixture) = withdrawal_fixture().await else {
        return;
    };
    let suffix = Uuid::new_v4().simple().to_string();
    let reviewer = format!("token-review-consent-{suffix}");
    let scoped_reviewer = format!("token-review-consent-scoped-{suffix}");
    let mut tokens = (*fixture.state.tokens).clone();
    insert_token(&mut tokens, &fixture.tenant, &reviewer, TokenRole::Reviewer);
    insert_token(
        &mut tokens,
        &fixture.tenant,
        &scoped_reviewer,
        TokenRole::Reviewer,
    );
    tokens
        .get_mut(&scoped_reviewer)
        .unwrap()
        .allowed_consent_scopes = BTreeSet::from([ConsentScope::ModelTraining]);
    Arc::make_mut(&mut fixture.state).tokens = Arc::new(tokens);
    let state = fixture.state.clone();
    let mut narrowed = state.clone();
    Arc::make_mut(&mut narrowed).tenant_policies = Arc::new(BTreeMap::from([(
        fixture.tenant.clone(),
        TenantSubmissionPolicy {
            allowed_consent_scopes: BTreeSet::from([ConsentScope::ModelTraining]),
            allowed_uses: BTreeSet::new(),
        },
    )]));
    let principal = "principal_sha256:review-consent";
    let claim = |state: Arc<AppState>, token: String, run_id: Uuid| async move {
        let (status, claim) = route_request(
            state,
            "POST",
            &format!("/v1/review/pipeline/runs/{run_id}/claim"),
            auth_headers(&token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{claim}");
        claim["lease_token"].clone()
    };
    let assess = |state: Arc<AppState>, token: String, run_id: Uuid, lease: serde_json::Value| async move {
        route_request(
            state,
            "POST",
            &format!("/v1/review/pipeline/runs/{run_id}/assessment"),
            auth_headers(&token),
            Some(serde_json::json!({
                "lease_token": lease,
                "recommendation": "reject",
                "reason": "privacy_review_required",
            })),
        )
        .await
    };
    let assessed = |run_id: Uuid| {
        let service = fixture.service.clone();
        let tenant = fixture.tenant.clone();
        async move {
            service
                .store()
                .load_review_assessment(&tenant, run_id)
                .await
                .unwrap()
                .is_some()
        }
    };

    let scoped_run = quarantined_pipeline_run(&fixture.service, &fixture.tenant, principal).await;
    let lease = claim(state.clone(), scoped_reviewer.clone(), scoped_run.run_id).await;
    let (status, refused) = assess(
        state.clone(),
        scoped_reviewer.clone(),
        scoped_run.run_id,
        lease,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert_eq!(
        refused["error"],
        "trace privileged action is not allowed for this tenant source"
    );
    assert!(!assessed(scoped_run.run_id).await);

    let run = quarantined_pipeline_run(&fixture.service, &fixture.tenant, principal).await;
    let lease = claim(state.clone(), reviewer.clone(), run.run_id).await;
    let (status, refused) = assess(narrowed, reviewer.clone(), run.run_id, lease.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{refused}");
    assert!(!assessed(run.run_id).await);
    let (status, recorded) = assess(state, reviewer, run.run_id, lease).await;
    assert_eq!(status, StatusCode::OK, "{recorded}");
    assert!(assessed(run.run_id).await);
}

/// N-2's sibling: `main`'s maintenance purge applies the privileged-action
/// consent check to every purge candidate
/// (`ensure_maintenance_purge_matches_privileged_action_policy`), and since
/// ruling RB-30 a submission with a pipeline run is one. Under a tenant
/// policy that allows none of its consent scopes, the purge answers `403`
/// and purges nothing; under the receipt's policy it purges.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_maintenance_purge_applies_the_consent_check_to_pipeline_submissions() {
    let Some(fixture) = withdrawal_fixture().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    let admin_token = format!("{}-admin", fixture.token);
    let principal = static_token_principal_ref(&fixture.token);
    let run = completed_pipeline_run(&fixture.service, tenant, &principal).await;
    let mut state = fixture.state.clone();
    {
        let state = Arc::make_mut(&mut state);
        let mut tokens = (*state.tokens).clone();
        insert_token(&mut tokens, tenant, &admin_token, TokenRole::Admin);
        state.tokens = Arc::new(tokens);
        state.pipeline_store = Some(Arc::new(PgPipelineStore::new(fixture.runtime.clone())));
    }
    let mut narrowed = state.clone();
    Arc::make_mut(&mut narrowed).tenant_policies = Arc::new(BTreeMap::from([(
        tenant.to_string(),
        TenantSubmissionPolicy {
            allowed_consent_scopes: BTreeSet::from([ConsentScope::ModelTraining]),
            allowed_uses: BTreeSet::new(),
        },
    )]));
    age_submission_past_expiry(&fixture.owner, tenant, run.submission_id).await;
    let purge = |state: Arc<AppState>| {
        let admin_token = admin_token.clone();
        async move {
            maintenance_handler(
                State(state),
                auth_headers(&admin_token),
                Json(TraceMaintenanceRequest {
                    purpose: Some("pipeline purge consent test".to_string()),
                    prune_export_cache: false,
                    purge_expired_before: Some(Utc::now()),
                    ..test_maintenance_request()
                }),
            )
            .await
            .map(|Json(response)| response.records_marked_purged)
        }
    };
    let status = || async {
        fixture
            .owner
            .get_trace_submission(tenant, run.submission_id)
            .await
            .unwrap()
            .expect("the submission row")
            .status
    };

    let refused = purge(narrowed)
        .await
        .expect_err("the policy refuses the purge");
    assert_eq!(refused.0, StatusCode::FORBIDDEN);
    assert_eq!(status().await, StorageTraceCorpusStatus::Accepted);
    assert_eq!(purge(state).await.expect("the purge runs"), 1);
    assert_eq!(status().await, StorageTraceCorpusStatus::Purged);
}
