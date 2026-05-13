//! Embedding trait + deterministic mock.

use sha2::{Digest, Sha256};

/// Dimensionality of the mock embedding. Fixed so the orchestrator and the
/// vector index agree on layout without needing to negotiate at runtime.
pub const MOCK_EMBEDDING_DIM: usize = 256;

/// Project a plaintext trace into an embedding vector. Real implementations
/// invoke a pinned embedder model inside the enclave; the mock here derives
/// a stable unit vector from a hash of the plaintext.
///
/// `embed` returns `anyhow::Result` so an inference failure refuses the gate
/// evaluation rather than silently returning a zero vector that the
/// orchestrator's `1 - max_similarity` novelty math would otherwise interpret
/// as "maximally novel". Callers MUST propagate the error.
pub trait Embedder: Send + Sync {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>>;
}

/// Deterministic mock embedder.
///
/// The output is a 256-dimensional L2-normalized `Vec<f32>` whose entries come
/// from SHAKE-style expansion of `sha256(plaintext)`. It is **not** a real
/// embedding — it just satisfies the contract of "stable test vectors".
#[derive(Debug, Default, Clone)]
pub struct MockEmbedder;

impl MockEmbedder {
    pub fn new() -> Self {
        Self
    }
}

impl Embedder for MockEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        // Build deterministic bytes by hashing (counter || plaintext) until we
        // have enough material for `MOCK_EMBEDDING_DIM * 4` bytes (each f32
        // is 4 bytes). Each f32 is then mapped from `u32` into `[-1, 1]`.
        let needed_bytes = MOCK_EMBEDDING_DIM * 4;
        let mut bytes = Vec::with_capacity(needed_bytes);
        let mut counter: u32 = 0;
        while bytes.len() < needed_bytes {
            let mut h = Sha256::new();
            h.update(b"trace_gate_enclave.mock_embedder.v1\n");
            h.update(counter.to_be_bytes());
            h.update(b"\n");
            h.update(plaintext);
            bytes.extend_from_slice(&h.finalize());
            counter += 1;
        }
        bytes.truncate(needed_bytes);

        let mut v = Vec::with_capacity(MOCK_EMBEDDING_DIM);
        for chunk in bytes.chunks_exact(4) {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(chunk);
            let raw = u32::from_be_bytes(buf);
            // Map u32 into [-1, 1]: ratio in [0, 1] then shift to [-1, 1].
            let ratio = raw as f64 / u32::MAX as f64;
            v.push(((ratio * 2.0) - 1.0) as f32);
        }

        // L2-normalize so cosine similarity reduces to a plain dot product.
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_embedder_is_deterministic() {
        let e = MockEmbedder::new();
        let a = e.embed(b"hello world").unwrap();
        let b = e.embed(b"hello world").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), MOCK_EMBEDDING_DIM);
    }

    #[test]
    fn mock_embedder_is_unit_norm() {
        let e = MockEmbedder::new();
        let v = e.embed(b"hello world").unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "expected unit norm, got {norm}");
    }

    #[test]
    fn mock_embedder_differs_per_input() {
        let e = MockEmbedder::new();
        let a = e.embed(b"hello world").unwrap();
        let b = e.embed(b"goodbye world").unwrap();
        assert_ne!(a, b);
    }
}
