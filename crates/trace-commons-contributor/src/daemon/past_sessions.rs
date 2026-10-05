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
use super::queue::{Queue, QueueEntry, QueueState};
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

impl DiscoveredSession {
    /// Whether the session has stopped growing: nothing written for
    /// `quiescence_secs`. The one test both the listing's `still_active` and
    /// the include's `session-still-active` apply, so they cannot disagree.
    pub fn quiescent(&self, now: DateTime<Utc>, quiescence_secs: u64) -> bool {
        now.signed_duration_since(self.modified_at).num_seconds() >= quiescence_secs as i64
    }
}

/// The latest offer at each path, superseded ones aside: queue order is
/// insertion order, so a later entry replaces an earlier one here.
fn latest_offers(queue: &Queue) -> HashMap<&Path, &QueueEntry> {
    let mut latest_at: HashMap<&Path, &QueueEntry> = HashMap::new();
    for e in queue.all() {
        if e.state != QueueState::Superseded {
            latest_at.insert(e.path.as_path(), e);
        }
    }
    latest_at
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
    let latest_at = latest_offers(&queue);

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
        let quiescent = d.quiescent(now, quiescence_secs);
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

// -- `include_past_sessions`: the picker's Continue ------------------------

/// The most session ids one include may name. The queue's own default cap,
/// so "Include every past session" of a folder the size of a full queue is
/// one call, and a runaway caller is refused before anything is read.
pub const MAX_SESSIONS_PER_INCLUDE: usize = 500;

/// Refusals of the whole call, and per-session skip labels. Fixed strings,
/// never a path, a session id or anything the caller sent.
pub const LABEL_SESSION_IDS_INVALID: &str = "session_ids-invalid";
pub const LABEL_TOO_MANY_SESSIONS: &str = "too-many-sessions";
pub const LABEL_SESSION_ID_UNRECOGNIZED: &str = "session-id-unrecognized";
pub const LABEL_PROJECT_MODE_NEVER: &str = "project-mode-never";
pub const LABEL_SESSION_STILL_ACTIVE: &str = "session-still-active";
pub const LABEL_HELD_FOR_REVIEW: &str = "held-for-review";
pub const LABEL_SESSION_DISMISSED: &str = "session-dismissed";
pub const LABEL_SESSION_KEPT: &str = "session-kept";
pub const LABEL_NOT_PENDING: &str = "not-pending";
pub const LABEL_SESSION_PROJECT_CHANGED: &str = "session-project-changed";
pub const LABEL_SESSION_FILE_VANISHED: &str = "session-file-vanished";
pub const LABEL_PROJECT_ID_INVALID: &str = "project_id-invalid";
pub const LABEL_SESSION_UNREADABLE: &str = "session-unreadable";

/// The audit row an include writes before it changes anything.
pub const AUDIT_PAST_SESSIONS_INCLUDED: &str = "past-sessions-included";

/// One chosen session the include did not approve, and why.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedSession {
    pub session_id: String,
    pub label: &'static str,
}

/// What an include came to. Every distinct id asked for is counted in
/// `approved` or listed in `skipped`, exactly once.
#[derive(Debug, Clone, Serialize)]
pub struct IncludeOutcome {
    pub approved: usize,
    pub skipped: Vec<SkippedSession>,
}

/// A refusal of the whole call: an IPC error code and a fixed label.
pub type IncludeRefusal = (&'static str, &'static str);

/// Approve the chosen past sessions of `project_key` as a person's
/// approval: pinned to a preview, held for the undo window, and recorded as
/// the person's own, so a later change of the folder's rule does not take
/// it back. Never `include_backlog`: each session is named.
///
/// The whole call is validated before anything changes. The Never
/// contribution override, a Never folder, and any id that is not one of
/// this folder's sessions in `discovered` refuse every session; then a
/// `past-sessions-included` audit row (label and count only) is written,
/// and an unwritable log refuses the call too. Only then is anything
/// revived, queued or approved.
///
/// Per session: kept and dismissed sessions are never revived; a session
/// still being written is skipped `session-still-active` and never queued
/// half-written; an offer held for a person's review is skipped
/// `held-for-review`, as a group approve leaves it; an expired offer is
/// revived; a session never offered is read and queued now, past the queue
/// cap (`watcher::offer_for_a_person`); anything already decided is
/// `not-pending`.
pub async fn include_past_sessions(
    shared: &DaemonShared,
    discovered: &[DiscoveredSession],
    project_key: &str,
    session_ids: &[String],
    now: DateTime<Utc>,
) -> Result<IncludeOutcome, IncludeRefusal> {
    use super::ipc::{ERR_BAD_PARAMS, ERR_UNAVAILABLE};

    // Distinct, in the order asked.
    let mut seen = std::collections::HashSet::new();
    let ids: Vec<&String> = session_ids.iter().filter(|id| seen.insert(*id)).collect();
    if ids.is_empty() {
        return Err((ERR_BAD_PARAMS, LABEL_SESSION_IDS_INVALID));
    }
    if ids.len() > MAX_SESSIONS_PER_INCLUDE {
        return Err((ERR_BAD_PARAMS, LABEL_TOO_MANY_SESSIONS));
    }
    {
        let policy = shared.policy.lock().expect("policy lock");
        // The "Never" contribution override promises nothing is sent
        // (#1208); refused as `approve` refuses it.
        if policy.holds_every_send() {
            return Err((ERR_BAD_PARAMS, super::ipc::ERR_CONTRIBUTION_OVERRIDE_NEVER));
        }
        if policy.resolve(project_key) == ProjectMode::Ignore {
            return Err((ERR_BAD_PARAMS, LABEL_PROJECT_MODE_NEVER));
        }
    }
    let by_id: HashMap<&str, &DiscoveredSession> = discovered
        .iter()
        .filter(|d| d.project_key == project_key)
        .map(|d| (d.session_id.as_str(), d))
        .collect();
    let mut chosen: Vec<&DiscoveredSession> = Vec::with_capacity(ids.len());
    for id in &ids {
        let Some(d) = by_id.get(id.as_str()) else {
            return Err((ERR_BAD_PARAMS, LABEL_SESSION_ID_UNRECOGNIZED));
        };
        chosen.push(d);
    }

    // The record first, as `approve`'s `bulk-approved` row: a rollback that
    // has to write to the disk that just refused a write is not a rollback.
    // The label is derived from the key the daemon holds, never from the
    // caller's string.
    let known = known_project_keys(shared, discovered);
    let project_label = super::policy::disambiguated_label(
        project_key,
        super::policy::display_path_for_key(project_key).as_deref(),
        &known,
    );
    if super::audit::append(
        &shared.store,
        &super::audit::AuditEntry {
            at: now,
            action: AUDIT_PAST_SESSIONS_INCLUDED.to_string(),
            project_label: Some(project_label),
            detail: Some(chosen.len().to_string()),
        },
    )
    .is_err()
    {
        return Err((ERR_UNAVAILABLE, "audit-write-failed"));
    }

    let quiescence_secs = shared
        .settings
        .lock()
        .expect("settings lock")
        .quiescence_secs;
    let mut skipped: Vec<SkippedSession> = Vec::new();
    let skip = |d: &DiscoveredSession, label: &'static str| SkippedSession {
        session_id: d.session_id.clone(),
        label,
    };
    // Entry ids to approve, each with the session it stands for.
    let mut to_approve: Vec<(Uuid, &DiscoveredSession)> = Vec::new();
    let mut to_offer: Vec<&DiscoveredSession> = Vec::new();
    {
        let mut queue = shared.queue.lock().expect("queue lock");
        let latest: Vec<Option<(Uuid, QueueState, bool)>> = {
            let latest_at = latest_offers(&queue);
            chosen
                .iter()
                .map(|d| {
                    latest_at.get(d.session_ref.path.as_path()).map(|e| {
                        (
                            e.entry_id,
                            e.state,
                            // Held now, or held once before it aged out: the
                            // expiry overwrote the hold's label, and the
                            // review clock is what still says so.
                            e.held_for_review()
                                || (e.state == QueueState::Expired
                                    && e.review_started_at.is_some()),
                        )
                    })
                })
                .collect()
        };
        for (d, latest) in chosen.iter().copied().zip(latest) {
            let path = d.session_ref.path.as_path();
            if queue.dismissed_at_path(path) {
                skipped.push(skip(d, LABEL_SESSION_DISMISSED));
            } else if queue.kept_at_path(path) {
                skipped.push(skip(d, LABEL_SESSION_KEPT));
            } else if !d.quiescent(now, quiescence_secs) {
                skipped.push(skip(d, LABEL_SESSION_STILL_ACTIVE));
            } else {
                match latest {
                    None => to_offer.push(d),
                    Some((_, _, true)) => skipped.push(skip(d, LABEL_HELD_FOR_REVIEW)),
                    Some((id, QueueState::Pending, false)) => to_approve.push((id, d)),
                    Some((id, QueueState::Expired, false)) => {
                        if queue.revive_expired(id, now) {
                            to_approve.push((id, d));
                        } else {
                            skipped.push(skip(d, LABEL_NOT_PENDING));
                        }
                    }
                    Some(_) => skipped.push(skip(d, LABEL_NOT_PENDING)),
                }
            }
        }
    }

    if !to_offer.is_empty() {
        let refs: Vec<SessionRef> = to_offer.iter().map(|d| d.session_ref.clone()).collect();
        let offered = super::run_blocking(|| {
            super::watcher::offer_for_a_person(shared, now, &refs, project_key)
        });
        for (d, offered) in to_offer.into_iter().zip(offered) {
            match offered {
                Ok(id) => to_approve.push((id, d)),
                Err(label) => skipped.push(skip(d, label)),
            }
        }
    }

    if to_approve.is_empty() {
        return Ok(IncludeOutcome {
            approved: 0,
            skipped,
        });
    }
    // The terms in force now, read as `approve` reads them; see there.
    let cfg = shared.store.load_config().ok().flatten();
    let scopes = cfg
        .as_ref()
        .map(|c| c.consent_scopes.clone())
        .unwrap_or_default();
    let (near_ai, attested_bodies, approval_hold_secs) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.near_ai.clone(),
            s.ironwire_attested_bodies,
            s.approval_hold_secs,
        )
    };
    let inputs = cfg
        .as_ref()
        .map(|c| super::preview::input_fingerprint(c, near_ai.as_ref(), attested_bodies));
    let terms = super::ipc::ApprovalTerms {
        cfg: cfg.as_ref(),
        scopes: &scopes,
        inputs: inputs.as_deref(),
        verdict: None,
        correction: None,
        // The call's one instant, as `approve` takes one.
        approved_at: now,
        approval_hold_secs,
    };
    let entry_ids: Vec<Uuid> = to_approve.iter().map(|(id, _)| *id).collect();
    let batch = super::ipc::approve_as_a_person(shared, &entry_ids, &terms)
        .await
        .map_err(|label| (ERR_UNAVAILABLE, label))?;
    let session_of: HashMap<Uuid, &DiscoveredSession> = to_approve.into_iter().collect();
    for (id, label) in batch.skipped {
        if let Some(d) = session_of.get(&id) {
            skipped.push(skip(d, label));
        }
    }
    Ok(IncludeOutcome {
        approved: batch.approved_ids.len(),
        skipped,
    })
}
