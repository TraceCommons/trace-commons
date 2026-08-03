//! Embedding trait shared by every gate implementation.

/// Dimensionality of the mock and reference embeddings. Fixed so the
/// orchestrator and the vector index agree on layout without needing to
/// negotiate at runtime.
pub const MOCK_EMBEDDING_DIM: usize = 256;

/// Project a plaintext trace into an embedding vector. Real implementations
/// invoke a pinned embedder model inside the enclave.
///
/// `embed` returns `anyhow::Result` so an inference failure refuses the gate
/// evaluation rather than silently returning a zero vector that the
/// orchestrator's `1 - max_similarity` novelty math would otherwise interpret
/// as "maximally novel". Callers MUST propagate the error.
pub trait Embedder: Send + Sync {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>>;
}
