//! Enclave-side abstractions for the Trace Commons gate service.
//!
//! This crate is the structural seam between code that would run inside a
//! dstack-attested TEE and the rest of the server. Today every concrete
//! implementation here is a deterministic mock so the host can exercise the
//! end-to-end shape without touching real LLMs, real embedders, or real
//! hardware. When the enclave binary lands, real implementations slot in
//! behind these same traits with no change to the host side.
//!
//! The crate intentionally has no dependency on `tracedao-server`; the host
//! crate depends on this one and adapts the orchestrator's `OrchestrationDecision`
//! to its own audit-row shape.

pub mod embedder;
pub mod orchestrator;
pub mod perplexity;
pub mod vector_index;

pub use embedder::{Embedder, MockEmbedder};
pub use orchestrator::{
    EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, OrchestrationDecision,
};
pub use perplexity::{MockPerplexityScorer, PerplexityResult, PerplexityScorer};
pub use vector_index::{MockVectorIndex, NearestNeighbor, VectorIndex};
