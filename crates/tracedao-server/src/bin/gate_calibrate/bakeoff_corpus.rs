use std::fs;
use std::io::BufReader;
use std::path::Path;

use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // used by `load_corpus` in the gate-calibrate binary; test target only re-imports the module for unit tests of other helpers
struct TarballManifest {
    #[allow(dead_code)] // accepted for forward compat; only v1 is recognized today
    version: u32,
    novel_sha256: String,
    duplicate_sha256: String,
    paraphrase_sha256: String,
}

#[derive(Debug, Clone)]
pub struct ParaphrasePair {
    pub original: String,
    pub paraphrase: String,
}

#[derive(Debug, Clone)]
pub struct LoadedCorpus {
    pub novel: Vec<String>,
    pub duplicate: Vec<String>,
    pub paraphrase: Vec<ParaphrasePair>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)] // deserialized inside `load_corpus`; not reached from the test target
struct ParaphraseLine {
    original: String,
    paraphrase: String,
}

#[allow(dead_code)] // called by the gate-calibrate binary; test target only includes the module for other helpers
pub fn load_corpus(tarball: &Path) -> anyhow::Result<LoadedCorpus> {
    let file = fs::File::open(tarball).map_err(|e| anyhow::anyhow!("open corpus tarball: {e}"))?;
    let dec = zstd::Decoder::new(BufReader::new(file))
        .map_err(|e| anyhow::anyhow!("zstd decode: {e}"))?;
    let mut archive = tar::Archive::new(dec);

    let extract_dir = tempfile::tempdir()
        .map_err(|e| anyhow::anyhow!("create tempdir for corpus extract: {e}"))?;
    archive
        .unpack(extract_dir.path())
        .map_err(|e| anyhow::anyhow!("untar corpus: {e}"))?;

    let manifest_path = extract_dir.path().join("manifest.json");
    let manifest_raw = fs::read_to_string(&manifest_path)
        .map_err(|e| anyhow::anyhow!("read manifest.json: {e}"))?;
    let manifest: TarballManifest = serde_json::from_str(&manifest_raw)
        .map_err(|e| anyhow::anyhow!("parse manifest.json: {e}"))?;

    // Read novel + duplicate slices as ordered .txt files; check sha256.
    let (novel, novel_sha) = read_text_slice(&extract_dir.path().join("novel"))?;
    verify_sha("novel", &novel_sha, &manifest.novel_sha256)?;

    let (duplicate, dup_sha) = read_text_slice(&extract_dir.path().join("duplicate"))?;
    verify_sha("duplicate", &dup_sha, &manifest.duplicate_sha256)?;

    // Paraphrase slice is a single jsonl file; sha covers the raw bytes.
    let jsonl_path = extract_dir
        .path()
        .join("paraphrase")
        .join("paraphrase.jsonl");
    let jsonl_bytes =
        fs::read(&jsonl_path).map_err(|e| anyhow::anyhow!("read paraphrase.jsonl: {e}"))?;
    let para_sha = sha256_label(&[&jsonl_bytes]);
    verify_sha("paraphrase", &para_sha, &manifest.paraphrase_sha256)?;

    let mut paraphrase = Vec::new();
    for (lineno, line) in std::str::from_utf8(&jsonl_bytes)
        .map_err(|e| anyhow::anyhow!("paraphrase.jsonl not utf8: {e}"))?
        .lines()
        .enumerate()
    {
        if line.trim().is_empty() {
            continue;
        }
        let pl: ParaphraseLine = serde_json::from_str(line)
            .map_err(|e| anyhow::anyhow!("paraphrase.jsonl line {lineno} parse error: {e}"))?;
        paraphrase.push(ParaphrasePair {
            original: pl.original,
            paraphrase: pl.paraphrase,
        });
    }

    Ok(LoadedCorpus {
        novel,
        duplicate,
        paraphrase,
    })
}

/// Read every regular file in `dir`, sorted by filename, returning the
/// UTF-8 contents and the `sha256:<hex>` of the concatenated raw bytes.
#[allow(dead_code)] // helper for `load_corpus`; not exercised directly from the test target
fn read_text_slice(dir: &Path) -> anyhow::Result<(Vec<String>, String)> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("read slice dir {}: {e}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut bodies: Vec<Vec<u8>> = Vec::with_capacity(entries.len());
    let mut texts: Vec<String> = Vec::with_capacity(entries.len());
    for entry in &entries {
        let bytes = fs::read(entry.path())
            .map_err(|e| anyhow::anyhow!("read slice file {:?}: {e}", entry.file_name()))?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|e| anyhow::anyhow!("slice file {:?} not utf8: {e}", entry.file_name()))?;
        bodies.push(bytes);
        texts.push(text);
    }
    let refs: Vec<&[u8]> = bodies.iter().map(|b| b.as_slice()).collect();
    let sha = sha256_label(&refs);
    Ok((texts, sha))
}

#[allow(dead_code)] // helper for `load_corpus`; not exercised directly from the test target
fn sha256_label(chunks: &[&[u8]]) -> String {
    let mut h = Sha256::new();
    for c in chunks {
        h.update(c);
    }
    let digest = h.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        write!(&mut hex, "{b:02x}").unwrap();
    }
    format!("sha256:{hex}")
}

#[allow(dead_code)] // helper for `load_corpus`; not exercised directly from the test target
fn verify_sha(slice: &str, actual: &str, expected: &str) -> anyhow::Result<()> {
    if actual != expected {
        anyhow::bail!(
            "{slice} slice sha256 mismatch: manifest claims {expected}, computed {actual}"
        );
    }
    Ok(())
}
