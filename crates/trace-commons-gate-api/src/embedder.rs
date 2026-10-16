// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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

/// Shares one embedder between holders (the legacy gate and the versioned
/// pipeline hold the same embedder, so the model is loaded once).
impl<T: Embedder + ?Sized> Embedder for std::sync::Arc<T> {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        (**self).embed(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedEmbedder;

    impl Embedder for FixedEmbedder {
        fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
            Ok(vec![plaintext.len() as f32, 1.0])
        }
    }

    /// An `Arc` (sized or `dyn`) forwards `embed` to the embedder it holds.
    #[test]
    fn arc_forwarding_keeps_embed() {
        fn embed_with<E: Embedder + ?Sized>(embedder: &E) -> Vec<f32> {
            embedder.embed(b"abc").unwrap()
        }
        let sized = std::sync::Arc::new(FixedEmbedder);
        let shared: std::sync::Arc<dyn Embedder> = sized.clone();
        assert_eq!(embed_with(&sized), vec![3.0, 1.0]);
        assert_eq!(embed_with(&shared), vec![3.0, 1.0]);
    }
}
