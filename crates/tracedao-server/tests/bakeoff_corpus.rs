#[path = "../src/bin/gate_calibrate/bakeoff_corpus.rs"]
mod bakeoff_corpus;

use std::fs;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

/// Build a deterministic synthetic corpus tarball with the given slice sizes.
/// Returns the path to the produced `.tar.zst`.
fn build_synthetic_corpus(
    dir: &tempfile::TempDir,
    novel_n: usize,
    dup_n: usize,
    para_n: usize,
) -> PathBuf {
    build_synthetic_corpus_with_tamper(dir, novel_n, dup_n, para_n, |_| {})
}

/// Like `build_synthetic_corpus`, but allows mutating the novel slice's
/// on-disk bytes after the manifest sha256 has been computed. Used to
/// produce a tamper case where the manifest claims a sha256 that no longer
/// matches the bytes packed into the tarball.
fn build_synthetic_corpus_with_tamper(
    dir: &tempfile::TempDir,
    novel_n: usize,
    dup_n: usize,
    para_n: usize,
    tamper_novel: impl Fn(&mut Vec<u8>),
) -> PathBuf {
    let staging = dir.path().join("staging");
    fs::create_dir_all(staging.join("novel")).unwrap();
    fs::create_dir_all(staging.join("duplicate")).unwrap();
    fs::create_dir_all(staging.join("paraphrase")).unwrap();

    // novel slice
    let mut novel_files: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..novel_n {
        let name = format!("novel-{i:04}.txt");
        let body = format!("novel-{i}-body").into_bytes();
        fs::write(staging.join("novel").join(&name), &body).unwrap();
        novel_files.push((name, body));
    }

    // duplicate slice
    let mut dup_files: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..dup_n {
        let name = format!("dup-{i:04}.txt");
        let body = format!("dup-{i}-body").into_bytes();
        fs::write(staging.join("duplicate").join(&name), &body).unwrap();
        dup_files.push((name, body));
    }

    // paraphrase slice (single jsonl)
    let mut jsonl = String::new();
    for i in 0..para_n {
        jsonl.push_str(&format!(
            "{{\"original\":\"orig-{i}\",\"paraphrase\":\"para-{i}\"}}\n"
        ));
    }
    fs::write(
        staging.join("paraphrase").join("paraphrase.jsonl"),
        jsonl.as_bytes(),
    )
    .unwrap();

    // Compute slice sha256s in sorted-filename order. Files within a slice
    // were inserted in sorted order already by construction.
    let novel_sha = sha_of_concat(&novel_files);
    let dup_sha = sha_of_concat(&dup_files);
    let para_sha = {
        let mut h = Sha256::new();
        h.update(jsonl.as_bytes());
        format!("sha256:{}", hex::encode_lower(h.finalize()))
    };

    let manifest = serde_json::json!({
        "version": 1,
        "novel_sha256": novel_sha,
        "duplicate_sha256": dup_sha,
        "paraphrase_sha256": para_sha,
    });
    fs::write(
        staging.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    // Apply tamper to the FIRST novel file's bytes on disk after sha was
    // captured into the manifest. This way the packed tarball will hash
    // differently from what manifest.json claims.
    if novel_n > 0 {
        let first = staging.join("novel").join(&novel_files[0].0);
        let mut bytes = fs::read(&first).unwrap();
        tamper_novel(&mut bytes);
        fs::write(&first, &bytes).unwrap();
    }

    // Pack staging dir into tar.zst.
    let tarball = dir.path().join("corpus.tar.zst");
    let out = fs::File::create(&tarball).unwrap();
    let zenc = zstd::Encoder::new(out, 0).unwrap().auto_finish();
    let mut tar = tar::Builder::new(zenc);
    tar.append_dir_all(".", &staging).unwrap();
    tar.finish().unwrap();
    drop(tar);

    tarball
}

fn sha_of_concat(files: &[(String, Vec<u8>)]) -> String {
    let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (_, body) in sorted {
        h.update(body);
    }
    format!("sha256:{}", hex::encode_lower(h.finalize()))
}

// Tiny inline hex encoder so we don't pull a new dep.
mod hex {
    pub fn encode_lower(bytes: impl AsRef<[u8]>) -> String {
        let bytes = bytes.as_ref();
        let mut s = String::with_capacity(bytes.len() * 2);
        for b in bytes {
            use std::fmt::Write as _;
            write!(&mut s, "{b:02x}").unwrap();
        }
        s
    }
}

#[test]
fn loads_synthetic_corpus_three_slices() {
    let dir = tempfile::tempdir().unwrap();
    let tarball = build_synthetic_corpus(&dir, 2, 2, 2);
    let corpus = bakeoff_corpus::load_corpus(&tarball).expect("loads");
    assert_eq!(corpus.novel.len(), 2);
    assert_eq!(corpus.duplicate.len(), 2);
    assert_eq!(corpus.paraphrase.len(), 2);
    assert_eq!(corpus.paraphrase[0].original, "orig-0");
    assert_eq!(corpus.paraphrase[0].paraphrase, "para-0");
}

#[test]
fn rejects_corpus_with_mismatched_slice_sha256() {
    let dir = tempfile::tempdir().unwrap();
    let tarball = build_synthetic_corpus_with_tamper(&dir, 2, 2, 2, |b| {
        // Flip the last byte of the first novel file so the bytes on
        // disk no longer match the sha256 captured in manifest.json.
        let last = b.last_mut().expect("non-empty body");
        *last = last.wrapping_add(1);
    });
    let err = bakeoff_corpus::load_corpus(&tarball)
        .err()
        .expect("expected sha256 mismatch error");
    let msg = format!("{err:#}").to_lowercase();
    assert!(
        msg.contains("sha256") || msg.contains("mismatch"),
        "error must mention sha256 or mismatch for operator debug: {msg}"
    );
}
