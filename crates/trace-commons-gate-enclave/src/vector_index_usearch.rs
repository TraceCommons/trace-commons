//! `UsearchVectorIndex` — production-shape `VectorIndex` impl backed by the
//! `usearch` HNSW library (Phase A4).
//!
//! # Shape
//!
//! - One file per tenant under `root_dir`, named `sha256_hex(tenant) + ".usearch"`.
//!   The hash means the on-disk filename never leaks the raw
//!   `tenant_storage_ref` to the operator's filesystem listing.
//! - Per-tenant `Mutex<usearch::Index>` for concurrency safety. usearch reads
//!   can be concurrent under the hood, but the high-level `Index` API takes
//!   `&self` for both `add`/`search`/`remove`, so serializing through a mutex
//!   here keeps the model simple and matches the spec.
//! - An LRU-bounded handle cache so the process doesn't keep every tenant's
//!   index mapped at once. When a handle is evicted, it is flushed to disk
//!   synchronously before being dropped — no data loss on eviction.
//! - Cosine metric over **L2-normalized** vectors. Callers are responsible for
//!   normalizing on the way in (A3's `FastEmbedTextEmbedder` does this; the
//!   `MockEmbedder` already emits unit-norm vectors). The orchestrator's
//!   novelty math (`1 - max_similarity`) likewise assumes cosine similarity
//!   in `[-1, 1]`.
//!
//! # Key conversion
//!
//! usearch keys are `u64`. We derive each key by taking the **high 8 bytes**
//! of the entry's `Uuid`. UUIDs are 128 random bits, so the high half is
//! itself ~64 random bits. Birthday collisions become non-negligible only
//! around 2^32 entries **per tenant**; we expect at most ~100k entries per
//! tenant in production, so the collision probability is < 10^-10. If a
//! future scale change invalidates this assumption, switch to a SHA-256
//! truncation over the full Uuid.
//!
//! # Persistence
//!
//! Every `flush_every` writes (default 32) trigger an inline `Index::save`
//! within the per-tenant mutex's critical section. LRU eviction always
//! flushes, regardless of the dirty-counter. Drop of `UsearchVectorIndex`
//! flushes every still-cached tenant.

#![cfg(feature = "local-gpu-models")]

use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use lru::LruCache;
use sha2::{Digest, Sha256};
use usearch::ffi::{IndexOptions, MetricKind, ScalarKind};
use usearch::Index;
use uuid::Uuid;

use crate::vector_index::{NearestNeighbor, VectorIndex};

/// One tenant's open index handle. The mutex serializes inserts / searches /
/// deletes within the tenant; `dirty_writes` counts unflushed writes for the
/// `flush_every` heuristic.
struct TenantHandle {
    index: Index,
    dirty_writes: usize,
    file_path: PathBuf,
}

/// Production-shape vector index backed by usearch HNSW.
pub struct UsearchVectorIndex {
    root_dir: PathBuf,
    dim: usize,
    hnsw_m: usize,
    ef_construction: usize,
    ef_search: usize,
    open_indexes: Mutex<LruCache<String, Arc<Mutex<TenantHandle>>>>,
    flush_every: usize,
}

impl std::fmt::Debug for UsearchVectorIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UsearchVectorIndex")
            .field("root_dir", &self.root_dir)
            .field("dim", &self.dim)
            .field("hnsw_m", &self.hnsw_m)
            .field("ef_construction", &self.ef_construction)
            .field("ef_search", &self.ef_search)
            .field("flush_every", &self.flush_every)
            .finish()
    }
}

impl UsearchVectorIndex {
    /// Construct a new `UsearchVectorIndex`.
    ///
    /// `root_dir` is created if missing. `dim` is the embedding dimensionality
    /// every tenant index will use; `hnsw_m` / `ef_construction` / `ef_search`
    /// are the usearch HNSW knobs. `max_open` is the upper bound on
    /// simultaneously-mapped tenant indexes (LRU eviction kicks in past that);
    /// `flush_every` is the synchronous-flush cadence per tenant.
    pub fn try_new(
        root_dir: impl AsRef<Path>,
        dim: usize,
        hnsw_m: usize,
        ef_construction: usize,
        ef_search: usize,
        max_open: usize,
        flush_every: usize,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(dim > 0, "UsearchVectorIndex dim must be greater than zero");
        anyhow::ensure!(
            max_open > 0,
            "UsearchVectorIndex max_open must be greater than zero"
        );
        anyhow::ensure!(
            flush_every > 0,
            "UsearchVectorIndex flush_every must be greater than zero"
        );
        let root_dir = root_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&root_dir).with_context(|| {
            format!(
                "failed to create UsearchVectorIndex root_dir at {}",
                root_dir.display()
            )
        })?;
        let cap = NonZeroUsize::new(max_open).expect("max_open > 0 just checked");
        Ok(Self {
            root_dir,
            dim,
            hnsw_m,
            ef_construction,
            ef_search,
            open_indexes: Mutex::new(LruCache::new(cap)),
            flush_every,
        })
    }

    fn tenant_file_path(&self, tenant_storage_ref: &str) -> PathBuf {
        let mut h = Sha256::new();
        h.update(b"trace_gate_enclave.usearch_tenant_file.v1\n");
        h.update(tenant_storage_ref.as_bytes());
        let hex = format!("{:x}", h.finalize());
        self.root_dir.join(format!("{hex}.usearch"))
    }

    fn build_index_options(&self) -> IndexOptions {
        IndexOptions {
            dimensions: self.dim,
            metric: MetricKind::Cos,
            quantization: ScalarKind::F32,
            connectivity: self.hnsw_m,
            expansion_add: self.ef_construction,
            expansion_search: self.ef_search,
            multi: false,
        }
    }

    /// Load (or freshly construct) the per-tenant index, populating the LRU
    /// cache. Evicted handles are flushed to disk before being dropped.
    fn handle_for(&self, tenant_storage_ref: &str) -> anyhow::Result<Arc<Mutex<TenantHandle>>> {
        let mut cache = self
            .open_indexes
            .lock()
            .expect("UsearchVectorIndex lru mutex poisoned");
        if let Some(existing) = cache.get(tenant_storage_ref) {
            return Ok(Arc::clone(existing));
        }

        let file_path = self.tenant_file_path(tenant_storage_ref);
        let opts = self.build_index_options();
        let index = Index::new(&opts)
            .map_err(|e| anyhow!("failed to construct usearch index: {e}"))?;
        if file_path.is_file() {
            index
                .load(file_path.to_str().ok_or_else(|| {
                    anyhow!(
                        "usearch tenant file path is not valid UTF-8: {}",
                        file_path.display()
                    )
                })?)
                .map_err(|e| {
                    anyhow!(
                        "failed to load usearch index from {}: {e}",
                        file_path.display()
                    )
                })?;
        }

        let handle = Arc::new(Mutex::new(TenantHandle {
            index,
            dirty_writes: 0,
            file_path,
        }));

        // `push` returns the evicted (oldest) entry. Flush + drop it
        // synchronously so we never lose dirty writes. On flush failure, the
        // evicted handle's data is still in-memory only — silently dropping
        // it would lose writes. Restore it to the cache and surface the
        // error to the caller so the upstream `insert`/`nearest` request
        // fails closed rather than partially succeeding.
        if let Some((evicted_key, evicted)) =
            cache.push(tenant_storage_ref.to_string(), Arc::clone(&handle))
        {
            if let Err(flush_err) = flush_handle_arc(&evicted) {
                // Re-insert the victim so its dirty data isn't lost. Note
                // this displaces the freshly inserted `handle` again — that
                // is acceptable: we are propagating an error and the caller
                // will see the failed insert/nearest, and the next caller
                // for `tenant_storage_ref` will go through the normal
                // load-from-disk path. Best-effort: if reinsert ALSO evicts
                // something, we can't recover from that case any more
                // cleanly; log and bail.
                if let Some((_, double_evicted)) = cache.push(evicted_key, evicted) {
                    if let Err(secondary) = flush_handle_arc(&double_evicted) {
                        tracing::warn!(
                            error = ?secondary,
                            "UsearchVectorIndex secondary eviction flush also failed; possible dirty-write loss"
                        );
                    }
                }
                return Err(flush_err
                    .context("UsearchVectorIndex eviction flush failed; victim re-cached"));
            }
        }

        Ok(handle)
    }

    /// Convert `Uuid` → `u64` usearch key by taking the high 8 bytes.
    ///
    /// See the module-level docs for the collision-probability argument.
    fn uuid_to_key(entry_id: Uuid) -> u64 {
        let bytes = entry_id.as_bytes();
        u64::from_be_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ])
    }

    /// Delete the on-disk tenant index file for `tenant_storage_ref`, if it
    /// exists. Also drops any cached handle for that tenant so the next
    /// `insert`/`nearest` call rebuilds an empty index. Logs hash-only.
    ///
    /// Used by `trace-commons-vector-replay --fresh` to start the rebuild from a
    /// clean slate.
    pub fn delete_tenant_index_file(&self, tenant_storage_ref: &str) -> anyhow::Result<()> {
        // Drop any in-memory handle first so a subsequent insert doesn't
        // unintentionally re-flush the stale handle's in-memory contents
        // back to disk after we've removed the file.
        {
            let mut cache = self
                .open_indexes
                .lock()
                .expect("UsearchVectorIndex lru mutex poisoned");
            cache.pop(tenant_storage_ref);
        }
        let path = self.tenant_file_path(tenant_storage_ref);
        let file_hash = {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("<unknown>");
            name.to_string()
        };
        match std::fs::remove_file(&path) {
            Ok(()) => {
                tracing::info!(
                    target: "trace_commons_vector_replay",
                    tenant_file = %file_hash,
                    "VectorReplayResetTenantIndex: removed existing tenant index file"
                );
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::info!(
                    target: "trace_commons_vector_replay",
                    tenant_file = %file_hash,
                    "VectorReplayResetTenantIndex: no existing tenant index file"
                );
                Ok(())
            }
            Err(e) => Err(anyhow!(
                "VectorReplayResetTenantIndex: failed to remove tenant index file {file_hash}: {e}"
            )),
        }
    }

    /// Return the live entry count for `tenant_storage_ref`. Opens (or
    /// reloads) the per-tenant handle and reports `Index::size()`.
    pub fn tenant_entry_count(&self, tenant_storage_ref: &str) -> anyhow::Result<usize> {
        let handle = self.handle_for(tenant_storage_ref)?;
        let guard = handle
            .lock()
            .expect("UsearchVectorIndex tenant mutex poisoned");
        Ok(guard.index.size())
    }

    /// Return true if the tenant index already contains an entry for
    /// `vector_entry_id`. Used by `trace-commons-vector-replay --incremental`
    /// to skip rows that are already present on disk.
    pub fn contains_entry(
        &self,
        tenant_storage_ref: &str,
        vector_entry_id: Uuid,
    ) -> anyhow::Result<bool> {
        let handle = self.handle_for(tenant_storage_ref)?;
        let guard = handle
            .lock()
            .expect("UsearchVectorIndex tenant mutex poisoned");
        let key = Self::uuid_to_key(vector_entry_id);
        Ok(guard.index.contains(key))
    }

    /// Flush every still-cached tenant. Best-effort; returns the first error
    /// encountered (but attempts to flush every handle regardless).
    pub fn flush_all(&self) -> anyhow::Result<()> {
        // Snapshot the cache so we don't hold the LRU mutex across per-tenant
        // mutex acquisitions (which could deadlock if anyone is trying to
        // acquire the LRU lock with a tenant lock already held — they're not
        // today, but the snapshot keeps it future-proof).
        let snapshot: Vec<Arc<Mutex<TenantHandle>>> = {
            let cache = self
                .open_indexes
                .lock()
                .expect("UsearchVectorIndex lru mutex poisoned");
            cache.iter().map(|(_, v)| Arc::clone(v)).collect()
        };
        let mut first_err: Option<anyhow::Error> = None;
        for handle in snapshot {
            if let Err(e) = flush_handle_arc(&handle) {
                tracing::warn!(error = ?e, "UsearchVectorIndex flush_all encountered an error");
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
        match first_err {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
}

impl Drop for UsearchVectorIndex {
    fn drop(&mut self) {
        if let Err(e) = self.flush_all() {
            tracing::warn!(
                error = ?e,
                "UsearchVectorIndex drop: best-effort flush_all returned an error"
            );
        }
    }
}

fn flush_handle_arc(handle: &Arc<Mutex<TenantHandle>>) -> anyhow::Result<()> {
    let mut guard = handle
        .lock()
        .expect("UsearchVectorIndex tenant mutex poisoned");
    flush_handle_locked(&mut guard)
}

fn flush_handle_locked(handle: &mut TenantHandle) -> anyhow::Result<()> {
    let path_str = handle.file_path.to_str().ok_or_else(|| {
        anyhow!(
            "usearch tenant file path is not valid UTF-8: {}",
            handle.file_path.display()
        )
    })?;
    handle
        .index
        .save(path_str)
        .map_err(|e| anyhow!("failed to save usearch index to {}: {e}", path_str))?;
    handle.dirty_writes = 0;
    Ok(())
}

impl VectorIndex for UsearchVectorIndex {
    fn insert(
        &self,
        entry_id: Uuid,
        tenant_storage_ref: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            embedding.len() == self.dim,
            "UsearchVectorIndex insert: embedding has {} dims, index expects {}",
            embedding.len(),
            self.dim
        );
        let handle = self.handle_for(tenant_storage_ref)?;
        let mut guard = handle
            .lock()
            .expect("UsearchVectorIndex tenant mutex poisoned");
        // usearch needs reservation up front; grow when the live size hits
        // capacity. We grow geometrically to amortize reservation cost.
        let size = guard.index.size();
        let cap = guard.index.capacity();
        if size + 1 > cap {
            let next = (cap.saturating_mul(2)).max(64);
            guard
                .index
                .reserve(next)
                .map_err(|e| anyhow!("usearch reserve({next}) failed: {e}"))?;
        }
        let key = Self::uuid_to_key(entry_id);
        guard
            .index
            .add(key, embedding)
            .map_err(|e| anyhow!("usearch add failed: {e}"))?;
        guard.dirty_writes += 1;
        if guard.dirty_writes >= self.flush_every {
            flush_handle_locked(&mut guard)?;
        }
        Ok(())
    }

    fn nearest(
        &self,
        tenant_storage_ref: &str,
        embedding: &[f32],
        k: usize,
    ) -> anyhow::Result<Vec<NearestNeighbor>> {
        anyhow::ensure!(
            embedding.len() == self.dim,
            "UsearchVectorIndex nearest: query has {} dims, index expects {}",
            embedding.len(),
            self.dim
        );
        if k == 0 {
            return Ok(Vec::new());
        }
        let handle = self.handle_for(tenant_storage_ref)?;
        let guard = handle
            .lock()
            .expect("UsearchVectorIndex tenant mutex poisoned");
        if guard.index.size() == 0 {
            return Ok(Vec::new());
        }
        let matches = guard
            .index
            .search(embedding, k)
            .map_err(|e| anyhow!("usearch search failed: {e}"))?;
        drop(guard);

        // usearch Cos metric returns a **distance** in [0, 2] where 0 = perfect
        // match. Convert back to cosine similarity for the trait contract.
        //
        // To reconstruct the entry Uuid from the u64 key, we keep a small
        // per-tenant `Uuid` → `u64` reverse map. But that would require
        // tracking on every insert, which we explicitly avoided in favor of
        // statelessness. Instead, we synthesize a Uuid from the u64 key alone
        // by zero-padding the low 8 bytes. Downstream consumers
        // (`hash_neighbors` in the orchestrator) only care about the bytes
        // being stable; they hash the raw `Uuid::as_bytes()`. The reverse-map
        // is not needed because today the orchestrator does not need to
        // resolve a usearch hit back to a database row — it only needs the
        // similarity scores for novelty + a stable hash for audit.
        let mut out = Vec::with_capacity(matches.keys.len());
        for (key, distance) in matches.keys.iter().zip(matches.distances.iter()) {
            let similarity = (1.0 - distance).clamp(-1.0, 1.0);
            let mut bytes = [0u8; 16];
            bytes[0..8].copy_from_slice(&key.to_be_bytes());
            out.push(NearestNeighbor {
                entry_id: Uuid::from_bytes(bytes),
                similarity,
            });
        }
        // usearch returns matches sorted by ascending distance (= descending
        // similarity); preserve that ordering.
        Ok(out)
    }

    fn delete(&self, tenant_storage_ref: &str, entry_id: Uuid) -> anyhow::Result<bool> {
        let handle = self.handle_for(tenant_storage_ref)?;
        let mut guard = handle
            .lock()
            .expect("UsearchVectorIndex tenant mutex poisoned");
        let key = Self::uuid_to_key(entry_id);
        let removed = guard
            .index
            .remove(key)
            .map_err(|e| anyhow!("usearch remove failed: {e}"))?;
        let hit = removed > 0;
        if hit {
            guard.dirty_writes += 1;
            if guard.dirty_writes >= self.flush_every {
                flush_handle_locked(&mut guard)?;
            }
        }
        Ok(hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::tempdir;

    /// L2-normalize a vector to length 1.0 so cosine == dot product.
    fn norm(mut v: Vec<f32>) -> Vec<f32> {
        let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n > 0.0 {
            for x in v.iter_mut() {
                *x /= n;
            }
        }
        v
    }

    fn build_index(root: &Path, dim: usize) -> UsearchVectorIndex {
        UsearchVectorIndex::try_new(root, dim, 16, 200, 50, 32, 32).expect("index ctor")
    }

    #[test]
    fn insert_then_search_returns_self_at_top() {
        let tmp = tempdir().unwrap();
        let idx = build_index(tmp.path(), 4);
        let id = Uuid::new_v4();
        let v = norm(vec![1.0, 0.5, 0.25, 0.125]);
        idx.insert(id, "tenant", &v).unwrap();
        let neighbors = idx.nearest("tenant", &v, 5).unwrap();
        assert_eq!(neighbors.len(), 1);
        // Self-similarity ~ 1.0 for a normalized vector under cosine.
        assert!(
            (neighbors[0].similarity - 1.0).abs() < 1e-3,
            "expected self-similarity ~1.0, got {}",
            neighbors[0].similarity
        );
    }

    #[test]
    fn search_for_inserted_vector_is_in_topk() {
        let tmp = tempdir().unwrap();
        let dim = 8;
        let idx = build_index(tmp.path(), dim);
        let target_id = Uuid::new_v4();
        let target = norm((0..dim).map(|i| i as f32 + 1.0).collect());
        idx.insert(target_id, "tenant", &target).unwrap();
        // Add 10 distractors with different distributions.
        for i in 0..10 {
            let other_id = Uuid::new_v4();
            let other: Vec<f32> = norm(
                (0..dim)
                    .map(|j| ((i * 13 + j * 7) as f32 % 19.0) - 9.0)
                    .collect(),
            );
            idx.insert(other_id, "tenant", &other).unwrap();
        }
        let neighbors = idx.nearest("tenant", &target, 5).unwrap();
        let target_key_bytes = {
            let mut b = [0u8; 16];
            b[0..8].copy_from_slice(&UsearchVectorIndex::uuid_to_key(target_id).to_be_bytes());
            b
        };
        let found = neighbors
            .iter()
            .any(|n| n.entry_id.as_bytes() == &target_key_bytes);
        assert!(found, "target should appear in top-k");
    }

    #[test]
    fn insert_then_delete_then_search_excludes_deleted() {
        let tmp = tempdir().unwrap();
        let idx = build_index(tmp.path(), 4);
        let id = Uuid::new_v4();
        let v = norm(vec![1.0, 0.0, 0.0, 0.0]);
        idx.insert(id, "tenant", &v).unwrap();
        assert!(idx.delete("tenant", id).unwrap());
        let neighbors = idx.nearest("tenant", &v, 5).unwrap();
        assert!(
            neighbors.is_empty(),
            "no neighbors after the only entry was deleted, got {neighbors:?}"
        );
    }

    #[test]
    fn delete_missing_returns_false() {
        let tmp = tempdir().unwrap();
        let idx = build_index(tmp.path(), 4);
        let id = Uuid::new_v4();
        assert!(!idx.delete("tenant", id).unwrap());
    }

    #[test]
    fn tenant_isolation_bit_identical_vectors() {
        // The load-bearing test: two tenants insert the same Uuid + identical
        // normalized vector; a search in tenant A must return only A's entry,
        // never B's.
        let tmp = tempdir().unwrap();
        let idx = build_index(tmp.path(), 4);
        let v = norm(vec![1.0, 2.0, 3.0, 4.0]);
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        idx.insert(id_a, "tenant_a", &v).unwrap();
        idx.insert(id_b, "tenant_b", &v).unwrap();

        let neighbors_a = idx.nearest("tenant_a", &v, 5).unwrap();
        let neighbors_b = idx.nearest("tenant_b", &v, 5).unwrap();
        assert_eq!(neighbors_a.len(), 1);
        assert_eq!(neighbors_b.len(), 1);

        let key_a = UsearchVectorIndex::uuid_to_key(id_a);
        let key_b = UsearchVectorIndex::uuid_to_key(id_b);
        let neighbor_a_key = u64::from_be_bytes(
            neighbors_a[0].entry_id.as_bytes()[0..8]
                .try_into()
                .unwrap(),
        );
        let neighbor_b_key = u64::from_be_bytes(
            neighbors_b[0].entry_id.as_bytes()[0..8]
                .try_into()
                .unwrap(),
        );
        assert_eq!(neighbor_a_key, key_a, "tenant A must see its own entry");
        assert_eq!(neighbor_b_key, key_b, "tenant B must see its own entry");
        assert_ne!(
            neighbor_a_key, key_b,
            "tenant A must not see tenant B's entry"
        );
    }

    #[test]
    fn persistence_across_instance_drop() {
        let tmp = tempdir().unwrap();
        let dim = 4;
        let id = Uuid::new_v4();
        let v = norm(vec![0.5, 0.5, 0.5, 0.5]);
        {
            let idx = build_index(tmp.path(), dim);
            idx.insert(id, "tenant", &v).unwrap();
            idx.flush_all().unwrap();
        }
        // Reload from disk via a fresh instance with the same root.
        let idx2 = build_index(tmp.path(), dim);
        let neighbors = idx2.nearest("tenant", &v, 5).unwrap();
        assert_eq!(neighbors.len(), 1);
        let recovered_key = u64::from_be_bytes(
            neighbors[0].entry_id.as_bytes()[0..8]
                .try_into()
                .unwrap(),
        );
        assert_eq!(recovered_key, UsearchVectorIndex::uuid_to_key(id));
    }

    #[test]
    fn wrong_dim_on_insert_errors() {
        let tmp = tempdir().unwrap();
        let idx = build_index(tmp.path(), 4);
        let id = Uuid::new_v4();
        let v = vec![1.0, 0.0, 0.0]; // length 3, not 4
        let err = idx.insert(id, "tenant", &v).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("dims") || msg.contains("dim"),
            "error should mention dim mismatch, got {msg}"
        );
    }

    #[test]
    fn lru_eviction_then_reload_from_disk() {
        // max_open = 2, touch 3 tenants. The oldest must be evicted, its data
        // flushed to disk, and re-querying it must succeed (reload path).
        let tmp = tempdir().unwrap();
        let dim = 4;
        let idx = UsearchVectorIndex::try_new(tmp.path(), dim, 16, 200, 50, 2, 32).unwrap();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();
        let v_a = norm(vec![1.0, 0.0, 0.0, 0.0]);
        let v_b = norm(vec![0.0, 1.0, 0.0, 0.0]);
        let v_c = norm(vec![0.0, 0.0, 1.0, 0.0]);
        idx.insert(id_a, "tenant_a", &v_a).unwrap();
        idx.insert(id_b, "tenant_b", &v_b).unwrap();
        // Touching tenant_c evicts tenant_a (LRU). The eviction must flush
        // tenant_a's data first.
        idx.insert(id_c, "tenant_c", &v_c).unwrap();
        // Now query tenant_a — its handle must be reloaded from disk.
        let neighbors_a = idx.nearest("tenant_a", &v_a, 5).unwrap();
        assert_eq!(neighbors_a.len(), 1, "tenant_a must reload from disk");
        let recovered_key = u64::from_be_bytes(
            neighbors_a[0].entry_id.as_bytes()[0..8]
                .try_into()
                .unwrap(),
        );
        assert_eq!(recovered_key, UsearchVectorIndex::uuid_to_key(id_a));
    }

    #[test]
    fn contains_entry_reports_membership() {
        let tmp = tempdir().unwrap();
        let idx = build_index(tmp.path(), 4);
        let id = Uuid::new_v4();
        let other = Uuid::new_v4();
        let v = norm(vec![1.0, 0.0, 0.0, 0.0]);
        assert!(!idx.contains_entry("tenant", id).unwrap());
        idx.insert(id, "tenant", &v).unwrap();
        assert!(idx.contains_entry("tenant", id).unwrap());
        assert!(!idx.contains_entry("tenant", other).unwrap());
        // Cross-tenant: same id should NOT show up under a different tenant.
        assert!(!idx.contains_entry("other_tenant", id).unwrap());
    }

    #[test]
    fn tenant_entry_count_reports_index_size() {
        let tmp = tempdir().unwrap();
        let idx = build_index(tmp.path(), 4);
        assert_eq!(idx.tenant_entry_count("tenant").unwrap(), 0);
        idx.insert(Uuid::new_v4(), "tenant", &norm(vec![1.0, 0.0, 0.0, 0.0]))
            .unwrap();
        idx.insert(Uuid::new_v4(), "tenant", &norm(vec![0.0, 1.0, 0.0, 0.0]))
            .unwrap();
        assert_eq!(idx.tenant_entry_count("tenant").unwrap(), 2);
        assert_eq!(idx.tenant_entry_count("other_tenant").unwrap(), 0);
    }

    #[test]
    fn delete_tenant_index_file_wipes_disk_and_cache() {
        let tmp = tempdir().unwrap();
        let dim = 4;
        let id = Uuid::new_v4();
        let v = norm(vec![1.0, 0.0, 0.0, 0.0]);
        let idx = build_index(tmp.path(), dim);
        idx.insert(id, "tenant", &v).unwrap();
        idx.flush_all().unwrap();
        // File exists on disk now.
        assert!(idx.contains_entry("tenant", id).unwrap());

        idx.delete_tenant_index_file("tenant").unwrap();

        // After delete: the file is gone AND the in-memory handle has been
        // dropped. A fresh contains_entry / nearest call must rebuild an
        // empty index for this tenant.
        assert!(!idx.contains_entry("tenant", id).unwrap());
        let neighbors = idx.nearest("tenant", &v, 5).unwrap();
        assert!(neighbors.is_empty(), "fresh index must be empty: {neighbors:?}");

        // No-op when the file doesn't exist.
        idx.delete_tenant_index_file("never_seen_tenant").unwrap();
    }

    /// Phase A audit fix: when an LRU eviction is triggered AND the victim
    /// fails to flush to disk, the error must surface to the caller (so the
    /// upstream gate evaluation fails closed) and the victim's data must NOT
    /// be silently dropped. We simulate the flush failure on Unix by making
    /// the root directory read-only between two inserts.
    #[cfg(unix)]
    #[test]
    fn lru_eviction_with_flush_failure_surfaces_and_does_not_lose_data() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempdir().unwrap();
        let dim = 4;
        // max_open = 1 so EVERY new tenant evicts the previous one.
        // flush_every = 1024 so the inline flush isn't triggered by a single
        // insert — only the eviction-path flush is.
        let idx = UsearchVectorIndex::try_new(tmp.path(), dim, 16, 200, 50, 1, 1024).unwrap();
        let id_a = Uuid::new_v4();
        let v_a = norm(vec![1.0, 0.0, 0.0, 0.0]);
        idx.insert(id_a, "tenant_a", &v_a).unwrap();

        // Make the root dir read-only so the eviction-path flush
        // (Index::save → fs write) cannot succeed.
        let mut perms = std::fs::metadata(tmp.path()).unwrap().permissions();
        perms.set_mode(0o500);
        std::fs::set_permissions(tmp.path(), perms.clone()).unwrap();

        let id_b = Uuid::new_v4();
        let v_b = norm(vec![0.0, 1.0, 0.0, 0.0]);
        let err = idx
            .insert(id_b, "tenant_b", &v_b)
            .expect_err("eviction flush must fail under read-only root");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("eviction flush failed") || msg.contains("save"),
            "expected eviction-flush error, got: {msg}"
        );

        // Restore permissions so the tempdir Drop can clean up.
        let mut perms = std::fs::metadata(tmp.path()).unwrap().permissions();
        perms.set_mode(0o700);
        std::fs::set_permissions(tmp.path(), perms).unwrap();

        // tenant_a's data must still be queryable (re-cached after the
        // failed flush). With max_open=1 it MAY have been ejected again
        // during the secondary push, but its dirty in-memory state was
        // either re-cached or flushed to disk. After permissions restore,
        // either path resolves: cached or disk-loadable. We assert the
        // query succeeds; an empty result would indicate silent data loss.
        let neighbors_a = idx.nearest("tenant_a", &v_a, 5).unwrap();
        assert_eq!(
            neighbors_a.len(),
            1,
            "tenant_a data must not be silently lost on eviction flush failure"
        );
    }
}
