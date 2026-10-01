// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `pipeline.py run` and `pipeline.py package` (P4-D3: no launcher binary,
//! no cargo feature). Two ignored tests are the implementation, and
//! `pipeline.py` starts each one with `--ignored --exact`:
//!
//! - `pipeline_corpus_run` serves the shared ingest app
//!   (`pipeline_runtime::run_pipeline_app`, the router and worker ingest
//!   boots) on a loopback port, with production auth and static test
//!   credentials, and drives every corpus fixture through real HTTP: the
//!   receipt, the review routes for a quarantine, the contributor status
//!   route and the administrator forensic route for the outcome, a replay,
//!   a changed-content conflict, and another tenant's reads. It writes a
//!   bounded, hash-only report and, when every fixture met its expectations,
//!   emits one check result.
//! - `pipeline_package_write` builds a named bundle's package, signs it, and
//!   writes the signed package and its trusted key.
//!
//! Ported from `ef97a459:crates/trace-commons-server/src/bin/trace-commons-pipeline-local.rs`:
//! the corpus file and report types (lines 292-470), the request builder
//! (lines 1580-1633), and the per-fixture checks of `fixture_report` (lines
//! 1794-1927), which here read through HTTP instead of the removed inspect
//! route. Nested inside `tests` beside `pipeline_http_pg_tests`, whose
//! `pub(super)` helpers it reuses.

use super::*;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use super::pipeline_http_pg_tests::{
    PassThroughPipelinePrivacyBoundary, TEST_PIPELINE_CREDIT_ISSUER, account_owner_backend,
    allow_all_test_authority, compatibility_reference_package, join_within, mains_database,
    minimal_storage_rebate_package, post_submission_status, post_trace, runtime_backend, send_http,
    serve_pipeline_app, wait_for_pipeline_ready,
};
use trace_commons_gate_api::pipeline::{
    AtomicUnits, BundlePackage, InstrumentId, ReasonCode, ReviewRecommendation,
};
use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer, SettlementAdapter};
use trace_commons_protocol::trace_contribution::{
    RawTraceCaptureTurn, RawTraceContribution, TracePipelineStatusUpdate,
};
use trace_commons_server::versioned_pipeline::{PipelineCaps, PipelineServiceBuilder};
use trace_commons_server::versioned_pipeline_bundle::PipelineBundleConfig;
use trace_commons_server::versioned_pipeline_compat::{
    COMPATIBILITY_SCORE_IMPLEMENTATION, CompatibilityBundleConfig,
};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapterRegistry,
};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_product::{
    PipelineForensicTrace, PipelineProductStore,
};
use trace_commons_server::versioned_pipeline_qualification::{
    BundlePackageTrustStore, PipelineCheckEmitter, SignedBundlePackage, TrustedBundleKey,
    package_digests, sign_bundle_package, trusted_key_for_pkcs8,
};

const CORPUS_SCHEMA: &str = "trace_commons.pipeline_corpus.v1";
const CORPUS_REPORT_SCHEMA: &str = "trace_commons.pipeline_corpus_report.v1";

/// The check ids `pipeline_corpus_run` may emit. `pipeline.py` chooses one:
/// `_<bundle>` for a built-in bundle, `_package` for a signed package, and
/// `_hf_local` for an HF pin (two partitions). `pipeline_http_corpus_package`
/// is not a required check (controller ruling PF-1).
const CORPUS_CHECK_IDS: [&str; 4] = [
    "pipeline_http_corpus_minimal",
    "pipeline_http_corpus_compatibility",
    "pipeline_http_corpus_hf_local",
    "pipeline_http_corpus_package",
];

/// Every corpus report is local test evidence (the port's `lab.py`
/// `BLOCKERS`).
const LOCAL_BLOCKERS: [&str; 6] = [
    "local_test_only",
    "local_reference_scorer",
    "local_reference_embedder",
    "synthetic_index",
    "synthetic_settlement",
    "static_bearer_authentication",
];

const CORPUS_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_CORPUS_PATH";
const CORPUS_HOLDOUT_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_CORPUS_HOLDOUT_PATH";
const CORPUS_CHECK_ID_VAR: &str = "TRACE_COMMONS_PIPELINE_CORPUS_CHECK_ID";
const CORPUS_BUNDLE_VAR: &str = "TRACE_COMMONS_PIPELINE_CORPUS_BUNDLE";
const CORPUS_PACKAGE_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_CORPUS_PACKAGE_PATH";
const CORPUS_TRUSTED_KEY_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_CORPUS_TRUSTED_KEY_PATH";
const CORPUS_REPORT_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_CORPUS_REPORT_PATH";
pub(super) const ARTIFACT_ROOT_VAR: &str = "TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT";
pub(super) const TEST_MASTER_KEY_VAR: &str = "TRACE_COMMONS_PIPELINE_TEST_MASTER_KEY_HEX";
const PACKAGE_BUNDLE_VAR: &str = "TRACE_COMMONS_PIPELINE_PACKAGE_BUNDLE";
const PACKAGE_OUTPUT_VAR: &str = "TRACE_COMMONS_PIPELINE_PACKAGE_OUTPUT";
const TRUSTED_KEY_OUTPUT_VAR: &str = "TRACE_COMMONS_PIPELINE_TRUSTED_KEY_OUTPUT";
const PACKAGE_SIGNING_KEY_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_PACKAGE_SIGNING_KEY_PATH";
const PACKAGE_KEY_ID_VAR: &str = "TRACE_COMMONS_PIPELINE_PACKAGE_KEY_ID";

/// The tenant routed to the pipeline, and a configured tenant that is not:
/// its credentials must see nothing of the first tenant's runs.
const CORPUS_TENANT: &str = "tenant-lab";
const OTHER_TENANT: &str = "tenant-other";
const CONTRIBUTOR_TOKEN: &str = "token-corpus-lab-contributor";
const REVIEWER_TOKEN: &str = "token-corpus-lab-reviewer";
const ADMIN_TOKEN: &str = "token-corpus-lab-admin";
const OTHER_CONTRIBUTOR_TOKEN: &str = "token-corpus-other-contributor";
const OTHER_ADMIN_TOKEN: &str = "token-corpus-other-admin";

/// The harness's own bearer tokens. They are probes too: none may appear in
/// a request body, a response body, the report, or the evidence.
const HARNESS_TOKENS: [&str; 5] = [
    CONTRIBUTOR_TOKEN,
    REVIEWER_TOKEN,
    ADMIN_TOKEN,
    OTHER_CONTRIBUTOR_TOKEN,
    OTHER_ADMIN_TOKEN,
];

/// The error label a pipeline receipt answers for changed content under an
/// idempotency key already used (`pipeline_content_conflict` in
/// `trace-commons-ingest.rs`).
const RECEIPT_CONTENT_CONFLICT_LABEL: &str = "receipt id reused with different content";

/// The `compatibility` bundle's `NoveltyUtility` delta, as the PR 3
/// compatibility HTTP test configures it.
pub(super) const COMPATIBILITY_NOVELTY_UTILITY_MICROCREDITS: u64 = 2_500_000;

/// How long one fixture's run may take to become terminal.
const FIXTURE_BOUND: std::time::Duration = std::time::Duration::from_secs(60);

/// A run the worker finished: `complete`, or `rejected` (by Admission or by
/// Review; its run is complete too).
const TERMINAL_STATES: [&str; 2] = ["complete", "rejected"];

/// The generated signing key's id when `pipeline.py package` is given none.
const EPHEMERAL_KEY_ID: &str = "local_pipeline_ephemeral";

// ---------------------------------------------------------------------------
// The corpus file (port lines 292-392).
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CorpusFile {
    schema: String,
    pub(super) fixtures: Vec<CorpusFixture>,
}

/// One fixture. The port's defaults stand for the decision fields; the
/// expected states and instrument count are optional, and a missing one is
/// derived by `expectations` (P4-D16), since `main`'s #971 corpus leaves
/// them out. Unknown fields are refused: a misspelled expectation must not
/// fall back to a default silently.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CorpusFixture {
    label: String,
    trace_id: Uuid,
    submission_id: Uuid,
    created_at: DateTime<Utc>,
    input: String,
    secret_probe: String,
    #[serde(default)]
    server_privacy_probe: Option<String>,
    #[serde(default = "default_privacy_risk")]
    privacy_risk: String,
    #[serde(default = "default_admission_decision")]
    expected_admission_decision: String,
    #[serde(default = "default_outcome_count")]
    expected_outcome_count: usize,
    #[serde(default)]
    review_recommendation: Option<ReviewRecommendation>,
    #[serde(default)]
    expected_consent_state: Option<String>,
    #[serde(default)]
    expected_privacy_state: Option<String>,
    #[serde(default)]
    expected_scoring_state: Option<String>,
    #[serde(default)]
    expected_settlement_state: Option<String>,
    #[serde(default)]
    expected_instrument_count: Option<usize>,
}

fn default_privacy_risk() -> String {
    "low".to_string()
}

fn default_admission_decision() -> String {
    "admit".to_string()
}

const fn default_outcome_count() -> usize {
    4
}

impl CorpusFile {
    /// The port's `CorpusFile::validate` rules, as safe labels.
    fn validate(&self) -> Result<(), &'static str> {
        if self.schema != CORPUS_SCHEMA {
            return Err("unsupported_corpus_schema");
        }
        if self.fixtures.is_empty() {
            return Err("empty_corpus");
        }
        let mut labels = BTreeSet::new();
        let mut traces = BTreeSet::new();
        let mut submissions = BTreeSet::new();
        for fixture in &self.fixtures {
            ReasonCode::new(&fixture.label).map_err(|_| "unsafe_fixture_label")?;
            if !(labels.insert(fixture.label.as_str())
                && traces.insert(fixture.trace_id)
                && submissions.insert(fixture.submission_id))
            {
                return Err("duplicate_corpus_identity");
            }
            if fixture.secret_probe.is_empty()
                || fixture.server_privacy_probe.as_deref() == Some("")
            {
                return Err("empty_secret_probe");
            }
            if !matches!(fixture.privacy_risk.as_str(), "low" | "medium" | "high") {
                return Err("invalid_privacy_risk");
            }
            if !matches!(
                fixture.expected_admission_decision.as_str(),
                "admit" | "quarantine" | "reject"
            ) {
                return Err("invalid_expected_admission_decision");
            }
            if !matches!(fixture.expected_outcome_count, 1 | 2 | 4) {
                return Err("invalid_expected_outcome_count");
            }
            if fixture
                .expected_instrument_count
                .is_some_and(|count| count > 16)
            {
                return Err("invalid_expected_instrument_count");
            }
        }
        Ok(())
    }
}

/// Reads and validates a corpus file, returning it with the `sha256:` digest
/// of its bytes.
pub(super) fn load_corpus(path: &Path) -> Result<(CorpusFile, String), &'static str> {
    let bytes = std::fs::read(path).map_err(|_| "corpus_unreadable")?;
    let corpus: CorpusFile = serde_json::from_slice(&bytes).map_err(|_| "corpus_invalid")?;
    corpus.validate()?;
    Ok((corpus, sha256_bytes(&bytes)))
}

/// What one fixture must produce: the fixture's own values, or, for a
/// missing field, P4-D16's derivation. Privacy follows `privacy_risk`;
/// scoring and settlement are `complete` for a four-outcome run and
/// `missing` and `incomplete` otherwise; consent is `allowed`; the
/// instrument count is the bundle's award count for a four-outcome run and
/// 0 otherwise. Each instrument leg's internal settlement state is the one
/// the bundle's award reaches when it is credited (ruling T9-6:
/// `bundle_award_states`), and there are no legs otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FixtureExpectations {
    admission_decision: String,
    outcome_count: usize,
    consent_state: String,
    privacy_state: String,
    scoring_state: String,
    settlement_state: String,
    instrument_count: usize,
    instrument_states: BTreeMap<String, String>,
}

impl CorpusFixture {
    fn expectations(&self, award_states: &BTreeMap<String, String>) -> FixtureExpectations {
        let complete = self.expected_outcome_count == 4;
        FixtureExpectations {
            admission_decision: self.expected_admission_decision.clone(),
            outcome_count: self.expected_outcome_count,
            consent_state: self
                .expected_consent_state
                .clone()
                .unwrap_or_else(|| "allowed".to_string()),
            privacy_state: self
                .expected_privacy_state
                .clone()
                .unwrap_or_else(|| self.privacy_risk.clone()),
            scoring_state: self
                .expected_scoring_state
                .clone()
                .unwrap_or_else(|| if complete { "complete" } else { "missing" }.to_string()),
            settlement_state: self
                .expected_settlement_state
                .clone()
                .unwrap_or_else(|| if complete { "complete" } else { "incomplete" }.to_string()),
            instrument_count: self.expected_instrument_count.unwrap_or(if complete {
                award_states.len()
            } else {
                0
            }),
            instrument_states: if complete {
                award_states.clone()
            } else {
                BTreeMap::new()
            },
        }
    }
}

/// The instrument legs one admitted run of `package` settles, each with the
/// internal settlement state a credited leg reaches (the status route's
/// `internal_settlement_state`). A minimal-family bundle settles each
/// configured award: `trace_credit` as a finalized batch (its `Accepted`
/// ledger event is batched and finalized in the Settle transaction), any
/// other instrument `not_applicable`. A compatibility bundle whose
/// `NoveltyUtility` delta is positive settles one `trace_credit` leg as
/// `not_settlement_eligible` (credited with a ledger event `main` never
/// batches; `withheld` is a refused credit). Its local floors are zero, so
/// every scored run passes them.
fn bundle_award_states(package: &BundlePackage) -> Result<BTreeMap<String, String>, &'static str> {
    let config = package
        .artifacts
        .get(&package.manifest.score.configuration_hash)
        .ok_or("corpus_package_configuration_missing")?;
    let trace_credit = InstrumentId::trace_credit().as_str().to_string();
    if package.manifest.score.implementation_id == COMPATIBILITY_SCORE_IMPLEMENTATION {
        let config: CompatibilityBundleConfig =
            serde_json::from_slice(config).map_err(|_| "corpus_package_configuration_invalid")?;
        Ok(if config.novelty_utility_microcredits > 0 {
            BTreeMap::from([(trace_credit, "not_settlement_eligible".to_string())])
        } else {
            BTreeMap::new()
        })
    } else {
        let config: PipelineBundleConfig =
            serde_json::from_slice(config).map_err(|_| "corpus_package_configuration_invalid")?;
        Ok(config
            .instrument_awards
            .iter()
            .map(|award| {
                let state = if award.instrument_id == trace_credit {
                    "finalized"
                } else {
                    "not_applicable"
                };
                (award.instrument_id.clone(), state.to_string())
            })
            .collect())
    }
}

// ---------------------------------------------------------------------------
// Configuration (the variables `pipeline.py run` and `package` set).
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum CorpusPackageSource {
    Bundle(String),
    Signed {
        package: PathBuf,
        trusted_key: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CorpusRunConfig {
    /// `corpus`, or `bootstrap` then `holdout` (an HF pin): run in this
    /// order against one app, one database, and one index.
    partitions: Vec<(&'static str, PathBuf)>,
    check_id: String,
    source: CorpusPackageSource,
    report_path: PathBuf,
    artifact_root: Option<PathBuf>,
    master_key_hex: Option<String>,
}

impl CorpusRunConfig {
    /// `Ok(None)` when none of the corpus variables is set (nothing to run);
    /// otherwise every required one must be set and valid.
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, &'static str> {
        let corpus = var(CORPUS_PATH_VAR);
        let holdout = var(CORPUS_HOLDOUT_PATH_VAR);
        let check_id = var(CORPUS_CHECK_ID_VAR);
        let bundle = var(CORPUS_BUNDLE_VAR);
        let package = var(CORPUS_PACKAGE_PATH_VAR);
        let trusted_key = var(CORPUS_TRUSTED_KEY_PATH_VAR);
        let report = var(CORPUS_REPORT_PATH_VAR);
        if [
            &corpus,
            &holdout,
            &check_id,
            &bundle,
            &package,
            &trusted_key,
            &report,
        ]
        .iter()
        .all(|value| value.is_none())
        {
            return Ok(None);
        }
        let corpus = corpus.ok_or("corpus_path_missing")?;
        let check_id = check_id.ok_or("corpus_check_id_missing")?;
        if !CORPUS_CHECK_IDS.contains(&check_id.as_str()) {
            return Err("corpus_check_id_invalid");
        }
        let source = match (bundle, package, trusted_key) {
            (Some(_), Some(_), _) | (Some(_), _, Some(_)) => {
                return Err("corpus_package_and_bundle_conflict");
            }
            (Some(bundle), None, None) => {
                if !matches!(bundle.as_str(), "minimal" | "compatibility") {
                    return Err("corpus_bundle_invalid");
                }
                CorpusPackageSource::Bundle(bundle)
            }
            (None, Some(package), Some(trusted_key)) => CorpusPackageSource::Signed {
                package: PathBuf::from(package),
                trusted_key: PathBuf::from(trusted_key),
            },
            (None, Some(_), None) | (None, None, Some(_)) => {
                return Err("corpus_package_and_key_required");
            }
            (None, None, None) => return Err("corpus_bundle_or_package_required"),
        };
        let report_path = PathBuf::from(report.ok_or("corpus_report_path_missing")?);
        let partitions = match holdout {
            Some(holdout) => vec![
                ("bootstrap", PathBuf::from(corpus)),
                ("holdout", PathBuf::from(holdout)),
            ],
            None => vec![("corpus", PathBuf::from(corpus))],
        };
        Ok(Some(Self {
            partitions,
            check_id,
            source,
            report_path,
            artifact_root: var(ARTIFACT_ROOT_VAR).map(PathBuf::from),
            master_key_hex: var(TEST_MASTER_KEY_VAR),
        }))
    }

    /// The package to serve: a built-in bundle, or a signed package that
    /// `BundlePackageTrustStore` verifies against its trusted key.
    fn package(&self) -> Result<BundlePackage, String> {
        match &self.source {
            CorpusPackageSource::Bundle(bundle) => {
                corpus_bundle_package(bundle).map_err(|_| "corpus_bundle_invalid".to_string())
            }
            CorpusPackageSource::Signed {
                package,
                trusted_key,
            } => {
                let signed: SignedBundlePackage = serde_json::from_slice(
                    &std::fs::read(package).map_err(|_| "corpus_package_unreadable")?,
                )
                .map_err(|_| "corpus_package_invalid")?;
                let key: TrustedBundleKey = serde_json::from_slice(
                    &std::fs::read(trusted_key).map_err(|_| "corpus_trusted_key_unreadable")?,
                )
                .map_err(|_| "corpus_trusted_key_invalid")?;
                BundlePackageTrustStore::new([key])?.verify(&signed)?;
                Ok(signed.package)
            }
        }
    }
}

/// The built-in bundles, by name: PR 2's minimal package with the
/// `storage_rebate` award, and the compatibility package over
/// `local_reference()` with a 2_500_000 microcredit `NoveltyUtility` delta.
fn corpus_bundle_package(bundle: &str) -> anyhow::Result<BundlePackage> {
    let scorer = ReferencePerplexityScorer::new();
    let embedder = ReferenceEmbedder::new();
    match bundle {
        "minimal" => minimal_storage_rebate_package(&scorer, &embedder),
        "compatibility" => compatibility_reference_package(
            COMPATIBILITY_NOVELTY_UTILITY_MICROCREDITS,
            &scorer,
            &embedder,
        ),
        _ => anyhow::bail!("corpus_bundle_invalid"),
    }
}

// ---------------------------------------------------------------------------
// The app.
// ---------------------------------------------------------------------------

/// P5: the injected runtime for `pipeline_corpus_run`. It serves exactly
/// the package the report and the check result name -- a built-in bundle or
/// a verified signed package -- with the reference scorer and embedder the
/// package pins, the isolated index, one recording settlement adapter on
/// payout rail `none` for each instrument the package pins, the allow-all
/// test authority, and the pass-through privacy boundary. Payout stays
/// disabled (the default), as in `CompatibilityTestAssembler`.
struct CorpusAssembler {
    package: BundlePackage,
    index: Arc<IsolatedPipelineIndex>,
}

impl IngestPipelineRuntimeAssembler for CorpusAssembler {
    fn assemble(
        &self,
        context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        let adapters = self
            .package
            .manifest
            .instruments
            .keys()
            .map(|instrument| {
                RecordingSettlementAdapter::new(
                    instrument.clone(),
                    format!("recording_{}_corpus_test_only", instrument.as_str()),
                    "none",
                ) as Arc<dyn SettlementAdapter>
            })
            .collect();
        let caps = PipelineCaps {
            per_instrument_atomic_units: self
                .package
                .manifest
                .instruments
                .keys()
                .map(|instrument: &InstrumentId| {
                    (
                        instrument.as_str().to_string(),
                        AtomicUnits::from_raw(u128::MAX),
                    )
                })
                .collect(),
        };
        let service = PipelineServiceBuilder::new(
            context.backend,
            context.artifact_store,
            self.package.clone(),
            self.index.clone(),
            self.index.clone(),
            SettlementAdapterRegistry::new(adapters)?,
            caps,
        )
        .with_scorer(Arc::new(ReferencePerplexityScorer::new()))
        .with_embedder(Arc::new(ReferenceEmbedder::new()))
        .with_object_store_name(context.object_store_name)
        .with_novelty_utility_checks(context.novelty_utility_checks)
        .with_authority(allow_all_test_authority())
        .with_privacy(Arc::new(PassThroughPipelinePrivacyBoundary))
        .build()?;
        Ok(Arc::new(service))
    }
}

/// The service comes out of `assemble_ingest_pipeline_runtime`, the seam
/// ingest's real boot uses, exactly as the other HTTP tests' services do,
/// with the pipeline's credit issuer configured, which a runtime that
/// routes the compatibility bundle needs (PR 3: Zaki review 1, round 2,
/// finding 15); the minimal bundle ignores it.
fn assemble_corpus_service(
    backend: Arc<PgBackend>,
    artifacts: Arc<LocalEncryptedTraceArtifactStore>,
    index: Arc<IsolatedPipelineIndex>,
    package: BundlePackage,
) -> Arc<PipelineService> {
    let assembler = CorpusAssembler { package, index };
    let connections = TraceCorpusDbConnections {
        database: backend.clone() as Arc<dyn Database>,
        postgres: backend,
    };
    assemble_ingest_pipeline_runtime(
        Some(&assembler),
        Some(&connections),
        Some(&ConfiguredTraceArtifactStore::legacy(artifacts)),
        false,
        trace_commons_server::versioned_pipeline::PipelineLeaseConfig::default(),
        true,
        true,
        None,
        TEST_NEAR_CONFIRMATION_INTERVAL,
        TEST_NEAR_PAYOUT_CONTROLS,
        &PipelineNoveltyUtilityChecks {
            issuer_principal_ref: Some(TEST_PIPELINE_CREDIT_ISSUER.to_string()),
            ..PipelineNoveltyUtilityChecks::default()
        },
        TEST_EMBED_INSERT_NOVELTY_MICROS,
    )
    .expect("assemble the injected corpus pipeline runtime")
    .expect("an assembler was given, so a service is returned")
}

// ---------------------------------------------------------------------------
// The request builder (port lines 1580-1633).
// ---------------------------------------------------------------------------

/// The fixture's envelope (ruling T9-5). The port's builder gives it the
/// fixture's input, identities, creation time, and deterministic event and
/// revocation ids, and the contributor side's local redaction runs first
/// (it removes, for example, a credential in the input).
///
/// - A Low-risk fixture sends that redacted text. The server re-scrub
///   classifies message text Medium with `ConsentContentFlag` as its only
///   basis, which the pipeline reads as Low.
/// - A Medium or High fixture stays metadata only, as the PR 3
///   compatibility HTTP test builds its Medium envelope, and takes its label
///   as a tool name so that two such fixtures stay distinct.
///
/// Every envelope consents to model training, as `model_training_envelope`
/// does: the allowed use a compatibility award needs to be credited rather
/// than withheld (ruling T9-6).
pub(super) async fn fixture_envelope(fixture: &CorpusFixture) -> TraceContributionEnvelope {
    let turn = RawTraceCaptureTurn {
        user_input: fixture.input.clone(),
        response: Some("Completed the local fixture safely.".to_string()),
        tool_calls: Vec::new(),
        started_at: fixture.created_at,
        completed_at: Some(fixture.created_at + chrono::Duration::seconds(1)),
        state: Some("completed".to_string()),
    };
    let mut raw = RawTraceContribution::from_capture_turns(
        &[turn],
        RecordedTraceContributionOptions {
            include_message_text: true,
            pseudonymous_contributor_id: Some("sha256:corpus-contributor".to_string()),
            tenant_scope_ref: Some("tenant_sha256:corpus".to_string()),
            ..RecordedTraceContributionOptions::default()
        },
    );
    raw.trace_id = fixture.trace_id;
    raw.submission_id = fixture.submission_id;
    raw.created_at = fixture.created_at;
    raw.contributor.revocation_handle = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:corpus-revocation:{}", fixture.label).as_bytes(),
    );
    for (index, event) in raw.events.iter_mut().enumerate() {
        event.event_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:corpus-event:{}:{index}", fixture.label).as_bytes(),
        );
    }
    let mut envelope = DeterministicTraceRedactor::try_default()
        .expect("corpus_redactor_unavailable")
        .redact_trace(raw)
        .await
        .expect("corpus_redaction_failed");
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
        _ => unreachable!("CorpusFile::validate refuses any other privacy risk"),
    };
    envelope.consent.scopes = vec![ConsentScope::ModelTraining];
    envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
    envelope
}

// ---------------------------------------------------------------------------
// Driving one fixture through HTTP.
// ---------------------------------------------------------------------------

/// The HTTP client, every request body the harness sent, and every
/// response body it received (success bodies too): none may carry a probe.
struct CorpusHttp {
    client: reqwest::Client,
    base: String,
    sent_bodies: Vec<Vec<u8>>,
    received_bodies: Vec<String>,
}

impl CorpusHttp {
    fn record(&mut self, body: &serde_json::Value) {
        self.received_bodies.push(body.to_string());
    }

    async fn submit(
        &mut self,
        token: &str,
        body: &[u8],
    ) -> (reqwest::StatusCode, serde_json::Value) {
        self.sent_bodies.push(body.to_vec());
        let (status, value) = post_trace(&self.client, &self.base, token, body).await;
        self.record(&value);
        (status, value)
    }

    async fn status_documents(
        &mut self,
        token: &str,
        submission_id: Uuid,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        // The body `post_submission_status` sends, serialized the same way.
        self.sent_bodies.push(
            serde_json::json!({ "submission_ids": [submission_id] })
                .to_string()
                .into_bytes(),
        );
        let (status, value) =
            post_submission_status(&self.client, &self.base, token, &[submission_id]).await;
        self.record(&value);
        (status, value)
    }

    async fn send(
        &mut self,
        method: reqwest::Method,
        path: &str,
        token: &str,
        body: Option<serde_json::Value>,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        if let Some(body) = &body {
            // `send_http` sends `body.to_string()`.
            self.sent_bodies.push(body.to_string().into_bytes());
        }
        let (status, value) = send_http(
            &self.client,
            method,
            format!("{}{path}", self.base),
            auth_headers(token),
            body,
        )
        .await;
        self.record(&value);
        (status, value)
    }

    /// The pipeline block of `submission_id`'s status document, read with
    /// the contributor credential; `None` until the route has one.
    async fn pipeline_status(&mut self, submission_id: Uuid) -> Option<TracePipelineStatusUpdate> {
        let (status, documents) = self
            .status_documents(CONTRIBUTOR_TOKEN, submission_id)
            .await;
        if status != reqwest::StatusCode::OK {
            return None;
        }
        documents
            .as_array()?
            .iter()
            .find(|document| document["submission_id"] == submission_id.to_string())
            .and_then(|document| document.get("pipeline"))
            .and_then(|pipeline| serde_json::from_value(pipeline.clone()).ok())
    }

    async fn forensic(
        &mut self,
        token: &str,
        run_id: Uuid,
    ) -> (reqwest::StatusCode, serde_json::Value) {
        self.send(
            reqwest::Method::GET,
            &format!("/v1/admin/pipeline/runs/{run_id}/forensic"),
            token,
            None,
        )
        .await
    }
}

/// What the harness observed of one fixture, all of it read over HTTP.
#[derive(Debug, Clone, Default)]
struct FixtureObservation {
    request_content_hash: String,
    receipt_accepted: bool,
    /// The status route's processing state when the harness stopped
    /// waiting, or `receipt_refused` / `no_run`.
    state: String,
    run_id: Option<Uuid>,
    /// The run parked in `awaiting_review`: Admission quarantined it.
    saw_quarantine: bool,
    /// The quarantine reason the review queue listed.
    admission_reason: Option<String>,
    status: Option<TracePipelineStatusUpdate>,
    forensic: Option<PipelineForensicTrace>,
    replay_same_run: bool,
    changed_content_refused: bool,
    tenant_isolation: bool,
}

/// Claims and assesses a quarantined run through the review routes with the
/// fixture's recommendation, resolving the reason the queue lists. `false`
/// when any route refuses.
async fn review_quarantine(
    http: &mut CorpusHttp,
    run_id: Uuid,
    recommendation: ReviewRecommendation,
    observed: &mut FixtureObservation,
) -> bool {
    let (status, queue) = http
        .send(
            reqwest::Method::GET,
            "/v1/review/pipeline/quarantine",
            REVIEWER_TOKEN,
            None,
        )
        .await;
    if status != reqwest::StatusCode::OK {
        return false;
    }
    let Some(reason) = queue
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["run_id"] == run_id.to_string())
        })
        .and_then(|item| item["admission_reason"].as_str())
        .map(str::to_string)
    else {
        return false;
    };
    observed.admission_reason = Some(reason.clone());
    let (status, claim) = http
        .send(
            reqwest::Method::POST,
            &format!("/v1/review/pipeline/runs/{run_id}/claim"),
            REVIEWER_TOKEN,
            None,
        )
        .await;
    if status != reqwest::StatusCode::OK {
        return false;
    }
    let (recommendation, resolved) = match recommendation {
        ReviewRecommendation::Approve => ("approve", vec![reason.clone()]),
        ReviewRecommendation::Reject => ("reject", Vec::new()),
    };
    let (status, assessment) = http
        .send(
            reqwest::Method::POST,
            &format!("/v1/review/pipeline/runs/{run_id}/assessment"),
            REVIEWER_TOKEN,
            Some(serde_json::json!({
                "lease_token": claim["lease_token"],
                "recommendation": recommendation,
                "reason": reason,
                "resolved_quarantine_reasons": resolved,
            })),
        )
        .await;
    status == reqwest::StatusCode::OK && assessment["assessment_id"].is_string()
}

/// One fixture, in the brief's order: the receipt; for a quarantine, the
/// review; the wait (60 s bound) for a terminal run, reading the status
/// route; the forensic read; a replay of the same bytes; changed content
/// under the same idempotency key; and the other tenant's reads.
async fn drive_fixture(http: &mut CorpusHttp, fixture: &CorpusFixture) -> FixtureObservation {
    let envelope = fixture_envelope(fixture).await;
    let body = serde_json::to_vec(&envelope).expect("the envelope serialises");
    let mut changed = envelope.clone();
    changed.privacy.warnings.push("changed-content".to_string());
    let changed_body = serde_json::to_vec(&changed).expect("the changed envelope serialises");
    let mut observed = FixtureObservation {
        request_content_hash: sha256_bytes(&body),
        ..FixtureObservation::default()
    };

    let (status, receipt) = http.submit(CONTRIBUTOR_TOKEN, &body).await;
    observed.receipt_accepted =
        status == reqwest::StatusCode::OK && receipt["status"] == "processing";
    if !observed.receipt_accepted {
        observed.state = "receipt_refused".to_string();
        return observed;
    }

    observed.state = "no_run".to_string();
    let deadline = std::time::Instant::now() + FIXTURE_BOUND;
    let mut reviewed = false;
    loop {
        if let Some(pipeline) = http.pipeline_status(fixture.submission_id).await {
            observed.run_id = Some(pipeline.run_id);
            observed.state = pipeline.processing_state.clone();
            match pipeline.processing_state.as_str() {
                "complete" | "rejected" | "failed" | "withdrawn" => break,
                "awaiting_review" => {
                    observed.saw_quarantine = true;
                    if !reviewed {
                        // Parked for a human: resolve it with the fixture's
                        // recommendation, or stop if it names none.
                        let Some(recommendation) = fixture.review_recommendation else {
                            break;
                        };
                        reviewed = true;
                        if !review_quarantine(http, pipeline.run_id, recommendation, &mut observed)
                            .await
                        {
                            break;
                        }
                    }
                }
                _ => {}
            }
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let Some(run_id) = observed.run_id else {
        return observed;
    };

    observed.status = http.pipeline_status(fixture.submission_id).await;
    if let Some(status) = &observed.status {
        observed.state = status.processing_state.clone();
    }
    let (status, forensic) = http.forensic(ADMIN_TOKEN, run_id).await;
    if status == reqwest::StatusCode::OK {
        observed.forensic = serde_json::from_value(forensic).ok();
    }

    let (status, receipt) = http.submit(CONTRIBUTOR_TOKEN, &body).await;
    let replayed_run = http
        .pipeline_status(fixture.submission_id)
        .await
        .map(|pipeline| pipeline.run_id);
    observed.replay_same_run = status == reqwest::StatusCode::OK
        && receipt["status"] == "processing"
        && replayed_run == Some(run_id);

    let (status, refused) = http.submit(CONTRIBUTOR_TOKEN, &changed_body).await;
    observed.changed_content_refused = status == reqwest::StatusCode::CONFLICT
        && refused["error"] == RECEIPT_CONTENT_CONFLICT_LABEL;

    // The status route omits what the caller cannot see (as `main`'s does)
    // rather than answering 404; the forensic route answers 404.
    let (status, documents) = http
        .status_documents(OTHER_CONTRIBUTOR_TOKEN, fixture.submission_id)
        .await;
    let status_hidden = status == reqwest::StatusCode::OK
        && documents.as_array().is_some_and(|documents| {
            documents
                .iter()
                .all(|document| document["submission_id"] != fixture.submission_id.to_string())
        });
    let (status, _) = http.forensic(OTHER_ADMIN_TOKEN, run_id).await;
    observed.tenant_isolation = status_hidden && status == reqwest::StatusCode::NOT_FOUND;
    observed
}

// ---------------------------------------------------------------------------
// The report (port lines 418-470 and `fixture_report`, lines 1794-1927).
// ---------------------------------------------------------------------------

pub(super) fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    format!("sha256:{}", hex::encode(sha2::Sha256::digest(bytes)))
}

/// A run or outcome id as it may appear in the report: its hash only.
pub(super) fn id_hash(id: Uuid) -> String {
    sha256_bytes(id.to_string().as_bytes())
}

/// Admission's decision, from what HTTP shows: a run that parked for a
/// human was quarantined; a run the status route calls rejected with
/// Admission responsible was rejected; any other run with an Admission
/// outcome was admitted.
fn observed_admission_decision(observed: &FixtureObservation) -> &'static str {
    if !observed.receipt_accepted {
        return "unknown";
    }
    if observed.saw_quarantine {
        return "quarantine";
    }
    if observed.status.as_ref().is_some_and(|status| {
        status.processing_state == "rejected"
            && status.responsible_phase.as_deref() == Some("admission")
    }) {
        return "reject";
    }
    let admitted = observed.forensic.as_ref().is_some_and(|forensic| {
        forensic
            .phases
            .iter()
            .any(|phase| phase.phase == "admission")
    });
    if admitted { "admit" } else { "unknown" }
}

/// The privacy state Admission acted on, from its decision and reason
/// label: the minimal and compatibility Admission policies admit only a
/// Low-risk receipt, quarantine a Medium one as `privacy_review_required`,
/// and reject a High one as `privacy_risk_rejected`.
fn observed_privacy_state(decision: &str, admission_reason: Option<&str>) -> &'static str {
    match (decision, admission_reason) {
        ("admit", _) => "low",
        (_, Some("privacy_review_required")) => "medium",
        (_, Some("privacy_risk_rejected")) => "high",
        _ => "unknown",
    }
}

/// The labels of every expectation a fixture missed, in the order
/// `pipeline_tooling.corpus.fixture_mismatches` recomputes them.
fn fixture_mismatches(item: &serde_json::Value) -> Vec<&'static str> {
    let mut found = Vec::new();
    if !TERMINAL_STATES.contains(&item["state"].as_str().unwrap_or_default()) {
        found.push("run_not_terminal");
    }
    if item["admission_decision"] != item["expected_admission_decision"] {
        found.push("admission_decision");
    }
    if item["phase_count"] != item["expected_outcome_count"] {
        found.push("outcome_count");
    }
    for field in [
        "consent_state",
        "privacy_state",
        "scoring_state",
        "settlement_state",
    ] {
        if item[field] != item[format!("expected_{field}").as_str()] {
            found.push(field);
        }
    }
    if item["instrument_count"] != item["expected_instrument_count"] {
        found.push("instrument_count");
    }
    if item["instrument_states"] != item["expected_instrument_states"] {
        found.push("instrument_states");
    }
    for flag in [
        "replay_same_run",
        "changed_content_refused",
        "tenant_isolation",
    ] {
        if item[flag] != serde_json::Value::Bool(true) {
            found.push(flag);
        }
    }
    found
}

/// One fixture's bounded report: labels, counts, booleans, and hashes (the
/// port's `FixtureReport` fields that HTTP shows, without run ids).
fn fixture_report(
    fixture: &CorpusFixture,
    expected: &FixtureExpectations,
    observed: &FixtureObservation,
) -> serde_json::Value {
    let phases = observed
        .forensic
        .as_ref()
        .map(|forensic| forensic.phases.as_slice())
        .unwrap_or_default();
    let has_phase = |name: &str| phases.iter().any(|phase| phase.phase == name);
    let instruments = observed
        .status
        .as_ref()
        .map(|status| status.instruments.as_slice())
        .unwrap_or_default();
    let all_complete = instruments
        .iter()
        .all(|instrument| instrument.operation_state == "complete");
    let admission_decision = observed_admission_decision(observed);
    // The review queue's reason when the harness reviewed the run; else, for
    // a run Admission did not admit, the status route's reason label, which
    // is the admission reason once no phase error label is set.
    let admission_reason = observed.admission_reason.clone().or_else(|| {
        (admission_decision != "admit")
            .then(|| {
                observed
                    .status
                    .as_ref()
                    .and_then(|status| status.reason_label.clone())
            })
            .flatten()
    });
    let credit_write_state = if instruments.is_empty() {
        "not_required"
    } else if all_complete {
        "complete"
    } else if instruments
        .iter()
        .any(|instrument| instrument.operation_state == "failed")
    {
        "failed"
    } else {
        "pending"
    };
    let forensic = observed.forensic.as_ref();
    let mut item = serde_json::json!({
        "label": fixture.label,
        "run_id_hash": observed.run_id.map(id_hash),
        "submission_id_hash": id_hash(fixture.submission_id),
        "request_content_hash": observed.request_content_hash,
        "state": observed.state,
        "admission_decision": admission_decision,
        "expected_admission_decision": expected.admission_decision,
        "admission_reason": admission_reason,
        "responsible_phase": observed.status.as_ref().and_then(|status| status.responsible_phase.clone()),
        "reason_label": observed.status.as_ref().and_then(|status| status.reason_label.clone()),
        "phase_count": phases.len(),
        "expected_outcome_count": expected.outcome_count,
        "phases": phases.iter().map(|phase| serde_json::json!({
            "phase": phase.phase,
            "outcome_id_hash": id_hash(phase.outcome_id),
            "outcome_schema_id": phase.outcome_schema_id,
            "outcome_schema_version": phase.outcome_schema_version,
            "decision_hash": phase.decision_hash,
            "evidence_hash": phase.evidence_hash,
            "evaluation_hash": phase.evaluation_hash,
        })).collect::<Vec<_>>(),
        "index_command_hash": forensic.and_then(|forensic| forensic.index_command_hash.clone()),
        "index_write_state": forensic.map_or("unknown", |forensic| forensic.index_write_state.as_str()),
        "index_invalidation_state": forensic.map_or("unknown", |forensic| forensic.index_invalidation_state.as_str()),
        "credit_write_state": credit_write_state,
        "payout_state": forensic.map_or("unknown", |forensic| forensic.payout_state.as_str()),
        "instruments": instruments.iter().map(|instrument| serde_json::json!({
            "instrument_id": instrument.instrument_id,
            "atomic_units": instrument.atomic_units,
            "operation_state": instrument.operation_state,
            "internal_settlement_state": instrument.internal_settlement_state,
            "payout_rail": instrument.payout_rail,
            "payout_state": instrument.payout_state,
        })).collect::<Vec<_>>(),
        "replay_same_run": observed.replay_same_run,
        "changed_content_refused": observed.changed_content_refused,
        "tenant_isolation": observed.tenant_isolation,
        "consent_state": if observed.receipt_accepted { "allowed" } else { "refused" },
        "expected_consent_state": expected.consent_state,
        "privacy_state": observed_privacy_state(admission_decision, admission_reason.as_deref()),
        "expected_privacy_state": expected.privacy_state,
        "scoring_state": if has_phase("score") { "complete" } else { "missing" },
        "expected_scoring_state": expected.scoring_state,
        "settlement_state": if has_phase("settle") && all_complete { "complete" } else { "incomplete" },
        "expected_settlement_state": expected.settlement_state,
        "instrument_count": instruments.len(),
        "expected_instrument_count": expected.instrument_count,
        "instrument_states": instruments
            .iter()
            .map(|instrument| {
                (
                    instrument.instrument_id.clone(),
                    instrument.internal_settlement_state.clone(),
                )
            })
            .collect::<BTreeMap<_, _>>(),
        "expected_instrument_states": expected.instrument_states,
    });
    let mismatches = fixture_mismatches(&item);
    item["mismatches"] = serde_json::json!(mismatches);
    item
}

fn count_where(
    fixtures: &[serde_json::Value],
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> usize {
    fixtures.iter().filter(|item| predicate(item)).count()
}

fn is_terminal(item: &serde_json::Value) -> bool {
    TERMINAL_STATES.contains(&item["state"].as_str().unwrap_or_default())
}

fn has_mismatch(item: &serde_json::Value) -> bool {
    item["mismatches"]
        .as_array()
        .is_some_and(|mismatches| !mismatches.is_empty())
}

/// One partition's section: the corpus digest, the fixture order, and each
/// fixture's report.
fn partition_report(
    partition: &str,
    bundle_id: &str,
    corpus_digest: &str,
    fixtures: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "partition": partition,
        "bundle_id": bundle_id,
        "corpus_digest": corpus_digest,
        "fixture_order": fixtures.iter().map(|item| item["label"].clone()).collect::<Vec<_>>(),
        "expected_fixture_count": fixtures.len(),
        "completed_fixture_count": count_where(&fixtures, is_terminal),
        "failure_count": count_where(&fixtures, has_mismatch),
        "fixtures": fixtures,
    })
}

/// The whole report, with its digest over the canonical JSON of every other
/// field (`pipeline_tooling.corpus.validate_report` recomputes it).
fn corpus_report(
    check_id: &str,
    package: &BundlePackage,
    sections: Vec<serde_json::Value>,
) -> Result<serde_json::Value, String> {
    let digests = package_digests(package)?;
    let every: Vec<serde_json::Value> = sections
        .iter()
        .flat_map(|section| section["fixtures"].as_array().cloned().unwrap_or_default())
        .collect();
    let mut report = serde_json::json!({
        "schema": CORPUS_REPORT_SCHEMA,
        "scope": "local_test",
        "production_ready": false,
        "external_payout_enabled": false,
        "safe_blockers": LOCAL_BLOCKERS,
        "check_id": check_id,
        "bundle_id": package.bundle_id,
        "package_hash": digests.package_hash,
        "configuration_digest": digests.configuration_digest,
        "dependency_digest": digests.dependency_digest,
        "policy_manifest": serde_json::to_value(&package.manifest)
            .map_err(|_| "corpus_report_manifest_invalid".to_string())?,
        "fixture_count": every.len(),
        "completed_fixture_count": count_where(&every, is_terminal),
        "failure_count": count_where(&every, has_mismatch),
        "replay_same_run_count": count_where(&every, |item| item["replay_same_run"] == true),
        "changed_content_refused_count":
            count_where(&every, |item| item["changed_content_refused"] == true),
        "tenant_isolation": every.iter().all(|item| item["tenant_isolation"] == true),
        "partitions": sections,
    });
    let digest = sha256_bytes(
        &trace_commons_protocol::canonical_json::to_canonical_vec(&report)
            .map_err(|_| "corpus_report_invalid".to_string())?,
    );
    report["report_digest"] = serde_json::Value::String(digest);
    Ok(report)
}

/// Every string that must never leave the harness: each fixture's
/// `secret_probe` and `server_privacy_probe`, and the harness's own bearer
/// tokens.
fn corpus_probes<'a>(fixtures: impl Iterator<Item = &'a CorpusFixture>) -> Vec<&'a str> {
    fixtures
        .flat_map(|fixture| {
            std::iter::once(fixture.secret_probe.as_str())
                .chain(fixture.server_privacy_probe.as_deref())
        })
        .chain(HARNESS_TOKENS)
        .collect()
}

fn contains_probe(haystack: &[u8], probes: &[&str]) -> bool {
    probes.iter().any(|probe| {
        !probe.is_empty()
            && haystack
                .windows(probe.len())
                .any(|window| window == probe.as_bytes())
    })
}

/// Writes `bytes` to a fresh name beside `path`, then renames it onto
/// `path`: the report, the signed package, and the trusted key.
pub(super) fn write_atomically(path: &Path, bytes: &[u8]) {
    let parent = path.parent().expect("pipeline_output_path_has_no_parent");
    std::fs::create_dir_all(parent).expect("pipeline_output_directory_unwritable");
    let temporary = parent.join(format!(".pipeline-output-{}.tmp", Uuid::new_v4()));
    std::fs::write(&temporary, bytes).expect("pipeline_output_unwritable");
    std::fs::rename(&temporary, path).expect("pipeline_output_unwritable");
}

// ---------------------------------------------------------------------------
// The two ignored tools.
// ---------------------------------------------------------------------------

/// `pipeline.py run`. Its assertions are the test: every fixture meets its
/// (derived) expectations, no fixture probe reaches the report, the evidence,
/// or an HTTP error body, and only then is the check id emitted, once, after
/// every partition.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the implementation of `pipeline.py run`: run it through that command"]
async fn pipeline_corpus_run() {
    let config = match CorpusRunConfig::from_vars(|name| std::env::var(name).ok()) {
        Ok(Some(config)) => config,
        Ok(None) => return,
        Err(label) => panic!("{label}"),
    };
    let package = config.package().unwrap_or_else(|label| panic!("{label}"));
    let award_states = bundle_award_states(&package).unwrap_or_else(|label| panic!("{label}"));
    let corpora: Vec<(&str, CorpusFile, String)> = config
        .partitions
        .iter()
        .map(|(partition, path)| {
            let (corpus, digest) = load_corpus(path).unwrap_or_else(|label| panic!("{label}"));
            (*partition, corpus, digest)
        })
        .collect();
    // One database and one index hold every partition's runs.
    let mut labels = BTreeSet::new();
    let mut identities = BTreeSet::new();
    for fixture in corpora.iter().flat_map(|(_, corpus, _)| &corpus.fixtures) {
        assert!(
            labels.insert(fixture.label.as_str())
                && identities.insert(fixture.trace_id)
                && identities.insert(fixture.submission_id),
            "duplicate_corpus_identity"
        );
    }
    let probes = corpus_probes(
        corpora
            .iter()
            .flat_map(|(_, corpus, _)| corpus.fixtures.iter()),
    );

    let runtime = runtime_backend(8)
        .await
        .expect("corpus_database_url_missing");
    // Migrates the suite's database and resets the account rate limiter.
    account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let state_dir = tempfile::tempdir().expect("temp dir");
    let generated_root = tempfile::tempdir().expect("temp dir");
    let artifact_root = config
        .artifact_root
        .clone()
        .unwrap_or_else(|| generated_root.path().to_path_buf());
    let master_key = config
        .master_key_hex
        .clone()
        .unwrap_or_else(trace_commons_server::secrets::keychain::generate_master_key_hex);
    let artifacts = test_artifact_store_with_key(&artifact_root, &master_key);
    let service = assemble_corpus_service(
        runtime.clone(),
        artifacts.clone(),
        IsolatedPipelineIndex::new(),
        package.clone(),
    );

    let mut tokens = BTreeMap::new();
    insert_token(
        &mut tokens,
        CORPUS_TENANT,
        CONTRIBUTOR_TOKEN,
        TokenRole::Contributor,
    );
    insert_token(
        &mut tokens,
        CORPUS_TENANT,
        REVIEWER_TOKEN,
        TokenRole::Reviewer,
    );
    insert_token(&mut tokens, CORPUS_TENANT, ADMIN_TOKEN, TokenRole::Admin);
    insert_token(
        &mut tokens,
        OTHER_TENANT,
        OTHER_CONTRIBUTOR_TOKEN,
        TokenRole::Contributor,
    );
    insert_token(
        &mut tokens,
        OTHER_TENANT,
        OTHER_ADMIN_TOKEN,
        TokenRole::Admin,
    );
    // As the compatibility HTTP test: `main`'s database, the pipeline
    // service, and the product store all on the runtime login (NOBYPASSRLS,
    // NOSUPERUSER, the pilot ingest login's groups only; PR 3's
    // `mains_database`), never the owner superuser.
    let mut state = test_state_with_options(
        state_dir.path().to_path_buf(),
        Some(mains_database().await),
        Some(artifacts),
        false,
        false,
        false,
        false,
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.tokens = Arc::new(tokens);
    state_mut.pipeline_service = Some(service);
    state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[CORPUS_TENANT],
    );
    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();
    wait_for_pipeline_ready(&client, &base).await;
    let mut http = CorpusHttp {
        client,
        base,
        sent_bodies: Vec::new(),
        received_bodies: Vec::new(),
    };

    let mut sections = Vec::new();
    for (partition, corpus, digest) in &corpora {
        let mut fixtures = Vec::new();
        for fixture in &corpus.fixtures {
            let expected = fixture.expectations(&award_states);
            let observed = drive_fixture(&mut http, fixture).await;
            fixtures.push(fixture_report(fixture, &expected, &observed));
        }
        sections.push(partition_report(
            partition,
            &package.bundle_id,
            digest,
            fixtures,
        ));
    }
    stop.send(()).expect("send shutdown to the corpus app");
    join_within(server, 20, "corpus app").await;

    let report = corpus_report(&config.check_id, &package, sections)
        .unwrap_or_else(|label| panic!("{label}"));
    let mut report_bytes = trace_commons_protocol::canonical_json::to_canonical_vec(&report)
        .expect("the report serialises");
    report_bytes.push(b'\n');
    assert!(
        !http
            .sent_bodies
            .iter()
            .any(|body| contains_probe(body, &probes)),
        "corpus_probe_in_request"
    );
    assert!(
        !http
            .received_bodies
            .iter()
            .any(|body| contains_probe(body.as_bytes(), &probes)),
        "corpus_probe_in_http_body"
    );
    assert!(
        !contains_probe(&report_bytes, &probes),
        "corpus_probe_in_report"
    );
    write_atomically(&config.report_path, &report_bytes);
    // The report names each fixture's mismatches.
    assert_eq!(report["failure_count"], 0, "corpus_fixture_mismatch");

    let evidence = serde_json::json!({
        "fixtures": report["fixture_count"],
        "completed": report["completed_fixture_count"],
        "replay_same_run": report["replay_same_run_count"],
        "changed_content_refused": report["changed_content_refused_count"],
        "tenant_isolation": report["tenant_isolation"],
        "report_hash": sha256_bytes(&report_bytes),
    });
    assert!(
        !contains_probe(
            &trace_commons_protocol::canonical_json::to_canonical_vec(&evidence)
                .expect("the evidence serialises"),
            &probes
        ),
        "corpus_probe_in_evidence"
    );
    PipelineCheckEmitter::emit_pass_from_env(&config.check_id, Some(&package), evidence);
}

/// `pipeline.py package`: builds the named bundle's package, signs it with
/// the PKCS#8 key at the optional path (and its key id) or with a generated
/// one, and writes the signed package and the trusted key. Its assertion:
/// the written package verifies with the written key.
#[tokio::test]
#[ignore = "the implementation of `pipeline.py package`: run it through that command"]
async fn pipeline_package_write() {
    use ring::signature::Ed25519KeyPair;

    let var = |name: &str| std::env::var(name).ok();
    let (bundle, output, key_output) = match (
        var(PACKAGE_BUNDLE_VAR),
        var(PACKAGE_OUTPUT_VAR),
        var(TRUSTED_KEY_OUTPUT_VAR),
    ) {
        (None, None, None) => return,
        (Some(bundle), Some(output), Some(key_output)) => {
            (bundle, PathBuf::from(output), PathBuf::from(key_output))
        }
        _ => panic!("package_environment_incomplete"),
    };
    assert_ne!(output, key_output, "package_outputs_must_differ");
    let package =
        corpus_bundle_package(&bundle).unwrap_or_else(|_| panic!("package_bundle_invalid"));
    let (pkcs8, key_id) = match (var(PACKAGE_SIGNING_KEY_PATH_VAR), var(PACKAGE_KEY_ID_VAR)) {
        (Some(path), Some(key_id)) => (
            zeroize::Zeroizing::new(
                std::fs::read(path).unwrap_or_else(|_| panic!("package_signing_key_unreadable")),
            ),
            key_id,
        ),
        (None, None) => (
            zeroize::Zeroizing::new(
                Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                    .expect("package_signing_key_generation_failed")
                    .as_ref()
                    .to_vec(),
            ),
            EPHEMERAL_KEY_ID.to_string(),
        ),
        _ => panic!("package_signing_key_and_key_id_required"),
    };
    let signed = sign_bundle_package(package, &key_id, &pkcs8)
        .unwrap_or_else(|_| panic!("package_signing_failed"));
    let trusted =
        trusted_key_for_pkcs8(&key_id, &pkcs8).unwrap_or_else(|_| panic!("package_signing_failed"));
    write_atomically(
        &output,
        &serde_json::to_vec_pretty(&signed).expect("the signed package serialises"),
    );
    write_atomically(
        &key_output,
        &serde_json::to_vec_pretty(&trusted).expect("the trusted key serialises"),
    );

    let written: SignedBundlePackage =
        serde_json::from_slice(&std::fs::read(&output).expect("read the written package"))
            .expect("parse the written package");
    let written_key: TrustedBundleKey =
        serde_json::from_slice(&std::fs::read(&key_output).expect("read the written key"))
            .expect("parse the written key");
    BundlePackageTrustStore::new([written_key])
        .and_then(|trust| trust.verify(&written))
        .expect("the written package verifies with the written key");
}

// ---------------------------------------------------------------------------
// Unit tests (no database).
// ---------------------------------------------------------------------------

pub(super) fn main_corpus_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/superpowers/specs/fixtures/versioned-pipeline-minimal-corpus-v1.json")
}

fn corpus_from(value: serde_json::Value) -> CorpusFile {
    serde_json::from_value(value).expect("a corpus file parses")
}

fn fixture_value(label: &str, trace: u8, submission: u8) -> serde_json::Value {
    serde_json::json!({
        "label": label,
        "trace_id": format!("{trace:02x}{trace:02x}{trace:02x}{trace:02x}-0000-4000-8000-000000000001"),
        "submission_id": format!("{submission:02x}{submission:02x}{submission:02x}{submission:02x}-0000-4000-8000-000000000002"),
        "created_at": "2026-09-11T12:00:00Z",
        "input": "fixture input",
        "secret_probe": format!("probe_{label}"),
    })
}

#[test]
fn corpus_file_refuses_duplicate_ids_unsafe_labels_and_empty_probes() {
    let valid = || {
        serde_json::json!({
            "schema": CORPUS_SCHEMA,
            "fixtures": [fixture_value("first_fixture", 1, 2), fixture_value("second_fixture", 3, 4)],
        })
    };
    assert_eq!(corpus_from(valid()).validate(), Ok(()));
    let (main_corpus, main_digest) = load_corpus(&main_corpus_path()).expect("main's corpus loads");
    assert_eq!(main_corpus.fixtures.len(), 5);
    assert!(main_digest.starts_with("sha256:"));

    let with = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut value = valid();
        edit(&mut value);
        corpus_from(value).validate()
    };
    assert_eq!(
        with(&|value| value["schema"] = "trace_commons.pipeline_corpus.v0".into()),
        Err("unsupported_corpus_schema")
    );
    assert_eq!(
        with(&|value| value["fixtures"] = serde_json::json!([])),
        Err("empty_corpus")
    );
    assert_eq!(
        with(&|value| value["fixtures"][0]["label"] = "Not Safe!".into()),
        Err("unsafe_fixture_label")
    );
    assert_eq!(
        with(&|value| value["fixtures"][1]["label"] = "first_fixture".into()),
        Err("duplicate_corpus_identity")
    );
    assert_eq!(
        with(&|value| value["fixtures"][1]["trace_id"] = value["fixtures"][0]["trace_id"].clone()),
        Err("duplicate_corpus_identity")
    );
    assert_eq!(
        with(&|value| {
            value["fixtures"][1]["submission_id"] = value["fixtures"][0]["submission_id"].clone()
        }),
        Err("duplicate_corpus_identity")
    );
    assert_eq!(
        with(&|value| value["fixtures"][0]["secret_probe"] = "".into()),
        Err("empty_secret_probe")
    );
    assert_eq!(
        with(&|value| value["fixtures"][0]["server_privacy_probe"] = "".into()),
        Err("empty_secret_probe")
    );
    assert_eq!(
        with(&|value| value["fixtures"][0]["privacy_risk"] = "extreme".into()),
        Err("invalid_privacy_risk")
    );
    assert_eq!(
        with(&|value| value["fixtures"][0]["expected_admission_decision"] = "maybe".into()),
        Err("invalid_expected_admission_decision")
    );
    assert_eq!(
        with(&|value| value["fixtures"][0]["expected_outcome_count"] = 3.into()),
        Err("invalid_expected_outcome_count")
    );
    assert_eq!(
        with(&|value| value["fixtures"][0]["expected_instrument_count"] = 17.into()),
        Err("invalid_expected_instrument_count")
    );
    let mut misspelled = valid();
    misspelled["fixtures"][0]["expected_admision_decision"] = "reject".into();
    assert!(
        serde_json::from_value::<CorpusFile>(misspelled).is_err(),
        "a misspelled expectation is refused, not defaulted"
    );
}

#[test]
fn fixture_expectations_default_from_the_outcome_count() {
    let (corpus, _) = load_corpus(&main_corpus_path()).expect("main's corpus loads");
    let minimal = corpus_bundle_package("minimal").expect("the minimal package builds");
    let compatibility =
        corpus_bundle_package("compatibility").expect("the compatibility package builds");
    let storage_rebate =
        BTreeMap::from([("storage_rebate".to_string(), "not_applicable".to_string())]);
    let credited = BTreeMap::from([(
        "trace_credit".to_string(),
        "not_settlement_eligible".to_string(),
    )]);
    assert_eq!(bundle_award_states(&minimal), Ok(storage_rebate.clone()));
    assert_eq!(bundle_award_states(&compatibility), Ok(credited.clone()));
    let scorer = ReferencePerplexityScorer::new();
    let embedder = ReferenceEmbedder::new();
    let no_delta = compatibility_reference_package(0, &scorer, &embedder).unwrap();
    assert_eq!(bundle_award_states(&no_delta), Ok(BTreeMap::new()));

    let fixture = |label: &str| {
        corpus
            .fixtures
            .iter()
            .find(|fixture| fixture.label == label)
            .expect("main's corpus names this fixture")
    };
    assert_eq!(
        fixture("clean_tool_plan").expectations(&storage_rebate),
        FixtureExpectations {
            admission_decision: "admit".into(),
            outcome_count: 4,
            consent_state: "allowed".into(),
            privacy_state: "low".into(),
            scoring_state: "complete".into(),
            settlement_state: "complete".into(),
            instrument_count: 1,
            instrument_states: storage_rebate.clone(),
        }
    );
    assert_eq!(
        fixture("privacy_quarantine_approved").expectations(&credited),
        FixtureExpectations {
            admission_decision: "quarantine".into(),
            outcome_count: 4,
            consent_state: "allowed".into(),
            privacy_state: "medium".into(),
            scoring_state: "complete".into(),
            settlement_state: "complete".into(),
            instrument_count: 1,
            instrument_states: credited.clone(),
        }
    );
    assert_eq!(
        fixture("privacy_risk_rejected").expectations(&credited),
        FixtureExpectations {
            admission_decision: "reject".into(),
            outcome_count: 1,
            consent_state: "allowed".into(),
            privacy_state: "high".into(),
            scoring_state: "missing".into(),
            settlement_state: "incomplete".into(),
            instrument_count: 0,
            instrument_states: BTreeMap::new(),
        }
    );
    let review_rejected = fixture("privacy_quarantine_rejected").expectations(&credited);
    assert_eq!(
        (
            review_rejected.scoring_state.as_str(),
            review_rejected.settlement_state.as_str(),
            review_rejected.instrument_count,
            review_rejected.instrument_states.len(),
        ),
        ("missing", "incomplete", 0, 0)
    );

    // An HF export states every expectation; those win over the derivation.
    let mut explicit = fixture_value("hf_bootstrap_0000", 5, 6);
    explicit["expected_instrument_count"] = 3.into();
    explicit["expected_privacy_state"] = "medium".into();
    let explicit: CorpusFixture = serde_json::from_value(explicit).unwrap();
    let expected = explicit.expectations(&credited);
    assert_eq!(expected.instrument_count, 3);
    assert_eq!(expected.privacy_state, "medium");
    assert_eq!(expected.instrument_states, credited);
}

/// Rulings T9-5 and T9-6: a Low-risk fixture sends its text after local
/// redaction (so `locally_redacted_secret`'s credential is gone from the
/// request bytes), a Medium or High fixture sends metadata only with its
/// label as a distinct tool name, and every envelope allows model training.
/// The probe set holds every fixture probe and the harness's own tokens.
#[tokio::test]
async fn fixture_envelopes_send_redacted_text_for_low_risk_and_metadata_otherwise() {
    let (corpus, _) = load_corpus(&main_corpus_path()).expect("main's corpus loads");
    let probes = corpus_probes(corpus.fixtures.iter());
    for token in HARNESS_TOKENS {
        assert!(probes.contains(&token), "{token} is a probe");
    }
    for fixture in &corpus.fixtures {
        assert!(probes.contains(&fixture.secret_probe.as_str()));
        let envelope = fixture_envelope(fixture).await;
        let bytes = serde_json::to_vec(&envelope).unwrap();
        assert!(
            !contains_probe(&bytes, &probes),
            "{}: no probe in the request bytes",
            fixture.label
        );
        assert_eq!(envelope.consent.scopes, vec![ConsentScope::ModelTraining]);
        assert_eq!(
            envelope.trace_card.allowed_uses,
            vec![TraceAllowedUse::ModelTraining]
        );
        let text: Vec<&str> = envelope
            .events
            .iter()
            .filter_map(|event| event.redacted_content.as_deref())
            .collect();
        if fixture.privacy_risk == "low" {
            assert_eq!(envelope.privacy.residual_pii_risk, ResidualPiiRisk::Low);
            assert!(envelope.consent.message_text_included);
            assert!(!text.is_empty(), "{}: the text is sent", fixture.label);
        } else {
            assert!(text.is_empty(), "{}: metadata only", fixture.label);
            assert!(
                envelope
                    .events
                    .iter()
                    .any(|event| event.tool_name.as_deref() == Some(fixture.label.as_str())),
                "{}: its label is its tool name",
                fixture.label
            );
        }
    }
    let clean = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.label == "clean_tool_plan")
        .unwrap();
    let clean_bytes = serde_json::to_vec(&fixture_envelope(clean).await).unwrap();
    assert!(contains_probe(&clean_bytes, &["smallest safe change"]));
    let secret = corpus
        .fixtures
        .iter()
        .find(|fixture| fixture.label == "locally_redacted_secret")
        .unwrap();
    assert!(
        secret.input.contains(&secret.secret_probe),
        "the fixture's input holds its probe until local redaction removes it"
    );
}

#[test]
fn corpus_run_config_refuses_bad_check_ids_and_a_package_with_a_bundle() {
    let config = |pairs: &[(&str, &str)]| {
        let vars: BTreeMap<String, String> = pairs
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect();
        CorpusRunConfig::from_vars(|name| vars.get(name).cloned())
    };
    assert_eq!(config(&[]), Ok(None), "nothing set: nothing to run");
    let base = [
        (CORPUS_PATH_VAR, "/tmp/corpus.json"),
        (CORPUS_CHECK_ID_VAR, "pipeline_http_corpus_minimal"),
        (CORPUS_BUNDLE_VAR, "minimal"),
        (CORPUS_REPORT_PATH_VAR, "/tmp/report.json"),
    ];
    let parsed = config(&base).unwrap().expect("a complete configuration");
    assert_eq!(
        parsed.partitions,
        vec![("corpus", PathBuf::from("/tmp/corpus.json"))]
    );
    assert_eq!(parsed.source, CorpusPackageSource::Bundle("minimal".into()));

    let mut bad_check = base;
    bad_check[1].1 = "pipeline_http_corpus_other";
    assert_eq!(config(&bad_check), Err("corpus_check_id_invalid"));
    let mut dotted_check = base;
    dotted_check[1].1 = "pipeline.http.corpus.minimal";
    assert_eq!(config(&dotted_check), Err("corpus_check_id_invalid"));

    let both: Vec<(&str, &str)> = base
        .iter()
        .copied()
        .chain([
            (CORPUS_PACKAGE_PATH_VAR, "/tmp/package.json"),
            (CORPUS_TRUSTED_KEY_PATH_VAR, "/tmp/key.json"),
        ])
        .collect();
    assert_eq!(config(&both), Err("corpus_package_and_bundle_conflict"));

    let package_only: Vec<(&str, &str)> = base
        .iter()
        .copied()
        .filter(|(name, _)| *name != CORPUS_BUNDLE_VAR)
        .chain([(CORPUS_PACKAGE_PATH_VAR, "/tmp/package.json")])
        .collect();
    assert_eq!(
        config(&package_only),
        Err("corpus_package_and_key_required")
    );

    let two_partitions: Vec<(&str, &str)> = base
        .iter()
        .copied()
        .chain([(CORPUS_HOLDOUT_PATH_VAR, "/tmp/holdout.json")])
        .collect();
    assert_eq!(
        config(&two_partitions).unwrap().unwrap().partitions,
        vec![
            ("bootstrap", PathBuf::from("/tmp/corpus.json")),
            ("holdout", PathBuf::from("/tmp/holdout.json")),
        ]
    );
}

#[test]
fn fixture_report_hashes_run_ids_and_names_each_mismatch() {
    let fixture: CorpusFixture =
        serde_json::from_value(fixture_value("clean_fixture", 7, 8)).unwrap();
    let expected = fixture.expectations(&BTreeMap::from([(
        "storage_rebate".to_string(),
        "not_applicable".to_string(),
    )]));
    let run_id = Uuid::new_v4();
    let outcome_id = Uuid::new_v4();
    let phase = |name: &str| trace_commons_server::versioned_pipeline_product::PipelinePhaseTrace {
        phase: name.to_string(),
        outcome_id,
        outcome_schema_id: format!("trace_commons.{name}_outcome"),
        outcome_schema_version: 1,
        decision_hash: sha256_bytes(b"decision"),
        evidence_hash: sha256_bytes(b"evidence"),
        evaluation_hash: sha256_bytes(b"evaluation"),
    };
    let instrument = trace_commons_protocol::trace_contribution::TraceInstrumentStatusUpdate {
        instrument_id: "storage_rebate".into(),
        atomic_units: "5".into(),
        operation_state: "complete".into(),
        internal_settlement_state: "not_applicable".into(),
        payout_rail: "none".into(),
        payout_state: "disabled".into(),
        reason_label: None,
    };
    let observed = FixtureObservation {
        request_content_hash: sha256_bytes(b"request"),
        receipt_accepted: true,
        state: "complete".into(),
        run_id: Some(run_id),
        saw_quarantine: false,
        admission_reason: None,
        status: Some(TracePipelineStatusUpdate {
            run_id,
            bundle_id: sha256_bytes(b"bundle"),
            processing_state: "complete".into(),
            current_phase: None,
            responsible_phase: Some("settle".into()),
            reason_label: None,
            instruments: vec![instrument],
        }),
        forensic: Some(PipelineForensicTrace {
            run_id,
            submission_id: fixture.submission_id,
            bundle_id: sha256_bytes(b"bundle"),
            phases: ["admission", "review", "score", "settle"]
                .into_iter()
                .map(phase)
                .collect(),
            index_command_hash: Some(sha256_bytes(b"index")),
            index_write_state: "complete".into(),
            score_outcome_id: Some(outcome_id),
            credit_event_id: None,
            settlement_batch_id: None,
            payout_state: "none".into(),
            instruments: Vec::new(),
            index_invalidation_state: "none".into(),
        }),
        replay_same_run: true,
        changed_content_refused: true,
        tenant_isolation: true,
    };

    let report = fixture_report(&fixture, &expected, &observed);
    assert_eq!(report["mismatches"], serde_json::json!([]), "{report}");
    assert_eq!(
        report["instrument_states"],
        serde_json::json!({"storage_rebate": "not_applicable"})
    );
    assert_eq!(report["admission_decision"], "admit");
    assert_eq!(report["privacy_state"], "low");
    assert_eq!(report["instrument_count"], 1);
    let text = report.to_string();
    for id in [run_id, outcome_id, fixture.submission_id, fixture.trace_id] {
        assert!(
            !text.contains(&id.to_string()),
            "no raw id in the report: {text}"
        );
    }
    assert_eq!(report["run_id_hash"], id_hash(run_id));
    assert!(!text.contains(&fixture.secret_probe));

    let mut expects_reject = expected.clone();
    expects_reject.admission_decision = "reject".into();
    let report = fixture_report(&fixture, &expects_reject, &observed);
    assert_eq!(
        report["mismatches"],
        serde_json::json!(["admission_decision"])
    );

    // A withheld leg where the bundle's award is credited is a mismatch,
    // though the leg completed and the count is met (ruling T9-6).
    let mut withheld = observed.clone();
    withheld.status.as_mut().unwrap().instruments[0].internal_settlement_state = "withheld".into();
    let report = fixture_report(&fixture, &expected, &withheld);
    assert_eq!(report["settlement_state"], "complete");
    assert_eq!(report["instrument_count"], 1);
    assert_eq!(
        report["mismatches"],
        serde_json::json!(["instrument_states"])
    );

    let mut parked = observed.clone();
    parked.saw_quarantine = true;
    parked.state = "awaiting_review".into();
    parked.admission_reason = Some("privacy_review_required".into());
    parked.forensic.as_mut().unwrap().phases.truncate(1);
    parked.replay_same_run = false;
    let report = fixture_report(&fixture, &expected, &parked);
    assert_eq!(
        report["mismatches"],
        serde_json::json!([
            "run_not_terminal",
            "admission_decision",
            "outcome_count",
            "privacy_state",
            "scoring_state",
            "settlement_state",
            "replay_same_run",
        ])
    );
    assert_eq!(report["privacy_state"], "medium");

    let refused = FixtureObservation {
        request_content_hash: sha256_bytes(b"request"),
        state: "receipt_refused".into(),
        ..FixtureObservation::default()
    };
    let report = fixture_report(&fixture, &expected, &refused);
    assert_eq!(report["consent_state"], "refused");
    assert_eq!(report["run_id_hash"], serde_json::Value::Null);
    assert!(
        report["mismatches"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("consent_state"))
    );
}
