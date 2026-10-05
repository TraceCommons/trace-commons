//! The first-run past-session picker: one folder's past sessions, whatever
//! the queue knows of them.
//!
//! The listing walks the declared sources itself rather than reading the
//! watcher's cwd cache, so it is complete before the first discovery pass
//! has run and it includes sessions no pass has ever visited. It never calls
//! `TraceSource::load`: a session the queue has not offered is described by
//! its date and size only, because nothing is read before the person
//! chooses.
//!
//! Session identity on the wire is an opaque id (`session_id_for`), never a
//! path. A row carries no path, cwd or project path.

use std::collections::HashMap;
use std::path::Path;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::ipc::DaemonShared;
use super::policy::ProjectMode;
use super::queue::QueueState;
use crate::source::{SessionRef, all_sources};

/// What every session id starts with, so a path, an empty string or a
/// project id is told apart from a session id without a lookup.
pub const SESSION_ID_PREFIX: &str = "sess_";

/// Hex characters of the sha256 kept in a session id.
const SESSION_ID_HEX_CHARS: usize = 32;

/// The opaque id a past session is named by on the wire: a prefix and the
/// first 32 hex characters of sha256 over the session path's bytes.
///
/// One-way, so it leaks no path component; deterministic, so the id the
/// listing gave is the id an include can re-derive from the same walk.
pub fn session_id_for(path: &Path) -> String {
    let digest = Sha256::digest(path.as_os_str().as_encoded_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{SESSION_ID_PREFIX}{}", &hex[..SESSION_ID_HEX_CHARS])
}

/// Where a past session stands, as the picker shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PastSessionState {
    /// Queued and waiting on the person.
    Pending,
    /// Already approved.
    Approved,
    /// Aged out of the queue without a decision.
    Expired,
    /// On disk, never offered.
    NotQueued,
    /// The folder's rule is Never.
    Never,
    /// Still being written: not quiescent yet.
    StillActive,
}

impl PastSessionState {
    /// Whether the picker may tick a row in this state.
    pub fn selectable(self) -> bool {
        matches!(self, Self::Pending | Self::Expired | Self::NotQueued)
    }
}

/// One row of the picker. Display fields only.
#[derive(Debug, Clone, Serialize)]
pub struct PastSessionRow {
    pub session_id: String,
    pub entry_id: Option<Uuid>,
    pub state: PastSessionState,
    pub selectable: bool,
    pub started_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<i64>,
    pub title: Option<String>,
    pub size_bytes: u64,
    pub source: String,
}

/// One session a declared source lists, keyed to its project.
#[derive(Debug, Clone)]
pub struct DiscoveredSession {
    pub session_id: String,
    pub project_key: String,
    pub session_ref: SessionRef,
    /// The newest mtime the ref covers: its group's, or the file's own.
    pub modified_at: DateTime<Utc>,
}

/// Walk every declared source and key each session to its project, without
/// opening a session file beyond what each adapter's `discover` already
/// does.
///
/// No daemon lock is held across the walk. The project key comes from the
/// cwd discovery knows, else the watcher's cwd cache, else the queue's
/// entry at that path, else the unknown bucket -- never from a `load`.
pub fn discover_sessions(shared: &DaemonShared) -> Vec<DiscoveredSession> {
    let roots = shared.source_roots_with_routing();
    let mut refs: Vec<(SessionRef, DateTime<Utc>)> = Vec::new();
    for source in all_sources(&roots) {
        let Ok(found) = source.discover() else {
            continue;
        };
        for session_ref in found {
            let modified_at = session_ref.group_modified_at.or_else(|| {
                std::fs::metadata(&session_ref.path)
                    .and_then(|m| m.modified())
                    .ok()
                    .map(DateTime::<Utc>::from)
            });
            // A file that cannot be stat'd is gone, as for the watcher.
            if let Some(modified_at) = modified_at {
                refs.push((session_ref, modified_at));
            }
        }
    }

    let cached: HashMap<String, String> = {
        let state = shared.state.lock().expect("state lock");
        refs.iter()
            .filter_map(|(r, _)| {
                let key = r.path.to_string_lossy().to_string();
                state
                    .cwd_cache
                    .get(&key)
                    .and_then(|hit| hit.project_key.clone())
                    .map(|project| (key, project))
            })
            .collect()
    };
    let queued: HashMap<std::path::PathBuf, String> = {
        let queue = shared.queue.lock().expect("queue lock");
        queue
            .all()
            .iter()
            .map(|e| (e.path.clone(), e.project_key.clone()))
            .collect()
    };

    refs.into_iter()
        .map(|(session_ref, modified_at)| {
            let project_key = match session_ref.cwd.as_deref() {
                Some(cwd) => super::policy::project_for(Some(cwd)).0,
                None => cached
                    .get(session_ref.path.to_string_lossy().as_ref())
                    .or_else(|| queued.get(&session_ref.path))
                    .cloned()
                    .unwrap_or_else(|| super::policy::project_for(None).0),
            };
            DiscoveredSession {
                session_id: session_id_for(&session_ref.path),
                project_key,
                session_ref,
                modified_at,
            }
        })
        .collect()
}

/// Every project key the daemon can resolve an id against for the picker:
/// the usual known set plus every project a declared source lists now.
pub fn known_project_keys(shared: &DaemonShared, discovered: &[DiscoveredSession]) -> Vec<String> {
    let policy = shared.policy.lock().expect("policy lock");
    let queue = shared.queue.lock().expect("queue lock");
    let mut keys =
        super::policy::known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()));
    keys.extend(discovered.iter().map(|d| d.project_key.clone()));
    keys.sort();
    keys.dedup();
    keys
}

/// The past sessions of `project_key`, newest first. See the module doc.
pub fn list_past_sessions(
    shared: &DaemonShared,
    project_key: &str,
    now: DateTime<Utc>,
) -> Vec<PastSessionRow> {
    rows_for(shared, &discover_sessions(shared), project_key, now)
}

/// `list_past_sessions` over a walk the caller already took.
///
/// Kept and dismissed sessions are not listed, nor is a session whose
/// latest offer was decided some other way (uploading, uploaded, refused,
/// failed): the picker is for sessions still open to a choice. A folder
/// whose rule is Never lists every session it would otherwise list as
/// `never`. A session still being written lists as `still_active`.
pub fn rows_for(
    shared: &DaemonShared,
    discovered: &[DiscoveredSession],
    project_key: &str,
    now: DateTime<Utc>,
) -> Vec<PastSessionRow> {
    let mode = shared
        .policy
        .lock()
        .expect("policy lock")
        .resolve(project_key);
    let quiescence_secs = shared
        .settings
        .lock()
        .expect("settings lock")
        .quiescence_secs;
    let queue = shared.queue.lock().expect("queue lock");
    // The latest offer at each path, superseded ones aside: queue order is
    // insertion order, so a later entry replaces an earlier one here.
    let mut latest_at: HashMap<&Path, &super::queue::QueueEntry> = HashMap::new();
    for e in queue.all() {
        if e.state != QueueState::Superseded {
            latest_at.insert(e.path.as_path(), e);
        }
    }

    let mut rows: Vec<(DateTime<Utc>, PastSessionRow)> = Vec::new();
    for d in discovered.iter().filter(|d| d.project_key == project_key) {
        let path = &d.session_ref.path;
        if queue.kept_at_path(path) || queue.dismissed_at_path(path) {
            continue;
        }
        let latest = latest_at.get(path.as_path()).copied();
        let queued_state = match latest.map(|e| e.state) {
            None => None,
            Some(QueueState::Pending) => Some(PastSessionState::Pending),
            Some(QueueState::Approved) => Some(PastSessionState::Approved),
            Some(QueueState::Expired) => Some(PastSessionState::Expired),
            Some(_) if mode == ProjectMode::Ignore => None,
            // Decided another way; not a past session to choose.
            Some(_) => continue,
        };
        let quiescent =
            now.signed_duration_since(d.modified_at).num_seconds() >= quiescence_secs as i64;
        let state = if mode == ProjectMode::Ignore {
            PastSessionState::Never
        } else if !quiescent {
            PastSessionState::StillActive
        } else {
            queued_state.unwrap_or(PastSessionState::NotQueued)
        };
        let entry = latest.filter(|_| queued_state.is_some());
        let started_at = match entry {
            Some(e) => e.shape.as_ref().and_then(|s| s.started_at),
            None => d.session_ref.started_at.or(Some(d.modified_at)),
        };
        let row = PastSessionRow {
            session_id: d.session_id.clone(),
            entry_id: entry.map(|e| e.entry_id),
            state,
            selectable: state.selectable(),
            started_at,
            duration_secs: entry
                .and_then(|e| e.shape.as_ref())
                .and_then(super::queue::SessionShape::duration_secs),
            title: entry.and_then(|e| e.title.clone()),
            size_bytes: entry.map_or(d.session_ref.size_bytes, |e| e.size_bytes),
            source: entry.map_or_else(
                || d.session_ref.displayed_source().to_string(),
                |e| {
                    e.declared_source
                        .clone()
                        .unwrap_or_else(|| e.source.clone())
                },
            ),
        };
        rows.push((started_at.unwrap_or(d.modified_at), row));
    }
    rows.sort_by(|(a_at, a), (b_at, b)| {
        b_at.cmp(a_at).then_with(|| a.session_id.cmp(&b.session_id))
    });
    rows.into_iter().map(|(_, row)| row).collect()
}
