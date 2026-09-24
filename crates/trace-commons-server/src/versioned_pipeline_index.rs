// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Test-only isolated index. It is never production qualified.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use trace_commons_gate_api::{
    IndexEntryKey, IndexSnapshot, IndexUpsertResult, IndexWriteError, NearestNeighbor,
    VectorIndexReader, VectorIndexWriter,
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexFault {
    None,
    FailBeforeApply,
    LostAfterApply,
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

    pub fn entry_count(&self, tenant_storage_ref: &str, index_id: &str) -> usize {
        self.state
            .lock()
            .expect("index mutex")
            .entries
            .keys()
            .filter(|(stored_tenant, stored_index, _)| {
                stored_tenant == tenant_storage_ref && stored_index == index_id
            })
            .count()
    }
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right.iter()).map(|(a, b)| a * b).sum()
}

/// Same construction as `IndexEntryKey::content_digest` in the port: binds
/// the stored embedding to the content hash it was derived from, so a second
/// upsert under the same key with different bytes is a conflict rather than a
/// silent overwrite.
fn content_digest(embedding: &[f32], content_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content_hash.as_bytes());
    hasher.update(b"\0");
    for value in embedding {
        hasher.update(value.to_le_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

impl VectorIndexReader for IsolatedPipelineIndex {
    fn snapshot(&self, tenant_storage_ref: &str, index_id: &str) -> anyhow::Result<IndexSnapshot> {
        let state = self.state.lock().expect("index mutex");
        let mut hasher = Sha256::new();
        let mut cardinality = 0_u64;
        for ((stored_tenant, stored_index, entry_id), entry) in &state.entries {
            if stored_tenant == tenant_storage_ref && stored_index == index_id {
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
        tenant_storage_ref: &str,
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
                stored_tenant == tenant_storage_ref
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
            IndexFault::LostAfterApply | IndexFault::None => {}
        }
        let map_key = (
            key.tenant_storage_ref.clone(),
            key.index_id.clone(),
            key.entry_id(),
        );
        let digest = content_digest(embedding, content_hash);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_refuses_equal_key_with_different_content() {
        let index = IsolatedPipelineIndex::new();
        let key = IndexEntryKey {
            tenant_storage_ref: "tenant_sha256:80a707af7dc77ee1228f9127180f3964".to_string(),
            index_id: "pipeline-test-index-v1".to_string(),
            revision_id: Uuid::nil(),
            projection_id: "pipeline-test-projection-v1".to_string(),
            model_id: "pipeline-test-embedder-v1".to_string(),
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
        assert_eq!(
            index.entry_count(
                "tenant_sha256:80a707af7dc77ee1228f9127180f3964",
                "pipeline-test-index-v1"
            ),
            1
        );
    }

    #[test]
    fn reader_excludes_the_queried_revision() {
        let index = IsolatedPipelineIndex::new();
        let revision = Uuid::from_u128(7);
        let key = IndexEntryKey {
            tenant_storage_ref: "tenant_sha256:80a707af7dc77ee1228f9127180f3964".to_string(),
            index_id: "pipeline-test-index-v1".to_string(),
            revision_id: revision,
            projection_id: "pipeline-test-projection-v1".to_string(),
            model_id: "pipeline-test-embedder-v1".to_string(),
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
            "tenant_sha256:80a707af7dc77ee1228f9127180f3964",
            "pipeline-test-index-v1",
            &[1.0, 0.0],
            8,
            Some(revision),
        )
        .unwrap();
        assert!(neighbors.is_empty());
        let snapshot = index
            .snapshot(
                "tenant_sha256:80a707af7dc77ee1228f9127180f3964",
                "pipeline-test-index-v1",
            )
            .unwrap();
        assert_eq!(snapshot.cardinality, 1);
        assert!(snapshot.snapshot_hash.starts_with("sha256:"));
    }

    #[test]
    fn tenants_do_not_see_each_other() {
        let index = IsolatedPipelineIndex::new();
        let key = IndexEntryKey {
            tenant_storage_ref: "tenant_sha256:80a707af7dc77ee1228f9127180f3964".to_string(),
            index_id: "pipeline-test-index-v1".to_string(),
            revision_id: Uuid::from_u128(9),
            projection_id: "pipeline-test-projection-v1".to_string(),
            model_id: "reference-embedder-v1".to_string(),
            chunk: 0,
        };
        let hash = format!("sha256:{}", "c".repeat(64));
        assert_eq!(
            index.upsert(&key, &[1.0, 0.0], &hash),
            Ok(IndexUpsertResult::Inserted)
        );
        let other = "tenant_sha256:00000000000000000000000000000000";
        assert_eq!(
            index
                .snapshot(other, "pipeline-test-index-v1")
                .unwrap()
                .cardinality,
            0
        );
        assert!(
            index
                .nearest(other, "pipeline-test-index-v1", &[1.0, 0.0], 4, None)
                .unwrap()
                .is_empty()
        );
    }
}
