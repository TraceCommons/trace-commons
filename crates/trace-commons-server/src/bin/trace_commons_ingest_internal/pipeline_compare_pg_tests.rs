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
//! `pipeline_compare_run` is the run. It takes the traces in serial order,
//! one after the other, because the two indexes must hold the same entries
//! before each trace. It writes two records for each trace to the records
//! file, and it writes the report before it fails for a difference.
//!
//! Code marked `// baseline-old-path:` applies to the old path only and goes
//! away with it. Nested inside `tests` beside `pipeline_corpus_pg_tests`.

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
    AdmissionLabel, AlignmentAction, COMPARISON_ALIGNMENT_LABEL, COMPARISON_BRANCH_LABEL,
    COMPARISON_REFUSED_LABEL, COMPARISON_UNEXPLAINED_LABEL, ComparisonRecord,
    ComparisonReportInput, ComparisonSide, ComparisonSummary, CreditEvent, DerivedFloors,
    GateValues, ReviewLabel, ReviewSource, TraceComparison, alignment_action, alignment_lost,
    compare_records, comparison_report, derive_floors, hash_rule,
};
use trace_commons_server::versioned_pipeline_compat::{CompatibilityBundleConfig, MainGateConfig};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapterRegistry,
};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_product::PipelineProductStore;
use trace_commons_server::versioned_pipeline_qualification::{PipelineCheckEmitter, is_safe_label};

use super::pipeline_corpus_pg_tests::{
    ARTIFACT_ROOT_VAR, CHECK_RESULT_DIR_VAR, TEST_MASTER_KEY_VAR, sha256_bytes, write_atomically,
};
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
    /// The number of lines that were read.
    lines: u64,
    digest: sha2::Sha256,
    line: Vec<u8>,
}

impl CompareCorpusReader {
    fn open(partition: &'static str, path: &Path) -> Result<Self, &'static str> {
        let file = std::fs::File::open(path).map_err(|_| "compare_corpus_read_failed")?;
        Ok(Self {
            reader: BufReader::new(file),
            partition,
            lines: 0,
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
        if read > 0 {
            self.lines += 1;
        }
        Ok(read > 0)
    }

    fn next_fixture(&mut self) -> Result<Option<CompareFixture>, &'static str> {
        if !self.read_line()? {
            return Ok(None);
        }
        match serde_json::from_slice::<CompareFixture>(&self.line) {
            Ok(fixture) if fixture.is_valid() => Ok(Some(fixture)),
            _ => {
                // The line number in this file, from 1. It is not the
                // `position` of a record, which counts through the two
                // partitions.
                eprintln!(
                    "compare_corpus_line_invalid partition={} line={}",
                    self.partition, self.lines
                );
                Err("compare_corpus_line_invalid")
            }
        }
    }

    /// The digest of the whole file; reads the lines that are left.
    fn finish(self) -> Result<String, &'static str> {
        self.finish_with_lines().map(|(digest, _)| digest)
    }

    /// The digest of the whole file and the number of its lines; reads the
    /// lines that are left.
    fn finish_with_lines(mut self) -> Result<(String, u64), &'static str> {
        while self.read_line()? {}
        Ok((
            format!("sha256:{}", hex::encode(self.digest.finalize())),
            self.lines,
        ))
    }
}

// ---------------------------------------------------------------------------
// The envelope.
// ---------------------------------------------------------------------------

/// The contribution of one fixture before the redaction. Each field that the
/// protocol crate fills at random or from the clock is a function of the
/// fixture here.
fn compare_raw(fixture: &CompareFixture) -> RawTraceContribution {
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
    raw
}

/// The envelope of one fixture. The same fixture gives the same bytes on
/// every call (see `compare_raw`). A redactor error is
/// `compare_envelope_failed`, so the run can write its partial report.
async fn compare_envelope(
    fixture: &CompareFixture,
) -> Result<TraceContributionEnvelope, &'static str> {
    let mut envelope = DeterministicTraceRedactor::try_default()
        .map_err(|_| "compare_envelope_failed")?
        .redact_trace(compare_raw(fixture))
        .await
        .map_err(|_| "compare_envelope_failed")?;
    match fixture.privacy_risk.as_str() {
        // The envelope keeps the risk that the redaction gave, as
        // `build_envelope_from_draft` of pilot-bootstrap does. The server
        // pass cannot make that risk lower.
        "low" => {}
        // A declared risk: only a local pin has one.
        "medium" | "high" => {
            make_metadata_only_low_risk(&mut envelope);
            set_metadata_only_tool_name(&mut envelope, &fixture.label);
            envelope.privacy.residual_pii_risk = if fixture.privacy_risk == "medium" {
                ResidualPiiRisk::Medium
            } else {
                ResidualPiiRisk::High
            };
        }
        _ => unreachable!("CompareFixture::is_valid refuses any other privacy risk"),
    }
    envelope.consent.scopes = vec![ConsentScope::ModelTraining];
    envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
    Ok(envelope)
}

// ---------------------------------------------------------------------------
// The gate configuration and the calibration.
// ---------------------------------------------------------------------------

/// `main`'s gate configuration for the run: the derived floors, `main`'s
/// default `top_k` (PC-D23), and the other values of
/// `CompatibilityBundleConfig::local_reference()`.
fn compare_main_gate(floors: DerivedFloors) -> MainGateConfig {
    let reference = CompatibilityBundleConfig::local_reference();
    MainGateConfig {
        perplexity_floor_micros: Some(floors.perplexity_floor_micros),
        tail_fraction_floor_micros: Some(floors.tail_fraction_floor_micros),
        novelty_floor_micros: Some(floors.novelty_floor_micros),
        embed_insert_novelty_micros: reference.embed_insert_novelty_micros,
        top_k: crate::TRACE_COMMONS_GATE_DEFAULT_TOP_K as u32,
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
// baseline-old-path: the old orchestrator and `MockVectorIndex`.
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
        let bytes = serde_json::to_vec(&compare_envelope(&fixture).await?)
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
    /// The time that the drivers spent inside those calls.
    http_nanos: std::sync::atomic::AtomicU64,
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
        let started = std::time::Instant::now();
        let response = request.send().await.map_err(|_| "compare_http_failed")?;
        let code = response.status().as_u16();
        let received = response.bytes().await.map_err(|_| "compare_http_failed")?;
        self.http_nanos.fetch_add(
            u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX),
            AtomicOrdering::Relaxed,
        );
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
        http_nanos: std::sync::atomic::AtomicU64::new(0),
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
// One trace through the two sides.
// ---------------------------------------------------------------------------

fn trace_hash_of(fixture: &CompareFixture) -> String {
    sha256_bytes(fixture.trace_id.to_string().as_bytes())
}

/// Splits the time of a driver step into the time inside HTTP calls and the
/// rest, which is the database reads of the step.
struct StepClock {
    started: std::time::Instant,
    http_nanos: u64,
}

impl StepClock {
    fn start(app: &CompareApp) -> Self {
        Self {
            started: std::time::Instant::now(),
            http_nanos: app.http_nanos.load(AtomicOrdering::Relaxed),
        }
    }

    /// The HTTP time and the other time since the start or the last lap.
    fn lap(&mut self, app: &CompareApp) -> (std::time::Duration, std::time::Duration) {
        let now = std::time::Instant::now();
        let http_nanos = app.http_nanos.load(AtomicOrdering::Relaxed);
        let http = std::time::Duration::from_nanos(http_nanos - self.http_nanos);
        let other = (now - self.started).saturating_sub(http);
        self.started = now;
        self.http_nanos = http_nanos;
        (http, other)
    }
}

/// The milliseconds of one trace, for the timing file. The five values add
/// up to the time of the four driver steps.
struct PairTiming {
    // baseline-old-path: the HTTP call of the receipt.
    baseline_receipt_ms: u64,
    // baseline-old-path: the HTTP calls of the finish step, which are the
    // review calls (when the action has a review) and the gate call.
    baseline_gate_ms: u64,
    /// The HTTP call of the receipt.
    candidate_receipt_ms: u64,
    /// The whole finish step: the wait for the worker, the review calls
    /// (when the action has a review), and the reads of the result.
    candidate_wait_ms: u64,
    /// The database reads of the other three steps.
    reads_ms: u64,
}

/// The two records of one trace, and what the drivers did for them.
struct Pair {
    action: AlignmentAction,
    baseline: ComparisonRecord,
    candidate: ComparisonRecord,
    /// The probes of this trace. Each received body and the two records went
    /// through `Probes::check`.
    probes: Probes,
    /// The HTTP calls of the two sides for this trace.
    http_calls: u64,
    timing: PairTiming,
}

/// Drives one fixture through the two sides in the order of spec section
/// 7.4. The run and the tests call this one function, so they cannot use two
/// sequences.
///
/// 1. The two receipts with the same body bytes: the baseline first, then
///    the candidate, as the parity test requires.
/// 2. The alignment action for the two admissions.
/// 3. The two finish steps. The candidate is first when its review can fail
///    (`ApproveCandidate` and `HashRuleBoth(Approve)`): the baseline index
///    then does not get a trace that the candidate index does not get.
///
/// The envelope and the body live only inside this call.
async fn compare_pair(
    app: &CompareApp,
    fixture: &CompareFixture,
    partition: &'static str,
    position: u64,
) -> Result<Pair, &'static str> {
    let envelope = compare_envelope(fixture).await?;
    let body = serde_json::to_vec(&envelope).map_err(|_| "compare_envelope_failed")?;
    let probes = Probes::new(&app.tenants, fixture, &envelope);
    drop(envelope);
    let trace_hash = trace_hash_of(fixture);
    let base = || RecordBase {
        position,
        partition,
        trace_hash: trace_hash.clone(),
    };
    let calls_before = app.http_call_count();
    let mut clock = StepClock::start(app);

    let baseline_seen = baseline_admission(app, &probes, &body, fixture.submission_id).await?;
    let baseline_receipt = clock.lap(app);
    let candidate_seen = candidate_admission(app, &probes, &body, fixture.submission_id).await?;
    let candidate_receipt = clock.lap(app);
    drop(body);

    let action = alignment_action(
        baseline_seen.admission,
        candidate_seen.admission,
        &trace_hash,
    );
    let candidate_first = matches!(
        action,
        AlignmentAction::ApproveCandidate | AlignmentAction::HashRuleBoth(ReviewLabel::Approve)
    );
    let (baseline, baseline_finished, candidate, candidate_finished) = if candidate_first {
        let candidate =
            candidate_finish(app, &probes, fixture, candidate_seen, action, base()).await?;
        let candidate_finished = clock.lap(app);
        let baseline =
            baseline_finish(app, &probes, fixture, baseline_seen, action, base()).await?;
        (baseline, clock.lap(app), candidate, candidate_finished)
    } else {
        let baseline =
            baseline_finish(app, &probes, fixture, baseline_seen, action, base()).await?;
        let baseline_finished = clock.lap(app);
        let candidate =
            candidate_finish(app, &probes, fixture, candidate_seen, action, base()).await?;
        (baseline, baseline_finished, candidate, clock.lap(app))
    };

    let millis = |time: std::time::Duration| u64::try_from(time.as_millis()).unwrap_or(u64::MAX);
    Ok(Pair {
        action,
        baseline,
        candidate,
        http_calls: app.http_call_count() - calls_before,
        timing: PairTiming {
            baseline_receipt_ms: millis(baseline_receipt.0),
            baseline_gate_ms: millis(baseline_finished.0),
            candidate_receipt_ms: millis(candidate_receipt.0),
            candidate_wait_ms: millis(candidate_finished.0 + candidate_finished.1),
            reads_ms: millis(baseline_receipt.1 + candidate_receipt.1 + baseline_finished.1),
        },
        probes,
    })
}

// ---------------------------------------------------------------------------
// The run: its configuration, the record guard, and the loop body.
// ---------------------------------------------------------------------------

const COMPARE_BOOTSTRAP_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_BOOTSTRAP_PATH";
const COMPARE_HOLDOUT_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_HOLDOUT_PATH";
const COMPARE_MANIFEST_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_MANIFEST_PATH";
const COMPARE_CHECK_ID_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_CHECK_ID";
const COMPARE_REPORT_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_REPORT_PATH";
const COMPARE_RECORDS_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_RECORDS_PATH";
const COMPARE_LIMIT_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_LIMIT";
// baseline-old-path:
const COMPARE_SKEW_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_SKEW";
const COMPARE_TIMING_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_COMPARE_TIMING_PATH";

const COMPARE_CHECK_IDS: [&str; 2] = ["pipeline_comparison_local", "pipeline_comparison_hf"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct CompareRunConfig {
    bootstrap: PathBuf,
    holdout: PathBuf,
    manifest: PathBuf,
    check_id: String,
    report_path: PathBuf,
    records_path: PathBuf,
    /// The run compares only the first `limit` traces.
    limit: Option<u64>,
    /// `None`, or `baseline_quality_floor` (PC-D8).
    // baseline-old-path:
    skew: Option<String>,
    timing_path: Option<PathBuf>,
    artifact_root: Option<PathBuf>,
    master_key_hex: Option<String>,
}

impl CompareRunConfig {
    /// `Ok(None)` when none of the compare variables is set (nothing to
    /// run). If one is set, each required variable must be set and not
    /// empty, and each value must be valid.
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, &'static str> {
        const RUN_VARS: [&str; 9] = [
            COMPARE_BOOTSTRAP_PATH_VAR,
            COMPARE_HOLDOUT_PATH_VAR,
            COMPARE_MANIFEST_PATH_VAR,
            COMPARE_CHECK_ID_VAR,
            COMPARE_REPORT_PATH_VAR,
            COMPARE_RECORDS_PATH_VAR,
            COMPARE_LIMIT_VAR,
            COMPARE_SKEW_VAR,
            COMPARE_TIMING_PATH_VAR,
        ];
        if RUN_VARS.iter().all(|name| var(name).is_none()) {
            return Ok(None);
        }
        let required = |name: &str, label: &'static str| {
            var(name).filter(|value| !value.is_empty()).ok_or(label)
        };
        let bootstrap = required(COMPARE_BOOTSTRAP_PATH_VAR, "compare_bootstrap_path_missing")?;
        let holdout = required(COMPARE_HOLDOUT_PATH_VAR, "compare_holdout_path_missing")?;
        let manifest = required(COMPARE_MANIFEST_PATH_VAR, "compare_manifest_path_missing")?;
        let check_id = required(COMPARE_CHECK_ID_VAR, "compare_check_id_missing")?;
        let report = required(COMPARE_REPORT_PATH_VAR, "compare_report_path_missing")?;
        let records = required(COMPARE_RECORDS_PATH_VAR, "compare_records_path_missing")?;
        if !COMPARE_CHECK_IDS.contains(&check_id.as_str()) {
            return Err("compare_check_id_invalid");
        }
        let limit = var(COMPARE_LIMIT_VAR)
            .map(|text| {
                text.parse::<u64>()
                    .ok()
                    .filter(|limit| *limit > 0)
                    .ok_or("compare_limit_invalid")
            })
            .transpose()?;
        let skew = var(COMPARE_SKEW_VAR);
        if skew
            .as_deref()
            .is_some_and(|skew| skew != "baseline_quality_floor")
        {
            return Err("compare_skew_invalid");
        }
        Ok(Some(Self {
            bootstrap: PathBuf::from(bootstrap),
            holdout: PathBuf::from(holdout),
            manifest: PathBuf::from(manifest),
            check_id,
            report_path: PathBuf::from(report),
            records_path: PathBuf::from(records),
            limit,
            skew,
            timing_path: var(COMPARE_TIMING_PATH_VAR).map(PathBuf::from),
            artifact_root: var(ARTIFACT_ROOT_VAR).map(PathBuf::from),
            master_key_hex: var(TEST_MASTER_KEY_VAR),
        }))
    }
}

/// `sha256:` and 64 lowercase hex characters.
fn is_sha256_value(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// What the run takes from the manifest of the export.
struct ComparePin {
    /// The keys of `ComparisonReportInput::pin_digests`: `source`, `order`,
    /// `configuration`, `bootstrap_corpus`, `holdout_corpus`.
    digests: BTreeMap<String, String>,
    /// `sample_count`: the number of traces in the two corpus files.
    trace_count: u64,
}

/// Reads `source-manifest.json` of an export. A file that cannot be read or
/// parsed, or that lacks a digest or `sample_count`, is
/// `compare_manifest_invalid`. An export without `--with-events` is
/// `compare_manifest_without_events`: its corpus files have no session
/// events.
fn read_compare_manifest(path: &Path) -> Result<ComparePin, &'static str> {
    const INVALID: &str = "compare_manifest_invalid";
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| INVALID)?).map_err(|_| INVALID)?;
    if manifest["source"]["with_events"] != true {
        return Err("compare_manifest_without_events");
    }
    let mut digests = BTreeMap::new();
    for key in [
        "source",
        "order",
        "configuration",
        "bootstrap_corpus",
        "holdout_corpus",
    ] {
        let digest = manifest[format!("{key}_digest")]
            .as_str()
            .filter(|digest| is_sha256_value(digest))
            .ok_or(INVALID)?;
        digests.insert(key.to_string(), digest.to_string());
    }
    Ok(ComparePin {
        digests,
        trace_count: manifest["sample_count"].as_u64().ok_or(INVALID)?,
    })
}

/// Review Focus 3. A record holds a text in four fields, and each text comes
/// from a database row. The run writes a record only when `trace_hash` is a
/// `sha256:` value, `privacy_risk` is one of the three risk labels, and each
/// `privacy_basis` entry and each credit `event_type` is a safe label.
fn check_record_values(record: &ComparisonRecord) -> Result<(), &'static str> {
    let safe = is_sha256_value(&record.trace_hash)
        && record
            .privacy_risk
            .as_deref()
            .is_none_or(|risk| matches!(risk, "low" | "medium" | "high"))
        && record
            .privacy_basis
            .iter()
            .all(|basis| is_safe_label(basis))
        && record
            .credit_events
            .iter()
            .all(|event| is_safe_label(&event.event_type));
    if safe {
        Ok(())
    } else {
        Err("compare_record_value_unsafe")
    }
}

fn output_failed<E>(_: E) -> &'static str {
    "compare_output_write_failed"
}

/// Creates the file `path`, and its directory, for a writer of lines.
fn create_output(path: &Path) -> Result<std::io::BufWriter<std::fs::File>, &'static str> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(output_failed)?;
    }
    std::fs::File::create(path)
        .map(std::io::BufWriter::new)
        .map_err(output_failed)
}

/// The report path at the start of the run, so that the run does not find
/// a directory that it cannot write after the last trace. It creates the
/// directory, proves that a file can be created in it, and removes a report
/// that exists already: a run that fails before it writes its report then
/// leaves no report of an earlier run beside its new records file.
fn prepare_report_path(path: &Path) -> Result<(), &'static str> {
    let parent = path.parent().ok_or("compare_output_write_failed")?;
    std::fs::create_dir_all(parent).map_err(output_failed)?;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err("compare_output_write_failed"),
    }
    // Removed at the drop.
    tempfile::NamedTempFile::new_in(parent).map_err(output_failed)?;
    Ok(())
}

/// What `PipelineCheckEmitter::emit_pass_from_env` refuses after the last
/// trace, examined at the start of the run with no write: a check result
/// environment that is not complete or not valid, and a result of
/// `check_id` that exists already.
fn check_result_preflight(check_id: &str) -> Result<(), String> {
    if PipelineCheckEmitter::from_env()?.is_none() {
        return Ok(());
    }
    let dir = std::env::var(CHECK_RESULT_DIR_VAR)
        .map_err(|_| "pipeline_check_environment_invalid".to_string())?;
    if Path::new(&dir)
        .join(format!("{check_id}.result.json"))
        .exists()
    {
        return Err("pipeline_check_already_emitted".to_string());
    }
    Ok(())
}

/// The verdict of a run. The order is fixed: a pair that loses the alignment
/// and a refused pair are also unexplained, and the more exact label must
/// win.
///
/// 1. `failure`: the label that the loop kept (a driver error, or
///    `COMPARISON_ALIGNMENT_LABEL`).
/// 2. A refused receipt (PC-D19), for each run, also a partial one.
/// 3. An unexplained difference.
/// 4. For a run that is not partial: a gate branch with no evidence.
fn run_verdict(
    failure: Option<&'static str>,
    summary: &ComparisonSummary,
    partial: bool,
) -> Result<(), &'static str> {
    if let Some(label) = failure {
        return Err(label);
    }
    if summary.refused_total() != 0 {
        return Err(COMPARISON_REFUSED_LABEL);
    }
    if summary.unexplained_total() != 0 {
        return Err(COMPARISON_UNEXPLAINED_LABEL);
    }
    if !partial && !summary.branch_gaps().is_empty() {
        return Err(COMPARISON_BRANCH_LABEL);
    }
    Ok(())
}

/// What the run keeps from one trace to the next. Nothing here grows with
/// the number of traces: the summary holds counts and a bounded list, and
/// the records go to the file.
struct CompareRun {
    summary: ComparisonSummary,
    /// The number of pairs in the summary and in the records file, which is
    /// also the position of the next trace. The position counts through the
    /// two partitions, from 0.
    compared: u64,
    records: std::io::BufWriter<std::fs::File>,
    /// The SHA-256 of the bytes that `records` got.
    records_digest: sha2::Sha256,
    /// The optional timing file. It is not part of the report or of a
    /// digest, so its first write error ends the timing and not the run.
    timing: Option<std::io::BufWriter<std::fs::File>>,
    /// PC-D20: the position of the pair at which the run stopped.
    alignment_lost_position: Option<u64>,
    /// The probes of the last trace that was driven, for the check of the
    /// report.
    probes: Option<Probes>,
}

impl CompareRun {
    /// Opens the records file and, when the configuration names one, the
    /// timing file, whose first line is the calibration time.
    fn open(
        config: &CompareRunConfig,
        calibration: std::time::Duration,
    ) -> Result<Self, &'static str> {
        let mut run = Self {
            summary: ComparisonSummary::default(),
            compared: 0,
            records: create_output(&config.records_path)?,
            records_digest: sha2::Sha256::new(),
            timing: None,
            alignment_lost_position: None,
            probes: None,
        };
        if let Some(path) = &config.timing_path {
            match create_output(path) {
                Ok(timing) => {
                    run.timing = Some(timing);
                    run.write_timing(
                        serde_json::json!({ "calibration_seconds": calibration.as_secs_f64() }),
                    );
                }
                Err(_) => eprintln!("compare_timing_write_failed"),
            }
        }
        Ok(run)
    }

    /// One line of the timing file. The first error prints one label line
    /// and drops the writer, and the run continues.
    fn write_timing(&mut self, line: serde_json::Value) {
        let Some(timing) = &mut self.timing else {
            return;
        };
        if writeln!(timing, "{line}")
            .and_then(|()| timing.flush())
            .is_err()
        {
            eprintln!("compare_timing_write_failed");
            self.timing = None;
        }
    }

    /// The loop body: one trace through the two sides, the guard for its two
    /// records, the comparison, and the two lines of the records file.
    ///
    /// An `Err` ends the run. A pair that loses the alignment (PC-D20) is in
    /// the summary and in the records file before this function answers
    /// `Err(COMPARISON_ALIGNMENT_LABEL)`. The fixture is dropped at the
    /// return.
    async fn compare_trace(
        &mut self,
        app: &CompareApp,
        fixture: CompareFixture,
        partition: &'static str,
    ) -> Result<(), &'static str> {
        let position = self.compared;
        let Pair {
            action,
            baseline,
            candidate,
            probes,
            timing,
            ..
        } = compare_pair(app, &fixture, partition, position).await?;
        // At once: an error below must not leave the probes of the trace
        // before for the check of the report.
        self.probes = Some(probes);
        check_record_values(&baseline)?;
        check_record_values(&candidate)?;
        let result = compare_records(&baseline, &candidate);
        self.summary.observe(&baseline, &candidate, &result);
        self.compared += 1;
        for record in [&baseline, &candidate] {
            let mut line = record_bytes(record);
            line.push(b'\n');
            self.records.write_all(&line).map_err(output_failed)?;
            self.records_digest.update(&line);
        }
        self.records.flush().map_err(output_failed)?;
        if self.timing.is_some() {
            let gate = baseline.gate.as_ref().or(candidate.gate.as_ref());
            self.write_timing(serde_json::json!({
                "position": position,
                "chunk_count": gate.map(|gate| gate.total_chunk_count),
                "baseline_receipt_ms": timing.baseline_receipt_ms,
                "baseline_gate_ms": timing.baseline_gate_ms,
                "candidate_receipt_ms": timing.candidate_receipt_ms,
                "candidate_wait_ms": timing.candidate_wait_ms,
                "reads_ms": timing.reads_ms,
            }));
        }
        // baseline-old-path: PC-D18. Each call and each read of this trace is
        // complete, and the next baseline receipt did not start.
        clear_baseline_derived(app)?;
        if alignment_lost(action, &baseline, &candidate) {
            self.alignment_lost_position = Some(position);
            return Err(COMPARISON_ALIGNMENT_LABEL);
        }
        Ok(())
    }
}

/// `pipeline.py compare`. Each trace of the two corpus files goes through the
/// old gate path and through the pipeline, in the order of the files. The
/// run writes two records for each trace and one report. It fails when the
/// two sides differ, and the report that it wrote before names each
/// difference. The report is written before the app stops, so a failed stop
/// does not remove it. A check result is emitted only for a full run with no
/// skew.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the implementation of `pipeline.py compare`: run it through that command"]
async fn pipeline_compare_run() {
    fn fail<T>(label: &'static str) -> T {
        panic!("{label}")
    }

    // 1. The configuration. A configured run needs its database: it never
    // skips.
    let config = match CompareRunConfig::from_vars(|name| std::env::var(name).ok()) {
        Ok(Some(config)) => config,
        Ok(None) => return,
        Err(label) => fail(label),
    };
    assert!(
        std::env::var_os("TRACE_COMMONS_PG_TEST_DATABASE_URL").is_some(),
        "compare_database_url_missing"
    );
    // 1a. The two corpus files are the files of the pin, before the first
    // trace: a streamed read with no parse. The manifest counts their lines.
    let pin = read_compare_manifest(&config.manifest).unwrap_or_else(fail);
    let partitions = [
        ("bootstrap", &config.bootstrap),
        ("holdout", &config.holdout),
    ];
    let pinned_digest = |partition: &str| &pin.digests[&format!("{partition}_corpus")];
    let mut corpus_lines = 0;
    for (partition, path) in partitions {
        let (digest, lines) = CompareCorpusReader::open(partition, path)
            .and_then(CompareCorpusReader::finish_with_lines)
            .unwrap_or_else(fail);
        assert!(
            digest == *pinned_digest(partition),
            "compare_corpus_digest_mismatch"
        );
        corpus_lines += lines;
    }
    assert!(
        corpus_lines == pin.trace_count,
        "compare_trace_count_mismatch"
    );
    // What the end of the run needs, examined now: the report path, and the
    // check result environment when this run can emit a result (item 10).
    prepare_report_path(&config.report_path).unwrap_or_else(fail);
    // baseline-old-path: the skew.
    if config.skew.is_none() && config.limit.is_none_or(|limit| limit >= pin.trace_count) {
        check_result_preflight(&config.check_id).unwrap_or_else(|label| panic!("{label}"));
    }

    // 2. The floors, from the full bootstrap partition (also with a limit).
    let calibration_started = std::time::Instant::now();
    let floors = calibrate_floors(&config.bootstrap)
        .await
        .unwrap_or_else(fail);
    let calibration = calibration_started.elapsed();

    // 3. One app for the two sides.
    let generated_root = tempfile::tempdir().expect("temp dir");
    let artifact_root = config
        .artifact_root
        .clone()
        .unwrap_or_else(|| generated_root.path().to_path_buf());
    let master_key = config
        .master_key_hex
        .clone()
        .unwrap_or_else(trace_commons_server::secrets::keychain::generate_master_key_hex);
    let app = start_compare_app(
        compare_main_gate(floors),
        &artifact_root,
        &master_key,
        CompareAppOptions {
            tenants: CompareTenants::run(),
            skew: config.skew.as_deref(),
            accept_medium_risk: true,
        },
    )
    .await;

    // 4. The records file.
    let mut run = CompareRun::open(&config, calibration).unwrap_or_else(fail);

    // 5. Each trace, in the order of the two files, until the limit. The
    // first error ends the loop. The run prints its label at once, because
    // a later panic (the app stop, for example) would hide it, and keeps it
    // for the verdict.
    let mut readers = partitions
        .map(|(partition, path)| CompareCorpusReader::open(partition, path).unwrap_or_else(fail));
    let mut failure = None;
    'partitions: for reader in &mut readers {
        while config.limit.is_none_or(|limit| run.compared < limit) {
            let step = match reader.next_fixture() {
                Ok(Some(fixture)) => run.compare_trace(&app, fixture, reader.partition).await,
                Ok(None) => break,
                Err(label) => Err(label),
            };
            if let Err(label) = step {
                eprintln!("{label}");
                failure = Some(label);
                break 'partitions;
            }
        }
    }

    // 6. The readers read the bytes that step 1a checked. With a limit,
    // `finish` reads the rest of a file and builds no envelope.
    if failure.is_none() {
        for reader in readers {
            let partition = reader.partition;
            let digest = reader.finish().unwrap_or_else(fail);
            assert!(
                digest == *pinned_digest(partition),
                "compare_corpus_digest_mismatch"
            );
        }
        let expected = config
            .limit
            .map_or(pin.trace_count, |limit| limit.min(pin.trace_count));
        assert!(run.compared == expected, "compare_trace_count_mismatch");
    }

    // 8. The report, before the app stop and before the verdict: a run that
    // fails, also in the app stop, leaves a report that names its
    // differences. The tokens do not depend on a trace, so each report is
    // checked for them.
    let records_digest = format!("sha256:{}", hex::encode(run.records_digest.finalize()));
    let partial = run.compared < pin.trace_count;
    let report = comparison_report(
        &ComparisonReportInput {
            check_id: &config.check_id,
            trace_count: pin.trace_count,
            partial,
            pin_digests: &pin.digests,
            package: &app.package,
            floors,
            skew: config.skew.as_deref(),
            alignment_lost_position: run.alignment_lost_position,
            records_digest: &records_digest,
        },
        &run.summary,
    )
    .unwrap_or_else(|label| panic!("{label}"));
    let mut report_bytes = trace_commons_protocol::canonical_json::to_canonical_vec(&report)
        .expect("the report serialises");
    report_bytes.push(b'\n');
    for token in app.tenants.tokens() {
        assert!(!holds(&report_bytes, token), "compare_probe_in_report");
    }
    if let Some(probes) = &run.probes {
        probes.check(&report_bytes, "compare_probe_in_report");
    }
    write_atomically(&config.report_path, &report_bytes);

    // 7. Stop the app.
    let package = app.package.clone();
    app.shutdown().await;

    // 9. The verdict.
    run_verdict(failure, &run.summary, partial).unwrap_or_else(fail::<()>);

    // 10. A check result only for a full run with no skew.
    // baseline-old-path: the skew.
    if partial || config.skew.is_some() {
        return;
    }
    let permitted: u64 = report["permitted_counts"]
        .as_object()
        .into_iter()
        .flat_map(|counts| counts.values())
        .filter_map(serde_json::Value::as_u64)
        .sum();
    PipelineCheckEmitter::emit_pass_from_env(
        &config.check_id,
        Some(&package),
        serde_json::json!({
            "traces": report["compared_count"],
            "equal": report["equal_count"],
            "permitted": permitted,
            "unexplained": 0,
            "records_hash": records_digest,
            "report_hash": sha256_bytes(&report_bytes),
        }),
    );
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
    let first = compare_envelope(&fixture).await.unwrap();
    let second = compare_envelope(&fixture).await.unwrap();
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
    let envelope = compare_envelope(&fixture).await.unwrap();
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
        let envelope = compare_envelope(&fixture).await.unwrap();
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

/// What the contributor-side redaction alone gives for `fixture`.
async fn redaction_risk(fixture: &CompareFixture) -> ResidualPiiRisk {
    DeterministicTraceRedactor::try_default()
        .unwrap()
        .redact_trace(compare_raw(fixture))
        .await
        .unwrap()
        .privacy
        .residual_pii_risk
}

#[tokio::test]
async fn a_trace_keeps_the_privacy_risk_of_the_redaction() {
    // The redaction does not classify a payload text of more than 32,000
    // bytes, and it then answers a risk that the server pass cannot derive.
    let large = large_argument_fixture("kept_large");
    let expected = redaction_risk(&large).await;
    assert_ne!(expected, ResidualPiiRisk::Low);
    let envelope = compare_envelope(&large).await.unwrap();
    assert_eq!(envelope.privacy.residual_pii_risk, expected);
    assert!(envelope.consent.tool_payloads_included);

    let prose = prose_fixture("kept_prose");
    let envelope = compare_envelope(&prose).await.unwrap();
    assert_eq!(
        envelope.privacy.residual_pii_risk,
        redaction_risk(&prose).await
    );
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
    // PC-D23: the two sides take `main`'s default, not the reference's 8.
    assert_eq!(config.top_k, 5);
    assert_eq!(config.top_k, crate::TRACE_COMMONS_GATE_DEFAULT_TOP_K);
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
    let body = serde_json::to_vec(&compare_envelope(&fixture).await.unwrap()).unwrap();

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

/// A fixture with no declared risk and about 220 words of text that no other
/// label has.
fn prose_fixture(label: &str) -> CompareFixture {
    risk_fixture(label, "low")
}

fn risk_fixture(label: &str, risk: &str) -> CompareFixture {
    fixture_from(fixture_line(label, risk, prose_steps(&long_text(label))))
}

/// A fixture with no declared risk and one tool call whose argument has
/// 40,000 bytes of text.
fn large_argument_fixture(label: &str) -> CompareFixture {
    fixture_from(fixture_line(
        label,
        "low",
        large_argument_steps(label, 40_000),
    ))
}

fn record_base(fixture: &CompareFixture, position: u64) -> RecordBase {
    RecordBase {
        position,
        partition: "holdout",
        trace_hash: trace_hash_of(fixture),
    }
}

/// `compare_pair` for a test: the partition is `holdout`, and an error
/// panics.
async fn drive_pair(app: &CompareApp, fixture: &CompareFixture, position: u64) -> Pair {
    compare_pair(app, fixture, "holdout", position)
        .await
        .unwrap()
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

/// Spec section 8.3 row 4, with no declared risk: the risk of the envelope is
/// the one that the contributor-side redaction gave.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_large_tool_argument_keeps_its_risk_on_both_sides() {
    let Some((app, _artifacts)) = start_test_app(PASSING_FLOORS, true).await else {
        return;
    };
    let pair = drive_pair(&app, &large_argument_fixture("large_argument"), 0).await;
    for record in [&pair.baseline, &pair.candidate] {
        assert_eq!(record.receipt_code, 200);
        assert_eq!(record.privacy_risk.as_deref(), Some("high"));
        assert_eq!(record.privacy_basis, ["consent_content_flag"]);
    }
    assert_eq!(pair.baseline.admission, AdmissionLabel::Quarantine);
    assert_eq!(pair.candidate.admission, AdmissionLabel::Reject);
    assert_eq!(pair.action, AlignmentAction::RejectBaseline);
    assert_eq!(pair.baseline.review, ReviewLabel::Reject);
    assert_eq!(pair.baseline.review_source, ReviewSource::Alignment);
    assert_eq!(pair.candidate.review, ReviewLabel::None);
    for record in [&pair.baseline, &pair.candidate] {
        assert!(record.terminal);
        assert!(!record.scored);
        assert!(!record.member);
    }
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
    let envelope = compare_envelope(&prose).await.unwrap();
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
    assert_eq!(pair.probes.check_count(), pair.http_calls + 2);

    // (Quarantine, Quarantine) with an approval: the pipeline review routes.
    let approved = hash_rule_fixture("probe_review", ReviewLabel::Approve);
    let pair = drive_pair(&app, &approved, 1).await;
    assert_eq!(
        pair.action,
        AlignmentAction::HashRuleBoth(ReviewLabel::Approve)
    );
    assert!(pair.http_calls >= 8, "{}", pair.http_calls);
    assert_eq!(pair.probes.check_count(), pair.http_calls + 2);
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
    let envelope = compare_envelope(&fixture).await.unwrap();
    let probes = Probes::new(&tenants, &fixture, &envelope);
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
    let envelope = compare_envelope(&fixture).await.unwrap();
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
        Probes::new(&tenants, &short, &compare_envelope(&short).await.unwrap())
            .content
            .is_empty()
    );
    let declared = risk_fixture("probe_declared", "medium");
    assert!(
        Probes::new(
            &tenants,
            &declared,
            &compare_envelope(&declared).await.unwrap()
        )
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
        let envelope = compare_envelope(&fixture).await.unwrap();
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
    let envelope = compare_envelope(&fixture).await.unwrap();
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
    let envelope = compare_envelope(&refused).await.unwrap();
    let probes = Probes::new(&app.tenants, &refused, &envelope);
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
    let envelope = compare_envelope(&fixture).await.unwrap();
    let probes = Probes::new(&app.tenants, &fixture, &envelope);
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
    let envelope = compare_envelope(&fixture).await.unwrap();
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

// ---------------------------------------------------------------------------
// Tests of the run configuration and of the record guard.
// ---------------------------------------------------------------------------

/// The six required variables with valid values, and no other variable.
fn compare_vars() -> BTreeMap<&'static str, String> {
    BTreeMap::from([
        (COMPARE_BOOTSTRAP_PATH_VAR, "/pin/bootstrap-compare.jsonl"),
        (COMPARE_HOLDOUT_PATH_VAR, "/pin/holdout-compare.jsonl"),
        (COMPARE_MANIFEST_PATH_VAR, "/pin/source-manifest.json"),
        (COMPARE_CHECK_ID_VAR, "pipeline_comparison_local"),
        (COMPARE_REPORT_PATH_VAR, "/run/report.json"),
        (COMPARE_RECORDS_PATH_VAR, "/run/comparison-records.jsonl"),
    ])
    .into_iter()
    .map(|(name, value)| (name, value.to_string()))
    .collect()
}

fn config_from(
    vars: &BTreeMap<&'static str, String>,
) -> Result<Option<CompareRunConfig>, &'static str> {
    CompareRunConfig::from_vars(|name| vars.get(name).cloned())
}

/// Each required variable with the label of a run that does not have it.
const REQUIRED_COMPARE_VARS: [(&str, &str); 6] = [
    (COMPARE_BOOTSTRAP_PATH_VAR, "compare_bootstrap_path_missing"),
    (COMPARE_HOLDOUT_PATH_VAR, "compare_holdout_path_missing"),
    (COMPARE_MANIFEST_PATH_VAR, "compare_manifest_path_missing"),
    (COMPARE_CHECK_ID_VAR, "compare_check_id_missing"),
    (COMPARE_REPORT_PATH_VAR, "compare_report_path_missing"),
    (COMPARE_RECORDS_PATH_VAR, "compare_records_path_missing"),
];

#[test]
fn no_compare_variable_means_nothing_to_run() {
    assert_eq!(CompareRunConfig::from_vars(|_| None), Ok(None));
    // The two variables that `pipeline_corpus_run` reads too start no run.
    let shared = BTreeMap::from([
        (ARTIFACT_ROOT_VAR, "/artifacts".to_string()),
        (TEST_MASTER_KEY_VAR, "00".repeat(32)),
    ]);
    assert_eq!(config_from(&shared), Ok(None));
}

#[test]
fn every_required_variable_is_checked() {
    assert_eq!(
        config_from(&compare_vars()),
        Ok(Some(CompareRunConfig {
            bootstrap: PathBuf::from("/pin/bootstrap-compare.jsonl"),
            holdout: PathBuf::from("/pin/holdout-compare.jsonl"),
            manifest: PathBuf::from("/pin/source-manifest.json"),
            check_id: "pipeline_comparison_local".to_string(),
            report_path: PathBuf::from("/run/report.json"),
            records_path: PathBuf::from("/run/comparison-records.jsonl"),
            limit: None,
            skew: None,
            timing_path: None,
            artifact_root: None,
            master_key_hex: None,
        }))
    );
    for (name, label) in REQUIRED_COMPARE_VARS {
        let mut vars = compare_vars();
        vars.remove(name);
        assert_eq!(config_from(&vars), Err(label), "{name}");
    }
    // One optional variable alone is a run with no required variable.
    for name in [COMPARE_LIMIT_VAR, COMPARE_SKEW_VAR, COMPARE_TIMING_PATH_VAR] {
        let vars = BTreeMap::from([(name, "1".to_string())]);
        assert_eq!(
            config_from(&vars),
            Err("compare_bootstrap_path_missing"),
            "{name}"
        );
    }
}

#[test]
fn the_check_id_and_the_limit_are_validated() {
    let with = |name: &'static str, value: &str| {
        let mut vars = compare_vars();
        vars.insert(name, value.to_string());
        config_from(&vars)
    };
    for check_id in ["pipeline_comparison_local", "pipeline_comparison_hf"] {
        let config = with(COMPARE_CHECK_ID_VAR, check_id).unwrap().unwrap();
        assert_eq!(config.check_id, check_id);
    }
    for check_id in ["pipeline_comparison", "pipeline_corpus_hf_local", "local"] {
        assert_eq!(
            with(COMPARE_CHECK_ID_VAR, check_id),
            Err("compare_check_id_invalid"),
            "{check_id}"
        );
    }

    assert_eq!(
        with(COMPARE_LIMIT_VAR, "25").unwrap().unwrap().limit,
        Some(25)
    );
    for limit in ["0", "-1", "ten", "1.5", ""] {
        assert_eq!(
            with(COMPARE_LIMIT_VAR, limit),
            Err("compare_limit_invalid"),
            "{limit:?}"
        );
    }

    assert_eq!(
        with(COMPARE_SKEW_VAR, "baseline_quality_floor")
            .unwrap()
            .unwrap()
            .skew
            .as_deref(),
        Some("baseline_quality_floor")
    );
    for skew in ["candidate_quality_floor", "none", ""] {
        assert_eq!(
            with(COMPARE_SKEW_VAR, skew),
            Err("compare_skew_invalid"),
            "{skew:?}"
        );
    }

    // The three other optional variables are taken as they are.
    let mut vars = compare_vars();
    vars.insert(COMPARE_TIMING_PATH_VAR, "/run/timing.jsonl".to_string());
    vars.insert(ARTIFACT_ROOT_VAR, "/artifacts".to_string());
    vars.insert(TEST_MASTER_KEY_VAR, "00".repeat(32));
    let config = config_from(&vars).unwrap().unwrap();
    assert_eq!(config.timing_path, Some(PathBuf::from("/run/timing.jsonl")));
    assert_eq!(config.artifact_root, Some(PathBuf::from("/artifacts")));
    assert_eq!(config.master_key_hex, Some("00".repeat(32)));
}

#[test]
fn the_manifest_gives_the_pin_digests() {
    let digest = |digit: char| format!("sha256:{}", digit.to_string().repeat(64));
    let manifest = serde_json::json!({
        "schema": "trace_commons.pipeline_hf_corpus_manifest.v1",
        "source": { "translator": "swival", "with_events": true },
        "source_digest": digest('1'),
        "configuration_digest": digest('3'),
        "order_digest": digest('2'),
        "bootstrap_corpus_digest": digest('4'),
        "holdout_corpus_digest": digest('5'),
        "sample_count": 10,
        "bootstrap_count": 4,
        "holdout_count": 6,
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("source-manifest.json");
    let read = |manifest: &serde_json::Value| {
        std::fs::write(&path, manifest.to_string()).unwrap();
        read_compare_manifest(&path)
    };
    let pin = read(&manifest).unwrap();
    assert_eq!(
        pin.digests,
        BTreeMap::from([
            ("source".to_string(), digest('1')),
            ("order".to_string(), digest('2')),
            ("configuration".to_string(), digest('3')),
            ("bootstrap_corpus".to_string(), digest('4')),
            ("holdout_corpus".to_string(), digest('5')),
        ])
    );
    assert_eq!(pin.trace_count, 10);

    // A v1 export has no `with_events` key.
    let mut without = manifest.clone();
    without["source"]
        .as_object_mut()
        .unwrap()
        .remove("with_events");
    assert_eq!(
        read(&without).err(),
        Some("compare_manifest_without_events")
    );
    let mut without = manifest.clone();
    without["source"]["with_events"] = serde_json::json!(false);
    assert_eq!(
        read(&without).err(),
        Some("compare_manifest_without_events")
    );

    for key in [
        "source_digest",
        "order_digest",
        "configuration_digest",
        "bootstrap_corpus_digest",
        "holdout_corpus_digest",
        "sample_count",
    ] {
        let mut missing = manifest.clone();
        missing.as_object_mut().unwrap().remove(key);
        assert_eq!(
            read(&missing).err(),
            Some("compare_manifest_invalid"),
            "{key}"
        );
    }
    let mut short = manifest.clone();
    short["order_digest"] = serde_json::json!("sha256:abc");
    assert_eq!(read(&short).err(), Some("compare_manifest_invalid"));
    std::fs::write(&path, "this is not json").unwrap();
    assert_eq!(
        read_compare_manifest(&path).err(),
        Some("compare_manifest_invalid")
    );
    assert_eq!(
        read_compare_manifest(&dir.path().join("no-such-file.json")).err(),
        Some("compare_manifest_invalid")
    );
}

#[test]
fn an_empty_variable_is_a_missing_variable() {
    for (name, label) in REQUIRED_COMPARE_VARS {
        let mut vars = compare_vars();
        vars.insert(name, String::new());
        assert_eq!(config_from(&vars), Err(label), "{name}");
    }
}

/// The helper record of the library's tests
/// (`versioned_pipeline_comparison.rs`): each field has a value.
fn full_record() -> ComparisonRecord {
    ComparisonRecord {
        position: 7,
        partition: "holdout".into(),
        trace_hash: format!("sha256:{}", "a".repeat(64)),
        side: ComparisonSide::Baseline,
        receipt_code: 200,
        terminal: true,
        privacy_risk: Some("medium".into()),
        privacy_basis: vec!["consent_content_flag".into()],
        admission: AdmissionLabel::Admit,
        review: ReviewLabel::None,
        review_source: ReviewSource::None,
        gate_skipped_by_alignment: false,
        scored: true,
        gate: Some(GateValues {
            quality_passed: true,
            novelty_passed: true,
            perplexity_micros: 31_000_000,
            tail_fraction_micros: 120_000,
            peak_perplexity_micros: 40_000_000,
            novelty_score_micros: 600_000,
            peak_novelty_micros: 900_000,
            chunk_count: 3,
            total_chunk_count: 3,
            chunks_capped: false,
            index_cardinality: Some(12),
            credit_quality_micros: Some(450_000),
            credit_quality_version: Some(2),
        }),
        member: true,
        member_chunks: vec![0, 1, 2],
        credit_events: vec![CreditEvent {
            event_type: "novelty_utility".into(),
            microcredits: 2_500_000,
        }],
    }
}

/// Review Focus 3. The record type permits a text in four fields. The run
/// refuses a record in which one of them is not a hash or a label.
#[test]
fn an_unsafe_record_value_is_refused() {
    const UNSAFE: Result<(), &str> = Err("compare_record_value_unsafe");
    let record = full_record();
    assert_eq!(check_record_values(&record), Ok(()));
    let mut unscored = full_record();
    unscored.privacy_risk = None;
    unscored.privacy_basis.clear();
    unscored.credit_events.clear();
    assert_eq!(check_record_values(&unscored), Ok(()));
    for risk in ["low", "medium", "high"] {
        let mut record = full_record();
        record.privacy_risk = Some(risk.to_string());
        assert_eq!(check_record_values(&record), Ok(()), "{risk}");
    }

    // 1. `trace_hash`: `sha256:` and 64 lowercase hex characters.
    for trace_hash in [
        "5f0c2b1e-7c1d-4c58-9d0e-0a6c5b1f2e3d".to_string(),
        "a".repeat(64),
        format!("sha256:{}", "a".repeat(63)),
        format!("sha256:{}", "a".repeat(65)),
        format!("sha256:{}", "A".repeat(64)),
        format!("sha256:{}", "g".repeat(64)),
        format!("sha512:{}", "a".repeat(64)),
        String::new(),
    ] {
        let mut record = full_record();
        record.trace_hash = trace_hash.clone();
        assert_eq!(check_record_values(&record), UNSAFE, "{trace_hash}");
    }
    // 2. `privacy_risk`: one of the three risk labels.
    for risk in ["critical", "Medium", "", "the user wrote a name"] {
        let mut record = full_record();
        record.privacy_risk = Some(risk.to_string());
        assert_eq!(check_record_values(&record), UNSAFE, "{risk:?}");
    }
    // 3. Each `privacy_basis` entry is a label.
    for basis in ["Consent Content Flag", "a name: Ada", "", &"a".repeat(65)] {
        let mut record = full_record();
        record.privacy_basis.push(basis.to_string());
        assert_eq!(check_record_values(&record), UNSAFE, "{basis:?}");
    }
    // 4. Each credit `event_type` is a label.
    for event_type in ["Novelty Utility", "credit for ada@example.com", ""] {
        let mut record = full_record();
        record.credit_events.push(CreditEvent {
            event_type: event_type.to_string(),
            microcredits: 1,
        });
        assert_eq!(check_record_values(&record), UNSAFE, "{event_type:?}");
    }
}

// ---------------------------------------------------------------------------
// Tests of the verdict of the run.
// ---------------------------------------------------------------------------

/// The two records of one trace: `full_record` with `change` applied to the
/// two sides.
fn verdict_pair(change: impl Fn(&mut ComparisonRecord)) -> (ComparisonRecord, ComparisonRecord) {
    let mut baseline = full_record();
    let mut candidate = full_record();
    candidate.side = ComparisonSide::Candidate;
    change(&mut baseline);
    change(&mut candidate);
    (baseline, candidate)
}

/// A summary of `pairs`, each compared with `compare_records`.
fn summary_of(pairs: &[(ComparisonRecord, ComparisonRecord)]) -> ComparisonSummary {
    let mut summary = ComparisonSummary::default();
    for (baseline, candidate) in pairs {
        summary.observe(baseline, candidate, &compare_records(baseline, candidate));
    }
    summary
}

/// A receipt that the side did not accept (PC-D19).
fn refuse(record: &mut ComparisonRecord) {
    record.receipt_code = 429;
    record.terminal = false;
    record.privacy_risk = None;
    record.privacy_basis.clear();
    record.admission = AdmissionLabel::Refused;
    unscore(record);
}

/// A trace that the gate did not see.
fn unscore(record: &mut ComparisonRecord) {
    record.scored = false;
    record.gate = None;
    record.member = false;
    record.member_chunks.clear();
    record.credit_events.clear();
}

#[test]
fn a_kept_label_wins_over_a_refused_pair() {
    let summary = summary_of(&[verdict_pair(refuse)]);
    assert_eq!(summary.refused_total(), 2);
    for label in ["compare_http_failed", COMPARISON_ALIGNMENT_LABEL] {
        for partial in [true, false] {
            assert_eq!(run_verdict(Some(label), &summary, partial), Err(label));
        }
    }
    // A kept label fails a run whose pairs are all equal, too.
    let equal = summary_of(&[verdict_pair(|_| {})]);
    assert_eq!(equal.unexplained_total(), 0);
    assert_eq!(
        run_verdict(Some("compare_database_failed"), &equal, true),
        Err("compare_database_failed")
    );
}

#[test]
fn a_refused_pair_that_is_unexplained_is_reported_as_refused() {
    let summary = summary_of(&[verdict_pair(refuse)]);
    // Two refused records are not terminal, so the pair is unexplained too.
    assert_eq!(summary.unexplained_total(), 1);
    for partial in [true, false] {
        assert_eq!(
            run_verdict(None, &summary, partial),
            Err("comparison_receipt_refused")
        );
    }
    // One refused side is sufficient.
    let (baseline, mut candidate) = verdict_pair(|_| {});
    refuse(&mut candidate);
    let one_side = summary_of(&[(baseline, candidate)]);
    assert_eq!(one_side.refused_total(), 1);
    assert_eq!(
        run_verdict(None, &one_side, true),
        Err("comparison_receipt_refused")
    );
}

#[test]
fn an_unexplained_pair_fails_the_run() {
    let (baseline, mut candidate) = verdict_pair(|_| {});
    candidate.admission = AdmissionLabel::Quarantine;
    let summary = summary_of(&[(baseline, candidate)]);
    assert_eq!(summary.refused_total(), 0);
    assert_eq!(summary.unexplained_total(), 1);
    for partial in [true, false] {
        assert_eq!(
            run_verdict(None, &summary, partial),
            Err("comparison_has_unexplained_differences")
        );
    }
}

#[test]
fn a_branch_gap_fails_only_a_full_run() {
    // One pair that passed the two gates and is a member: the three `false`
    // branches have no evidence.
    let passed = verdict_pair(|_| {});
    let summary = summary_of(std::slice::from_ref(&passed));
    assert_eq!(summary.unexplained_total(), 0);
    assert_eq!(
        summary.branch_gaps(),
        [
            "quality_passed_false",
            "novelty_passed_false",
            "member_false"
        ]
    );
    assert_eq!(
        run_verdict(None, &summary, false),
        Err("comparison_gate_branch_not_exercised")
    );
    assert_eq!(run_verdict(None, &summary, true), Ok(()));

    // A second pair that failed the two gates gives the other three branches.
    let failed = verdict_pair(|record| {
        let gate = record.gate.as_mut().unwrap();
        gate.quality_passed = false;
        gate.novelty_passed = false;
        record.member = false;
        record.member_chunks.clear();
        record.credit_events.clear();
    });
    let complete = summary_of(&[passed, failed]);
    assert!(complete.branch_gaps().is_empty());
    assert_eq!(run_verdict(None, &complete, false), Ok(()));

    // A run with no scored trace has each gap.
    let empty = summary_of(&[verdict_pair(unscore)]);
    assert_eq!(empty.unexplained_total(), 0);
    assert_eq!(
        run_verdict(None, &empty, false),
        Err("comparison_gate_branch_not_exercised")
    );
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

/// The steps of a prose fixture, and one `write_file` call whose `content`
/// argument has `bytes` bytes of words or a few more. The redaction keeps
/// this argument: the tool name has the filesystem profile. It replaces a
/// long text of a tool that it does not know with a marker.
fn large_argument_steps(seed: &str, bytes: usize) -> Vec<TraceStep> {
    let mut content = String::with_capacity(bytes + 16);
    let mut index = 0;
    while content.len() < bytes {
        content.push_str(&format!("{seed}{} ", (index * 7 + seed.len()) % 61));
        index += 1;
    }
    let mut steps = prose_steps(&long_text(seed));
    steps.push(TraceStep {
        request_hint: None,
        response: TraceResponse::ToolCalls {
            tool_calls: vec![TraceToolCall {
                id: "call_1".to_string(),
                name: "write_file".to_string(),
                arguments: serde_json::json!({ "content": content }),
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
            content: "The file is written.".to_string(),
            input_tokens: 1,
            output_tokens: 1,
        },
        expected_tool_results: vec![ExpectedToolResult {
            tool_call_id: "call_1".to_string(),
            name: "write_file".to_string(),
            content: "written".to_string(),
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
