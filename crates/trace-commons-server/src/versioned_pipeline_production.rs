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

    /// The descriptor whose [`Self::bytes`] are exactly `bytes` (a package's
    /// stored scorer descriptor), or `None`. The cutoff comes back from its
    /// micros, and the round trip is checked, so a descriptor this cannot
    /// reproduce byte for byte is refused rather than approximated.
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let object = value.as_object()?;
        if object.len() != 4 || object.get("schema")?.as_str()? != NEAR_AI_SCORER_DESCRIPTOR_SCHEMA
        {
            return None;
        }
        let micros = object.get("tail_logprob_cutoff_micros")?.as_i64()?;
        let descriptor = Self {
            model: object.get("model")?.as_str()?.to_string(),
            tail_logprob_cutoff: (micros as f64 / 1_000_000.0) as f32,
            logprobs_top_k: u32::try_from(object.get("logprobs_top_k")?.as_u64()?).ok()?,
        };
        (descriptor.bytes() == bytes).then_some(descriptor)
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

    /// The descriptor whose [`Self::bytes`] are exactly `bytes`, or `None`
    /// (see [`NearAiScorerDescriptor::from_bytes`]).
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let object = value.as_object()?;
        if object.len() != 5
            || object.get("schema")?.as_str()? != FASTEMBED_EMBEDDER_DESCRIPTOR_SCHEMA
        {
            return None;
        }
        let size =
            |field: &str| -> Option<usize> { usize::try_from(object.get(field)?.as_u64()?).ok() };
        let matryoshka_dim = match object.get("matryoshka_dim")? {
            serde_json::Value::Null => None,
            _ => Some(size("matryoshka_dim")?),
        };
        let descriptor = Self {
            model_id: object.get("model_id")?.as_str()?.to_string(),
            output_dim: size("output_dim")?,
            max_tokens: size("max_tokens")?,
            matryoshka_dim,
        };
        (descriptor.bytes() == bytes).then_some(descriptor)
    }
}

/// Stands in for the scorer and embedder when only a package is built: the
/// package names its dependencies by their descriptors alone, so building it
/// loads no model and calls no network. Never served.
struct NotLoaded;

const PRODUCTION_DEPENDENCY_NOT_LOADED_LABEL: &str = "production_dependency_not_loaded";

impl PerplexityScorer for NotLoaded {
    fn score(&self, _plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        anyhow::bail!(PRODUCTION_DEPENDENCY_NOT_LOADED_LABEL)
    }

    fn score_chunk(&self, _chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
        anyhow::bail!(PRODUCTION_DEPENDENCY_NOT_LOADED_LABEL)
    }
}

impl Embedder for NotLoaded {
    fn embed(&self, _plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        anyhow::bail!(PRODUCTION_DEPENDENCY_NOT_LOADED_LABEL)
    }
}

/// The production compatibility package (spec A-D10, B-D5): the package
/// [`assemble_production_pipeline`] serves for these descriptors and `main`'s
/// gate configuration. It reads only the descriptors, so `pipeline.py
/// package --bundle production` builds it offline, and the assembler builds
/// its own package through this same function, so the two cannot differ.
pub fn production_compatibility_package(
    scorer_descriptor: &NearAiScorerDescriptor,
    embedder_descriptor: &FastEmbedDescriptor,
    main_gate: &MainGateConfig,
) -> anyhow::Result<trace_commons_gate_api::pipeline::BundlePackage> {
    use crate::versioned_pipeline_bundle::MinimalPolicyBundle;
    use crate::versioned_pipeline_compat::CompatibilityBundleConfig;

    let config = CompatibilityBundleConfig::production_compatible(
        scorer_descriptor.compatibility_scorer_model_id(),
        PRODUCTION_COMPATIBILITY_PROJECTION_ID.to_string(),
        PRODUCTION_COMPATIBILITY_INDEX_ID.to_string(),
        main_gate,
    )?;
    let scorer = NearAiPipelineScorer::new(Arc::new(NotLoaded), scorer_descriptor.clone());
    let embedder = FastEmbedPipelineEmbedder::new(Arc::new(NotLoaded), embedder_descriptor.clone());
    MinimalPolicyBundle::compatibility_package(&config, &scorer, &embedder)
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
    let package = production_compatibility_package(
        &components.scorer_descriptor,
        &components.embedder_descriptor,
        &inputs.main_gate,
    )?;
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

/// `path` as the filesystem resolves it: made absolute against the working
/// directory, its longest existing ancestor canonicalized (symlinks and
/// `..` resolved), and the components that do not exist yet appended with
/// `.` and `..` resolved lexically -- a root that does not exist yet (the
/// dedup default is `<novelty_root>/../dedup-index`) still compares. An
/// ancestor that cannot be canonicalized is kept as written.
fn resolved_root(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    let mut resolved = loop {
        if let Ok(canonical) = existing.canonicalize() {
            break canonical;
        }
        match (existing.parent(), existing.components().next_back()) {
            (Some(parent), Some(last)) => {
                missing.push(last);
                existing = parent;
            }
            _ => break existing.to_path_buf(),
        }
    };
    for component in missing.into_iter().rev() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other.as_os_str()),
        }
    }
    resolved
}

/// Refuses, with `pipeline_vector_index_root_shared`, a pipeline index root
/// equal to, or nested in either direction with, any of `legacy_roots`: two
/// usearch indexes on one root corrupt each other's `nearest`. The roots are
/// compared as the filesystem resolves them, so a symlink cannot hide an
/// overlap (PR #1295 review).
pub fn validate_pipeline_index_root(
    pipeline_root: &std::path::Path,
    legacy_roots: &[&std::path::Path],
) -> anyhow::Result<()> {
    let pipeline_root = resolved_root(pipeline_root);
    for legacy in legacy_roots {
        let legacy = resolved_root(legacy);
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
mod usearch_pipeline_index;

#[cfg(test)]
mod tests {
    use super::*;

    /// The overlap check compares the paths the filesystem resolves, not the
    /// text: a pipeline root reached through a symlink into the legacy root,
    /// or a legacy root reached through a symlink into the pipeline root,
    /// is refused. A symlink to a separate directory is not an overlap.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_overlap_with_a_legacy_root_is_refused() {
        let base = tempfile::tempdir().unwrap();
        let legacy = base.path().join("vector-index");
        let separate = base.path().join("separate");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&separate).unwrap();
        let into_legacy = base.path().join("into-legacy");
        std::os::unix::fs::symlink(&legacy, &into_legacy).unwrap();
        let into_separate = base.path().join("into-separate");
        std::os::unix::fs::symlink(&separate, &into_separate).unwrap();

        for shared in [
            into_legacy.clone(),
            into_legacy.join("pipeline"),
            into_legacy.join("pipeline/../pipeline"),
        ] {
            assert_eq!(
                validate_pipeline_index_root(&shared, &[&legacy])
                    .unwrap_err()
                    .to_string(),
                PIPELINE_VECTOR_INDEX_ROOT_SHARED_LABEL,
                "{}",
                shared.display()
            );
        }
        // The legacy root named through a symlink, the pipeline root plainly.
        assert!(validate_pipeline_index_root(&legacy.join("pipeline"), &[&into_legacy]).is_err());
        // A legacy root that is a symlink into the pipeline root.
        let pipeline = base.path().join("pipeline-index");
        std::fs::create_dir_all(pipeline.join("nested")).unwrap();
        let legacy_link = base.path().join("legacy-link");
        std::os::unix::fs::symlink(pipeline.join("nested"), &legacy_link).unwrap();
        assert!(validate_pipeline_index_root(&pipeline, &[&legacy_link]).is_err());

        validate_pipeline_index_root(&into_separate.join("pipeline"), &[&legacy]).unwrap();
        validate_pipeline_index_root(&separate, &[&legacy, &legacy.join("../dedup-index")])
            .unwrap();
    }
}
