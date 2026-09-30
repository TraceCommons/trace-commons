// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Test-only isolated index. It is never production qualified.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::TenantStorageRef;
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
    /// The digest `content_digest` derives from the raw content hash and the
    /// embedding together -- binds the two so a second upsert under the same
    /// key with either changed is a conflict rather than a silent overwrite.
    content_digest: String,
    /// The raw content hash `upsert` was called with, kept alongside the
    /// digest so `entry_set_hash` can hash over the same fields a caller
    /// actually wrote, not a derived digest.
    content_hash: String,
    embedding: Vec<f32>,
}

/// Entries keyed by the tenant's derived storage reference, the index id,
/// and the entry id.
#[derive(Debug)]
struct IsolatedIndexState {
    entries: BTreeMap<(TenantStorageRef, String, Uuid), StoredEntry>,
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

    pub fn entry_count(&self, tenant_storage_ref: &TenantStorageRef, index_id: &str) -> usize {
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

    /// The number of distinct revisions with at least one entry under
    /// `tenant_storage_ref` and `index_id`.
    pub fn revision_count(&self, tenant_storage_ref: &TenantStorageRef, index_id: &str) -> usize {
        self.state
            .lock()
            .expect("index mutex")
            .entries
            .iter()
            .filter(|((stored_tenant, stored_index, _), _)| {
                stored_tenant == tenant_storage_ref && stored_index == index_id
            })
            .map(|(_, entry)| entry.revision_id)
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    }

    /// SHA-256 over every entry this tenant's index holds under `index_id`:
    /// the entry id, its raw content hash, and its embedding as
    /// little-endian `f32` bytes, in ascending entry-id order. The
    /// `BTreeMap` already iterates in `(tenant, index, entry_id)` order, so
    /// filtering to one tenant and index yields entries already sorted by
    /// entry id -- no separate sort is needed.
    ///
    /// Used to compare two indexes' contents for exact equality -- for
    /// example a live index against one rebuilt from the same sealed
    /// commands -- without comparing every entry field by hand. Two indexes
    /// with the same entries under the same tenant and index id hash equal
    /// regardless of the order they were written in.
    pub fn entry_set_hash(&self, tenant_storage_ref: &TenantStorageRef, index_id: &str) -> String {
        let state = self.state.lock().expect("index mutex");
        let mut hasher = Sha256::new();
        for ((stored_tenant, stored_index, entry_id), entry) in &state.entries {
            if stored_tenant == tenant_storage_ref && stored_index == index_id {
                hasher.update(entry_id.as_bytes());
                hasher.update(entry.content_hash.as_bytes());
                hasher.update(b"\0");
                for value in &entry.embedding {
                    hasher.update(value.to_le_bytes());
                }
            }
        }
        format!("sha256:{:x}", hasher.finalize())
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
    fn snapshot(
        &self,
        tenant_storage_ref: &TenantStorageRef,
        index_id: &str,
    ) -> anyhow::Result<IndexSnapshot> {
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
        tenant_storage_ref: &TenantStorageRef,
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
            if existing.content_digest == digest {
                IndexUpsertResult::Unchanged
            } else {
                return Err(IndexWriteError::ContentConflict);
            }
        } else {
            state.entries.insert(
                map_key,
                StoredEntry {
                    revision_id: key.revision_id,
                    content_digest: digest,
                    content_hash: content_hash.to_string(),
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

    /// Removes every entry of `revision_id` in `index_id` under
    /// `tenant_storage_ref`, and nothing else: `Ok(true)` when it removed
    /// entries, `Ok(false)` when there were none, so a repeated call is
    /// `Ok(false)`. It never reports `ContentConflict`. An injected fault
    /// applies as it does to `upsert`: `FailBeforeApply` removes nothing and
    /// is `Failed`, and `LostAfterApply` removes the entries and is
    /// `Uncertain`, so a failed invalidation never reads as success.
    fn invalidate_revision(
        &self,
        tenant_storage_ref: &TenantStorageRef,
        index_id: &str,
        revision_id: Uuid,
    ) -> Result<bool, IndexWriteError> {
        let mut state = self.state.lock().expect("index mutex");
        if state.fault == IndexFault::FailBeforeApply {
            state.fault = IndexFault::None;
            return Err(IndexWriteError::Failed);
        }
        let before = state.entries.len();
        state
            .entries
            .retain(|(stored_tenant, stored_index, _), entry| {
                !(stored_tenant == tenant_storage_ref
                    && stored_index == index_id
                    && entry.revision_id == revision_id)
            });
        let removed = state.entries.len() < before;
        if state.fault == IndexFault::LostAfterApply {
            state.fault = IndexFault::None;
            return Err(IndexWriteError::Uncertain);
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant() -> TenantStorageRef {
        TenantStorageRef::new("tenant_sha256:80a707af7dc77ee1228f9127180f3964").unwrap()
    }

    #[test]
    fn writer_refuses_equal_key_with_different_content() {
        let index = IsolatedPipelineIndex::new();
        let key = IndexEntryKey {
            tenant_storage_ref: tenant(),
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
        assert_eq!(index.entry_count(&tenant(), "pipeline-test-index-v1"), 1);
    }

    #[test]
    fn reader_excludes_the_queried_revision() {
        let index = IsolatedPipelineIndex::new();
        let revision = Uuid::from_u128(7);
        let key = IndexEntryKey {
            tenant_storage_ref: tenant(),
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
            &tenant(),
            "pipeline-test-index-v1",
            &[1.0, 0.0],
            8,
            Some(revision),
        )
        .unwrap();
        assert!(neighbors.is_empty());
        let snapshot = index.snapshot(&tenant(), "pipeline-test-index-v1").unwrap();
        assert_eq!(snapshot.cardinality, 1);
        assert!(snapshot.snapshot_hash.starts_with("sha256:"));
    }

    #[test]
    fn tenants_do_not_see_each_other() {
        let index = IsolatedPipelineIndex::new();
        let key = IndexEntryKey {
            tenant_storage_ref: tenant(),
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
        let other =
            TenantStorageRef::new("tenant_sha256:00000000000000000000000000000000").unwrap();
        assert_eq!(
            index
                .snapshot(&other, "pipeline-test-index-v1")
                .unwrap()
                .cardinality,
            0
        );
        assert!(
            index
                .nearest(&other, "pipeline-test-index-v1", &[1.0, 0.0], 4, None)
                .unwrap()
                .is_empty()
        );
    }

    /// Invalidation removes every entry of one revision in one index under
    /// one tenant, and nothing else: another revision, another index, and
    /// another tenant keep their entries. A second call reports that no
    /// entries remained, and a fault stays visible instead of reading as
    /// success.
    #[test]
    fn invalidate_revision_is_idempotent_and_removes_only_that_revision() {
        const INDEX: &str = "pipeline-test-index-v1";
        const OTHER_INDEX: &str = "pipeline-test-index-v2";
        let index = IsolatedPipelineIndex::new();
        let other_tenant =
            TenantStorageRef::new("tenant_sha256:00000000000000000000000000000000").unwrap();
        let revision = Uuid::from_u128(11);
        let other_revision = Uuid::from_u128(12);
        let key = |tenant: &TenantStorageRef, index_id: &str, revision_id: Uuid, chunk: u32| {
            IndexEntryKey {
                tenant_storage_ref: tenant.clone(),
                index_id: index_id.to_string(),
                revision_id,
                projection_id: "pipeline-test-projection-v1".to_string(),
                model_id: "reference-embedder-v1".to_string(),
                chunk,
            }
        };
        let entries = [
            key(&tenant(), INDEX, revision, 0),
            key(&tenant(), INDEX, revision, 1),
            key(&tenant(), INDEX, other_revision, 0),
            key(&tenant(), OTHER_INDEX, revision, 0),
            key(&other_tenant, INDEX, revision, 0),
        ];
        for (position, entry) in entries.iter().enumerate() {
            assert_eq!(
                index.upsert(
                    entry,
                    &[1.0, position as f32],
                    &format!("sha256:{position:064x}")
                ),
                Ok(IndexUpsertResult::Inserted)
            );
        }
        let nearest_ids = |tenant: &TenantStorageRef, index_id: &str| {
            let mut ids = index
                .nearest(tenant, index_id, &[1.0, 0.0], 8, None)
                .unwrap()
                .into_iter()
                .map(|neighbor| neighbor.entry_id)
                .collect::<Vec<_>>();
            ids.sort();
            ids
        };

        assert_eq!(
            index.invalidate_revision(&tenant(), INDEX, revision),
            Ok(true)
        );
        assert_eq!(index.entry_count(&tenant(), INDEX), 1);
        assert_eq!(
            nearest_ids(&tenant(), INDEX),
            vec![entries[2].entry_id()],
            "only the other revision remains in the index"
        );
        assert_eq!(
            nearest_ids(&tenant(), OTHER_INDEX),
            vec![entries[3].entry_id()],
            "another index keeps the revision's entry"
        );
        assert_eq!(
            nearest_ids(&other_tenant, INDEX),
            vec![entries[4].entry_id()],
            "another tenant keeps its entry"
        );

        assert_eq!(
            index.invalidate_revision(&tenant(), INDEX, revision),
            Ok(false),
            "a repeated call reports that no entries remained"
        );
        assert_eq!(index.entry_count(&tenant(), INDEX), 1);
        assert_eq!(
            index.invalidate_revision(&tenant(), INDEX, Uuid::from_u128(99)),
            Ok(false)
        );

        index.set_fault(IndexFault::FailBeforeApply);
        assert_eq!(
            index.invalidate_revision(&other_tenant, INDEX, revision),
            Err(IndexWriteError::Failed)
        );
        assert_eq!(
            index.entry_count(&other_tenant, INDEX),
            1,
            "a failed invalidation removes nothing"
        );
        index.set_fault(IndexFault::LostAfterApply);
        assert_eq!(
            index.invalidate_revision(&other_tenant, INDEX, revision),
            Err(IndexWriteError::Uncertain)
        );
        assert_eq!(
            index.invalidate_revision(&other_tenant, INDEX, revision),
            Ok(false),
            "a retry after an uncertain result is safe"
        );
        assert_eq!(index.entry_count(&other_tenant, INDEX), 0);
    }
}
