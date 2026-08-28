// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

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
    ///
    /// `tenant_storage_ref` is required so per-tenant implementations (e.g.
    /// `UsearchVectorIndex`, which keeps one file per tenant) can route the
    /// deletion to the right shard without doing a global scan.
    fn delete(&self, tenant_storage_ref: &str, entry_id: Uuid) -> anyhow::Result<bool>;

    /// Persist every pending write to whatever durable medium the
    /// implementation owns.
    ///
    /// Purely in-memory implementations (the mocks, the reference index) have
    /// nothing to persist and keep the no-op default. Implementations that own
    /// a durable corpus (`UsearchVectorIndex` and its per-tenant files) MUST
    /// override this: the corpus is what "duplicate" means, so a process that
    /// exits without flushing silently redefines the novelty gate.
    fn flush(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
