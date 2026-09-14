//! Private deterministic observations from explicitly selected files.
//!
//! No discovery, enrollment, network, or contribution path is invoked. A file
//! is a provisional session boundary, never an inferred completed task.
pub mod card_presentation;
pub mod card_store;
pub mod cards;
pub mod claude_task_attribution;
#[cfg(test)]
pub(crate) mod comparison_estimator;
pub(crate) mod comparison_exact;
pub mod comparison_spec_store;
pub mod comparison_specs;
pub mod comparison_task_store;
pub mod comparison_tasks;
pub mod episode_store;
pub mod episodes;
pub mod models;
pub mod outcomes;
pub mod pricing_catalog;
pub mod provider;
pub mod service;
pub mod summary;
pub mod task_attribution;
pub mod time_evidence;
pub mod usage;
pub mod usage_evidence;
#[cfg(windows)]
mod win_store_acl;

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
// Advanced to 11 for model-observation schemas 2 and 3. A nested schema bump
// without one here lets an older client accept the store and then reject
// individual snapshots, with no diagnosable event and no downgrade path.
// Advanced again to 12 for the per-address file identity an alias now records,
// which is what lets a rename drop the superseded snapshot instead of keeping
// it alive under a name the user no longer has.
const STORE_VERSION: u32 = 12;
const MAX_OUTCOME_LINKS: usize = 128;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Codex,
    ClaudeCode,
    Trajectory,
}

impl SourceFormat {
    /// The persisted identity of this format, as it appears in the preimage of
    /// every saved snapshot id. It is deliberately an explicit string rather
    /// than `Debug` output: renaming a variant would otherwise change every id
    /// the next release recomputes, and orphan every snapshot already saved.
    pub fn stable_id(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "ClaudeCode",
            Self::Trajectory => "Trajectory",
        }
    }
}

/// The preimage a saved snapshot's id is taken over. One definition, so the
/// writer and the load-time re-check can never disagree about its shape.
fn snapshot_identity(format: SourceFormat, source_digest: &str) -> String {
    digest(format!("{}:{}", format.stable_id(), source_digest).as_bytes())
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

impl TaskCategory {
    /// Every variant, in the order sibling tables (summaries, CLI parsers)
    /// should present them. A new variant must be added here or the
    /// exhaustive-match test in `insights::tests` fails to compile.
    pub const ALL: [TaskCategory; 6] = [
        TaskCategory::Refactor,
        TaskCategory::Tests,
        TaskCategory::Docs,
        TaskCategory::Debugging,
        TaskCategory::Other,
        TaskCategory::Unknown,
    ];
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

impl TaskOutcome {
    /// Every variant, in the order sibling tables (summaries, CLI parsers)
    /// should present them. A new variant must be added here or the
    /// exhaustive-match test in `insights::tests` fails to compile.
    pub const ALL: [TaskOutcome; 4] = [
        TaskOutcome::Accepted,
        TaskOutcome::Partial,
        TaskOutcome::Rejected,
        TaskOutcome::Unknown,
    ];
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
    /// Source-bound structural task attribution. Legacy and non-Codex snapshots
    /// remain unavailable until a qualified adapter produces this evidence.
    #[serde(default)]
    pub task_attribution: Option<task_attribution::CodexTaskAttributionEvidence>,
    /// Observed Claude agent-branch structure. It does not establish a whole
    /// task, human prompt, outcome, independence, or asynchronous completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_task_attribution: Option<claude_task_attribution::ClaudeTaskAttributionEvidence>,
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
        SourceFormat::ClaudeCode => crate::source::claude_code::parse_selected_file_bytes(&bytes)?,
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
    let id = snapshot_identity(format, &source_digest);
    let evidence = EvidenceRef {
        id: id.clone(),
        source_digest,
    };
    let time_evidence = match format {
        SourceFormat::ClaudeCode => None,
        SourceFormat::Codex | SourceFormat::Trajectory => Some(
            time_evidence::extract_recorded_time_evidence(format, &bytes)?,
        ),
    };
    let usage_evidence = match format {
        SourceFormat::Codex => Some(usage_evidence::extract_codex_usage_evidence(&bytes)?),
        SourceFormat::ClaudeCode | SourceFormat::Trajectory => None,
    };
    let task_attribution = match format {
        SourceFormat::Codex => Some(
            task_attribution::classify_codex_task_attribution_for_import(
                task_attribution::CodexTaskSourceProfile::CodexRustV0_154_0TaskRecords,
                &bytes,
            )?,
        ),
        SourceFormat::ClaudeCode | SourceFormat::Trajectory => None,
    };
    let claude_task_attribution = match format {
        SourceFormat::ClaudeCode => Some(
            claude_task_attribution::classify_claude_task_attribution(&bytes)?,
        ),
        SourceFormat::Codex | SourceFormat::Trajectory => None,
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
        model_observations: match format {
            SourceFormat::ClaudeCode | SourceFormat::Codex | SourceFormat::Trajectory => {
                Some(models::extract_model_observations(format, &bytes)?)
            }
        },
        outcome_links: Vec::new(),
        time_evidence,
        usage_evidence,
        task_attribution,
        claude_task_attribution,
        analyzed_at: chrono::Utc::now(),
    })
}

/// The operational outcomes a native shell has to tell apart. Each carries a
/// fixed label and nothing else: no path, no identifier, no parser detail. The
/// type is what survives the bridge -- a plain string error would be masked as
/// a generic failure there, which is how a routine lock contention came to be
/// indistinguishable from a corrupt store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsightsStoreError {
    /// No saved entry has that identifier.
    NotFound,
    /// That evidence association is not on this snapshot.
    EvidenceLinkNotFound,
    /// Another window or client holds the store. Retrying is the remedy.
    Busy,
    /// The entry cannot be read. `repair` removes exactly what is unreadable.
    Invalid,
    /// The store was written by a newer build. Nothing here can repair that,
    /// and it is not the same event as an entry this build cannot read.
    VersionUnsupported,
    /// A symlink stands where the store or its index must be.
    SymlinkRefused,
    /// The store directory is reachable by someone other than its owner.
    PrivateDirectoryRequired,
    /// Local storage could not be opened, read, or written.
    Unavailable,
}

impl std::fmt::Display for InsightsStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFound => "insights_not_found",
            Self::EvidenceLinkNotFound => "insights_evidence_link_not_found",
            Self::Busy => "insights_store_busy",
            Self::Invalid => "insights_store_invalid",
            Self::VersionUnsupported => "insights_store_version_unsupported",
            Self::SymlinkRefused => "insights_store_symlink_refused",
            Self::PrivateDirectoryRequired => "insights_store_requires_private_directory",
            Self::Unavailable => "insights_store_unavailable",
        })
    }
}
impl std::error::Error for InsightsStoreError {}

/// Effects committed atomically with a snapshot mutation. Never persisted as evidence.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MutationEffects {
    pub invalidated_episode_ids: Vec<String>,
    /// Retained tasks whose frozen episode/snapshot evidence is now stale.
    pub stale_comparison_task_ids: Vec<String>,
    /// Stable, content-free reasons for each affected retained task.
    pub stale_comparison_tasks: Vec<StaleComparisonTaskEffect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StaleComparisonTaskEffect {
    pub task_id: String,
    pub reasons: Vec<ComparisonTaskMutationReason>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonTaskMutationReason {
    EpisodeMissing,
    EpisodeRevisionChanged,
    EpisodeMembershipChanged,
    SnapshotMissingOrReplaced,
    OverlapChanged,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotMutation<T> {
    pub value: T,
    pub mutation_effects: MutationEffects,
}

/// What a saved snapshot was last imported from. The canonical path is kept
/// only as a digest, so the address cannot be read back out of the index.
/// `identity` is the same file as the operating system knows it, which is what
/// survives a rename; on platforms without a stable file identity it is absent
/// and the rename case degrades to the address alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AliasEntry {
    report_id: String,
    #[serde(default)]
    identity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Alias {
    Current(AliasEntry),
    /// Stores written before v6 recorded the report id alone.
    Legacy(String),
}

impl Alias {
    fn report_id(&self) -> &str {
        match self {
            Self::Current(entry) => &entry.report_id,
            Self::Legacy(id) => id,
        }
    }
    fn identity(&self) -> Option<&str> {
        match self {
            Self::Current(entry) => entry.identity.as_deref(),
            Self::Legacy(_) => None,
        }
    }
}

/// The file the operating system resolved, independent of the name it was
/// reached through. A rename preserves it; a copy does not.
#[cfg(unix)]
fn file_identity(path: &Path) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let metadata = fs::symlink_metadata(path).ok()?;
    Some(digest(
        format!("{}:{}", metadata.dev(), metadata.ino()).as_bytes(),
    ))
}

#[cfg(not(unix))]
fn file_identity(_path: &Path) -> Option<String> {
    None
}

/// Entries withheld from every read because they failed load-time validation.
/// Identifiers only: a quarantined entry's content is never surfaced.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct QuarantineReport {
    pub snapshot_ids: Vec<String>,
    pub episode_ids: Vec<String>,
    #[serde(default)]
    pub comparison_task_ids: Vec<String>,
    #[serde(default)]
    pub comparison_specification_ids: Vec<String>,
}

impl QuarantineReport {
    pub fn is_empty(&self) -> bool {
        self.snapshot_ids.is_empty()
            && self.episode_ids.is_empty()
            && self.comparison_task_ids.is_empty()
            && self.comparison_specification_ids.is_empty()
    }
}

/// What `repair` removed. Original files are never touched.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepairReport {
    pub quarantined: QuarantineReport,
    pub invalidated_episode_ids: Vec<String>,
    pub removed_aliases: usize,
}

/// Entries moved aside at load, kept exactly as they were written. They stay
/// on disk, so `delete` and `repair` can still remove them and an unrelated
/// mutation cannot drop them silently. They are held as raw values because an
/// entry can fail at the schema, before it has a type at all.
#[derive(Debug, Default)]
struct Quarantine {
    reports: BTreeMap<String, serde_json::Value>,
    episodes: BTreeMap<String, serde_json::Value>,
    comparison_tasks: BTreeMap<String, serde_json::Value>,
    comparison_specifications: BTreeMap<String, serde_json::Value>,
    dangling_aliases: usize,
}

impl Quarantine {
    fn report(&self) -> QuarantineReport {
        QuarantineReport {
            snapshot_ids: self.reports.keys().cloned().collect(),
            episode_ids: self.episodes.keys().cloned().collect(),
            comparison_task_ids: self.comparison_tasks.keys().cloned().collect(),
            comparison_specification_ids: self.comparison_specifications.keys().cloned().collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Index {
    version: u32,
    /// Canonical path hashes only; never source paths or bodies.
    aliases: BTreeMap<String, Alias>,
    reports: BTreeMap<String, LocalInsight>,
    #[serde(default)]
    episodes: BTreeMap<String, episodes::LocalEpisode>,
    #[serde(default)]
    comparison_tasks: BTreeMap<String, comparison_tasks::LocalComparisonTaskV1>,
    #[serde(default)]
    comparison_specifications: BTreeMap<String, comparison_specs::ComparisonSpecificationV1>,
    /// Populated at load, never persisted as its own field: the entries it
    /// holds are written back alongside the live ones.
    #[serde(skip)]
    quarantine: Quarantine,
}

/// An entry on its way back to disk: either one this store read, or one it
/// withheld and is preserving verbatim.
#[derive(Serialize)]
#[serde(untagged)]
enum Stored<'a, T: Serialize> {
    Live(&'a T),
    Quarantined(&'a serde_json::Value),
}

fn stored<'a, T: Serialize>(
    live: &'a BTreeMap<String, T>,
    quarantined: &'a BTreeMap<String, serde_json::Value>,
) -> BTreeMap<&'a String, Stored<'a, T>> {
    live.iter()
        .map(|(id, value)| (id, Stored::Live(value)))
        .chain(
            quarantined
                .iter()
                .map(|(id, value)| (id, Stored::Quarantined(value))),
        )
        .collect()
}

/// The persisted shape. Quarantined entries rejoin their live siblings here so
/// that saving an unrelated mutation cannot delete an entry the user has not
/// chosen to remove.
#[derive(Serialize)]
struct PersistedIndex<'a> {
    version: u32,
    aliases: &'a BTreeMap<String, Alias>,
    reports: BTreeMap<&'a String, Stored<'a, LocalInsight>>,
    episodes: BTreeMap<&'a String, Stored<'a, episodes::LocalEpisode>>,
    comparison_tasks: BTreeMap<&'a String, Stored<'a, comparison_tasks::LocalComparisonTaskV1>>,
    comparison_specifications:
        BTreeMap<&'a String, Stored<'a, comparison_specs::ComparisonSpecificationV1>>,
}

impl<'a> From<&'a Index> for PersistedIndex<'a> {
    fn from(index: &'a Index) -> Self {
        Self {
            version: index.version,
            aliases: &index.aliases,
            reports: stored(&index.reports, &index.quarantine.reports),
            episodes: stored(&index.episodes, &index.quarantine.episodes),
            comparison_tasks: stored(&index.comparison_tasks, &index.quarantine.comparison_tasks),
            comparison_specifications: stored(
                &index.comparison_specifications,
                &index.quarantine.comparison_specifications,
            ),
        }
    }
}

/// The index as it is read: every entry still a value, so one entry that does
/// not fit its schema cannot take the whole file with it.
#[derive(Deserialize)]
struct RawIndex {
    version: u32,
    aliases: BTreeMap<String, Alias>,
    reports: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    episodes: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    comparison_tasks: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    comparison_specifications: BTreeMap<String, serde_json::Value>,
}

impl Index {
    fn quarantine_holds_episode(&self, id: &str) -> bool {
        self.quarantine.episodes.contains_key(id)
    }
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
                .map_err(|_| anyhow!(InsightsStoreError::Unavailable))?;
            let permissions = fs::metadata(dir)?.permissions();
            if permissions.mode() & 0o077 != 0 {
                bail!(InsightsStoreError::PrivateDirectoryRequired);
            }
        }
        #[cfg(windows)]
        {
            fs::create_dir_all(dir).map_err(|_| anyhow!(InsightsStoreError::Unavailable))?;
            // Windows has no mode bits, so the equivalent of the 0700 above is
            // an owner-only protected DACL. It is applied on every open, not
            // only at creation, so a directory an older build or another tool
            // left widened cannot stay that way. Failing to apply it refuses
            // the store rather than proceeding without the control.
            win_store_acl::restrict_to_current_user(dir)?;
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = dir;
            bail!(InsightsStoreError::PrivateDirectoryRequired);
        }
        reject_leaf_symlink(dir)?;
        let dir = dir
            .canonicalize()
            .map_err(|_| anyhow!(InsightsStoreError::Unavailable))?;
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
            Ok(metadata) if !metadata.is_file() => bail!(InsightsStoreError::SymlinkRefused),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!(InsightsStoreError::Unavailable),
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
            .map_err(|_| anyhow!(InsightsStoreError::Unavailable))?;
        lock.try_lock()
            .map_err(|_| anyhow!(InsightsStoreError::Busy))?;
        let lock = StoreLock { file: lock };
        let path = self.dir.join("index.json");
        reject_symlinks(&path)?;
        let raw = match fs::symlink_metadata(&path) {
            Ok(_) => {
                let bytes = bounded_read(&path)?;
                serde_json::from_slice::<RawIndex>(&bytes)
                    .map_err(|_| anyhow!(InsightsStoreError::Invalid))?
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => RawIndex {
                version: STORE_VERSION,
                aliases: BTreeMap::new(),
                reports: BTreeMap::new(),
                episodes: BTreeMap::new(),
                comparison_tasks: BTreeMap::new(),
                comparison_specifications: BTreeMap::new(),
            },
            Err(_) => bail!(InsightsStoreError::Unavailable),
        };
        if !(1..=STORE_VERSION).contains(&raw.version) {
            bail!(InsightsStoreError::VersionUnsupported);
        }
        // One unreadable entry must not take the store with it. Both the
        // schema and the invariants are checked per entry: a failing snapshot,
        // episode, comparison task, or comparison specification is moved
        // aside, named, and withheld from every read, while the rest stay
        // fully operable and `delete` and `repair` can still remove the one
        // that failed.
        let mut index = Index {
            version: raw.version,
            aliases: raw.aliases,
            reports: BTreeMap::new(),
            episodes: BTreeMap::new(),
            comparison_tasks: BTreeMap::new(),
            comparison_specifications: BTreeMap::new(),
            quarantine: Quarantine::default(),
        };
        for (id, value) in raw.reports {
            match serde_json::from_value::<LocalInsight>(value.clone()) {
                Ok(insight) if validate_stored_report(index.version, &id, &insight).is_ok() => {
                    index.reports.insert(id, insight);
                }
                _ => {
                    index.quarantine.reports.insert(id, value);
                }
            }
        }
        // An alias naming an entry that is gone is index-level damage rather
        // than a user entry; drop it here and let `repair` persist the removal.
        let live = index
            .reports
            .keys()
            .chain(index.quarantine.reports.keys())
            .cloned()
            .collect::<BTreeSet<_>>();
        let before = index.aliases.len();
        index
            .aliases
            .retain(|_, alias| live.contains(alias.report_id()));
        index.quarantine.dangling_aliases = before - index.aliases.len();
        for (id, value) in raw.episodes {
            match serde_json::from_value::<episodes::LocalEpisode>(value.clone()) {
                Ok(episode) => {
                    index.episodes.insert(id, episode);
                }
                Err(_) => {
                    index.quarantine.episodes.insert(id, value);
                }
            }
        }
        for (id, value) in raw.comparison_tasks {
            match serde_json::from_value::<comparison_tasks::LocalComparisonTaskV1>(value.clone()) {
                Ok(task) => {
                    index.comparison_tasks.insert(id, task);
                }
                Err(_) => {
                    index.quarantine.comparison_tasks.insert(id, value);
                }
            }
        }
        for (id, value) in raw.comparison_specifications {
            match serde_json::from_value::<comparison_specs::ComparisonSpecificationV1>(
                value.clone(),
            ) {
                Ok(specification) => {
                    index.comparison_specifications.insert(id, specification);
                }
                Err(_) => {
                    index.quarantine.comparison_specifications.insert(id, value);
                }
            }
        }
        for id in episode_store::invalid_index_episodes(&index) {
            if let Some(episode) = index.episodes.remove(&id) {
                index
                    .quarantine
                    .episodes
                    .insert(id, serde_json::to_value(&episode)?);
            }
        }
        for id in comparison_task_store::invalid_index_comparison_tasks(&index) {
            if let Some(task) = index.comparison_tasks.remove(&id) {
                index
                    .quarantine
                    .comparison_tasks
                    .insert(id, serde_json::to_value(&task)?);
            }
        }
        for id in comparison_spec_store::invalid_index_comparison_specifications(&index) {
            if let Some(specification) = index.comparison_specifications.remove(&id) {
                index
                    .quarantine
                    .comparison_specifications
                    .insert(id, serde_json::to_value(&specification)?);
            }
        }
        // Legacy snapshots remain readable; the next mutation persists v12.
        index.version = STORE_VERSION;
        Ok((lock, index))
    }

    fn save(&self, index: &Index) -> Result<()> {
        let bytes = serde_json::to_vec(&PersistedIndex::from(index))?;
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
        let previous_report_ids = index.reports.keys().cloned().collect::<BTreeSet<_>>();
        // Content-identical copies share both evidence and annotation. A new
        // digest never inherits the old snapshot's user assessment.
        if let Some(previous) = index.reports.get(&insight.id) {
            insight.manual_annotation = previous.manual_annotation.clone();
            insight.outcome_links = previous.outcome_links.clone();
        }
        let identity = file_identity(&canonical);
        index.aliases.insert(
            alias.clone(),
            Alias::Current(AliasEntry {
                report_id: insight.id.clone(),
                identity: identity.clone(),
            }),
        );
        // A rename leaves the old address pointing at the snapshot it had then,
        // which would keep a superseded snapshot alive forever under a name the
        // user no longer has. The file is the same file, so drop that address.
        if let Some(identity) = identity.as_deref() {
            index
                .aliases
                .retain(|key, entry| key == &alias || entry.identity() != Some(identity));
        }
        index.reports.insert(insight.id.clone(), insight.clone());
        let referenced = index
            .aliases
            .values()
            .map(|alias| alias.report_id().to_owned())
            .collect::<BTreeSet<_>>();
        index.reports.retain(|id, _| referenced.contains(id));
        let current_report_ids = index.reports.keys().cloned().collect::<BTreeSet<_>>();
        let affected_snapshot_ids = previous_report_ids
            .symmetric_difference(&current_report_ids)
            .cloned()
            .collect();
        let mutation_effects =
            episode_store::invalidate_missing_members(&mut index, &affected_snapshot_ids);
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
            .ok_or_else(|| anyhow!(InsightsStoreError::NotFound))?;
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
            .ok_or_else(|| anyhow!(InsightsStoreError::NotFound))?;
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
        evidence.validate_fresh()?;
        let evidence_id = evidence.identity_digest()?;
        let (_lock, mut index) = self.locked()?;
        let insight = index
            .reports
            .get_mut(id)
            .ok_or_else(|| anyhow!(InsightsStoreError::NotFound))?;
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
    /// Removing nothing is not a removal: a mistyped identifier, or a link a
    /// second window already removed, must not present as a completed removal.
    pub fn unlink_outcome(&self, id: &str, evidence_id: &str) -> Result<LocalInsight> {
        let (_lock, mut index) = self.locked()?;
        let insight = index
            .reports
            .get_mut(id)
            .ok_or_else(|| anyhow!(InsightsStoreError::NotFound))?;
        let before = insight.outcome_links.len();
        insight.outcome_links.retain(|link| link.id != evidence_id);
        if insight.outcome_links.len() == before {
            bail!(InsightsStoreError::EvidenceLinkNotFound);
        }
        let result = insight.clone();
        self.save(&index)?;
        Ok(result)
    }

    pub fn list(&self) -> Result<Vec<LocalInsight>> {
        let (_lock, index) = self.locked()?;
        Ok(index.reports.into_values().collect())
    }

    /// A read. It must not mutate the index, even transiently: `remove` on a
    /// throwaway copy only happens not to persist today, and would start
    /// deleting snapshots the moment the lock plumbing saved on drop.
    pub fn explain(&self, id: &str) -> Result<LocalInsight> {
        let (_lock, index) = self.locked()?;
        if index.quarantine.reports.contains_key(id) {
            bail!(InsightsStoreError::Invalid);
        }
        index
            .reports
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!(InsightsStoreError::NotFound))
    }

    /// Identifiers of the entries this store is withholding, if any.
    pub fn quarantine(&self) -> Result<QuarantineReport> {
        let (_lock, index) = self.locked()?;
        Ok(index.quarantine.report())
    }

    /// Remove exactly the entries that failed validation, and the addresses
    /// that no longer name anything. Nothing else is touched, and no original
    /// file is read or removed.
    pub fn repair(&self) -> Result<RepairReport> {
        let (_lock, mut index) = self.locked()?;
        let quarantined = index.quarantine.report();
        let removed_aliases = index.quarantine.dangling_aliases;
        // The snapshots repair is about to drop are exactly the inputs whose
        // disappearance can leave a retained comparison task holding frozen
        // evidence that no longer resolves.
        let removed_snapshot_ids = index
            .quarantine
            .reports
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        index.quarantine.reports.clear();
        index.quarantine.episodes.clear();
        index.quarantine.comparison_tasks.clear();
        index.quarantine.comparison_specifications.clear();
        index
            .aliases
            .retain(|_, alias| index.reports.contains_key(alias.report_id()));
        let effects = episode_store::invalidate_missing_members(&mut index, &removed_snapshot_ids);
        self.save(&index)?;
        Ok(RepairReport {
            quarantined,
            invalidated_episode_ids: effects.invalidated_episode_ids,
            removed_aliases,
        })
    }

    /// Removes the derived report and all imported aliases. Does not delete the
    /// user's original file. An explicit later import can analyze it again.
    pub fn delete(&self, id: &str) -> Result<bool> {
        Ok(self.delete_with_effects(id)?.value)
    }

    pub fn delete_with_effects(&self, id: &str) -> Result<SnapshotMutation<bool>> {
        let (_lock, mut index) = self.locked()?;
        let removed =
            index.reports.remove(id).is_some() | index.quarantine.reports.remove(id).is_some();
        let affected_snapshot_ids = if removed {
            BTreeSet::from([id.to_owned()])
        } else {
            BTreeSet::new()
        };
        let mutation_effects =
            episode_store::invalidate_missing_members(&mut index, &affected_snapshot_ids);
        if removed {
            index.aliases.retain(|_, alias| alias.report_id() != id);
            self.save(&index)?;
        }
        Ok(SnapshotMutation {
            value: removed,
            mutation_effects,
        })
    }
}

/// Everything a stored snapshot must satisfy to be readable. Structural only:
/// nothing here consults the wall clock, so a correct entry cannot become
/// invalid between two reads.
fn validate_stored_report(version: u32, id: &str, insight: &LocalInsight) -> Result<()> {
    if id != insight.id || insight.report.provider != ProviderManifest::first_party() {
        bail!(InsightsStoreError::Invalid);
    }
    let evidence = &insight.report.evidence;
    let Some(reference) = evidence.first() else {
        bail!(InsightsStoreError::Invalid);
    };
    if evidence.len() != 1
        || reference.id != *id
        || snapshot_identity(insight.source_format, &reference.source_digest) != *id
        || insight.estimated_cost_usd.is_some()
        || insight.task_category.is_some()
        || insight.cost_unavailable_reason != "adapter_usage_unavailable"
    {
        bail!(InsightsStoreError::Invalid);
    }
    if let Some(annotation) = &insight.manual_annotation
        && (version == 1 || annotation.source_digest != reference.source_digest)
    {
        bail!(InsightsStoreError::Invalid);
    }
    insight
        .report
        .validate_for(&ProviderManifest::first_party(), evidence)?;
    validate_local_metrics(insight)?;
    if version < 3 && (insight.model_observations.is_some() || !insight.outcome_links.is_empty()) {
        bail!(InsightsStoreError::Invalid);
    }
    if version < 5 && insight.time_evidence.is_some() {
        bail!(InsightsStoreError::Invalid);
    }
    if version < 6 && insight.usage_evidence.is_some() {
        bail!(InsightsStoreError::Invalid);
    }
    if version < 9 && insight.task_attribution.is_some() {
        bail!(InsightsStoreError::Invalid);
    }
    if version < 10 && insight.claude_task_attribution.is_some() {
        bail!(InsightsStoreError::Invalid);
    }
    if version < models::MODEL_OBSERVATIONS_STORE_VERSION_FLOOR
        && insight
            .model_observations
            .as_ref()
            .is_some_and(models::ModelObservations::requires_store_version_floor)
    {
        bail!(InsightsStoreError::Invalid);
    }
    if let Some(usage) = &insight.usage_evidence {
        usage.validate_binding(insight.source_format, &reference.source_digest)?;
    }
    if let Some(attribution) = &insight.task_attribution {
        attribution.validate()?;
        if insight.source_format != SourceFormat::Codex
            || attribution.source_digest != reference.source_digest
        {
            bail!(InsightsStoreError::Invalid);
        }
    }
    if let Some(attribution) = &insight.claude_task_attribution {
        attribution.validate()?;
        if insight.source_format != SourceFormat::ClaudeCode
            || attribution.source_digest != reference.source_digest
        {
            bail!(InsightsStoreError::Invalid);
        }
    }
    if let Some(time_evidence) = &insight.time_evidence {
        time_evidence.validate()?;
        if time_evidence.source_digest != reference.source_digest
            || time_evidence.source_format != insight.source_format
        {
            bail!(InsightsStoreError::Invalid);
        }
    }
    if let Some(models) = &insight.model_observations {
        models.validate()?;
        if models.source_digest != reference.source_digest
            || models.source_format != insight.source_format
        {
            bail!(InsightsStoreError::Invalid);
        }
    }
    if insight.outcome_links.len() > MAX_OUTCOME_LINKS {
        bail!(InsightsStoreError::Invalid);
    }
    let mut link_ids = BTreeSet::new();
    for link in &insight.outcome_links {
        link.evidence.validate()?;
        if link.source_digest != reference.source_digest
            || link.id != link.evidence.identity_digest()?
            || !link_ids.insert(&link.id)
        {
            bail!(InsightsStoreError::Invalid);
        }
    }
    Ok(())
}

// A cache is not an authentication boundary, but must not silently upgrade
// unsupported observations when corrupted or written by another schema.
fn validate_local_metrics(insight: &LocalInsight) -> Result<()> {
    let metrics = &insight.report.metrics;
    if metrics.len() != 7 {
        bail!(InsightsStoreError::Invalid);
    }
    let events = metrics
        .iter()
        .find(|metric| metric.id == MetricId::Events)
        .and_then(|metric| metric.value)
        .filter(|count| *count > 0)
        .ok_or_else(|| anyhow!(InsightsStoreError::Invalid))?;
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
            .ok_or_else(|| anyhow!(InsightsStoreError::Invalid))?;
        if metric.evidence_ids != [insight.id.clone()] {
            bail!(InsightsStoreError::Invalid);
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
            bail!(InsightsStoreError::Invalid);
        }
    }
    Ok(())
}

fn reject_leaf_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(InsightsStoreError::SymlinkRefused)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => bail!(InsightsStoreError::Unavailable),
    }
}

fn reject_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!(InsightsStoreError::SymlinkRefused)
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!(InsightsStoreError::Unavailable),
        }
    }
    Ok(())
}

/// A corrupted entry must cost the caller that entry and nothing else: the
/// store still reads, the entry is withheld, and it is named so the user can
/// remove it.
#[cfg(test)]
fn assert_quarantined(store: &LocalInsightStore, id: &str, context: &str) {
    let listed = store
        .list()
        .unwrap_or_else(|error| panic!("{context}: one bad entry failed the whole store: {error}"));
    assert!(
        !listed.iter().any(|insight| insight.id == id),
        "{context}: a quarantined entry was listed"
    );
    assert!(
        store
            .quarantine()
            .unwrap()
            .snapshot_ids
            .iter()
            .any(|held| held == id),
        "{context}: the entry was accepted instead of quarantined"
    );
    assert_eq!(
        store.explain(id).unwrap_err().to_string(),
        "insights_store_invalid",
        "{context}: explain must name the entry as unreadable"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Adding a `TaskCategory` variant without updating this match is a
    /// compile error, and adding it here without also listing it in `ALL`
    /// fails this assertion. Both must move together.
    #[test]
    fn task_category_all_covers_every_variant_exhaustively() {
        fn ordinal(value: TaskCategory) -> usize {
            match value {
                TaskCategory::Refactor => 0,
                TaskCategory::Tests => 1,
                TaskCategory::Docs => 2,
                TaskCategory::Debugging => 3,
                TaskCategory::Other => 4,
                TaskCategory::Unknown => 5,
            }
        }
        assert_eq!(TaskCategory::ALL.len(), 6);
        for (index, value) in TaskCategory::ALL.iter().enumerate() {
            assert_eq!(ordinal(*value), index);
        }
    }

    /// Same guarantee as above, for `TaskOutcome`.
    #[test]
    fn task_outcome_all_covers_every_variant_exhaustively() {
        fn ordinal(value: TaskOutcome) -> usize {
            match value {
                TaskOutcome::Accepted => 0,
                TaskOutcome::Partial => 1,
                TaskOutcome::Rejected => 2,
                TaskOutcome::Unknown => 3,
            }
        }
        assert_eq!(TaskOutcome::ALL.len(), 4);
        for (index, value) in TaskOutcome::ALL.iter().enumerate() {
            assert_eq!(ordinal(*value), index);
        }
    }

    /// The CLI's hand-written `value_parser` string lists
    /// (`contributor_cli/insights.rs`) must accept exactly the wire form of
    /// every variant. This does not touch the CLI directly (it lives in a
    /// separate binary crate) but pins the wire strings the CLI list is
    /// required to mirror, so a diff here is a signal to update it too.
    #[test]
    fn task_category_and_outcome_wire_forms_match_the_cli_value_parser_lists() {
        let category_cli_list = ["refactor", "tests", "docs", "debugging", "other", "unknown"];
        let category_wire: Vec<String> = TaskCategory::ALL
            .iter()
            .map(|c| {
                serde_json::to_value(c)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(category_wire, category_cli_list);

        let outcome_cli_list = ["accepted", "partial", "rejected", "unknown"];
        let outcome_wire: Vec<String> = TaskOutcome::ALL
            .iter()
            .map(|o| {
                serde_json::to_value(o)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(outcome_wire, outcome_cli_list);
    }

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

    #[test]
    fn one_unreadable_snapshot_leaves_every_other_operation_working() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.jsonl");
        let b = root.join("b.jsonl");
        trajectory(&a, "first");
        trajectory(&b, "second");
        let store = LocalInsightStore::open(&root.join("insights")).unwrap();
        let good = store.import(SourceFormat::Trajectory, &a).unwrap();
        let bad = store.import(SourceFormat::Trajectory, &b).unwrap();
        let path = store.dir.join("index.json");
        let mut index: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // A schema the current build cannot read at all, not merely an
        // invariant it fails: the entry has no type before it has a verdict.
        index["reports"][&bad.id]["boundary"] = "unknown_future_boundary".into();
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();

        assert_quarantined(&store, &bad.id, "an unreadable snapshot schema");
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, good.id);
        assert_eq!(store.explain(&good.id).unwrap().id, good.id);
        assert!(
            store
                .annotate(&good.id, TaskCategory::Docs, TaskOutcome::Unknown)
                .is_ok()
        );
        // Saving an unrelated mutation must not silently drop the entry the
        // user has not chosen to remove.
        assert_eq!(
            store.quarantine().unwrap().snapshot_ids,
            vec![bad.id.clone()]
        );
        assert!(store.delete(&bad.id).unwrap());
        assert!(store.quarantine().unwrap().is_empty());
        assert_eq!(store.list().unwrap().len(), 1);
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn repair_removes_only_what_the_store_is_withholding() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.jsonl");
        let b = root.join("b.jsonl");
        trajectory(&a, "first");
        trajectory(&b, "second");
        let store = LocalInsightStore::open(&root.join("insights")).unwrap();
        let good = store.import(SourceFormat::Trajectory, &a).unwrap();
        let bad = store.import(SourceFormat::Trajectory, &b).unwrap();
        let path = store.dir.join("index.json");
        let mut index: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        index["reports"][&bad.id]["cost_unavailable_reason"] = "invented".into();
        index["aliases"]["0".repeat(64)] =
            serde_json::json!({"report_id": "f".repeat(64), "identity": null});
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();

        let repaired = store.repair().unwrap();
        assert_eq!(repaired.quarantined.snapshot_ids, vec![bad.id.clone()]);
        assert!(repaired.quarantined.episode_ids.is_empty());
        assert_eq!(repaired.removed_aliases, 1);
        assert!(store.quarantine().unwrap().is_empty());
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, good.id);
        // Repair is idempotent, and never reaches an original file.
        assert_eq!(store.repair().unwrap(), RepairReport::default());
        assert!(a.exists() && b.exists());
    }

    #[test]
    fn a_store_written_before_file_identity_still_reads_and_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let file = root.join("legacy.jsonl");
        trajectory(&file, "first");
        let store = LocalInsightStore::open(&root.join("insights")).unwrap();
        let saved = store.import(SourceFormat::Trajectory, &file).unwrap();
        let path = store.dir.join("index.json");
        let mut index: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // The shape a v5 store has on disk: an address maps to a bare id.
        index["version"] = 5.into();
        for alias in index["aliases"].as_object_mut().unwrap().values_mut() {
            *alias = alias["report_id"].clone();
        }
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();

        assert_eq!(store.list().unwrap().len(), 1);
        assert!(store.quarantine().unwrap().is_empty());
        assert_eq!(store.explain(&saved.id).unwrap().id, saved.id);
        // Reimporting migrates the address in place rather than duplicating it.
        trajectory(&file, "second");
        let second = store.import(SourceFormat::Trajectory, &file).unwrap();
        let migrated: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(migrated["version"], STORE_VERSION);
        assert_eq!(migrated["aliases"].as_object().unwrap().len(), 1);
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, second.id);
    }

    #[test]
    fn a_renamed_source_does_not_keep_its_superseded_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let before = root.join("before.jsonl");
        let after = root.join("after.jsonl");
        trajectory(&before, "first");
        let store = LocalInsightStore::open(&root.join("insights")).unwrap();
        let first = store.import(SourceFormat::Trajectory, &before).unwrap();
        fs::rename(&before, &after).unwrap();
        // The same file, edited and reimported under its new name. The address
        // it used to have must not keep the superseded snapshot alive.
        trajectory(&after, "second");
        let second = store.import(SourceFormat::Trajectory, &after).unwrap();
        assert_ne!(first.id, second.id);
        let listed = store.list().unwrap();
        assert_eq!(listed.len(), 1, "a superseded snapshot outlived its source");
        assert_eq!(listed[0].id, second.id);
    }

    #[test]
    fn a_second_copy_still_keeps_the_snapshot_it_was_imported_as() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a.jsonl");
        let b = root.join("b.jsonl");
        trajectory(&a, "first");
        fs::copy(&a, &b).unwrap();
        let store = LocalInsightStore::open(&root.join("insights")).unwrap();
        let first = store.import(SourceFormat::Trajectory, &a).unwrap();
        store.import(SourceFormat::Trajectory, &b).unwrap();
        // A distinct file that still holds the old content is not a rename.
        trajectory(&a, "second");
        let second = store.import(SourceFormat::Trajectory, &a).unwrap();
        let ids = store
            .list()
            .unwrap()
            .into_iter()
            .map(|insight| insight.id)
            .collect::<BTreeSet<_>>();
        assert!(ids.contains(&first.id) && ids.contains(&second.id));
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn saved_snapshot_identity_is_pinned_and_never_taken_from_debug_output() {
        // Renaming a `SourceFormat` variant must not move a stored id. These
        // are the digests over the live preimage, independent of this code.
        let source_digest = "a".repeat(64);
        assert_eq!(SourceFormat::Codex.stable_id(), "Codex");
        assert_eq!(SourceFormat::ClaudeCode.stable_id(), "ClaudeCode");
        assert_eq!(SourceFormat::Trajectory.stable_id(), "Trajectory");
        assert_eq!(
            snapshot_identity(SourceFormat::Codex, &source_digest),
            "6166afe59337481cc1dc9df90da004870d6c8ac8b5729c0cef675d8d0f6e51c1"
        );
        assert_eq!(
            snapshot_identity(SourceFormat::Trajectory, &source_digest),
            "cc98f6d04157a005973700695cd6e074214648b55cd6dec655e7753bded0334a"
        );
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
            assert_quarantined(&store, &insight.id, field);
        }
        let mut corrupted = original;
        corrupted["version"] = 1.into();
        fs::write(&path, serde_json::to_vec(&corrupted).unwrap()).unwrap();
        assert_quarantined(&store, &insight.id, "an annotation under the v1 schema");
        // The entry the user cannot read is still the entry the user can remove.
        assert!(store.delete(&insight.id).unwrap());
        assert!(store.quarantine().unwrap().is_empty());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn claude_attribution_store_is_source_exclusive_and_requires_v10() {
        let dir = tempfile::tempdir().unwrap();
        let claude_path = dir.path().join("claude.jsonl");
        fs::write(
            &claude_path,
            include_bytes!("../fixtures/insights/claude-task-attribution/agent-alpha.jsonl"),
        )
        .unwrap();
        let codex_path = dir.path().join("codex.jsonl");
        fs::write(
            &codex_path,
            include_bytes!("../fixtures/insights/codex-task-attribution/codex-release-0.154.0-alpha-direct.jsonl"),
        )
        .unwrap();
        let store = LocalInsightStore::open(&dir.path().join("store")).unwrap();
        let claude = store
            .import(SourceFormat::ClaudeCode, &claude_path)
            .unwrap();
        let codex = store.import(SourceFormat::Codex, &codex_path).unwrap();
        let path = store.dir.join("index.json");
        let original: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();

        let mut both = original.clone();
        both["reports"][&claude.id]["task_attribution"] =
            original["reports"][&codex.id]["task_attribution"].clone();
        fs::write(&path, serde_json::to_vec(&both).unwrap()).unwrap();
        assert_quarantined(
            &store,
            &claude.id,
            "a Codex attribution on a Claude snapshot",
        );

        let mut legacy = original.clone();
        legacy["version"] = 9.into();
        fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        assert_quarantined(&store, &claude.id, "Claude attribution in a v9 store");

        let mut wrong_source = original;
        wrong_source["reports"][&claude.id]["source_format"] = "codex".into();
        fs::write(&path, serde_json::to_vec(&wrong_source).unwrap()).unwrap();
        assert_quarantined(&store, &claude.id, "a relabelled source format");
        // Only the relabelled entry is withheld; the Codex snapshot still reads.
        assert_eq!(store.list().unwrap().len(), 1);
        assert_eq!(store.explain(&codex.id).unwrap().id, codex.id);
    }

    #[test]
    fn shared_native_claude_snapshot_matches_the_rust_evidence_contract() {
        const SHARED: &[u8] = include_bytes!(
            "../fixtures/insights/claude-task-attribution/native-agent-alpha-snapshot.json"
        );
        let insight: LocalInsight = serde_json::from_slice(SHARED).unwrap();
        // The native shells read this verdict instead of re-deriving it, so the
        // shared fixture's stored value must be the one Rust recomputes.
        let raw: serde_json::Value = serde_json::from_slice(SHARED).unwrap();
        let observations = insight.model_observations.as_ref().unwrap();
        assert_eq!(
            raw["model_observations"]["contract"],
            serde_json::to_value(observations.contract).unwrap()
        );
        assert_eq!(
            observations.contract,
            models::ModelCoverageContract::ClaudeAssistantMessageV3
        );
        assert_eq!(insight.source_format, SourceFormat::ClaudeCode);
        assert!(insight.task_attribution.is_none());
        let attribution = insight.claude_task_attribution.as_ref().unwrap();
        attribution.validate().unwrap();
        assert_eq!(
            attribution.source_digest,
            insight.report.evidence[0].source_digest
        );
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
        let analyzed = analyze_file(SourceFormat::Codex, &path).unwrap();
        assert!(matches!(
            analyzed.task_attribution.unwrap().state,
            task_attribution::TaskAttributionState::Unavailable {
                reason: task_attribution::TaskAttributionUnavailableReason::SourceProfileMismatch,
                ..
            }
        ));
        fs::write(&path, "{\"type\":\"session_meta\",\"payload\":{}}\n\n{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"fixture\"}]}}\n").unwrap();
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
    fn claude_code_import_preserves_distinct_same_id_blocks_with_typed_unavailability() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"type\":\"user\",\"timestamp\":\"2026-09-12T12:00:00Z\",\"message\":{\"content\":\"synthetic request\"}}\n",
                "{\"type\":\"assistant\",\"timestamp\":\"2026-09-12T12:00:01Z\",\"message\":{\"id\":\"msg_synthetic\",\"model\":\"fixture-model\",\"content\":[{\"type\":\"tool_use\",\"id\":\"tool_synthetic\",\"name\":\"Read\",\"input\":{}}]}}\n",
                "{\"type\":\"assistant\",\"timestamp\":\"2026-09-12T12:00:02Z\",\"message\":{\"id\":\"msg_synthetic\",\"model\":\"fixture-model\",\"content\":[{\"type\":\"tool_use\",\"id\":\"tool_synthetic\",\"name\":\"Read\",\"input\":{}},{\"type\":\"text\",\"text\":\"synthetic answer\"}]}}\n",
                "{\"type\":\"user\",\"timestamp\":\"2026-09-12T12:00:03Z\",\"message\":{\"content\":[{\"type\":\"tool_result\",\"tool_use_id\":\"tool_synthetic\",\"content\":\"synthetic result\",\"is_error\":false}]}}\n"
            ),
        )
        .unwrap();

        let analyzed = analyze_file(SourceFormat::ClaudeCode, &path).unwrap();
        let metric = |id| {
            analyzed
                .report
                .metrics
                .iter()
                .find(|metric| metric.id == id)
                .unwrap()
        };
        assert_eq!(metric(MetricId::Events).value, Some(5));
        assert_eq!(metric(MetricId::ToolCalls).value, Some(2));
        assert_eq!(metric(MetricId::ToolFailures).value, Some(0));
        assert_eq!(metric(MetricId::InputTokens).value, None);
        assert_eq!(analyzed.source_format, SourceFormat::ClaudeCode);
        let models = analyzed.model_observations.unwrap();
        assert_eq!(models.schema_version, 3);
        assert_eq!(models.candidate_records, 2);
        assert_eq!(models.valid_declarations, 2);
        assert_eq!(models.declared_models, ["fixture-model"]);
        assert!(analyzed.usage_evidence.is_none());
        assert!(analyzed.time_evidence.is_none());
        assert!(analyzed.task_attribution.is_none());
        assert!(matches!(
            analyzed.claude_task_attribution.unwrap().state,
            claude_task_attribution::ClaudeTaskAttributionState::Unavailable {
                reason: claude_task_attribution::ClaudeTaskUnavailableReason::UnsupportedRecord,
                ..
            }
        ));
        assert_eq!(
            analyzed.report.evidence[0].source_digest,
            digest(&fs::read(path).unwrap())
        );
    }

    #[test]
    fn claude_code_import_deduplicates_only_exact_same_id_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.jsonl");
        let record = "{\"type\":\"assistant\",\"message\":{\"id\":\"msg_synthetic\",\"content\":[{\"type\":\"text\",\"text\":\"synthetic answer\"}]}}\n";
        fs::write(&path, format!("{record}{record}")).unwrap();
        let analyzed = analyze_file(SourceFormat::ClaudeCode, &path).unwrap();
        assert_eq!(
            analyzed
                .report
                .metrics
                .iter()
                .find(|metric| metric.id == MetricId::Events)
                .unwrap()
                .value,
            Some(1)
        );
    }

    #[test]
    fn claude_code_import_rejects_malformed_selected_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fixture.jsonl");
        fs::write(&path, "{\"type\":\"user\",\"message\":{}\n").unwrap();
        assert_eq!(
            analyze_file(SourceFormat::ClaudeCode, &path)
                .unwrap_err()
                .to_string(),
            "insights_invalid_claude_code"
        );
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
        let cleared = index.reports.keys().next().unwrap().clone();
        fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();
        assert_quarantined(&store, &cleared, "a snapshot with no metrics");
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
        assert_quarantined(&store, &cleared, "a fabricated token measurement");
    }
}

#[cfg(test)]
#[path = "insights/evidence_tests.rs"]
mod evidence_tests;

#[cfg(test)]
#[path = "insights/usage_store_tests.rs"]
mod usage_store_tests;
