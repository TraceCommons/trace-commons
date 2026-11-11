//! Bounded local skill inventory and content-based publisher comparisons.
//!
//! This module never installs skills, executes instructions, or contacts a network.
//! Locations are symbolic; a name is presentation, never publisher identity.

mod document;
mod inventory;
mod policy;
mod provenance;
#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub use document::parse_skill_document;
pub use inventory::{
    configured_global_roots, scan_inventory, scan_inventory_with_policy, scan_inventory_with_roots,
};
pub use provenance::{normalize_repository, normalize_skill_path};

/// One document is read once and hashed without newline or whitespace changes.
pub const MAX_SKILL_BYTES: u64 = 256 * 1024;
pub const MAX_SKILL_NAME_BYTES: usize = 512;
pub const MAX_SKILL_DESCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_SKILLS: usize = 2048;
pub const MAX_PROJECT_ROOTS: usize = 64;
pub(crate) const MAX_METADATA_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillSource {
    /// Proven GitHub owner/repository, normalized to lowercase.
    pub repository: String,
    /// Repository-relative file path, including SKILL.md.
    pub path: String,
    pub revision: Option<String>,
    /// Exact tracked upstream entrypoint bytes for this installation. Never
    /// inferred from revision labels, matching names, or directory hashes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledSkill {
    pub id: String,
    pub name: String,
    pub description: String,
    pub agent: String,
    pub scope: String,
    pub location: String,
    pub sha256: String,
    pub source: Option<SkillSource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogPublisher {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub skills_root: String,
    pub instructions_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogSkill {
    pub id: String,
    pub publisher_id: String,
    pub name: String,
    pub description: String,
    pub repository: String,
    pub path: String,
    pub revision: String,
    pub sha256: String,
    pub source_url: String,
    pub instructions_url: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CatalogSnapshot {
    pub publishers: Vec<CatalogPublisher>,
    pub skills: Vec<CatalogSkill>,
    pub checked_at: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Current,
    UpdateAvailable,
    LocallyModified,
    UnknownSource,
    NotChecked,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    Complete,
    Missing,
    Partial,
    CachedPluginActivationUnknown,
    PolicyExcluded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScanCoverage {
    pub location: String,
    pub status: ScanStatus,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Inventory {
    /// Local display metadata. Raw canonical paths are never serialized.
    #[serde(default)]
    pub details: std::collections::BTreeMap<String, SkillInstallationDetails>,
    pub skills: Vec<InstalledSkill>,
    pub partial: bool,
    #[serde(default)]
    pub coverage: Vec<ScanCoverage>,
}

/// Only content observed at the same repository/path can prove an update.
/// A mismatch with no matching history stays unchecked. Revision differences
/// alone do not establish an update or prove that local content was modified.
pub fn freshness(
    installed: &InstalledSkill,
    latest: &CatalogSkill,
    known_previous_hashes: &[String],
) -> Freshness {
    let Some(source) = &installed.source else {
        return Freshness::UnknownSource;
    };
    if source.repository != latest.repository || source.path != latest.path {
        return Freshness::UnknownSource;
    }
    if installed.sha256.is_empty() || latest.sha256.is_empty() {
        return Freshness::NotChecked;
    }
    if installed.sha256 == latest.sha256 {
        Freshness::Current
    } else if source
        .baseline_sha256
        .as_deref()
        .is_some_and(|baseline| baseline != installed.sha256)
    {
        Freshness::LocallyModified
    } else if known_previous_hashes.contains(&installed.sha256) {
        Freshness::UpdateAvailable
    } else {
        Freshness::NotChecked
    }
}

/// A baseline is the exact upstream content recorded for this installation,
/// rather than an arbitrary historical hash. Callers must preserve that link.
pub fn freshness_with_baseline(
    installed: &InstalledSkill,
    latest: &CatalogSkill,
    known_previous_hashes: &[String],
    installed_baseline_hash: Option<&str>,
) -> Freshness {
    let status = freshness(installed, latest, known_previous_hashes);
    if status == Freshness::NotChecked
        && installed_baseline_hash.is_some_and(|baseline| baseline != installed.sha256)
    {
        Freshness::LocallyModified
    } else {
        status
    }
}

pub(crate) fn hash_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub(crate) struct Root {
    path: PathBuf,
    agent: String,
    scope: String,
    location: String,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillAvailability {
    Available,
    Disabled,
    Cached,
    Missing,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillDefinitionStatus {
    #[default]
    Valid,
    Invalid,
    Unreadable,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillInstallationDetails {
    pub aliases: Vec<String>,
    pub availability: SkillAvailability,
    pub definition_status: SkillDefinitionStatus,
    /// Ephemeral path for local authorization and attribution. Never persisted.
    #[serde(skip)]
    pub canonical_path: PathBuf,
}
