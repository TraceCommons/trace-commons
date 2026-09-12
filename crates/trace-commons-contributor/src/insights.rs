//! Private deterministic observations from explicitly selected files.
//!
//! No discovery, enrollment, network, or contribution path is invoked. A file
//! is a provisional session boundary, never an inferred completed task.
pub mod card_presentation;
pub mod card_store;
pub mod cards;
pub mod episode_store;
pub mod episodes;
pub mod models;
pub mod outcomes;
pub mod pricing_catalog;
pub mod provider;
pub mod service;
pub mod summary;
pub mod time_evidence;
pub mod usage;
pub mod usage_evidence;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use trace_commons_protocol::insights::{
    Coverage, EvidenceRef, InsightReport, MetricId, ProviderManifest,
};

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const STORE_VERSION: u32 = 6;
const MAX_OUTCOME_LINKS: usize = 128;

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

/// A user label on a saved session proxy, not an inferred task boundary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskCategory {
    Refactor,
    Tests,
    Docs,
    Debugging,
    Other,
    Unknown,
}

/// User-reported assessment; does not establish independent task success.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskOutcome {
    Accepted,
    Partial,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationProvenance {
    UserReported,
}

/// No free text or user identity is retained. The digest binds this assessment
/// to exactly the saved source bytes, independently of the last import time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ManualAnnotation {
    pub category: TaskCategory,
    pub outcome: TaskOutcome,
    pub provenance: AnnotationProvenance,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
    pub source_digest: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeLinkProvenance {
    UserLinked,
}

/// An explicit user association, not proof that this session caused an outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutcomeLink {
    pub id: String,
    pub source_digest: String,
    pub linked_at: chrono::DateTime<chrono::Utc>,
    pub provenance: OutcomeLinkProvenance,
    pub evidence: outcomes::OutcomeEvidence,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalInsight {
    pub id: String,
    pub source_format: SourceFormat,
    pub boundary: EpisodeBoundary,
    pub report: InsightReport,
    /// Deprecated compatibility field. Estimates require separately validated
    /// usage attribution and versioned pricing; this field remains null.
    pub estimated_cost_usd: Option<f64>,
    pub cost_unavailable_reason: String,
    /// No semantic task classification is performed in this release.
    pub task_category: Option<String>,
    /// Separate user-reported evidence; never folded into observed metrics.
    #[serde(default)]
    pub manual_annotation: Option<ManualAnnotation>,
    /// Legacy snapshots remain unknown until explicit reimport.
    #[serde(default)]
    pub model_observations: Option<models::ModelObservations>,
    /// Explicit many-to-many evidence associations; no inferred success metric.
    #[serde(default)]
    pub outcome_links: Vec<OutcomeLink>,
    /// Timestamp coverage from records in the exact imported source bytes.
    /// Legacy snapshots remain unknown until explicit reimport.
    #[serde(default)]
    pub time_evidence: Option<time_evidence::RecordedTimeEvidence>,
    /// Native usage from the exact imported bytes. Legacy snapshots stay
    /// unknown until reimport; trajectory has no supported native contract.
    #[serde(default)]
    pub usage_evidence: Option<usage_evidence::PersistedUsageEvidence>,
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
    let time_evidence = time_evidence::extract_recorded_time_evidence(format, &bytes)?;
    let usage_evidence = match format {
        SourceFormat::Codex => Some(usage_evidence::extract_codex_usage_evidence(&bytes)?),
        SourceFormat::Trajectory => None,
    };
    let input = provider::ProviderInput::first_party(evidence, &events);
    let report = provider::dispatch(&provider::FirstPartyProvider, &input)?;
    Ok(LocalInsight {
        id,
        source_format: format,
        boundary: EpisodeBoundary::SessionProxy,
        report,
        estimated_cost_usd: None,
        cost_unavailable_reason: "adapter_usage_unavailable".into(),
        task_category: None,
        manual_annotation: None,
        model_observations: Some(models::extract_model_observations(format, &bytes)?),
        outcome_links: Vec::new(),
        time_evidence: Some(time_evidence),
        usage_evidence,
        analyzed_at: chrono::Utc::now(),
    })
}

/// Effects committed atomically with a snapshot mutation. Never persisted as evidence.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MutationEffects {
    pub invalidated_episode_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotMutation<T> {
    pub value: T,
    pub mutation_effects: MutationEffects,
}

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    /// Canonical path hashes only; never source paths or bodies.
    aliases: BTreeMap<String, String>,
    reports: BTreeMap<String, LocalInsight>,
    #[serde(default)]
    episodes: BTreeMap<String, episodes::LocalEpisode>,
}

/// Dedicated local directory, independent of the enrollment/config store.
/// The lock remains on a stable file while the JSON index is atomically replaced.
pub struct LocalInsightStore {
    dir: PathBuf,
    #[cfg(test)]
    fail_writes: std::sync::atomic::AtomicBool,
}

// Closing only this descriptor is insufficient when a concurrent process spawn
// inherited the same open file description. Explicit unlock ends our transaction
// independently of when a child reaches exec or exits.
struct StoreLock {
    file: File,
}
impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
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
        Ok(Self {
            dir,
            #[cfg(test)]
            fail_writes: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn locked(&self) -> Result<(StoreLock, Index)> {
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
        let lock = StoreLock { file: lock };
        let path = self.dir.join("index.json");
        reject_symlinks(&path)?;
        let mut index = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let bytes = bounded_read(&path)?;
                serde_json::from_slice::<Index>(&bytes)
                    .map_err(|_| anyhow!("insights_store_invalid"))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Index {
                version: STORE_VERSION,
                aliases: BTreeMap::new(),
                reports: BTreeMap::new(),
                episodes: BTreeMap::new(),
            },
            Err(_) => bail!("insights_store_unreadable"),
        };
        if !(1..=STORE_VERSION).contains(&index.version) {
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
            if let Some(annotation) = &insight.manual_annotation
                && (index.version == 1 || annotation.source_digest != evidence[0].source_digest)
            {
                bail!("insights_store_invalid");
            }
            insight
                .report
                .validate_for(&ProviderManifest::first_party(), evidence)?;
            validate_local_metrics(insight)?;
            if index.version < 3
                && (insight.model_observations.is_some() || !insight.outcome_links.is_empty())
            {
                bail!("insights_store_invalid");
            }
            if index.version < 5 && insight.time_evidence.is_some() {
                bail!("insights_store_invalid");
            }
            if index.version < 6 && insight.usage_evidence.is_some() {
                bail!("insights_store_invalid");
            }
            if let Some(usage) = &insight.usage_evidence {
                usage.validate_binding(insight.source_format, &evidence[0].source_digest)?;
            }
            if let Some(time_evidence) = &insight.time_evidence {
                time_evidence.validate()?;
                if time_evidence.source_digest != evidence[0].source_digest
                    || time_evidence.source_format != insight.source_format
                {
                    bail!("insights_store_invalid");
                }
            }
            if let Some(models) = &insight.model_observations {
                models.validate()?;
                if models.source_digest != evidence[0].source_digest
                    || models.source_format != insight.source_format
                {
                    bail!("insights_store_invalid");
                }
            }
            if insight.outcome_links.len() > MAX_OUTCOME_LINKS {
                bail!("insights_store_invalid");
            }
            let mut link_ids = BTreeSet::new();
            for link in &insight.outcome_links {
                link.evidence.validate()?;
                if link.source_digest != evidence[0].source_digest
                    || link.id != link.evidence.identity_digest()?
                    || !link_ids.insert(&link.id)
                {
                    bail!("insights_store_invalid");
                }
            }
        }
        if index
            .aliases
            .values()
            .any(|id| !index.reports.contains_key(id))
        {
            bail!("insights_store_invalid");
        }
        episode_store::validate_index_episodes(&index)?;
        // Legacy snapshots remain readable; the next mutation persists v5.
        index.version = STORE_VERSION;
        Ok((lock, index))
    }

    fn save(&self, index: &Index) -> Result<()> {
        let bytes = serde_json::to_vec(index)?;
        if bytes.len() as u64 > MAX_SOURCE_BYTES {
            bail!("insights_store_full");
        }
        #[cfg(test)]
        if self.fail_writes.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("insights_store_write_failed");
        }
        crate::config::write_atomic_0600(&self.dir, &self.dir.join("index.json"), &bytes)
            .map_err(|_| anyhow!("insights_store_write_failed"))
    }

    /// Reimport replaces this path's snapshot. Identical copies share a report;
    /// a replaced report survives only while another imported alias references it.
    pub fn import(&self, format: SourceFormat, path: &Path) -> Result<LocalInsight> {
        Ok(self.import_with_effects(format, path)?.value)
    }

    pub fn import_with_effects(
        &self,
        format: SourceFormat,
        path: &Path,
    ) -> Result<SnapshotMutation<LocalInsight>> {
        // Analyze the selected leaf through the same no-follow reader used by
        // unsaved analysis. Canonicalization is only for deduplication identity.
        let mut insight = analyze_file(format, path)?;
        let canonical = path
            .canonicalize()
            .map_err(|_| anyhow!("insights_source_unreadable"))?;
        let alias = digest(canonical.as_os_str().as_encoded_bytes());
        let (_lock, mut index) = self.locked()?;
        // Content-identical copies share both evidence and annotation. A new
        // digest never inherits the old snapshot's user assessment.
        if let Some(previous) = index.reports.get(&insight.id) {
            insight.manual_annotation = previous.manual_annotation.clone();
            insight.outcome_links = previous.outcome_links.clone();
        }
        index.aliases.insert(alias, insight.id.clone());
        index.reports.insert(insight.id.clone(), insight.clone());
        let referenced = index.aliases.values().cloned().collect::<BTreeSet<_>>();
        index.reports.retain(|id, _| referenced.contains(id));
        let mutation_effects = episode_store::invalidate_missing_members(&mut index);
        self.save(&index)?;
        Ok(SnapshotMutation {
            value: insight,
            mutation_effects,
        })
    }

    /// Replace the user assessment on an existing saved snapshot. Both labels
    /// are explicit, including Unknown; absence is represented by clear_annotation.
    pub fn annotate(
        &self,
        id: &str,
        category: TaskCategory,
        outcome: TaskOutcome,
    ) -> Result<LocalInsight> {
        let (_lock, mut index) = self.locked()?;
        let insight = index
            .reports
            .get_mut(id)
            .ok_or_else(|| anyhow!("insights_not_found"))?;
        insight.manual_annotation = Some(ManualAnnotation {
            category,
            outcome,
            provenance: AnnotationProvenance::UserReported,
            recorded_at: chrono::Utc::now(),
            source_digest: insight.report.evidence[0].source_digest.clone(),
        });
        let result = insight.clone();
        self.save(&index)?;
        Ok(result)
    }

    /// Clear only the assessment, preserving the snapshot and source file.
    pub fn clear_annotation(&self, id: &str) -> Result<LocalInsight> {
        let (_lock, mut index) = self.locked()?;
        let insight = index
            .reports
            .get_mut(id)
            .ok_or_else(|| anyhow!("insights_not_found"))?;
        insight.manual_annotation = None;
        let result = insight.clone();
        self.save(&index)?;
        Ok(result)
    }

    /// Associate inspected/imported evidence only after explicit user selection.
    /// Repeated associations preserve the original link time and do not duplicate.
    pub fn link_outcome(
        &self,
        id: &str,
        evidence: outcomes::OutcomeEvidence,
    ) -> Result<LocalInsight> {
        evidence.validate()?;
        let evidence_id = evidence.identity_digest()?;
        let (_lock, mut index) = self.locked()?;
        let insight = index
            .reports
            .get_mut(id)
            .ok_or_else(|| anyhow!("insights_not_found"))?;
        if !insight
            .outcome_links
            .iter()
            .any(|link| link.id == evidence_id)
        {
            if insight.outcome_links.len() >= MAX_OUTCOME_LINKS {
                bail!("insights_outcome_links_full");
            }
            insight.outcome_links.push(OutcomeLink {
                id: evidence_id,
                source_digest: insight.report.evidence[0].source_digest.clone(),
                linked_at: chrono::Utc::now(),
                provenance: OutcomeLinkProvenance::UserLinked,
                evidence,
            });
            insight.outcome_links.sort_by(|a, b| a.id.cmp(&b.id));
            let result = insight.clone();
            self.save(&index)?;
            return Ok(result);
        }
        Ok(insight.clone())
    }

    /// Remove only the selected evidence association; original artifacts remain.
    pub fn unlink_outcome(&self, id: &str, evidence_id: &str) -> Result<LocalInsight> {
        let (_lock, mut index) = self.locked()?;
        let insight = index
            .reports
            .get_mut(id)
            .ok_or_else(|| anyhow!("insights_not_found"))?;
        insight.outcome_links.retain(|link| link.id != evidence_id);
        let result = insight.clone();
        self.save(&index)?;
        Ok(result)
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
        Ok(self.delete_with_effects(id)?.value)
    }

    pub fn delete_with_effects(&self, id: &str) -> Result<SnapshotMutation<bool>> {
        let (_lock, mut index) = self.locked()?;
        let removed = index.reports.remove(id).is_some();
        let mutation_effects = episode_store::invalidate_missing_members(&mut index);
        if removed {
            index.aliases.retain(|_, value| value != id);
            self.save(&index)?;
        }
        Ok(SnapshotMutation {
            value: removed,
            mutation_effects,
        })
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

    #[cfg(unix)]
    #[test]
    fn dropping_store_guard_releases_lock_even_with_an_inherited_descriptor() {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let (guard, _) = store.locked().unwrap();
        let inherited = guard.file.try_clone().unwrap();
        let contender = OpenOptions::new()
            .read(true)
            .write(true)
            .open(store.dir.join("store.lock"))
            .unwrap();
        assert!(matches!(
            contender.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        drop(guard);
        contender
            .try_lock()
            .expect("a completed store operation must not leave a lock in an inherited descriptor");
        contender.unlock().unwrap();
        drop(inherited);
    }

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
    fn manual_annotations_follow_saved_evidence_lifecycle() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        let copy = dir.path().join("copy.jsonl");
        trajectory(&source, "first");
        let store = LocalInsightStore::open(&dir.path().join("store")).unwrap();
        let unsaved = analyze_file(SourceFormat::Trajectory, &source).unwrap();
        assert!(
            store
                .annotate(&unsaved.id, TaskCategory::Docs, TaskOutcome::Accepted)
                .is_err()
        );
        assert!(store.clear_annotation(&unsaved.id).is_err());
        let initial = store.import(SourceFormat::Trajectory, &source).unwrap();
        let annotated = store
            .annotate(&initial.id, TaskCategory::Refactor, TaskOutcome::Partial)
            .unwrap();
        let annotation = annotated.manual_annotation.clone().unwrap();
        assert_eq!(annotation.provenance, AnnotationProvenance::UserReported);
        assert_eq!(
            annotation.source_digest,
            initial.report.evidence[0].source_digest
        );
        assert!(annotation.recorded_at >= initial.analyzed_at);
        assert_eq!(
            serde_json::to_value(&annotated.report).unwrap(),
            serde_json::to_value(&initial.report).unwrap()
        );
        let reopened = LocalInsightStore::open(&dir.path().join("store")).unwrap();
        assert_eq!(
            reopened.explain(&initial.id).unwrap().manual_annotation,
            Some(annotation.clone())
        );
        assert_eq!(
            store
                .import(SourceFormat::Trajectory, &source)
                .unwrap()
                .manual_annotation,
            Some(annotation.clone())
        );
        fs::copy(&source, &copy).unwrap();
        assert_eq!(
            store
                .import(SourceFormat::Trajectory, &copy)
                .unwrap()
                .manual_annotation,
            Some(annotation.clone())
        );
        trajectory(&source, "changed");
        let changed = store.import(SourceFormat::Trajectory, &source).unwrap();
        assert_ne!(changed.id, initial.id);
        assert!(changed.manual_annotation.is_none());
        assert_eq!(
            store.explain(&initial.id).unwrap().manual_annotation,
            Some(annotation)
        );
        let updated = store
            .annotate(&initial.id, TaskCategory::Unknown, TaskOutcome::Unknown)
            .unwrap();
        assert_eq!(
            updated.manual_annotation.unwrap().outcome,
            TaskOutcome::Unknown
        );
        assert!(
            store
                .clear_annotation(&initial.id)
                .unwrap()
                .manual_annotation
                .is_none()
        );
        store
            .annotate(&initial.id, TaskCategory::Tests, TaskOutcome::Rejected)
            .unwrap();
        assert!(store.delete(&initial.id).unwrap());
        assert!(store.explain(&initial.id).is_err());
        assert!(
            store
                .import(SourceFormat::Trajectory, &copy)
                .unwrap()
                .manual_annotation
                .is_none()
        );
        assert!(source.exists() && copy.exists());
    }

    #[test]
    fn legacy_store_migrates_on_mutation_without_inventing_annotations() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        trajectory(&source, "fixture");
        let store = LocalInsightStore::open(&dir.path().join("store")).unwrap();
        let insight = store.import(SourceFormat::Trajectory, &source).unwrap();
        let path = store.dir.join("index.json");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        legacy["version"] = 1.into();
        legacy["reports"][&insight.id]
            .as_object_mut()
            .unwrap()
            .remove("manual_annotation");
        legacy["reports"][&insight.id]
            .as_object_mut()
            .unwrap()
            .remove("model_observations");
        legacy["reports"][&insight.id]
            .as_object_mut()
            .unwrap()
            .remove("outcome_links");
        legacy["reports"][&insight.id]
            .as_object_mut()
            .unwrap()
            .remove("time_evidence");
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert!(
            store
                .explain(&insight.id)
                .unwrap()
                .manual_annotation
                .is_none()
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["version"],
            1
        );
        store
            .annotate(&insight.id, TaskCategory::Debugging, TaskOutcome::Accepted)
            .unwrap();
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(migrated["version"], STORE_VERSION);
        assert_eq!(
            migrated["reports"][&insight.id]["manual_annotation"]["provenance"],
            "user_reported"
        );
    }

    #[test]
    fn invalid_annotation_cache_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.jsonl");
        trajectory(&source, "fixture");
        let store = LocalInsightStore::open(&dir.path().join("store")).unwrap();
        let insight = store.import(SourceFormat::Trajectory, &source).unwrap();
        store
            .annotate(&insight.id, TaskCategory::Other, TaskOutcome::Accepted)
            .unwrap();
        let path = store.dir.join("index.json");
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        for (field, invalid) in [
            ("source_digest", "0".repeat(64)),
            ("provenance", "verified".into()),
            ("category", "arbitrary free text".into()),
            ("outcome", "success".into()),
            ("recorded_at", "invalid".into()),
            ("comment", "raw free text".into()),
        ] {
            let mut corrupted = original.clone();
            corrupted["reports"][&insight.id]["manual_annotation"][field] = invalid.into();
            fs::write(&path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
            assert!(store.list().is_err(), "accepted invalid {field}");
        }
        let mut corrupted = original;
        corrupted["version"] = 1.into();
        fs::write(&path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
        assert!(store.list().is_err());
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

#[cfg(test)]
#[path = "insights/evidence_tests.rs"]
mod evidence_tests;

#[cfg(test)]
#[path = "insights/usage_store_tests.rs"]
mod usage_store_tests;
