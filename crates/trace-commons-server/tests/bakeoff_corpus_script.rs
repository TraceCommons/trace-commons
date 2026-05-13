//! End-to-end check that the operator script's dry-run path produces a
//! tarball the bake-off corpus loader can consume.
//!
//! Shells out to `scripts/operator/build-bakeoff-corpus.sh` with
//! `BAKEOFF_CORPUS_DRY_RUN=1`, then feeds the resulting `.tar.zst`
//! through `bakeoff_corpus::load_corpus` and asserts the slice counts +
//! the pinned synthetic strings.

#[path = "../src/bin/gate_calibrate/bakeoff_corpus.rs"]
mod bakeoff_corpus;

use std::path::PathBuf;
use std::process::Command;

#[test]
fn dry_run_script_produces_loadable_corpus() {
    // Repo root is two levels up from this crate's manifest dir
    // (crates/trace-commons-server -> repo root).
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("repo root from CARGO_MANIFEST_DIR");
    let script = repo_root.join("scripts/operator/build-bakeoff-corpus.sh");
    assert!(script.exists(), "missing script: {}", script.display());

    let tmp = tempfile::tempdir().expect("tempdir");
    let out = tmp.path().join("dry.tar.zst");

    let status = Command::new("bash")
        .arg(&script)
        .arg(&out)
        .env("BAKEOFF_CORPUS_DRY_RUN", "1")
        .status()
        .expect("spawn build-bakeoff-corpus.sh");
    assert!(status.success(), "script exited non-zero: {status}");
    assert!(out.exists(), "tarball not produced");

    let corpus = bakeoff_corpus::load_corpus(&out).expect("loads dry-run corpus");
    assert_eq!(corpus.novel.len(), 2);
    assert_eq!(corpus.duplicate.len(), 2);
    assert_eq!(corpus.paraphrase.len(), 2);
    assert_eq!(corpus.paraphrase[0].original, "orig-0");
    assert_eq!(corpus.paraphrase[0].paraphrase, "para-0");
}
