// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `pipeline.py run`, `package`, `keygen`, and the signing step of `qualify`
//! (P4-D3: no launcher binary, no cargo feature). Four ignored tests are the
//! implementation, and `pipeline.py` starts each one with `--ignored --exact`:
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
//! - `pipeline_check_attestations_write` signs each check result of a run
//!   (`<check_id>.attestation.json`) with a PKCS#8 Ed25519 key, then verifies
//!   every file it wrote (P5-D13). It needs no database.
//! - `pipeline_signing_key_write` generates that key and its trusted key
//!   (`pipeline.py keygen`).
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
    minimal_storage_rebate_package, post_submission_status, post_trace,
    qualification_candidate_package, runtime_backend, send_http, serve_pipeline_app,
    wait_for_pipeline_ready,
};
use trace_commons_gate_api::pipeline::{
    AtomicUnits, BundlePackage, InstrumentId, ReasonCode, ReviewRecommendation,
};
use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer, SettlementAdapter};
use trace_commons_protocol::trace_contribution::{
    RawTraceCaptureTurn, RawTraceContribution, TracePipelineStatusUpdate,
};
use trace_commons_server::versioned_pipeline::{PipelineCaps, PipelineServiceBuilder};
use trace_commons_server::versioned_pipeline_bundle::{
    PipelineBundleConfig, package_compatibility_config,
};
use trace_commons_server::versioned_pipeline_compat::{
    COMPATIBILITY_SCORE_IMPLEMENTATION, CompatibilityBundleConfig,
};
use trace_commons_server::versioned_pipeline_credit::{
    RecordingSettlementAdapter, SettlementAdapterRegistry,
};
use trace_commons_server::versioned_pipeline_harness::{
    HarnessAssembly, HarnessDependencies, assemble_harness_production,
    harness_production_components, production_package_pins,
};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_product::{
    PipelineForensicTrace, PipelineProductStore,
};
use trace_commons_server::versioned_pipeline_qualification::{
    BundlePackageTrustStore, CheckResultTrustStore, PipelineCheckAttestation, PipelineCheckEmitter,
    PipelineCheckResult, PipelineCheckStatus, QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS,
    SignedBundlePackage, TrustedBundleKey, evidence_hash, package_digests, sign_bundle_package,
    sign_check_result, trusted_key_for_pkcs8,
};

const CORPUS_SCHEMA: &str = "trace_commons.pipeline_corpus.v1";
const CORPUS_REPORT_SCHEMA: &str = "trace_commons.pipeline_corpus_report.v1";

/// The corpus check whose result names no package (P5-D15): it serves a
/// test bundle, not the qualification candidate.
const MINIMAL_CORPUS_CHECK_ID: &str = "pipeline_http_corpus_minimal";

/// The check ids `pipeline_corpus_run` may emit. `pipeline.py` chooses one:
/// `_<bundle>` for a built-in bundle, `_package` for a signed package, and
/// `_hf_local` for an HF pin (two partitions). `pipeline_http_corpus_package`
/// is not a required check (controller ruling PF-1).
const CORPUS_CHECK_IDS: [&str; 4] = [
    MINIMAL_CORPUS_CHECK_ID,
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

/// A production-mode report's blockers (spec B-D1): the scorer, embedder,
/// index and settlement adapter are the production assembly's, but the run
/// is still a harness run, with static bearer tokens, the allow-all test
/// authority and the pass-through privacy boundary, so it is never a
/// production report.
const PRODUCTION_HARNESS_BLOCKERS: [&str; 4] = [
    "local_test_only",
    "static_bearer_authentication",
    "harness_test_authority",
    "harness_pass_through_privacy",
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
/// The signing step of `pipeline.py qualify --signing-key`: the directory of
/// the run's check results, the staging directory the attestations are written
/// to, the ids of the checks the run accepted (comma separated), the PKCS#8
/// key and its id, and the maximum age the signer gives each result.
const CHECK_RESULT_DIR_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR";
const CHECK_ATTESTATION_DIR_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_ATTESTATION_DIR";
const CHECK_IDS_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_IDS";
const CHECK_SIGNING_KEY_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_PATH";
const CHECK_SIGNING_KEY_ID_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_SIGNING_KEY_ID";
const CHECK_MAX_AGE_SECONDS_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_MAX_AGE_SECONDS";
/// `pipeline.py keygen`: where to write the key and the trusted key, and the
/// key's id.
const KEYGEN_OUTPUT_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_OUTPUT";
const KEYGEN_KEY_ID_VAR: &str = "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_KEY_ID";
const KEYGEN_TRUSTED_KEY_OUTPUT_VAR: &str =
    "TRACE_COMMONS_PIPELINE_CHECK_KEYGEN_TRUSTED_KEY_OUTPUT";

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

/// The production package (spec B-D5): the scorer and embedder descriptors
/// `lookup` names, through the parsers the deployed binary uses, under
/// `main_gate`. Loads no model and calls no network
/// (`production_compatibility_package` reads descriptors only).
fn production_bundle_package(
    lookup: &dyn Fn(&str) -> Option<String>,
    main_gate: &trace_commons_server::versioned_pipeline_compat::MainGateConfig,
) -> anyhow::Result<BundlePackage> {
    let scorer = production_assembly::near_ai_scorer_descriptor_from_lookup(lookup)?;
    let embedder = production_assembly::fastembed_descriptor_from_lookup(lookup)?;
    trace_commons_server::versioned_pipeline_production::production_compatibility_package(
        &scorer, &embedder, main_gate,
    )
}

/// [`production_bundle_package`] from the process environment, with
/// `main`'s gate configuration parsed exactly as ingest parses it under a
/// pipeline runtime. `pipeline.py package --bundle production` passes the
/// env file's descriptor and gate variables, and nothing else.
fn production_bundle_package_from_env() -> anyhow::Result<BundlePackage> {
    let main_gate = pipeline_main_gate_config_from_env(
        true,
        parse_novelty_utility_credit_points_delta_from_env()?,
    )?;
    production_bundle_package(&|var: &str| std::env::var(var).ok(), &main_gate)
}

/// The built-in bundles, by name: PR 2's minimal package with the
/// `storage_rebate` award, and the qualification candidate, the
/// compatibility package over `local_reference()` with a 2_500_000
/// microcredit `NoveltyUtility` delta (`qualification_candidate_package`).
fn corpus_bundle_package(bundle: &str) -> anyhow::Result<BundlePackage> {
    match bundle {
        "minimal" => minimal_storage_rebate_package(
            &ReferencePerplexityScorer::new(),
            &ReferenceEmbedder::new(),
        ),
        "compatibility" => qualification_candidate_package(),
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
    dependencies: CorpusDependencies,
}

/// What the corpus service runs on: the reference assembly over an
/// isolated index, or the production assembly (spec B-D1) over the
/// dependencies the signed production package names.
enum CorpusDependencies {
    Reference(Arc<IsolatedPipelineIndex>),
    Production(HarnessDependencies),
}

impl CorpusDependencies {
    fn assembly(&self) -> HarnessAssembly {
        match self {
            Self::Reference(_) => HarnessAssembly::Reference,
            Self::Production(_) => HarnessAssembly::Production,
        }
    }
}

impl IngestPipelineRuntimeAssembler for CorpusAssembler {
    fn assemble(
        &self,
        context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        let index = match &self.dependencies {
            CorpusDependencies::Reference(index) => index.clone(),
            CorpusDependencies::Production(dependencies) => {
                return production_corpus_service(&self.package, dependencies.clone(), context);
            }
        };
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
            index.clone(),
            index,
            SettlementAdapterRegistry::new(adapters)?,
            caps,
        )
        .with_scorer(Arc::new(ReferencePerplexityScorer::new()))
        .with_embedder(Arc::new(ReferenceEmbedder::new()))
        .with_object_store_name(context.object_store_name)
        .with_novelty_utility_checks(context.novelty_utility_checks)
        .with_authority(allow_all_test_authority())
        .with_privacy(Arc::new(PassThroughPipelinePrivacyBoundary))
        .with_unqualified_routing(context.unqualified_routing_allowed)
        .build()?;
        Ok(Arc::new(service))
    }
}

/// The production corpus service: the production assembler, through the
/// ingest seam's context, over `dependencies` under `package`'s own pins,
/// with the harness's allow-all authority and pass-through privacy boundary
/// (the corpus tenants are the harness's, not the deployment's). Refused
/// unless it serves exactly `package`.
fn production_corpus_service(
    package: &BundlePackage,
    dependencies: HarnessDependencies,
    context: pipeline_runtime::IngestPipelineRuntimeContext,
) -> anyhow::Result<Arc<PipelineService>> {
    let pins = production_package_pins(package)?;
    let components = harness_production_components(
        &pins,
        dependencies,
        allow_all_test_authority(),
        Arc::new(PassThroughPipelinePrivacyBoundary),
    );
    Ok(Arc::new(assemble_harness_production(
        package,
        trace_commons_server::versioned_pipeline_production::ProductionPipelineInputs {
            backend: context.backend,
            artifact_store: context.artifact_store,
            object_store_name: context.object_store_name,
            lease_config: context.lease_config,
            novelty_utility_checks: context.novelty_utility_checks,
            unqualified_routing_allowed: context.unqualified_routing_allowed,
            main_gate: context.main_gate,
            components,
        },
    )?))
}

/// The service comes out of `assemble_ingest_pipeline_runtime`, the seam
/// ingest's real boot uses, exactly as the other HTTP tests' services do,
/// with the pipeline's credit issuer configured, which a runtime that
/// routes the compatibility bundle needs (PR 3: Zaki review 1, round 2,
/// finding 15); the minimal bundle ignores it. `main`'s gate configuration
/// is the one a compatibility package holds, which ingest requires of a
/// default package (PR 3, 9b54b647); a package of another family holds
/// none and is handed `TEST_MAIN_GATE`.
fn assemble_corpus_service(
    backend: Arc<PgBackend>,
    artifacts: Arc<LocalEncryptedTraceArtifactStore>,
    dependencies: CorpusDependencies,
    package: BundlePackage,
) -> Arc<PipelineService> {
    let main_gate = package_compatibility_config(&package)
        .map(|config| main_gate_of(&config))
        .unwrap_or(TEST_MAIN_GATE);
    let assembler = CorpusAssembler {
        package,
        dependencies,
    };
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
        true,
        None,
        TEST_NEAR_CONFIRMATION_INTERVAL,
        TEST_NEAR_PAYOUT_CONTROLS,
        &PipelineNoveltyUtilityChecks {
            issuer_principal_ref: Some(TEST_PIPELINE_CREDIT_ISSUER.to_string()),
            ..PipelineNoveltyUtilityChecks::default()
        },
        main_gate,
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
            .and_then(pipeline_block)
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
/// The expectations `item` misses, in a fixed order. In production mode
/// (spec section 2, O-B3) the outcome count, the scoring and settlement
/// states and the instrument legs are not compared: real NEAR AI scores
/// against the deployment's floors can legitimately move them, so they stay
/// in the report as evidence only.
fn fixture_mismatches(item: &serde_json::Value, assembly: HarnessAssembly) -> Vec<&'static str> {
    let compare_scoring = assembly == HarnessAssembly::Reference;
    let mut found = Vec::new();
    if !TERMINAL_STATES.contains(&item["state"].as_str().unwrap_or_default()) {
        found.push("run_not_terminal");
    }
    if item["admission_decision"] != item["expected_admission_decision"] {
        found.push("admission_decision");
    }
    if compare_scoring && item["phase_count"] != item["expected_outcome_count"] {
        found.push("outcome_count");
    }
    for (field, scoring) in [
        ("consent_state", false),
        ("privacy_state", false),
        ("scoring_state", true),
        ("settlement_state", true),
    ] {
        if (compare_scoring || !scoring)
            && item[field] != item[format!("expected_{field}").as_str()]
        {
            found.push(field);
        }
    }
    if compare_scoring && item["instrument_count"] != item["expected_instrument_count"] {
        found.push("instrument_count");
    }
    if compare_scoring && item["instrument_states"] != item["expected_instrument_states"] {
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
    assembly: HarnessAssembly,
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
    let mismatches = fixture_mismatches(&item, assembly);
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
    assembly: HarnessAssembly,
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
    // A reference report is exactly what it always was; a production one
    // says so, with its own blockers.
    if assembly == HarnessAssembly::Production {
        report["harness_assembly"] = serde_json::json!(assembly.label());
        report["safe_blockers"] = serde_json::json!(PRODUCTION_HARNESS_BLOCKERS);
    }
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

/// The two digests the corpus check's evidence carries beside its counts, for
/// the attestation to sign (P5-D14): `corpus_digest`, the digest of the
/// normalized direct corpus the run loaded (the digest of that one corpus
/// file's bytes, or, for the two partitions of an HF pin, the canonical hash
/// of the list of both digests in order); and `input_digest`, the canonical
/// hash of the list of every fixture's request-content hash, in the order the
/// run posted them. Both come from the report's own fields, so
/// `pipeline.py` recomputes them from the report and requires the evidence to
/// say the same (`corpus.evidence_digests`; each pins the same vectors).
fn corpus_evidence_digests(report: &serde_json::Value) -> Result<(String, String), String> {
    let invalid = || "corpus_report_digests_invalid".to_string();
    let sections = report["partitions"].as_array().ok_or_else(invalid)?;
    let mut corpus_digests = Vec::new();
    let mut requests = Vec::new();
    for section in sections {
        corpus_digests.push(section["corpus_digest"].as_str().ok_or_else(invalid)?);
        for fixture in section["fixtures"].as_array().ok_or_else(invalid)? {
            requests.push(
                fixture["request_content_hash"]
                    .as_str()
                    .ok_or_else(invalid)?,
            );
        }
    }
    if corpus_digests.is_empty() || requests.is_empty() {
        return Err(invalid());
    }
    let corpus_digest = match corpus_digests.as_slice() {
        [only] => (*only).to_string(),
        several => evidence_hash(&serde_json::json!(several))?,
    };
    Ok((corpus_digest, evidence_hash(&serde_json::json!(requests))?))
}

// ---------------------------------------------------------------------------
// The signing tools: `pipeline.py qualify --signing-key` and `keygen`.
// ---------------------------------------------------------------------------

/// The files of `dir` whose names end in `suffix`, in name order.
fn files_ending_in(dir: &Path, suffix: &str) -> Result<Vec<PathBuf>, String> {
    let mut found = std::fs::read_dir(dir)
        .map_err(|_| "check_result_dir_unreadable".to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(suffix))
        })
        .collect::<Vec<_>>();
    found.sort();
    Ok(found)
}

/// The corpus and input digests an evidence value names, for the attestation
/// to carry: both when it holds both keys, neither when it holds neither.
/// Holding exactly one is refused: the harness writes both or none, and one
/// alone would leave a digest that no attestation covers.
fn attested_digests(
    evidence: &serde_json::Value,
) -> Result<(Option<String>, Option<String>), String> {
    let digest = |key: &str| {
        evidence
            .get(key)
            .map(|value| {
                value
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| "check_evidence_digest_invalid".to_string())
            })
            .transpose()
    };
    match (digest("corpus_digest")?, digest("input_digest")?) {
        (None, None) => Ok((None, None)),
        (Some(corpus), Some(input)) => Ok((Some(corpus), Some(input))),
        _ => Err("check_evidence_digests_incomplete".to_string()),
    }
}

/// Signs the check results of `results_dir` with the PKCS#8 key `pkcs8` (key
/// id `key_id`) and writes `<check_id>.attestation.json` for each into
/// `attestation_dir` (a staging directory: the caller moves the files once
/// the run is done with them), and returns how many it wrote.
///
/// `accepted` is the set of check ids the run accepted
/// (`require_current_pass_results`). The results directory must hold a result
/// file for each of them and for no other id: an extra file is refused
/// (`check_result_unexpected`), as is a missing one (`check_result_missing`),
/// before anything is signed. Each result's evidence file must hash to the
/// result's `evidence_hash`. All results are signed before any file is
/// written, and each file is reserved with `create_new` (a repeat is refused,
/// as the emitter refuses one) and then written under a temporary name and
/// renamed. Last, every attestation file in `attestation_dir` is read back,
/// must equal what was signed, and must verify with a trust store built from
/// the same key; their count must equal the number of accepted checks. Every
/// refusal is a label: no path, key byte, or result value.
fn write_check_attestations(
    results_dir: &Path,
    attestation_dir: &Path,
    accepted: &BTreeSet<String>,
    pkcs8: &[u8],
    key_id: &str,
    maximum_age_seconds: u64,
) -> Result<usize, String> {
    let result_files = files_ending_in(results_dir, ".result.json")?;
    if result_files.is_empty() {
        return Err("check_attestation_no_results".to_string());
    }
    let mut results = Vec::new();
    for result_path in &result_files {
        let result: PipelineCheckResult = serde_json::from_slice(
            &std::fs::read(result_path).map_err(|_| "check_result_unreadable".to_string())?,
        )
        .map_err(|_| "check_result_invalid".to_string())?;
        result
            .validate()
            .map_err(|_| "check_result_invalid".to_string())?;
        if result_path.file_name().and_then(|name| name.to_str())
            != Some(format!("{}.result.json", result.check_id).as_str())
        {
            return Err("check_result_name_mismatch".to_string());
        }
        results.push(result);
    }
    let present = results
        .iter()
        .map(|result| result.check_id.clone())
        .collect::<BTreeSet<_>>();
    if !present.is_subset(accepted) {
        return Err("check_result_unexpected".to_string());
    }
    if !accepted.is_subset(&present) {
        return Err("check_result_missing".to_string());
    }
    let mut signed = Vec::new();
    for result in results {
        let check_id = result.check_id.clone();
        let evidence: serde_json::Value = serde_json::from_slice(
            &std::fs::read(results_dir.join(format!("{check_id}.evidence.json")))
                .map_err(|_| "check_evidence_unreadable".to_string())?,
        )
        .map_err(|_| "check_evidence_invalid".to_string())?;
        if evidence_hash(&evidence).map_err(|_| "check_evidence_invalid".to_string())?
            != result.evidence_hash
        {
            return Err("check_evidence_hash_mismatch".to_string());
        }
        let (corpus_digest, input_digest) = attested_digests(&evidence)?;
        let attestation = sign_check_result(
            result,
            maximum_age_seconds,
            corpus_digest,
            input_digest,
            key_id,
            pkcs8,
        )
        .map_err(|_| "check_attestation_signing_failed".to_string())?;
        signed.push((
            attestation_dir.join(format!("{check_id}.attestation.json")),
            attestation,
        ));
    }
    for (path, attestation) in &signed {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|_| "check_attestation_already_written".to_string())?;
        write_atomically(
            path,
            &serde_json::to_vec(attestation)
                .map_err(|_| "check_attestation_invalid".to_string())?,
        );
    }

    let store = CheckResultTrustStore::new([trusted_key_for_pkcs8(key_id, pkcs8)
        .map_err(|_| "check_attestation_signing_failed".to_string())?])?;
    let read_back = verified_attestations(attestation_dir, &store, accepted.len())?;
    for (_, attestation) in &signed {
        if !read_back.contains(attestation) {
            return Err("check_attestation_changed_on_disk".to_string());
        }
    }
    Ok(read_back.len())
}

/// The check ids of `list` (comma separated labels), each once.
fn parse_accepted_ids(list: &str) -> Result<BTreeSet<String>, String> {
    let mut ids = BTreeSet::new();
    for id in list.split(',') {
        let is_label = !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !is_label || !ids.insert(id.to_string()) {
            return Err("check_attestation_ids_invalid".to_string());
        }
    }
    Ok(ids)
}

/// Every `.attestation.json` file in `dir`, read back: there must be exactly
/// `expected` of them, each must parse, and all must verify with `store`.
fn verified_attestations(
    dir: &Path,
    store: &CheckResultTrustStore,
    expected: usize,
) -> Result<Vec<PipelineCheckAttestation>, String> {
    let written = files_ending_in(dir, ".attestation.json")?;
    if written.len() != expected {
        return Err("check_attestation_count_mismatch".to_string());
    }
    let mut read_back = Vec::new();
    for path in &written {
        read_back.push(
            serde_json::from_slice::<PipelineCheckAttestation>(
                &std::fs::read(path).map_err(|_| "check_attestation_unreadable".to_string())?,
            )
            .map_err(|_| "check_attestation_invalid".to_string())?,
        );
    }
    if store.verify_all(&read_back)?.evidence.len() != expected {
        return Err("check_attestation_count_mismatch".to_string());
    }
    Ok(read_back)
}

/// Creates `path` and writes `bytes`, refusing an existing file (a dangling
/// link included). A private file is created with mode 0600 and no wider.
fn create_new_output(path: &Path, bytes: &[u8], private: bool) -> Result<(), &'static str> {
    use std::io::Write as _;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    if private {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        #[cfg(not(unix))]
        return Err("signing_key_mode_unsupported");
    }
    let mut file = options
        .open(path)
        .map_err(|_| "signing_key_output_unwritable")?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "signing_key_output_unwritable")
}

/// `pipeline.py keygen`: generates an Ed25519 PKCS#8 key with `ring`, as
/// `pipeline_package_write` generates one, writes it to `key_output` (mode
/// 0600) and the [`TrustedBundleKey`] to `trusted_output`, and refuses either
/// path if it already exists. Its assertion: a result signed with the written
/// key verifies with the written trusted key.
fn write_signing_key_pair(
    key_output: &Path,
    key_id: &str,
    trusted_output: &Path,
) -> Result<(), String> {
    use ring::signature::Ed25519KeyPair;

    if key_output == trusted_output {
        return Err("keygen_outputs_must_differ".to_string());
    }
    // `symlink_metadata`: a dangling link is something to refuse as well.
    if std::fs::symlink_metadata(key_output).is_ok()
        || std::fs::symlink_metadata(trusted_output).is_ok()
    {
        return Err("signing_key_output_exists".to_string());
    }
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .map_err(|_| "signing_key_generation_failed".to_string())?;
    let trusted = trusted_key_for_pkcs8(key_id, pkcs8.as_ref())
        .map_err(|_| "signing_key_generation_failed".to_string())?;
    // The key id is checked here, before a file exists: the trust store
    // refuses an id that is not an identifier, and a refused id leaves
    // neither the key nor the trusted key behind.
    CheckResultTrustStore::new([trusted.clone()])
        .map_err(|_| "signing_key_id_invalid".to_string())?;
    create_new_output(key_output, pkcs8.as_ref(), true)?;
    let trusted_bytes = serde_json::to_vec_pretty(&trusted)
        .map_err(|_| "signing_key_generation_failed".to_string())?;
    if let Err(label) = create_new_output(trusted_output, &trusted_bytes, false) {
        // Leave no private key without its trusted key behind.
        let _ = std::fs::remove_file(key_output);
        return Err(label.to_string());
    }

    let written = zeroize::Zeroizing::new(
        std::fs::read(key_output).map_err(|_| "signing_key_unreadable".to_string())?,
    );
    let written_key: TrustedBundleKey = serde_json::from_slice(
        &std::fs::read(trusted_output).map_err(|_| "trusted_key_unreadable".to_string())?,
    )
    .map_err(|_| "trusted_key_invalid".to_string())?;
    let probe = PipelineCheckResult {
        schema: "trace_commons.pipeline_check_result.v1".to_string(),
        run_id: "qkeygen".to_string(),
        check_id: "pipeline_signing_key_probe".to_string(),
        status: PipelineCheckStatus::Pass,
        code_revision_hash: sha256_bytes(b"keygen"),
        package_hash: None,
        configuration_digest: None,
        dependency_digest: None,
        observed_at: Utc::now(),
        evidence_hash: sha256_bytes(b"keygen"),
        safe_blockers: Vec::new(),
    };
    let attestation = sign_check_result(probe, 60, None, None, key_id, &written)
        .map_err(|_| "signing_key_invalid".to_string())?;
    CheckResultTrustStore::new([written_key])
        .and_then(|store| store.verify_all(&[attestation]).map(|_| ()))
        .map_err(|_| "signing_key_pair_mismatch".to_string())
}

// ---------------------------------------------------------------------------
// The ignored tools.
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
    let assembly = HarnessAssembly::from_env().unwrap_or_else(|error| panic!("{error}"));
    let package = config.package().unwrap_or_else(|label| panic!("{label}"));
    let dependencies = match assembly {
        HarnessAssembly::Reference => CorpusDependencies::Reference(IsolatedPipelineIndex::new()),
        HarnessAssembly::Production => CorpusDependencies::Production(
            production_corpus_dependencies_from_env(&config, &package).await,
        ),
    };
    run_corpus(&config, package, dependencies).await;
}

/// Spec B-D1: production mode serves a signed production package, never a
/// built-in bundle, and never as the minimal corpus check (which names no
/// package). Refused before any database or model is touched.
fn require_production_corpus_config(
    config: &CorpusRunConfig,
    package: &BundlePackage,
) -> Result<(), String> {
    if !matches!(config.source, CorpusPackageSource::Signed { .. }) {
        return Err("corpus_production_requires_signed_package".to_string());
    }
    if config.check_id == MINIMAL_CORPUS_CHECK_ID {
        return Err("corpus_production_check_id_invalid".to_string());
    }
    production_package_pins(package)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The real production dependencies for a corpus run on the operator host:
/// NEAR AI, fastembed, and a usearch index at
/// `TRACE_COMMONS_PIPELINE_HARNESS_INDEX_ROOT`.
#[cfg(feature = "near-ai-scorer")]
async fn production_corpus_dependencies_from_env(
    config: &CorpusRunConfig,
    package: &BundlePackage,
) -> HarnessDependencies {
    use trace_commons_server::versioned_pipeline_harness::{
        HARNESS_PRODUCTION_INDEX_ROOT_MISSING_LABEL, TRACE_COMMONS_PIPELINE_HARNESS_INDEX_ROOT,
        harness_dependencies_from_env,
    };
    require_production_corpus_config(config, package).unwrap_or_else(|label| panic!("{label}"));
    let lookup = |var: &str| std::env::var(var).ok();
    let pins = production_package_pins(package).unwrap_or_else(|error| panic!("{error}"));
    let index_root = lookup(TRACE_COMMONS_PIPELINE_HARNESS_INDEX_ROOT)
        .unwrap_or_else(|| panic!("{HARNESS_PRODUCTION_INDEX_ROOT_MISSING_LABEL}"));
    harness_dependencies_from_env(&pins, Path::new(&index_root), &lookup)
        .await
        .unwrap_or_else(|error| panic!("{error}"))
}

/// Never reached: `HarnessAssembly::from_env` refuses production mode
/// without `near-ai-scorer`.
#[cfg(not(feature = "near-ai-scorer"))]
async fn production_corpus_dependencies_from_env(
    _config: &CorpusRunConfig,
    _package: &BundlePackage,
) -> HarnessDependencies {
    panic!(
        "{}",
        trace_commons_server::versioned_pipeline_harness::HARNESS_PRODUCTION_ASSEMBLY_UNAVAILABLE_LABEL
    )
}

/// One corpus run over `package` on `dependencies`: the body of
/// `pipeline_corpus_run`, which a production-mode test also drives with
/// qualified doubles.
async fn run_corpus(
    config: &CorpusRunConfig,
    package: BundlePackage,
    dependencies: CorpusDependencies,
) {
    let assembly = dependencies.assembly();
    if assembly == HarnessAssembly::Production {
        require_production_corpus_config(config, &package)
            .unwrap_or_else(|label| panic!("{label}"));
    }
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
        dependencies,
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
    state_mut.pipeline_activation = routing_store(&runtime);
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
            fixtures.push(fixture_report(fixture, &expected, &observed, assembly));
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

    let report = corpus_report(&config.check_id, &package, sections, assembly)
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

    // The corpus loaded and the requests posted: what an attestation signs
    // for this check beside its result (P5-D14).
    let (corpus_digest, input_digest) =
        corpus_evidence_digests(&report).unwrap_or_else(|label| panic!("{label}"));
    let mut evidence = serde_json::json!({
        "fixtures": report["fixture_count"],
        "completed": report["completed_fixture_count"],
        "replay_same_run": report["replay_same_run_count"],
        "changed_content_refused": report["changed_content_refused_count"],
        "tenant_isolation": report["tenant_isolation"],
        "report_hash": sha256_bytes(&report_bytes),
        "corpus_digest": corpus_digest,
        "input_digest": input_digest,
    });
    if assembly == HarnessAssembly::Production {
        evidence["harness_assembly"] = serde_json::json!(assembly.label());
    }
    assert!(
        !contains_probe(
            &trace_commons_protocol::canonical_json::to_canonical_vec(&evidence)
                .expect("the evidence serialises"),
            &probes
        ),
        "corpus_probe_in_evidence"
    );
    // The minimal corpus check serves a test bundle, so its result names no
    // package (P5-D15); the compatibility, HF-local, and signed-package
    // checks name the package they served. The report keeps its own
    // digests either way.
    let named = (config.check_id != MINIMAL_CORPUS_CHECK_ID).then_some(&package);
    PipelineCheckEmitter::emit_pass_from_env(&config.check_id, named, evidence);
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
    let package = if bundle == "production" {
        let package =
            production_bundle_package_from_env().unwrap_or_else(|error| panic!("{error}"));
        trace_commons_server::versioned_pipeline_qualification::validate_production_package(
            &package,
        )
        .unwrap_or_else(|_| panic!("package_production_invalid"));
        production_package_pins(&package).unwrap_or_else(|error| panic!("{error}"));
        package
    } else {
        corpus_bundle_package(&bundle).unwrap_or_else(|_| panic!("package_bundle_invalid"))
    };
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

/// The signing step of `pipeline.py qualify --signing-key`: signs the check
/// results the run accepted into a staging directory and verifies what it
/// wrote (see [`write_check_attestations`]). Needs no database. The key path
/// is read here and never printed: every failure is a label.
#[test]
#[ignore = "the implementation of the signing step of `pipeline.py qualify`: run it through that command"]
fn pipeline_check_attestations_write() {
    let var = |name: &str| std::env::var(name).ok();
    let (results_dir, attestation_dir, accepted, key_path, key_id, maximum_age) = match (
        var(CHECK_RESULT_DIR_VAR),
        var(CHECK_ATTESTATION_DIR_VAR),
        var(CHECK_IDS_VAR),
        var(CHECK_SIGNING_KEY_PATH_VAR),
        var(CHECK_SIGNING_KEY_ID_VAR),
        var(CHECK_MAX_AGE_SECONDS_VAR),
    ) {
        (None, None, None, None, None, None) => return,
        (
            Some(results_dir),
            Some(attestation_dir),
            Some(accepted),
            Some(key_path),
            Some(key_id),
            Some(maximum_age),
        ) => (
            PathBuf::from(results_dir),
            PathBuf::from(attestation_dir),
            accepted,
            key_path,
            key_id,
            maximum_age,
        ),
        _ => panic!("check_attestation_environment_incomplete"),
    };
    let accepted = parse_accepted_ids(&accepted).unwrap_or_else(|label| panic!("{label}"));
    let maximum_age: u64 = maximum_age
        .parse()
        .unwrap_or_else(|_| panic!("check_attestation_max_age_invalid"));
    assert!(
        (1..=QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS).contains(&maximum_age),
        "check_attestation_max_age_invalid"
    );
    assert!(attestation_dir.is_dir(), "check_attestation_dir_missing");
    let pkcs8 = zeroize::Zeroizing::new(
        std::fs::read(key_path).unwrap_or_else(|_| panic!("check_signing_key_unreadable")),
    );
    let written = write_check_attestations(
        &results_dir,
        &attestation_dir,
        &accepted,
        &pkcs8,
        &key_id,
        maximum_age,
    )
    .unwrap_or_else(|label| panic!("{label}"));
    assert_eq!(written, accepted.len(), "check_attestation_count_mismatch");
}

/// `pipeline.py keygen`: writes a new Ed25519 key (mode 0600) and its trusted
/// key (see [`write_signing_key_pair`]).
#[test]
#[ignore = "the implementation of `pipeline.py keygen`: run it through that command"]
fn pipeline_signing_key_write() {
    let var = |name: &str| std::env::var(name).ok();
    let (output, key_id, trusted_output) = match (
        var(KEYGEN_OUTPUT_VAR),
        var(KEYGEN_KEY_ID_VAR),
        var(KEYGEN_TRUSTED_KEY_OUTPUT_VAR),
    ) {
        (None, None, None) => return,
        (Some(output), Some(key_id), Some(trusted_output)) => {
            (PathBuf::from(output), key_id, PathBuf::from(trusted_output))
        }
        _ => panic!("keygen_environment_incomplete"),
    };
    write_signing_key_pair(&output, &key_id, &trusted_output)
        .unwrap_or_else(|label| panic!("{label}"));
}

// ---------------------------------------------------------------------------
// Unit tests (no database).
// ---------------------------------------------------------------------------

/// A directory of check results as a run leaves them: one passing result and
/// its evidence for each of `evidence`'s `(check_id, evidence)` pairs.
fn emitted_results(evidence: &[(&str, serde_json::Value)]) -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let emitter = PipelineCheckEmitter::new(
        dir.path().to_path_buf(),
        "qattest01",
        &sha256_bytes(b"code-revision"),
    )
    .expect("emitter builds");
    for (check_id, observed) in evidence {
        emitter
            .emit(
                check_id,
                PipelineCheckStatus::Pass,
                None,
                &[],
                observed.clone(),
            )
            .expect("the check result is written");
    }
    dir
}

fn generated_signing_key() -> zeroize::Zeroizing<Vec<u8>> {
    zeroize::Zeroizing::new(
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .expect("a key generates")
            .as_ref()
            .to_vec(),
    )
}

/// A run as the signing step sees it: the check results in one directory, an
/// empty staging directory for the attestations, and the ids the run accepted
/// (here, every result emitted).
struct StagedRun {
    results: tempfile::TempDir,
    staging: tempfile::TempDir,
    accepted: BTreeSet<String>,
}

impl StagedRun {
    fn new(evidence: &[(&str, serde_json::Value)]) -> Self {
        Self {
            results: emitted_results(evidence),
            staging: tempfile::TempDir::new().expect("temp dir"),
            accepted: evidence.iter().map(|(id, _)| (*id).to_string()).collect(),
        }
    }

    fn sign(&self, pkcs8: &[u8], key_id: &str) -> Result<usize, String> {
        write_check_attestations(
            self.results.path(),
            self.staging.path(),
            &self.accepted,
            pkcs8,
            key_id,
            3_600,
        )
    }

    fn staged(&self) -> Vec<PathBuf> {
        files_ending_in(self.staging.path(), ".attestation.json").unwrap()
    }
}

#[test]
fn check_attestations_are_written_once_and_verify_with_the_signing_key() {
    let corpus = sha256_bytes(b"corpus");
    let input = sha256_bytes(b"input");
    let run = StagedRun::new(&[
        (
            "pipeline_http_corpus_minimal",
            serde_json::json!({"fixtures": 5, "corpus_digest": corpus, "input_digest": input}),
        ),
        ("pipeline_crash_matrix", serde_json::json!({"runs": 3})),
        ("pipeline_lease_renewal", serde_json::json!({"renewals": 2})),
    ]);
    let pkcs8 = generated_signing_key();
    assert_eq!(run.sign(&pkcs8, "unit_check_key"), Ok(3));
    assert_eq!(
        files_ending_in(run.results.path(), ".attestation.json").unwrap(),
        Vec::<PathBuf>::new(),
        "the attestations go to the staging directory, not beside the results"
    );

    let read = |check_id: &str| -> PipelineCheckAttestation {
        serde_json::from_slice(
            &std::fs::read(
                run.staging
                    .path()
                    .join(format!("{check_id}.attestation.json")),
            )
            .expect("the attestation was written"),
        )
        .expect("the attestation parses")
    };
    let corpus_check = read("pipeline_http_corpus_minimal");
    assert_eq!(corpus_check.corpus_digest.as_deref(), Some(corpus.as_str()));
    assert_eq!(corpus_check.input_digest.as_deref(), Some(input.as_str()));
    assert_eq!(corpus_check.maximum_age_seconds, 3_600);
    assert_eq!(corpus_check.signature.key_id, "unit_check_key");
    let mechanics = read("pipeline_crash_matrix");
    assert_eq!(
        (mechanics.corpus_digest, mechanics.input_digest),
        (None, None)
    );
    let result: PipelineCheckResult = serde_json::from_slice(
        &std::fs::read(run.results.path().join("pipeline_crash_matrix.result.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        mechanics.result, result,
        "the result inside is the result file"
    );

    // Another key does not verify what was written.
    let other = generated_signing_key();
    let other_store =
        CheckResultTrustStore::new([trusted_key_for_pkcs8("unit_check_key", &other).unwrap()])
            .unwrap();
    assert!(other_store.verify_all(&[corpus_check]).is_err());

    // A second run into the same staging directory refuses and writes
    // nothing new.
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_attestation_already_written".to_string())
    );

    // What is read back is checked against the key the files were signed
    // with: the right store verifies all three; a store of another key, or of
    // none, does not, and the number of files must be the one expected.
    let own_store =
        CheckResultTrustStore::new([trusted_key_for_pkcs8("unit_check_key", &pkcs8).unwrap()])
            .unwrap();
    let staging = run.staging.path();
    assert_eq!(
        verified_attestations(staging, &own_store, 3).map(|all| all.len()),
        Ok(3)
    );
    assert_eq!(
        verified_attestations(staging, &own_store, 2).map(|all| all.len()),
        Err("check_attestation_count_mismatch".to_string())
    );
    assert_eq!(
        verified_attestations(staging, &other_store, 3).map(|all| all.len()),
        Err("check_attestation_signature_invalid".to_string())
    );
    let empty_store = CheckResultTrustStore::new(std::iter::empty()).unwrap();
    assert_eq!(
        verified_attestations(staging, &empty_store, 3).map(|all| all.len()),
        Err("check_attestation_signer_untrusted".to_string())
    );
    std::fs::write(
        staging.join("pipeline_crash_matrix.attestation.json"),
        b"not json",
    )
    .unwrap();
    assert_eq!(
        verified_attestations(staging, &own_store, 3).map(|all| all.len()),
        Err("check_attestation_invalid".to_string())
    );
}

/// The signer signs the checks the run accepted, no other: a result file of
/// another id, and an accepted check without a result file, are refused
/// before anything is signed. The attestation directory may also be the
/// results directory (the signer does not mind).
#[test]
fn check_attestations_cover_exactly_the_accepted_checks() {
    let pkcs8 = generated_signing_key();
    let crash = ("pipeline_crash_matrix", serde_json::json!({"runs": 3}));
    let lease = ("pipeline_lease_renewal", serde_json::json!({"renewals": 2}));

    // A result file the run did not accept (a stray one in the directory).
    let mut run = StagedRun::new(&[crash.clone(), lease.clone()]);
    run.accepted.remove("pipeline_lease_renewal");
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_result_unexpected".to_string())
    );
    assert!(run.staged().is_empty(), "nothing was signed");

    // An accepted check with no result file.
    let mut run = StagedRun::new(std::slice::from_ref(&crash));
    run.accepted.insert("pipeline_lease_renewal".to_string());
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_result_missing".to_string())
    );
    assert!(run.staged().is_empty(), "nothing was signed");

    // Both at once: the extra file is named first.
    let mut run = StagedRun::new(std::slice::from_ref(&crash));
    run.accepted = ["pipeline_lease_renewal".to_string()].into();
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_result_unexpected".to_string())
    );

    // Exactly the accepted checks: signed, in the results directory too.
    let run = StagedRun::new(&[crash, lease]);
    assert_eq!(
        write_check_attestations(
            run.results.path(),
            run.results.path(),
            &run.accepted,
            &pkcs8,
            "unit_check_key",
            3_600
        ),
        Ok(2)
    );

    // The list of ids the tool passes.
    assert_eq!(
        parse_accepted_ids("pipeline_crash_matrix,pipeline_lease_renewal"),
        Ok([
            "pipeline_crash_matrix".to_string(),
            "pipeline_lease_renewal".to_string()
        ]
        .into())
    );
    for broken in [
        "",
        ",",
        "a,,b",
        "a,a",
        "Not_A_Label",
        "a b",
        "pipeline_crash_matrix,",
        &"x".repeat(65),
    ] {
        assert_eq!(
            parse_accepted_ids(broken),
            Err("check_attestation_ids_invalid".to_string()),
            "{broken}"
        );
    }
}

#[test]
fn check_attestations_refuse_evidence_that_is_not_the_results() {
    let pkcs8 = generated_signing_key();
    let crash = || ("pipeline_crash_matrix", serde_json::json!({"runs": 3}));

    // Evidence changed after the result was written.
    let run = StagedRun::new(&[crash()]);
    std::fs::write(
        run.results
            .path()
            .join("pipeline_crash_matrix.evidence.json"),
        br#"{"runs": 4}"#,
    )
    .unwrap();
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_evidence_hash_mismatch".to_string())
    );
    assert!(run.staged().is_empty(), "nothing was signed");

    // An attestation file that this run did not write (another run's, left
    // in the staging directory) makes the count wrong: the step refuses.
    let run = StagedRun::new(&[crash()]);
    std::fs::write(
        run.staging
            .path()
            .join("pipeline_old_check.attestation.json"),
        b"{}",
    )
    .unwrap();
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_attestation_count_mismatch".to_string())
    );

    // Missing evidence, an unreadable result, and an empty directory.
    let run = StagedRun::new(&[crash()]);
    std::fs::remove_file(
        run.results
            .path()
            .join("pipeline_crash_matrix.evidence.json"),
    )
    .unwrap();
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_evidence_unreadable".to_string())
    );
    let run = StagedRun::new(&[crash()]);
    std::fs::write(
        run.results.path().join("pipeline_crash_matrix.result.json"),
        b"",
    )
    .unwrap();
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_result_invalid".to_string())
    );
    let empty = StagedRun::new(&[]);
    assert_eq!(
        empty.sign(&pkcs8, "unit_check_key"),
        Err("check_attestation_no_results".to_string())
    );

    // A result file named for another check.
    let run = StagedRun::new(&[crash()]);
    std::fs::rename(
        run.results.path().join("pipeline_crash_matrix.result.json"),
        run.results
            .path()
            .join("pipeline_lease_renewal.result.json"),
    )
    .unwrap();
    assert_eq!(
        run.sign(&pkcs8, "unit_check_key"),
        Err("check_result_name_mismatch".to_string())
    );

    // Exactly one of the two digests, or one that is not a string or not a
    // digest: the attestation would carry a digest it does not cover.
    for (what, evidence, label) in [
        (
            "only the corpus digest",
            serde_json::json!({"corpus_digest": sha256_bytes(b"c")}),
            "check_evidence_digests_incomplete",
        ),
        (
            "only the input digest",
            serde_json::json!({"input_digest": sha256_bytes(b"i")}),
            "check_evidence_digests_incomplete",
        ),
        (
            "a digest that is a number",
            serde_json::json!({"corpus_digest": 7, "input_digest": sha256_bytes(b"i")}),
            "check_evidence_digest_invalid",
        ),
        (
            "a digest that is not a digest",
            serde_json::json!({"corpus_digest": "not-a-digest", "input_digest": sha256_bytes(b"i")}),
            "check_attestation_signing_failed",
        ),
    ] {
        let run = StagedRun::new(&[("pipeline_http_corpus_minimal", evidence)]);
        assert_eq!(
            run.sign(&pkcs8, "unit_check_key"),
            Err(label.to_string()),
            "{what}"
        );
        assert!(run.staged().is_empty(), "{what}");
    }

    // A key that is not a key, and a key id that is not an identifier.
    let run = StagedRun::new(&[crash()]);
    assert_eq!(
        run.sign(b"not a key", "unit_check_key"),
        Err("check_attestation_signing_failed".to_string())
    );
    assert_eq!(
        run.sign(&pkcs8, "key id with spaces"),
        Err("check_attestation_signing_failed".to_string())
    );
    assert!(run.staged().is_empty());
}

#[test]
fn a_signing_key_pair_is_written_private_and_never_over_an_existing_file() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let key = dir.path().join("check-key.pk8");
    let trusted = dir.path().join("check-key.json");
    write_signing_key_pair(&key, "unit_check_key", &trusted).expect("a key pair is written");

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "the key is private to its owner");
    }
    let trusted_key: TrustedBundleKey =
        serde_json::from_slice(&std::fs::read(&trusted).unwrap()).unwrap();
    assert_eq!(trusted_key.key_id, "unit_check_key");
    assert!(
        !std::fs::read_to_string(&trusted)
            .unwrap()
            .contains("PRIVATE"),
        "the trusted key file holds the public key only"
    );
    let pkcs8 = std::fs::read(&key).unwrap();
    assert_eq!(
        trusted_key,
        trusted_key_for_pkcs8("unit_check_key", &pkcs8).unwrap(),
        "the trusted key is the written key's public half"
    );

    // Nothing is overwritten: not the key, not the trusted key, not a link.
    let before = (
        std::fs::read(&key).unwrap(),
        std::fs::read(&trusted).unwrap(),
    );
    let other = dir.path().join("other.json");
    let other_key = dir.path().join("other.pk8");
    for (key_path, trusted_path) in [(&key, &other), (&other_key, &trusted), (&key, &trusted)] {
        assert_eq!(
            write_signing_key_pair(key_path, "unit_check_key", trusted_path),
            Err("signing_key_output_exists".to_string())
        );
    }
    assert!(!other.exists() && !other_key.exists());
    assert_eq!(
        before,
        (
            std::fs::read(&key).unwrap(),
            std::fs::read(&trusted).unwrap()
        )
    );
    #[cfg(unix)]
    {
        let link = dir.path().join("dangling.pk8");
        std::os::unix::fs::symlink(dir.path().join("nowhere"), &link).unwrap();
        assert_eq!(
            write_signing_key_pair(&link, "unit_check_key", &other),
            Err("signing_key_output_exists".to_string())
        );
        assert!(!other.exists());
    }
    assert_eq!(
        write_signing_key_pair(&other_key, "unit_check_key", &other_key),
        Err("keygen_outputs_must_differ".to_string())
    );
    // A key id that is not an identifier leaves no key behind: neither file
    // exists afterwards, and the label names the id.
    assert_eq!(
        write_signing_key_pair(&other_key, "bad key id", &other),
        Err("signing_key_id_invalid".to_string())
    );
    assert!(
        !other_key.exists() && !other.exists(),
        "a refused key id leaves neither the key nor the trusted key"
    );
}

/// `corpus.evidence_digests` in `scripts/operator/pipeline_tooling/corpus.py`
/// pins the same three literals.
#[test]
fn corpus_evidence_digests_name_the_corpus_and_the_requests() {
    let digest = |digit: char| format!("sha256:{}", digit.to_string().repeat(64));
    let report = |sections: &[(char, &[char])]| {
        serde_json::json!({
            "partitions": sections
                .iter()
                .map(|(corpus, requests)| serde_json::json!({
                    "corpus_digest": digest(*corpus),
                    "fixtures": requests
                        .iter()
                        .map(|request| serde_json::json!({"request_content_hash": digest(*request)}))
                        .collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        })
    };
    assert_eq!(
        corpus_evidence_digests(&report(&[('1', &['a', 'b'])])),
        Ok((
            digest('1'),
            "sha256:b7cc444b1f09d1e380d2a82f6ae10447932ce3b91f31b0ab2e659f7a7fdea15f".to_string()
        )),
        "one partition: its own corpus digest"
    );
    assert_eq!(
        corpus_evidence_digests(&report(&[('1', &['a', 'b']), ('2', &['c'])])),
        Ok((
            "sha256:636d591478be5cb76f2025a46b17a229fa29ef91724891793005baa9eef2100e".to_string(),
            "sha256:208dcba10edb6eea7324c3be4765e976cb578c1d9715d91e60d4042df6650461".to_string()
        )),
        "two partitions: the hash of both digests, and every request in order"
    );
    // A report that is not a report names no digests.
    for broken in [
        serde_json::json!({}),
        serde_json::json!({"partitions": []}),
        serde_json::json!({"partitions": [{"corpus_digest": digest('1'), "fixtures": []}]}),
        serde_json::json!({"partitions": [{"corpus_digest": 5, "fixtures": []}]}),
    ] {
        assert!(corpus_evidence_digests(&broken).is_err(), "{broken}");
    }
}

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

/// The pipeline block of a status `document`, as the harness reads it.
/// `None` for a document with no block, a block that does not read, and a
/// block with an amount the client cannot read: an unreadable amount is not
/// a status the harness accepts.
fn pipeline_block(document: &serde_json::Value) -> Option<TracePipelineStatusUpdate> {
    document
        .get("pipeline")
        .and_then(|pipeline| serde_json::from_value(pipeline.clone()).ok())
        .filter(|status: &TracePipelineStatusUpdate| {
            status
                .instruments
                .iter()
                .all(|instrument| instrument.atomic_units.readable().is_some())
        })
}

/// Multi-lens review C27: an amount the client cannot read is not a status
/// the harness accepts. Its block reads as absent, so the run never reads
/// as terminal and the fixture fails as `run_not_terminal`, as it did when a
/// malformed amount failed the whole block.
#[test]
fn the_corpus_harness_does_not_accept_a_pipeline_block_with_an_unreadable_amount() {
    let document = |atomic_units: serde_json::Value| {
        serde_json::json!({
            "submission_id": Uuid::nil(),
            "pipeline": {
                "run_id": Uuid::nil(),
                "bundle_id": "sha256:bundle",
                "processing_state": "complete",
                "current_phase": null,
                "responsible_phase": null,
                "reason_label": null,
                "instruments": [{
                    "instrument_id": "storage_rebate",
                    "atomic_units": atomic_units,
                    "operation_state": "complete",
                    "internal_settlement_state": "not_applicable",
                    "payout_rail": "none",
                    "payout_state": "disabled",
                    "reason_label": null
                }]
            }
        })
    };
    let readable = pipeline_block(&document(serde_json::json!("5"))).expect("a readable block");
    assert_eq!(readable.instruments.len(), 1);
    assert_eq!(pipeline_block(&document(serde_json::json!(5))), None);
    assert_eq!(pipeline_block(&document(serde_json::Value::Null)), None);
    assert_eq!(
        pipeline_block(&serde_json::json!({"submission_id": Uuid::nil()})),
        None
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
        atomic_units: trace_commons_protocol::trace_contribution::InstrumentAmount::Readable(
            trace_commons_protocol::trace_contribution::DecimalAtomicUnits::from(5),
        ),
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

    let report = fixture_report(&fixture, &expected, &observed, HarnessAssembly::Reference);
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
    let report = fixture_report(
        &fixture,
        &expects_reject,
        &observed,
        HarnessAssembly::Reference,
    );
    assert_eq!(
        report["mismatches"],
        serde_json::json!(["admission_decision"])
    );

    // A withheld leg where the bundle's award is credited is a mismatch,
    // though the leg completed and the count is met (ruling T9-6).
    let mut withheld = observed.clone();
    withheld.status.as_mut().unwrap().instruments[0].internal_settlement_state = "withheld".into();
    let report = fixture_report(&fixture, &expected, &withheld, HarnessAssembly::Reference);
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
    let report = fixture_report(&fixture, &expected, &parked, HarnessAssembly::Reference);
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
    let report = fixture_report(&fixture, &expected, &refused, HarnessAssembly::Reference);
    assert_eq!(report["consent_state"], "refused");
    assert_eq!(report["run_id_hash"], serde_json::Value::Null);
    assert!(
        report["mismatches"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("consent_state"))
    );
}

/// Spec section 2, O-B3 (plan B3): in production mode the corpus check
/// compares only what the scorer and the deployment's floors cannot move --
/// whether the run finished, the admission decision, consent, privacy,
/// replay, changed-content refusal and tenant isolation. The outcome count,
/// scoring and settlement states and the instrument legs stay in the report
/// as evidence and are not compared. Reference mode compares every field.
#[test]
fn production_corpus_mode_compares_only_deterministic_fields() {
    use trace_commons_server::versioned_pipeline_harness::HarnessAssembly;

    let item = serde_json::json!({
        "state": "complete",
        "admission_decision": "admit",
        "expected_admission_decision": "admit",
        "phase_count": 3,
        "expected_outcome_count": 4,
        "consent_state": "allowed",
        "expected_consent_state": "allowed",
        "privacy_state": "low",
        "expected_privacy_state": "low",
        "scoring_state": "complete",
        "expected_scoring_state": "complete",
        "settlement_state": "incomplete",
        "expected_settlement_state": "complete",
        "instrument_count": 0,
        "expected_instrument_count": 1,
        "instrument_states": {},
        "expected_instrument_states": {"trace_credit": "not_settlement_eligible"},
        "replay_same_run": true,
        "changed_content_refused": true,
        "tenant_isolation": true,
    });
    assert_eq!(
        fixture_mismatches(&item, HarnessAssembly::Reference),
        [
            "outcome_count",
            "settlement_state",
            "instrument_count",
            "instrument_states"
        ]
    );
    assert!(fixture_mismatches(&item, HarnessAssembly::Production).is_empty());
    let mut unscored = item.clone();
    unscored["scoring_state"] = serde_json::json!("missing");
    assert!(fixture_mismatches(&unscored, HarnessAssembly::Production).is_empty());

    for (field, value, label) in [
        (
            "state",
            serde_json::json!("awaiting_review"),
            "run_not_terminal",
        ),
        (
            "admission_decision",
            serde_json::json!("quarantine"),
            "admission_decision",
        ),
        (
            "consent_state",
            serde_json::json!("refused"),
            "consent_state",
        ),
        (
            "privacy_state",
            serde_json::json!("medium"),
            "privacy_state",
        ),
        (
            "replay_same_run",
            serde_json::json!(false),
            "replay_same_run",
        ),
        (
            "changed_content_refused",
            serde_json::json!(false),
            "changed_content_refused",
        ),
        (
            "tenant_isolation",
            serde_json::json!(false),
            "tenant_isolation",
        ),
    ] {
        let mut changed = item.clone();
        changed[field] = value;
        assert_eq!(
            fixture_mismatches(&changed, HarnessAssembly::Production),
            [label],
            "{field}"
        );
    }
}

/// A production package for the harness tests: the pilot's descriptors and
/// gate settings (`deploy/pilot-gcp/ingest.env.template`'s floors), built
/// the way `pipeline.py package --bundle production` builds one.
pub(super) fn production_test_package() -> BundlePackage {
    use trace_commons_server::versioned_pipeline_compat::MainGateConfig;
    use trace_commons_server::versioned_pipeline_production::{
        FastEmbedDescriptor, NearAiScorerDescriptor, production_compatibility_package,
    };
    production_compatibility_package(
        &NearAiScorerDescriptor {
            model: "Qwen/Qwen3.6-35B-A3B-FP8".to_string(),
            tail_logprob_cutoff: -8.0,
            logprobs_top_k: 1,
        },
        &FastEmbedDescriptor {
            model_id: "BAAI/bge-large-en-v1.5".to_string(),
            output_dim: 1024,
            max_tokens: 512,
            matryoshka_dim: None,
        },
        &MainGateConfig {
            perplexity_floor_micros: Some(0),
            tail_fraction_floor_micros: Some(0),
            novelty_floor_micros: Some(500_000),
            embed_insert_novelty_micros: 50_000,
            top_k: 5,
            chunk_target_tokens: 2048,
            chunk_max_tokens: 3072,
            chunk_cap: 16,
            chunk_min_tokens: 64,
            novelty_utility_microcredits: COMPATIBILITY_NOVELTY_UTILITY_MICROCREDITS,
        },
    )
    .expect("the production test package builds")
}

/// `package` signed with a generated key, written with its trusted key
/// into `dir`: the two files `promote init` keeps for a run.
pub(super) fn write_signed_test_package(dir: &Path, package: &BundlePackage) -> (PathBuf, PathBuf) {
    let pkcs8 = generated_signing_key();
    let signed = sign_bundle_package(package.clone(), "production_test_key", &pkcs8)
        .expect("sign the production test package");
    let trusted = trusted_key_for_pkcs8("production_test_key", &pkcs8).expect("trusted key");
    let package_path = dir.join("signed-package.json");
    let key_path = dir.join("trusted-package-key.json");
    write_atomically(&package_path, &serde_json::to_vec(&signed).unwrap());
    write_atomically(&key_path, &serde_json::to_vec(&trusted).unwrap());
    (package_path, key_path)
}

/// Production mode end to end (spec B-D1, plan B3) through `run_corpus`, the
/// body the operator's `promote package-checks` runs, over qualified
/// doubles instead of NEAR AI, fastembed and usearch: the production
/// assembler serves the signed production package over real HTTP, every
/// deterministic field matches the corpus's expectations, and the report
/// says it is a production-mode harness report. Production mode refuses a
/// built-in bundle and a reference package before any database is touched.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn production_corpus_run_serves_the_signed_package_over_doubles() {
    let dir = tempfile::tempdir().unwrap();
    let package = production_test_package();
    let (package_path, key_path) = write_signed_test_package(dir.path(), &package);
    let config = |source: CorpusPackageSource, check_id: &str| CorpusRunConfig {
        partitions: vec![("corpus", main_corpus_path())],
        check_id: check_id.to_string(),
        source,
        report_path: dir.path().join("report.json"),
        artifact_root: None,
        master_key_hex: None,
    };
    let signed = CorpusPackageSource::Signed {
        package: package_path,
        trusted_key: key_path,
    };
    let production = config(signed.clone(), "pipeline_http_corpus_compatibility");
    assert_eq!(production.package().unwrap(), package);
    let bundle = config(
        CorpusPackageSource::Bundle("compatibility".to_string()),
        "pipeline_http_corpus_compatibility",
    );
    assert_eq!(
        require_production_corpus_config(&bundle, &bundle.package().unwrap()),
        Err("corpus_production_requires_signed_package".to_string())
    );
    assert_eq!(
        require_production_corpus_config(
            &config(signed.clone(), MINIMAL_CORPUS_CHECK_ID),
            &package
        ),
        Err("corpus_production_check_id_invalid".to_string())
    );
    assert_eq!(
        require_production_corpus_config(&production, &qualification_candidate_package().unwrap()),
        Err("harness_production_package_invalid".to_string())
    );

    if runtime_backend(1).await.is_none() {
        return;
    }
    let index = IsolatedPipelineIndex::new();
    run_corpus(
        &production,
        package.clone(),
        CorpusDependencies::Production(HarnessDependencies {
            scorer: Arc::new(ReferencePerplexityScorer::new()),
            embedder: Arc::new(ReferenceEmbedder::new()),
            index_reader: index.clone(),
            index_writer: index,
        }),
    )
    .await;
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("report.json")).unwrap()).unwrap();
    assert_eq!(report["harness_assembly"], "production");
    assert!(report["fixture_count"].as_u64().unwrap() > 0);
    assert_eq!(report["completed_fixture_count"], report["fixture_count"]);
    assert_eq!(report["failure_count"], 0);
    assert_eq!(
        report["safe_blockers"],
        serde_json::json!(PRODUCTION_HARNESS_BLOCKERS)
    );
    assert_eq!(report["bundle_id"], package.bundle_id);
    assert_eq!(
        report["package_hash"],
        package_digests(&package).unwrap().package_hash
    );
}

/// Spec B-D5, plan B6: `pipeline.py package --bundle production` builds the
/// production package from the deployment's env file alone. The
/// descriptors come from the variables the deployed binary parses, the
/// scorer and embedder are never loaded, and no network is reachable: the
/// package validates as a production package and is the one the production
/// assembler serves for the same configuration.
#[test]
fn production_package_build_needs_no_network() {
    use trace_commons_server::versioned_pipeline_compat::MainGateConfig;

    let env = BTreeMap::from([
        ("TRACE_COMMONS_NEAR_AI_MODEL", "Qwen/Qwen3.6-35B-A3B-FP8"),
        ("TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF", "-8.0"),
        ("TRACE_COMMONS_EMBEDDER_MODEL_ID", "BAAI/bge-large-en-v1.5"),
        ("TRACE_COMMONS_VECTOR_INDEX_DIM", "1024"),
        // Never read: the build has no endpoint and no key.
        (
            "TRACE_COMMONS_NEAR_AI_BASE_URL",
            "https://unreachable.invalid/v1",
        ),
    ]);
    let lookup = |var: &str| env.get(var).map(|value| value.to_string());
    let gate = MainGateConfig {
        perplexity_floor_micros: Some(0),
        tail_fraction_floor_micros: Some(0),
        novelty_floor_micros: Some(500_000),
        embed_insert_novelty_micros: 50_000,
        top_k: 5,
        chunk_target_tokens: 2048,
        chunk_max_tokens: 3072,
        chunk_cap: 16,
        chunk_min_tokens: 64,
        novelty_utility_microcredits: COMPATIBILITY_NOVELTY_UTILITY_MICROCREDITS,
    };
    let package = production_bundle_package(&lookup, &gate).expect("builds offline");
    trace_commons_server::versioned_pipeline_qualification::validate_production_package(&package)
        .expect("a production package");
    assert_eq!(package, production_test_package());
    let pins = production_package_pins(&package).unwrap();
    assert_eq!(pins.main_gate, gate);
    assert_eq!(pins.scorer_descriptor.model, "Qwen/Qwen3.6-35B-A3B-FP8");

    // No model pin, no package; an all-zero floor configuration is refused
    // as `main` refuses it.
    let no_model = |var: &str| {
        (var != "TRACE_COMMONS_NEAR_AI_MODEL")
            .then(|| lookup(var))
            .flatten()
    };
    assert_eq!(
        production_bundle_package(&no_model, &gate)
            .unwrap_err()
            .to_string(),
        "pipeline_scorer_model_missing"
    );
    let zero = MainGateConfig {
        novelty_floor_micros: Some(0),
        ..gate
    };
    assert!(production_bundle_package(&lookup, &zero).is_err());
}
