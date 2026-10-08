// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `pipeline.py compare`: the same traces go through the old gate path (side
//! `baseline`) and through the versioned pipeline (side `candidate`), and the
//! decisions are compared for each trace.
//!
//! This part of the harness has no database and no HTTP:
//!
//! - `CompareCorpusReader` reads a corpus one line at a time and hashes the
//!   bytes it reads, so a 10,000-trace corpus never sits in memory.
//! - `compare_envelope` builds the same envelope bytes for one fixture on
//!   every call, so two runs of one pin give one report.
//! - `calibrate_floors` runs the bootstrap partition through `main`'s gate
//!   with floors of zero and derives the three floors from the measured
//!   values.
//!
//! The run takes the traces in serial order, one after the other, because
//! the two indexes must hold the same entries before each trace. Code marked
//! `// baseline-old-path:` applies to the old path only and goes away with
//! it. Nested inside `tests` beside `pipeline_corpus_pg_tests`.

use super::*;

use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering as AtomicOrdering;

use trace_commons_gate_api::pipeline::{
    AtomicUnits, BundlePackage, InstrumentId, Microcredits, ScoreEvidence, TenantStorageRef,
};
use trace_commons_gate_api::{
    Embedder, IndexEntryKey, NearestNeighbor, ReferenceEmbedder, ReferencePerplexityScorer,
    SettlementAdapter, VectorIndex, VectorIndexReader, VectorIndexWriter,
};
use trace_commons_gate_enclave::{
    EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, MockVectorIndex,
};
use trace_commons_protocol::llm::recording::{ExpectedToolResult, TraceFile, TraceToolCall};
use trace_commons_protocol::trace_contribution::{
    RawTraceContribution, RecordedTraceContributionOptions, TraceContributionEventType,
};
use trace_commons_server::trace_gate_service::EnclaveGateService;
use trace_commons_server::versioned_pipeline::{PipelineCaps, PipelineServiceBuilder};
use trace_commons_server::versioned_pipeline_authority::DeterministicPipelinePrivacyBoundary;
use trace_commons_server::versioned_pipeline_bundle::{
    MINIMAL_INDEX_ID, MINIMAL_PROJECTION_ID, MinimalPolicyBundle,
};
use trace_commons_server::versioned_pipeline_comparison::{
    AdmissionLabel, AlignmentAction, ComparisonRecord, ComparisonSide, CreditEvent, DerivedFloors,
    GateValues, ReviewLabel, ReviewSource, TraceComparison, alignment_action, compare_records,
    derive_floors, hash_rule,
};
use trace_commons_server::versioned_pipeline_compat::{CompatibilityBundleConfig, MainGateConfig};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapterRegistry,
};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_product::PipelineProductStore;

use super::pipeline_corpus_pg_tests::sha256_bytes;
use super::pipeline_http_pg_tests::{
    LEGACY_NOVELTY_UTILITY_CREDIT_POINTS_DELTA, TEST_PIPELINE_CREDIT_ISSUER, account_owner_backend,
    allow_all_test_authority, join_within, mains_database, post_trace, runtime_backend, send_http,
    serve_pipeline_app, tenant_tx, wait_for_pipeline_ready, wait_for_run_state,
};

// ---------------------------------------------------------------------------
// The corpus: one fixture on each line.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareFixture {
    label: String,
    trace_id: Uuid,
    submission_id: Uuid,
    created_at: DateTime<Utc>,
    secret_probe: String,
    privacy_risk: String,
    trace_file: TraceFile,
}

impl CompareFixture {
    fn is_valid(&self) -> bool {
        let label_ok = (1..=64).contains(&self.label.len())
            && self
                .label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        label_ok
            && !self.secret_probe.is_empty()
            && matches!(self.privacy_risk.as_str(), "low" | "medium" | "high")
    }
}

/// Reads one fixture for each call. It never holds more than one line.
struct CompareCorpusReader {
    reader: BufReader<std::fs::File>,
    partition: &'static str,
    position: u64,
    digest: sha2::Sha256,
    line: Vec<u8>,
}

impl CompareCorpusReader {
    fn open(partition: &'static str, path: &Path) -> Result<Self, &'static str> {
        let file = std::fs::File::open(path).map_err(|_| "compare_corpus_read_failed")?;
        Ok(Self {
            reader: BufReader::new(file),
            partition,
            position: 0,
            digest: sha2::Sha256::new(),
            line: Vec::new(),
        })
    }

    /// Reads the next line into `self.line` and hashes it. False at the end.
    fn read_line(&mut self) -> Result<bool, &'static str> {
        self.line.clear();
        let read = self
            .reader
            .read_until(b'\n', &mut self.line)
            .map_err(|_| "compare_corpus_read_failed")?;
        self.digest.update(&self.line);
        Ok(read > 0)
    }

    fn next_fixture(&mut self) -> Result<Option<CompareFixture>, &'static str> {
        if !self.read_line()? {
            return Ok(None);
        }
        let position = self.position;
        self.position += 1;
        match serde_json::from_slice::<CompareFixture>(&self.line) {
            Ok(fixture) if fixture.is_valid() => Ok(Some(fixture)),
            _ => {
                eprintln!(
                    "compare_corpus_line_invalid partition={} position={position}",
                    self.partition
                );
                Err("compare_corpus_line_invalid")
            }
        }
    }

    /// The digest of the whole file; reads the lines that are left.
    fn finish(mut self) -> Result<String, &'static str> {
        while self.read_line()? {}
        Ok(format!("sha256:{}", hex::encode(self.digest.finalize())))
    }
}

// ---------------------------------------------------------------------------
// The envelope.
// ---------------------------------------------------------------------------

/// The envelope of one fixture. The same fixture gives the same bytes on
/// every call: each field that the protocol crate fills at random or from the
/// clock is a function of the fixture here.
async fn compare_envelope(fixture: &CompareFixture) -> TraceContributionEnvelope {
    // `from_recorded_trace` gives `Utc::now()` to a step with no timestamp.
    let mut trace = fixture.trace_file.clone();
    for step in &mut trace.steps {
        step.timestamp.get_or_insert(fixture.created_at);
    }
    let mut raw = RawTraceContribution::from_recorded_trace(
        &trace,
        RecordedTraceContributionOptions {
            include_message_text: true,
            include_tool_payloads: true,
            pseudonymous_contributor_id: Some("sha256:compare-contributor".to_string()),
            tenant_scope_ref: Some("tenant_sha256:compare".to_string()),
            ..RecordedTraceContributionOptions::default()
        },
    );
    raw.trace_id = fixture.trace_id;
    raw.submission_id = fixture.submission_id;
    raw.created_at = fixture.created_at;
    raw.contributor.revocation_handle = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:compare-revocation:{}", fixture.label).as_bytes(),
    );
    let mut new_ids = std::collections::BTreeMap::new();
    for (index, event) in raw.events.iter_mut().enumerate() {
        let new_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:compare-event:{}:{index}", fixture.label).as_bytes(),
        );
        new_ids.insert(event.event_id, new_id);
        event.event_id = new_id;
        // An HTTP exchange event takes the clock time of the call.
        if event.event_type == TraceContributionEventType::HttpExchange {
            event.timestamp = fixture.created_at;
        }
    }
    for event in &mut raw.events {
        event.parent_event_id = event
            .parent_event_id
            .and_then(|parent| new_ids.get(&parent).copied());
    }
    let mut envelope = DeterministicTraceRedactor::try_default()
        .expect("compare_redactor_unavailable")
        .redact_trace(raw)
        .await
        .expect("compare_redaction_failed");
    envelope.privacy.residual_pii_risk = match fixture.privacy_risk.as_str() {
        "low" => ResidualPiiRisk::Low,
        "medium" | "high" => {
            make_metadata_only_low_risk(&mut envelope);
            set_metadata_only_tool_name(&mut envelope, &fixture.label);
            if fixture.privacy_risk == "medium" {
                ResidualPiiRisk::Medium
            } else {
                ResidualPiiRisk::High
            }
        }
        _ => unreachable!("CompareFixture::is_valid refuses any other privacy risk"),
    };
    envelope.consent.scopes = vec![ConsentScope::ModelTraining];
    envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
    envelope
}

// ---------------------------------------------------------------------------
// The gate configuration and the calibration.
// ---------------------------------------------------------------------------

/// `main`'s gate configuration for the run: the derived floors and the
/// values of `CompatibilityBundleConfig::local_reference()`.
fn compare_main_gate(floors: DerivedFloors) -> MainGateConfig {
    let reference = CompatibilityBundleConfig::local_reference();
    MainGateConfig {
        perplexity_floor_micros: Some(floors.perplexity_floor_micros),
        tail_fraction_floor_micros: Some(floors.tail_fraction_floor_micros),
        novelty_floor_micros: Some(floors.novelty_floor_micros),
        embed_insert_novelty_micros: reference.embed_insert_novelty_micros,
        top_k: reference.top_k,
        chunk_target_tokens: reference.chunk_target_tokens,
        chunk_max_tokens: reference.chunk_max_tokens,
        chunk_cap: reference.chunk_cap,
        chunk_min_tokens: reference.chunk_min_tokens,
        novelty_utility_microcredits: 2_500_000,
    }
}

// baseline-old-path:
fn baseline_orchestrator_config(gate: &MainGateConfig) -> EnclaveGateOrchestratorConfig {
    let floor = |value: Option<u64>| value.expect("compare_main_gate sets every floor");
    let perplexity_floor_micros = floor(gate.perplexity_floor_micros);
    EnclaveGateOrchestratorConfig {
        gate_policy_version: "compare_baseline_v1".to_string(),
        gate_version_hash: "compare_baseline".to_string(),
        perplexity_floor_micros,
        tail_fraction_floor_micros: floor(gate.tail_fraction_floor_micros),
        novelty_floor_micros: floor(gate.novelty_floor_micros),
        top_k: gate.top_k as usize,
        chunk_target_tokens: gate.chunk_target_tokens as usize,
        chunk_max_tokens: gate.chunk_max_tokens as usize,
        chunk_cap: gate.chunk_cap as usize,
        chunk_min_tokens: gate.chunk_min_tokens,
        embed_insert_novelty_micros: gate.embed_insert_novelty_micros,
        // PC-D7: `main` takes the perplexity floor when no knob is set.
        qualifying_chunk_floor_micros: perplexity_floor_micros,
    }
}

/// Runs the bootstrap partition through `main`'s gate with floors of zero and
/// derives the three floors from the values it measured.
async fn calibrate_floors(bootstrap: &Path) -> Result<DerivedFloors, &'static str> {
    let mut config = baseline_orchestrator_config(&compare_main_gate(DerivedFloors {
        perplexity_floor_micros: 0,
        tail_fraction_floor_micros: 0,
        novelty_floor_micros: 0,
    }));
    config.gate_policy_version = "compare_calibration_v1".to_string();
    let orchestrator = std::sync::Arc::new(EnclaveGateOrchestrator::new(
        ReferencePerplexityScorer::new(),
        ReferenceEmbedder::new(),
        MockVectorIndex::new(),
        config,
    ));
    let mut perplexity = Vec::new();
    let mut tail_fraction = Vec::new();
    let mut novelty = Vec::new();
    let mut reader = CompareCorpusReader::open("bootstrap", bootstrap)?;
    while let Some(fixture) = reader.next_fixture()? {
        let bytes = serde_json::to_vec(&compare_envelope(&fixture).await)
            .map_err(|_| "comparison_calibration_failed")?;
        let gate = std::sync::Arc::clone(&orchestrator);
        let decision =
            tokio::task::spawn_blocking(move || gate.evaluate(&bytes, "compare-calibration"))
                .await
                .map_err(|_| "comparison_calibration_failed")?
                .map_err(|_| "comparison_calibration_failed")?;
        perplexity.push(decision.perplexity_micros);
        tail_fraction.push(decision.tail_fraction_micros);
        novelty.push(decision.novelty_score_micros);
    }
    derive_floors(&perplexity, &tail_fraction, &novelty)
}

// ---------------------------------------------------------------------------
// The app: one process, one database, two tenants.
// ---------------------------------------------------------------------------

/// The tenant names and the five tokens of one app.
struct CompareTenants {
    // baseline-old-path: the baseline tenant and its three tokens. The gate
    // token is for the old gate route (`TokenRole::VectorWorker`).
    baseline: String,
    baseline_contributor: String,
    baseline_reviewer: String,
    baseline_gate: String,
    candidate: String,
    candidate_contributor: String,
    candidate_reviewer: String,
}

impl CompareTenants {
    /// The fixed names of `pipeline_compare_run`: `tenant-compare-baseline`,
    /// `tenant-compare-candidate`, and their tokens.
    fn run() -> Self {
        Self::named("")
    }

    /// Names with a random suffix, for a test: a tenant keeps its first
    /// bundle, and two tests with different floors have different bundles
    /// (PC-D17).
    fn random() -> Self {
        Self::named(&format!("-{}", Uuid::new_v4().simple()))
    }

    fn named(suffix: &str) -> Self {
        Self {
            baseline: format!("tenant-compare-baseline{suffix}"),
            candidate: format!("tenant-compare-candidate{suffix}"),
            baseline_contributor: format!("token-compare-baseline-contributor{suffix}"),
            baseline_reviewer: format!("token-compare-baseline-reviewer{suffix}"),
            baseline_gate: format!("token-compare-baseline-gate{suffix}"),
            candidate_contributor: format!("token-compare-candidate-contributor{suffix}"),
            candidate_reviewer: format!("token-compare-candidate-reviewer{suffix}"),
        }
    }

    fn tokens(&self) -> [&str; 5] {
        [
            &self.baseline_contributor,
            &self.baseline_reviewer,
            &self.baseline_gate,
            &self.candidate_contributor,
            &self.candidate_reviewer,
        ]
    }
}

struct CompareAppOptions<'a> {
    tenants: CompareTenants,
    /// `None`, or `baseline_quality_floor` (PC-D8).
    // baseline-old-path:
    skew: Option<&'a str>,
    /// `true` in the run (spec section 8.1). `false` only in the tests that
    /// need a baseline quarantine of a medium-risk trace.
    // baseline-old-path:
    accept_medium_risk: bool,
}

struct CompareApp {
    base: String,
    client: reqwest::Client,
    owner: Arc<PgBackend>,
    runtime: Arc<PgBackend>,
    service: Arc<PipelineService>,
    package: BundlePackage,
    tenants: CompareTenants,
    /// A copy of `state.root`: the directory that `read_all_derived_records`
    /// reads (PC-D18).
    root: PathBuf,
    /// The HTTP calls that the drivers sent through `CompareApp::send`.
    http_calls: std::sync::atomic::AtomicU64,
    stop: tokio::sync::oneshot::Sender<()>,
    server: tokio::task::JoinHandle<anyhow::Result<()>>,
    _state_dir: tempfile::TempDir,
}

impl CompareApp {
    async fn shutdown(self) {
        self.stop.send(()).expect("compare_app_stop_failed");
        join_within(self.server, 20, "compare app").await;
    }

    fn http_call_count(&self) -> u64 {
        self.http_calls.load(AtomicOrdering::Relaxed)
    }

    /// One HTTP call of a driver, with the bearer `token`. It answers the
    /// status code and the body, as JSON when the body is JSON. The tokens
    /// are checked against a sent body, and all probes against the received
    /// body. A transport error is `compare_http_failed`, where `send_http`
    /// and `post_trace` panic. The body is not kept.
    async fn send(
        &self,
        probes: &Probes,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<Vec<u8>>,
    ) -> Result<(u16, serde_json::Value), &'static str> {
        self.http_calls.fetch_add(1, AtomicOrdering::Relaxed);
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.base))
            .bearer_auth(token);
        if let Some(body) = body {
            probes.check_sent(&body, "compare_probe_in_request");
            request = request
                .header("content-type", "application/json")
                .body(body);
        }
        let response = request.send().await.map_err(|_| "compare_http_failed")?;
        let code = response.status().as_u16();
        let received = response.bytes().await.map_err(|_| "compare_http_failed")?;
        probes.check(&received, "compare_probe_in_response");
        Ok((
            code,
            serde_json::from_slice(&received).unwrap_or(serde_json::Value::Null),
        ))
    }
}

/// The time limit of one HTTP call of the harness.
const HTTP_CALL_BOUND: std::time::Duration = std::time::Duration::from_secs(60);

/// The candidate's runtime: the compatibility package with `main`'s gate
/// values (`CompatibilityBundleConfig::production_compatible`), the reference
/// scorer and embedder, the isolated index, one recording `trace_credit`
/// adapter on the rail `none`, and `main`'s deterministic privacy boundary
/// (PC-D6). The shape of `CompatibilityTestAssembler`.
struct CompareAssembler;

impl IngestPipelineRuntimeAssembler for CompareAssembler {
    fn assemble(
        &self,
        context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        let scorer = Arc::new(ReferencePerplexityScorer::new());
        let embedder = Arc::new(ReferenceEmbedder::new());
        let config = CompatibilityBundleConfig::production_compatible(
            "reference_perplexity.v1".into(),
            MINIMAL_PROJECTION_ID.into(),
            MINIMAL_INDEX_ID.into(),
            &context.main_gate,
        )?;
        let package = MinimalPolicyBundle::compatibility_package(
            &config,
            scorer.as_ref(),
            embedder.as_ref(),
        )?;
        let trace_credit: Arc<dyn SettlementAdapter> = RecordingSettlementAdapter::new(
            InstrumentId::trace_credit(),
            "recording_trace_credit_compare_test_only",
            "none",
        );
        let caps = PipelineCaps {
            per_instrument_atomic_units: BTreeMap::from([(
                InstrumentId::trace_credit().as_str().to_string(),
                AtomicUnits::from_raw(u128::MAX),
            )]),
        };
        let index = IsolatedPipelineIndex::new();
        let service = PipelineServiceBuilder::new(
            context.backend,
            context.artifact_store,
            package,
            index.clone(),
            index,
            SettlementAdapterRegistry::new(vec![trace_credit])?,
            caps,
        )
        .with_scorer(scorer)
        .with_embedder(embedder)
        .with_object_store_name(context.object_store_name)
        .with_novelty_utility_checks(context.novelty_utility_checks)
        .with_authority(allow_all_test_authority())
        .with_privacy(Arc::new(DeterministicPipelinePrivacyBoundary))
        .with_unqualified_routing(context.unqualified_routing_allowed)
        .build()?;
        Ok(Arc::new(service))
    }
}

/// The old gate's configuration for the run: `main`'s gate values, and the
/// skew when one is given. Any skew but `baseline_quality_floor` panics.
// baseline-old-path:
fn compare_baseline_config(
    gate: &MainGateConfig,
    skew: Option<&str>,
) -> EnclaveGateOrchestratorConfig {
    let mut config = baseline_orchestrator_config(gate);
    config.gate_policy_version = "compare_gate_v1".to_string();
    match skew {
        None => {}
        // PC-D8: the baseline fails each trace on quality, and the candidate
        // does not, so a run with this skew must report differences.
        Some("baseline_quality_floor") => config.perplexity_floor_micros = u64::MAX,
        Some(_) => panic!("compare_skew_invalid"),
    }
    config
}

/// Starts one ingest app that serves the two sides. The baseline tenant stays
/// on the old path, with an in-process gate that holds `gate`. The candidate
/// tenant is on the receipts list, so its receipts go to the pipeline, whose
/// compatibility bundle holds the same `gate`. The model is
/// `legacy_and_pipeline_tenants_match_under_equivalent_configuration`.
///
/// This function takes no value of the two sides from an environment
/// variable. The receipt handler builds a redactor for each receipt
/// (`rescrub_trace_envelope`), and that reads `TRACE_PRIVACY_FILTER_BACKEND`,
/// the home directory, and the current directory, for the two sides alike.
///
/// The app is `run_pipeline_app`, which starts the pipeline worker only. The
/// in-process perplexity score driver and the PII backstop driver are started
/// by `run_ingest`, and the test state holds `None` for the two
/// (`perplexity_score_driver`, `pii_backstop_driver`).
async fn start_compare_app(
    gate: MainGateConfig,
    artifact_root: &Path,
    master_key_hex: &str,
    options: CompareAppOptions<'_>,
) -> CompareApp {
    assert_eq!(gate.novelty_utility_microcredits, 2_500_000);
    let baseline_config = compare_baseline_config(&gate, options.skew);
    let tenants = options.tenants;
    let runtime = runtime_backend(8)
        .await
        .expect("compare_database_url_missing");
    // Migrates the suite's database and resets the account rate limiter.
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");

    let mut tokens = BTreeMap::new();
    for (tenant, token, role) in [
        (
            &tenants.baseline,
            &tenants.baseline_contributor,
            TokenRole::Contributor,
        ),
        (
            &tenants.baseline,
            &tenants.baseline_reviewer,
            TokenRole::Reviewer,
        ),
        (
            &tenants.baseline,
            &tenants.baseline_gate,
            TokenRole::VectorWorker,
        ),
        (
            &tenants.candidate,
            &tenants.candidate_contributor,
            TokenRole::Contributor,
        ),
        (
            &tenants.candidate,
            &tenants.candidate_reviewer,
            TokenRole::Reviewer,
        ),
    ] {
        insert_token(&mut tokens, tenant, token, role);
    }

    // baseline-old-path: the old gate reads only KEK-wrapped (v2) envelopes,
    // so the two paths store into the service-owned store of the gate-worker
    // tests, and the old gate gets the decryptor of that store.
    let state_dir = tempfile::tempdir().expect("temp dir");
    let (configured_store, decryptor, _) =
        fixture_gate_worker_artifact_store_with_key(artifact_root, master_key_hex);
    let mut state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
        state_dir.path().to_path_buf(),
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
    let connections = TraceCorpusDbConnections {
        database: runtime.clone() as Arc<dyn Database>,
        postgres: runtime.clone(),
    };
    // The argument values of `assemble_compatibility_pipeline_service_with`:
    // not production-required, tenants processed, test dependencies allowed,
    // and unqualified routing allowed (PC-D15).
    let service = assemble_ingest_pipeline_runtime(
        Some(&CompareAssembler),
        Some(&connections),
        Some(state.artifact_store.as_ref().expect("a configured store")),
        false,
        trace_commons_server::versioned_pipeline::PipelineLeaseConfig::default(),
        true,
        true,
        true,
        None,
        TEST_NEAR_CONFIRMATION_INTERVAL,
        TEST_NEAR_PAYOUT_CONTROLS,
        &PipelineNoveltyUtilityChecks {
            issuer_principal_ref: Some(TEST_PIPELINE_CREDIT_ISSUER.to_string()),
            ..PipelineNoveltyUtilityChecks::default()
        },
        gate,
    )
    .expect("compare_pipeline_assembly_failed")
    .expect("an assembler was given, so a service is returned");
    let package = service.default_package().clone();

    let state_mut = Arc::make_mut(&mut state);
    // PC-D16: the run sends more than 30 receipts for each token. This call
    // is after the last `account_owner_backend()` call, which resets the
    // limiter and removes an earlier override.
    configure_unbounded_submit_limits_for_test(&tokens);
    state_mut.tokens = Arc::new(tokens);
    // baseline-old-path: `main`'s gate, in process, with the values of `gate`.
    state_mut.gate_service = Arc::new(EnclaveGateService::new(
        EnclaveGateOrchestrator::new(
            ReferencePerplexityScorer::new(),
            ReferenceEmbedder::new(),
            MockVectorIndex::new(),
            baseline_config,
        ),
        decryptor,
        "enclave_reference_compare",
    ));
    // baseline-old-path: the credit delta and the medium-risk rule of the
    // old path. The candidate has them in its bundle.
    state_mut.novelty_utility_credit_points_delta = LEGACY_NOVELTY_UTILITY_CREDIT_POINTS_DELTA;
    state_mut.accept_medium_risk_submissions = options.accept_medium_risk;
    state_mut.pipeline_service = Some(service.clone());
    // PC-D15: the harness writes no routing row. The receipts list alone
    // routes the candidate tenant.
    state_mut.pipeline_activation = routing_store(&runtime);
    state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[tenants.candidate.as_str()],
    );
    assert!(
        state_mut.submission_quota.is_disabled(),
        "compare_submission_quota_enabled"
    );
    assert!(
        state_mut.perplexity_score_driver.is_none() && state_mut.pii_backstop_driver.is_none(),
        "compare_driver_configured"
    );
    let root = state_mut.root.clone();

    let (base, stop, server) = serve_pipeline_app(state).await;
    // A server that does not answer gives `compare_http_failed`, so the run
    // can write its partial report.
    let client = reqwest::Client::builder()
        .timeout(HTTP_CALL_BOUND)
        .build()
        .expect("compare_http_client_failed");
    wait_for_pipeline_ready(&client, &base).await;
    let app = CompareApp {
        base,
        client,
        owner,
        runtime,
        service,
        package,
        tenants,
        root,
        http_calls: std::sync::atomic::AtomicU64::new(0),
        stop,
        server,
        _state_dir: state_dir,
    };
    // PC-D17: a tenant keeps its first bundle, so a tenant that an earlier
    // app used with other floors would be scored with those floors.
    assert_eq!(
        active_bundle_id(&app).await.as_deref(),
        Some(app.package.bundle_id.as_str()),
        "compare_candidate_bundle_mismatch"
    );
    app
}

/// The candidate tenant's active bundle.
async fn active_bundle_id(app: &CompareApp) -> Option<String> {
    let mut client = app
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .expect("compare_database_failed");
    let tx = tenant_tx(&mut client, &app.tenants.candidate).await;
    let bundle_id = tx
        .query_opt(
            "SELECT bundle_id FROM pipeline_active_bundles WHERE tenant_id = $1",
            &[&app.tenants.candidate],
        )
        .await
        .expect("compare_database_failed")
        .map(|row| row.get(0));
    tx.commit().await.expect("compare_database_failed");
    bundle_id
}

// ---------------------------------------------------------------------------
// The probes: the strings that no response, record, or report may contain.
// ---------------------------------------------------------------------------

/// The number of bytes of trace text in the content probe.
const CONTENT_PROBE_BYTES: usize = 48;

/// The strings that no HTTP response, record, or report may contain, for one
/// fixture. `secret_probe` is a label beside the text, so it cannot find trace
/// text. The content probe can: the first 48 bytes (down to a character
/// boundary) of the first event text of the envelope that has 48 bytes or
/// more, as it is and as JSON text holds it. It is not checked against a sent
/// body, which holds the text by design.
struct Probes {
    tokens: Vec<String>,
    secret_probe: String,
    /// No entry for an envelope with no such text; two entries when JSON
    /// escapes a character of the window.
    content: Vec<String>,
    /// The number of `check` calls.
    checks: std::sync::atomic::AtomicU64,
}

impl Probes {
    fn new(
        tenants: &CompareTenants,
        fixture: &CompareFixture,
        envelope: &TraceContributionEnvelope,
    ) -> Self {
        let mut content = Vec::new();
        let text = envelope
            .events
            .iter()
            .filter_map(|event| event.redacted_content.as_deref())
            .find(|text| text.len() >= CONTENT_PROBE_BYTES);
        if let Some(text) = text {
            let mut end = CONTENT_PROBE_BYTES;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let window = &text[..end];
            content.push(window.to_string());
            let json = serde_json::Value::from(window).to_string();
            let escaped = &json[1..json.len() - 1];
            if escaped != window {
                content.push(escaped.to_string());
            }
        }
        Self {
            tokens: tenants.tokens().map(str::to_string).to_vec(),
            secret_probe: fixture.secret_probe.clone(),
            content,
            checks: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn check_count(&self) -> u64 {
        self.checks.load(AtomicOrdering::Relaxed)
    }

    /// All probes. For a received body, a record, and the report. Panics
    /// with `label`: a probe in one of these is a defect of the server or of
    /// the harness, and the run must not continue.
    fn check(&self, bytes: &[u8], label: &'static str) {
        self.checks.fetch_add(1, AtomicOrdering::Relaxed);
        self.check_sent(bytes, label);
        for probe in std::iter::once(&self.secret_probe).chain(&self.content) {
            assert!(!holds(bytes, probe), "{label}");
        }
    }

    /// The tokens only. For a sent body, which holds the trace text by
    /// design.
    fn check_sent(&self, bytes: &[u8], label: &'static str) {
        for token in &self.tokens {
            assert!(!holds(bytes, token), "{label}");
        }
    }
}

fn holds(bytes: &[u8], probe: &str) -> bool {
    bytes
        .windows(probe.len())
        .any(|window| window == probe.as_bytes())
}

// ---------------------------------------------------------------------------
// The admission step: the receipt of each side, and what the side decided.
// ---------------------------------------------------------------------------

/// What a driver knows after the receipt, before any review or gate call.
#[derive(Debug, Clone)]
struct AdmissionObservation {
    receipt_code: u16,
    privacy_risk: Option<String>,
    privacy_basis: Vec<String>,
    admission: AdmissionLabel,
}

fn database_failed<E>(_: E) -> &'static str {
    "compare_database_failed"
}

/// A connection of the owner pool, for the reads of the drivers.
async fn compare_client(app: &CompareApp) -> Result<deadpool_postgres::Client, &'static str> {
    app.owner
        .trace_pool_for_test()
        .get()
        .await
        .map_err(database_failed)
}

/// `tenant_tx` with a label in place of a panic.
async fn compare_tx<'a>(
    client: &'a mut deadpool_postgres::Client,
    tenant: &str,
) -> Result<deadpool_postgres::Transaction<'a>, &'static str> {
    let tx = client.transaction().await.map_err(database_failed)?;
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .map_err(database_failed)?;
    Ok(tx)
}

/// Sends the receipt of one side and reads what the side stored: the
/// submission row, and the Admission decision of the run when a run exists
/// (the candidate). One statement serves the two sides.
async fn observe_receipt(
    app: &CompareApp,
    probes: &Probes,
    tenant: &str,
    token: &str,
    body: &[u8],
    submission_id: Uuid,
) -> Result<(u16, Option<tokio_postgres::Row>), &'static str> {
    let (receipt_code, _) = app
        .send(
            probes,
            reqwest::Method::POST,
            "/v1/traces",
            token,
            Some(body.to_vec()),
        )
        .await?;
    let mut client = compare_client(app).await?;
    let tx = compare_tx(&mut client, tenant).await?;
    let row = tx
        .query_opt(
            "SELECT s.status, s.privacy_risk, s.residual_risk_basis, r.admission_decision
               FROM trace_submissions s
               LEFT JOIN pipeline_runs r
                 ON r.tenant_id = s.tenant_id AND r.submission_id = s.submission_id
              WHERE s.tenant_id = $1 AND s.submission_id = $2",
            &[&tenant, &submission_id],
        )
        .await
        .map_err(database_failed)?;
    tx.commit().await.map_err(database_failed)?;
    Ok((receipt_code, row))
}

/// The observation of one side. `decision` gives the admission of a receipt
/// that answered 200 from its row; a receipt that did not is `Refused`, and a
/// receipt with no row is `Other`.
fn admission_observation(
    receipt_code: u16,
    row: Option<tokio_postgres::Row>,
    decision: impl Fn(&tokio_postgres::Row) -> Result<AdmissionLabel, &'static str>,
) -> Result<AdmissionObservation, &'static str> {
    let mut seen = AdmissionObservation {
        receipt_code,
        privacy_risk: None,
        privacy_basis: Vec::new(),
        admission: if receipt_code == 200 {
            AdmissionLabel::Other
        } else {
            AdmissionLabel::Refused
        },
    };
    let Some(row) = row else {
        return Ok(seen);
    };
    seen.privacy_risk = row.try_get(1).map_err(database_failed)?;
    // A NULL column gives an empty list.
    let basis: Option<serde_json::Value> = row.try_get(2).map_err(database_failed)?;
    if let Some(basis) = basis {
        seen.privacy_basis = serde_json::from_value(basis).map_err(database_failed)?;
        seen.privacy_basis.sort();
    }
    if receipt_code == 200 {
        seen.admission = decision(&row)?;
    }
    Ok(seen)
}

// baseline-old-path:
async fn baseline_admission(
    app: &CompareApp,
    probes: &Probes,
    body: &[u8],
    submission_id: Uuid,
) -> Result<AdmissionObservation, &'static str> {
    let tenants = &app.tenants;
    let (receipt_code, row) = observe_receipt(
        app,
        probes,
        &tenants.baseline,
        &tenants.baseline_contributor,
        body,
        submission_id,
    )
    .await?;
    // The old path has no direct rejection: it accepts or it quarantines.
    admission_observation(receipt_code, row, |row| {
        let status: String = row.try_get(0).map_err(database_failed)?;
        Ok(match status.as_str() {
            "accepted" => AdmissionLabel::Admit,
            "quarantined" => AdmissionLabel::Quarantine,
            _ => AdmissionLabel::Other,
        })
    })
}

async fn candidate_admission(
    app: &CompareApp,
    probes: &Probes,
    body: &[u8],
    submission_id: Uuid,
) -> Result<AdmissionObservation, &'static str> {
    let tenants = &app.tenants;
    let (receipt_code, row) = observe_receipt(
        app,
        probes,
        &tenants.candidate,
        &tenants.candidate_contributor,
        body,
        submission_id,
    )
    .await?;
    // `pipeline_runs.admission_decision`; no run after a 200 receipt is
    // `Other`.
    admission_observation(receipt_code, row, |row| {
        let decision: Option<String> = row.try_get(3).map_err(database_failed)?;
        Ok(match decision.as_deref() {
            Some("admit") => AdmissionLabel::Admit,
            Some("quarantine") => AdmissionLabel::Quarantine,
            Some("reject") => AdmissionLabel::Reject,
            _ => AdmissionLabel::Other,
        })
    })
}

// ---------------------------------------------------------------------------
// The finish step: each side reaches a terminal state and gives its record.
// ---------------------------------------------------------------------------

/// `position`, `partition`, `trace_hash`: equal on the two sides.
struct RecordBase {
    position: u64,
    partition: &'static str,
    trace_hash: String,
}

impl RecordBase {
    /// The record of a side that did nothing after its receipt.
    fn record(self, side: ComparisonSide, seen: AdmissionObservation) -> ComparisonRecord {
        ComparisonRecord {
            position: self.position,
            partition: self.partition.to_string(),
            trace_hash: self.trace_hash,
            side,
            receipt_code: seen.receipt_code,
            terminal: false,
            privacy_risk: seen.privacy_risk,
            privacy_basis: seen.privacy_basis,
            admission: seen.admission,
            review: ReviewLabel::None,
            review_source: ReviewSource::None,
            gate_skipped_by_alignment: false,
            scored: false,
            gate: None,
            member: false,
            member_chunks: Vec::new(),
            credit_events: Vec::new(),
        }
    }
}

/// The canonical bytes of a record: the bytes that the run writes and hashes.
fn record_bytes(record: &ComparisonRecord) -> Vec<u8> {
    let value = serde_json::to_value(record).expect("a record is numbers and labels");
    trace_commons_protocol::canonical_json::to_canonical_vec(&value)
        .expect("a record is numbers and labels")
}

/// The last step of each finish: no probe is in the record.
fn checked(probes: &Probes, record: ComparisonRecord) -> ComparisonRecord {
    probes.check(&record_bytes(&record), "compare_probe_in_record");
    record
}

fn review_word(decision: ReviewLabel) -> &'static str {
    if decision == ReviewLabel::Approve {
        "approve"
    } else {
        "reject"
    }
}

/// The credit events of one submission, in the order of the ledger, as
/// `ledger_rows` of the parity test reads them.
async fn credit_events(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    submission_id: Uuid,
) -> Result<Vec<CreditEvent>, &'static str> {
    tx.query(
        "SELECT event_type, points_delta FROM trace_credit_ledger
          WHERE tenant_id = $1 AND submission_id = $2
          ORDER BY occurred_at",
        &[&tenant, &submission_id],
    )
    .await
    .map_err(database_failed)?
    .iter()
    .map(|row| {
        let points: String = row.try_get(1).map_err(database_failed)?;
        Ok(CreditEvent {
            event_type: row.try_get(0).map_err(database_failed)?,
            microcredits: Microcredits::from_credit_decimal(&points)
                .map_err(database_failed)?
                .get(),
        })
    })
    .collect()
}

/// The gate values of one `trace_gate_decisions` row, in the column order of
/// `baseline_finish`. A NULL in a column that `main` always writes is
/// `compare_database_failed`.
// baseline-old-path:
fn baseline_gate_values(row: &tokio_postgres::Row) -> Result<GateValues, &'static str> {
    let micros = |column: usize| -> Result<u64, &'static str> {
        u64::try_from(row.try_get::<_, i64>(column).map_err(database_failed)?)
            .map_err(database_failed)
    };
    // The two counts are INT4: a read as `i64` is an error.
    let count = |column: usize| -> Result<u32, &'static str> {
        u32::try_from(row.try_get::<_, i32>(column).map_err(database_failed)?)
            .map_err(database_failed)
    };
    Ok(GateValues {
        quality_passed: row.try_get(1).map_err(database_failed)?,
        novelty_passed: row.try_get(2).map_err(database_failed)?,
        perplexity_micros: micros(3)?,
        tail_fraction_micros: micros(4)?,
        peak_perplexity_micros: micros(5)?,
        novelty_score_micros: micros(6)?,
        peak_novelty_micros: micros(7)?,
        chunk_count: count(8)?,
        total_chunk_count: count(9)?,
        chunks_capped: row.try_get(10).map_err(database_failed)?,
        index_cardinality: row
            .try_get::<_, Option<i64>>(11)
            .map_err(database_failed)?
            .map(u64::try_from)
            .transpose()
            .map_err(database_failed)?,
        credit_quality_micros: row.try_get(12).map_err(database_failed)?,
        credit_quality_version: row
            .try_get::<_, Option<i32>>(13)
            .map_err(database_failed)?
            .map(i64::from),
    })
}

/// Moves the baseline to a terminal state for `action` and gives its record.
///
/// - A review action claims the lease and sends the decision, with the
///   request shapes of `reviewer_can_approve_quarantined_trace_and_export_dataset`
///   (`tests.rs`).
/// - A trace that is then accepted goes to the gate route, except for
///   `SkipBaselineGate` and `Stop`.
/// - `terminal` is true when each call of this side answered 200. The gate
///   route adds to the index before it writes the decision row, so a call
///   that failed can leave the index changed (PC-D20).
/// - A side that is `Refused` or `Other` gets no call and no read, and is
///   not terminal (PC-D19).
// baseline-old-path:
async fn baseline_finish(
    app: &CompareApp,
    probes: &Probes,
    fixture: &CompareFixture,
    seen: AdmissionObservation,
    action: AlignmentAction,
    base: RecordBase,
) -> Result<ComparisonRecord, &'static str> {
    let tenant = app.tenants.baseline.as_str();
    let submission_id = fixture.submission_id;
    let admission = seen.admission;
    let mut record = base.record(ComparisonSide::Baseline, seen);
    if !matches!(
        admission,
        AdmissionLabel::Admit | AdmissionLabel::Quarantine
    ) {
        return Ok(checked(probes, record));
    }

    let review = match action {
        AlignmentAction::ApproveBaseline => Some((ReviewLabel::Approve, ReviewSource::Alignment)),
        AlignmentAction::RejectBaseline => Some((ReviewLabel::Reject, ReviewSource::Alignment)),
        AlignmentAction::HashRuleBoth(decision @ (ReviewLabel::Approve | ReviewLabel::Reject)) => {
            Some((decision, ReviewSource::HashRule))
        }
        _ => None,
    };
    let mut calls_succeeded = true;
    let mut accepted = admission == AdmissionLabel::Admit;
    if let Some((decision, source)) = review {
        record.review = decision;
        record.review_source = source;
        let reviewer = &app.tenants.baseline_reviewer;
        let (code, _) = app
            .send(
                probes,
                reqwest::Method::POST,
                &format!("/v1/review/{submission_id}/lease"),
                reviewer,
                Some(b"{}".to_vec()),
            )
            .await?;
        calls_succeeded = code == 200;
        if calls_succeeded {
            let body = serde_json::json!({
                "decision": review_word(decision),
                "reason": "comparison_review",
            });
            let (code, _) = app
                .send(
                    probes,
                    reqwest::Method::POST,
                    &format!("/v1/review/{submission_id}/decision"),
                    reviewer,
                    Some(body.to_string().into_bytes()),
                )
                .await?;
            calls_succeeded = code == 200;
        }
        accepted = calls_succeeded && decision == ReviewLabel::Approve;
    }
    record.gate_skipped_by_alignment = accepted && action == AlignmentAction::SkipBaselineGate;
    if accepted
        && !matches!(
            action,
            AlignmentAction::SkipBaselineGate | AlignmentAction::Stop
        )
    {
        let body = serde_json::json!({ "submission_id": submission_id });
        let (code, _) = app
            .send(
                probes,
                reqwest::Method::POST,
                "/v1/workers/gate/evaluate",
                &app.tenants.baseline_gate,
                Some(body.to_string().into_bytes()),
            )
            .await?;
        record.scored = code == 200;
        calls_succeeded = record.scored;
    }
    record.terminal = calls_succeeded;

    let mut client = compare_client(app).await?;
    let tx = compare_tx(&mut client, tenant).await?;
    let decisions = tx
        .query(
            "SELECT decision_id, perplexity_passed, novelty_passed, perplexity_micros,
                    tail_fraction_micros, peak_perplexity_micros, novelty_score_micros,
                    peak_novelty_micros, chunk_count, total_chunk_count, chunks_capped,
                    index_cardinality_at_scoring, credit_quality_micros,
                    credit_quality_calibration_version
               FROM trace_gate_decisions
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &submission_id],
        )
        .await
        .map_err(database_failed)?;
    // Each gate call makes a new decision and new index entries, so a second
    // decision means that the baseline index holds the trace twice.
    if decisions.len() > 1 {
        return Err("compare_baseline_scored_twice");
    }
    if record.scored {
        let decision = decisions.first().ok_or("compare_database_failed")?;
        record.gate = Some(baseline_gate_values(decision)?);
        let decision_id: Uuid = decision.try_get(0).map_err(database_failed)?;
        record.member_chunks = tx
            .query(
                "SELECT chunk_index FROM trace_gate_chunk_vector_entries
                  WHERE tenant_id = $1 AND decision_id = $2
                  ORDER BY chunk_index",
                &[&tenant, &decision_id],
            )
            .await
            .map_err(database_failed)?
            .iter()
            .map(|row| {
                u32::try_from(row.try_get::<_, i32>(0).map_err(database_failed)?)
                    .map_err(database_failed)
            })
            .collect::<Result<_, _>>()?;
        record.member = !record.member_chunks.is_empty();
    }
    record.credit_events = credit_events(&tx, tenant, submission_id).await?;
    tx.commit().await.map_err(database_failed)?;
    Ok(checked(probes, record))
}

/// The bound for the whole finish step of the candidate, and the pause
/// between two reads of the run state (PC-D14).
const CANDIDATE_FINISH_BOUND: std::time::Duration = std::time::Duration::from_secs(60);
const CANDIDATE_POLL_PAUSE: std::time::Duration = std::time::Duration::from_millis(25);

/// Reads the run of `submission_id` until its state is one at which the
/// worker stops (`awaiting_review`, `complete`, `failed`) or `deadline`
/// passes. It answers the run id and the last state. The state read is the
/// only statement in the loop.
async fn wait_for_candidate_run(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    submission_id: Uuid,
    deadline: std::time::Instant,
) -> Result<(Uuid, String), &'static str> {
    let read = tx
        .prepare(
            "SELECT run_id, state FROM pipeline_runs WHERE tenant_id = $1 AND submission_id = $2",
        )
        .await
        .map_err(database_failed)?;
    loop {
        let row = tx
            .query_one(&read, &[&tenant, &submission_id])
            .await
            .map_err(database_failed)?;
        let state: String = row.try_get(1).map_err(database_failed)?;
        if matches!(state.as_str(), "awaiting_review" | "complete" | "failed")
            || std::time::Instant::now() >= deadline
        {
            return Ok((row.try_get(0).map_err(database_failed)?, state));
        }
        tokio::time::sleep(CANDIDATE_POLL_PAUSE).await;
    }
}

/// The three pipeline review calls that `review_quarantine` of the corpus
/// harness sends: the queue (for the admission reason), the claim, and the
/// assessment. False when a call does not answer 200.
async fn review_candidate_run(
    app: &CompareApp,
    probes: &Probes,
    run_id: Uuid,
    decision: ReviewLabel,
) -> Result<bool, &'static str> {
    let reviewer = &app.tenants.candidate_reviewer;
    let (code, queue) = app
        .send(
            probes,
            reqwest::Method::GET,
            // The queue is oldest first, and its default limit is 50. The
            // store permits 500 at most (`list_review_queue`).
            "/v1/review/pipeline/quarantine?limit=500",
            reviewer,
            None,
        )
        .await?;
    let run_text = run_id.to_string();
    let reason = queue
        .as_array()
        .filter(|_| code == 200)
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["run_id"] == run_text.as_str())
        })
        .and_then(|item| item["admission_reason"].as_str());
    let Some(reason) = reason else {
        return Ok(false);
    };
    let (code, claim) = app
        .send(
            probes,
            reqwest::Method::POST,
            &format!("/v1/review/pipeline/runs/{run_id}/claim"),
            reviewer,
            None,
        )
        .await?;
    if code != 200 {
        return Ok(false);
    }
    let resolved = if decision == ReviewLabel::Approve {
        vec![reason]
    } else {
        Vec::new()
    };
    let body = serde_json::json!({
        "lease_token": claim["lease_token"],
        "recommendation": review_word(decision),
        "reason": reason,
        "resolved_quarantine_reasons": resolved,
    });
    let (code, assessment) = app
        .send(
            probes,
            reqwest::Method::POST,
            &format!("/v1/review/pipeline/runs/{run_id}/assessment"),
            reviewer,
            Some(body.to_string().into_bytes()),
        )
        .await?;
    Ok(code == 200 && assessment["assessment_id"].is_string())
}

/// The gate values of a Score outcome. The compatibility Score writes each
/// of these fields; a missing one is `compare_candidate_evidence_incomplete`.
fn candidate_gate_values(evidence: &ScoreEvidence) -> Result<GateValues, &'static str> {
    const INCOMPLETE: &str = "compare_candidate_evidence_incomplete";
    Ok(GateValues {
        quality_passed: evidence.quality_passed.ok_or(INCOMPLETE)?,
        novelty_passed: evidence.novelty_passed.ok_or(INCOMPLETE)?,
        perplexity_micros: evidence.perplexity_micros.ok_or(INCOMPLETE)?,
        tail_fraction_micros: evidence.tail_fraction_micros.ok_or(INCOMPLETE)?,
        peak_perplexity_micros: evidence.peak_perplexity_micros.ok_or(INCOMPLETE)?,
        novelty_score_micros: evidence.novelty_score_micros.ok_or(INCOMPLETE)?,
        peak_novelty_micros: evidence.peak_novelty_micros.ok_or(INCOMPLETE)?,
        chunk_count: evidence.chunk_count.ok_or(INCOMPLETE)?,
        total_chunk_count: evidence.total_chunk_count.ok_or(INCOMPLETE)?,
        chunks_capped: evidence.chunks_capped.ok_or(INCOMPLETE)?,
        index_cardinality: evidence.index_cardinality,
        credit_quality_micros: evidence
            .credit_quality_micros
            .map(i64::try_from)
            .transpose()
            .map_err(|_| INCOMPLETE)?,
        credit_quality_version: evidence.credit_quality_version.map(i64::from),
    })
}

/// Moves the candidate to a terminal state for `action` and gives its record.
/// The harness sends no call for an admitted or a rejected run: the worker
/// moves it. One bound of 60 s holds for the whole step.
///
/// - A review action first waits until the worker parks the run as
///   `awaiting_review`: the review routes refuse a run that the worker holds.
///   It then sends the three review calls, and sends them again after a call
///   that did not answer 200. No review inside the bound is
///   `compare_candidate_review_failed`.
/// - `terminal` is true only for the state `complete`. A run that Admission or
///   Review rejects ends as `complete` too. A quarantined run with no review
///   action stays `awaiting_review`, and the step stops at once.
/// - A side that is `Refused` or `Other` has no run to wait for: no poll, no
///   read, and not terminal (PC-D19).
async fn candidate_finish(
    app: &CompareApp,
    probes: &Probes,
    fixture: &CompareFixture,
    seen: AdmissionObservation,
    action: AlignmentAction,
    base: RecordBase,
) -> Result<ComparisonRecord, &'static str> {
    let tenant = app.tenants.candidate.as_str();
    let submission_id = fixture.submission_id;
    let admission = seen.admission;
    let mut record = base.record(ComparisonSide::Candidate, seen);
    if matches!(admission, AdmissionLabel::Refused | AdmissionLabel::Other) {
        return Ok(checked(probes, record));
    }
    let review = match action {
        AlignmentAction::ApproveCandidate => Some((ReviewLabel::Approve, ReviewSource::Alignment)),
        AlignmentAction::HashRuleBoth(decision @ (ReviewLabel::Approve | ReviewLabel::Reject)) => {
            Some((decision, ReviewSource::HashRule))
        }
        _ => None,
    };

    // One transaction for the whole step. It is READ COMMITTED, so each read
    // sees what the worker committed before it.
    let deadline = std::time::Instant::now() + CANDIDATE_FINISH_BOUND;
    let mut client = compare_client(app).await?;
    let tx = compare_tx(&mut client, tenant).await?;
    let (run_id, mut state) = wait_for_candidate_run(&tx, tenant, submission_id, deadline).await?;
    if let Some((decision, source)) = review {
        record.review = decision;
        record.review_source = source;
        if state != "awaiting_review" {
            return Err("compare_candidate_review_failed");
        }
        while !review_candidate_run(app, probes, run_id, decision).await? {
            if std::time::Instant::now() >= deadline {
                return Err("compare_candidate_review_failed");
            }
            tokio::time::sleep(CANDIDATE_POLL_PAUSE).await;
        }
        // The assessment moved the run out of `awaiting_review`.
        (_, state) = wait_for_candidate_run(&tx, tenant, submission_id, deadline).await?;
    }
    record.terminal = state == "complete";

    let row = tx
        .query_one(
            "SELECT r.index_membership, r.index_write_state, o.evidence
               FROM pipeline_runs r
               LEFT JOIN phase_outcomes o
                 ON o.tenant_id = r.tenant_id AND o.run_id = r.run_id AND o.phase = 'score'
              WHERE r.tenant_id = $1 AND r.run_id = $2",
            &[&tenant, &run_id],
        )
        .await
        .map_err(database_failed)?;
    record.credit_events = credit_events(&tx, tenant, submission_id).await?;
    tx.commit().await.map_err(database_failed)?;
    drop(client);

    let membership: String = row.try_get(0).map_err(database_failed)?;
    let write_state: String = row.try_get(1).map_err(database_failed)?;
    let evidence = row
        .try_get::<_, Option<serde_json::Value>>(2)
        .map_err(database_failed)?
        .map(serde_json::from_value::<ScoreEvidence>)
        .transpose()
        .map_err(|_| "compare_candidate_evidence_incomplete")?;
    if let Some(evidence) = &evidence {
        record.scored = true;
        record.gate = Some(candidate_gate_values(evidence)?);
    }
    // As `main` counts an include (`rebuildable_index_run_predicate`).
    record.member = membership == "included" && write_state == "complete";
    if record.member {
        let evidence = evidence
            .as_ref()
            .ok_or("compare_candidate_evidence_incomplete")?;
        let run = app
            .service
            .store()
            .get_run(tenant, run_id)
            .await
            .map_err(database_failed)?
            .ok_or("compare_database_failed")?;
        let command = app
            .service
            .load_index_command(&run, evidence)
            .await
            .map_err(|_| "compare_candidate_evidence_incomplete")?
            .ok_or("compare_candidate_evidence_incomplete")?;
        record.member_chunks = command.entries().iter().map(|entry| entry.chunk).collect();
        record.member_chunks.sort_unstable();
    }
    Ok(checked(probes, record))
}

/// PC-D18. Removes each `.json` file of the baseline tenant's `derived/`
/// directory (`<app.root>/tenants/<tenant_storage_key(baseline)>/derived/`)
/// and answers the number of files that it removed. A directory that does
/// not exist answers 0. It removes nothing else: no other directory, no file
/// of the candidate tenant, no database row. An I/O error is
/// `compare_baseline_derived_clear_failed`.
// baseline-old-path:
fn clear_baseline_derived(app: &CompareApp) -> Result<u64, &'static str> {
    const FAILED: &str = "compare_baseline_derived_clear_failed";
    let dir = app
        .root
        .join("tenants")
        .join(tenant_storage_key(&app.tenants.baseline))
        .join("derived");
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(_) => return Err(FAILED),
    };
    let mut removed = 0;
    for entry in entries {
        let path = entry.map_err(|_| FAILED)?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            std::fs::remove_file(path).map_err(|_| FAILED)?;
            removed += 1;
        }
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn the_corpus_reader_is_lazy() {
    let lines = [
        fixture_line("lazy_a", "low", prose_steps("one")),
        fixture_line("lazy_b", "low", prose_steps("two")),
        "this line is not json".to_string(),
    ];
    let (_dir, path) = write_corpus(&lines);
    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    assert!(reader.next_fixture().unwrap().is_some());
    assert!(reader.next_fixture().unwrap().is_some());
    assert_eq!(
        reader.next_fixture().err(),
        Some("compare_corpus_line_invalid")
    );

    // A reader that loaded the file in `open` cannot see a line that is
    // appended after the first read.
    let (_dir, path) = write_corpus(&[fixture_line("lazy_c", "low", prose_steps("three"))]);
    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    assert_eq!(reader.next_fixture().unwrap().unwrap().label, "lazy_c");
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(
        file,
        "{}",
        fixture_line("lazy_d", "low", prose_steps("four"))
    )
    .unwrap();
    assert_eq!(reader.next_fixture().unwrap().unwrap().label, "lazy_d");
    assert!(reader.next_fixture().unwrap().is_none());
}

#[test]
fn the_corpus_reader_refuses_bad_fixtures() {
    let good = serde_json::from_str::<serde_json::Value>(&fixture_line(
        "bad_base",
        "low",
        prose_steps("x"),
    ))
    .unwrap();
    let with = |key: &str, value: serde_json::Value| {
        let mut object = good.clone();
        object[key] = value;
        object.to_string()
    };
    let bad_lines = [
        with("unexpected", serde_json::json!(1)),
        with("privacy_risk", serde_json::json!("critical")),
        with("secret_probe", serde_json::json!("")),
        with("label", serde_json::json!("Not A Label")),
        with("label", serde_json::json!("")),
        with("label", serde_json::json!("a".repeat(65))),
    ];
    for line in bad_lines {
        let (_dir, path) = write_corpus(&[line]);
        let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
        assert_eq!(
            reader.next_fixture().err(),
            Some("compare_corpus_line_invalid")
        );
    }
    let (_dir, path) = write_corpus(&[]);
    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    assert!(reader.next_fixture().unwrap().is_none());
    assert_eq!(reader.finish().unwrap(), sha256_bytes(&[]));
}

#[test]
fn the_reader_digest_is_the_file_digest() {
    let lines = [
        fixture_line("digest_a", "low", prose_steps("one")),
        fixture_line("digest_b", "medium", prose_steps("two")),
        fixture_line("digest_c", "high", prose_steps("three")),
    ];
    let (_dir, path) = write_corpus(&lines);
    let expected = sha256_bytes(&std::fs::read(&path).unwrap());

    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    reader.next_fixture().unwrap().unwrap();
    assert_eq!(reader.finish().unwrap(), expected);

    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    while reader.next_fixture().unwrap().is_some() {}
    assert_eq!(reader.finish().unwrap(), expected);
}

#[tokio::test]
async fn the_same_fixture_gives_the_same_envelope_bytes() {
    let fixture = fixture_from(fixture_line("same_bytes", "low", tool_steps()));
    let first = compare_envelope(&fixture).await;
    let second = compare_envelope(&fixture).await;
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(first.trace_id, fixture.trace_id);
    assert_eq!(first.submission_id, fixture.submission_id);
    assert_eq!(first.created_at, fixture.created_at);
    assert!(first.events.len() > 1);
    assert_eq!(first.consent.scopes, vec![ConsentScope::ModelTraining]);
    assert_eq!(
        first.trace_card.allowed_uses,
        vec![TraceAllowedUse::ModelTraining]
    );
    for envelope in [&first, &second] {
        let call_position = envelope
            .events
            .iter()
            .position(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event");
        let call = &envelope.events[call_position];
        assert_eq!(
            call.event_id,
            Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("tracecommons:compare-event:same_bytes:{call_position}").as_bytes(),
            )
        );
        let result = envelope
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolResult)
            .expect("a tool result event");
        assert_eq!(result.parent_event_id, Some(call.event_id));
        for event in &envelope.events {
            assert_eq!(event.timestamp, fixture.created_at);
        }
    }
}

#[tokio::test]
async fn a_tool_session_keeps_its_tool_events() {
    let fixture = fixture_from(fixture_line("tool_events", "low", tool_steps()));
    let envelope = compare_envelope(&fixture).await;
    assert!(
        envelope
            .events
            .iter()
            .any(|event| event.event_type == TraceContributionEventType::ToolCall)
    );
    assert!(
        envelope
            .events
            .iter()
            .any(|event| event.event_type == TraceContributionEventType::ToolResult)
    );
}

#[tokio::test]
async fn a_declared_risk_gives_a_metadata_only_envelope() {
    for (risk, expected) in [
        ("medium", ResidualPiiRisk::Medium),
        ("high", ResidualPiiRisk::High),
    ] {
        let fixture = fixture_from(fixture_line("risky", risk, prose_steps("secret words")));
        let envelope = compare_envelope(&fixture).await;
        assert_eq!(envelope.privacy.residual_pii_risk, expected);
        assert!(!envelope.consent.message_text_included);
        assert!(
            envelope
                .events
                .iter()
                .all(|event| event.redacted_content.is_none())
        );
    }
}

#[test]
fn the_baseline_configuration_holds_mains_gate() {
    let gate = compare_main_gate(DerivedFloors {
        perplexity_floor_micros: 20,
        tail_fraction_floor_micros: 5,
        novelty_floor_micros: 8,
    });
    let config = baseline_orchestrator_config(&gate);
    assert_eq!(config.perplexity_floor_micros, 20);
    assert_eq!(config.tail_fraction_floor_micros, 5);
    assert_eq!(config.novelty_floor_micros, 8);
    assert_eq!(config.top_k, 8);
    assert_eq!(config.chunk_target_tokens, 2048);
    assert_eq!(config.chunk_max_tokens, 3072);
    assert_eq!(config.chunk_cap, 16);
    assert_eq!(config.chunk_min_tokens, 64);
    assert_eq!(config.embed_insert_novelty_micros, 50_000);
    // PC-D7: `main` defaults this to the perplexity floor.
    assert_eq!(config.qualifying_chunk_floor_micros, 20);
    let bundle = CompatibilityBundleConfig::production_compatible(
        "reference_perplexity.v1".into(),
        MINIMAL_PROJECTION_ID.into(),
        MINIMAL_INDEX_ID.into(),
        &gate,
    )
    .expect("the bundle accepts main's gate");
    assert!(bundle.matches_main_gate(&gate));
    assert_eq!(gate.novelty_utility_microcredits, 2_500_000);
}

#[tokio::test]
async fn calibration_is_deterministic_and_not_zero() {
    let lines: Vec<String> = ["alpha", "bravo", "charlie", "delta"]
        .iter()
        .map(|word| fixture_line(&format!("cal_{word}"), "low", prose_steps(&long_text(word))))
        .collect();
    let (_dir, path) = write_corpus(&lines);
    let first = calibrate_floors(&path).await.unwrap();
    let second = calibrate_floors(&path).await.unwrap();
    assert_eq!(first, second);
    assert!(first.perplexity_floor_micros > 0);

    let (_dir, empty) = write_corpus(&[]);
    assert_eq!(
        calibrate_floors(&empty).await.err(),
        Some("comparison_calibration_empty")
    );
}

#[test]
fn the_two_indexes_give_the_same_similarity() {
    let embedder = ReferenceEmbedder::new();
    let embed = |text: &str| embedder.embed(text.as_bytes()).unwrap();
    let stored: Vec<Vec<f32>> = [
        "the quick brown fox jumps over the lazy dog",
        "pack my box with five dozen liquor jugs",
        "how vexingly quick daft zebras jump",
        "sphinx of black quartz judge my vow",
        "the five boxing wizards jump quickly",
    ]
    .iter()
    .map(|text| embed(text))
    .collect();
    let queries: Vec<Vec<f32>> = [
        "a quick brown dog jumps over a fox",
        "five dozen jugs of liquor in a box",
        "something that shares nothing at all",
    ]
    .iter()
    .map(|text| embed(text))
    .collect();

    let tenant = TenantStorageRef::new("tenant_sha256:80a707af7dc77ee1228f9127180f3964").unwrap();
    let mock = MockVectorIndex::new();
    let isolated = IsolatedPipelineIndex::new();
    for (position, embedding) in stored.iter().enumerate() {
        mock.insert(
            Uuid::from_u128(position as u128),
            tenant.as_str(),
            embedding,
        )
        .unwrap();
        let key = IndexEntryKey {
            tenant_storage_ref: tenant.clone(),
            index_id: MINIMAL_INDEX_ID.to_string(),
            revision_id: Uuid::from_u128(position as u128),
            projection_id: MINIMAL_PROJECTION_ID.to_string(),
            model_id: "reference-embedder-v1".to_string(),
            chunk: 0,
        };
        isolated
            .upsert(&key, embedding, &format!("sha256:{position:064x}"))
            .unwrap();
    }
    for query in &queries {
        let from_mock = mock.nearest(tenant.as_str(), query, 8).unwrap();
        let from_isolated = VectorIndexReader::nearest(
            isolated.as_ref(),
            &tenant,
            MINIMAL_INDEX_ID,
            query,
            8,
            None,
        )
        .unwrap();
        let bits = |list: &[NearestNeighbor]| -> Vec<u32> {
            list.iter().map(|n| n.similarity.to_bits()).collect()
        };
        assert_eq!(from_mock.len(), 5);
        assert_eq!(bits(&from_mock)[0], bits(&from_isolated)[0]);
        assert_eq!(bits(&from_mock), bits(&from_isolated));
    }
}

/// One app for one test, with random tenant names (PC-D17), a temporary
/// artifact root, and a generated key. `None` without
/// `TRACE_COMMONS_PG_TEST_DATABASE_URL`: the test then skips.
async fn start_test_app(
    floors: DerivedFloors,
    accept_medium_risk: bool,
) -> Option<(CompareApp, tempfile::TempDir)> {
    std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").ok()?;
    let artifacts = tempfile::tempdir().expect("temp dir");
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let app = start_compare_app(
        compare_main_gate(floors),
        artifacts.path(),
        &key,
        CompareAppOptions {
            tenants: CompareTenants::random(),
            skew: None,
            accept_medium_risk,
        },
    )
    .await;
    Some((app, artifacts))
}

/// Floors of zero except a perplexity floor of 1: every scored trace passes.
const PASSING_FLOORS: DerivedFloors = DerivedFloors {
    perplexity_floor_micros: 1,
    tail_fraction_floor_micros: 0,
    novelty_floor_micros: 0,
};

/// The spike of Task 5: one trace reaches the old gate and the pipeline's
/// Score in one app, with the shared test helpers and no driver code.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_trace_reaches_both_gates() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    // 0. The candidate tenant's active bundle is this app's package.
    // `start_compare_app` asserts it and the disabled submission quota too.
    assert_eq!(
        active_bundle_id(&app).await.as_deref(),
        Some(app.package.bundle_id.as_str())
    );

    let fixture = fixture_from(fixture_line(
        "spike_one",
        "low",
        prose_steps(&long_text("spike")),
    ));
    let body = serde_json::to_vec(&compare_envelope(&fixture).await).unwrap();

    // 1. The baseline receipt stays on the old path.
    let (code, receipt) = post_trace(
        &app.client,
        &app.base,
        &app.tenants.baseline_contributor,
        &body,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(receipt["status"], "accepted");

    // 2. The old gate route scores it.
    let (code, _) = send_http(
        &app.client,
        reqwest::Method::POST,
        format!("{}/v1/workers/gate/evaluate", app.base),
        auth_headers(&app.tenants.baseline_gate),
        Some(serde_json::json!({ "submission_id": fixture.submission_id })),
    )
    .await;
    assert_eq!(code, 200);
    let mut client = app.owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &app.tenants.baseline).await;
    let baseline_perplexity: i64 = tx
        .query_one(
            "SELECT perplexity_micros FROM trace_gate_decisions
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&app.tenants.baseline, &fixture.submission_id],
        )
        .await
        .expect("one gate decision of the baseline")
        .get(0);
    tx.commit().await.unwrap();
    assert!(baseline_perplexity > 0);

    // 3. The candidate receipt goes to the pipeline, and the worker
    // completes the run.
    let (code, receipt) = post_trace(
        &app.client,
        &app.base,
        &app.tenants.candidate_contributor,
        &body,
    )
    .await;
    assert_eq!(code, 200);
    assert_eq!(receipt["status"], "processing");
    wait_for_run_state(
        &app.runtime,
        &app.tenants.candidate,
        fixture.submission_id,
        "complete",
    )
    .await;

    // 4. The Score evidence holds a measured perplexity.
    let tx = tenant_tx(&mut client, &app.tenants.candidate).await;
    let row = tx
        .query_one(
            "SELECT r.run_id, o.evidence FROM pipeline_runs r
               JOIN phase_outcomes o ON o.tenant_id = r.tenant_id AND o.run_id = r.run_id
              WHERE r.tenant_id = $1 AND r.submission_id = $2 AND o.phase = 'score'",
            &[&app.tenants.candidate, &fixture.submission_id],
        )
        .await
        .expect("the Score outcome of the candidate");
    tx.commit().await.unwrap();
    let run_id: Uuid = row.get(0);
    let evidence: ScoreEvidence =
        serde_json::from_value(row.get(1)).expect("the evidence is a ScoreEvidence");
    let candidate_perplexity = evidence.perplexity_micros.expect("a measured perplexity");

    // 5. The index command of the run loads.
    let run = app
        .service
        .store()
        .get_run(&app.tenants.candidate, run_id)
        .await
        .unwrap()
        .expect("the run exists");
    app.service
        .load_index_command(&run, &evidence)
        .await
        .expect("the index command loads");

    assert_eq!(u64::try_from(baseline_perplexity), Ok(candidate_perplexity));
    app.shutdown().await;
}

// ---------------------------------------------------------------------------
// Tests of the two side drivers.
// ---------------------------------------------------------------------------

/// A low-risk fixture with about 220 words of text that no other label has.
fn prose_fixture(label: &str) -> CompareFixture {
    risk_fixture(label, "low")
}

fn risk_fixture(label: &str, risk: &str) -> CompareFixture {
    fixture_from(fixture_line(label, risk, prose_steps(&long_text(label))))
}

fn trace_hash_of(fixture: &CompareFixture) -> String {
    sha256_bytes(fixture.trace_id.to_string().as_bytes())
}

fn record_base(fixture: &CompareFixture, position: u64) -> RecordBase {
    RecordBase {
        position,
        partition: "holdout",
        trace_hash: trace_hash_of(fixture),
    }
}

/// The two records of one trace, and what the drivers did for them.
struct Pair {
    action: AlignmentAction,
    baseline: ComparisonRecord,
    candidate: ComparisonRecord,
    /// The HTTP calls of the two sides for this trace.
    http_calls: u64,
    /// The `Probes::check` calls for this trace.
    probe_checks: u64,
}

/// Drives one fixture through the two sides as the run does: the two
/// receipts with the same body bytes, the alignment action, and the two
/// finish steps in the order that the action needs.
async fn drive_pair(app: &CompareApp, fixture: &CompareFixture, position: u64) -> Pair {
    let envelope = compare_envelope(fixture).await;
    let body = serde_json::to_vec(&envelope).unwrap();
    let probes = Probes::new(&app.tenants, fixture, &envelope);
    let calls_before = app.http_call_count();
    let baseline_seen = baseline_admission(app, &probes, &body, fixture.submission_id)
        .await
        .unwrap();
    let candidate_seen = candidate_admission(app, &probes, &body, fixture.submission_id)
        .await
        .unwrap();
    let action = alignment_action(
        baseline_seen.admission,
        candidate_seen.admission,
        &trace_hash_of(fixture),
    );
    // The candidate is first when its review can fail: the baseline index
    // then does not get a trace that the candidate index does not get.
    let candidate_first = matches!(
        action,
        AlignmentAction::ApproveCandidate | AlignmentAction::HashRuleBoth(ReviewLabel::Approve)
    );
    let finish_baseline = || {
        let base = record_base(fixture, position);
        baseline_finish(app, &probes, fixture, baseline_seen.clone(), action, base)
    };
    let finish_candidate = || {
        let base = record_base(fixture, position);
        candidate_finish(app, &probes, fixture, candidate_seen.clone(), action, base)
    };
    let (baseline, candidate) = if candidate_first {
        let candidate = finish_candidate().await.unwrap();
        (finish_baseline().await.unwrap(), candidate)
    } else {
        let baseline = finish_baseline().await.unwrap();
        (baseline, finish_candidate().await.unwrap())
    };
    Pair {
        action,
        baseline,
        candidate,
        http_calls: app.http_call_count() - calls_before,
        probe_checks: probes.check_count(),
    }
}

/// The number of `.json` files in the `derived/` directory of `tenant`.
fn derived_file_count(app: &CompareApp, tenant: &str) -> usize {
    let dir = app
        .root
        .join("tenants")
        .join(tenant_storage_key(tenant))
        .join("derived");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .count()
}

// baseline-old-path:
async fn gate_decision_count(app: &CompareApp, submission_id: Uuid) -> i64 {
    let mut client = app.owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, &app.tenants.baseline).await;
    let count = tx
        .query_one(
            "SELECT COUNT(*) FROM trace_gate_decisions
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&app.tenants.baseline, &submission_id],
        )
        .await
        .unwrap()
        .get(0);
    tx.commit().await.unwrap();
    count
}

fn novelty_credit() -> Vec<CreditEvent> {
    vec![CreditEvent {
        event_type: "novelty_utility".to_string(),
        microcredits: 2_500_000,
    }]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_admitted_trace_gives_two_full_records() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let pair = drive_pair(&app, &prose_fixture("admitted_one"), 0).await;
    assert_eq!(pair.action, AlignmentAction::None);
    for record in [&pair.baseline, &pair.candidate] {
        assert_eq!(record.receipt_code, 200);
        assert!(record.terminal);
        assert_eq!(record.admission, AdmissionLabel::Admit);
        assert_eq!(record.review, ReviewLabel::None);
        assert_eq!(record.review_source, ReviewSource::None);
        assert!(!record.gate_skipped_by_alignment);
        assert!(record.scored);
        assert!(record.gate.is_some());
    }
    assert_eq!(pair.baseline.side, ComparisonSide::Baseline);
    assert_eq!(pair.candidate.side, ComparisonSide::Candidate);
    assert_eq!(
        compare_records(&pair.baseline, &pair.candidate),
        TraceComparison::Equal
    );
    // The efficiency lens: two receipts and one gate call, and no more.
    assert_eq!(pair.http_calls, 3);
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn privacy_fields_come_from_the_submission_row() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let pair = drive_pair(&app, &prose_fixture("privacy_one"), 0).await;
    // The server rescrub classifies message text as medium risk with the
    // consent flag as its only basis. These are the stored spellings.
    for record in [&pair.baseline, &pair.candidate] {
        assert_eq!(record.privacy_risk.as_deref(), Some("medium"));
        assert_eq!(record.privacy_basis, ["consent_content_flag"]);
    }
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_declared_medium_trace_is_aligned() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let pair = drive_pair(&app, &risk_fixture("declared_medium", "medium"), 0).await;
    assert_eq!(pair.action, AlignmentAction::ApproveCandidate);
    assert_eq!(pair.baseline.admission, AdmissionLabel::Admit);
    assert_eq!(pair.baseline.review, ReviewLabel::None);
    assert_eq!(pair.candidate.admission, AdmissionLabel::Quarantine);
    assert_eq!(pair.candidate.review, ReviewLabel::Approve);
    assert_eq!(pair.candidate.review_source, ReviewSource::Alignment);
    assert!(pair.candidate.terminal);
    assert!(pair.candidate.scored);
    // The admission is the one difference: each gate value, the membership,
    // and the credit event are equal after the alignment.
    assert_eq!(
        compare_records(&pair.baseline, &pair.candidate),
        TraceComparison::Unexplained {
            fields: vec!["admission"]
        }
    );
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_declared_high_trace_is_aligned() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let pair = drive_pair(&app, &risk_fixture("declared_high", "high"), 0).await;
    assert_eq!(pair.action, AlignmentAction::RejectBaseline);
    assert_eq!(pair.baseline.admission, AdmissionLabel::Quarantine);
    assert_eq!(pair.baseline.review, ReviewLabel::Reject);
    assert_eq!(pair.baseline.review_source, ReviewSource::Alignment);
    assert!(pair.baseline.terminal);
    assert!(!pair.baseline.scored);
    assert_eq!(pair.candidate.admission, AdmissionLabel::Reject);
    assert_eq!(pair.candidate.review, ReviewLabel::None);
    assert!(pair.candidate.terminal);
    assert!(!pair.candidate.scored);
    // The two sides are not scored, so the admission is the one difference.
    assert_eq!(
        compare_records(&pair.baseline, &pair.candidate),
        TraceComparison::Unexplained {
            fields: vec!["admission"]
        }
    );
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_gate_gives_no_membership_and_no_credit() {
    let floors = DerivedFloors {
        perplexity_floor_micros: u64::MAX,
        tail_fraction_floor_micros: 0,
        novelty_floor_micros: 0,
    };
    let Some((app, _artifacts)) = start_test_app(floors, true).await else {
        return;
    };
    let pair = drive_pair(&app, &prose_fixture("failed_gate"), 0).await;
    for record in [&pair.baseline, &pair.candidate] {
        assert!(record.terminal);
        let gate = record.gate.as_ref().expect("the trace is scored");
        assert!(!gate.quality_passed);
        assert!(!record.member);
        assert!(record.member_chunks.is_empty());
        assert!(record.credit_events.is_empty());
    }
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_passed_gate_gives_membership_and_one_credit_event() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let pair = drive_pair(&app, &prose_fixture("passed_gate"), 0).await;
    for record in [&pair.baseline, &pair.candidate] {
        assert!(record.terminal);
        let gate = record.gate.as_ref().expect("the trace is scored");
        assert!(gate.quality_passed && gate.novelty_passed);
        assert!(record.member);
        assert!(!record.member_chunks.is_empty());
        assert_eq!(record.credit_events, novelty_credit());
    }
    assert_eq!(pair.baseline.member_chunks, pair.candidate.member_chunks);
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_second_trace_sees_the_first_in_both_indexes() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let first = drive_pair(&app, &prose_fixture("index_first"), 0).await;
    assert!(first.baseline.member && first.candidate.member);
    let second = drive_pair(&app, &prose_fixture("index_second"), 1).await;
    let cardinality = |record: &ComparisonRecord| {
        record
            .gate
            .as_ref()
            .expect("the trace is scored")
            .index_cardinality
    };
    let seen = cardinality(&second.baseline).expect("the baseline records a cardinality");
    assert!(seen > 0);
    assert_eq!(cardinality(&second.candidate), Some(seen));
    app.shutdown().await;
}

/// Review Focus 3. Each received body and each record of the two traces went
/// through `Probes::check`, with the five tokens, the `secret_probe`, and the
/// content probe. The two traces use each route of the two drivers.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_probe_reaches_a_body_or_a_record() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, false).await else {
        return;
    };
    // (Quarantine, Admit): the old review routes and the old gate route.
    let prose = prose_fixture("probe_prose");
    let envelope = compare_envelope(&prose).await;
    let probes = Probes::new(&app.tenants, &prose, &envelope);
    assert_eq!(probes.tokens.len(), 5);
    assert_eq!(probes.secret_probe, prose.secret_probe);
    // The text of this fixture has no character that JSON escapes, so the
    // probe has one form.
    assert_eq!(probes.content.len(), 1);
    assert_eq!(probes.content[0].len(), 48);
    assert!(long_text("probe_prose").starts_with(&probes.content[0]));
    let pair = drive_pair(&app, &prose, 0).await;
    assert_eq!(pair.action, AlignmentAction::ApproveBaseline);
    assert_eq!(pair.http_calls, 5);
    assert_eq!(pair.probe_checks, pair.http_calls + 2);

    // (Quarantine, Quarantine) with an approval: the pipeline review routes.
    let approved = hash_rule_fixture("probe_review", ReviewLabel::Approve);
    let pair = drive_pair(&app, &approved, 1).await;
    assert_eq!(
        pair.action,
        AlignmentAction::HashRuleBoth(ReviewLabel::Approve)
    );
    assert!(pair.http_calls >= 8, "{}", pair.http_calls);
    assert_eq!(pair.probe_checks, pair.http_calls + 2);
    app.shutdown().await;
}

/// The text of a panic of `call`, or `None` when it returns.
fn panic_text(call: impl FnOnce()) -> Option<String> {
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).err()?;
    Some(match payload.downcast::<String>() {
        Ok(text) => *text,
        Err(payload) => payload
            .downcast::<&'static str>()
            .map(|text| text.to_string())
            .unwrap_or_default(),
    })
}

#[tokio::test]
async fn a_token_in_checked_bytes_panics_with_the_label() {
    let fixture = prose_fixture("probe_token");
    let tenants = CompareTenants::run();
    let probes = Probes::new(&tenants, &fixture, &compare_envelope(&fixture).await);
    assert_eq!(
        tenants.tokens(),
        [
            "token-compare-baseline-contributor",
            "token-compare-baseline-reviewer",
            "token-compare-baseline-gate",
            "token-compare-candidate-contributor",
            "token-compare-candidate-reviewer",
        ]
    );
    assert_eq!(tenants.baseline, "tenant-compare-baseline");
    assert_eq!(tenants.candidate, "tenant-compare-candidate");
    for token in tenants.tokens() {
        let bytes = format!("{{\"error\":\"bad credential {token}\"}}");
        assert_eq!(
            panic_text(|| probes.check(bytes.as_bytes(), "compare_probe_in_response")).as_deref(),
            Some("compare_probe_in_response")
        );
        assert_eq!(
            panic_text(|| probes.check_sent(bytes.as_bytes(), "compare_probe_in_request"))
                .as_deref(),
            Some("compare_probe_in_request")
        );
    }
    assert_eq!(
        panic_text(|| probes.check(br#"{"status":"accepted"}"#, "compare_probe_in_response")),
        None
    );
    // One call of `check` was counted for each of the six calls above.
    assert_eq!(probes.check_count(), 6);
}

#[tokio::test]
async fn the_content_probe_in_checked_bytes_panics_with_the_label() {
    // The first 48 bytes of this text hold a quotation mark and end inside a
    // character of two bytes.
    let text = format!(
        "Tell me why \"{}\u{e9}\" is the answer, with all details.",
        "x".repeat(34)
    );
    assert!(!text.is_char_boundary(48));
    let fixture = fixture_from(fixture_line("probe_content", "low", prose_steps(&text)));
    let envelope = compare_envelope(&fixture).await;
    let tenants = CompareTenants::run();
    let probes = Probes::new(&tenants, &fixture, &envelope);
    let window = &text[..47];
    let escaped = window.replace('"', "\\\"");
    assert_eq!(probes.content, [window.to_string(), escaped.clone()]);
    for leak in [
        format!("the stored text was: {window} and more"),
        format!("{{\"error\":\"{escaped}\"}}"),
        format!("{{\"probe\":\"{}\"}}", fixture.secret_probe),
    ] {
        assert_eq!(
            panic_text(|| probes.check(leak.as_bytes(), "compare_probe_in_record")).as_deref(),
            Some("compare_probe_in_record")
        );
        // A sent body holds the trace text by design.
        assert_eq!(
            panic_text(|| probes.check_sent(leak.as_bytes(), "compare_probe_in_request")),
            None
        );
    }

    // A text of fewer than 48 bytes gives no content probe, and a
    // metadata-only envelope has no text.
    let short = fixture_from(fixture_line("probe_short", "low", prose_steps("short")));
    assert!(
        Probes::new(&tenants, &short, &compare_envelope(&short).await)
            .content
            .is_empty()
    );
    let declared = risk_fixture("probe_declared", "medium");
    assert!(
        Probes::new(&tenants, &declared, &compare_envelope(&declared).await)
            .content
            .is_empty()
    );
}

#[test]
fn the_skew_changes_only_the_baseline_quality_floor() {
    let gate = compare_main_gate(DerivedFloors {
        perplexity_floor_micros: 20,
        tail_fraction_floor_micros: 5,
        novelty_floor_micros: 8,
    });
    let plain = compare_baseline_config(&gate, None);
    assert_eq!(plain.gate_policy_version, "compare_gate_v1");
    assert_eq!(plain.perplexity_floor_micros, 20);
    let skewed = compare_baseline_config(&gate, Some("baseline_quality_floor"));
    assert_eq!(skewed.perplexity_floor_micros, u64::MAX);
    assert_eq!(
        (
            skewed.tail_fraction_floor_micros,
            skewed.novelty_floor_micros,
            skewed.qualifying_chunk_floor_micros,
        ),
        (5, 8, 20)
    );
    assert_eq!(
        panic_text(|| {
            compare_baseline_config(&gate, Some("candidate_quality_floor"));
        })
        .as_deref(),
        Some("compare_skew_invalid")
    );
}

/// PC-D16. The default limit is 30 receipts in one window for one token.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn thirty_five_receipts_pass_the_rate_limit() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    for number in 0..35 {
        let fixture = fixture_from(fixture_line(
            &format!("rate_{number}"),
            "low",
            prose_steps(&format!("receipt number {number}")),
        ));
        let envelope = compare_envelope(&fixture).await;
        let body = serde_json::to_vec(&envelope).unwrap();
        let probes = Probes::new(&app.tenants, &fixture, &envelope);
        let baseline = baseline_admission(&app, &probes, &body, fixture.submission_id)
            .await
            .unwrap();
        assert_eq!(
            baseline.receipt_code,
            200,
            "baseline receipt {}",
            number + 1
        );
        let candidate = candidate_admission(&app, &probes, &body, fixture.submission_id)
            .await
            .unwrap();
        assert_eq!(
            candidate.receipt_code,
            200,
            "candidate receipt {}",
            number + 1
        );
    }
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn twenty_quarantined_traces_all_complete() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    for number in 0..20 {
        let fixture = risk_fixture(&format!("quarantined_{number}"), "medium");
        let pair = drive_pair(&app, &fixture, number).await;
        assert_eq!(
            pair.action,
            AlignmentAction::ApproveCandidate,
            "trace {number}"
        );
        assert_eq!(
            pair.candidate.review,
            ReviewLabel::Approve,
            "trace {number}"
        );
        assert!(pair.candidate.terminal, "trace {number}");
        assert!(pair.baseline.terminal, "trace {number}");
        assert_eq!(
            compare_records(&pair.baseline, &pair.candidate),
            TraceComparison::Unexplained {
                fields: vec!["admission"]
            },
            "trace {number}"
        );
    }
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approve_baseline_is_driven() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, false).await else {
        return;
    };
    let pair = drive_pair(&app, &prose_fixture("approve_baseline"), 0).await;
    assert_eq!(pair.action, AlignmentAction::ApproveBaseline);
    assert_eq!(pair.baseline.admission, AdmissionLabel::Quarantine);
    assert_eq!(pair.baseline.review, ReviewLabel::Approve);
    assert_eq!(pair.baseline.review_source, ReviewSource::Alignment);
    assert_eq!(pair.candidate.admission, AdmissionLabel::Admit);
    assert_eq!(pair.candidate.review, ReviewLabel::None);
    assert!(pair.baseline.terminal && pair.candidate.terminal);
    assert!(pair.baseline.scored && pair.candidate.scored);
    assert_eq!(
        compare_records(&pair.baseline, &pair.candidate),
        TraceComparison::Unexplained {
            fields: vec!["admission"]
        }
    );
    app.shutdown().await;
}

/// A fixture with a declared medium risk for which `hash_rule` gives
/// `decision`. The label is `<prefix>_<number>` with the first number that
/// has that decision.
fn hash_rule_fixture(prefix: &str, decision: ReviewLabel) -> CompareFixture {
    (0..64)
        .map(|number| risk_fixture(&format!("{prefix}_{number}"), "medium"))
        .find(|fixture| hash_rule(&trace_hash_of(fixture)) == decision)
        .expect("one of 64 labels has each decision")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_hash_rule_is_driven_on_both_sides() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, false).await else {
        return;
    };
    let even = hash_rule_fixture("hash_even", ReviewLabel::Approve);
    let pair = drive_pair(&app, &even, 0).await;
    assert_eq!(
        pair.action,
        AlignmentAction::HashRuleBoth(ReviewLabel::Approve)
    );
    for record in [&pair.baseline, &pair.candidate] {
        assert_eq!(record.admission, AdmissionLabel::Quarantine);
        assert_eq!(record.review, ReviewLabel::Approve);
        assert_eq!(record.review_source, ReviewSource::HashRule);
        assert!(record.terminal);
        assert!(record.scored);
    }
    assert_eq!(
        compare_records(&pair.baseline, &pair.candidate),
        TraceComparison::Equal
    );

    let odd = hash_rule_fixture("hash_odd", ReviewLabel::Reject);
    let pair = drive_pair(&app, &odd, 1).await;
    assert_eq!(
        pair.action,
        AlignmentAction::HashRuleBoth(ReviewLabel::Reject)
    );
    for record in [&pair.baseline, &pair.candidate] {
        assert_eq!(record.admission, AdmissionLabel::Quarantine);
        assert_eq!(record.review, ReviewLabel::Reject);
        assert_eq!(record.review_source, ReviewSource::HashRule);
        assert!(record.terminal);
        assert!(!record.scored);
    }
    assert_eq!(
        compare_records(&pair.baseline, &pair.candidate),
        TraceComparison::Equal
    );
    app.shutdown().await;
}

/// Review Focus 2: a trace that needed an alignment action is a member of
/// the two indexes, and the next trace sees the same index on the two sides.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_aligned_member_keeps_the_indexes_equal() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, false).await else {
        return;
    };
    let first = drive_pair(&app, &prose_fixture("aligned_first"), 0).await;
    assert_eq!(first.action, AlignmentAction::ApproveBaseline);
    assert!(first.baseline.member && first.candidate.member);
    let second = drive_pair(&app, &prose_fixture("aligned_second"), 1).await;
    let cardinality = |record: &ComparisonRecord| {
        record
            .gate
            .as_ref()
            .expect("the trace is scored")
            .index_cardinality
    };
    let seen = cardinality(&second.baseline).expect("the baseline records a cardinality");
    assert!(seen > 0);
    assert_eq!(cardinality(&second.candidate), Some(seen));
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skip_baseline_gate_and_stop_are_driven() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    // `SkipBaselineGate` for a trace that the two sides accepted.
    let fixture = prose_fixture("skip_gate");
    let envelope = compare_envelope(&fixture).await;
    let body = serde_json::to_vec(&envelope).unwrap();
    let probes = Probes::new(&app.tenants, &fixture, &envelope);
    let baseline_seen = baseline_admission(&app, &probes, &body, fixture.submission_id)
        .await
        .unwrap();
    let candidate_seen = candidate_admission(&app, &probes, &body, fixture.submission_id)
        .await
        .unwrap();
    assert_eq!(baseline_seen.admission, AdmissionLabel::Admit);
    assert_eq!(candidate_seen.admission, AdmissionLabel::Admit);
    let calls_before = app.http_call_count();
    let baseline = baseline_finish(
        &app,
        &probes,
        &fixture,
        baseline_seen,
        AlignmentAction::SkipBaselineGate,
        record_base(&fixture, 0),
    )
    .await
    .unwrap();
    let candidate = candidate_finish(
        &app,
        &probes,
        &fixture,
        candidate_seen,
        AlignmentAction::SkipBaselineGate,
        record_base(&fixture, 0),
    )
    .await
    .unwrap();
    assert!(baseline.terminal);
    assert!(!baseline.scored);
    assert!(baseline.gate.is_none());
    assert!(baseline.gate_skipped_by_alignment);
    assert!(!baseline.member);
    assert_eq!(gate_decision_count(&app, fixture.submission_id).await, 0);
    assert!(candidate.terminal);
    assert!(candidate.scored);
    assert!(!candidate.gate_skipped_by_alignment);
    assert_eq!(app.http_call_count(), calls_before);

    // `Stop` for a trace that the two sides refused (PC-D19): no call, no
    // poll, and no wait.
    let refused = prose_fixture("stop_refused");
    let probes = Probes::new(&app.tenants, &refused, &compare_envelope(&refused).await);
    let seen = AdmissionObservation {
        receipt_code: 429,
        privacy_risk: None,
        privacy_basis: Vec::new(),
        admission: AdmissionLabel::Refused,
    };
    let started = std::time::Instant::now();
    let baseline = baseline_finish(
        &app,
        &probes,
        &refused,
        seen.clone(),
        AlignmentAction::Stop,
        record_base(&refused, 1),
    )
    .await
    .unwrap();
    let candidate = candidate_finish(
        &app,
        &probes,
        &refused,
        seen,
        AlignmentAction::Stop,
        record_base(&refused, 1),
    )
    .await
    .unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    assert_eq!(app.http_call_count(), calls_before);
    for record in [&baseline, &candidate] {
        assert_eq!(record.receipt_code, 429);
        assert!(!record.terminal);
        assert!(!record.scored);
    }
    assert_eq!(
        compare_records(&baseline, &candidate),
        TraceComparison::Unexplained {
            fields: vec!["terminal"]
        }
    );
    app.shutdown().await;
}

/// Each gate call adds the trace to the baseline index again. A driver that
/// sends a second call for one trace must not give a record.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_second_gate_decision_is_refused() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let fixture = prose_fixture("scored_twice");
    let pair = drive_pair(&app, &fixture, 0).await;
    assert!(pair.baseline.scored);
    assert_eq!(gate_decision_count(&app, fixture.submission_id).await, 1);
    let probes = Probes::new(&app.tenants, &fixture, &compare_envelope(&fixture).await);
    let seen = AdmissionObservation {
        receipt_code: 200,
        privacy_risk: pair.baseline.privacy_risk.clone(),
        privacy_basis: pair.baseline.privacy_basis.clone(),
        admission: AdmissionLabel::Admit,
    };
    let again = baseline_finish(
        &app,
        &probes,
        &fixture,
        seen,
        AlignmentAction::None,
        record_base(&fixture, 0),
    )
    .await;
    assert_eq!(again.err(), Some("compare_baseline_scored_twice"));
    assert_eq!(gate_decision_count(&app, fixture.submission_id).await, 2);
    app.shutdown().await;
}

/// PC-D20. A review call that the old path refuses gives a baseline that is
/// not terminal, and no gate call follows it. A review action for a run that
/// the worker completed gives no candidate record, and it does not wait for
/// the bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_review_call_is_not_terminal() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let fixture = prose_fixture("review_refused");
    let envelope = compare_envelope(&fixture).await;
    let body = serde_json::to_vec(&envelope).unwrap();
    let probes = Probes::new(&app.tenants, &fixture, &envelope);
    let baseline_seen = baseline_admission(&app, &probes, &body, fixture.submission_id)
        .await
        .unwrap();
    let candidate_seen = candidate_admission(&app, &probes, &body, fixture.submission_id)
        .await
        .unwrap();
    assert_eq!(baseline_seen.admission, AdmissionLabel::Admit);
    assert_eq!(candidate_seen.admission, AdmissionLabel::Admit);

    // The old path refuses a review lease for a trace that it accepted.
    let calls_before = app.http_call_count();
    let baseline = baseline_finish(
        &app,
        &probes,
        &fixture,
        baseline_seen,
        AlignmentAction::ApproveBaseline,
        record_base(&fixture, 0),
    )
    .await
    .unwrap();
    assert_eq!(app.http_call_count(), calls_before + 1);
    assert!(!baseline.terminal);
    assert!(!baseline.scored);
    assert_eq!(baseline.review, ReviewLabel::Approve);
    assert_eq!(gate_decision_count(&app, fixture.submission_id).await, 0);

    // The worker completes an admitted run, so it never waits for a review.
    let started = std::time::Instant::now();
    let candidate = candidate_finish(
        &app,
        &probes,
        &fixture,
        candidate_seen,
        AlignmentAction::ApproveCandidate,
        record_base(&fixture, 0),
    )
    .await;
    assert_eq!(candidate.err(), Some("compare_candidate_review_failed"));
    assert!(started.elapsed() < std::time::Duration::from_secs(30));
    app.shutdown().await;
}

#[test]
fn a_score_outcome_with_a_missing_field_is_refused() {
    let awards = trace_commons_gate_api::pipeline::InstrumentAwards::new(Vec::new()).unwrap();
    let mut evidence = ScoreEvidence::fixed(awards);
    evidence.quality_passed = Some(true);
    evidence.novelty_passed = Some(false);
    evidence.perplexity_micros = Some(11);
    evidence.tail_fraction_micros = Some(12);
    evidence.peak_perplexity_micros = Some(13);
    evidence.novelty_score_micros = Some(14);
    evidence.peak_novelty_micros = Some(15);
    evidence.chunk_count = Some(2);
    evidence.total_chunk_count = Some(3);
    evidence.chunks_capped = Some(true);
    assert_eq!(
        candidate_gate_values(&evidence),
        Ok(GateValues {
            quality_passed: true,
            novelty_passed: false,
            perplexity_micros: 11,
            tail_fraction_micros: 12,
            peak_perplexity_micros: 13,
            novelty_score_micros: 14,
            peak_novelty_micros: 15,
            chunk_count: 2,
            total_chunk_count: 3,
            chunks_capped: true,
            index_cardinality: None,
            credit_quality_micros: None,
            credit_quality_version: None,
        })
    );
    let without: [fn(&mut ScoreEvidence); 10] = [
        |evidence| evidence.quality_passed = None,
        |evidence| evidence.novelty_passed = None,
        |evidence| evidence.perplexity_micros = None,
        |evidence| evidence.tail_fraction_micros = None,
        |evidence| evidence.peak_perplexity_micros = None,
        |evidence| evidence.novelty_score_micros = None,
        |evidence| evidence.peak_novelty_micros = None,
        |evidence| evidence.chunk_count = None,
        |evidence| evidence.total_chunk_count = None,
        |evidence| evidence.chunks_capped = None,
    ];
    for remove in without {
        let mut incomplete = evidence.clone();
        remove(&mut incomplete);
        assert_eq!(
            candidate_gate_values(&incomplete),
            Err("compare_candidate_evidence_incomplete")
        );
    }
    // A credit quality that `i64` cannot hold is refused too.
    evidence.credit_quality_micros = Some(u64::MAX);
    assert_eq!(
        candidate_gate_values(&evidence),
        Err("compare_candidate_evidence_incomplete")
    );
}

/// PC-D18. The baseline receipt scans the tenant's derived files, and the
/// candidate tenant has none. The run removes the baseline's files after each
/// trace. This test proves that no record changes when it does.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn removing_the_derived_files_changes_no_record() {
    // The third trace has the text of the second and another label: another
    // `submission_id` and the same canonical summary.
    let fixtures = [
        prose_fixture("derived_one"),
        prose_fixture("derived_two"),
        fixture_from(fixture_line(
            "derived_three",
            "low",
            prose_steps(&long_text("derived_two")),
        )),
    ];
    let mut records: Vec<Vec<Vec<u8>>> = Vec::new();
    for clears in [true, false] {
        let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
            return;
        };
        let mut bytes = Vec::new();
        for (position, fixture) in fixtures.iter().enumerate() {
            let pair = drive_pair(&app, fixture, position as u64).await;
            assert!(pair.baseline.terminal && pair.candidate.terminal);
            bytes.push(record_bytes(&pair.baseline));
            bytes.push(record_bytes(&pair.candidate));
            if clears {
                // (b) Each call removes the one file of this trace, and no
                // other file of the state directory.
                let files_before = count_files_under_dir(&app.root);
                assert_eq!(clear_baseline_derived(&app), Ok(1));
                assert_eq!(derived_file_count(&app, &app.tenants.baseline), 0);
                assert_eq!(count_files_under_dir(&app.root), files_before - 1);
            }
            // (d) A candidate receipt sees an empty directory.
            assert_eq!(derived_file_count(&app, &app.tenants.candidate), 0);
        }
        if clears {
            assert_eq!(clear_baseline_derived(&app), Ok(0));
        } else {
            // (a) The scan found the second trace as a neighbor of the third.
            let derived = read_all_derived_records(&app.root, &app.tenants.baseline).unwrap();
            assert_eq!(derived.len(), 3);
            let third = derived
                .iter()
                .find(|record| record.submission_id == fixtures[2].submission_id)
                .expect("the derived record of the third trace");
            assert!(third.duplicate_score > 0.0);
        }
        records.push(bytes);
        app.shutdown().await;
    }
    // (c) The six records are equal with and without the removal.
    assert_eq!(records[0].len(), 6);
    assert_eq!(records[0], records[1]);
}

fn prose_steps(text: &str) -> Vec<TraceStep> {
    vec![
        TraceStep {
            request_hint: None,
            response: TraceResponse::UserInput {
                content: text.to_string(),
            },
            expected_tool_results: vec![],
            timestamp: None,
        },
        TraceStep {
            request_hint: None,
            response: TraceResponse::Text {
                content: format!("Answer to {text}"),
                input_tokens: 1,
                output_tokens: 1,
            },
            expected_tool_results: vec![],
            timestamp: None,
        },
    ]
}

fn tool_steps() -> Vec<TraceStep> {
    let mut steps = prose_steps("look up the weather");
    steps.push(TraceStep {
        request_hint: None,
        response: TraceResponse::ToolCalls {
            tool_calls: vec![TraceToolCall {
                id: "call_1".to_string(),
                name: "weather".to_string(),
                arguments: serde_json::json!({"city": "Lisbon"}),
            }],
            input_tokens: 1,
            output_tokens: 1,
        },
        expected_tool_results: vec![],
        timestamp: None,
    });
    steps.push(TraceStep {
        request_hint: None,
        response: TraceResponse::Text {
            content: "It is sunny.".to_string(),
            input_tokens: 1,
            output_tokens: 1,
        },
        expected_tool_results: vec![ExpectedToolResult {
            tool_call_id: "call_1".to_string(),
            name: "weather".to_string(),
            content: "sunny".to_string(),
        }],
        timestamp: None,
    });
    steps
}

fn long_text(seed: &str) -> String {
    (0..220)
        .map(|index| format!("{seed}{}", (index * 7 + seed.len()) % 61))
        .collect::<Vec<_>>()
        .join(" ")
}

fn fixture_line(label: &str, risk: &str, steps: Vec<TraceStep>) -> String {
    let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, label.as_bytes());
    serde_json::json!({
        "label": label,
        "trace_id": id,
        "submission_id": Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("submission:{label}").as_bytes()),
        "created_at": "2026-01-01T00:00:00Z",
        "secret_probe": format!("compare_probe_{label}"),
        "privacy_risk": risk,
        "trace_file": {
            "model_name": "compare-test",
            "memory_snapshot": [],
            "http_exchanges": [],
            "steps": steps,
        },
    })
    .to_string()
}

fn fixture_from(line: String) -> CompareFixture {
    serde_json::from_str(&line).unwrap()
}

fn write_corpus(lines: &[String]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compare.jsonl");
    let mut text = lines.join("\n");
    if !lines.is_empty() {
        text.push('\n');
    }
    std::fs::write(&path, text).unwrap();
    (dir, path)
}
