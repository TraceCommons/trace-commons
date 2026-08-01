//! Public contracts for the Trace Commons gate.
//!
//! This crate is the stable seam between the open protocol server and any
//! scoring backend. It holds traits and data types only — no scoring logic
//! beyond the deliberately-simple [`mod@reference`] implementations. Proprietary
//! backends live outside this repository and depend on this crate.

pub mod decision;
pub mod embedder;
pub mod perplexity;
pub mod reference;
pub mod vector_index;

pub use decision::{
    EnclaveGateOrchestratorConfig, InsertedChunkEntry, OrchestrationDecision, PerplexityOnlyOutcome,
};
pub use embedder::{Embedder, MOCK_EMBEDDING_DIM};
pub use perplexity::{
    ChunkPerplexity, PerplexityResult, PerplexityScorer, TokenRarityResult, TokenRarityScorer,
};
pub use reference::{ReferenceEmbedder, ReferencePerplexityScorer};
pub use vector_index::{NearestNeighbor, VectorIndex};
