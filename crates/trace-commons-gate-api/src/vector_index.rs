// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::pipeline::TenantStorageRef;
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
    pub tenant_storage_ref: TenantStorageRef,
    pub index_id: String,
    pub revision_id: Uuid,
    pub projection_id: String,
    pub model_id: String,
    pub chunk: u32,
}

impl IndexEntryKey {
    /// Encodes each string with the pipeline's length-prefix framing: a
    /// big-endian `u64` length followed by the bytes.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        use crate::pipeline::encode_string;
        let mut bytes = b"trace-commons-index-entry-key\0".to_vec();
        encode_string(&mut bytes, self.tenant_storage_ref.as_str());
        encode_string(&mut bytes, &self.index_id);
        bytes.extend_from_slice(self.revision_id.as_bytes());
        encode_string(&mut bytes, &self.projection_id);
        encode_string(&mut bytes, &self.model_id);
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
    fn snapshot(
        &self,
        tenant_storage_ref: &TenantStorageRef,
        index_id: &str,
    ) -> anyhow::Result<IndexSnapshot>;

    fn nearest(
        &self,
        tenant_storage_ref: &TenantStorageRef,
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

    /// Removes every entry of `revision_id` in `index_id` under
    /// `tenant_storage_ref`, and nothing else.
    ///
    /// Returns `Ok(true)` when it removed entries and `Ok(false)` when there
    /// were none. It is idempotent: a repeated call returns `Ok(false)` and
    /// changes nothing. A failure returns an `IndexWriteError` and never
    /// `Ok(false)`, so a failed invalidation stays visible. It never returns
    /// `ContentConflict`. `Uncertain` means some entries may have been
    /// removed; `Failed` means none were. Either way the invalidation is not
    /// complete, and a retry is safe because the method is idempotent.
    ///
    /// Used after a withdrawal of a submission whose revision is already in
    /// the index (the index invalidation worker). LIF-004.
    fn invalidate_revision(
        &self,
        tenant_storage_ref: &TenantStorageRef,
        index_id: &str,
        revision_id: Uuid,
    ) -> Result<bool, IndexWriteError>;
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

/// Shares one index between holders. Forwards all five methods, the two
/// defaulted ones (`snapshot`, `flush`) included: without the overrides an
/// `Arc` would report no shard and never persist its corpus.
impl<T: VectorIndex + ?Sized> VectorIndex for std::sync::Arc<T> {
    fn snapshot(&self, tenant_storage_ref: &str) -> Option<VectorIndexSnapshot> {
        (**self).snapshot(tenant_storage_ref)
    }

    fn insert(
        &self,
        entry_id: Uuid,
        tenant_storage_ref: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        (**self).insert(entry_id, tenant_storage_ref, embedding)
    }

    fn nearest(
        &self,
        tenant_storage_ref: &str,
        embedding: &[f32],
        k: usize,
    ) -> anyhow::Result<Vec<NearestNeighbor>> {
        (**self).nearest(tenant_storage_ref, embedding, k)
    }

    fn delete(&self, tenant_storage_ref: &str, entry_id: Uuid) -> anyhow::Result<bool> {
        (**self).delete(tenant_storage_ref, entry_id)
    }

    fn flush(&self) -> anyhow::Result<()> {
        (**self).flush()
    }
}

#[cfg(test)]
mod pipeline_index_tests {
    use super::*;

    fn key() -> IndexEntryKey {
        IndexEntryKey {
            tenant_storage_ref: TenantStorageRef::new(
                "tenant_sha256:00112233445566778899aabbccddeeff",
            )
            .unwrap(),
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
        value.tenant_storage_ref =
            TenantStorageRef::new("tenant_sha256:ffeeddccbbaa99887766554433221100").unwrap();
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

    /// Records which methods were called, and answers every defaulted method
    /// with a value the default could not produce.
    #[derive(Default)]
    struct CountingIndex {
        flushed: std::sync::atomic::AtomicUsize,
        inserted: std::sync::atomic::AtomicUsize,
        deleted: std::sync::atomic::AtomicUsize,
    }

    impl VectorIndex for CountingIndex {
        fn snapshot(&self, _tenant_storage_ref: &str) -> Option<VectorIndexSnapshot> {
            Some(VectorIndexSnapshot {
                snapshot_id: Uuid::from_u128(7),
                cardinality: 3,
            })
        }

        fn insert(
            &self,
            _entry_id: Uuid,
            _tenant_storage_ref: &str,
            _embedding: &[f32],
        ) -> anyhow::Result<()> {
            self.inserted
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn nearest(
            &self,
            _tenant_storage_ref: &str,
            _embedding: &[f32],
            k: usize,
        ) -> anyhow::Result<Vec<NearestNeighbor>> {
            Ok(vec![
                NearestNeighbor {
                    entry_id: Uuid::from_u128(9),
                    similarity: 0.5,
                };
                k
            ])
        }

        fn delete(&self, _tenant_storage_ref: &str, _entry_id: Uuid) -> anyhow::Result<bool> {
            self.deleted
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(true)
        }

        fn flush(&self) -> anyhow::Result<()> {
            self.flushed
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
    }

    /// An `Arc` (sized or `dyn`) forwards all five methods, including the
    /// two the trait defaults (`snapshot`, `flush`): a shared index must
    /// neither stop describing its shard nor stop persisting its corpus.
    #[test]
    fn arc_forwarding_keeps_index_snapshot_and_flush() {
        fn exercise<V: VectorIndex + ?Sized>(index: &V) -> Option<VectorIndexSnapshot> {
            index.insert(Uuid::nil(), "t", &[1.0]).unwrap();
            assert_eq!(index.nearest("t", &[1.0], 2).unwrap().len(), 2);
            assert!(index.delete("t", Uuid::nil()).unwrap());
            index.flush().unwrap();
            index.snapshot("t")
        }
        let sized = std::sync::Arc::new(CountingIndex::default());
        let shared: std::sync::Arc<dyn VectorIndex> = sized.clone();
        let expected = Some(VectorIndexSnapshot {
            snapshot_id: Uuid::from_u128(7),
            cardinality: 3,
        });
        assert_eq!(exercise(&sized), expected);
        assert_eq!(exercise(&shared), expected);
        let load = |counter: &std::sync::atomic::AtomicUsize| {
            counter.load(std::sync::atomic::Ordering::SeqCst)
        };
        assert_eq!(load(&sized.flushed), 2, "flush is forwarded");
        assert_eq!(load(&sized.inserted), 2);
        assert_eq!(load(&sized.deleted), 2);
    }
}
