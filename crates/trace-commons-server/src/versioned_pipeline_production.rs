// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The production pipeline assembly (spec
//! `docs/superpowers/specs/2026-10-08-pipeline-production-assembly-design.md`,
//! Slice A): the descriptors, the adapters, the assembler and the
//! `pipeline_production_adapters` check.
//!
//! Library code, so an integration target can construct the production
//! assembly ([`assemble_production_pipeline`]) over the components it holds
//! (PR #1295 review, Major 1). Everything here takes its inputs as values
//! and trait objects; only the ingest binary reads the environment and
//! names the NEAR AI and fastembed types that fill [`PipelineGateComponents`].
//! [`UsearchPipelineIndex`] needs `near-ai-scorer`, the feature that
//! compiles usearch for the pipeline.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use sha2::{Digest, Sha256};
use trace_commons_gate_api::{
    ChunkPerplexity, Embedder, IdentifiedEmbedder, IdentifiedPerplexityScorer, PerplexityResult,
    PerplexityScorer,
};

use crate::db::postgres::PgBackend;
use crate::trace_artifact_store::TraceArtifactStore;
use crate::trace_authority::{SubmissionAllowlists, SubmissionAuthority};
use crate::versioned_pipeline::{
    PipelineCaps, PipelineLeaseConfig, PipelineNoveltyUtilityChecks, PipelineService,
};
use crate::versioned_pipeline_authority::{PipelineAuthorityProvider, PipelinePrivacyBoundary};
use crate::versioned_pipeline_compat::MainGateConfig;
use crate::versioned_pipeline_qualification::ProductionInfrastructureProfile;

/// The identity the pipeline's NEAR AI scorer adapter reports (spec A-D4).
pub const NEAR_AI_PIPELINE_SCORER_IDENTITY: &str = "near_ai_perplexity_scorer";
/// The identity the pipeline's fastembed embedder adapter reports (spec A-D5).
pub const FASTEMBED_PIPELINE_EMBEDDER_IDENTITY: &str = "fastembed_text_embedder";
/// The identities of the usearch-backed pipeline index (spec A-D6).
pub const USEARCH_PIPELINE_INDEX_READER_IDENTITY: &str = "usearch_pipeline_index_reader";
pub const USEARCH_PIPELINE_INDEX_WRITER_IDENTITY: &str = "usearch_pipeline_index_writer";
/// The identity of the production Trace Credit settlement adapter (spec
/// A-D9).
pub const INTERNAL_TRACE_CREDIT_ADAPTER_IDENTITY: &str = "internal_trace_credit_ledger";
/// The projection and index ids the production compatibility bundle binds
/// (spec A-D10). Neither carries a development marker or `test`, which
/// `validate_production_package` refuses.
pub const PRODUCTION_COMPATIBILITY_PROJECTION_ID: &str = "compatibility_projection_v1";
pub const PRODUCTION_COMPATIBILITY_INDEX_ID: &str = "compatibility_index_v1";

const NEAR_AI_SCORER_DESCRIPTOR_SCHEMA: &str = "trace_commons.near_ai_scorer_descriptor.v1";
const FASTEMBED_EMBEDDER_DESCRIPTOR_SCHEMA: &str = "trace_commons.fastembed_embedder_descriptor.v1";

fn canonical_bytes(value: &serde_json::Value) -> Vec<u8> {
    trace_commons_protocol::canonical_json::to_canonical_vec(value)
        .expect("a descriptor is integers and strings only")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// What the NEAR AI scorer's identity binds (spec A-D4): the hosted model
/// pin and the scorer's own configuration, never its endpoint, key or
/// timeout. Built from the environment by the ingest binary, which makes no
/// network call and loads nothing, so the production package can be built
/// offline.
#[derive(Debug, Clone, PartialEq)]
pub struct NearAiScorerDescriptor {
    pub model: String,
    pub tail_logprob_cutoff: f32,
    pub logprobs_top_k: u32,
}

impl NearAiScorerDescriptor {
    pub fn tail_logprob_cutoff_micros(&self) -> i64 {
        (f64::from(self.tail_logprob_cutoff) * 1_000_000.0).round() as i64
    }

    /// Canonical JSON (sorted keys) of the descriptor: the bytes the bundle
    /// package names the scorer by.
    pub fn bytes(&self) -> Vec<u8> {
        canonical_bytes(&serde_json::json!({
            "schema": NEAR_AI_SCORER_DESCRIPTOR_SCHEMA,
            "model": self.model,
            "tail_logprob_cutoff_micros": self.tail_logprob_cutoff_micros(),
            "logprobs_top_k": self.logprobs_top_k,
        }))
    }

    /// `sha256:<hex>` of [`Self::bytes`].
    pub fn hash(&self) -> String {
        format!("sha256:{}", sha256_hex(&self.bytes()))
    }

    /// The compatibility configuration's `scorer_model_id` (spec A-D10):
    /// `near_ai:` and the descriptor's SHA-256, so the stored configuration
    /// carries no model name in clear.
    pub fn compatibility_scorer_model_id(&self) -> String {
        format!("near_ai:{}", sha256_hex(&self.bytes()))
    }
}

/// What the fastembed embedder's identity binds (spec A-D5). `output_dim`
/// is `TRACE_COMMONS_VECTOR_INDEX_DIM`, which the gate builder requires to
/// equal the loaded model's output dimension, so the descriptor needs no
/// model load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FastEmbedDescriptor {
    pub model_id: String,
    pub output_dim: usize,
    pub max_tokens: usize,
    pub matryoshka_dim: Option<usize>,
}

impl FastEmbedDescriptor {
    pub fn bytes(&self) -> Vec<u8> {
        canonical_bytes(&serde_json::json!({
            "schema": FASTEMBED_EMBEDDER_DESCRIPTOR_SCHEMA,
            "model_id": self.model_id,
            "output_dim": self.output_dim,
            "max_tokens": self.max_tokens,
            "matryoshka_dim": self.matryoshka_dim,
        }))
    }

    pub fn hash(&self) -> String {
        format!("sha256:{}", sha256_hex(&self.bytes()))
    }
}

/// The NEAR AI scorer as the pipeline names it (spec A-D4). Holds the
/// scorer as a trait object -- the legacy gate holds the same one -- and
/// forwards both scoring methods, so the lossless `score_chunk` survives.
pub struct NearAiPipelineScorer {
    inner: Arc<dyn PerplexityScorer>,
    descriptor: NearAiScorerDescriptor,
}

impl NearAiPipelineScorer {
    pub fn new(inner: Arc<dyn PerplexityScorer>, descriptor: NearAiScorerDescriptor) -> Self {
        Self { inner, descriptor }
    }

    pub fn descriptor(&self) -> &NearAiScorerDescriptor {
        &self.descriptor
    }
}

impl PerplexityScorer for NearAiPipelineScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        self.inner.score(plaintext)
    }

    fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
        self.inner.score_chunk(chunk)
    }
}

impl IdentifiedPerplexityScorer for NearAiPipelineScorer {
    fn dependency_identity(&self) -> &str {
        NEAR_AI_PIPELINE_SCORER_IDENTITY
    }

    fn content_descriptor(&self) -> Vec<u8> {
        self.descriptor.bytes()
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

/// The fastembed embedder as the pipeline names it (spec A-D5).
pub struct FastEmbedPipelineEmbedder {
    inner: Arc<dyn Embedder>,
    descriptor: FastEmbedDescriptor,
    index_model_id: String,
}

/// The identifier index entries and sealed index commands record for
/// `model_id`: a sealed command accepts only `[a-z0-9_.-]{1,64}`, which a
/// hosted model id such as `BAAI/bge-large-en-v1.5` is not. Lowercased with
/// every other byte replaced by `_`, or, when that is longer than 64,
/// `fastembed_` and 32 hex characters of the id's SHA-256. The descriptor
/// keeps the configured id, so the package binds the exact model either way.
fn index_model_identifier(model_id: &str) -> String {
    let mapped = model_id
        .chars()
        .map(|c| {
            let c = c.to_ascii_lowercase();
            if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    if !mapped.is_empty() && mapped.len() <= 64 {
        mapped
    } else {
        format!("fastembed_{}", &sha256_hex(model_id.as_bytes())[..32])
    }
}

impl FastEmbedPipelineEmbedder {
    pub fn new(inner: Arc<dyn Embedder>, descriptor: FastEmbedDescriptor) -> Self {
        let index_model_id = index_model_identifier(&descriptor.model_id);
        Self {
            inner,
            descriptor,
            index_model_id,
        }
    }

    pub fn descriptor(&self) -> &FastEmbedDescriptor {
        &self.descriptor
    }
}

impl Embedder for FastEmbedPipelineEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        self.inner.embed(plaintext)
    }
}

impl IdentifiedEmbedder for FastEmbedPipelineEmbedder {
    fn dependency_identity(&self) -> &str {
        FASTEMBED_PIPELINE_EMBEDDER_IDENTITY
    }

    fn model_id(&self) -> &str {
        &self.index_model_id
    }

    fn content_descriptor(&self) -> Vec<u8> {
        self.descriptor.bytes()
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

/// The production `trace_credit` settlement adapter (spec A-D9). Trace
/// Credit has no effect outside the ledger: the pipeline's own Settle
/// transaction writes the `trace_credit_ledger` row, so `settle` answers an
/// internal receipt for the expected result and does nothing else, which
/// makes it idempotent by construction. It refuses any other instrument.
pub struct InternalTraceCreditSettlementAdapter {
    instrument_id: trace_commons_gate_api::pipeline::InstrumentId,
}

impl InternalTraceCreditSettlementAdapter {
    pub fn new() -> Self {
        Self {
            instrument_id: trace_commons_gate_api::pipeline::InstrumentId::trace_credit(),
        }
    }
}

impl Default for InternalTraceCreditSettlementAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl trace_commons_gate_api::SettlementAdapter for InternalTraceCreditSettlementAdapter {
    fn instrument_id(&self) -> &trace_commons_gate_api::pipeline::InstrumentId {
        &self.instrument_id
    }

    fn adapter_identity(&self) -> &str {
        INTERNAL_TRACE_CREDIT_ADAPTER_IDENTITY
    }

    fn production_qualified(&self) -> bool {
        true
    }

    fn payout_rail(&self) -> &str {
        "none"
    }

    async fn settle(
        &self,
        request: &trace_commons_gate_api::SettlementRequest,
    ) -> Result<trace_commons_gate_api::SettlementReceipt, trace_commons_gate_api::SettlementError>
    {
        if request.instrument_id() != &self.instrument_id {
            return Err(trace_commons_gate_api::SettlementError::Rejected);
        }
        trace_commons_gate_api::SettlementReceipt::internal(request.expected_result_ref_hash())
            .map_err(|_| trace_commons_gate_api::SettlementError::Rejected)
    }
}

/// Whether `main` reads a tenant's submission policy from the database
/// (`TRACE_COMMONS_DB_TENANT_POLICY_READS` and its tenant rollout).
pub type DbTenantPolicyReads = Arc<dyn Fn(&str) -> bool + Send + Sync>;

/// The production authority source (spec A-D8, O-A1): the tenant
/// submission policies `main` parsed from `TRACE_COMMONS_TENANT_POLICIES`
/// and `main`'s `TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY`.
///
/// How it mirrors `main`, read from the admission code:
/// - `main` checks a submission against the tenant's policy
///   (`enforce_tenant_submission_policy`) and the token's claim allowlists
///   (`enforce_signed_claim_submission_restrictions`) before it routes the
///   receipt to the pipeline, so `tenant` carries no allowlist of its own:
///   the claim allowlists are per request, and were already applied.
/// - `policy` is the tenant's policy, `None` for a tenant absent from the
///   map, and `require_policy` is `main`'s flag, so an absent tenant is
///   permitted exactly when `main` permits it. The pipeline's
///   `NoveltyUtility` leg reads `policy` as `main`'s utility credit check
///   (`record_matches_utility_credit_policy_abac`) reads the same policy.
/// - A tenant whose policy `main` reads from the database is answered `None`
///   (the pipeline then refuses it with `authority_control_missing`): this
///   provider is synchronous and cannot read the database, and answering
///   from the environment map instead could apply a stale policy.
pub struct TenantPolicyPipelineAuthorityProvider {
    policies: Arc<BTreeMap<String, SubmissionAllowlists>>,
    require_policy: bool,
    db_policy_reads: DbTenantPolicyReads,
}

impl TenantPolicyPipelineAuthorityProvider {
    pub fn new(
        policies: Arc<BTreeMap<String, SubmissionAllowlists>>,
        require_policy: bool,
        db_policy_reads: DbTenantPolicyReads,
    ) -> Self {
        Self {
            policies,
            require_policy,
            db_policy_reads,
        }
    }
}

impl PipelineAuthorityProvider for TenantPolicyPipelineAuthorityProvider {
    fn authority_for_tenant(&self, tenant_id: &str) -> Option<SubmissionAuthority> {
        if (self.db_policy_reads)(tenant_id) {
            return None;
        }
        Some(SubmissionAuthority {
            tenant: SubmissionAllowlists::default(),
            policy: self.policies.get(tenant_id).cloned(),
            require_policy: self.require_policy,
        })
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

pub const PIPELINE_PRODUCTION_COMPONENTS_MISSING_LABEL: &str =
    "pipeline_production_components_missing";

/// The gate components the pipeline shares with the legacy gate (spec
/// A-D3), built once at boot. Holds trait objects and descriptors only:
/// the ingest binary's `near-ai-scorer` builder fills it from the NEAR AI
/// scorer, the fastembed embedder and the usearch index, and a test fills
/// it with qualified doubles.
pub struct PipelineGateComponents {
    pub scorer: Arc<dyn PerplexityScorer>,
    pub scorer_descriptor: NearAiScorerDescriptor,
    pub embedder: Arc<dyn Embedder>,
    pub embedder_descriptor: FastEmbedDescriptor,
    pub index_reader: Arc<dyn trace_commons_gate_api::IdentifiedIndexReader>,
    pub index_writer: Arc<dyn trace_commons_gate_api::IdentifiedIndexWriter>,
    /// Whether the pipeline index root is (or nests with) a legacy root.
    /// The builder refuses that, so a built set always holds `false`; the
    /// adapters check records it.
    pub index_root_shared_with_legacy: bool,
    pub authority: Arc<dyn PipelineAuthorityProvider>,
    pub tenant_policy_count: usize,
    /// `None` when no privacy filter backend is configured; the runtime is
    /// then not production-qualified.
    pub privacy: Option<Arc<dyn PipelinePrivacyBoundary>>,
    pub privacy_backend:
        Option<trace_commons_protocol::trace_contribution::PrivacyFilterBackendTag>,
}

/// The Settle caps of the production assembly: `trace_credit` at `main`'s
/// `NoveltyUtility` delta, the only amount the compatibility Score awards,
/// so an award above it fails as `credit_cap_exceeded` (Settle refuses a
/// leg with no cap at all as `settlement_cap_missing`).
pub fn production_pipeline_caps(main_gate: &MainGateConfig) -> PipelineCaps {
    PipelineCaps {
        per_instrument_atomic_units: BTreeMap::from([(
            trace_commons_gate_api::pipeline::InstrumentId::trace_credit()
                .as_str()
                .to_string(),
            trace_commons_gate_api::pipeline::AtomicUnits::from_raw(u128::from(
                main_gate.novelty_utility_microcredits,
            )),
        )]),
    }
}

/// What the production assembly is built from besides the gate components:
/// the connections and configuration ingest already holds (the ingest
/// binary's `IngestPipelineRuntimeContext` carries the same values).
pub struct ProductionPipelineInputs {
    pub backend: Arc<PgBackend>,
    pub artifact_store: Arc<dyn TraceArtifactStore>,
    pub object_store_name: String,
    pub lease_config: PipelineLeaseConfig,
    pub novelty_utility_checks: PipelineNoveltyUtilityChecks,
    pub unqualified_routing_allowed: bool,
    pub main_gate: MainGateConfig,
    pub components: Arc<PipelineGateComponents>,
}

/// The production pipeline assembly (spec A-D10): the compatibility bundle
/// under `main`'s gate configuration, over the shared NEAR AI scorer,
/// fastembed embedder and usearch pipeline index, the tenant-policy
/// authority, the classifier privacy boundary, and the internal Trace Credit
/// adapter. Never enables payout (spec A-D9).
pub fn assemble_production_pipeline(
    inputs: ProductionPipelineInputs,
) -> anyhow::Result<PipelineService> {
    use crate::versioned_pipeline::PipelineServiceBuilder;
    use crate::versioned_pipeline_bundle::MinimalPolicyBundle;
    use crate::versioned_pipeline_compat::CompatibilityBundleConfig;
    use crate::versioned_pipeline_credit::SettlementAdapterRegistry;

    let components = inputs.components;
    let scorer = Arc::new(NearAiPipelineScorer::new(
        components.scorer.clone(),
        components.scorer_descriptor.clone(),
    ));
    let embedder = Arc::new(FastEmbedPipelineEmbedder::new(
        components.embedder.clone(),
        components.embedder_descriptor.clone(),
    ));
    let config = CompatibilityBundleConfig::production_compatible(
        components.scorer_descriptor.compatibility_scorer_model_id(),
        PRODUCTION_COMPATIBILITY_PROJECTION_ID.to_string(),
        PRODUCTION_COMPATIBILITY_INDEX_ID.to_string(),
        &inputs.main_gate,
    )?;
    let package =
        MinimalPolicyBundle::compatibility_package(&config, scorer.as_ref(), embedder.as_ref())?;
    let registry =
        SettlementAdapterRegistry::new(vec![
            Arc::new(InternalTraceCreditSettlementAdapter::new())
                as Arc<dyn trace_commons_gate_api::SettlementAdapter>,
        ])?;
    let mut builder = PipelineServiceBuilder::new(
        inputs.backend,
        inputs.artifact_store,
        package,
        components.index_reader.clone(),
        components.index_writer.clone(),
        registry,
        production_pipeline_caps(&inputs.main_gate),
    )
    .with_scorer(scorer)
    .with_embedder(embedder)
    .with_authority(components.authority.clone())
    .with_object_store_name(inputs.object_store_name)
    .with_lease_config(inputs.lease_config)
    .with_novelty_utility_checks(inputs.novelty_utility_checks)
    .with_unqualified_routing(inputs.unqualified_routing_allowed);
    if let Some(privacy) = components.privacy.clone() {
        builder = builder.with_privacy(privacy);
    }
    builder.build()
}

/// The check the production assembly's startup emits (spec A-D11).
pub const PRODUCTION_ADAPTERS_CHECK_ID: &str = "pipeline_production_adapters";
const PRODUCTION_ADAPTERS_EVIDENCE_SCHEMA: &str = "trace_commons.pipeline_production_adapters.v1";
pub const PIPELINE_CHECK_REVISION_MISMATCH_LABEL: &str = "pipeline_check_revision_mismatch";

/// The three `TRACE_COMMONS_PIPELINE_CHECK_*` variables, as read.
#[derive(Debug, Clone, Default)]
pub struct PipelineCheckVars {
    pub dir: Option<String>,
    pub run_id: Option<String>,
    pub code_revision_hash: Option<String>,
}

/// What the startup emit did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductionAdaptersEmit {
    /// No `TRACE_COMMONS_PIPELINE_CHECK_*` variable is set.
    NotRequested,
    /// A result was written with this status.
    Emitted(crate::versioned_pipeline_qualification::PipelineCheckStatus),
    /// The directory already holds this check's result; it stands.
    AlreadyEmitted,
}

/// Emits `pipeline_production_adapters` from the startup path (spec A-D11),
/// once the production runtime has passed every startup refusal. Only the
/// deployed host can pass it: the infrastructure profile is production only
/// with the GCS store, the Cloud KMS key wrapper and managed EdDSA tokens.
///
/// - No variable set: nothing happens. A partial or malformed set refuses
///   the start with `PipelineCheckEmitter`'s label (never `emit_from_env`,
///   which panics).
/// - The variables' revision must be the build's
///   (`DEPLOYED_CODE_REVISION_HASH`), else `pipeline_check_revision_mismatch`
///   refuses the start: a result names the revision that produced it.
/// - A result already in the directory is logged
///   (`pipeline_production_adapters_already_emitted`) and the start
///   continues.
/// - `pass` exactly when the default bundle's dependencies, the
///   infrastructure, the privacy boundary's prose-PII classification and the
///   package's production markers have no blocker; otherwise `fail` with
///   those blockers. It names the default package (spec R-1).
pub fn emit_production_adapters_check(
    vars: PipelineCheckVars,
    deployed_code_revision: Option<&str>,
    service: &PipelineService,
    components: &PipelineGateComponents,
    infrastructure: ProductionInfrastructureProfile,
    near_settlement_mode: &str,
) -> anyhow::Result<ProductionAdaptersEmit> {
    use crate::versioned_pipeline_qualification::{PipelineCheckEmitter, PipelineCheckStatus};
    let dir = vars.dir.clone();
    let requested_revision = vars.code_revision_hash.clone();
    let Some(emitter) =
        PipelineCheckEmitter::from_vars(vars.dir, vars.run_id, vars.code_revision_hash)
            .map_err(|label| anyhow::anyhow!(label))?
    else {
        return Ok(ProductionAdaptersEmit::NotRequested);
    };
    anyhow::ensure!(
        deployed_code_revision.is_some() && deployed_code_revision == requested_revision.as_deref(),
        PIPELINE_CHECK_REVISION_MISMATCH_LABEL
    );
    let dir = PathBuf::from(dir.unwrap_or_default());
    if dir
        .join(format!("{PRODUCTION_ADAPTERS_CHECK_ID}.result.json"))
        .exists()
    {
        tracing::warn!("pipeline_production_adapters_already_emitted");
        return Ok(ProductionAdaptersEmit::AlreadyEmitted);
    }
    let (blockers, evidence) =
        production_adapters_observation(service, components, infrastructure, near_settlement_mode)?;
    let status = if blockers.is_empty() {
        PipelineCheckStatus::Pass
    } else {
        PipelineCheckStatus::Fail
    };
    let blocker_refs = blockers.iter().map(String::as_str).collect::<Vec<_>>();
    emitter
        .emit(
            PRODUCTION_ADAPTERS_CHECK_ID,
            status,
            Some(service.default_package()),
            &blocker_refs,
            evidence,
        )
        .map_err(|label| anyhow::anyhow!(label))?;
    tracing::info!(
        status = if status == PipelineCheckStatus::Pass {
            "pass"
        } else {
            "fail"
        },
        "pipeline_production_adapters_emitted"
    );
    Ok(ProductionAdaptersEmit::Emitted(status))
}

/// The blockers (sorted, labels only) and the evidence (labels, digests and
/// counts only: no URL, key, tenant id or model name in clear) of
/// `pipeline_production_adapters`.
fn production_adapters_observation(
    service: &PipelineService,
    components: &PipelineGateComponents,
    infrastructure: ProductionInfrastructureProfile,
    near_settlement_mode: &str,
) -> anyhow::Result<(Vec<String>, serde_json::Value)> {
    use crate::versioned_pipeline_qualification::{
        ProductionDependencyProfile, is_safe_label, validate_production_package,
    };
    let package = service.default_package();
    let bundle = service
        .bundle_qualification(package)
        .map_err(|label| anyhow::anyhow!(label))?;
    let bundle_blockers = bundle
        .blockers()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let profile = ProductionDependencyProfile::new(bundle.clone(), infrastructure);
    let infrastructure_blockers = profile
        .blockers()
        .into_iter()
        .skip(bundle_blockers.len())
        .collect::<Vec<_>>();
    let runtime_identity_digest = profile
        .runtime_identity_digest()
        .map_err(|label| anyhow::anyhow!(label))?;

    let mut blockers = bundle_blockers.clone();
    blockers.extend(infrastructure_blockers.iter().cloned());
    let classifies_prose_pii = service.privacy_classifies_prose_pii();
    if !classifies_prose_pii {
        blockers.push("pipeline_privacy_filter_required".to_string());
    }
    if components.index_root_shared_with_legacy {
        blockers.push(PIPELINE_VECTOR_INDEX_ROOT_SHARED_LABEL.to_string());
    }
    if let Err(label) = validate_production_package(package) {
        blockers.push(if is_safe_label(&label) {
            label
        } else {
            "pipeline_package_not_production".to_string()
        });
    }
    blockers.sort();
    blockers.dedup();

    let settlement_adapters = bundle
        .settlement_adapters
        .iter()
        .map(|(instrument, check)| {
            (
                instrument.clone(),
                serde_json::json!({
                    "identity": check.identity,
                    "payout_rail": if check.identity == INTERNAL_TRACE_CREDIT_ADAPTER_IDENTITY {
                        "none"
                    } else {
                        "unrecorded"
                    },
                    "qualified": check.production_qualified,
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    let evidence = serde_json::json!({
        "schema": PRODUCTION_ADAPTERS_EVIDENCE_SCHEMA,
        "runtime_identity_digest": runtime_identity_digest,
        "scorer": {
            "identity": bundle.scorer.identity,
            "descriptor_hash": components.scorer_descriptor.hash(),
            "qualified": bundle.scorer.production_qualified,
        },
        "embedder": {
            "identity": bundle.embedder.identity,
            "descriptor_hash": components.embedder_descriptor.hash(),
            "output_dim": components.embedder_descriptor.output_dim,
            "qualified": bundle.embedder.production_qualified,
        },
        "index_reader": {
            "identity": bundle.index_reader.identity,
            "qualified": bundle.index_reader.production_qualified,
        },
        "index_writer": {
            "identity": bundle.index_writer.identity,
            "qualified": bundle.index_writer.production_qualified,
        },
        "index_root_shared_with_legacy": components.index_root_shared_with_legacy,
        "settlement_adapters": settlement_adapters,
        "payout_enabled": service.payout_enabled(),
        "near_settlement_mode": near_settlement_mode,
        "authority": {
            "qualified": bundle.authority,
            "tenant_policy_count": components.tenant_policy_count,
        },
        "privacy": {
            "backend": components.privacy_backend.map_or("none", |backend| backend.label()),
            "classifies_prose_pii": classifies_prose_pii,
            "qualified": bundle.privacy,
        },
        "compatibility_configuration_qualifiable": bundle.configuration_qualifiable,
        "infrastructure_blockers": infrastructure_blockers,
        "bundle_blockers": bundle_blockers,
    });
    Ok((blockers, evidence))
}

pub const PIPELINE_VECTOR_INDEX_ROOT_SHARED_LABEL: &str = "pipeline_vector_index_root_shared";
pub const PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL: &str =
    "pipeline_vector_index_manifest_mismatch";

/// `path` made absolute against the working directory and with `.` and `..`
/// resolved lexically, so a root that does not exist yet (the dedup default
/// is `<novelty_root>/../dedup-index`) still compares.
fn normalized_root(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Refuses, with `pipeline_vector_index_root_shared`, a pipeline index root
/// equal to, or nested in either direction with, any of `legacy_roots`: two
/// usearch indexes on one root corrupt each other's `nearest`.
pub fn validate_pipeline_index_root(
    pipeline_root: &std::path::Path,
    legacy_roots: &[&std::path::Path],
) -> anyhow::Result<()> {
    let pipeline_root = normalized_root(pipeline_root);
    for legacy in legacy_roots {
        let legacy = normalized_root(legacy);
        anyhow::ensure!(
            !(pipeline_root.starts_with(&legacy) || legacy.starts_with(&pipeline_root)),
            PIPELINE_VECTOR_INDEX_ROOT_SHARED_LABEL
        );
    }
    Ok(())
}

#[cfg(feature = "near-ai-scorer")]
pub use usearch_pipeline_index::UsearchPipelineIndex;

#[cfg(feature = "near-ai-scorer")]
mod usearch_pipeline_index {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use trace_commons_gate_api::pipeline::TenantStorageRef;
    use trace_commons_gate_api::{
        IdentifiedIndexReader, IdentifiedIndexWriter, IndexEntryKey, IndexSnapshot,
        IndexUpsertResult, IndexWriteError, NearestNeighbor, VectorIndex, VectorIndexReader,
        VectorIndexWriter,
    };
    use trace_commons_gate_enclave::vector_index_usearch::{
        UsearchVectorIndex, UsearchVectorIndexConfig,
    };
    use uuid::Uuid;

    use super::{
        PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL, USEARCH_PIPELINE_INDEX_READER_IDENTITY,
        USEARCH_PIPELINE_INDEX_WRITER_IDENTITY,
    };

    const MANIFEST_SCHEMA: &str = "trace_commons.pipeline_vector_index_manifest.v1";
    const MANIFEST_SUFFIX: &str = ".manifest.json";
    /// The extension of the usearch implementation's per-namespace files.
    const USEARCH_SUFFIX: &str = ".usearch";

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct ManifestEntry {
        revision_id: Uuid,
        /// Binds the stored embedding to its content hash, so a second
        /// upsert under one key with either changed is a conflict.
        content_digest: String,
        content_hash: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ManifestFile {
        schema: String,
        namespace: String,
        entries: BTreeMap<Uuid, ManifestEntry>,
    }

    /// The entries of one `(tenant_storage_ref, index_id)` namespace.
    #[derive(Debug, Default)]
    struct Namespace {
        entries: BTreeMap<Uuid, ManifestEntry>,
    }

    /// The pipeline's vector index (spec A-D6): the legacy gate's usearch
    /// implementation on its own root, held as a trait object, with a
    /// manifest per namespace that supplies what usearch does not -- the real
    /// entry id behind a usearch key, each entry's revision and content
    /// digest (conflict detection, `exclude_revision`,
    /// `invalidate_revision`), and the snapshot hash.
    ///
    /// Every write is persisted before it returns: usearch is flushed, then
    /// the namespace's manifest is written atomically. A crash between the
    /// two leaves them disagreeing, and the next start refuses
    /// (`pipeline_vector_index_manifest_mismatch`); the operator then
    /// rebuilds the tenant's index into an emptied root. Calls are
    /// synchronous; the pipeline runs index calls on the blocking pool.
    pub struct UsearchPipelineIndex {
        root: PathBuf,
        index: Arc<dyn VectorIndex>,
        namespaces: Mutex<BTreeMap<String, Namespace>>,
    }

    fn namespace_of(tenant_storage_ref: &TenantStorageRef, index_id: &str) -> String {
        format!("pipeline:{index_id}:{}", tenant_storage_ref.as_str())
    }

    fn manifest_path(root: &Path, namespace: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(b"trace_commons.pipeline_vector_index_manifest_file.v1\n");
        hasher.update(namespace.as_bytes());
        root.join(format!("{:x}{MANIFEST_SUFFIX}", hasher.finalize()))
    }

    fn usearch_key(entry_id: Uuid) -> [u8; 8] {
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&entry_id.as_bytes()[..8]);
        prefix
    }

    fn content_digest(embedding: &[f32], content_hash: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content_hash.as_bytes());
        hasher.update(b"\0");
        for value in embedding {
            hasher.update(value.to_le_bytes());
        }
        format!("sha256:{:x}", hasher.finalize())
    }

    fn mismatch() -> anyhow::Error {
        anyhow::anyhow!(PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL)
    }

    impl UsearchPipelineIndex {
        /// Opens (creating when absent) the index at `root`, and refuses with
        /// `pipeline_vector_index_manifest_mismatch` when any manifest there
        /// is unreadable or does not hold exactly as many entries as its
        /// usearch file.
        pub fn open(root: &Path, config: UsearchVectorIndexConfig) -> anyhow::Result<Self> {
            let usearch = UsearchVectorIndex::try_new(root, config)
                .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
            let index: Arc<dyn VectorIndex> = Arc::new(usearch);
            let mut namespaces = BTreeMap::new();
            let mut usearch_files = 0_usize;
            let listing = std::fs::read_dir(root)
                .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
            for entry in listing {
                let path = entry
                    .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?
                    .path();
                let name = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default();
                if name.ends_with(USEARCH_SUFFIX) {
                    usearch_files += 1;
                }
                if !name.ends_with(MANIFEST_SUFFIX) {
                    continue;
                }
                let bytes = std::fs::read(&path).map_err(|_| mismatch())?;
                let manifest: ManifestFile =
                    serde_json::from_slice(&bytes).map_err(|_| mismatch())?;
                if manifest.schema != MANIFEST_SCHEMA
                    || manifest_path(root, &manifest.namespace) != path
                {
                    return Err(mismatch());
                }
                let stored = index
                    .snapshot(&manifest.namespace)
                    .ok_or_else(mismatch)?
                    .cardinality;
                if stored != manifest.entries.len() as u64 {
                    return Err(mismatch());
                }
                namespaces.insert(
                    manifest.namespace,
                    Namespace {
                        entries: manifest.entries,
                    },
                );
            }
            // Every usearch file this index writes has a manifest (the
            // manifest is written right after the flush that creates the
            // file). One without -- a crash between the two on a namespace's
            // first write -- holds entries no manifest names.
            if usearch_files > namespaces.len() {
                return Err(mismatch());
            }
            Ok(Self {
                root: root.to_path_buf(),
                index,
                namespaces: Mutex::new(namespaces),
            })
        }

        /// Flushes usearch, then writes `namespace`'s manifest atomically.
        fn persist(
            &self,
            namespace: &str,
            entries: &BTreeMap<Uuid, ManifestEntry>,
        ) -> anyhow::Result<()> {
            self.index.flush()?;
            let file = ManifestFile {
                schema: MANIFEST_SCHEMA.to_string(),
                namespace: namespace.to_string(),
                entries: entries.clone(),
            };
            let bytes = serde_json::to_vec(&file)?;
            let path = manifest_path(&self.root, namespace);
            let staging = path.with_extension("json.staging");
            std::fs::write(&staging, &bytes)?;
            std::fs::File::open(&staging)?.sync_all()?;
            std::fs::rename(&staging, &path)?;
            Ok(())
        }
    }

    impl VectorIndexReader for UsearchPipelineIndex {
        /// The hash is SHA-256 over every entry's id and raw content hash in
        /// ascending entry-id order (the manifest's order), so it does not
        /// depend on write order.
        fn snapshot(
            &self,
            tenant_storage_ref: &TenantStorageRef,
            index_id: &str,
        ) -> anyhow::Result<IndexSnapshot> {
            let namespaces = self.namespaces.lock().expect("pipeline index mutex");
            let mut hasher = Sha256::new();
            let mut cardinality = 0_u64;
            if let Some(namespace) = namespaces.get(&namespace_of(tenant_storage_ref, index_id)) {
                for (entry_id, entry) in &namespace.entries {
                    cardinality += 1;
                    hasher.update(entry_id.as_bytes());
                    hasher.update(entry.content_hash.as_bytes());
                }
            }
            let snapshot_hash = format!("sha256:{:x}", hasher.finalize());
            Ok(IndexSnapshot {
                snapshot_id: snapshot_hash.clone(),
                snapshot_hash,
                cardinality,
            })
        }

        fn nearest(
            &self,
            tenant_storage_ref: &TenantStorageRef,
            index_id: &str,
            embedding: &[f32],
            k: usize,
            exclude_revision: Option<Uuid>,
        ) -> anyhow::Result<Vec<NearestNeighbor>> {
            let namespace = namespace_of(tenant_storage_ref, index_id);
            let namespaces = self.namespaces.lock().expect("pipeline index mutex");
            let Some(entries) = namespaces.get(&namespace).map(|ns| &ns.entries) else {
                return Ok(Vec::new());
            };
            if k == 0 || entries.is_empty() {
                return Ok(Vec::new());
            }
            let excluded = exclude_revision.map_or(0, |revision| {
                entries
                    .values()
                    .filter(|entry| entry.revision_id == revision)
                    .count()
            });
            let by_key = entries
                .iter()
                .map(|(entry_id, entry)| (usearch_key(*entry_id), (*entry_id, entry)))
                .collect::<BTreeMap<_, _>>();
            let mut neighbors = Vec::new();
            for hit in self.index.nearest(&namespace, embedding, k + excluded)? {
                let Some((entry_id, entry)) = by_key.get(&usearch_key(hit.entry_id)) else {
                    continue;
                };
                if exclude_revision == Some(entry.revision_id) {
                    continue;
                }
                neighbors.push(NearestNeighbor {
                    entry_id: *entry_id,
                    similarity: hit.similarity,
                });
                if neighbors.len() == k {
                    break;
                }
            }
            Ok(neighbors)
        }
    }

    impl VectorIndexWriter for UsearchPipelineIndex {
        fn upsert(
            &self,
            key: &IndexEntryKey,
            embedding: &[f32],
            content_hash: &str,
        ) -> Result<IndexUpsertResult, IndexWriteError> {
            let namespace = namespace_of(&key.tenant_storage_ref, &key.index_id);
            let entry_id = key.entry_id();
            let digest = content_digest(embedding, content_hash);
            let mut namespaces = self.namespaces.lock().expect("pipeline index mutex");
            let entries = &mut namespaces.entry(namespace.clone()).or_default().entries;
            if let Some(existing) = entries.get(&entry_id) {
                return if existing.content_digest == digest {
                    Ok(IndexUpsertResult::Unchanged)
                } else {
                    Err(IndexWriteError::ContentConflict)
                };
            }
            // usearch keys an entry by the first eight bytes of its id; a
            // second id with the same prefix would silently replace the
            // first, so it is refused instead.
            if entries
                .keys()
                .any(|existing| usearch_key(*existing) == usearch_key(entry_id))
            {
                return Err(IndexWriteError::Failed);
            }
            self.index
                .insert(entry_id, &namespace, embedding)
                .map_err(|_| IndexWriteError::Failed)?;
            entries.insert(
                entry_id,
                ManifestEntry {
                    revision_id: key.revision_id,
                    content_digest: digest,
                    content_hash: content_hash.to_string(),
                },
            );
            let entries = entries.clone();
            self.persist(&namespace, &entries)
                .map_err(|_| IndexWriteError::Uncertain)?;
            Ok(IndexUpsertResult::Inserted)
        }

        fn invalidate_revision(
            &self,
            tenant_storage_ref: &TenantStorageRef,
            index_id: &str,
            revision_id: Uuid,
        ) -> Result<bool, IndexWriteError> {
            let namespace = namespace_of(tenant_storage_ref, index_id);
            let mut namespaces = self.namespaces.lock().expect("pipeline index mutex");
            let Some(entries) = namespaces.get_mut(&namespace).map(|ns| &mut ns.entries) else {
                return Ok(false);
            };
            let doomed = entries
                .iter()
                .filter(|(_, entry)| entry.revision_id == revision_id)
                .map(|(entry_id, _)| *entry_id)
                .collect::<Vec<_>>();
            if doomed.is_empty() {
                return Ok(false);
            }
            for (removed, entry_id) in doomed.iter().enumerate() {
                if self.index.delete(&namespace, *entry_id).is_err() {
                    return Err(if removed == 0 {
                        IndexWriteError::Failed
                    } else {
                        IndexWriteError::Uncertain
                    });
                }
                entries.remove(entry_id);
            }
            let entries = entries.clone();
            self.persist(&namespace, &entries)
                .map_err(|_| IndexWriteError::Uncertain)?;
            Ok(true)
        }
    }

    impl IdentifiedIndexReader for UsearchPipelineIndex {
        fn dependency_identity(&self) -> &str {
            USEARCH_PIPELINE_INDEX_READER_IDENTITY
        }

        fn production_qualified(&self) -> bool {
            true
        }
    }

    impl IdentifiedIndexWriter for UsearchPipelineIndex {
        fn dependency_identity(&self) -> &str {
            USEARCH_PIPELINE_INDEX_WRITER_IDENTITY
        }

        fn production_qualified(&self) -> bool {
            true
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::versioned_pipeline_production::{
            PRODUCTION_COMPATIBILITY_INDEX_ID, PRODUCTION_COMPATIBILITY_PROJECTION_ID,
        };
        use trace_commons_gate_api::pipeline::TenantStorageRef;
        use trace_commons_gate_api::{
            IndexEntryKey, IndexUpsertResult, IndexWriteError, VectorIndexReader, VectorIndexWriter,
        };

        const DIM: usize = 4;

        fn usearch_test_config(
            dim: usize,
        ) -> trace_commons_gate_enclave::vector_index_usearch::UsearchVectorIndexConfig {
            trace_commons_gate_enclave::vector_index_usearch::UsearchVectorIndexConfig {
                dim,
                hnsw_m: 16,
                ef_construction: 64,
                ef_search: 64,
                max_open: 8,
                flush_every: 1_000,
                flush_interval: None,
            }
        }

        fn open(root: &std::path::Path) -> anyhow::Result<UsearchPipelineIndex> {
            UsearchPipelineIndex::open(root, usearch_test_config(DIM))
        }

        fn tenant(n: u8) -> TenantStorageRef {
            TenantStorageRef::new(format!("tenant_sha256:{}", format!("{n:02x}").repeat(16)))
                .unwrap()
        }

        fn key(tenant_ref: &TenantStorageRef, revision: u128, chunk: u32) -> IndexEntryKey {
            IndexEntryKey {
                tenant_storage_ref: tenant_ref.clone(),
                index_id: PRODUCTION_COMPATIBILITY_INDEX_ID.to_string(),
                revision_id: Uuid::from_u128(revision),
                projection_id: PRODUCTION_COMPATIBILITY_PROJECTION_ID.to_string(),
                model_id: "BAAI/bge-large-en-v1.5".to_string(),
                chunk,
            }
        }

        fn unit(axis: usize) -> Vec<f32> {
            let mut vector = vec![0.0; DIM];
            vector[axis] = 1.0;
            vector
        }

        /// The writer contract `IsolatedPipelineIndex`'s tests cover, against
        /// the usearch-backed index.
        #[test]
        fn usearch_pipeline_index_meets_the_writer_contract() {
            let dir = tempfile::tempdir().unwrap();
            let index = open(dir.path()).unwrap();
            let a = tenant(1);
            let b = tenant(2);
            let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;

            let first = key(&a, 1, 0);
            assert_eq!(
                index.upsert(&first, &unit(0), "sha256:one").unwrap(),
                IndexUpsertResult::Inserted
            );
            assert_eq!(
                index.upsert(&first, &unit(0), "sha256:one").unwrap(),
                IndexUpsertResult::Unchanged
            );
            assert_eq!(
                index.upsert(&first, &unit(0), "sha256:other").unwrap_err(),
                IndexWriteError::ContentConflict
            );
            assert_eq!(
                index.upsert(&first, &unit(1), "sha256:one").unwrap_err(),
                IndexWriteError::ContentConflict
            );
            index
                .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
                .unwrap();
            index
                .upsert(&key(&b, 3, 0), &unit(0), "sha256:three")
                .unwrap();

            // `nearest` answers the real entry ids, best first, within one
            // tenant, and leaves out the excluded revision.
            let nearest = index.nearest(&a, index_id, &unit(0), 2, None).unwrap();
            assert_eq!(nearest.len(), 2);
            assert_eq!(nearest[0].entry_id, first.entry_id());
            assert!((nearest[0].similarity - 1.0).abs() < 1e-4);
            assert_eq!(nearest[1].entry_id, key(&a, 2, 0).entry_id());
            let excluded = index
                .nearest(&a, index_id, &unit(0), 2, Some(Uuid::from_u128(1)))
                .unwrap();
            assert_eq!(
                excluded
                    .iter()
                    .map(|neighbor| neighbor.entry_id)
                    .collect::<Vec<_>>(),
                vec![key(&a, 2, 0).entry_id()]
            );
            assert!(
                index
                    .nearest(&a, "another_index", &unit(0), 2, None)
                    .unwrap()
                    .is_empty()
            );

            assert_eq!(index.snapshot(&a, index_id).unwrap().cardinality, 2);
            assert_eq!(index.snapshot(&b, index_id).unwrap().cardinality, 1);

            // `invalidate_revision` removes that revision's entries only, and is
            // idempotent.
            assert!(
                index
                    .invalidate_revision(&a, index_id, Uuid::from_u128(1))
                    .unwrap()
            );
            assert!(
                !index
                    .invalidate_revision(&a, index_id, Uuid::from_u128(1))
                    .unwrap()
            );
            assert_eq!(index.snapshot(&a, index_id).unwrap().cardinality, 1);
            assert_eq!(index.snapshot(&b, index_id).unwrap().cardinality, 1);
            let after = index.nearest(&a, index_id, &unit(0), 5, None).unwrap();
            assert_eq!(
                after
                    .iter()
                    .map(|neighbor| neighbor.entry_id)
                    .collect::<Vec<_>>(),
                vec![key(&a, 2, 0).entry_id()]
            );
            // A removed entry may be written again.
            assert_eq!(
                index.upsert(&first, &unit(0), "sha256:one").unwrap(),
                IndexUpsertResult::Inserted
            );
        }

        /// The snapshot hash depends on the entries, not the order they were
        /// written in.
        #[test]
        fn usearch_pipeline_snapshot_hash_is_order_independent() {
            let a = tenant(1);
            let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;
            let entries = [
                (key(&a, 1, 0), unit(0), "sha256:one"),
                (key(&a, 1, 1), unit(1), "sha256:two"),
                (key(&a, 2, 0), unit(2), "sha256:three"),
            ];
            let forward_dir = tempfile::tempdir().unwrap();
            let forward = open(forward_dir.path()).unwrap();
            for (entry, embedding, hash) in &entries {
                forward.upsert(entry, embedding, hash).unwrap();
            }
            let reverse_dir = tempfile::tempdir().unwrap();
            let reverse = open(reverse_dir.path()).unwrap();
            for (entry, embedding, hash) in entries.iter().rev() {
                reverse.upsert(entry, embedding, hash).unwrap();
            }
            let left = forward.snapshot(&a, index_id).unwrap();
            let right = reverse.snapshot(&a, index_id).unwrap();
            assert_eq!(left, right);
            assert_eq!(left.cardinality, 3);
            let empty = open(tempfile::tempdir().unwrap().path())
                .unwrap()
                .snapshot(&a, index_id)
                .unwrap();
            assert_ne!(empty.snapshot_hash, left.snapshot_hash);
            assert_eq!(empty.cardinality, 0);
        }

        /// The manifest is persisted beside the usearch files, so a reopened
        /// index answers the same entries, snapshot and real entry ids.
        #[test]
        fn manifest_survives_reopen() {
            let dir = tempfile::tempdir().unwrap();
            let a = tenant(1);
            let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;
            let before = {
                let index = open(dir.path()).unwrap();
                index
                    .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
                    .unwrap();
                index
                    .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
                    .unwrap();
                index.snapshot(&a, index_id).unwrap()
            };
            let reopened = open(dir.path()).unwrap();
            assert_eq!(reopened.snapshot(&a, index_id).unwrap(), before);
            assert_eq!(
                reopened
                    .nearest(&a, index_id, &unit(1), 1, None)
                    .unwrap()
                    .first()
                    .map(|neighbor| neighbor.entry_id),
                Some(key(&a, 2, 0).entry_id())
            );
            assert_eq!(
                reopened
                    .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
                    .unwrap(),
                IndexUpsertResult::Unchanged
            );
        }

        /// A manifest that does not match its usearch file refuses the start.
        #[test]
        fn truncated_manifest_refuses_open() {
            let dir = tempfile::tempdir().unwrap();
            let a = tenant(1);
            {
                let index = open(dir.path()).unwrap();
                index
                    .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
                    .unwrap();
                index
                    .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
                    .unwrap();
            }
            let manifests = std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.to_string_lossy().ends_with(".manifest.json"))
                .collect::<Vec<_>>();
            assert_eq!(manifests.len(), 1);
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&manifests[0]).unwrap()).unwrap();
            let mut truncated = manifest.clone();
            let entries = truncated["entries"].as_object_mut().unwrap();
            let first = entries.keys().next().unwrap().clone();
            entries.remove(&first);
            std::fs::write(&manifests[0], serde_json::to_vec(&truncated).unwrap()).unwrap();
            assert_eq!(
                open(dir.path()).err().unwrap().to_string(),
                "pipeline_vector_index_manifest_mismatch"
            );
            std::fs::write(&manifests[0], b"{not json").unwrap();
            assert_eq!(
                open(dir.path()).err().unwrap().to_string(),
                "pipeline_vector_index_manifest_mismatch"
            );
        }

        /// A crash between the usearch flush and the first manifest write of a
        /// namespace leaves a usearch file no manifest describes; the start
        /// refuses it rather than serve a namespace whose entries it cannot name.
        #[test]
        fn an_orphaned_usearch_file_refuses_open() {
            let dir = tempfile::tempdir().unwrap();
            let a = tenant(1);
            {
                let index = open(dir.path()).unwrap();
                index
                    .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
                    .unwrap();
            }
            let files = |suffix: &str| {
                std::fs::read_dir(dir.path())
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.to_string_lossy().ends_with(suffix))
                    .collect::<Vec<_>>()
            };
            assert_eq!(files(".usearch").len(), 1);
            for manifest in files(".manifest.json") {
                std::fs::remove_file(manifest).unwrap();
            }
            assert_eq!(
                open(dir.path()).err().unwrap().to_string(),
                "pipeline_vector_index_manifest_mismatch"
            );
        }

        /// Every write persists before it returns, and returns well within the
        /// 60 s index write fence margin (measured with a generous bound).
        #[test]
        fn flush_returns_within_the_fence_margin() {
            let dir = tempfile::tempdir().unwrap();
            let index = open(dir.path()).unwrap();
            let a = tenant(1);
            let started = std::time::Instant::now();
            for revision in 0..50 {
                index
                    .upsert(
                        &key(&a, revision, 0),
                        &unit((revision % 4) as usize),
                        "sha256:x",
                    )
                    .unwrap();
            }
            index
                .invalidate_revision(&a, PRODUCTION_COMPATIBILITY_INDEX_ID, Uuid::from_u128(3))
                .unwrap();
            let elapsed = started.elapsed();
            assert!(
                elapsed
                    < std::time::Duration::from_secs(
                        crate::versioned_pipeline::PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS as u64
                            / 4
                    ),
                "{elapsed:?}"
            );
            drop(index);
            assert_eq!(
                open(dir.path())
                    .unwrap()
                    .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
                    .unwrap()
                    .cardinality,
                49
            );
        }
    }
}
