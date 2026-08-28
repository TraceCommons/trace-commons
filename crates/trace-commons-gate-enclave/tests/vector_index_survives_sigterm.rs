// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Acceptance test for novelty-corpus durability: a real process, killed by a
//! real SIGTERM, must leave the corpus on disk.
//!
//! This is deliberately not a `Drop` test. SIGTERM's default disposition
//! terminates the process without unwinding, so `Drop` — and every flush that
//! hangs off it — never runs. The only thing that can save the corpus in that
//! situation is a flush that already happened, which is what the periodic
//! flusher provides.
//!
//! Shape: the parent test re-executes this same test binary with
//! `--ignored --exact <child test>`, which makes the child insert vectors,
//! announce readiness, and then idle. The parent waits past one flush
//! interval, sends SIGTERM via `kill(1)` (no new dependency), confirms the
//! child died on the signal, and reopens the index from a fresh process-local
//! instance to assert the entries survived.

#![cfg(all(unix, any(feature = "local-gpu-models", feature = "near-ai-scorer")))]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use trace_commons_gate_api::VectorIndex;
use trace_commons_gate_enclave::vector_index_usearch::{
    UsearchVectorIndex, UsearchVectorIndexConfig,
};
use uuid::Uuid;

const ROOT_ENV: &str = "TRACE_COMMONS_TEST_VECTOR_ROOT";
const CHILD_TEST: &str = "child_inserts_then_idles_until_killed";
const TENANT: &str = "tenant_sha256:sigterm-acceptance";
const DIM: usize = 4;
const ENTRIES: usize = 5;
/// Far above `ENTRIES`, so the inline `flush_every` trigger cannot be what
/// persists the corpus — only the periodic flusher can.
const FLUSH_EVERY: usize = 1024;
const FLUSH_INTERVAL: Duration = Duration::from_millis(200);

fn build_index(root: &Path, flush_interval: Option<Duration>) -> UsearchVectorIndex {
    UsearchVectorIndex::try_new(
        root,
        UsearchVectorIndexConfig {
            dim: DIM,
            hnsw_m: 16,
            ef_construction: 200,
            ef_search: 50,
            max_open: 32,
            flush_every: FLUSH_EVERY,
            flush_interval,
        },
    )
    .expect("index ctor")
}

fn unit_vector(i: usize) -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];
    v[i % DIM] = 1.0;
    v[(i + 1) % DIM] = (i as f32 + 1.0) / 100.0;
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    for x in v.iter_mut() {
        *x /= n;
    }
    v
}

#[test]
#[ignore = "child process half of vector_index_corpus_survives_sigterm"]
fn child_inserts_then_idles_until_killed() {
    let Ok(root) = std::env::var(ROOT_ENV) else {
        // Someone ran the whole suite with `--ignored`; there is no parent to
        // serve, so do nothing rather than idling forever.
        return;
    };
    let root = PathBuf::from(root);
    let index = build_index(&root, Some(FLUSH_INTERVAL));
    for i in 0..ENTRIES {
        index
            .insert(Uuid::new_v4(), TENANT, &unit_vector(i))
            .expect("insert");
    }
    std::fs::write(root.join("ready"), b"1").expect("ready marker");
    // Idle until the parent kills us. Never flush explicitly, never exit
    // cleanly: the whole point is that nothing but the periodic flusher runs.
    loop {
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn vector_index_corpus_survives_sigterm() {
    let root = std::env::temp_dir().join(format!("tc-vector-sigterm-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&root).expect("root dir");

    let mut child = Command::new(std::env::current_exe().expect("current exe"))
        .args(["--exact", CHILD_TEST, "--ignored", "--nocapture"])
        .env(ROOT_ENV, &root)
        .spawn()
        .expect("spawn child");

    let ready = root.join("ready");
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready.exists() {
        assert!(Instant::now() < deadline, "child never became ready");
        assert!(
            child.try_wait().expect("try_wait").is_none(),
            "child exited before becoming ready"
        );
        std::thread::sleep(Duration::from_millis(25));
    }

    // Let the periodic flusher run. Nothing else can write the file: there are
    // far fewer than `flush_every` writes and only one tenant, so neither the
    // inline flush nor LRU eviction fires.
    std::thread::sleep(FLUSH_INTERVAL * 5);

    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill");
    assert!(status.success(), "kill(1) failed: {status:?}");

    let exit = child.wait().expect("wait child");
    assert!(
        exit.code().is_none(),
        "child exited normally ({exit:?}); it must have died on the signal, \
         otherwise this test proves nothing about SIGTERM"
    );

    let reopened = build_index(&root, None);
    let count = reopened
        .tenant_entry_count(TENANT)
        .expect("tenant entry count");
    assert_eq!(
        count, ENTRIES,
        "corpus did not survive SIGTERM: {count} of {ENTRIES} entries on disk"
    );

    drop(reopened);
    let _ = std::fs::remove_dir_all(&root);
}
