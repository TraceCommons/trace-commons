//! Isolated vector reader and writer used by the versioned pipeline.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_gate_api::embedder::Embedder;
use trace_commons_gate_api::{
    IndexEntryKey, IndexSnapshot, IndexUpsertResult, IndexWriteError, NearestNeighbor,
    VectorIndexReader, VectorIndexWriter,
};
use uuid::Uuid;

pub const PIPELINE_INDEX_ID: &str = "pipeline-test-index-v1";
pub const PIPELINE_PROJECTION_ID: &str = "pipeline-test-projection-v1";
pub const PIPELINE_EMBEDDER_MODEL_ID: &str = "pipeline-test-embedder-v1";
pub const PIPELINE_INDEX_COMMAND_SCHEMA: &str = "trace_commons.pipeline_index_command.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexFault {
    None,
    FailBeforeApply,
    LostAfterApply,
    FailInvalidation,
}

#[derive(Debug, Clone)]
struct StoredEntry {
    revision_id: Uuid,
    content_hash: String,
    embedding: Vec<f32>,
}

#[derive(Debug)]
struct IsolatedIndexState {
    entries: BTreeMap<(String, String, Uuid), StoredEntry>,
    fault: IndexFault,
}

#[derive(Debug)]
pub struct IsolatedPipelineIndex {
    state: Mutex<IsolatedIndexState>,
    writer_calls: AtomicUsize,
}

impl IsolatedPipelineIndex {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(IsolatedIndexState {
                entries: BTreeMap::new(),
                fault: IndexFault::None,
            }),
            writer_calls: AtomicUsize::new(0),
        })
    }

    pub fn writer_calls(&self) -> usize {
        self.writer_calls.load(Ordering::SeqCst)
    }

    pub fn set_fault(&self, fault: IndexFault) {
        self.state.lock().expect("index mutex").fault = fault;
    }

    pub fn entry_count(&self, tenant_id: &str, index_id: &str) -> usize {
        self.state
            .lock()
            .expect("index mutex")
            .entries
            .keys()
            .filter(|(stored_tenant, stored_index, _)| {
                stored_tenant == tenant_id && stored_index == index_id
            })
            .count()
    }

    pub fn contains_revision(&self, tenant_id: &str, index_id: &str, revision_id: Uuid) -> bool {
        self.state.lock().expect("index mutex").entries.iter().any(
            |((stored_tenant, stored_index, _), entry)| {
                stored_tenant == tenant_id
                    && stored_index == index_id
                    && entry.revision_id == revision_id
            },
        )
    }

    pub fn invalidate_revision(&self, tenant_id: &str, index_id: &str, revision_id: Uuid) -> bool {
        self.try_invalidate_revision(tenant_id, index_id, revision_id)
            .unwrap_or(false)
    }

    pub fn try_invalidate_revision(
        &self,
        tenant_id: &str,
        index_id: &str,
        revision_id: Uuid,
    ) -> Result<bool, IndexWriteError> {
        let mut state = self.state.lock().expect("index mutex");
        if state.fault == IndexFault::FailInvalidation {
            state.fault = IndexFault::None;
            return Err(IndexWriteError::Failed);
        }
        let before = state.entries.len();
        state
            .entries
            .retain(|(stored_tenant, stored_index, _), entry| {
                stored_tenant != tenant_id
                    || stored_index != index_id
                    || entry.revision_id != revision_id
            });
        Ok(state.entries.len() != before)
    }
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
}

impl VectorIndexReader for IsolatedPipelineIndex {
    fn snapshot(&self, tenant_id: &str, index_id: &str) -> anyhow::Result<IndexSnapshot> {
        let state = self.state.lock().expect("index mutex");
        let mut hasher = Sha256::new();
        let mut cardinality = 0_u64;
        for ((stored_tenant, stored_index, entry_id), entry) in &state.entries {
            if stored_tenant == tenant_id && stored_index == index_id {
                cardinality += 1;
                hasher.update(entry_id.as_bytes());
                hasher.update(entry.content_hash.as_bytes());
            }
        }
        let snapshot_hash = format!("sha256:{:x}", hasher.finalize());
        Ok(IndexSnapshot {
            snapshot_id: snapshot_hash.clone(),
            snapshot_hash,
            cardinality,
        })
    }

    fn nearest(
        &self,
        tenant_id: &str,
        index_id: &str,
        embedding: &[f32],
        k: usize,
        exclude_revision: Option<Uuid>,
    ) -> anyhow::Result<Vec<NearestNeighbor>> {
        let state = self.state.lock().expect("index mutex");
        let mut neighbors = state
            .entries
            .iter()
            .filter(|((stored_tenant, stored_index, _), entry)| {
                stored_tenant == tenant_id
                    && stored_index == index_id
                    && entry.embedding.len() == embedding.len()
                    && exclude_revision != Some(entry.revision_id)
            })
            .map(|((_, _, entry_id), entry)| NearestNeighbor {
                entry_id: *entry_id,
                similarity: dot(&entry.embedding, embedding),
            })
            .collect::<Vec<_>>();
        neighbors.sort_by(|left, right| {
            right
                .similarity
                .partial_cmp(&left.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        neighbors.truncate(k);
        Ok(neighbors)
    }
}

impl VectorIndexWriter for IsolatedPipelineIndex {
    fn upsert(
        &self,
        key: &IndexEntryKey,
        embedding: &[f32],
        content_hash: &str,
    ) -> Result<IndexUpsertResult, IndexWriteError> {
        self.writer_calls.fetch_add(1, Ordering::SeqCst);
        let mut state = self.state.lock().expect("index mutex");
        match state.fault {
            IndexFault::FailBeforeApply => {
                state.fault = IndexFault::None;
                return Err(IndexWriteError::Failed);
            }
            IndexFault::LostAfterApply | IndexFault::FailInvalidation | IndexFault::None => {}
        }
        let map_key = (key.tenant_id.clone(), key.index_id.clone(), key.entry_id());
        let digest = IndexEntryKey::content_digest(embedding, content_hash);
        let result = if let Some(existing) = state.entries.get(&map_key) {
            if existing.content_hash == digest {
                IndexUpsertResult::Unchanged
            } else {
                return Err(IndexWriteError::ContentConflict);
            }
        } else {
            state.entries.insert(
                map_key,
                StoredEntry {
                    revision_id: key.revision_id,
                    content_hash: digest,
                    embedding: embedding.to_vec(),
                },
            );
            IndexUpsertResult::Inserted
        };
        if state.fault == IndexFault::LostAfterApply {
            state.fault = IndexFault::None;
            return Err(IndexWriteError::Uncertain);
        }
        Ok(result)
    }
}

/// Writer wrapper that records upserts. Score receives only the reader trait.
pub struct WriteDetectingIndex {
    inner: Arc<IsolatedPipelineIndex>,
    writes: AtomicUsize,
}

impl WriteDetectingIndex {
    pub fn wrap(inner: Arc<IsolatedPipelineIndex>) -> Arc<Self> {
        Arc::new(Self {
            inner,
            writes: AtomicUsize::new(0),
        })
    }

    pub fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

impl VectorIndexReader for WriteDetectingIndex {
    fn snapshot(&self, tenant_id: &str, index_id: &str) -> anyhow::Result<IndexSnapshot> {
        self.inner.snapshot(tenant_id, index_id)
    }

    fn nearest(
        &self,
        tenant_id: &str,
        index_id: &str,
        embedding: &[f32],
        k: usize,
        exclude_revision: Option<Uuid>,
    ) -> anyhow::Result<Vec<NearestNeighbor>> {
        self.inner
            .nearest(tenant_id, index_id, embedding, k, exclude_revision)
    }
}

impl VectorIndexWriter for WriteDetectingIndex {
    fn upsert(
        &self,
        key: &IndexEntryKey,
        embedding: &[f32],
        content_hash: &str,
    ) -> Result<IndexUpsertResult, IndexWriteError> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.upsert(key, embedding, content_hash)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SealedIndexCommand {
    pub schema: String,
    pub index_id: String,
    pub revision_id: Uuid,
    pub projection_id: String,
    pub model_id: String,
    pub entries: Vec<SealedIndexEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SealedIndexEntry {
    pub chunk: u32,
    pub content_hash: String,
    pub embedding: Vec<f32>,
}

impl SealedIndexCommand {
    pub fn include(
        revision_id: Uuid,
        content_hash: String,
        embedding: Vec<f32>,
    ) -> anyhow::Result<Self> {
        Self::include_chunks(revision_id, content_hash, vec![embedding])
    }

    pub fn include_chunks(
        revision_id: Uuid,
        content_hash: String,
        embeddings: Vec<Vec<f32>>,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            content_hash.starts_with("sha256:") && content_hash.len() == 71,
            "index command content hash is malformed"
        );
        anyhow::ensure!(
            !embeddings.is_empty(),
            "index command needs at least one embedding"
        );
        Ok(Self {
            schema: PIPELINE_INDEX_COMMAND_SCHEMA.to_string(),
            index_id: PIPELINE_INDEX_ID.to_string(),
            revision_id,
            projection_id: PIPELINE_PROJECTION_ID.to_string(),
            model_id: PIPELINE_EMBEDDER_MODEL_ID.to_string(),
            entries: embeddings
                .into_iter()
                .enumerate()
                .map(|(chunk, embedding)| SealedIndexEntry {
                    chunk: chunk as u32,
                    content_hash: content_hash.clone(),
                    embedding,
                })
                .collect(),
        })
    }

    pub fn canonical_bytes(&self) -> anyhow::Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(|error| anyhow::anyhow!(error))
    }

    pub fn command_hash(&self) -> anyhow::Result<String> {
        Ok(format!(
            "sha256:{:x}",
            Sha256::digest(self.canonical_bytes()?)
        ))
    }

    pub fn entry_keys(&self, tenant_id: &str) -> Vec<(IndexEntryKey, Vec<f32>, String)> {
        self.entries
            .iter()
            .map(|entry| {
                (
                    IndexEntryKey {
                        tenant_id: tenant_id.to_string(),
                        index_id: self.index_id.clone(),
                        revision_id: self.revision_id,
                        projection_id: self.projection_id.clone(),
                        model_id: self.model_id.clone(),
                        chunk: entry.chunk,
                    },
                    entry.embedding.clone(),
                    entry.content_hash.clone(),
                )
            })
            .collect()
    }
}

pub fn deterministic_pipeline_embedding(source_content_hash: &str) -> Vec<f32> {
    trace_commons_gate_api::ReferenceEmbedder
        .embed(source_content_hash.as_bytes())
        .expect("reference embedder accepts hash bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_refuses_equal_key_with_different_content() {
        let index = IsolatedPipelineIndex::new();
        let key = IndexEntryKey {
            tenant_id: "tenant".to_string(),
            index_id: PIPELINE_INDEX_ID.to_string(),
            revision_id: Uuid::nil(),
            projection_id: PIPELINE_PROJECTION_ID.to_string(),
            model_id: PIPELINE_EMBEDDER_MODEL_ID.to_string(),
            chunk: 0,
        };
        let embedding = vec![1.0; 4];
        assert_eq!(
            index
                .upsert(
                    &key,
                    &embedding,
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                )
                .unwrap(),
            IndexUpsertResult::Inserted
        );
        assert_eq!(
            index
                .upsert(
                    &key,
                    &embedding,
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                )
                .unwrap(),
            IndexUpsertResult::Unchanged
        );
        assert_eq!(
            index.upsert(
                &key,
                &[0.0; 4],
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
            ),
            Err(IndexWriteError::ContentConflict)
        );
        assert_eq!(index.entry_count("tenant", PIPELINE_INDEX_ID), 1);
    }

    #[test]
    fn reader_excludes_the_queried_revision() {
        let index = IsolatedPipelineIndex::new();
        let revision = Uuid::from_u128(7);
        let key = IndexEntryKey {
            tenant_id: "tenant".to_string(),
            index_id: PIPELINE_INDEX_ID.to_string(),
            revision_id: revision,
            projection_id: PIPELINE_PROJECTION_ID.to_string(),
            model_id: PIPELINE_EMBEDDER_MODEL_ID.to_string(),
            chunk: 0,
        };
        index
            .upsert(
                &key,
                &[1.0, 0.0],
                "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            )
            .unwrap();
        let neighbors = VectorIndexReader::nearest(
            index.as_ref(),
            "tenant",
            PIPELINE_INDEX_ID,
            &[1.0, 0.0],
            8,
            Some(revision),
        )
        .unwrap();
        assert!(neighbors.is_empty());
        let snapshot = index.snapshot("tenant", PIPELINE_INDEX_ID).unwrap();
        assert_eq!(snapshot.cardinality, 1);
        assert!(snapshot.snapshot_hash.starts_with("sha256:"));
    }
}
