//! Contribution missions, matched on this Mac (K16, #1173).
//!
//! **Not** the published mission packages in [`crate::mission_catalog`]
//! (`GET /v1/missions`): those are skill-evaluation tasks with their own
//! reviewer and reward ledger. A contribution mission is completed by
//! contributing sessions through the paths that already exist. The rules
//! are the "Missions" section, M1–M4, of
//! `docs/superpowers/specs/2026-09-23-connect-and-forget-consent-design.md`.
//!
//! This module is the catalogue the daemon is given and the pure matcher
//! over it. What the matcher may read is decided before it runs, in
//! `daemon::mission_matching`: it is handed facts, never the policy, the
//! queue or a file, so it cannot widen what it reads or change anything.
//!
//! # PROVISIONAL: shape owned by Z7/Z8
//!
//! The server catalogue does not exist yet. Until Z7/Z8 publish one, the
//! catalogue arrives as a parameter of the `mission_matches` IPC call, and
//! this type is the minimum matching needs. Z7/Z8 own the shape; expect it to
//! change, under a new `schema_version`. It is read leniently -- unknown
//! fields are ignored and every field but `schema_version` and `mission_id`
//! has a default -- so a catalogue written for a later minor addition still
//! loads. A catalogue whose `schema_version` is newer than this build knows
//! is refused rather than half-read.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// The catalogue schema this build reads. PROVISIONAL: shape owned by Z7/Z8.
pub const CONTRIBUTION_MISSION_CATALOGUE_SCHEMA_VERSION: u32 = 1;

/// The most missions one catalogue may carry.
pub const MAX_MISSIONS: usize = 256;

/// The most characters a mission id or title may have.
pub const MAX_TEXT_CHARS: usize = 200;

/// The most values one criterion list may carry.
pub const MAX_CRITERION_VALUES: usize = 32;

/// A catalogue of contribution missions: the vendor requests the commons
/// publishes, the same for every contributor (M1).
///
/// PROVISIONAL: shape owned by Z7/Z8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionMissionCatalogue {
    pub schema_version: u32,
    #[serde(default)]
    pub missions: Vec<ContributionMission>,
}

/// One contribution mission. PROVISIONAL: shape owned by Z7/Z8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionMission {
    /// The mission's id, as the catalogue spells it. The only thing
    /// `mission_matches` returns about a mission.
    pub mission_id: String,
    /// Display only. Never read by the matcher.
    #[serde(default)]
    pub title: String,
    /// What a contributor's local work must show for the mission to match.
    /// Absent is "no criteria": any work matching may read.
    #[serde(default)]
    pub criteria: MissionCriteria,
}

/// What a mission asks for. Every list is "any of"; an empty list does not
/// restrict. The lists are combined with AND, per session: a session counts
/// when its tool, its tool's family and its folder's languages all satisfy
/// the lists that are not empty. The mission matches when at least
/// `min_sessions` such sessions were readable.
///
/// PROVISIONAL: shape owned by Z7/Z8. The names follow the words this crate
/// already uses: a tool is a session source's name as `list_projects`'
/// `tools[].source` reports it, and a family is
/// [`crate::source::source_default_family`]'s answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionCriteria {
    /// Session sources, e.g. `claude-code`, `codex`, `gemini-cli`.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Tool families, e.g. `anthropic`, `openai`, `google`.
    #[serde(default)]
    pub tool_families: Vec<String>,
    /// Languages a folder is worked in, from the files at its root (see
    /// [`LANGUAGE_MARKERS`]), e.g. `rust`, `python`.
    #[serde(default)]
    pub languages: Vec<String>,
    /// How many readable sessions must fit. Read as at least 1: a mission
    /// never matches on nothing read.
    #[serde(default = "one")]
    pub min_sessions: u32,
}

fn one() -> u32 {
    1
}

impl Default for MissionCriteria {
    fn default() -> Self {
        Self {
            tools: Vec::new(),
            tool_families: Vec::new(),
            languages: Vec::new(),
            min_sessions: 1,
        }
    }
}

/// Why a catalogue was not read. Labels only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueError {
    /// Not an object of this shape, or over a bound.
    Invalid,
    /// A `schema_version` this build does not read.
    SchemaUnsupported,
}

impl CatalogueError {
    pub fn label(self) -> &'static str {
        match self {
            CatalogueError::Invalid => "catalogue-invalid",
            CatalogueError::SchemaUnsupported => "catalogue-schema-unsupported",
        }
    }
}

impl ContributionMissionCatalogue {
    /// Read a catalogue, ignoring fields this build does not know and
    /// refusing one that is newer than it, malformed, or over a bound.
    pub fn from_value(value: &serde_json::Value) -> Result<Self, CatalogueError> {
        // The version first, so a later shape is refused as such rather than
        // as malformed.
        let version = value
            .get("schema_version")
            .and_then(serde_json::Value::as_u64)
            .ok_or(CatalogueError::Invalid)?;
        if version == 0 {
            return Err(CatalogueError::Invalid);
        }
        if version > u64::from(CONTRIBUTION_MISSION_CATALOGUE_SCHEMA_VERSION) {
            return Err(CatalogueError::SchemaUnsupported);
        }
        let catalogue: Self =
            serde_json::from_value(value.clone()).map_err(|_| CatalogueError::Invalid)?;
        catalogue.check_bounds()?;
        Ok(catalogue)
    }

    fn check_bounds(&self) -> Result<(), CatalogueError> {
        if self.missions.len() > MAX_MISSIONS {
            return Err(CatalogueError::Invalid);
        }
        let mut ids = BTreeSet::new();
        for m in &self.missions {
            let id_ok = !m.mission_id.trim().is_empty()
                && m.mission_id.chars().count() <= MAX_TEXT_CHARS
                && !m.mission_id.chars().any(char::is_control);
            if !id_ok || m.title.chars().count() > MAX_TEXT_CHARS || !ids.insert(&m.mission_id) {
                return Err(CatalogueError::Invalid);
            }
            let c = &m.criteria;
            if [&c.tools, &c.tool_families, &c.languages]
                .iter()
                .any(|l| l.len() > MAX_CRITERION_VALUES)
            {
                return Err(CatalogueError::Invalid);
            }
        }
        Ok(())
    }

    /// Whether any mission asks about languages, so a folder's root is only
    /// looked at when a criterion needs it.
    pub fn needs_languages(&self) -> bool {
        self.missions
            .iter()
            .any(|m| !m.criteria.languages.is_empty())
    }
}

/// The files at a folder's root that say which language it is worked in.
/// Existence only: nothing is opened.
pub const LANGUAGE_MARKERS: &[(&str, &str)] = &[
    ("Cargo.toml", "rust"),
    ("go.mod", "go"),
    ("package.json", "javascript"),
    ("tsconfig.json", "typescript"),
    ("pyproject.toml", "python"),
    ("setup.py", "python"),
    ("requirements.txt", "python"),
    ("Package.swift", "swift"),
    ("Gemfile", "ruby"),
    ("pom.xml", "java"),
    ("build.gradle", "java"),
    ("build.gradle.kts", "kotlin"),
    ("composer.json", "php"),
    ("mix.exs", "elixir"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A catalogue carrying fields this build has never heard of, at every
    /// level, still loads, and the fields it does know are read.
    #[test]
    fn a_catalogue_with_unknown_fields_loads() {
        let cat = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1,
            "published_at": "2026-10-02T00:00:00Z",
            "missions": [{
                "mission_id": "m-1",
                "title": "Rust refactors",
                "bonus_factor": 2,
                "criteria": {"tools": ["claude-code"], "languages": ["rust"], "repo_size": "large"},
            }],
        }))
        .expect("unknown fields are ignored");
        assert_eq!(cat.missions[0].mission_id, "m-1");
        assert_eq!(cat.missions[0].criteria.tools, vec!["claude-code"]);
        assert_eq!(cat.missions[0].criteria.languages, vec!["rust"]);
        assert_eq!(cat.missions[0].criteria.min_sessions, 1);
    }

    /// A catalogue written with less than this build reads -- no missions,
    /// a mission with no title or criteria -- loads with defaults that ask
    /// for at least one readable session, never zero.
    #[test]
    fn a_legacy_catalogue_loads_with_safe_defaults() {
        let empty = ContributionMissionCatalogue::from_value(&json!({"schema_version": 1}))
            .expect("missions default to none");
        assert!(empty.missions.is_empty());
        let bare = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1, "missions": [{"mission_id": "m"}],
        }))
        .expect("title and criteria default");
        assert_eq!(bare.missions[0].title, "");
        assert_eq!(bare.missions[0].criteria, MissionCriteria::default());
        assert_eq!(bare.missions[0].criteria.min_sessions, 1);
        assert!(!bare.needs_languages());
    }

    /// A newer schema is refused as such, not half-read; a malformed or
    /// oversized one is refused as invalid.
    #[test]
    fn a_newer_or_malformed_catalogue_is_refused() {
        let refuse = |v: serde_json::Value| ContributionMissionCatalogue::from_value(&v).err();
        assert_eq!(
            refuse(json!({"schema_version": 2, "missions": "anything"})),
            Some(CatalogueError::SchemaUnsupported)
        );
        assert_eq!(refuse(json!({})), Some(CatalogueError::Invalid));
        assert_eq!(refuse(json!([])), Some(CatalogueError::Invalid));
        assert_eq!(
            refuse(json!({"schema_version": 0})),
            Some(CatalogueError::Invalid)
        );
        assert_eq!(
            refuse(json!({"schema_version": 1, "missions": [{"title": "no id"}]})),
            Some(CatalogueError::Invalid)
        );
        assert_eq!(
            refuse(json!({"schema_version": 1, "missions": [{"mission_id": " "}]})),
            Some(CatalogueError::Invalid)
        );
        assert_eq!(
            refuse(json!({"schema_version": 1, "missions": [
                {"mission_id": "a"}, {"mission_id": "a"},
            ]})),
            Some(CatalogueError::Invalid),
            "a duplicate id would make a match ambiguous"
        );
        let many: Vec<_> = (0..=MAX_MISSIONS)
            .map(|i| json!({"mission_id": format!("m{i}")}))
            .collect();
        assert_eq!(
            refuse(json!({"schema_version": 1, "missions": many})),
            Some(CatalogueError::Invalid)
        );
        assert_eq!(CatalogueError::Invalid.label(), "catalogue-invalid");
        assert_eq!(
            CatalogueError::SchemaUnsupported.label(),
            "catalogue-schema-unsupported"
        );
    }
}
