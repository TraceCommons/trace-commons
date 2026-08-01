//! Enclave-side abstractions for the Trace Commons gate service.
//!
//! This crate is the structural seam between code that would run inside a
//! dstack-attested TEE and the rest of the server. Today every concrete
//! implementation here is a deterministic mock so the host can exercise the
//! end-to-end shape without touching real LLMs, real embedders, or real
//! hardware. When the enclave binary lands, real implementations slot in
//! behind these same traits with no change to the host side.
//!
//! The crate intentionally has no dependency on `trace-commons-server`; the host
//! crate depends on this one and adapts the orchestrator's `OrchestrationDecision`
//! to its own audit-row shape.

pub mod chunk_aggregate;
pub mod chunker;
pub mod embedder;
pub mod embedder_fastembed;
pub mod orchestrator;
pub mod perplexity;
pub mod perplexity_local;
#[cfg(feature = "near-ai-scorer")]
pub mod perplexity_near_ai;
pub mod vector_index;
#[cfg(any(feature = "local-gpu-models", feature = "near-ai-scorer"))]
pub mod vector_index_usearch;

// Contracts now live in `trace-commons-gate-api`; re-exported here so existing
// `trace_commons_gate_enclave::{Embedder, PerplexityScorer, ...}` paths keep
// resolving. Implementations below remain local to this crate.
pub use trace_commons_gate_api::{
    ChunkPerplexity, Embedder, NearestNeighbor, PerplexityResult, PerplexityScorer,
    TokenRarityResult, TokenRarityScorer, VectorIndex, MOCK_EMBEDDING_DIM,
};

pub use embedder::MockEmbedder;
pub use orchestrator::{
    EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, OrchestrationDecision,
    PerplexityOnlyOutcome,
};
pub use perplexity::{MockPerplexityScorer, MockTokenRarityScorer};
#[cfg(feature = "near-ai-scorer")]
pub use perplexity_near_ai::{NearAiPerplexityScorer, NearAiScorerConfig};
pub use vector_index::MockVectorIndex;
#[cfg(any(feature = "local-gpu-models", feature = "near-ai-scorer"))]
pub use vector_index_usearch::UsearchVectorIndex;
