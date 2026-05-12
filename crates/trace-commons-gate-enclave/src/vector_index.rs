//! Vector-index trait + in-memory mock.
//!
//! The host stores vector entries by `(tenant_storage_ref, entry_id)`. The
//! production index will be a real ANN index running inside the enclave; the
//! mock here is a `BTreeMap` keyed by tenant + entry_id with a brute-force
//! cosine-similarity scan for nearest-neighbor lookups.

use std::collections::BTreeMap;
use std::sync::Mutex;

use uuid::Uuid;

/// A nearest-neighbor result. `entry_id` is the `(tenant, entry_id)` UUID;
/// `similarity` is cosine similarity in `[-1.0, 1.0]`.
#[derive(Debug, Clone, PartialEq)]
pub struct NearestNeighbor {
    pub entry_id: Uuid,
    pub similarity: f32,
}

/// Pluggable vector index used by the gate orchestrator.
pub trait VectorIndex: Send + Sync {
    /// Insert (or upsert) a vector for `entry_id` under `tenant_storage_ref`.
    fn insert(
        &self,
        entry_id: Uuid,
        tenant_storage_ref: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()>;

    /// Return up to `k` nearest neighbors for `embedding` within
    /// `tenant_storage_ref`. Results are sorted by descending similarity.
    fn nearest(
        &self,
        tenant_storage_ref: &str,
        embedding: &[f32],
        k: usize,
    ) -> anyhow::Result<Vec<NearestNeighbor>>;

    /// Remove an entry from the index. Returns `Ok(true)` if removed,
    /// `Ok(false)` if no such entry existed.
    fn delete(&self, entry_id: Uuid) -> anyhow::Result<bool>;
}

/// In-memory `MockVectorIndex` for tests and local development.
///
/// Stores normalized vectors per tenant in a `BTreeMap`. Cosine similarity
/// reduces to a plain dot product because callers (the orchestrator + mock
/// embedder) feed unit-norm vectors.
#[derive(Debug, Default)]
pub struct MockVectorIndex {
    // Outer map: tenant_storage_ref → (entry_id → embedding)
    entries: Mutex<BTreeMap<String, BTreeMap<Uuid, Vec<f32>>>>,
}

impl MockVectorIndex {
    pub fn new() -> Self {
        Self::default()
    }
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

impl VectorIndex for MockVectorIndex {
    fn insert(
        &self,
        entry_id: Uuid,
        tenant_storage_ref: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        let mut g = self.entries.lock().expect("MockVectorIndex mutex poisoned");
        g.entry(tenant_storage_ref.to_string())
            .or_default()
            .insert(entry_id, embedding.to_vec());
        Ok(())
    }

    fn nearest(
        &self,
        tenant_storage_ref: &str,
        embedding: &[f32],
        k: usize,
    ) -> anyhow::Result<Vec<NearestNeighbor>> {
        let g = self.entries.lock().expect("MockVectorIndex mutex poisoned");
        let mut out: Vec<NearestNeighbor> = match g.get(tenant_storage_ref) {
            Some(m) => m
                .iter()
                .filter(|(_, v)| v.len() == embedding.len())
                .map(|(id, v)| NearestNeighbor {
                    entry_id: *id,
                    similarity: dot(embedding, v),
                })
                .collect(),
            None => Vec::new(),
        };
        out.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.truncate(k);
        Ok(out)
    }

    fn delete(&self, entry_id: Uuid) -> anyhow::Result<bool> {
        let mut g = self.entries.lock().expect("MockVectorIndex mutex poisoned");
        let mut removed = false;
        for (_, inner) in g.iter_mut() {
            if inner.remove(&entry_id).is_some() {
                removed = true;
            }
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_then_nearest_returns_self_as_top_hit() {
        let idx = MockVectorIndex::new();
        let id = Uuid::new_v4();
        let v: Vec<f32> = (0..4).map(|i| i as f32 * 0.25).collect();
        idx.insert(id, "tenant", &v).unwrap();
        let neighbors = idx.nearest("tenant", &v, 5).unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].entry_id, id);
    }

    #[test]
    fn nearest_respects_tenant_scope() {
        let idx = MockVectorIndex::new();
        let id_a = Uuid::new_v4();
        let v_a: Vec<f32> = vec![1.0, 0.0, 0.0];
        let id_b = Uuid::new_v4();
        let v_b: Vec<f32> = vec![1.0, 0.0, 0.0];
        idx.insert(id_a, "tenant_a", &v_a).unwrap();
        idx.insert(id_b, "tenant_b", &v_b).unwrap();
        let neighbors = idx.nearest("tenant_a", &v_a, 5).unwrap();
        assert_eq!(neighbors.len(), 1);
        assert_eq!(neighbors[0].entry_id, id_a);
    }

    #[test]
    fn delete_removes_entry() {
        let idx = MockVectorIndex::new();
        let id = Uuid::new_v4();
        idx.insert(id, "tenant", &vec![1.0, 0.0]).unwrap();
        assert!(idx.delete(id).unwrap());
        assert!(!idx.delete(id).unwrap());
    }
}
