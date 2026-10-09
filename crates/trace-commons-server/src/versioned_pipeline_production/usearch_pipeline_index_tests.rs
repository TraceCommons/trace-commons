// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::versioned_pipeline_production::{
    PRODUCTION_COMPATIBILITY_INDEX_ID, PRODUCTION_COMPATIBILITY_PROJECTION_ID,
};
use trace_commons_gate_api::pipeline::TenantStorageRef;
use trace_commons_gate_api::{
    IndexEntryKey, IndexUpsertResult, IndexWriteError, VectorIndexReader, VectorIndexWriter,
};

const DIM: usize = 4;

fn usearch_test_config(
    dim: usize,
) -> trace_commons_gate_enclave::vector_index_usearch::UsearchVectorIndexConfig {
    trace_commons_gate_enclave::vector_index_usearch::UsearchVectorIndexConfig {
        dim,
        hnsw_m: 16,
        ef_construction: 64,
        ef_search: 64,
        max_open: 8,
        flush_every: 1_000,
        flush_interval: None,
    }
}

fn open(root: &std::path::Path) -> anyhow::Result<UsearchPipelineIndex> {
    UsearchPipelineIndex::open(root, usearch_test_config(DIM))
}

fn tenant(n: u8) -> TenantStorageRef {
    TenantStorageRef::new(format!("tenant_sha256:{}", format!("{n:02x}").repeat(16))).unwrap()
}

fn key(tenant_ref: &TenantStorageRef, revision: u128, chunk: u32) -> IndexEntryKey {
    IndexEntryKey {
        tenant_storage_ref: tenant_ref.clone(),
        index_id: PRODUCTION_COMPATIBILITY_INDEX_ID.to_string(),
        revision_id: Uuid::from_u128(revision),
        projection_id: PRODUCTION_COMPATIBILITY_PROJECTION_ID.to_string(),
        model_id: "BAAI/bge-large-en-v1.5".to_string(),
        chunk,
    }
}

fn unit(axis: usize) -> Vec<f32> {
    let mut vector = vec![0.0; DIM];
    vector[axis] = 1.0;
    vector
}

/// The writer contract `IsolatedPipelineIndex`'s tests cover, against
/// the usearch-backed index.
#[test]
fn usearch_pipeline_index_meets_the_writer_contract() {
    let dir = tempfile::tempdir().unwrap();
    let index = open(dir.path()).unwrap();
    let a = tenant(1);
    let b = tenant(2);
    let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;

    let first = key(&a, 1, 0);
    assert_eq!(
        index.upsert(&first, &unit(0), "sha256:one").unwrap(),
        IndexUpsertResult::Inserted
    );
    assert_eq!(
        index.upsert(&first, &unit(0), "sha256:one").unwrap(),
        IndexUpsertResult::Unchanged
    );
    assert_eq!(
        index.upsert(&first, &unit(0), "sha256:other").unwrap_err(),
        IndexWriteError::ContentConflict
    );
    assert_eq!(
        index.upsert(&first, &unit(1), "sha256:one").unwrap_err(),
        IndexWriteError::ContentConflict
    );
    index
        .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
        .unwrap();
    index
        .upsert(&key(&b, 3, 0), &unit(0), "sha256:three")
        .unwrap();

    // `nearest` answers the real entry ids, best first, within one
    // tenant, and leaves out the excluded revision.
    let nearest = index.nearest(&a, index_id, &unit(0), 2, None).unwrap();
    assert_eq!(nearest.len(), 2);
    assert_eq!(nearest[0].entry_id, first.entry_id());
    assert!((nearest[0].similarity - 1.0).abs() < 1e-4);
    assert_eq!(nearest[1].entry_id, key(&a, 2, 0).entry_id());
    let excluded = index
        .nearest(&a, index_id, &unit(0), 2, Some(Uuid::from_u128(1)))
        .unwrap();
    assert_eq!(
        excluded
            .iter()
            .map(|neighbor| neighbor.entry_id)
            .collect::<Vec<_>>(),
        vec![key(&a, 2, 0).entry_id()]
    );
    assert!(
        index
            .nearest(&a, "another_index", &unit(0), 2, None)
            .unwrap()
            .is_empty()
    );

    assert_eq!(index.snapshot(&a, index_id).unwrap().cardinality, 2);
    assert_eq!(index.snapshot(&b, index_id).unwrap().cardinality, 1);

    // `invalidate_revision` removes that revision's entries only, and is
    // idempotent.
    assert!(
        index
            .invalidate_revision(&a, index_id, Uuid::from_u128(1))
            .unwrap()
    );
    assert!(
        !index
            .invalidate_revision(&a, index_id, Uuid::from_u128(1))
            .unwrap()
    );
    assert_eq!(index.snapshot(&a, index_id).unwrap().cardinality, 1);
    assert_eq!(index.snapshot(&b, index_id).unwrap().cardinality, 1);
    let after = index.nearest(&a, index_id, &unit(0), 5, None).unwrap();
    assert_eq!(
        after
            .iter()
            .map(|neighbor| neighbor.entry_id)
            .collect::<Vec<_>>(),
        vec![key(&a, 2, 0).entry_id()]
    );
    // A removed entry may be written again.
    assert_eq!(
        index.upsert(&first, &unit(0), "sha256:one").unwrap(),
        IndexUpsertResult::Inserted
    );
}

/// The snapshot hash depends on the entries, not the order they were
/// written in.
#[test]
fn usearch_pipeline_snapshot_hash_is_order_independent() {
    let a = tenant(1);
    let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;
    let entries = [
        (key(&a, 1, 0), unit(0), "sha256:one"),
        (key(&a, 1, 1), unit(1), "sha256:two"),
        (key(&a, 2, 0), unit(2), "sha256:three"),
    ];
    let forward_dir = tempfile::tempdir().unwrap();
    let forward = open(forward_dir.path()).unwrap();
    for (entry, embedding, hash) in &entries {
        forward.upsert(entry, embedding, hash).unwrap();
    }
    let reverse_dir = tempfile::tempdir().unwrap();
    let reverse = open(reverse_dir.path()).unwrap();
    for (entry, embedding, hash) in entries.iter().rev() {
        reverse.upsert(entry, embedding, hash).unwrap();
    }
    let left = forward.snapshot(&a, index_id).unwrap();
    let right = reverse.snapshot(&a, index_id).unwrap();
    assert_eq!(left, right);
    assert_eq!(left.cardinality, 3);
    let empty = open(tempfile::tempdir().unwrap().path())
        .unwrap()
        .snapshot(&a, index_id)
        .unwrap();
    assert_ne!(empty.snapshot_hash, left.snapshot_hash);
    assert_eq!(empty.cardinality, 0);
}

/// The manifest is persisted beside the usearch files, so a reopened
/// index answers the same entries, snapshot and real entry ids.
#[test]
fn manifest_survives_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let a = tenant(1);
    let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;
    let before = {
        let index = open(dir.path()).unwrap();
        index
            .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
            .unwrap();
        index
            .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
            .unwrap();
        index.snapshot(&a, index_id).unwrap()
    };
    let reopened = open(dir.path()).unwrap();
    assert_eq!(reopened.snapshot(&a, index_id).unwrap(), before);
    assert_eq!(
        reopened
            .nearest(&a, index_id, &unit(1), 1, None)
            .unwrap()
            .first()
            .map(|neighbor| neighbor.entry_id),
        Some(key(&a, 2, 0).entry_id())
    );
    assert_eq!(
        reopened
            .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
            .unwrap(),
        IndexUpsertResult::Unchanged
    );
}

fn manifest_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.to_string_lossy().ends_with(MANIFEST_SUFFIX))
        .collect()
}

/// A manifest that does not match its usearch file refuses the start: one
/// with a record missing, one that is not a manifest, and one whose last
/// append was torn (no closing newline).
#[test]
fn truncated_manifest_refuses_open() {
    let dir = tempfile::tempdir().unwrap();
    let a = tenant(1);
    {
        let index = open(dir.path()).unwrap();
        index
            .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
            .unwrap();
        index
            .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
            .unwrap();
    }
    let manifests = manifest_files(dir.path());
    assert_eq!(manifests.len(), 1);
    let original = std::fs::read(&manifests[0]).unwrap();
    let lines = original
        .split_inclusive(|byte| *byte == b'\n')
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 3, "a header and one record per insert");

    let refuses = |bytes: Vec<u8>| {
        std::fs::write(&manifests[0], bytes).unwrap();
        assert_eq!(
            open(dir.path()).err().unwrap().to_string(),
            "pipeline_vector_index_manifest_mismatch"
        );
    };
    refuses(lines[..2].concat());
    refuses(b"{not json\n".to_vec());
    refuses(Vec::new());
    let mut torn = original.clone();
    torn.pop();
    refuses(torn);
    let mut duplicated = original.clone();
    duplicated.extend_from_slice(lines[2]);
    refuses(duplicated);

    std::fs::write(&manifests[0], &original).unwrap();
    assert_eq!(
        open(dir.path())
            .unwrap()
            .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
            .unwrap()
            .cardinality,
        2
    );
}

/// A crash between the usearch flush and the first manifest write of a
/// namespace leaves a usearch file no manifest describes; the start
/// refuses it rather than serve a namespace whose entries it cannot name.
#[test]
fn an_orphaned_usearch_file_refuses_open() {
    let dir = tempfile::tempdir().unwrap();
    let a = tenant(1);
    {
        let index = open(dir.path()).unwrap();
        index
            .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
            .unwrap();
    }
    let files = |suffix: &str| {
        std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().ends_with(suffix))
            .collect::<Vec<_>>()
    };
    assert_eq!(files(".usearch").len(), 1);
    for manifest in files(MANIFEST_SUFFIX) {
        std::fs::remove_file(manifest).unwrap();
    }
    assert_eq!(
        open(dir.path()).err().unwrap().to_string(),
        "pipeline_vector_index_manifest_mismatch"
    );
}

/// Every write persists before it returns, and returns well within the
/// 60 s index write fence margin (measured with a generous bound).
#[test]
fn flush_returns_within_the_fence_margin() {
    let dir = tempfile::tempdir().unwrap();
    let index = open(dir.path()).unwrap();
    let a = tenant(1);
    let started = std::time::Instant::now();
    for revision in 0..50 {
        index
            .upsert(
                &key(&a, revision, 0),
                &unit((revision % 4) as usize),
                "sha256:x",
            )
            .unwrap();
    }
    index
        .invalidate_revision(&a, PRODUCTION_COMPATIBILITY_INDEX_ID, Uuid::from_u128(3))
        .unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed
            < std::time::Duration::from_secs(
                crate::versioned_pipeline::PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS as u64 / 4
            ),
        "{elapsed:?}"
    );
    drop(index);
    assert_eq!(
        open(dir.path())
            .unwrap()
            .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
            .unwrap()
            .cardinality,
        49
    );
}

/// The real usearch index behind the trait-object seam, with injectable
/// faults and a record of what was flushed.
#[derive(Default)]
struct FaultyIndex {
    inner: Option<Arc<UsearchVectorIndex>>,
    /// `insert` adds the vector, then fails: usearch's own save failing
    /// after `add` (PR #1295 review, Minor 6).
    fail_insert_after_add: std::sync::atomic::AtomicBool,
    /// How many of the next `flush_tenant` calls fail.
    fail_flush_tenant: std::sync::atomic::AtomicUsize,
    fail_delete: std::sync::atomic::AtomicBool,
    flush_all_calls: std::sync::atomic::AtomicUsize,
    flushed: Mutex<Vec<String>>,
    /// While set, a `flush_tenant` of this namespace reports on the first
    /// channel and waits on the second.
    hold: Mutex<
        Option<(
            String,
            std::sync::mpsc::Sender<()>,
            std::sync::mpsc::Receiver<()>,
        )>,
    >,
}

impl FaultyIndex {
    fn over(root: &std::path::Path) -> Arc<Self> {
        let mut config = usearch_test_config(DIM);
        config.flush_every = usize::MAX;
        Arc::new(Self {
            inner: Some(Arc::new(UsearchVectorIndex::try_new(root, config).unwrap())),
            ..Self::default()
        })
    }

    fn inner(&self) -> &UsearchVectorIndex {
        self.inner.as_ref().unwrap()
    }

    fn set(flag: &std::sync::atomic::AtomicBool, value: bool) {
        flag.store(value, std::sync::atomic::Ordering::SeqCst);
    }
}

impl VectorIndex for FaultyIndex {
    fn snapshot(
        &self,
        tenant_storage_ref: &str,
    ) -> Option<trace_commons_gate_api::VectorIndexSnapshot> {
        self.inner().snapshot(tenant_storage_ref)
    }

    fn insert(
        &self,
        entry_id: Uuid,
        tenant_storage_ref: &str,
        embedding: &[f32],
    ) -> anyhow::Result<()> {
        self.inner()
            .insert(entry_id, tenant_storage_ref, embedding)?;
        anyhow::ensure!(
            !self
                .fail_insert_after_add
                .load(std::sync::atomic::Ordering::SeqCst),
            "injected save failure after add"
        );
        Ok(())
    }

    fn nearest(
        &self,
        tenant_storage_ref: &str,
        embedding: &[f32],
        k: usize,
    ) -> anyhow::Result<Vec<trace_commons_gate_api::NearestNeighbor>> {
        self.inner().nearest(tenant_storage_ref, embedding, k)
    }

    fn delete(&self, tenant_storage_ref: &str, entry_id: Uuid) -> anyhow::Result<bool> {
        anyhow::ensure!(
            !self.fail_delete.load(std::sync::atomic::Ordering::SeqCst),
            "injected delete failure"
        );
        self.inner().delete(tenant_storage_ref, entry_id)
    }

    fn flush(&self) -> anyhow::Result<()> {
        self.flush_all_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner().flush_all()
    }

    fn flush_tenant(&self, tenant_storage_ref: &str) -> anyhow::Result<()> {
        let held = {
            let mut hold = self.hold.lock().unwrap();
            match hold.as_ref() {
                Some((namespace, _, _)) if namespace == tenant_storage_ref => hold.take(),
                _ => None,
            }
        };
        if let Some((_, entered, release)) = held {
            entered.send(()).unwrap();
            release.recv().unwrap();
        }
        let failing = self
            .fail_flush_tenant
            .fetch_update(
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
                |left| left.checked_sub(1),
            )
            .is_ok();
        anyhow::ensure!(!failing, "injected save failure");
        self.flushed
            .lock()
            .unwrap()
            .push(tenant_storage_ref.to_string());
        self.inner().flush_tenant(tenant_storage_ref)
    }
}

fn open_faulty(root: &std::path::Path) -> (UsearchPipelineIndex, Arc<FaultyIndex>) {
    let faulty = FaultyIndex::over(root);
    let index = UsearchPipelineIndex::open_over(root, faulty.clone()).unwrap();
    (index, faulty)
}

fn stored(faulty: &FaultyIndex, tenant_ref: &TenantStorageRef) -> u64 {
    faulty
        .snapshot(&namespace_of(tenant_ref, PRODUCTION_COMPATIBILITY_INDEX_ID))
        .unwrap()
        .cardinality
}

/// PR #1295 review, Major 2: a write saves only the namespace it wrote,
/// never every open namespace.
#[test]
fn a_write_saves_only_its_own_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let (index, faulty) = open_faulty(dir.path());
    let a = tenant(1);
    let b = tenant(2);
    index
        .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
        .unwrap();
    index
        .upsert(&key(&b, 2, 0), &unit(1), "sha256:two")
        .unwrap();
    index
        .invalidate_revision(&a, PRODUCTION_COMPATIBILITY_INDEX_ID, Uuid::from_u128(1))
        .unwrap();
    assert_eq!(
        faulty
            .flush_all_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0
    );
    let namespace_a = namespace_of(&a, PRODUCTION_COMPATIBILITY_INDEX_ID);
    let namespace_b = namespace_of(&b, PRODUCTION_COMPATIBILITY_INDEX_ID);
    assert_eq!(
        *faulty.flushed.lock().unwrap(),
        vec![namespace_a.clone(), namespace_b, namespace_a]
    );
}

/// PR #1295 review, Major 2: one namespace's write I/O does not hold up
/// another namespace's reads and writes. A write to `a` is held inside its
/// save while `b` is written and read.
#[test]
fn a_namespace_saving_does_not_block_another() {
    let dir = tempfile::tempdir().unwrap();
    let (index, faulty) = open_faulty(dir.path());
    let index = Arc::new(index);
    let a = tenant(1);
    let b = tenant(2);
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    *faulty.hold.lock().unwrap() = Some((
        namespace_of(&a, PRODUCTION_COMPATIBILITY_INDEX_ID),
        entered_tx,
        release_rx,
    ));
    let writer = {
        let index = index.clone();
        let a = a.clone();
        std::thread::spawn(move || index.upsert(&key(&a, 1, 0), &unit(0), "sha256:one"))
    };
    entered_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the write to a reaches its save");

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    {
        let index = index.clone();
        let b = b.clone();
        std::thread::spawn(move || {
            let written = index.upsert(&key(&b, 2, 0), &unit(1), "sha256:two");
            let read = index
                .nearest(&b, PRODUCTION_COMPATIBILITY_INDEX_ID, &unit(1), 1, None)
                .map(|neighbors| neighbors.len());
            let snapshot = index
                .snapshot(&b, PRODUCTION_COMPATIBILITY_INDEX_ID)
                .map(|snapshot| snapshot.cardinality);
            done_tx.send((written, read, snapshot)).unwrap();
        });
    }
    let (written, read, snapshot) = done_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("b is served while a is saving");
    assert_eq!(written.unwrap(), IndexUpsertResult::Inserted);
    assert_eq!(read.unwrap(), 1);
    assert_eq!(snapshot.unwrap(), 1);

    release_tx.send(()).unwrap();
    assert_eq!(writer.join().unwrap().unwrap(), IndexUpsertResult::Inserted);
}

/// PR #1295 review, Major 2: the manifest is an append-only log, so a
/// rebuild of N entries writes O(N) manifest bytes, not O(N^2): the bytes
/// per entry do not grow with N.
#[test]
fn a_rebuild_writes_linear_manifest_bytes() {
    let per_entry = |count: u128| {
        let dir = tempfile::tempdir().unwrap();
        let index = open(dir.path()).unwrap();
        let a = tenant(1);
        for revision in 0..count {
            index
                .upsert(
                    &key(&a, revision, 0),
                    &unit((revision % 4) as usize),
                    "sha256:x",
                )
                .unwrap();
        }
        let bytes = index
            .manifest_bytes_written
            .load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            bytes,
            std::fs::metadata(&manifest_files(dir.path())[0])
                .unwrap()
                .len(),
            "every manifest byte written is in the file: nothing was rewritten"
        );
        bytes as f64 / count as f64
    };
    let small = per_entry(50);
    let large = per_entry(400);
    assert!(
        large < small * 1.2,
        "manifest bytes per entry grew from {small} to {large}"
    );
}

/// Removals append too, and the log is compacted once it holds more than
/// twice as many records as live entries, so it stays proportional to the
/// live entries, and a reopen reads the same entries.
#[test]
fn the_manifest_log_is_compacted() {
    let dir = tempfile::tempdir().unwrap();
    let a = tenant(1);
    let before = {
        let index = open(dir.path()).unwrap();
        for revision in 0..200 {
            index
                .upsert(
                    &key(&a, revision, 0),
                    &unit((revision % 4) as usize),
                    "sha256:x",
                )
                .unwrap();
        }
        for revision in 0..190 {
            assert!(
                index
                    .invalidate_revision(
                        &a,
                        PRODUCTION_COMPATIBILITY_INDEX_ID,
                        Uuid::from_u128(revision)
                    )
                    .unwrap()
            );
        }
        index
            .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
            .unwrap()
    };
    assert_eq!(before.cardinality, 10);
    let lines = std::fs::read(&manifest_files(dir.path())[0])
        .unwrap()
        .iter()
        .filter(|byte| **byte == b'\n')
        .count();
    assert!(
        lines <= 1 + COMPACT_MIN_RECORDS + 1,
        "{lines} manifest lines for 10 live entries"
    );
    let reopened = open(dir.path()).unwrap();
    assert_eq!(
        reopened
            .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
            .unwrap(),
        before
    );
}

/// PR #1295 review, Minor 6: an insert that fails after usearch added the
/// vector leaves no vector behind, the write is `Failed` (nothing written),
/// a retry inserts it, and the next start opens.
#[test]
fn a_failed_insert_leaves_no_orphan_vector() {
    let dir = tempfile::tempdir().unwrap();
    let a = tenant(1);
    let first = key(&a, 1, 0);
    {
        let (index, faulty) = open_faulty(dir.path());
        index
            .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
            .unwrap();
        FaultyIndex::set(&faulty.fail_insert_after_add, true);
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap_err(),
            IndexWriteError::Failed
        );
        assert_eq!(stored(&faulty, &a), 1, "the added vector was removed");
        assert_eq!(
            index
                .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
                .unwrap()
                .cardinality,
            1
        );
        FaultyIndex::set(&faulty.fail_insert_after_add, false);
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap(),
            IndexUpsertResult::Inserted
        );
    }
    let reopened = open(dir.path()).unwrap();
    assert_eq!(
        reopened
            .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
            .unwrap()
            .cardinality,
        2
    );
}

/// A save of the namespace that fails is undone the same way: `Failed`, no
/// vector left, and the retry inserts. When the undo's own save fails too,
/// the outcome is unknown: `Uncertain`.
#[test]
fn a_failed_save_is_undone() {
    let dir = tempfile::tempdir().unwrap();
    let a = tenant(1);
    let first = key(&a, 1, 0);
    {
        let (index, faulty) = open_faulty(dir.path());
        index
            .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
            .unwrap();
        faulty
            .fail_flush_tenant
            .store(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap_err(),
            IndexWriteError::Failed
        );
        assert_eq!(stored(&faulty, &a), 1, "the added vector was removed");
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap(),
            IndexUpsertResult::Inserted
        );
    }
    assert_eq!(
        open(dir.path())
            .unwrap()
            .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
            .unwrap()
            .cardinality,
        2
    );

    let dir = tempfile::tempdir().unwrap();
    let (index, faulty) = open_faulty(dir.path());
    faulty
        .fail_flush_tenant
        .store(2, std::sync::atomic::Ordering::SeqCst);
    assert_eq!(
        index.upsert(&first, &unit(0), "sha256:one").unwrap_err(),
        IndexWriteError::Uncertain
    );
}

/// PR #1295 review, the earlier minor: a manifest append that fails is
/// undone, so the write is `Failed` and a retry inserts the entry (it used
/// to answer `Uncertain`, then `Unchanged` for an entry no manifest named).
#[test]
fn a_failed_manifest_append_is_retried_not_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let a = tenant(1);
    let first = key(&a, 1, 0);
    {
        let (index, faulty) = open_faulty(dir.path());
        index
            .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
            .unwrap();
        index
            .fail_next_append
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap_err(),
            IndexWriteError::Failed
        );
        assert_eq!(stored(&faulty, &a), 1);
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap(),
            IndexUpsertResult::Inserted
        );
    }
    let reopened = open(dir.path()).unwrap();
    assert_eq!(
        reopened
            .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
            .unwrap()
            .cardinality,
        2
    );
}

/// A failed write whose undo also fails leaves memory, the usearch file and
/// the manifest possibly disagreeing: that write and every later write to
/// the namespace answer `Uncertain`, other namespaces are unaffected, and
/// the next start re-reads the disk.
#[test]
fn an_undo_that_fails_poisons_only_its_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let (index, faulty) = open_faulty(dir.path());
    let a = tenant(1);
    let b = tenant(2);
    index
        .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
        .unwrap();
    FaultyIndex::set(&faulty.fail_insert_after_add, true);
    FaultyIndex::set(&faulty.fail_delete, true);
    assert_eq!(
        index
            .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
            .unwrap_err(),
        IndexWriteError::Uncertain
    );
    FaultyIndex::set(&faulty.fail_insert_after_add, false);
    FaultyIndex::set(&faulty.fail_delete, false);
    assert_eq!(
        index
            .upsert(&key(&a, 3, 0), &unit(2), "sha256:three")
            .unwrap_err(),
        IndexWriteError::Uncertain
    );
    assert_eq!(
        index
            .invalidate_revision(&a, PRODUCTION_COMPATIBILITY_INDEX_ID, Uuid::from_u128(2))
            .unwrap_err(),
        IndexWriteError::Uncertain
    );
    assert_eq!(
        index
            .upsert(&key(&b, 4, 0), &unit(3), "sha256:four")
            .unwrap(),
        IndexUpsertResult::Inserted
    );
}

/// An invalidation whose manifest append fails answers `Uncertain` and is
/// safe to retry: the retry completes it, and the next start opens.
#[test]
fn a_failed_invalidation_is_completed_by_its_retry() {
    let dir = tempfile::tempdir().unwrap();
    let a = tenant(1);
    {
        let index = open(dir.path()).unwrap();
        index
            .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
            .unwrap();
        index
            .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
            .unwrap();
        index
            .fail_next_append
            .store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            index
                .invalidate_revision(&a, PRODUCTION_COMPATIBILITY_INDEX_ID, Uuid::from_u128(1))
                .unwrap_err(),
            IndexWriteError::Uncertain
        );
        assert!(
            index
                .invalidate_revision(&a, PRODUCTION_COMPATIBILITY_INDEX_ID, Uuid::from_u128(1))
                .unwrap()
        );
    }
    let reopened = open(dir.path()).unwrap();
    assert_eq!(
        reopened
            .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
            .unwrap()
            .cardinality,
        1
    );
}
