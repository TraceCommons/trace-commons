use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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
}

/// Deterministic index entry identity for Settle writes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct IndexEntryKey {
    pub tenant_id: String,
    pub index_id: String,
    pub revision_id: Uuid,
    pub projection_id: String,
    pub model_id: String,
    pub chunk: u32,
}

impl IndexEntryKey {
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = b"trace-commons-index-entry-key\0".to_vec();
        encode_len_prefixed(&mut bytes, self.tenant_id.as_bytes());
        encode_len_prefixed(&mut bytes, self.index_id.as_bytes());
        bytes.extend_from_slice(self.revision_id.as_bytes());
        encode_len_prefixed(&mut bytes, self.projection_id.as_bytes());
        encode_len_prefixed(&mut bytes, self.model_id.as_bytes());
        bytes.extend_from_slice(&self.chunk.to_be_bytes());
        bytes
    }

    pub fn entry_id(&self) -> Uuid {
        Uuid::new_v5(&Uuid::NAMESPACE_URL, &self.canonical_bytes())
    }

    pub fn content_digest(embedding: &[f32], content_hash: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content_hash.as_bytes());
        hasher.update(b"\0");
        for value in embedding {
            hasher.update(value.to_le_bytes());
        }
        format!("sha256:{:x}", hasher.finalize())
    }
}

fn encode_len_prefixed(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&(value.len() as u32).to_be_bytes());
    output.extend_from_slice(value);
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

/// Read-only index capability for Score and query tests.
pub trait VectorIndexReader: Send + Sync {
    fn snapshot(&self, tenant_id: &str, index_id: &str) -> anyhow::Result<IndexSnapshot>;

    fn nearest(
        &self,
        tenant_id: &str,
        index_id: &str,
        embedding: &[f32],
        k: usize,
        exclude_revision: Option<Uuid>,
    ) -> anyhow::Result<Vec<NearestNeighbor>>;
}

/// Write-only index capability for Settle.
pub trait VectorIndexWriter: Send + Sync {
    fn upsert(
        &self,
        key: &IndexEntryKey,
        embedding: &[f32],
        content_hash: &str,
    ) -> Result<IndexUpsertResult, IndexWriteError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> IndexEntryKey {
        IndexEntryKey {
            tenant_id: "tenant-a".to_string(),
            index_id: "index-v1".to_string(),
            revision_id: Uuid::nil(),
            projection_id: "projection-v1".to_string(),
            model_id: "model-v1".to_string(),
            chunk: 0,
        }
    }

    #[test]
    fn each_index_key_field_changes_entry_identity() {
        let base = key().entry_id();
        let mut changed = key();
        changed.tenant_id = "tenant-b".to_string();
        assert_ne!(changed.entry_id(), base);
        let mut changed = key();
        changed.index_id = "index-v2".to_string();
        assert_ne!(changed.entry_id(), base);
        let mut changed = key();
        changed.revision_id = Uuid::from_u128(1);
        assert_ne!(changed.entry_id(), base);
        let mut changed = key();
        changed.projection_id = "projection-v2".to_string();
        assert_ne!(changed.entry_id(), base);
        let mut changed = key();
        changed.model_id = "model-v2".to_string();
        assert_ne!(changed.entry_id(), base);
        let mut changed = key();
        changed.chunk = 1;
        assert_ne!(changed.entry_id(), base);
    }
}
