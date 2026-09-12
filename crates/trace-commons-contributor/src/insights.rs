//! Private deterministic observations from explicitly selected files.
//!
//! No discovery, enrollment, network, or contribution path is invoked. A file
//! is a provisional session boundary, never an inferred completed task.
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_protocol::insights::{
    Coverage, EvidenceRef, INSIGHT_SCHEMA_VERSION, InsightMetric, InsightReport, MetricId,
    ProviderManifest,
};

use crate::source::SessionEventKind;

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const STORE_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Codex,
    Trajectory,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeBoundary {
    SessionProxy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalInsight {
    pub id: String,
    pub source_format: SourceFormat,
    pub boundary: EpisodeBoundary,
    pub report: InsightReport,
    /// Neither initial adapter preserves enough usage to estimate cost.
    pub estimated_cost_usd: Option<f64>,
    pub cost_unavailable_reason: String,
    /// No semantic task classification is performed in this release.
    pub task_category: Option<String>,
    /// Import snapshot time; source freshness requires explicit reimport.
    pub analyzed_at: chrono::DateTime<chrono::Utc>,
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let flags = if cfg!(target_os = "macos") {
            0x4 | 0x100
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            0x800 | 0x20000
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            0x800 | 0x8000
        } else {
            bail!("insights_platform_unsupported");
        };
        options.custom_flags(flags);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = path.components().next()
            && !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        {
            bail!("insights_source_namespace_unsupported");
        }
        options.custom_flags(0x00200000);
    }
    if !fs::symlink_metadata(path)
        .map_err(|_| anyhow!("insights_source_unreadable"))?
        .is_file()
    {
        bail!("insights_source_not_file");
    }
    let file = options
        .open(path)
        .map_err(|_| anyhow!("insights_source_unreadable"))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT: inspect the opened object as well as
        // the path preflight, as the path can change before CreateFile.
        if file.metadata()?.file_attributes() & 0x400 != 0 {
            bail!("insights_source_not_file");
        }
    }
    if !file.metadata()?.is_file() {
        bail!("insights_source_not_file");
    }
    let mut bytes = Vec::new();
    file.take(MAX_SOURCE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SOURCE_BYTES {
        bail!("insights_source_too_large");
    }
    Ok(bytes)
}

/// Analyze exactly one file. Malformed JSON and unsupported record envelopes
/// fail closed; native adapter normalization can retain unknown records as
/// opaque events, so classified activity is reported with partial coverage.
pub fn analyze_file(format: SourceFormat, path: &Path) -> Result<LocalInsight> {
    let bytes = bounded_read(path)?;
    let source_digest = digest(&bytes);
    let events = match format {
        SourceFormat::Trajectory => {
            crate::source::trajectory::parse_trajectory(&bytes)
                .map_err(|_| anyhow!("insights_invalid_trajectory"))?
                .events
        }
        SourceFormat::Codex => {
            let text =
                std::str::from_utf8(&bytes).map_err(|_| anyhow!("insights_invalid_codex"))?;
            let mut meta_count = 0;
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                let value: serde_json::Value =
                    serde_json::from_str(line).map_err(|_| anyhow!("insights_invalid_codex"))?;
                if value.get("type").and_then(|v| v.as_str()).is_none()
                    || !value.get("payload").is_some_and(|v| v.is_object())
                {
                    bail!("insights_invalid_codex");
                }
                if value["type"] == "session_meta" {
                    meta_count += 1;
                }
            }
            if meta_count != 1 {
                bail!("insights_codex_requires_one_session");
            }
            let transcript = crate::source::codex::parse_session_bytes(&bytes)
                .map_err(|_| anyhow!("insights_invalid_codex"))?;
            transcript.events
        }
    };
    if events.is_empty() {
        bail!("insights_no_normalized_events");
    }
    let id = digest(format!("{format:?}:{source_digest}").as_bytes());
    let evidence = EvidenceRef {
        id: id.clone(),
        source_digest,
    };
    let count = events.len() as u64;
    let classified = events
        .iter()
        .filter(|e| e.kind != SessionEventKind::Opaque)
        .count() as u64;
    let calls = events
        .iter()
        .filter(|e| e.kind == SessionEventKind::ToolCall)
        .count() as u64;
    let results = events
        .iter()
        .filter(|e| e.kind == SessionEventKind::ToolResult)
        .collect::<Vec<_>>();
    let observed_results = results.iter().filter(|e| e.success.is_some()).count() as u64;
    let failures = results.iter().filter(|e| e.success == Some(false)).count() as u64;
    let metric = |id, value, observed, total| InsightMetric {
        id,
        value,
        coverage: Coverage { observed, total },
        evidence_ids: vec![evidence.id.clone()],
    };
    let report = InsightReport {
        schema_version: INSIGHT_SCHEMA_VERSION,
        provider: ProviderManifest::first_party(),
        evidence: vec![evidence.clone()],
        metrics: vec![
            metric(MetricId::Sessions, Some(1), 1, 1),
            metric(MetricId::Events, Some(count), count, count),
            metric(
                MetricId::ToolCalls,
                (classified > 0).then_some(calls),
                classified,
                count,
            ),
            metric(
                MetricId::ToolFailures,
                (observed_results > 0).then_some(failures),
                observed_results,
                results.len() as u64,
            ),
            metric(MetricId::InputTokens, None, 0, 1),
            metric(MetricId::OutputTokens, None, 0, 1),
            metric(MetricId::KnownOutcomes, None, 0, 1),
        ],
    };
    report.validate_for(&ProviderManifest::first_party(), &[evidence])?;
    Ok(LocalInsight {
        id,
        source_format: format,
        boundary: EpisodeBoundary::SessionProxy,
        report,
        estimated_cost_usd: None,
        cost_unavailable_reason: "adapter_usage_unavailable".into(),
        task_category: None,
        analyzed_at: chrono::Utc::now(),
    })
}

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    /// Canonical path hashes only; never source paths or bodies.
    aliases: BTreeMap<String, String>,
    reports: BTreeMap<String, LocalInsight>,
}

/// Dedicated local directory, independent of the enrollment/config store.
/// The lock remains on a stable file while the JSON index is atomically replaced.
pub struct LocalInsightStore {
    dir: PathBuf,
}

impl LocalInsightStore {
    pub fn open(dir: &Path) -> Result<Self> {
        // The caller selects the directory, so resolve its ancestor aliases
        // (notably macOS /tmp and /var). Never follow the store leaf itself.
        reject_leaf_symlink(dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder
                .create(dir)
                .map_err(|_| anyhow!("insights_store_unavailable"))?;
            let permissions = fs::metadata(dir)?.permissions();
            if permissions.mode() & 0o077 != 0 {
                bail!("insights_store_requires_private_directory");
            }
        }
        #[cfg(not(unix))]
        fs::create_dir_all(dir).map_err(|_| anyhow!("insights_store_unavailable"))?;
        reject_leaf_symlink(dir)?;
        let dir = dir
            .canonicalize()
            .map_err(|_| anyhow!("insights_store_unavailable"))?;
        reject_symlinks(&dir)?;
        Ok(Self { dir })
    }

    fn locked(&self) -> Result<(File, Index)> {
        reject_symlinks(&self.dir)?;
        let lock_path = self.dir.join("store.lock");
        reject_symlinks(&lock_path)?;
        match fs::symlink_metadata(&lock_path) {
            Ok(metadata) if !metadata.is_file() => bail!("insights_store_lock_not_file"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("insights_store_lock_unavailable"),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock = options
            .open(lock_path)
            .map_err(|_| anyhow!("insights_store_lock_unavailable"))?;
        lock.try_lock()
            .map_err(|_| anyhow!("insights_store_busy"))?;
        let path = self.dir.join("index.json");
        reject_symlinks(&path)?;
        let index = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let bytes = bounded_read(&path)?;
                serde_json::from_slice::<Index>(&bytes)
                    .map_err(|_| anyhow!("insights_store_invalid"))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Index {
                version: STORE_VERSION,
                aliases: BTreeMap::new(),
                reports: BTreeMap::new(),
            },
            Err(_) => bail!("insights_store_unreadable"),
        };
        if index.version != STORE_VERSION {
            bail!("insights_store_version_unsupported");
        }
        for (id, insight) in &index.reports {
            if id != &insight.id || insight.report.provider != ProviderManifest::first_party() {
                bail!("insights_store_invalid");
            }
            let evidence = &insight.report.evidence;
            if evidence.len() != 1
                || evidence[0].id != *id
                || digest(
                    format!("{:?}:{}", insight.source_format, evidence[0].source_digest).as_bytes(),
                ) != *id
                || insight.estimated_cost_usd.is_some()
                || insight.task_category.is_some()
                || insight.cost_unavailable_reason != "adapter_usage_unavailable"
            {
                bail!("insights_store_invalid");
            }
            insight
                .report
                .validate_for(&ProviderManifest::first_party(), evidence)?;
            validate_local_metrics(insight)?;
        }
        if index
            .aliases
            .values()
            .any(|id| !index.reports.contains_key(id))
        {
            bail!("insights_store_invalid");
        }
        Ok((lock, index))
    }

    fn save(&self, index: &Index) -> Result<()> {
        let bytes = serde_json::to_vec(index)?;
        if bytes.len() as u64 > MAX_SOURCE_BYTES {
            bail!("insights_store_full");
        }
        crate::config::write_atomic_0600(&self.dir, &self.dir.join("index.json"), &bytes)
            .map_err(|_| anyhow!("insights_store_write_failed"))
    }

    /// Reimport replaces this path's snapshot. Identical copies share a report;
    /// a replaced report survives only while another imported alias references it.
    pub fn import(&self, format: SourceFormat, path: &Path) -> Result<LocalInsight> {
        // Analyze the selected leaf through the same no-follow reader used by
        // unsaved analysis. Canonicalization is only for deduplication identity.
        let insight = analyze_file(format, path)?;
        let canonical = path
            .canonicalize()
            .map_err(|_| anyhow!("insights_source_unreadable"))?;
        let alias = digest(canonical.as_os_str().as_encoded_bytes());
        let (_lock, mut index) = self.locked()?;
        index.aliases.insert(alias, insight.id.clone());
        index.reports.insert(insight.id.clone(), insight.clone());
        let referenced = index.aliases.values().cloned().collect::<BTreeSet<_>>();
        index.reports.retain(|id, _| referenced.contains(id));
        self.save(&index)?;
        Ok(insight)
    }

    pub fn list(&self) -> Result<Vec<LocalInsight>> {
        let (_lock, index) = self.locked()?;
        Ok(index.reports.into_values().collect())
    }

    pub fn explain(&self, id: &str) -> Result<LocalInsight> {
        let (_lock, mut index) = self.locked()?;
        index
            .reports
            .remove(id)
            .ok_or_else(|| anyhow!("insights_not_found"))
    }

    /// Removes the derived report and all imported aliases. Does not delete the
    /// user's original file. An explicit later import can analyze it again.
    pub fn delete(&self, id: &str) -> Result<bool> {
        let (_lock, mut index) = self.locked()?;
        let removed = index.reports.remove(id).is_some();
        if removed {
            index.aliases.retain(|_, value| value != id);
            self.save(&index)?;
        }
        Ok(removed)
    }
}

// A cache is not an authentication boundary, but must not silently upgrade
// unsupported observations when corrupted or written by another schema.
fn validate_local_metrics(insight: &LocalInsight) -> Result<()> {
    let metrics = &insight.report.metrics;
    if metrics.len() != 7 {
        bail!("insights_store_invalid");
    }
    let events = metrics
        .iter()
        .find(|metric| metric.id == MetricId::Events)
        .and_then(|metric| metric.value)
        .filter(|count| *count > 0)
        .ok_or_else(|| anyhow!("insights_store_invalid"))?;
    for id in [
        MetricId::Sessions,
        MetricId::Events,
        MetricId::InputTokens,
        MetricId::OutputTokens,
        MetricId::ToolCalls,
        MetricId::ToolFailures,
        MetricId::KnownOutcomes,
    ] {
        let metric = metrics
            .iter()
            .find(|metric| metric.id == id)
            .ok_or_else(|| anyhow!("insights_store_invalid"))?;
        if metric.evidence_ids != [insight.id.clone()] {
            bail!("insights_store_invalid");
        }
        let Coverage { observed, total } = metric.coverage;
        let valid = match id {
            MetricId::Sessions => metric.value == Some(1) && observed == 1 && total == 1,
            MetricId::Events => observed == events && total == events,
            MetricId::InputTokens | MetricId::OutputTokens | MetricId::KnownOutcomes => {
                metric.value.is_none() && observed == 0 && total == 1
            }
            MetricId::ToolCalls => {
                total == events && metric.value.is_none_or(|value| value <= observed)
            }
            MetricId::ToolFailures => {
                total <= events && metric.value.is_none_or(|value| value <= observed)
            }
        };
        if !valid {
            bail!("insights_store_invalid");
        }
    }
    Ok(())
}

fn reject_leaf_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("insights_store_symlink_refused")
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => bail!("insights_store_unavailable"),
    }
}

fn reject_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("insights_store_symlink_refused")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("insights_store_unavailable"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trajectory(path: &Path, suffix: &str) {
        fs::write(
            path,
            format!(
                r#"{{"role":"meta","source":"fixture"}}
{{"role":"user","timestamp":"2026-01-01T00:00:00Z","content":"secret-body-{suffix}"}}
{{"role":"assistant","timestamp":"2026-01-01T00:00:01Z","content":"secret-answer"}}
"#
            ),
        )
        .unwrap();
    }

    #[test]
    fn imports_replace_deduplicate_and_delete_without_persisting_bodies() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("private-project-a.jsonl");
        let b = root.join("private-project-b.jsonl");
        trajectory(&a, "first");
        fs::copy(&a, &b).unwrap();
        let store = LocalInsightStore::open(&root.join("insights")).unwrap();
        let first = store.import(SourceFormat::Trajectory, &a).unwrap();
        assert_eq!(
            first.id,
            store.import(SourceFormat::Trajectory, &b).unwrap().id
        );
        assert_eq!(store.list().unwrap().len(), 1);
        trajectory(&a, "second");
        let second = store.import(SourceFormat::Trajectory, &a).unwrap();
        assert_ne!(first.id, second.id);
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(store.delete(&first.id).unwrap());
        assert!(store.explain(&first.id).is_err());
        assert!(a.exists() && b.exists());
        assert_eq!(store.list().unwrap().len(), 1);
        let json = fs::read_to_string(root.join("insights/index.json")).unwrap();
        for secret in [
            "secret-body",
            "secret-answer",
            "private-project",
            "first",
            "second",
        ] {
            assert!(!json.contains(secret));
        }
        assert!(store.delete(&second.id).unwrap());
        assert!(store.list().unwrap().is_empty());
        assert!(!store.delete(&second.id).unwrap());
    }

    #[test]
    fn unknown_usage_and_outcomes_are_not_zero_or_tool_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.jsonl");
        trajectory(&path, "fixture");
        let report = analyze_file(SourceFormat::Trajectory, &path).unwrap();
        for id in [
            MetricId::InputTokens,
            MetricId::OutputTokens,
            MetricId::KnownOutcomes,
        ] {
            let metric = report.report.metrics.iter().find(|m| m.id == id).unwrap();
            assert_eq!(metric.value, None);
            assert_eq!(metric.coverage.observed, 0);
        }
        assert_eq!(report.estimated_cost_usd, None);
    }

    #[test]
    fn codex_requires_valid_single_session_without_silently_skipping_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.jsonl");
        fs::write(&path, "{\"type\":\"session_meta\",\"payload\":{}}\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fixture\"}]}}\n").unwrap();
        assert!(analyze_file(SourceFormat::Codex, &path).is_ok());
        fs::write(
            &path,
            "{\"type\":\"session_meta\",\"payload\":{}}\nmalformed\n",
        )
        .unwrap();
        assert!(analyze_file(SourceFormat::Codex, &path).is_err());
        assert!(analyze_file(SourceFormat::Trajectory, &path).is_err());
    }

    #[test]
    fn overlapping_store_writes_fail_instead_of_clobbering() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let store = LocalInsightStore::open(&root.join("store")).unwrap();
        let (_lock, _) = store.locked().unwrap();
        assert!(store.list().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_store_and_index() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        symlink(&root, root.join("link")).unwrap();
        assert!(LocalInsightStore::open(&root.join("link")).is_err());
        let store = LocalInsightStore::open(&root.join("store")).unwrap();
        fs::write(root.join("external"), "must-not-change").unwrap();
        symlink(root.join("external"), root.join("store/index.json")).unwrap();
        assert!(store.list().is_err());
        assert_eq!(
            fs::read_to_string(root.join("external")).unwrap(),
            "must-not-change"
        );
    }
    #[cfg(unix)]
    #[test]
    fn resolves_ancestor_aliases_but_refuses_store_and_source_leaves() {
        use std::os::unix::fs::symlink;
        // Keep the ordinary tempdir spelling to cover /var/folders on macOS.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("real")).unwrap();
        symlink(root.join("real"), root.join("alias")).unwrap();
        let store = LocalInsightStore::open(&root.join("alias/nested/store")).unwrap();
        assert_eq!(
            store.dir,
            root.join("real/nested/store").canonicalize().unwrap()
        );
        let source = root.join("source.jsonl");
        trajectory(&source, "fixture");
        symlink(&source, root.join("source-link")).unwrap();
        assert!(analyze_file(SourceFormat::Trajectory, &root.join("source-link")).is_err());
        assert!(
            store
                .import(SourceFormat::Trajectory, &root.join("source-link"))
                .is_err()
        );
        assert!(store.list().unwrap().is_empty());
        symlink(&store.dir, root.join("store-link")).unwrap();
        assert!(LocalInsightStore::open(&root.join("store-link")).is_err());
        // A lock leaf cannot redirect locking to another file either.
        fs::remove_file(store.dir.join("store.lock")).unwrap();
        symlink(&source, store.dir.join("store.lock")).unwrap();
        assert!(store.list().is_err());
    }
    #[test]
    fn rejects_cache_with_unsupported_or_missing_measurements() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        trajectory(&source, "fixture");
        let store = LocalInsightStore::open(&dir.path().join("store")).unwrap();
        store.import(SourceFormat::Trajectory, &source).unwrap();
        let path = store.dir.join("index.json");
        let original = fs::read(&path).unwrap();
        let mut index: Index = serde_json::from_slice(&original).unwrap();
        index
            .reports
            .values_mut()
            .next()
            .unwrap()
            .report
            .metrics
            .clear();
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
        assert!(store.list().is_err());
        let mut index: Index = serde_json::from_slice(&original).unwrap();
        let tokens = index
            .reports
            .values_mut()
            .next()
            .unwrap()
            .report
            .metrics
            .iter_mut()
            .find(|metric| metric.id == MetricId::InputTokens)
            .unwrap();
        tokens.value = Some(1);
        tokens.coverage = Coverage {
            observed: 1,
            total: 1,
        };
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
        assert!(store.list().is_err());
    }
}
