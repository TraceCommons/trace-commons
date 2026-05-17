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

pub mod embedder;
pub mod embedder_fastembed;
pub mod orchestrator;
pub mod perplexity;
pub mod perplexity_local;
#[cfg(feature = "near-ai-scorer")]
pub mod perplexity_near_ai;
pub mod vector_index;
#[cfg(feature = "local-gpu-models")]
pub mod vector_index_usearch;

pub use embedder::{Embedder, MockEmbedder};
pub use orchestrator::{
    EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, OrchestrationDecision,
};
pub use perplexity::{
    MockPerplexityScorer, MockTokenRarityScorer, PerplexityResult, PerplexityScorer,
    TokenRarityResult, TokenRarityScorer,
};
#[cfg(feature = "near-ai-scorer")]
pub use perplexity_near_ai::{NearAiPerplexityScorer, NearAiScorerConfig};
pub use vector_index::{MockVectorIndex, NearestNeighbor, VectorIndex};
#[cfg(feature = "local-gpu-models")]
pub use vector_index_usearch::UsearchVectorIndex;
