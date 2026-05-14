//! HuggingFace JSONL session reader. The three pilot-bootstrap target
//! datasets (`jedisct1/agent-traces-swival`, `badlogicgames/pi-mono`,
//! `TeichAI/DeepSeek-v4-Pro-Agent`) all ship one `.jsonl` file per session,
//! not parquet. The earlier parquet-shaped row iterator was based on a
//! schema that never actually existed on disk; see
//! `docs/operator/pilot-bootstrap-dryrun-notes.md` for the post-mortem.
//!
//! This module lists `*.jsonl` siblings, downloads them through `hf-hub`'s
//! cache, and exposes them in deterministic (lexicographic) order. Each
//! file is one session = one trace submission; per-event row parsing lives
//! in `translators`.

use std::path::{Path, PathBuf};

use anyhow::Context;

/// One session file resolved on the local hf-hub cache. `local_path` is the
/// downloaded file; `sibling_name` is the HF dataset-relative filename,
/// useful as a stable `source_row_id` substrate.
#[derive(Debug, Clone)]
pub struct SessionFile {
    pub local_path: PathBuf,
    pub sibling_name: String,
}

/// Discover `.jsonl` session files for the given HF dataset id, downloading
/// missing files into the local cache. Returned in stable lexicographic
/// order so sampling at a fixed seed is reproducible across runs.
pub async fn list_jsonl_sessions(
    dataset_id: &str,
    cache_dir: Option<&Path>,
) -> anyhow::Result<Vec<SessionFile>> {
    use hf_hub::api::tokio::ApiBuilder;
    use hf_hub::{Repo, RepoType};

    // `from_env()` honors `HF_HOME` and `HF_ENDPOINT`. The latter lets the
    // operator smoke harness redirect dataset discovery at a loopback mock
    // without rebuilding; production runs leave both unset and pick up the
    // standard huggingface.co endpoint plus `~/.cache/huggingface`.
    let mut builder = ApiBuilder::from_env();
    if let Some(dir) = cache_dir {
        builder = builder.with_cache_dir(dir.to_path_buf());
    }
    let api = builder.build().with_context(|| "build hf-hub api")?;
    let repo = api.repo(Repo::new(dataset_id.to_string(), RepoType::Dataset));

    let info = repo
        .info()
        .await
        .with_context(|| format!("fetch HF dataset info for {dataset_id}"))?;

    let mut names: Vec<String> = info
        .siblings
        .into_iter()
        .map(|s| s.rfilename)
        .filter(|n| n.ends_with(".jsonl"))
        .collect();
    names.sort();
    if names.is_empty() {
        anyhow::bail!("dataset {dataset_id} exposes no .jsonl session files");
    }

    let mut sessions = Vec::with_capacity(names.len());
    for name in names {
        let local = repo
            .get(&name)
            .await
            .with_context(|| format!("download jsonl session {name}"))?;
        sessions.push(SessionFile {
            local_path: local,
            sibling_name: name,
        });
    }
    Ok(sessions)
}

/// List `.jsonl` files in a directory in deterministic lexicographic order.
/// Used by unit tests and by the smoke harness when the hf-hub cache has
/// already been primed; production runs go through `list_jsonl_sessions`.
#[allow(dead_code)]
pub fn list_local_jsonl_sessions(dir: &Path) -> anyhow::Result<Vec<SessionFile>> {
    let entries =
        std::fs::read_dir(dir).with_context(|| format!("read session dir {}", dir.display()))?;
    let mut files: Vec<SessionFile> = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| "read session dir entry")?;
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "jsonl") {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            files.push(SessionFile {
                local_path: path,
                sibling_name: name,
            });
        }
    }
    files.sort_by(|a, b| a.sibling_name.cmp(&b.sibling_name));
    Ok(files)
}

/// Read a session file as raw bytes. Translators interpret the bytes as a
/// stream of newline-delimited JSON event rows. Returns the raw bytes so
/// translators can choose how to walk them.
pub fn read_session_bytes(path: &Path) -> anyhow::Result<Vec<u8>> {
    std::fs::read(path).with_context(|| format!("read session file {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsonl_shard_discovery_returns_session_files_in_deterministic_order() {
        let tmp = tempfile::tempdir().unwrap();
        // Write in scrambled order; lister must return them sorted.
        for name in &["02.jsonl", "10.jsonl", "01.jsonl", "ignore.txt"] {
            std::fs::write(tmp.path().join(name), b"{}\n").unwrap();
        }
        let sessions = list_local_jsonl_sessions(tmp.path()).unwrap();
        let names: Vec<_> = sessions.iter().map(|s| s.sibling_name.clone()).collect();
        assert_eq!(names, vec!["01.jsonl", "02.jsonl", "10.jsonl"]);
        // Re-run yields identical order.
        let again = list_local_jsonl_sessions(tmp.path()).unwrap();
        let names_again: Vec<_> = again.iter().map(|s| s.sibling_name.clone()).collect();
        assert_eq!(names, names_again);
    }

    #[test]
    fn read_session_bytes_returns_raw_file_contents() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(tmp.path(), b"line-1\nline-2\n").unwrap();
        let bytes = read_session_bytes(tmp.path()).unwrap();
        assert_eq!(bytes, b"line-1\nline-2\n");
    }
}
