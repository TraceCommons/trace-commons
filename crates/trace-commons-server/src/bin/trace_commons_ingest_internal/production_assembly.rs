// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The production pipeline assembly (spec
//! `docs/superpowers/specs/2026-10-08-pipeline-production-assembly-design.md`,
//! Slice A).
//!
//! The adapters, the assembler and the selection are compiled in every
//! build, so the default-features tests exercise them over qualified
//! doubles; only the builder that constructs the NEAR AI scorer, the
//! fastembed embedder and the usearch index from the environment needs
//! `near-ai-scorer`. A default build therefore never constructs most of
//! what is here, and says so instead of warning about it.
#![cfg_attr(not(feature = "near-ai-scorer"), allow(dead_code))]

use super::*;

use trace_commons_gate_api::{
    ChunkPerplexity, Embedder, IdentifiedEmbedder, IdentifiedPerplexityScorer, PerplexityResult,
    PerplexityScorer,
};

/// The identity the pipeline's NEAR AI scorer adapter reports (spec A-D4).
pub(crate) const NEAR_AI_PIPELINE_SCORER_IDENTITY: &str = "near_ai_perplexity_scorer";
/// The identity the pipeline's fastembed embedder adapter reports (spec A-D5).
pub(crate) const FASTEMBED_PIPELINE_EMBEDDER_IDENTITY: &str = "fastembed_text_embedder";
/// The identities of the usearch-backed pipeline index (spec A-D6).
pub(crate) const USEARCH_PIPELINE_INDEX_READER_IDENTITY: &str = "usearch_pipeline_index_reader";
pub(crate) const USEARCH_PIPELINE_INDEX_WRITER_IDENTITY: &str = "usearch_pipeline_index_writer";
/// The identity of the production Trace Credit settlement adapter (spec
/// A-D9).
pub(crate) const INTERNAL_TRACE_CREDIT_ADAPTER_IDENTITY: &str = "internal_trace_credit_ledger";
/// The projection and index ids the production compatibility bundle binds
/// (spec A-D10). Neither carries a development marker or `test`, which
/// `validate_production_package` refuses.
pub(crate) const PRODUCTION_COMPATIBILITY_PROJECTION_ID: &str = "compatibility_projection_v1";
pub(crate) const PRODUCTION_COMPATIBILITY_INDEX_ID: &str = "compatibility_index_v1";

const NEAR_AI_SCORER_DESCRIPTOR_SCHEMA: &str = "trace_commons.near_ai_scorer_descriptor.v1";
const FASTEMBED_EMBEDDER_DESCRIPTOR_SCHEMA: &str = "trace_commons.fastembed_embedder_descriptor.v1";

/// Reads one variable through `lookup`, treating an empty or all-blank value
/// as unset.
fn lookup_trimmed(lookup: &dyn Fn(&str) -> Option<String>, var: &str) -> Option<String> {
    lookup(var)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn std_env_lookup(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

fn canonical_bytes(value: &serde_json::Value) -> Vec<u8> {
    trace_commons_protocol::canonical_json::to_canonical_vec(value)
        .expect("a descriptor is integers and strings only")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// What the NEAR AI scorer's identity binds (spec A-D4): the hosted model
/// pin and the scorer's own configuration, never its endpoint, key or
/// timeout. A pure function of the environment: `from_env` makes no network
/// call and loads nothing, so the production package can be built offline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NearAiScorerDescriptor {
    pub(crate) model: String,
    pub(crate) tail_logprob_cutoff: f32,
    pub(crate) logprobs_top_k: u32,
}

impl NearAiScorerDescriptor {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(&std_env_lookup)
    }

    /// The same parse the legacy `enclave_near_ai` gate applies to the model
    /// and the tail cutoff; `logprobs_top_k` is the constant both use.
    pub(crate) fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let model = lookup_trimmed(lookup, TRACE_COMMONS_NEAR_AI_MODEL)
            .ok_or_else(|| anyhow::anyhow!("pipeline_scorer_model_missing"))?;
        let tail_logprob_cutoff =
            match lookup_trimmed(lookup, TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF) {
                Some(raw) => raw
                    .parse::<f32>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| anyhow::anyhow!("pipeline_scorer_tail_cutoff_invalid"))?,
                None => TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF,
            };
        Ok(Self {
            model,
            tail_logprob_cutoff,
            logprobs_top_k: TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K,
        })
    }

    pub(crate) fn tail_logprob_cutoff_micros(&self) -> i64 {
        (f64::from(self.tail_logprob_cutoff) * 1_000_000.0).round() as i64
    }

    /// Canonical JSON (sorted keys) of the descriptor: the bytes the bundle
    /// package names the scorer by.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        canonical_bytes(&serde_json::json!({
            "schema": NEAR_AI_SCORER_DESCRIPTOR_SCHEMA,
            "model": self.model,
            "tail_logprob_cutoff_micros": self.tail_logprob_cutoff_micros(),
            "logprobs_top_k": self.logprobs_top_k,
        }))
    }

    /// `sha256:<hex>` of [`Self::bytes`].
    pub(crate) fn hash(&self) -> String {
        format!("sha256:{}", sha256_hex(&self.bytes()))
    }

    /// The compatibility configuration's `scorer_model_id` (spec A-D10):
    /// `near_ai:` and the descriptor's SHA-256, so the stored configuration
    /// carries no model name in clear.
    pub(crate) fn compatibility_scorer_model_id(&self) -> String {
        format!("near_ai:{}", sha256_hex(&self.bytes()))
    }
}

/// What the fastembed embedder's identity binds (spec A-D5). `output_dim`
/// is `TRACE_COMMONS_VECTOR_INDEX_DIM`, which the gate builder requires to
/// equal the loaded model's output dimension, so the descriptor needs no
/// model load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FastEmbedDescriptor {
    pub(crate) model_id: String,
    pub(crate) output_dim: usize,
    pub(crate) max_tokens: usize,
    pub(crate) matryoshka_dim: Option<usize>,
}

impl FastEmbedDescriptor {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(&std_env_lookup)
    }

    pub(crate) fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let positive = |var: &str| -> anyhow::Result<Option<usize>> {
            lookup_trimmed(lookup, var)
                .map(|raw| {
                    raw.parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| anyhow::anyhow!("pipeline_embedder_descriptor_invalid"))
                })
                .transpose()
        };
        Ok(Self {
            model_id: lookup_trimmed(lookup, TRACE_COMMONS_EMBEDDER_MODEL_ID)
                .unwrap_or_else(|| TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID.to_string()),
            output_dim: positive(TRACE_COMMONS_VECTOR_INDEX_DIM)?
                .unwrap_or(TRACE_COMMONS_VECTOR_INDEX_DEFAULT_DIM),
            max_tokens: positive(TRACE_COMMONS_EMBEDDER_MAX_TOKENS)?
                .unwrap_or(TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS),
            matryoshka_dim: positive(TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM)?,
        })
    }

    pub(crate) fn bytes(&self) -> Vec<u8> {
        canonical_bytes(&serde_json::json!({
            "schema": FASTEMBED_EMBEDDER_DESCRIPTOR_SCHEMA,
            "model_id": self.model_id,
            "output_dim": self.output_dim,
            "max_tokens": self.max_tokens,
            "matryoshka_dim": self.matryoshka_dim,
        }))
    }

    pub(crate) fn hash(&self) -> String {
        format!("sha256:{}", sha256_hex(&self.bytes()))
    }
}

/// The NEAR AI scorer as the pipeline names it (spec A-D4). Holds the
/// scorer as a trait object -- the legacy gate holds the same one -- and
/// forwards both scoring methods, so the lossless `score_chunk` survives.
pub(crate) struct NearAiPipelineScorer {
    inner: Arc<dyn PerplexityScorer>,
    descriptor: NearAiScorerDescriptor,
}

impl NearAiPipelineScorer {
    pub(crate) fn new(
        inner: Arc<dyn PerplexityScorer>,
        descriptor: NearAiScorerDescriptor,
    ) -> Self {
        Self { inner, descriptor }
    }

    #[cfg(test)]
    pub(crate) fn descriptor(&self) -> &NearAiScorerDescriptor {
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
pub(crate) struct FastEmbedPipelineEmbedder {
    inner: Arc<dyn Embedder>,
    descriptor: FastEmbedDescriptor,
}

impl FastEmbedPipelineEmbedder {
    pub(crate) fn new(inner: Arc<dyn Embedder>, descriptor: FastEmbedDescriptor) -> Self {
        Self { inner, descriptor }
    }

    #[cfg(test)]
    pub(crate) fn descriptor(&self) -> &FastEmbedDescriptor {
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
        &self.descriptor.model_id
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
pub(crate) struct InternalTraceCreditSettlementAdapter {
    instrument_id: trace_commons_gate_api::pipeline::InstrumentId,
}

impl InternalTraceCreditSettlementAdapter {
    pub(crate) fn new() -> Self {
        Self {
            instrument_id: trace_commons_gate_api::pipeline::InstrumentId::trace_credit(),
        }
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
pub(crate) type DbTenantPolicyReads = Arc<dyn Fn(&str) -> bool + Send + Sync>;

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
pub(crate) struct TenantPolicyPipelineAuthorityProvider {
    policies: Arc<BTreeMap<String, TenantSubmissionPolicy>>,
    require_policy: bool,
    db_policy_reads: DbTenantPolicyReads,
}

impl TenantPolicyPipelineAuthorityProvider {
    pub(crate) fn new(
        policies: Arc<BTreeMap<String, TenantSubmissionPolicy>>,
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

impl trace_commons_server::versioned_pipeline_authority::PipelineAuthorityProvider
    for TenantPolicyPipelineAuthorityProvider
{
    fn authority_for_tenant(
        &self,
        tenant_id: &str,
    ) -> Option<trace_commons_server::trace_authority::SubmissionAuthority> {
        if (self.db_policy_reads)(tenant_id) {
            return None;
        }
        Some(trace_commons_server::trace_authority::SubmissionAuthority {
            tenant: trace_commons_server::trace_authority::SubmissionAllowlists::default(),
            policy: self.policies.get(tenant_id).map(|policy| {
                trace_commons_server::trace_authority::SubmissionAllowlists {
                    allowed_consent_scopes: policy.allowed_consent_scopes.clone(),
                    allowed_uses: policy.allowed_uses.clone(),
                }
            }),
            require_policy: self.require_policy,
        })
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

/// Selects the pipeline runtime ingest starts (spec A-D2).
pub(crate) const TRACE_COMMONS_PIPELINE_RUNTIME: &str = "TRACE_COMMONS_PIPELINE_RUNTIME";
pub(crate) const PIPELINE_RUNTIME_SELECTION_UNKNOWN_LABEL: &str =
    "pipeline_runtime_selection_unknown";
pub(crate) const PIPELINE_RUNTIME_PRODUCTION_REQUIRES_NEAR_AI_SCORER_LABEL: &str =
    "pipeline_runtime_production_requires_near_ai_scorer";
pub(crate) const PIPELINE_RUNTIME_PRODUCTION_REQUIRES_ENCLAVE_NEAR_AI_LABEL: &str =
    "pipeline_runtime_production_requires_enclave_near_ai";
pub(crate) const PIPELINE_PRODUCTION_COMPONENTS_MISSING_LABEL: &str =
    "pipeline_production_components_missing";

/// Which pipeline runtime this process starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipelineRuntimeSelection {
    /// No pipeline runtime: `run_ingest(None)`, exactly as before.
    None,
    /// [`ProductionPipelineAssembler`] over the shared NEAR AI gate
    /// components.
    Production,
}

impl PipelineRuntimeSelection {
    /// What `GET /v1/admin/config-status` reports.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Production => "production",
        }
    }

    /// The assembler `main` passes `run_ingest` for this selection.
    pub(crate) fn assembler(self) -> Option<&'static dyn IngestPipelineRuntimeAssembler> {
        match self {
            Self::None => None,
            Self::Production => Some(&ProductionPipelineAssembler),
        }
    }
}

/// Spec A-D2: unset (or blank) selects no runtime; `production` selects the
/// production assembly, and only on a `near-ai-scorer` build whose legacy
/// gate is `enclave_near_ai` (the components the pipeline reuses are that
/// gate's); any other value refuses the start.
pub(crate) fn pipeline_runtime_selection(
    raw: Option<&str>,
    gate_service: Option<&str>,
    near_ai_scorer_compiled: bool,
) -> anyhow::Result<PipelineRuntimeSelection> {
    match raw.map(str::trim).unwrap_or_default() {
        "" => Ok(PipelineRuntimeSelection::None),
        "production" => {
            anyhow::ensure!(
                near_ai_scorer_compiled,
                PIPELINE_RUNTIME_PRODUCTION_REQUIRES_NEAR_AI_SCORER_LABEL
            );
            anyhow::ensure!(
                gate_service.map(str::trim) == Some("enclave_near_ai"),
                PIPELINE_RUNTIME_PRODUCTION_REQUIRES_ENCLAVE_NEAR_AI_LABEL
            );
            Ok(PipelineRuntimeSelection::Production)
        }
        _ => anyhow::bail!(PIPELINE_RUNTIME_SELECTION_UNKNOWN_LABEL),
    }
}

pub(crate) fn pipeline_runtime_selection_from_env() -> anyhow::Result<PipelineRuntimeSelection> {
    pipeline_runtime_selection(
        std::env::var(TRACE_COMMONS_PIPELINE_RUNTIME)
            .ok()
            .as_deref(),
        std::env::var(TRACE_COMMONS_GATE_SERVICE).ok().as_deref(),
        cfg!(feature = "near-ai-scorer"),
    )
}

/// The gate components the pipeline shares with the legacy gate (spec
/// A-D3), built once at boot. Holds trait objects and descriptors only:
/// the `near-ai-scorer` environment builder fills it from the NEAR AI
/// scorer, the fastembed embedder and the usearch index, and a
/// default-features test fills it with qualified doubles.
pub struct PipelineGateComponents {
    pub(crate) scorer: Arc<dyn PerplexityScorer>,
    pub(crate) scorer_descriptor: NearAiScorerDescriptor,
    pub(crate) embedder: Arc<dyn Embedder>,
    pub(crate) embedder_descriptor: FastEmbedDescriptor,
    pub(crate) index_reader: Arc<dyn trace_commons_gate_api::IdentifiedIndexReader>,
    pub(crate) index_writer: Arc<dyn trace_commons_gate_api::IdentifiedIndexWriter>,
    /// Whether the pipeline index root is (or nests with) a legacy root.
    /// The builder refuses that, so a built set always holds `false`; the
    /// adapters check records it.
    pub(crate) index_root_shared_with_legacy: bool,
    pub(crate) authority:
        Arc<dyn trace_commons_server::versioned_pipeline_authority::PipelineAuthorityProvider>,
    pub(crate) tenant_policy_count: usize,
    /// `None` when no privacy filter backend is configured; the runtime is
    /// then not production-qualified.
    pub(crate) privacy: Option<
        Arc<dyn trace_commons_server::versioned_pipeline_authority::PipelinePrivacyBoundary>,
    >,
    pub(crate) privacy_backend:
        Option<trace_commons_protocol::trace_contribution::PrivacyFilterBackendTag>,
}

/// The production pipeline assembly (spec A-D10): the compatibility bundle
/// under `main`'s gate configuration, over the shared NEAR AI scorer,
/// fastembed embedder and usearch pipeline index, the tenant-policy
/// authority, the classifier privacy boundary, and the internal Trace Credit
/// adapter. Never enables payout (spec A-D9).
pub(crate) struct ProductionPipelineAssembler;

impl IngestPipelineRuntimeAssembler for ProductionPipelineAssembler {
    fn assemble(
        &self,
        context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        use trace_commons_server::versioned_pipeline::{PipelineCaps, PipelineServiceBuilder};
        use trace_commons_server::versioned_pipeline_bundle::MinimalPolicyBundle;
        use trace_commons_server::versioned_pipeline_compat::CompatibilityBundleConfig;
        use trace_commons_server::versioned_pipeline_credit::SettlementAdapterRegistry;

        let components = context
            .components
            .clone()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_PRODUCTION_COMPONENTS_MISSING_LABEL))?;
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
            &context.main_gate,
        )?;
        let package = MinimalPolicyBundle::compatibility_package(
            &config,
            scorer.as_ref(),
            embedder.as_ref(),
        )?;
        let registry =
            SettlementAdapterRegistry::new(vec![
                Arc::new(InternalTraceCreditSettlementAdapter::new())
                    as Arc<dyn trace_commons_gate_api::SettlementAdapter>,
            ])?;
        let mut builder = PipelineServiceBuilder::new(
            context.backend,
            context.artifact_store,
            package,
            components.index_reader.clone(),
            components.index_writer.clone(),
            registry,
            PipelineCaps {
                per_instrument_atomic_units: BTreeMap::new(),
            },
        )
        .with_scorer(scorer)
        .with_embedder(embedder)
        .with_authority(components.authority.clone())
        .with_object_store_name(context.object_store_name)
        .with_lease_config(context.lease_config)
        .with_novelty_utility_checks(context.novelty_utility_checks)
        .with_unqualified_routing(context.unqualified_routing_allowed);
        if let Some(privacy) = components.privacy.clone() {
            builder = builder.with_privacy(privacy);
        }
        Ok(Arc::new(builder.build()?))
    }

    fn needs_gate_components(&self) -> bool {
        true
    }
}

/// What a production boot hands the components builder besides the
/// environment: `main`'s tenant policies, its require-policy flag, and
/// which tenants it reads policies for from the database.
pub(crate) struct PipelineComponentInputs {
    pub(crate) tenant_policies: Arc<BTreeMap<String, TenantSubmissionPolicy>>,
    pub(crate) require_tenant_submission_policy: bool,
    pub(crate) db_policy_reads: DbTenantPolicyReads,
}

/// The values the legacy gate was built with, which the pipeline's
/// descriptors must name: the two parse the same variables, and this
/// refuses a start where they would not agree, so the package can never
/// name a scorer or embedder other than the one that runs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LegacyGatePins {
    pub(crate) model: String,
    pub(crate) tail_logprob_cutoff: f32,
    pub(crate) embedder_model_id: String,
    pub(crate) embedder_output_dim: usize,
    pub(crate) embedder_max_tokens: usize,
    pub(crate) embedder_matryoshka_dim: Option<usize>,
}

pub(crate) fn ensure_descriptors_match_gate(
    scorer: &NearAiScorerDescriptor,
    embedder: &FastEmbedDescriptor,
    gate: &LegacyGatePins,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        scorer.model == gate.model
            && scorer.tail_logprob_cutoff == gate.tail_logprob_cutoff
            && scorer.logprobs_top_k == TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K,
        "pipeline_scorer_descriptor_mismatch"
    );
    anyhow::ensure!(
        embedder.model_id == gate.embedder_model_id
            && embedder.output_dim == gate.embedder_output_dim
            && embedder.max_tokens == gate.embedder_max_tokens
            && embedder.matryoshka_dim == gate.embedder_matryoshka_dim,
        "pipeline_embedder_descriptor_mismatch"
    );
    Ok(())
}

/// The novelty and dedup roots the legacy gate opens, resolved exactly as
/// `build_dedup_vector_index_from_env` resolves the dedup default.
fn legacy_index_roots(lookup: &dyn Fn(&str) -> Option<String>) -> (PathBuf, PathBuf) {
    let novelty = lookup(TRACE_COMMONS_VECTOR_INDEX_ROOT)
        .unwrap_or_else(|| TRACE_COMMONS_VECTOR_INDEX_DEFAULT_ROOT.to_string());
    let dedup = lookup(TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT)
        .unwrap_or_else(|| format!("{novelty}/../dedup-index"));
    (PathBuf::from(novelty), PathBuf::from(dedup))
}

/// The pipeline index root: required, and never a legacy root.
pub(crate) fn pipeline_index_root_from(
    lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<PathBuf> {
    let root = lookup_trimmed(lookup, TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT)
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!(PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL))?;
    let (novelty, dedup) = legacy_index_roots(lookup);
    validate_pipeline_index_root(&root, &[&novelty, &dedup])?;
    Ok(root)
}

/// The pipeline's privacy boundary (spec A-D7): `main`'s classifier adapter
/// and policy, or `None` when no backend is configured.
fn pipeline_privacy_from_env() -> anyhow::Result<(
    Option<Arc<dyn trace_commons_server::versioned_pipeline_authority::PipelinePrivacyBoundary>>,
    Option<trace_commons_protocol::trace_contribution::PrivacyFilterBackendTag>,
)> {
    let Some((adapter, backend)) =
        trace_commons_protocol::trace_contribution::privacy_filter_adapter_from_env()
            .map_err(|_| anyhow::anyhow!("pipeline_privacy_filter_unavailable"))?
    else {
        return Ok((None, None));
    };
    let boundary =
        trace_commons_server::versioned_pipeline_authority::ClassifierRedactorPipelinePrivacyBoundary::new(
            adapter,
            backend,
            PiiClassifyPolicy::from_env(),
        );
    Ok((Some(Arc::new(boundary)), Some(backend)))
}

/// Builds the legacy `enclave_near_ai` gate and, from the same scorer,
/// embedder and settings, the components the production pipeline holds
/// (spec A-D3, A-D6, A-D7, A-D8). Only this builder names the concrete NEAR
/// AI, fastembed and usearch types.
#[cfg(feature = "near-ai-scorer")]
pub(crate) async fn build_near_ai_gate_service_with_pipeline_components(
    inputs: PipelineComponentInputs,
) -> anyhow::Result<(Arc<dyn TraceGateService>, Arc<PipelineGateComponents>)> {
    let scorer_descriptor = NearAiScorerDescriptor::from_env()?;
    let embedder_descriptor = FastEmbedDescriptor::from_env()?;
    let pipeline_root = pipeline_index_root_from(&std_env_lookup)?;
    let parts = near_ai_gate_parts_from_env().await?;
    ensure_descriptors_match_gate(
        &scorer_descriptor,
        &embedder_descriptor,
        &LegacyGatePins {
            model: parts.model.clone(),
            tail_logprob_cutoff: parts.tail_logprob_cutoff,
            embedder_model_id: parts.embedder_model_id.clone(),
            embedder_output_dim: parts.embedder_output_dim,
            embedder_max_tokens: parts.embedder_max_tokens,
            embedder_matryoshka_dim: parts.embedder_matryoshka_dim,
        },
    )?;
    // Every pipeline write flushes, so no periodic flusher thread is needed.
    let mut index_config = parts.vector_index_config.clone();
    index_config.flush_interval = None;
    let index = Arc::new(UsearchPipelineIndex::open(&pipeline_root, index_config)?);
    let (privacy, privacy_backend) = pipeline_privacy_from_env()?;
    let tenant_policy_count = inputs.tenant_policies.len();
    let components = Arc::new(PipelineGateComponents {
        scorer: parts.scorer.clone(),
        scorer_descriptor,
        embedder: parts.embedder.clone(),
        embedder_descriptor,
        index_reader: index.clone(),
        index_writer: index,
        index_root_shared_with_legacy: false,
        authority: Arc::new(TenantPolicyPipelineAuthorityProvider::new(
            inputs.tenant_policies,
            inputs.require_tenant_submission_policy,
            inputs.db_policy_reads,
        )),
        tenant_policy_count,
        privacy,
        privacy_backend,
    });
    Ok((near_ai_gate_service_from_parts(parts), components))
}

#[cfg(not(feature = "near-ai-scorer"))]
pub(crate) async fn build_near_ai_gate_service_with_pipeline_components(
    _inputs: PipelineComponentInputs,
) -> anyhow::Result<(Arc<dyn TraceGateService>, Arc<PipelineGateComponents>)> {
    anyhow::bail!(PIPELINE_RUNTIME_PRODUCTION_REQUIRES_NEAR_AI_SCORER_LABEL)
}

/// Where the pipeline's own vector index lives (spec A-D6). Required under
/// the production selection, and never the legacy novelty or dedup root.
pub(crate) const TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT: &str =
    "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT";
pub(crate) const PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL: &str =
    "pipeline_vector_index_root_missing";
pub(crate) const PIPELINE_VECTOR_INDEX_ROOT_SHARED_LABEL: &str =
    "pipeline_vector_index_root_shared";
pub(crate) const PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL: &str =
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
pub(crate) fn validate_pipeline_index_root(
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
pub(crate) use usearch_pipeline_index::UsearchPipelineIndex;

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
    pub(crate) struct UsearchPipelineIndex {
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
        pub(crate) fn open(root: &Path, config: UsearchVectorIndexConfig) -> anyhow::Result<Self> {
            let usearch = UsearchVectorIndex::try_new(root, config)
                .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
            let index: Arc<dyn VectorIndex> = Arc::new(usearch);
            let mut namespaces = BTreeMap::new();
            let listing = std::fs::read_dir(root)
                .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
            for entry in listing {
                let path = entry
                    .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?
                    .path();
                if !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(MANIFEST_SUFFIX))
                {
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
}

#[cfg(test)]
#[path = "production_assembly_tests.rs"]
mod tests;
