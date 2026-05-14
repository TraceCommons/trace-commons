//! Append-only JSONL sidecar writer. One line per attempted submission;
//! calibration tooling joins these by `submission_id` against
//! `trace_gate_decisions`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;

/// One sidecar row per submission attempt. Hash-only / label-only — never
/// includes raw trace bodies or bearer-token-derived fields, per the repo's
/// hash-only audit-and-logging convention.
#[derive(Debug, Clone, Serialize)]
pub struct SidecarRecord {
    pub submission_id: String,
    pub source_dataset: String,
    pub source_row_id: String,
    pub source_domain_tag: String,
    pub http_status: Option<u16>,
    /// `"accepted"`, `"quarantined"`, `"rejected"`, or `"error"` — the
    /// translator-friendly summary of the gate result.
    pub gate_decision: String,
    pub elapsed_ms: u64,
    pub timestamp: DateTime<Utc>,
}

/// Process-wide append-only JSONL writer. Flushes per record so a crash
/// preserves all already-submitted rows; idempotency handles the duplicate
/// row on restart.
pub struct Sidecar {
    path: PathBuf,
    inner: Mutex<std::fs::File>,
}

impl Sidecar {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create sidecar dir {}", parent.display()))?;
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("open sidecar {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            inner: Mutex::new(file),
        })
    }

    pub fn write(&self, record: &SidecarRecord) -> Result<()> {
        let line = serde_json::to_string(record)
            .with_context(|| "serialize sidecar record")?;
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("sidecar mutex poisoned"))?;
        guard
            .write_all(line.as_bytes())
            .with_context(|| format!("write sidecar line to {}", self.path.display()))?;
        guard
            .write_all(b"\n")
            .with_context(|| "write sidecar newline")?;
        guard
            .flush()
            .with_context(|| "flush sidecar")?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
