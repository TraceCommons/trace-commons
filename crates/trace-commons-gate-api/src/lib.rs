//! Public contracts for the Trace Commons gate.
//!
//! This crate is the stable seam between the open protocol server and any
//! scoring backend. It holds traits and data types only — no scoring logic
//! beyond the deliberately-simple reference implementations. Proprietary
//! backends live outside this repository and depend on this crate.

pub mod embedder;
pub mod perplexity;
pub mod vector_index;

pub use embedder::{Embedder, MOCK_EMBEDDING_DIM};
pub use perplexity::{
    ChunkPerplexity, PerplexityResult, PerplexityScorer, TokenRarityResult, TokenRarityScorer,
};
pub use vector_index::{NearestNeighbor, VectorIndex};
