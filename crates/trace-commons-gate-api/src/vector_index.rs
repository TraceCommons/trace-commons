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

/// Deterministic identity for one index entry that Settle writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct IndexEntryKey {
    pub tenant_storage_ref: String,
    pub index_id: String,
    pub revision_id: Uuid,
    pub projection_id: String,
    pub model_id: String,
    pub chunk: u32,
}

impl IndexEntryKey {
    /// Every length is a big-endian `u64`, as in the pipeline contracts.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        fn string(output: &mut Vec<u8>, value: &str) {
            output.extend_from_slice(&(value.len() as u64).to_be_bytes());
            output.extend_from_slice(value.as_bytes());
        }
        let mut bytes = b"trace-commons-index-entry-key\0".to_vec();
        string(&mut bytes, &self.tenant_storage_ref);
        string(&mut bytes, &self.index_id);
        bytes.extend_from_slice(self.revision_id.as_bytes());
        string(&mut bytes, &self.projection_id);
        string(&mut bytes, &self.model_id);
        bytes.extend_from_slice(&self.chunk.to_be_bytes());
        bytes
    }

    pub fn entry_id(&self) -> Uuid {
        Uuid::new_v5(&Uuid::NAMESPACE_URL, &self.canonical_bytes())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexSnapshot {
    pub snapshot_id: String,
    pub snapshot_hash: String,
    pub cardinality: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexUpsertResult {
    Inserted,
    Unchanged,
}

#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum IndexWriteError {
    #[error("index key already exists with different content")]
    ContentConflict,
    #[error("index write did not complete")]
    Uncertain,
    #[error("index write failed")]
    Failed,
}

/// Read-only index capability. Score receives this and never a writer.
/// Every call is keyed by the tenant's derived storage reference.
pub trait VectorIndexReader: Send + Sync {
    fn snapshot(&self, tenant_storage_ref: &str, index_id: &str) -> anyhow::Result<IndexSnapshot>;

    fn nearest(
        &self,
        tenant_storage_ref: &str,
        index_id: &str,
        embedding: &[f32],
        k: usize,
        exclude_revision: Option<Uuid>,
    ) -> anyhow::Result<Vec<NearestNeighbor>>;
}

/// Write-only index capability. The runner uses it to apply a sealed command.
pub trait VectorIndexWriter: Send + Sync {
    fn upsert(
        &self,
        key: &IndexEntryKey,
        embedding: &[f32],
        content_hash: &str,
    ) -> Result<IndexUpsertResult, IndexWriteError>;
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

#[cfg(test)]
mod pipeline_index_tests {
    use super::*;

    fn key() -> IndexEntryKey {
        IndexEntryKey {
            tenant_storage_ref: "tenant_sha256:00112233445566778899aabbccddeeff".to_string(),
            index_id: "pipeline-test-index-v1".to_string(),
            revision_id: Uuid::nil(),
            projection_id: "pipeline-test-projection-v1".to_string(),
            model_id: "reference-embedder-v1".to_string(),
            chunk: 0,
        }
    }

    /// Computed outside Rust: SHA-1 UUIDv5 over the URL namespace and the
    /// canonical bytes (domain, u64 big-endian lengths, u32 chunk).
    #[test]
    fn entry_id_is_pinned_for_a_fixed_key() {
        assert_eq!(key().canonical_bytes().len(), 198);
        assert_eq!(
            key().entry_id().to_string(),
            "edc91810-e9e3-5ffc-8d5d-c1b32b30ad30"
        );
    }

    #[test]
    fn each_index_key_field_changes_entry_identity() {
        let base = key().entry_id();
        let mut changes = Vec::new();
        let mut value = key();
        value.tenant_storage_ref = "tenant_sha256:ffeeddccbbaa99887766554433221100".to_string();
        changes.push(value);
        let mut value = key();
        value.index_id = "pipeline-test-index-v2".to_string();
        changes.push(value);
        let mut value = key();
        value.revision_id = Uuid::from_u128(1);
        changes.push(value);
        let mut value = key();
        value.projection_id = "pipeline-test-projection-v2".to_string();
        changes.push(value);
        let mut value = key();
        value.model_id = "reference-embedder-v2".to_string();
        changes.push(value);
        let mut value = key();
        value.chunk = 1;
        changes.push(value);
        assert!(changes.iter().all(|changed| changed.entry_id() != base));
    }
}
