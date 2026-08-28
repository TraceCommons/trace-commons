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

/// Instrumentation-only description of one tenant's index shard at the moment
/// a novelty score was computed against it (#199).
///
/// Novelty is `1 - max cosine similarity` against whatever the shard held at
/// scoring time, so the score is not reproducible — and not comparable across
/// time — without this. Recomputing it later scores against a fuller shard and
/// produces a number production never used, which is why it is recorded when
/// the decision is made rather than derived afterwards.
///
/// Hash-only/label-only safe by construction: an opaque generation UUID and a
/// count. No entry ids, no embeddings, no tenant identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VectorIndexSnapshot {
    /// Identifies the SHARD, not the write: two decisions carrying the same id
    /// were scored against the same corpus lineage — the same tenant's index
    /// under the same index root — and `cardinality` distinguishes states
    /// within it. A different id is a different corpus, which is the
    /// discontinuity an analyst must not average across. It is deliberately
    /// not a content hash: computing one would mean reading every vector on
    /// every decision.
    pub snapshot_id: Uuid,
    /// How many entries the shard held. Monotone within a generation (novelty
    /// drifts downward as it fills), so this is the covariate a chronological
    /// estimate conditions on. `0` is a real observation — the first trace of
    /// a tenant scores against an empty shard.
    pub cardinality: u64,
}

/// Pluggable vector index used by the gate orchestrator.
pub trait VectorIndex: Send + Sync {
    /// Describe the shard for `tenant_storage_ref` as it stands right now,
    /// for the instrumentation on the gate decision row.
    ///
    /// `None` means "this index cannot describe its own state", which is
    /// recorded as not-instrumented rather than as a zero-cardinality shard.
    /// The default is `None` so a substituted backend that has no shard
    /// generation to report says so instead of fabricating one; every index
    /// that gates real traffic should override it.
    ///
    /// Implementations MUST NOT mutate index contents here, and MUST be cheap:
    /// the orchestrator calls this on the scoring path of every trace.
    fn snapshot(&self, tenant_storage_ref: &str) -> Option<VectorIndexSnapshot> {
        let _ = tenant_storage_ref;
        None
    }

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
