//! Cline session adapter.
//!
//! Reads `<root>/<session-id>/<session-id>.messages.json`, where `<root>` is
//! Cline's own session data directory: `$CLINE_SESSION_DATA_DIR`, else
//! `$CLINE_DATA_DIR/sessions`, else `$CLINE_DIR/data/sessions`, else
//! `~/.cline/data/sessions`. One directory is one session; the messages
//! document is a single JSON object, not JSONL, and a sibling
//! `<session-id>.json` manifest carries the working directory, the model and
//! the start time when the session had one.
//!
//! This is the store the current Cline release (extension 4.1.17, built on
//! the `@cline/core` SDK) writes. The pre-SDK layout under VS Code's global
//! storage (`tasks/<id>/api_conversation_history.json`) is not read: upstream
//! itself treats it as read-only legacy, and it carries neither timestamps
//! nor model information per message.
//!
//! **Message-type dispatch is tolerant, and only that**, on the same terms as
//! `gemini_cli`: an unrecognised content block becomes an `Opaque` event
//! with a type marker rather than rejecting the file, because the SDK's
//! message shape is young and moving. Everything a gate depends on -- path
//! containment, the byte budget, and the requirement that the document
//! actually carry a `messages` array -- stays fail-closed.
//!
//! Image blocks are never copied: their `data` is base64 pixels, which is
//! neither text a gate scores nor something a contributor reviewed.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde_json::Value;

use super::{SOURCE_CLINE, SessionRef, SessionTranscript, TraceSource, real_file_within_root};

/// Overrides the whole Cline directory; sessions live under `data/sessions`.
pub const CLINE_DIR_ENV: &str = "CLINE_DIR";
/// Overrides the data directory; sessions live under `sessions`.
pub const CLINE_DATA_DIR_ENV: &str = "CLINE_DATA_DIR";
/// Overrides the session directory itself.
pub const CLINE_SESSION_DATA_DIR_ENV: &str = "CLINE_SESSION_DATA_DIR";

/// The largest session document this adapter will load, shared with every
/// other adapter's budget: they all bound how much of one conversation may
/// become resident on its way to being discarded.
pub(crate) const CLINE_SESSION_BUDGET: u64 = super::claude_code::GROUP_RAW_BYTE_BUDGET;

const MESSAGES_SUFFIX: &str = ".messages.json";
const MANIFEST_SUFFIX: &str = ".json";

/// The conventional store, resolved the way Cline's own `paths.ts` does it.
/// An empty variable counts as unset, matching upstream's `.trim()` check.
pub fn conventional_root(home: &Path, env: impl Fn(&str) -> Option<String>) -> PathBuf {
    let set = |key: &str| env(key).filter(|v| !v.trim().is_empty()).map(PathBuf::from);
    if let Some(sessions) = set(CLINE_SESSION_DATA_DIR_ENV) {
        return sessions;
    }
    if let Some(data) = set(CLINE_DATA_DIR_ENV) {
        return data.join("sessions");
    }
    set(CLINE_DIR_ENV)
        .unwrap_or_else(|| home.join(".cline"))
        .join("data")
        .join("sessions")
}

/// The conventional store, resolved against this machine's real home and
/// environment.
pub fn conventional_root_this_machine() -> PathBuf {
    conventional_root(&dirs::home_dir().unwrap_or_default(), |key| {
        std::env::var(key).ok()
    })
}

pub struct ClineSource {
    root: PathBuf,
}

impl ClineSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

/// The messages file a session directory must hold: `<dir name>.messages.json`.
fn messages_file_for(session_dir: &Path) -> Option<PathBuf> {
    let id = session_dir.file_name()?.to_str()?;
    Some(session_dir.join(format!("{id}{MESSAGES_SUFFIX}")))
}

/// The sibling manifest, if the session wrote one.
fn manifest_for(messages_path: &Path) -> Option<PathBuf> {
    let dir = messages_path.parent()?;
    let id = dir.file_name()?.to_str()?;
    let candidate = dir.join(format!("{id}{MANIFEST_SUFFIX}"));
    candidate.is_file().then_some(candidate)
}

/// What the manifest says about the session, where it says it. Every field
/// is optional: a session interrupted before its manifest was written is
/// still a session.
#[derive(Default)]
struct Manifest {
    cwd: Option<String>,
    model: Option<String>,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn read_manifest(messages_path: &Path) -> Manifest {
    let Some(path) = manifest_for(messages_path) else {
        return Manifest::default();
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return Manifest::default();
    };
    let Ok(doc) = serde_json::from_slice::<Value>(&bytes) else {
        return Manifest::default();
    };
    let string = |key: &str| {
        doc.get(key)
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
    };
    Manifest {
        cwd: string("cwd"),
        model: string("model"),
        started_at: timestamp_rfc3339(doc.get("started_at")),
    }
}

/// The label a picker renders: the basename of the working directory when
/// there is one, otherwise the session directory's own name.
fn project_label(session_dir: &Path, cwd: Option<&str>) -> Option<String> {
    cwd.map(Path::new)
        .or(Some(session_dir))
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
}

/// The one way a Cline `SessionRef` is built, shared by `discover` and
/// `session_at` so a scoped scan and a full sweep cannot disagree.
///
/// `None` for a file that is no longer there, which on the event path is an
/// ordinary race rather than a failure.
fn session_ref_for(path: PathBuf) -> Option<SessionRef> {
    let session_dir = path.parent()?.to_path_buf();
    let metadata = std::fs::metadata(&path).ok()?;
    let manifest = read_manifest(&path);
    let started_at = manifest.started_at.or_else(|| {
        metadata
            .modified()
            .ok()
            .map(chrono::DateTime::<chrono::Utc>::from)
    });
    let project = project_label(&session_dir, manifest.cwd.as_deref());
    Some(SessionRef {
        source: SOURCE_CLINE,
        declared_source: None,
        path,
        project,
        cwd: manifest.cwd,
        started_at,
        size_bytes: metadata.len(),
        // One document is one session. A subagent session is its own
        // directory with an `origin.parentThreadId` back-reference that this
        // adapter does not follow, so there is no group to describe.
        group_modified_at: None,
        group_member_count: 0,
    })
}

impl TraceSource for ClineSource {
    fn name(&self) -> &'static str {
        SOURCE_CLINE
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        let mut sessions = Vec::new();
        let mut skipped = 0usize;
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Ok(sessions);
        };
        for entry in entries {
            let Ok(entry) = entry else {
                skipped += 1;
                continue;
            };
            // `file_type` does not follow, so a symlinked session directory
            // is not descended into: a link planted under the store by any
            // same-user process must not steer collection elsewhere.
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => {}
                Ok(_) => continue,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            }
            let Some(messages) = messages_file_for(&entry.path()) else {
                continue;
            };
            match std::fs::symlink_metadata(&messages) {
                Ok(m) if m.is_file() => {}
                _ => continue,
            }
            match session_ref_for(messages) {
                Some(r) => sessions.push(r),
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(
                skipped,
                "skipped unreadable cline session entries during discovery"
            );
        }
        Ok(sessions)
    }

    /// A changed messages file is its own session, on exactly the terms
    /// `discover` uses: `<root>/<id>/<id>.messages.json`, two components
    /// deep. The manifest is deliberately not mapped: it changing does not
    /// change the bytes the transcript hashes.
    fn session_for_path(&self, path: &Path) -> Option<PathBuf> {
        let path = real_file_within_root(&self.root, path)?;
        let session_dir = path.parent()?;
        if session_dir.parent() != Some(self.root.as_path()) {
            return None;
        }
        (messages_file_for(session_dir)? == path).then_some(path)
    }

    fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
        let Some(address) = self.session_for_path(path) else {
            return Ok(None);
        };
        Ok(session_ref_for(address))
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        load_session(&r.path)
    }
}

fn timestamp_rfc3339(value: Option<&Value>) -> Option<chrono::DateTime<chrono::Utc>> {
    value
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

fn load_session(path: &Path) -> anyhow::Result<SessionTranscript> {
    // Declined rather than truncated, and named rather than silent. The size
    // is the contributor's own file's and safe to state; the path is not.
    let declared = std::fs::metadata(path)?.len();
    if declared > CLINE_SESSION_BUDGET {
        return Err(super::SessionTooLarge {
            label: "cline-session-too-large",
            declared_bytes: declared,
            budget_bytes: CLINE_SESSION_BUDGET,
        }
        .into());
    }
    // Task 3 replaces the rest of this body. Until then a load is a
    // refusal, so the discovery tests can run against a compiling adapter.
    let _ = read_manifest(path).model;
    Err(anyhow!("malformed_cline_session"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SOURCE_CLINE, TraceSource};

    fn fixture_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cline/sessions")
    }

    fn source() -> ClineSource {
        ClineSource::new(fixture_root())
    }

    #[test]
    fn the_conventional_root_follows_clines_own_precedence() {
        let home = Path::new("/home/c");
        let none = |_: &str| None;
        assert_eq!(
            conventional_root(home, none),
            PathBuf::from("/home/c/.cline/data/sessions")
        );
        let dir = |k: &str| (k == CLINE_DIR_ENV).then(|| "/opt/cline".to_string());
        assert_eq!(
            conventional_root(home, dir),
            PathBuf::from("/opt/cline/data/sessions")
        );
        let data = |k: &str| match k {
            CLINE_DIR_ENV => Some("/opt/cline".to_string()),
            CLINE_DATA_DIR_ENV => Some("/data/cl".to_string()),
            _ => None,
        };
        assert_eq!(
            conventional_root(home, data),
            PathBuf::from("/data/cl/sessions")
        );
        let sessions = |k: &str| match k {
            CLINE_DATA_DIR_ENV => Some("/data/cl".to_string()),
            CLINE_SESSION_DATA_DIR_ENV => Some("/s".to_string()),
            _ => None,
        };
        assert_eq!(conventional_root(home, sessions), PathBuf::from("/s"));
        // An empty value is unset, as upstream's `.trim()` check treats it.
        let empty = |k: &str| (k == CLINE_SESSION_DATA_DIR_ENV).then(String::new);
        assert_eq!(
            conventional_root(home, empty),
            PathBuf::from("/home/c/.cline/data/sessions")
        );
    }

    #[test]
    fn discovery_finds_each_messages_file_and_nothing_else() {
        let refs = source().discover().unwrap();
        let mut names: Vec<String> = refs
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "1756900000000_k3x9q.messages.json",
                "1756900100000_p2m7z.messages.json",
                "1756900200000_bad00.messages.json",
            ],
            "the stray directory is skipped; a malformed document is still discovered and refused at load"
        );
        for r in &refs {
            assert_eq!(r.source, SOURCE_CLINE);
            assert!(r.size_bytes > 0);
            assert_eq!(r.group_member_count, 0);
        }
    }

    #[test]
    fn a_manifest_gives_discovery_the_cwd_and_project() {
        let refs = source().discover().unwrap();
        let with = refs
            .iter()
            .find(|r| r.path.to_string_lossy().contains("k3x9q"))
            .unwrap();
        assert_eq!(with.cwd.as_deref(), Some("/home/contributor/code/alpha"));
        assert_eq!(with.project.as_deref(), Some("alpha"));
        let without = refs
            .iter()
            .find(|r| r.path.to_string_lossy().contains("p2m7z"))
            .unwrap();
        assert_eq!(without.cwd, None, "no manifest, no guess");
        assert_eq!(
            without.project.as_deref(),
            Some("1756900100000_p2m7z"),
            "the session directory name is the fallback label"
        );
    }

    #[test]
    fn a_changed_messages_file_maps_to_its_own_session_and_nothing_else_does() {
        let s = source();
        let messages = fixture_root().join("1756900000000_k3x9q/1756900000000_k3x9q.messages.json");
        assert_eq!(s.session_for_path(&messages), Some(messages.clone()));
        // The manifest changing does not change the transcript's bytes.
        let manifest = fixture_root().join("1756900000000_k3x9q/1756900000000_k3x9q.json");
        assert_eq!(s.session_for_path(&manifest), None);
        // Outside the root, and a name that does not follow the rule.
        assert_eq!(s.session_for_path(Path::new("/etc/passwd")), None);
        let stray = fixture_root().join("not-a-session/notes.txt");
        assert_eq!(s.session_for_path(&stray), None);
        // A messages file whose name disagrees with its directory is not a
        // session: the id is the directory, and the file must repeat it.
        let wrong = fixture_root().join("1756900000000_k3x9q/other.messages.json");
        assert_eq!(s.session_for_path(&wrong), None);
    }

    #[test]
    fn session_at_agrees_with_discover() {
        let s = source();
        for r in s.discover().unwrap() {
            let again = s.session_at(&r.path).unwrap().expect("the same session");
            assert_eq!(again.path, r.path);
            assert_eq!(again.size_bytes, r.size_bytes);
            assert_eq!(again.cwd, r.cwd);
            assert_eq!(again.project, r.project);
        }
    }
}
