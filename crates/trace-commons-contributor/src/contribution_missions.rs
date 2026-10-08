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

use std::collections::{BTreeMap, BTreeSet};

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

/// One session matching was allowed to read, as the matcher sees it: the
/// tool it came from and the folder it ran in. Nothing else about it.
///
/// Built only by `daemon::mission_matching`, which leaves out every session
/// from a tool that is switched off or a folder set to Never (M1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionFact {
    /// The session's tool, as `list_projects`' `tools[].source` names it.
    pub tool: String,
    /// The folder's project key. Never leaves this process.
    pub folder: String,
}

/// Everything the matcher is given: the readable sessions, and the
/// languages of the folders they ran in when a mission asks about
/// languages. Local only: no part of it is sent, stored or logged (M1).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LocalFacts {
    pub sessions: Vec<SessionFact>,
    pub folder_languages: BTreeMap<String, BTreeSet<String>>,
}

impl LocalFacts {
    /// How many distinct tools matching read sessions from.
    pub fn tools_read(&self) -> usize {
        self.sessions
            .iter()
            .map(|s| s.tool.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// How many distinct folders matching read sessions from.
    pub fn folders_read(&self) -> usize {
        self.sessions
            .iter()
            .map(|s| s.folder.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// The ids of the missions in `catalogue` that `facts` fit, in catalogue
/// order.
///
/// Pure: it reads only what it is handed, and has no way to read a policy,
/// a queue or a file, to change one, or to send anything (M1, M2). A match
/// is a suggestion for this Mac's screen and nothing more.
pub fn match_missions(catalogue: &ContributionMissionCatalogue, facts: &LocalFacts) -> Vec<String> {
    catalogue
        .missions
        .iter()
        .filter(|m| {
            let need = m.criteria.min_sessions.max(1) as usize;
            facts
                .sessions
                .iter()
                .filter(|s| session_fits(&m.criteria, s, facts))
                .take(need)
                .count()
                == need
        })
        .map(|m| m.mission_id.clone())
        .collect()
}

/// How many of `matched_ids`' missions `session` fits: the per-session
/// half of [`match_missions`], for counting what one waiting session would
/// count toward.
///
/// A mission counts only when it is in `matched_ids` -- what
/// [`match_missions`] returned over the same `facts` -- as well as fitting
/// the session's criteria. So a session never fits a mission whose
/// `min_sessions` the readable sessions have not reached, and the count
/// agrees with what a Missions screen would show (OWNER DECISION V3).
pub fn missions_fitting(
    catalogue: &ContributionMissionCatalogue,
    matched_ids: &[String],
    session: &SessionFact,
    facts: &LocalFacts,
) -> usize {
    catalogue
        .missions
        .iter()
        .filter(|m| {
            matched_ids.contains(&m.mission_id) && session_fits(&m.criteria, session, facts)
        })
        .count()
}

pub(crate) fn session_fits(
    criteria: &MissionCriteria,
    session: &SessionFact,
    facts: &LocalFacts,
) -> bool {
    let any = |wanted: &[String], have: &str| wanted.iter().any(|w| w == have);
    (criteria.tools.is_empty() || any(&criteria.tools, &session.tool))
        && (criteria.tool_families.is_empty()
            || crate::source::source_default_family(&session.tool)
                .is_some_and(|family| any(&criteria.tool_families, family)))
        && (criteria.languages.is_empty()
            || facts
                .folder_languages
                .get(&session.folder)
                .is_some_and(|langs| langs.iter().any(|l| any(&criteria.languages, l))))
}

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
    fn fact(tool: &str, folder: &str) -> SessionFact {
        SessionFact {
            tool: tool.to_string(),
            folder: folder.to_string(),
        }
    }

    fn catalogue(missions: serde_json::Value) -> ContributionMissionCatalogue {
        ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1, "missions": missions,
        }))
        .unwrap()
    }

    /// Each criterion restricts only when given, the lists are combined per
    /// session, and min_sessions counts sessions that fit every one.
    #[test]
    fn missions_match_on_tool_family_language_and_count() {
        let facts = LocalFacts {
            sessions: vec![
                fact("claude-code", "/a"),
                fact("claude-code", "/a"),
                fact("codex", "/b"),
            ],
            folder_languages: BTreeMap::from([
                ("/a".to_string(), BTreeSet::from(["rust".to_string()])),
                ("/b".to_string(), BTreeSet::from(["python".to_string()])),
            ]),
        };
        let cat = catalogue(json!([
            {"mission_id": "any"},
            {"mission_id": "claude", "criteria": {"tools": ["claude-code"]}},
            {"mission_id": "openai", "criteria": {"tool_families": ["openai"]}},
            {"mission_id": "rust-2", "criteria": {"languages": ["rust"], "min_sessions": 2}},
            {"mission_id": "rust-3", "criteria": {"languages": ["rust"], "min_sessions": 3}},
            {"mission_id": "codex-rust", "criteria": {"tools": ["codex"], "languages": ["rust"]}},
            {"mission_id": "gemini", "criteria": {"tools": ["gemini-cli"]}},
        ]));
        assert_eq!(
            match_missions(&cat, &facts),
            vec!["any", "claude", "openai", "rust-2"]
        );
        assert_eq!(facts.tools_read(), 2);
        assert_eq!(facts.folders_read(), 2);
    }

    /// Nothing read matches nothing, even a mission asking for no sessions.
    #[test]
    fn nothing_read_matches_nothing() {
        let cat = catalogue(json!([
            {"mission_id": "any"},
            {"mission_id": "zero", "criteria": {"min_sessions": 0}},
        ]));
        assert!(match_missions(&cat, &LocalFacts::default()).is_empty());
    }

    /// `missions_fitting` counts a mission only when it was matched and the
    /// session meets its criteria; one it meets but that was not matched
    /// (its `min_sessions` unreached), or one matched that it does not
    /// meet, is not counted (OWNER DECISION V3).
    #[test]
    fn missions_fitting_counts_matched_missions_the_session_meets() {
        let cat = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1,
            "missions": [
                {"mission_id": "claude", "criteria": {"tools": ["claude-code"]}},
                {"mission_id": "anthropic", "criteria": {"tool_families": ["anthropic"]}},
                {"mission_id": "claude-5", "criteria": {"tools": ["claude-code"], "min_sessions": 5}},
                {"mission_id": "codex", "criteria": {"tools": ["codex"]}},
            ],
        }))
        .unwrap();
        let fact = |tool: &str| SessionFact {
            tool: tool.to_string(),
            folder: "/work/a".to_string(),
        };
        let facts = LocalFacts {
            sessions: vec![fact("claude-code"), fact("codex")],
            folder_languages: BTreeMap::new(),
        };
        let matched = match_missions(&cat, &facts);
        assert_eq!(matched, vec!["claude", "anthropic", "codex"]);
        assert_eq!(
            missions_fitting(&cat, &matched, &fact("claude-code"), &facts),
            2
        );
        assert_eq!(missions_fitting(&cat, &matched, &fact("codex"), &facts), 1);
        assert_eq!(missions_fitting(&cat, &[], &fact("claude-code"), &facts), 0);
    }
}
