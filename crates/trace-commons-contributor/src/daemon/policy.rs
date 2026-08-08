//! Which projects may upload without asking.
//!
//! Autonomy is per-project and opt-in. An unknown project is `NotifyOnly`, so
//! a freshly installed daemon uploads nothing until the contributor has
//! deliberately said otherwise about a specific project.
//!
//! The policy key is the session's true working directory. It is deliberately
//! *not* derived from the project basename: Claude Code encodes a cwd into a
//! directory name by replacing every `/` with `-`, which is ambiguous for any
//! hyphenated project name, and guessing wrong here would apply one project's
//! autonomy to a different project's traces.
//!
//! Sessions whose working directory cannot be resolved go to a single locked
//! bucket that can never be granted autonomy. Subagent transcripts and
//! normalized trajectory files land there. Since the daemon cannot tell which
//! project such a session belongs to, it cannot honour any opt-in for it.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigStore, DAEMON_PROJECTS_FILE};

pub const DAEMON_PROJECTS_SCHEMA: &str = "trace_commons.daemon_projects.v1";

/// The bucket for sessions with no resolvable working directory. Permanently
/// notify-only.
pub const UNKNOWN_PROJECT_KEY: &str = "unknown-project";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMode {
    /// Upload without asking.
    AutoUpload,
    /// Queue it and mention it in the next digest.
    NotifyOnly,
    /// Never offer sessions from this project at all.
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub mode: ProjectMode,
    pub added_at: DateTime<Utc>,
    /// Display name for consumers. Shells render this; they never render the
    /// key, which is a full local path.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPolicy {
    pub schema_version: String,
    pub projects: BTreeMap<String, ProjectEntry>,
}

impl Default for ProjectPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectPolicy {
    pub fn new() -> Self {
        Self {
            schema_version: DAEMON_PROJECTS_SCHEMA.to_string(),
            projects: BTreeMap::new(),
        }
    }

    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_PROJECTS_FILE)? else {
            return Ok(Self::new());
        };
        serde_json::from_slice(&body).context("parsing daemon project policy")
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serializing daemon project policy")?;
        store.write_daemon_file(DAEMON_PROJECTS_FILE, &body)
    }

    /// The mode in force for a project key.
    ///
    /// Autonomy for the unknown-cwd bucket is denied here, after the stored
    /// map is consulted rather than before it, so a hand-edited or tampered
    /// policy file cannot grant autonomy to sessions the daemon cannot
    /// attribute. `Ignore` is still honoured for that bucket: refusing to
    /// upload it unattended is not a reason to refuse to silence it.
    pub fn resolve(&self, project_key: &str) -> ProjectMode {
        let stored = self
            .projects
            .get(project_key)
            .map(|e| e.mode)
            .unwrap_or(ProjectMode::NotifyOnly);
        if project_key == UNKNOWN_PROJECT_KEY && stored == ProjectMode::AutoUpload {
            return ProjectMode::NotifyOnly;
        }
        stored
    }

    pub fn set_mode(
        &mut self,
        project_key: &str,
        label: &str,
        mode: ProjectMode,
        now: DateTime<Utc>,
    ) -> Result<()> {
        if project_key == UNKNOWN_PROJECT_KEY && mode == ProjectMode::AutoUpload {
            bail!(
                "unknown-project sessions cannot be set to auto_upload: \
                 their working directory could not be resolved, so no \
                 per-project opt-in can apply to them"
            );
        }
        self.projects.insert(
            project_key.to_string(),
            ProjectEntry {
                mode,
                added_at: now,
                label: label.to_string(),
            },
        );
        Ok(())
    }
}

/// The policy key for a session: its true working directory, or the locked
/// unknown bucket. Never falls back to a basename heuristic.
pub fn project_key_for(cwd: Option<&str>) -> String {
    match cwd {
        Some(cwd) if !cwd.trim().is_empty() => cwd.to_string(),
        _ => UNKNOWN_PROJECT_KEY.to_string(),
    }
}

/// A display label for a project key: the final path segment, or the bucket
/// name. Consumers render this instead of the key, which is a local path.
pub fn project_label_for(project_key: &str) -> String {
    if project_key == UNKNOWN_PROJECT_KEY {
        return UNKNOWN_PROJECT_KEY.to_string();
    }
    std::path::Path::new(project_key)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project_key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    fn now() -> DateTime<Utc> {
        "2026-08-08T12:00:00Z".parse().unwrap()
    }

    #[test]
    fn an_unknown_project_defaults_to_notify_only() {
        let p = ProjectPolicy::new();
        assert_eq!(
            p.resolve("/Users/z/code/never-seen"),
            ProjectMode::NotifyOnly
        );
    }

    #[test]
    fn sessions_without_a_cwd_land_in_the_unknown_bucket() {
        assert_eq!(project_key_for(None), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("   ")), UNKNOWN_PROJECT_KEY);
        assert_eq!(
            project_key_for(Some("/Users/z/code/proj")),
            "/Users/z/code/proj"
        );
    }

    #[test]
    fn the_unknown_bucket_cannot_be_set_to_auto_upload() {
        let mut p = ProjectPolicy::new();
        let err = p
            .set_mode(
                UNKNOWN_PROJECT_KEY,
                "unknown",
                ProjectMode::AutoUpload,
                now(),
            )
            .unwrap_err();
        assert!(err.to_string().contains("unknown-project"));
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn the_unknown_bucket_stays_notify_only_even_if_the_file_says_auto() {
        // Defence against a hand-edited or tampered policy file.
        let mut p = ProjectPolicy::new();
        p.projects.insert(
            UNKNOWN_PROJECT_KEY.to_string(),
            ProjectEntry {
                mode: ProjectMode::AutoUpload,
                added_at: now(),
                label: "unknown".into(),
            },
        );
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn the_unknown_bucket_may_still_be_ignored() {
        // Refusing autonomy is not the same as refusing to be silenced.
        let mut p = ProjectPolicy::new();
        p.set_mode(UNKNOWN_PROJECT_KEY, "unknown", ProjectMode::Ignore, now())
            .unwrap();
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::Ignore);
    }

    #[test]
    fn set_and_resolve_round_trip_for_a_real_project() {
        let mut p = ProjectPolicy::new();
        p.set_mode("/Users/z/code/proj", "proj", ProjectMode::AutoUpload, now())
            .unwrap();
        assert_eq!(p.resolve("/Users/z/code/proj"), ProjectMode::AutoUpload);
        p.set_mode("/Users/z/code/proj", "proj", ProjectMode::Ignore, now())
            .unwrap();
        assert_eq!(p.resolve("/Users/z/code/proj"), ProjectMode::Ignore);
    }

    #[test]
    fn labels_are_basenames_so_consumers_never_need_the_path() {
        assert_eq!(project_label_for("/Users/z/code/my-proj"), "my-proj");
        assert_eq!(project_label_for(UNKNOWN_PROJECT_KEY), UNKNOWN_PROJECT_KEY);
    }

    #[test]
    fn policy_round_trips_through_the_store() {
        let (_d, store) = temp_store();
        let mut p = ProjectPolicy::new();
        p.set_mode("/Users/z/code/proj", "proj", ProjectMode::AutoUpload, now())
            .unwrap();
        p.save(&store).unwrap();
        assert_eq!(ProjectPolicy::load(&store).unwrap(), p);
    }

    #[test]
    fn policy_defaults_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        assert_eq!(ProjectPolicy::load(&store).unwrap(), ProjectPolicy::new());
    }
}
