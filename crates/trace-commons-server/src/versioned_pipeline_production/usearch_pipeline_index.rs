// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The usearch-backed pipeline vector index (spec A-D6).

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::TenantStorageRef;
use trace_commons_gate_api::{
    IdentifiedIndexReader, IdentifiedIndexWriter, IndexEntryKey, IndexSnapshot, IndexUpsertResult,
    IndexWriteError, NearestNeighbor, VectorIndex, VectorIndexReader, VectorIndexWriter,
};
use trace_commons_gate_enclave::vector_index_usearch::{
    UsearchVectorIndex, UsearchVectorIndexConfig,
};
use uuid::Uuid;

use super::{
    PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL, USEARCH_PIPELINE_INDEX_READER_IDENTITY,
    USEARCH_PIPELINE_INDEX_WRITER_IDENTITY,
};

const MANIFEST_SCHEMA: &str = "trace_commons.pipeline_vector_index_manifest.v2";
const MANIFEST_SUFFIX: &str = ".manifest.jsonl";
/// The extension of the usearch implementation's per-namespace files.
const USEARCH_SUFFIX: &str = ".usearch";
/// A manifest log is compacted once it holds more than twice as many
/// records as live entries, and more than this many: each compaction writes
/// the live entries once, and at least as many appends came before it, so
/// compaction adds O(1) amortized bytes per write.
const COMPACT_MIN_RECORDS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestEntry {
    revision_id: Uuid,
    /// Binds the stored embedding to its content hash, so a second upsert
    /// under one key with either changed is a conflict.
    content_digest: String,
    content_hash: String,
}

/// The first line of a manifest log.
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestHeader {
    schema: String,
    namespace: String,
}

/// Every later line: one write, in the order the writes were made.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum ManifestRecord {
    Insert {
        entry_id: Uuid,
        revision_id: Uuid,
        content_digest: String,
        content_hash: String,
    },
    /// Written before an entry is deleted from usearch: the removal the
    /// next start may complete (see [`UsearchPipelineIndex::open`]).
    RemoveIntent {
        entry_id: Uuid,
    },
    Remove {
        entry_id: Uuid,
    },
}

impl ManifestRecord {
    fn insert(entry_id: Uuid, entry: &ManifestEntry) -> Self {
        Self::Insert {
            entry_id,
            revision_id: entry.revision_id,
            content_digest: entry.content_digest.clone(),
            content_hash: entry.content_hash.clone(),
        }
    }
}

/// One `(tenant_storage_ref, index_id)` namespace: its entries, and where
/// its manifest log stands.
#[derive(Debug, Default)]
struct Namespace {
    entries: BTreeMap<Uuid, ManifestEntry>,
    /// Bytes of the manifest log known to be complete; `0` before the
    /// header is written.
    log_len: u64,
    /// Records in the log, the header not counted.
    log_records: usize,
    /// Entries whose `RemoveIntent` is logged and whose `Remove` is not:
    /// usearch may or may not still hold them. They stay in `entries`.
    pending_removals: BTreeSet<Uuid>,
    /// A failed write could not be undone, so memory, the usearch file and
    /// the manifest may disagree. Every later write answers `Uncertain`
    /// until a restart re-reads the disk, which refuses the start if they
    /// do disagree.
    poisoned: bool,
}

/// The pipeline's vector index (spec A-D6): the legacy gate's usearch
/// implementation on its own root, held as a trait object, with a manifest
/// per namespace that supplies what usearch does not -- the real entry id
/// behind a usearch key, each entry's revision and content digest (conflict
/// detection, `exclude_revision`, `invalidate_revision`), and the snapshot
/// hash.
///
/// Every write is persisted before it returns: usearch saves the written
/// namespace only (`VectorIndex::flush_tenant`), then one record is
/// appended to that namespace's manifest log and fsynced. A namespace's
/// header is written before its first usearch save, so every usearch file
/// has a manifest. The log is compacted on open and once it is more than
/// twice its live entries, so a rebuild of N entries writes O(N) manifest
/// bytes (PR #1295 review, Major 2).
///
/// A failed write is undone (the vector taken back out of usearch, the
/// namespace saved again, a partial log record cut off) and answers
/// `Failed`, so a retry writes it again; one that cannot be undone answers
/// `Uncertain` and poisons the namespace. A removal logs its intent before
/// it deletes anything, so the next start completes a removal that stopped
/// part way. Any other disagreement -- a crash between an insert's usearch
/// save and its append, or a usearch file lost or replaced -- refuses the
/// next start (`pipeline_vector_index_manifest_mismatch`); the operator then
/// rebuilds the tenant's index into an emptied root.
///
/// Locking is per namespace: the map of namespaces is held only to find
/// one, and a namespace's lock is held across its own writes and reads, so
/// one tenant's save never holds up another tenant. Calls are synchronous;
/// the pipeline runs index calls on the blocking pool.
pub struct UsearchPipelineIndex {
    root: PathBuf,
    index: Arc<dyn VectorIndex>,
    namespaces: Mutex<BTreeMap<String, Arc<Mutex<Namespace>>>>,
    #[cfg(test)]
    pub(super) manifest_bytes_written: std::sync::atomic::AtomicU64,
    #[cfg(test)]
    pub(super) fail_next_append: std::sync::atomic::AtomicBool,
    /// Fails the next append that carries a `Remove` record, the same way.
    #[cfg(test)]
    pub(super) fail_next_remove_append: std::sync::atomic::AtomicBool,
}

fn namespace_of(tenant_storage_ref: &TenantStorageRef, index_id: &str) -> String {
    format!("pipeline:{index_id}:{}", tenant_storage_ref.as_str())
}

fn manifest_path(root: &Path, namespace: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(b"trace_commons.pipeline_vector_index_manifest_file.v1\n");
    hasher.update(namespace.as_bytes());
    root.join(format!("{:x}{MANIFEST_SUFFIX}", hasher.finalize()))
}

fn usearch_key(entry_id: Uuid) -> [u8; 8] {
    let mut prefix = [0u8; 8];
    prefix.copy_from_slice(&entry_id.as_bytes()[..8]);
    prefix
}

fn content_digest(embedding: &[f32], content_hash: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content_hash.as_bytes());
    hasher.update(b"\0");
    for value in embedding {
        hasher.update(value.to_le_bytes());
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn mismatch() -> anyhow::Error {
    anyhow::anyhow!(PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL)
}

fn encode_line<T: Serialize>(value: &T) -> anyhow::Result<Vec<u8>> {
    let mut line = serde_json::to_vec(value)?;
    line.push(b'\n');
    Ok(line)
}

fn sync_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::File::open(dir)?.sync_all()
}

/// A manifest log as read: its namespace, its entries, how many records it
/// holds and its length.
struct ManifestLog {
    namespace: String,
    entries: BTreeMap<Uuid, ManifestEntry>,
    pending_removals: BTreeSet<Uuid>,
    records: usize,
    len: u64,
}

/// Reads the log at `path`: the header, then each record applied in order.
/// A line that does not parse, a record that does not apply (an insert of
/// a present id, a remove or remove intent of an absent one), and a last
/// line without its newline (a torn append) are each a mismatch.
fn read_log(path: &Path) -> anyhow::Result<ManifestLog> {
    let bytes = std::fs::read(path).map_err(|_| mismatch())?;
    let Some((b'\n', complete)) = bytes.split_last() else {
        return Err(mismatch());
    };
    let mut lines = complete.split(|byte| *byte == b'\n');
    let header: ManifestHeader = lines
        .next()
        .and_then(|line| serde_json::from_slice(line).ok())
        .ok_or_else(mismatch)?;
    if header.schema != MANIFEST_SCHEMA {
        return Err(mismatch());
    }
    let mut entries = BTreeMap::new();
    let mut pending_removals = BTreeSet::new();
    let mut records = 0_usize;
    for line in lines {
        match serde_json::from_slice(line).map_err(|_| mismatch())? {
            ManifestRecord::Insert {
                entry_id,
                revision_id,
                content_digest,
                content_hash,
            } => {
                let entry = ManifestEntry {
                    revision_id,
                    content_digest,
                    content_hash,
                };
                if entries.insert(entry_id, entry).is_some() {
                    return Err(mismatch());
                }
            }
            ManifestRecord::RemoveIntent { entry_id } => {
                if !entries.contains_key(&entry_id) {
                    return Err(mismatch());
                }
                pending_removals.insert(entry_id);
            }
            ManifestRecord::Remove { entry_id } => {
                if entries.remove(&entry_id).is_none() {
                    return Err(mismatch());
                }
                pending_removals.remove(&entry_id);
            }
        }
        records += 1;
    }
    Ok(ManifestLog {
        namespace: header.namespace,
        entries,
        pending_removals,
        records,
        len: bytes.len() as u64,
    })
}

impl UsearchPipelineIndex {
    /// Opens (creating when absent) the index at `root`, and refuses with
    /// `pipeline_vector_index_manifest_mismatch` when any manifest there is
    /// unreadable or does not hold exactly as many entries as its usearch
    /// file.
    ///
    /// Before that count is compared, each namespace's logged removals are
    /// completed: an entry with a `RemoveIntent` and no `Remove` is deleted
    /// from usearch (a no-op when the earlier delete reached the disk), the
    /// namespace is saved, and the `Remove` records are appended and
    /// fsynced. That heals a restart between a failed invalidation and its
    /// retry (PR #1295 review round 2, Minor 2). Failing to complete them
    /// refuses the start with `pipeline_vector_index_open_failed`.
    ///
    /// Which disagreement is healed, and which still refuses:
    /// - The manifest names an entry usearch lacks. With a logged intent
    ///   this is a removal the caller asked for that stopped part way, and
    ///   completing it is what the retry would do. Without one it is not:
    ///   it is a usearch file lost, replaced by an older copy or created
    ///   empty, and dropping the entries would silently thin the tenant's
    ///   novelty corpus, so a duplicate would score as novel. Absence alone
    ///   never authorizes a removal; that case still refuses.
    /// - usearch holds an entry the manifest does not name: an insert whose
    ///   record never became durable. Its revision and content hash cannot
    ///   be recovered, so it is never adopted; that case still refuses.
    ///
    /// usearch's own save every `flush_every` writes is turned off: every
    /// write here saves its namespace explicitly, and a save inside
    /// `insert`, after the vector is added, could fail and leave a vector
    /// no manifest names (PR #1295 review, Minor 6).
    pub fn open(root: &Path, mut config: UsearchVectorIndexConfig) -> anyhow::Result<Self> {
        config.flush_every = usize::MAX;
        let usearch = UsearchVectorIndex::try_new(root, config)
            .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
        Self::open_over(root, Arc::new(usearch))
    }

    /// [`Self::open`] over `index`, which must be the usearch index on
    /// `root`.
    pub(super) fn open_over(root: &Path, index: Arc<dyn VectorIndex>) -> anyhow::Result<Self> {
        let mut usearch_files = 0_usize;
        let listing = std::fs::read_dir(root)
            .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
        let mut logs = Vec::new();
        for entry in listing {
            let path = entry
                .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?
                .path();
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if name.ends_with(USEARCH_SUFFIX) {
                usearch_files += 1;
            }
            if !name.ends_with(MANIFEST_SUFFIX) {
                continue;
            }
            let log = read_log(&path)?;
            if manifest_path(root, &log.namespace) != path {
                return Err(mismatch());
            }
            logs.push(log);
        }
        // Every usearch file this index writes has a manifest (the header
        // is written before the namespace's first save). One without holds
        // entries no manifest names.
        if usearch_files > logs.len() {
            return Err(mismatch());
        }
        let opened = Self {
            root: root.to_path_buf(),
            index,
            namespaces: Mutex::new(BTreeMap::new()),
            #[cfg(test)]
            manifest_bytes_written: std::sync::atomic::AtomicU64::new(0),
            #[cfg(test)]
            fail_next_append: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            fail_next_remove_append: std::sync::atomic::AtomicBool::new(false),
        };
        let mut namespaces = BTreeMap::new();
        for log in logs {
            let mut state = Namespace {
                entries: log.entries,
                log_len: log.len,
                log_records: log.records,
                pending_removals: log.pending_removals,
                poisoned: false,
            };
            let pending = state.pending_removals.iter().copied().collect::<Vec<_>>();
            if !pending.is_empty() {
                opened
                    .remove_logged(&log.namespace, &mut state, &pending)
                    .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
            }
            let stored = opened
                .index
                .snapshot(&log.namespace)
                .ok_or_else(mismatch)?
                .cardinality;
            if stored != state.entries.len() as u64 {
                return Err(mismatch());
            }
            if state.log_records != state.entries.len() {
                // Best effort: an uncompacted log is still a correct one.
                if opened.compact(&log.namespace, &mut state).is_err() {
                    tracing::warn!("pipeline_vector_index_compaction_failed");
                }
            }
            namespaces.insert(log.namespace, Arc::new(Mutex::new(state)));
        }
        *opened.namespaces.lock().expect("pipeline index mutex") = namespaces;
        Ok(opened)
    }

    /// The namespace's state, created empty when absent. The map's lock is
    /// released before the caller locks the namespace.
    fn namespace(&self, namespace: &str) -> Arc<Mutex<Namespace>> {
        let mut namespaces = self.namespaces.lock().expect("pipeline index mutex");
        Arc::clone(namespaces.entry(namespace.to_string()).or_default())
    }

    fn existing_namespace(&self, namespace: &str) -> Option<Arc<Mutex<Namespace>>> {
        let namespaces = self.namespaces.lock().expect("pipeline index mutex");
        namespaces.get(namespace).map(Arc::clone)
    }

    /// Writes the header of a namespace's manifest log, atomically, before
    /// the namespace's first usearch save.
    fn start_log(&self, namespace: &str, state: &mut Namespace) -> anyhow::Result<()> {
        if state.log_len > 0 {
            return Ok(());
        }
        let header = encode_line(&ManifestHeader {
            schema: MANIFEST_SCHEMA.to_string(),
            namespace: namespace.to_string(),
        })?;
        self.replace_log(namespace, &header)?;
        state.log_len = header.len() as u64;
        state.log_records = 0;
        Ok(())
    }

    /// Replaces the namespace's log with `bytes`: a staging file, fsynced,
    /// renamed over it, and the directory fsynced.
    fn replace_log(&self, namespace: &str, bytes: &[u8]) -> anyhow::Result<()> {
        let path = manifest_path(&self.root, namespace);
        let staging = path.with_extension("jsonl.staging");
        let mut file = std::fs::File::create(&staging)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&staging, &path)?;
        sync_dir(&self.root)?;
        #[cfg(test)]
        self.manifest_bytes_written
            .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    /// Appends `records` to the namespace's log and fsyncs it. On failure
    /// the log is cut back to its last complete length; the error says
    /// whether that cut succeeded (`true`: the log is as before).
    fn append(
        &self,
        namespace: &str,
        state: &mut Namespace,
        records: &[ManifestRecord],
    ) -> Result<(), bool> {
        let mut bytes = Vec::new();
        for record in records {
            bytes.extend(encode_line(record).map_err(|_| true)?);
        }
        let path = manifest_path(&self.root, namespace);
        let written = (|| -> std::io::Result<()> {
            #[cfg(test)]
            if self
                .fail_next_append
                .swap(false, std::sync::atomic::Ordering::SeqCst)
                || (records
                    .iter()
                    .any(|record| matches!(record, ManifestRecord::Remove { .. }))
                    && self
                        .fail_next_remove_append
                        .swap(false, std::sync::atomic::Ordering::SeqCst))
            {
                // A torn append: half the bytes reach the file.
                let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
                file.write_all(&bytes[..bytes.len() / 2])?;
                return Err(std::io::Error::other("injected append failure"));
            }
            let mut file = std::fs::OpenOptions::new().append(true).open(&path)?;
            file.write_all(&bytes)?;
            file.sync_all()
        })();
        match written {
            Ok(()) => {
                state.log_len += bytes.len() as u64;
                state.log_records += records.len();
                #[cfg(test)]
                self.manifest_bytes_written
                    .fetch_add(bytes.len() as u64, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
            Err(_) => Err((|| -> std::io::Result<()> {
                let file = std::fs::OpenOptions::new().write(true).open(&path)?;
                file.set_len(state.log_len)?;
                file.sync_all()
            })()
            .is_ok()),
        }
    }

    /// Rewrites the log as a header, one insert per live entry, and one
    /// remove intent per pending removal: an intent dropped here would leave
    /// a removal that reached usearch unauthorized at the next start.
    fn compact(&self, namespace: &str, state: &mut Namespace) -> anyhow::Result<()> {
        let mut bytes = encode_line(&ManifestHeader {
            schema: MANIFEST_SCHEMA.to_string(),
            namespace: namespace.to_string(),
        })?;
        for (entry_id, entry) in &state.entries {
            bytes.extend(encode_line(&ManifestRecord::insert(*entry_id, entry))?);
        }
        for entry_id in &state.pending_removals {
            bytes.extend(encode_line(&ManifestRecord::RemoveIntent {
                entry_id: *entry_id,
            })?);
        }
        self.replace_log(namespace, &bytes)?;
        state.log_len = bytes.len() as u64;
        state.log_records = state.entries.len() + state.pending_removals.len();
        Ok(())
    }

    fn compact_when_due(&self, namespace: &str, state: &mut Namespace) {
        if state.log_records > COMPACT_MIN_RECORDS.max(2 * state.entries.len())
            && self.compact(namespace, state).is_err()
        {
            // The old log is intact: the rename is the only step that
            // replaces it.
            tracing::warn!("pipeline_vector_index_compaction_failed");
        }
    }

    /// Takes `entry_id` back out of usearch after a failed write and saves
    /// the namespace, so memory and disk again hold only what the manifest
    /// names: `Failed`. When that fails too, the namespace is poisoned:
    /// `Uncertain`.
    /// Deletes `entry_ids`, whose remove intents are logged, from usearch,
    /// saves the namespace, and appends their `Remove` records; the entries
    /// leave memory only once those records are durable. A failure before
    /// any delete is `Failed`; after one, `Uncertain`, and the intents let
    /// the retry or the next start complete it.
    fn remove_logged(
        &self,
        namespace: &str,
        state: &mut Namespace,
        entry_ids: &[Uuid],
    ) -> Result<(), IndexWriteError> {
        for (removed, entry_id) in entry_ids.iter().enumerate() {
            if self.index.delete(namespace, *entry_id).is_err() {
                return Err(if removed == 0 {
                    IndexWriteError::Failed
                } else {
                    IndexWriteError::Uncertain
                });
            }
        }
        if self.index.flush_tenant(namespace).is_err() {
            return Err(IndexWriteError::Uncertain);
        }
        let records = entry_ids
            .iter()
            .map(|entry_id| ManifestRecord::Remove {
                entry_id: *entry_id,
            })
            .collect::<Vec<_>>();
        if let Err(log_restored) = self.append(namespace, state, &records) {
            if !log_restored {
                state.poisoned = true;
            }
            return Err(IndexWriteError::Uncertain);
        }
        for entry_id in entry_ids {
            state.entries.remove(entry_id);
            state.pending_removals.remove(entry_id);
        }
        Ok(())
    }

    fn undo_insert(
        &self,
        namespace: &str,
        state: &mut Namespace,
        entry_id: Uuid,
    ) -> IndexWriteError {
        if self.index.delete(namespace, entry_id).is_ok()
            && self.index.flush_tenant(namespace).is_ok()
        {
            IndexWriteError::Failed
        } else {
            state.poisoned = true;
            IndexWriteError::Uncertain
        }
    }
}

impl VectorIndexReader for UsearchPipelineIndex {
    /// The hash is SHA-256 over every entry's id and raw content hash in
    /// ascending entry-id order (the manifest's order), so it does not
    /// depend on write order.
    fn snapshot(
        &self,
        tenant_storage_ref: &TenantStorageRef,
        index_id: &str,
    ) -> anyhow::Result<IndexSnapshot> {
        let mut hasher = Sha256::new();
        let mut cardinality = 0_u64;
        if let Some(slot) = self.existing_namespace(&namespace_of(tenant_storage_ref, index_id)) {
            let state = slot.lock().expect("pipeline index namespace mutex");
            for (entry_id, entry) in &state.entries {
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
        let namespace = namespace_of(tenant_storage_ref, index_id);
        let Some(slot) = self.existing_namespace(&namespace) else {
            return Ok(Vec::new());
        };
        let state = slot.lock().expect("pipeline index namespace mutex");
        let entries = &state.entries;
        if k == 0 || entries.is_empty() {
            return Ok(Vec::new());
        }
        let excluded = exclude_revision.map_or(0, |revision| {
            entries
                .values()
                .filter(|entry| entry.revision_id == revision)
                .count()
        });
        let by_key = entries
            .iter()
            .map(|(entry_id, entry)| (usearch_key(*entry_id), (*entry_id, entry)))
            .collect::<BTreeMap<_, _>>();
        let mut neighbors = Vec::new();
        for hit in self.index.nearest(&namespace, embedding, k + excluded)? {
            let Some((entry_id, entry)) = by_key.get(&usearch_key(hit.entry_id)) else {
                continue;
            };
            if exclude_revision == Some(entry.revision_id) {
                continue;
            }
            neighbors.push(NearestNeighbor {
                entry_id: *entry_id,
                similarity: hit.similarity,
            });
            if neighbors.len() == k {
                break;
            }
        }
        Ok(neighbors)
    }
}

impl VectorIndexWriter for UsearchPipelineIndex {
    fn upsert(
        &self,
        key: &IndexEntryKey,
        embedding: &[f32],
        content_hash: &str,
    ) -> Result<IndexUpsertResult, IndexWriteError> {
        let namespace = namespace_of(&key.tenant_storage_ref, &key.index_id);
        let entry_id = key.entry_id();
        let digest = content_digest(embedding, content_hash);
        let slot = self.namespace(&namespace);
        let mut state = slot.lock().expect("pipeline index namespace mutex");
        if state.poisoned {
            return Err(IndexWriteError::Uncertain);
        }
        if let Some(existing) = state.entries.get(&entry_id) {
            return if existing.content_digest == digest {
                Ok(IndexUpsertResult::Unchanged)
            } else {
                Err(IndexWriteError::ContentConflict)
            };
        }
        // usearch keys an entry by the first eight bytes of its id; a second
        // id with the same prefix would silently replace the first, so it is
        // refused instead.
        if state
            .entries
            .keys()
            .any(|existing| usearch_key(*existing) == usearch_key(entry_id))
        {
            return Err(IndexWriteError::Failed);
        }
        if self.start_log(&namespace, &mut state).is_err() {
            return Err(IndexWriteError::Failed);
        }
        if self.index.insert(entry_id, &namespace, embedding).is_err()
            || self.index.flush_tenant(&namespace).is_err()
        {
            return Err(self.undo_insert(&namespace, &mut state, entry_id));
        }
        let entry = ManifestEntry {
            revision_id: key.revision_id,
            content_digest: digest,
            content_hash: content_hash.to_string(),
        };
        if let Err(log_restored) = self.append(
            &namespace,
            &mut state,
            &[ManifestRecord::insert(entry_id, &entry)],
        ) {
            let undone = self.undo_insert(&namespace, &mut state, entry_id);
            if !log_restored {
                state.poisoned = true;
                return Err(IndexWriteError::Uncertain);
            }
            return Err(undone);
        }
        state.entries.insert(entry_id, entry);
        self.compact_when_due(&namespace, &mut state);
        Ok(IndexUpsertResult::Inserted)
    }

    /// Logs a remove intent for each of the revision's entries (fsynced),
    /// then removes them from usearch, saves the namespace, and appends one
    /// remove record per entry; the in-memory entries go only once those
    /// records are durable. A failure part way answers `Uncertain` and
    /// leaves the entries listed. The retry completes it (removing an entry
    /// usearch no longer holds is a no-op), and so does the next start,
    /// because the intent is on disk before any delete: a save by eviction
    /// or `Drop` after the failure cannot leave a removal the start refuses.
    fn invalidate_revision(
        &self,
        tenant_storage_ref: &TenantStorageRef,
        index_id: &str,
        revision_id: Uuid,
    ) -> Result<bool, IndexWriteError> {
        let namespace = namespace_of(tenant_storage_ref, index_id);
        let Some(slot) = self.existing_namespace(&namespace) else {
            return Ok(false);
        };
        let mut state = slot.lock().expect("pipeline index namespace mutex");
        if state.poisoned {
            return Err(IndexWriteError::Uncertain);
        }
        let doomed = state
            .entries
            .iter()
            .filter(|(_, entry)| entry.revision_id == revision_id)
            .map(|(entry_id, _)| *entry_id)
            .collect::<Vec<_>>();
        if doomed.is_empty() {
            return Ok(false);
        }
        let intents = doomed
            .iter()
            .filter(|entry_id| !state.pending_removals.contains(entry_id))
            .map(|entry_id| ManifestRecord::RemoveIntent {
                entry_id: *entry_id,
            })
            .collect::<Vec<_>>();
        if !intents.is_empty() {
            if let Err(log_restored) = self.append(&namespace, &mut state, &intents) {
                // Nothing was deleted yet: with the log cut back, the call
                // changed nothing.
                if !log_restored {
                    state.poisoned = true;
                    return Err(IndexWriteError::Uncertain);
                }
                return Err(IndexWriteError::Failed);
            }
            state.pending_removals.extend(doomed.iter().copied());
        }
        self.remove_logged(&namespace, &mut state, &doomed)?;
        self.compact_when_due(&namespace, &mut state);
        Ok(true)
    }
}

impl IdentifiedIndexReader for UsearchPipelineIndex {
    fn dependency_identity(&self) -> &str {
        USEARCH_PIPELINE_INDEX_READER_IDENTITY
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

impl IdentifiedIndexWriter for UsearchPipelineIndex {
    fn dependency_identity(&self) -> &str {
        USEARCH_PIPELINE_INDEX_WRITER_IDENTITY
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

#[cfg(test)]
#[path = "usearch_pipeline_index_tests.rs"]
mod tests;
