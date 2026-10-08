// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The production pipeline assembly (spec
//! `docs/superpowers/specs/2026-10-08-pipeline-production-assembly-design.md`,
//! Slice A).

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
    pub(crate) fn new(inner: Arc<dyn PerplexityScorer>, descriptor: NearAiScorerDescriptor) -> Self {
        Self { inner, descriptor }
    }

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

#[cfg(test)]
#[path = "production_assembly_tests.rs"]
mod tests;
