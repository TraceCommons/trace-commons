//! Matching contribution missions on this Mac (K16, #1173).
//!
//! The rules are the consent design's "Missions" section:
//!
//! - **M1. Matching stays on this Mac.** It reads only sessions from
//!   adapters the contributor has left on, in folders not set to Never, and
//!   under a Never contribution override it reads nothing. What it reads,
//!   and what it finds, go into no network request, audit row or log line.
//! - **M2. A mission sends nothing by itself.** Matching takes the policy
//!   by shared reference and the queue only to read, never mutably: it
//!   cannot arm, approve or widen anything. A session counts toward a
//!   mission only when it is contributed through one of the three consent
//!   paths; a kept session explicitly declined that, and a withdrawn one
//!   reversed it, so [`readable_sessions`] excludes both (#1227 review).
//!   An already-uploaded session, and one still being written
//!   (not-yet-quiescent), stay candidates -- neither has been kept or
//!   withdrawn, and quiescence is a fact about *when* a session is ready to
//!   read, not about whether it was ever offered.
//!
//! [`readable_sessions`] is where M1's and M2's read rules live, and the
//! only place: everything after it, including
//! [`crate::contribution_missions::match_missions`], sees only what it let
//! through. The handler ([`handle_mission_matches`]) applies no gate of its
//! own before calling it -- a pre-filter there would make the rule live in
//! two places, and removing one silently would stop being caught by a test
//! that only exercises the other (#1227 review, finding 1).
//!
//! # Nits carried over from review (#1227)
//!
//! - **The adapter, not the self-declared tool, is what gates reading.**
//!   [`SeenSession::adapter`] is `SessionRef::source` -- the adapter that
//!   actually read the bytes -- recorded in [`super::state::CwdCacheEntry`]
//!   when the cache entry is written. `SeenSession::tool` is the
//!   contributor-facing display name (`SessionRef::displayed_source`),
//!   self-declared for a staged import and never gated on here, only used
//!   to label a readable session once it has already passed the adapter
//!   gate. A cache entry written before the adapter field existed carries
//!   `adapter: None` and is not read: fails closed, same as a missing
//!   `tool`, rather than trusting an un-recorded adapter.
//! - **The unknown-project bucket is read.** A session whose folder key is
//!   [`UNKNOWN_PROJECT_KEY`] resolves to `NotifyOnly`
//!   (`ProjectPolicy::resolve`), not `Ignore`, so it is read and counted in
//!   `read.folders` like any other folder not set to Never. It is kept out
//!   of the language probe (`local_facts` never asks `languages_of` about
//!   it), because it has no folder root to look at.
//! - **An unset `claude-code` or `codex` declaration counts as on.**
//!   `SourceRoots::source_identities` gives both a conventional root when
//!   the contributor has never been asked (`Undeclared::Conventional`),
//!   matching what the watcher reads from by default. Gemini, Cline and
//!   OpenCode are `Undeclared::Nothing` and count as off until declared.
//! - **The case-folded language-root fallback fails closed.** When a
//!   folder has no `display_path` recorded in the policy (never armed, or
//!   discovered only through the cwd cache), [`handle_mission_matches`]
//!   falls back to the project key itself, which is case-folded on macOS
//!   and Windows. On a case-sensitive volume that fallback can miss a
//!   marker file that is really there under its original case, so
//!   `languages_at` under-reports rather than over-reports -- the safe
//!   direction for a signal that can only add a mission, never remove one.
//!
//! Every known adapter name is registered in `source::NATIVE_SOURCES`
//! verbatim as `SessionRef::source`, so gating on the adapter needs no
//! alias: unlike the old `tool` gate, there is no case here where an
//! imported Antigravity conversation's adapter reads as anything other than
//! `trajectory`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::contribution_missions::{
    CatalogueError, ContributionMissionCatalogue, LANGUAGE_MARKERS, LocalFacts, SessionFact,
    match_missions, missions_fitting,
};

use super::history::{HistoryCache, is_taken_back};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use super::policy::{ProjectMode, ProjectPolicy, UNKNOWN_PROJECT_KEY};
use super::queue::QueueEntry;

/// The one log line matching writes: a label, nothing about what was read
/// or found (M1).
pub const LOG_LABEL: &str = "mission-matches-answered";

/// How long a received contribution-mission catalogue stays live. Past
/// this, the slot reads as empty: `mission_fit` goes absent (unknown), never
/// to zero.
///
/// OWNER DECISION V2 (nudge value addendum): 24 hours.
pub const MISSION_CATALOGUE_MAX_AGE: std::time::Duration =
    std::time::Duration::from_secs(24 * 60 * 60);

/// The contribution-mission catalogue this daemon holds, for counting how
/// many matched missions each waiting session fits (`list_pending`'s
/// `mission_fit`).
///
/// - **Memory only.** Never persisted: a restarted daemon starts empty, and
///   `unenroll` empties it. Empty means unknown.
/// - **One writer.** The daemon's scheduled refresh
///   ([`super::activity_missions::refresh_mission_slot`]) fills it from the
///   published activity catalogue's missions that carry a supported
///   predicate, and empties it when none do. `mission_matches` never writes
///   it -- its catalogue is a parameter, and matching changes nothing (M2).
///   Tests also seed it with [`receive_catalogue`].
/// - **Holds the parsed catalogue, when it arrived, and a per-folder
///   language cache** -- nothing derived from which sessions exist.
///   Replacing the catalogue drops the cache.
/// - **Ages out** after [`MISSION_CATALOGUE_MAX_AGE`], or sooner at the end
///   of the policy it came from; an expired slot reads as empty.
#[derive(Debug, Clone)]
pub struct MissionCatalogueSlot {
    catalogue: ContributionMissionCatalogue,
    received_at: DateTime<Utc>,
    /// The instant the published policy's missions stop being on offer
    /// (its `ends_before`, at 00:00 UTC), when the catalogue came from one.
    /// The slot is not live from then on, whatever its age.
    not_after: Option<DateTime<Utc>>,
    /// Languages of the folders the join has already looked at. Filled with
    /// no lock held, the first time `list_pending` sees a folder, and only
    /// while the catalogue asks about languages.
    folder_languages: BTreeMap<String, BTreeSet<String>>,
}

impl MissionCatalogueSlot {
    /// Whether this catalogue may still be read at `now`. A `received_at`
    /// in the future (the clock moved back) is not live either: unknown,
    /// the safe direction.
    fn is_live(&self, now: DateTime<Utc>) -> bool {
        let age = now.signed_duration_since(self.received_at);
        age >= chrono::TimeDelta::zero()
            && chrono::TimeDelta::from_std(MISSION_CATALOGUE_MAX_AGE).is_ok_and(|max| age <= max)
            && self.not_after.is_none_or(|end| now < end)
    }
}

/// Put a catalogue into the slot, read with
/// [`ContributionMissionCatalogue::from_value`]. A catalogue that is read
/// replaces whatever was there, cache included, never merging into it. One
/// that is refused (newer schema, malformed, over a bound) empties the
/// slot: a stale catalogue is never kept in its place.
pub fn receive_catalogue(
    slot: &Mutex<Option<MissionCatalogueSlot>>,
    raw: &serde_json::Value,
    now: DateTime<Utc>,
) -> Result<(), CatalogueError> {
    receive_catalogue_until(slot, raw, now, None)
}

/// [`receive_catalogue`] for a catalogue that is on offer only until
/// `not_after`: from then on the slot reads as empty, even inside
/// [`MISSION_CATALOGUE_MAX_AGE`].
pub fn receive_catalogue_until(
    slot: &Mutex<Option<MissionCatalogueSlot>>,
    raw: &serde_json::Value,
    now: DateTime<Utc>,
    not_after: Option<DateTime<Utc>>,
) -> Result<(), CatalogueError> {
    let read = ContributionMissionCatalogue::from_value(raw);
    let mut slot = slot.lock().expect("mission catalogue lock");
    match read {
        Ok(catalogue) => {
            *slot = Some(MissionCatalogueSlot {
                catalogue,
                received_at: now,
                not_after,
                folder_languages: BTreeMap::new(),
            });
            Ok(())
        }
        Err(e) => {
            *slot = None;
            Err(e)
        }
    }
}

/// Empty the slot. `unenroll` calls this: a next account starts unknown.
pub fn clear_catalogue(slot: &Mutex<Option<MissionCatalogueSlot>>) {
    *slot.lock().expect("mission catalogue lock") = None;
}

/// The catalogue the slot holds, when it is live at `now`.
pub fn live_catalogue(
    slot: &Mutex<Option<MissionCatalogueSlot>>,
    now: DateTime<Utc>,
) -> Option<ContributionMissionCatalogue> {
    let slot = slot.lock().expect("mission catalogue lock");
    slot.as_ref()
        .filter(|s| s.is_live(now))
        .map(|s| s.catalogue.clone())
}

/// A session the daemon has seen, as its cwd cache records it: the
/// self-declared tool (display only), the adapter that actually read it,
/// the folder's project key, and the cache's own path key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenSession {
    pub tool: Option<String>,
    pub adapter: Option<String>,
    pub folder: String,
    /// The cwd cache's own key: the session file's path. Used only to look
    /// up whether this session is kept or withdrawn (M2) -- never put into
    /// a [`SessionFact`], logged, or sent.
    pub path: String,
}

/// Whether sessions from `adapter` may be read: it is one the contributor
/// has left on. `adapter` is `SessionRef::source`, the adapter that
/// actually discovered the session -- never the self-declared display name
/// (`SessionRef::displayed_source`), which a staged import can spoof
/// (#1227 review, finding 2).
pub fn adapter_is_on(adapter: &str, tools_on: &BTreeSet<String>) -> bool {
    tools_on.contains(adapter)
}

/// M1's and M2's read rule: the sessions matching may read, out of
/// everything seen. The only place either rule lives (#1227 review, finding
/// 1): the handler applies no gate of its own before calling this.
///
/// - A session whose adapter is switched off is not read, nor one with no
///   recorded adapter -- a cache entry written before that field existed --
///   which cannot be shown to be on. That is fail-closed, not a default
///   grant (M1).
/// - A session in a folder whose mode in force is Never is not read. That
///   is `resolve`, not the folder's own mode, so a Never contribution
///   override (#1208) makes every folder Never and nothing is read; and a
///   folder set to Never stays Never under every other override (M1).
/// - A session that is kept, or whose upload was withdrawn, is not read:
///   a mission counts a session only when it is contributed through one of
///   the three consent paths, and a keep explicitly declined that while a
///   withdrawal reversed it (M2). An already-uploaded session that is
///   neither kept nor withdrawn, and one still being written
///   (not-yet-quiescent), are excluded from neither set and stay readable.
/// - Folders that are not armed are read; nothing read leaves the device.
///
/// `policy` is borrowed, never mutably: matching changes nothing (M2).
pub fn readable_sessions(
    policy: &ProjectPolicy,
    tools_on: &BTreeSet<String>,
    excluded_paths: &BTreeSet<String>,
    seen: &[SeenSession],
) -> Vec<SessionFact> {
    seen.iter()
        .filter_map(|s| {
            let tool = s.tool.as_deref()?;
            let adapter = s.adapter.as_deref()?;
            (adapter_is_on(adapter, tools_on)
                && policy.resolve(&s.folder) != ProjectMode::Ignore
                && !excluded_paths.contains(&s.path))
            .then(|| SessionFact {
                tool: tool.to_string(),
                folder: s.folder.clone(),
            })
        })
        .collect()
}

/// The facts the matcher gets: the readable sessions, and, only when a
/// mission asks about languages, the languages of the folders they ran in.
/// `languages_of` is asked about readable folders only, and never about the
/// unknown bucket, which has no folder to look in.
pub fn local_facts(
    sessions: Vec<SessionFact>,
    needs_languages: bool,
    mut languages_of: impl FnMut(&str) -> BTreeSet<String>,
) -> LocalFacts {
    let mut folder_languages = BTreeMap::new();
    if needs_languages {
        let folders: BTreeSet<&str> = sessions
            .iter()
            .map(|s| s.folder.as_str())
            .filter(|f| *f != UNKNOWN_PROJECT_KEY)
            .collect();
        for folder in folders {
            folder_languages.insert(folder.to_string(), languages_of(folder));
        }
    }
    LocalFacts {
        sessions,
        folder_languages,
    }
}

/// The languages a folder is worked in, from which of
/// [`LANGUAGE_MARKERS`] sit at its root. Existence only: no file is opened,
/// and a link is not followed.
pub fn languages_at(root: &Path) -> BTreeSet<String> {
    LANGUAGE_MARKERS
        .iter()
        .filter(|(marker, _)| {
            std::fs::symlink_metadata(root.join(marker)).is_ok_and(|m| m.is_file())
        })
        .map(|(_, language)| (*language).to_string())
        .collect()
}

/// What M1 and M2 gate reading on, read out of the daemon: the adapters
/// the contributor has left on, and the paths of the sessions that are
/// kept or whose upload was withdrawn. Handed to [`readable_sessions`],
/// which is where the rules themselves live; nothing here filters.
pub(crate) struct ReadGates {
    pub(crate) tools_on: BTreeSet<String>,
    pub(crate) excluded_paths: BTreeSet<String>,
}

/// Read the [`ReadGates`] in force, each input under its own lock and
/// released, none mutably.
pub(crate) fn read_gates(shared: &DaemonShared) -> ReadGates {
    let tools_on: BTreeSet<String> = {
        let settings = shared.settings.lock().expect("settings lock");
        settings
            .source_roots(&shared.store)
            .source_identities()
            .into_keys()
            .map(str::to_string)
            .collect()
    };
    // Kept and withdrawn sessions can never count toward a mission (M2): a
    // keep is the contributor explicitly declining the one path a session
    // could have counted through, and a withdrawal reverses a contribution
    // that already happened. Both are read here, never written -- matching
    // changes neither the queue nor the history cache. An already-uploaded
    // session that is neither kept nor withdrawn, and one still being
    // written, are excluded from neither set and stay candidates: see the
    // module doc.
    let excluded_paths: BTreeSet<String> = {
        let queue = shared.queue.lock().expect("queue lock");
        let withdrawn_submissions: BTreeSet<uuid::Uuid> = HistoryCache::load(&shared.store)
            .unwrap_or_default()
            .iter()
            .filter(|r| is_taken_back(r))
            .map(|r| r.submission_id)
            .collect();
        queue
            .all()
            .iter()
            .filter(|e| {
                e.is_kept()
                    || e.submission_id
                        .is_some_and(|id| withdrawn_submissions.contains(&id))
            })
            .map(|e| e.path.to_string_lossy().to_string())
            .collect()
    };
    ReadGates {
        tools_on,
        excluded_paths,
    }
}

/// Where a folder's language markers are looked for: the unfolded path the
/// policy recorded for it, else the project key itself (the case-folded
/// fallback the module doc describes, which under-reports).
fn language_root(policy: &ProjectPolicy, folder: &str) -> PathBuf {
    let shown = policy
        .projects
        .get(folder)
        .and_then(|e| e.display_path.clone())
        .unwrap_or_else(|| folder.to_string());
    PathBuf::from(shown)
}

/// The facts matching gets from this daemon's cwd cache: the sessions
/// [`readable_sessions`] lets through under `gates`, and, when
/// `needs_languages`, the languages `languages_of` gives for each of their
/// folders. `languages_of` is called with no lock held, with the folder's
/// key and its [`language_root`].
///
/// Shared by `mission_matches`, which probes the folder live, and the
/// per-entry join behind `list_pending`'s `mission_fit`, which reads the
/// catalogue slot's cache first.
pub(crate) fn current_facts(
    shared: &DaemonShared,
    gates: &ReadGates,
    needs_languages: bool,
    mut languages_of: impl FnMut(&str, &Path) -> BTreeSet<String>,
) -> LocalFacts {
    // A key the cache has not filled in yet is worked out here and not
    // written back: matching leaves the cache as it found it.
    let seen: Vec<SeenSession> = {
        let state = shared.state.lock().expect("state lock");
        state
            .cwd_cache
            .iter()
            .map(|(path, e)| SeenSession {
                tool: e.tool.clone(),
                adapter: e.adapter.clone(),
                folder: e
                    .project_key
                    .clone()
                    .unwrap_or_else(|| super::policy::project_for(e.cwd.as_deref()).0),
                path: path.clone(),
            })
            .collect()
    };
    let (sessions, roots) = {
        let policy = shared.policy.lock().expect("policy lock");
        let sessions = readable_sessions(&policy, &gates.tools_on, &gates.excluded_paths, &seen);
        let roots: BTreeMap<String, PathBuf> = sessions
            .iter()
            .map(|s| (s.folder.clone(), language_root(&policy, &s.folder)))
            .collect();
        (sessions, roots)
    };
    local_facts(sessions, needs_languages, |folder| {
        roots
            .get(folder)
            .map(|root| languages_of(folder, root))
            .unwrap_or_default()
    })
}

/// How many of the live catalogue's matched missions each pending queue
/// entry fits, by entry id: `list_pending`'s `mission_fit`. `None` when the
/// slot holds no live catalogue -- unknown, which a caller renders as an
/// absent field, never as zero.
pub(crate) fn pending_mission_fit(
    shared: &DaemonShared,
    now: DateTime<Utc>,
) -> Option<BTreeMap<Uuid, usize>> {
    let join = MissionJoin::read(shared, now, Probe::Folders)?;
    let policy = shared.policy.lock().expect("policy lock");
    let queue = shared.queue.lock().expect("queue lock");
    Some(
        queue
            .pending()
            .into_iter()
            .map(|e| (e.entry_id, join.fit(&policy, e)))
            .collect(),
    )
}

/// Whether building a [`MissionJoin`] may look at folder roots for their
/// languages (OWNER DECISION V4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Probe {
    /// `list_pending`: a folder the slot's cache has not seen is looked at,
    /// with no lock held, and the answer is written back to the cache.
    Folders,
    /// `status`: the cache only. A folder it has not seen has no known
    /// language, so it fits no language criterion.
    CacheOnly,
}

/// Everything needed to count, per queue entry, the live catalogue's
/// matched missions it fits, read once with every lock released again.
/// Built before the caller takes the policy and queue locks it renders
/// under; [`MissionJoin::fit`] then needs only those guards.
pub(crate) struct MissionJoin {
    catalogue: ContributionMissionCatalogue,
    matched: Vec<String>,
    facts: LocalFacts,
    gates: ReadGates,
}

impl MissionJoin {
    /// `None` when the slot holds no live catalogue: unknown, which every
    /// caller renders as an absent field, never as zero.
    pub(crate) fn read(shared: &DaemonShared, now: DateTime<Utc>, probe: Probe) -> Option<Self> {
        // The slot is a leaf lock: cloned out and released straight away.
        let (catalogue, received_at, cached) = {
            let slot = shared
                .mission_catalogue
                .lock()
                .expect("mission catalogue lock");
            let slot = slot.as_ref().filter(|s| s.is_live(now))?;
            (
                slot.catalogue.clone(),
                slot.received_at,
                slot.folder_languages.clone(),
            )
        };
        let needs_languages = catalogue.needs_languages();
        let gates = read_gates(shared);
        let mut probed: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut languages_of = |folder: &str, root: &Path| -> BTreeSet<String> {
            if let Some(known) = cached.get(folder).or_else(|| probed.get(folder)) {
                return known.clone();
            }
            match probe {
                Probe::CacheOnly => BTreeSet::new(),
                Probe::Folders => {
                    let found = languages_at(root);
                    probed.insert(folder.to_string(), found.clone());
                    found
                }
            }
        };
        let mut facts = current_facts(shared, &gates, needs_languages, &mut languages_of);
        if needs_languages {
            // The waiting entries' folders as well: an entry can sit in a
            // folder the cwd cache holds no readable session for. Adding
            // their languages cannot change `match_missions`, which looks
            // a language up only for a readable session's own folder.
            //
            // An entry's folder is probed only when `readable_sessions` lets
            // the entry itself through, the same gate `fit` applies: never
            // for an entry whose adapter is off, in a Never folder or under
            // a Never override, or kept or withdrawn. Checking which files
            // exist at a folder's root is reading it (M1). An entry queued
            // before its folder went Never, or before its adapter was
            // switched off, is still waiting.
            let roots: Vec<(String, PathBuf)> = {
                let entries: Vec<SeenSession> = {
                    let queue = shared.queue.lock().expect("queue lock");
                    queue
                        .pending()
                        .into_iter()
                        .filter(|e| e.project_key != UNKNOWN_PROJECT_KEY)
                        .filter(|e| !facts.folder_languages.contains_key(&e.project_key))
                        .map(|e| SeenSession {
                            tool: Some(e.displayed_source().to_string()),
                            adapter: Some(e.source.clone()),
                            folder: e.project_key.clone(),
                            path: e.path.to_string_lossy().to_string(),
                        })
                        .collect()
                };
                let policy = shared.policy.lock().expect("policy lock");
                let entry_folders: BTreeSet<String> =
                    readable_sessions(&policy, &gates.tools_on, &gates.excluded_paths, &entries)
                        .into_iter()
                        .map(|s| s.folder)
                        .collect();
                entry_folders
                    .into_iter()
                    .map(|f| {
                        let root = language_root(&policy, &f);
                        (f, root)
                    })
                    .collect()
            };
            for (folder, root) in roots {
                let langs = languages_of(&folder, &root);
                facts.folder_languages.insert(folder, langs);
            }
        }
        if !probed.is_empty() {
            // Written back only into the catalogue it was probed for: a slot
            // replaced or emptied meanwhile keeps its own (empty) cache.
            let mut slot = shared
                .mission_catalogue
                .lock()
                .expect("mission catalogue lock");
            if let Some(slot) = slot
                .as_mut()
                .filter(|s| s.received_at == received_at && s.catalogue == catalogue)
            {
                slot.folder_languages.extend(probed);
            }
        }
        let matched = match_missions(&catalogue, &facts);
        Some(MissionJoin {
            catalogue,
            matched,
            facts,
            gates,
        })
    }

    /// How many matched missions `entry` fits. An entry
    /// [`readable_sessions`] would not let through -- adapter off, folder
    /// Never, session kept or withdrawn -- fits none: a known zero.
    pub(crate) fn fit(&self, policy: &ProjectPolicy, entry: &QueueEntry) -> usize {
        let seen = SeenSession {
            tool: Some(entry.displayed_source().to_string()),
            adapter: Some(entry.source.clone()),
            folder: entry.project_key.clone(),
            path: entry.path.to_string_lossy().to_string(),
        };
        readable_sessions(
            policy,
            &self.gates.tools_on,
            &self.gates.excluded_paths,
            std::slice::from_ref(&seen),
        )
        .first()
        .map_or(0, |session| {
            missions_fitting(&self.catalogue, &self.matched, session, &self.facts)
        })
    }
}

/// `mission_matches {catalogue}`: the ids of the catalogue's missions that
/// this contributor's local work fits, and how many tools and folders
/// matching was allowed to read -- counts only.
///
/// The catalogue is a parameter; the daemon fetches nothing here, and the
/// slot the scheduled refresh fills is neither read nor written. Every input is read under its own lock and
/// released, none mutably: no policy, queue, state or file is written, no
/// audit row is appended, and the log gets [`LOG_LABEL`] and nothing else
/// (M1, M2). The answer goes back over the local socket only.
pub fn handle_mission_matches(shared: &DaemonShared, req: &Request) -> Response {
    let Some(raw) = req.params.get("catalogue") else {
        return Response::err(req.id, ERR_BAD_PARAMS, "catalogue-required");
    };
    let catalogue = match ContributionMissionCatalogue::from_value(raw) {
        Ok(catalogue) => catalogue,
        Err(e) => return Response::err(req.id, ERR_BAD_PARAMS, e.label()),
    };
    let gates = read_gates(shared);
    let facts = current_facts(shared, &gates, catalogue.needs_languages(), |_, root| {
        languages_at(root)
    });
    let matches = match_missions(&catalogue, &facts);
    tracing::debug!("{LOG_LABEL}");
    Response::ok(
        req.id,
        serde_json::json!({
            "matches": matches,
            "read": {"tools": facts.tools_read(), "folders": facts.folders_read()},
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contribution_missions::{ContributionMissionCatalogue, match_missions};
    use chrono::Utc;
    use serde_json::json;

    fn seen(tool: Option<&str>, adapter: Option<&str>, folder: &str) -> SeenSession {
        // A unique-enough path: nothing here exercises the kept/withdrawn
        // exclusion, so it only needs to not collide within one `all` list.
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        SeenSession {
            tool: tool.map(str::to_string),
            adapter: adapter.map(str::to_string),
            folder: folder.to_string(),
            path: format!("/seen/{n}"),
        }
    }

    fn tools(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// A folder set to Never and an adapter switched off are not read: their
    /// sessions never reach the matcher, a Never folder's root is never
    /// looked at, and a mission only they would fit does not match. Nor is
    /// a session with no recorded adapter -- a cache entry written before
    /// that field existed -- read: that fails closed exactly like a missing
    /// tool, even though its folder and (self-declared) tool are otherwise
    /// fine (#1227 review, finding 2).
    #[test]
    fn never_folders_switched_off_adapters_and_legacy_entries_are_not_read() {
        let mut policy = ProjectPolicy::new();
        policy
            .set_mode("/work/never", ProjectMode::Ignore, Utc::now())
            .unwrap();
        let on = tools(&["claude-code", "trajectory"]);
        let all = vec![
            seen(Some("claude-code"), Some("claude-code"), "/work/ask"),
            seen(Some("claude-code"), Some("claude-code"), "/work/never"),
            seen(Some("codex"), Some("codex"), "/work/ask"),
            // An imported Antigravity conversation: displayed as
            // "antigravity", but read by the trajectory adapter, which is on.
            seen(Some("antigravity"), Some("trajectory"), "/work/ask"),
            seen(None, Some("claude-code"), "/work/ask"),
            // A legacy cache entry: a tool was recorded, but no adapter.
            seen(Some("claude-code"), None, "/work/ask"),
        ];
        let read = readable_sessions(&policy, &on, &BTreeSet::new(), &all);
        assert_eq!(
            read,
            vec![
                SessionFact {
                    tool: "claude-code".into(),
                    folder: "/work/ask".into()
                },
                SessionFact {
                    tool: "antigravity".into(),
                    folder: "/work/ask".into()
                },
            ]
        );

        let mut asked = Vec::new();
        let facts = local_facts(read, true, |folder| {
            asked.push(folder.to_string());
            // Only the Never folder is a Rust repository.
            if folder == "/work/never" {
                BTreeSet::from(["rust".to_string()])
            } else {
                BTreeSet::new()
            }
        });
        assert_eq!(asked, vec!["/work/ask"], "a Never folder is not looked at");
        let cat = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1,
            "missions": [
                {"mission_id": "codex", "criteria": {"tools": ["codex"]}},
                {"mission_id": "rust", "criteria": {"languages": ["rust"]}},
                {"mission_id": "claude-2", "criteria": {"tools": ["claude-code"], "min_sessions": 2}},
                {"mission_id": "claude", "criteria": {"tools": ["claude-code"]}},
            ],
        }))
        .unwrap();
        assert_eq!(match_missions(&cat, &facts), vec!["claude"]);
        assert_eq!((facts.tools_read(), facts.folders_read()), (2, 1));
    }

    /// Under a Never contribution override nothing is read, from any
    /// folder, whatever its own mode, so nothing matches.
    #[test]
    fn the_never_override_reads_nothing() {
        let mut policy = ProjectPolicy::new();
        policy
            .set_mode("/work/ask", ProjectMode::NotifyOnly, Utc::now())
            .unwrap();
        policy
            .set_contribution_override(ProjectMode::Ignore, Utc::now(), None)
            .unwrap();
        let on = tools(&["claude-code"]);
        let all = vec![
            seen(Some("claude-code"), Some("claude-code"), "/work/ask"),
            seen(
                Some("claude-code"),
                Some("claude-code"),
                "/work/unconfigured",
            ),
            seen(
                Some("claude-code"),
                Some("claude-code"),
                UNKNOWN_PROJECT_KEY,
            ),
        ];
        let read = readable_sessions(&policy, &on, &BTreeSet::new(), &all);
        assert!(read.is_empty(), "{read:?}");
        let facts = local_facts(read, true, |folder| {
            panic!("no folder is looked at under a Never override: {folder}")
        });
        let cat = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1, "missions": [{"mission_id": "any"}],
        }))
        .unwrap();
        assert!(match_missions(&cat, &facts).is_empty());
        assert_eq!((facts.tools_read(), facts.folders_read()), (0, 0));
    }

    /// A staged import that declares itself `claude-code` is read by
    /// whichever adapter actually discovered it -- `trajectory`, for one
    /// staged as a trajectory file -- and gating follows that adapter, not
    /// the self-declared claim. With trajectory off and claude-code on, the
    /// import is not read, and a mission asking only for `claude-code` does
    /// not match on it; once trajectory is also on, the same import is read
    /// and counts under the name it declared (#1227 review, finding 2).
    #[test]
    fn a_staged_claude_code_import_is_gated_on_its_adapter_not_its_claim() {
        let policy = ProjectPolicy::new();
        let staged = vec![seen(Some("claude-code"), Some("trajectory"), "/work/ask")];
        let cat = ContributionMissionCatalogue::from_value(&json!({
            "schema_version": 1,
            "missions": [{"mission_id": "claude", "criteria": {"tools": ["claude-code"]}}],
        }))
        .unwrap();

        let claude_code_only = tools(&["claude-code"]);
        let read = readable_sessions(&policy, &claude_code_only, &BTreeSet::new(), &staged);
        assert!(read.is_empty(), "{read:?}");
        let facts = local_facts(read, false, |_| BTreeSet::new());
        assert!(match_missions(&cat, &facts).is_empty());

        let both_on = tools(&["claude-code", "trajectory"]);
        let read = readable_sessions(&policy, &both_on, &BTreeSet::new(), &staged);
        let facts = local_facts(read, false, |_| BTreeSet::new());
        assert_eq!(match_missions(&cat, &facts), vec!["claude"]);
    }

    /// A session whose folder key is the unknown-project bucket resolves to
    /// `NotifyOnly`, not `Ignore`, so it is read and counted in
    /// `folders_read` like any other folder not set to Never -- but it is
    /// never asked about languages, since it has no folder root to look at
    /// (#1227 review, finding 4).
    #[test]
    fn the_unknown_project_bucket_is_read_but_never_language_probed() {
        let policy = ProjectPolicy::new();
        let on = tools(&["claude-code"]);
        let all = vec![
            seen(Some("claude-code"), Some("claude-code"), "/work/ask"),
            seen(
                Some("claude-code"),
                Some("claude-code"),
                UNKNOWN_PROJECT_KEY,
            ),
        ];
        let read = readable_sessions(&policy, &on, &BTreeSet::new(), &all);
        assert_eq!(read.len(), 2, "{read:?}");
        let mut asked = Vec::new();
        let facts = local_facts(read, true, |folder| {
            asked.push(folder.to_string());
            BTreeSet::new()
        });
        assert_eq!(
            asked,
            vec!["/work/ask"],
            "the unknown bucket is never looked at"
        );
        assert_eq!(facts.folders_read(), 2, "the unknown bucket still counts");
    }

    /// The marker files decide a folder's languages; a directory of the
    /// same name, or a link, does not.
    #[test]
    fn languages_come_from_marker_files_at_the_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        std::fs::create_dir(dir.path().join("go.mod")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("Cargo.toml"), dir.path().join("Gemfile"))
            .unwrap();
        assert_eq!(
            languages_at(dir.path()),
            BTreeSet::from(["rust".to_string()])
        );
        assert!(languages_at(&dir.path().join("missing")).is_empty());
    }
    // ---- mission_matches over IPC ----

    use super::super::history::{HistoryRecord, STATUS_ACCEPTED, STATUS_WITHDRAWN};
    use super::super::ipc::handle_request;
    use super::super::queue::{QueueEntry, QueueState};
    use super::super::settings::SourceDeclaration;
    use super::super::state::CwdCacheEntry;

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn ask(s: &DaemonShared, params: serde_json::Value) -> Response {
        handle_request(
            s,
            &Request {
                id: 7,
                method: "mission_matches".to_string(),
                params,
            },
        )
    }

    /// A daemon with Claude Code on and Codex off, an Ask me folder that is
    /// a Rust repository, a Never folder that is a Python one, and sessions
    /// in both from both tools -- one with its project key not yet cached.
    fn seeded() -> (tempfile::TempDir, DaemonShared, tempfile::TempDir) {
        let (dir, s) = shared();
        let work = tempfile::tempdir().unwrap();
        let ask_dir = work.path().join("ask-repo");
        let never_dir = work.path().join("never-repo");
        std::fs::create_dir_all(&ask_dir).unwrap();
        std::fs::create_dir_all(&never_dir).unwrap();
        std::fs::write(ask_dir.join("Cargo.toml"), "").unwrap();
        std::fs::write(never_dir.join("pyproject.toml"), "").unwrap();
        {
            let mut settings = s.settings.lock().unwrap();
            settings.claude_source = Some(SourceDeclaration::Watch {
                path: work.path().join("claude-root"),
            });
            settings.codex_source = Some(SourceDeclaration::Off);
        }
        let key = |p: &Path| super::super::policy::project_for(Some(p.to_str().unwrap())).0;
        let (ask_key, never_key) = (key(&ask_dir), key(&never_dir));
        s.policy
            .lock()
            .unwrap()
            .set_mode(&never_key, ProjectMode::Ignore, Utc::now())
            .unwrap();
        let mut state = s.state.lock().unwrap();
        let mut put = |path: &str, tool: &str, cwd: &Path, cached_key: Option<&str>| {
            state.cwd_cache.insert(
                path.to_string(),
                CwdCacheEntry {
                    size_bytes: 1,
                    modified_at: Utc::now(),
                    cwd: Some(cwd.to_str().unwrap().to_string()),
                    project_key: cached_key.map(str::to_string),
                    tool: Some(tool.to_string()),
                    adapter: Some(tool.to_string()),
                },
            );
        };
        put(
            "/s/claude-ask-1.jsonl",
            "claude-code",
            &ask_dir,
            Some(&ask_key),
        );
        put("/s/claude-ask-2.jsonl", "claude-code", &ask_dir, None);
        put(
            "/s/claude-never.jsonl",
            "claude-code",
            &never_dir,
            Some(&never_key),
        );
        put("/s/codex-ask.jsonl", "codex", &ask_dir, Some(&ask_key));
        drop(state);
        (dir, s, work)
    }

    fn catalogue() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "missions": [
                {"mission_id": "rust-claude", "title": "Rust with Claude",
                 "criteria": {"tools": ["claude-code"], "languages": ["rust"], "min_sessions": 2}},
                {"mission_id": "python", "title": "Python work",
                 "criteria": {"languages": ["python"]}},
                {"mission_id": "codex", "title": "Codex work",
                 "criteria": {"tools": ["codex"]}},
                {"mission_id": "claude-3", "title": "Three Claude sessions",
                 "criteria": {"tools": ["claude-code"], "min_sessions": 3}},
            ],
        })
    }

    /// Over IPC: the Never folder's sessions and the switched-off tool's
    /// are not read, so only the mission the Ask me folder's Claude Code
    /// sessions fit matches, and the counts say one tool, one folder.
    #[test]
    fn mission_matches_reads_only_tools_on_in_folders_not_never() {
        let (_dir, s, _work) = seeded();
        let result = ask(&s, json!({"catalogue": catalogue()})).result.unwrap();
        assert_eq!(
            result,
            json!({"matches": ["rust-claude"], "read": {"tools": 1, "folders": 1}})
        );
    }

    /// Under a Never contribution override, nothing is read and nothing
    /// matches.
    #[test]
    fn mission_matches_under_a_never_override_reads_nothing() {
        let (_dir, s, _work) = seeded();
        s.policy
            .lock()
            .unwrap()
            .set_contribution_override(ProjectMode::Ignore, Utc::now(), None)
            .unwrap();
        let result = ask(
            &s,
            json!({"catalogue": {"schema_version": 1, "missions": [{"mission_id": "any"}]}}),
        )
        .result
        .unwrap();
        assert_eq!(
            result,
            json!({"matches": [], "read": {"tools": 0, "folders": 0}})
        );
    }

    /// An IPC-level pin for finding 1 (#1227 review): the handler applies
    /// no adapter gate of its own before calling `readable_sessions`, so
    /// this fails if that gate is ever removed from `readable_sessions`.
    /// The pre-fix handler filtered the cache by the same rule before
    /// building `seen`, so the same scenario kept passing even with the
    /// gate missing from `readable_sessions` alone -- see the module doc.
    #[test]
    fn mission_matches_over_ipc_does_not_read_a_switched_off_adapter() {
        let (_dir, s, _work) = seeded();
        let result = ask(
            &s,
            json!({"catalogue": {
                "schema_version": 1,
                "missions": [{"mission_id": "codex", "criteria": {"tools": ["codex"]}}],
            }}),
        )
        .result
        .unwrap();
        assert_eq!(
            result,
            json!({"matches": [], "read": {"tools": 1, "folders": 1}}),
            "{result}"
        );
    }

    fn claude_two_mission() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "missions": [{"mission_id": "claude-2",
                "criteria": {"tools": ["claude-code"], "min_sessions": 2}}],
        })
    }

    /// A minimal history record, for the kept/withdrawn tests below. Most
    /// fields are irrelevant to them; only `submission_id` and `status`
    /// matter.
    fn history_record(submission_id: uuid::Uuid, status: &str) -> HistoryRecord {
        HistoryRecord {
            submission_id,
            submitted_at: Utc::now(),
            project_id: String::new(),
            project_label: "ask".to_string(),
            source: "claude-code".to_string(),
            session_hash: "sha256:seed".to_string(),
            status: status.to_string(),
            consent_scopes: vec![],
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanations: vec![],
            last_refreshed_at: None,
            withdrawn_at: None,
            revoked_at: None,
            approved_unattended: None,
            approved_verdict: None,
            uploaded_bytes: None,
        }
    }

    /// A kept session can never count toward a mission: M2 says a session
    /// counts only when it is contributed through one of the three consent
    /// paths, and a keep is the contributor explicitly declining that
    /// (#1227 review, finding 3).
    #[test]
    fn mission_matches_excludes_a_kept_session() {
        let (_dir, s, _work) = seeded();
        let before = ask(&s, json!({"catalogue": claude_two_mission()}))
            .result
            .unwrap();
        assert_eq!(before["matches"], json!(["claude-2"]), "{before}");

        let kept_id = uuid::Uuid::new_v4();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                QueueEntry {
                    entry_id: kept_id,
                    session_hash: "sha256:kept".to_string(),
                    source: "claude-code".to_string(),
                    project_key: "/seed".to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from("/s/claude-ask-1.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        s.queue.lock().unwrap().keep(kept_id).unwrap();

        let after = ask(&s, json!({"catalogue": claude_two_mission()}))
            .result
            .unwrap();
        assert_eq!(
            after,
            json!({"matches": [], "read": {"tools": 1, "folders": 1}}),
            "a kept session is not a candidate: {after}"
        );
    }

    /// A withdrawn session can never count toward a mission: the withdrawal
    /// reverses a contribution that already happened (#1227 review, finding
    /// 3).
    #[test]
    fn mission_matches_excludes_a_withdrawn_session() {
        let (_dir, s, _work) = seeded();
        let before = ask(&s, json!({"catalogue": claude_two_mission()}))
            .result
            .unwrap();
        assert_eq!(before["matches"], json!(["claude-2"]), "{before}");

        let submission_id = uuid::Uuid::new_v4();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                QueueEntry {
                    entry_id: uuid::Uuid::new_v4(),
                    session_hash: "sha256:withdrawn".to_string(),
                    source: "claude-code".to_string(),
                    project_key: "/seed".to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from("/s/claude-ask-1.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    state: QueueState::Uploaded,
                    submission_id: Some(submission_id),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        HistoryCache::save(&s.store, &[history_record(submission_id, STATUS_WITHDRAWN)]).unwrap();

        let after = ask(&s, json!({"catalogue": claude_two_mission()}))
            .result
            .unwrap();
        assert_eq!(
            after,
            json!({"matches": [], "read": {"tools": 1, "folders": 1}}),
            "a withdrawn session is not a candidate: {after}"
        );
    }

    /// An already-uploaded session that is neither kept nor withdrawn stays
    /// a candidate: M2 excludes only a keep (explicitly declined) and a
    /// withdrawal (reversed); an ordinary accepted upload did neither.
    /// `seeded()`'s own sessions, which carry no queue entry at all, are
    /// already the not-yet-quiescent case: this pins both halves of the
    /// module doc's claim (#1227 review, finding 3).
    #[test]
    fn mission_matches_still_counts_an_accepted_upload() {
        let (_dir, s, _work) = seeded();
        let submission_id = uuid::Uuid::new_v4();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                QueueEntry {
                    entry_id: uuid::Uuid::new_v4(),
                    session_hash: "sha256:accepted".to_string(),
                    source: "claude-code".to_string(),
                    project_key: "/seed".to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from("/s/claude-ask-1.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    state: QueueState::Uploaded,
                    submission_id: Some(submission_id),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        HistoryCache::save(&s.store, &[history_record(submission_id, STATUS_ACCEPTED)]).unwrap();

        let result = ask(&s, json!({"catalogue": catalogue()})).result.unwrap();
        assert_eq!(
            result,
            json!({"matches": ["rust-claude"], "read": {"tools": 1, "folders": 1}}),
            "an accepted upload that is neither kept nor withdrawn still counts: {result}"
        );
    }

    /// Every file in the store directory, by name.
    fn files(dir: &Path) -> BTreeMap<String, Vec<u8>> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.path().is_file())
            .map(|e| {
                (
                    e.file_name().to_string_lossy().to_string(),
                    std::fs::read(e.path()).unwrap(),
                )
            })
            .collect()
    }

    /// M2: matching changes no policy, queue or daemon state, in memory or
    /// on disk -- not even the cache key it had to work out.
    #[test]
    fn mission_matches_changes_no_policy_or_queue_state() {
        let (_dir, s, _work) = seeded();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                super::super::queue::QueueEntry {
                    entry_id: uuid::Uuid::new_v4(),
                    session_hash: "sha256:seed".to_string(),
                    source: "claude-code".to_string(),
                    project_key: "/seed".to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from("/s/claude-ask-1.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        s.policy.lock().unwrap().save(&s.store).unwrap();
        let snapshot = |s: &DaemonShared| {
            (
                s.policy.lock().unwrap().clone(),
                s.queue.lock().unwrap().all().to_vec(),
                serde_json::to_value(&s.state.lock().unwrap().cwd_cache).unwrap(),
                files(s.store.dir()),
            )
        };
        let before = snapshot(&s);
        let reply = ask(&s, json!({"catalogue": catalogue()}));
        assert!(reply.result.is_some(), "{reply:?}");
        assert!(
            before == snapshot(&s),
            "matching changed policy, queue or state"
        );
    }

    /// Records every event and span field a test's code emits.
    #[derive(Clone, Default)]
    struct Capture(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

    struct Fields(String);

    impl tracing::field::Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write;
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }

    impl tracing::Subscriber for Capture {
        fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            let mut fields = Fields(span.metadata().name().to_string());
            span.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
            let mut fields = Fields(String::new());
            values.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            let mut fields = Fields(event.metadata().target().to_string());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    /// M1: what matching writes to the log is its label, and to the audit
    /// log nothing -- no folder, tool, mission or count.
    #[test]
    fn mission_matching_logs_labels_only_and_audits_nothing() {
        let (_dir, s, work) = seeded();
        let audit_before = super::super::audit::load(&s.store).unwrap();
        let capture = Capture::default();
        // Run the capture on a dedicated, short-lived thread rather than
        // whichever pool thread cargo's test harness happened to reuse for
        // this test (#1227 review: "not verified -- could be flaky under
        // parallel test threads"). Tracing's per-callsite interest cache is
        // process-wide -- a single atomic per callsite, rebuilt by asking
        // every currently-registered `Dispatch`, including ones from
        // unrelated tests concurrently alive on other threads -- so a
        // dedicated thread at least guarantees nothing else ever ran on it
        // before this block, and that the thread, and every `Dispatch`
        // created on it, is gone the moment `scope` returns.
        //
        // With a single scoped subscriber alive, tracing works out a
        // callsite's interest from whichever thread registers it first --
        // another test's, which has none, so the handler's line would be
        // cached as unwanted. A second, silent dispatcher makes it ask every
        // live subscriber instead, which settles the callsite's cached
        // interest at `Interest::sometimes()` rather than a fixed yes/no.
        // From then on every event at that callsite asks whichever
        // dispatch is active on the thread raising it, which on this
        // thread stays `capture` for as long as `with_default` is in
        // scope here -- regardless of what any other test thread does
        // concurrently.
        let reply = std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    tracing::subscriber::with_default(capture.clone(), || {
                        let _second =
                            tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
                        tracing::callsite::rebuild_interest_cache();
                        ask(&s, json!({"catalogue": catalogue()}))
                    })
                })
                .join()
                .unwrap()
        });
        assert_eq!(reply.result.unwrap()["matches"], json!(["rust-claude"]));
        let lines = capture.0.lock().unwrap().clone();
        assert!(
            lines.iter().any(|l| l.contains(LOG_LABEL)),
            "the label is logged: {lines:?}"
        );
        let logged = lines.join("\n");
        let work = work.path().to_string_lossy().to_string();
        for secret in [
            work.as_str(),
            "ask-repo",
            "never-repo",
            "claude-code",
            "codex",
            "rust",
            "python",
            "Rust with Claude",
            "/s/",
        ] {
            assert!(
                !logged.contains(secret),
                "the log names {secret:?}: {lines:?}"
            );
        }
        assert!(
            lines.iter().all(|l| l.trim_end().ends_with(LOG_LABEL)),
            "nothing but the label: {lines:?}"
        );
        assert_eq!(super::super::audit::load(&s.store).unwrap(), audit_before);
    }

    /// The catalogue is required and read strictly about its version.
    #[test]
    fn mission_matches_refuses_a_missing_or_newer_catalogue() {
        let (_dir, s) = shared();
        let label = |r: Response| r.error.unwrap().message;
        assert_eq!(label(ask(&s, json!({}))), "catalogue-required");
        assert_eq!(
            label(ask(&s, json!({"catalogue": {"schema_version": 2}}))),
            "catalogue-schema-unsupported"
        );
        assert_eq!(
            label(ask(&s, json!({"catalogue": "missions"}))),
            "catalogue-invalid"
        );
    }

    // ---- list_pending's mission_fit (the catalogue slot) ----

    fn list_pending(s: &DaemonShared) -> serde_json::Value {
        let reply = handle_request(
            s,
            &Request {
                id: 8,
                method: "list_pending".to_string(),
                params: json!({}),
            },
        );
        reply.result.expect("list_pending answers")
    }

    /// `mission_fit` of the listed entry whose session hash is `hash`:
    /// `None` when the field is absent.
    fn fit_of(listed: &serde_json::Value, hash: &str) -> Option<u64> {
        let row = listed["pending"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["session_hash"] == hash)
            .unwrap_or_else(|| panic!("{hash} is listed: {listed}"));
        row.get("mission_fit").map(|v| v.as_u64().expect("a count"))
    }

    /// The project key `seeded()` gives a folder under its work directory.
    fn key_under(work: &tempfile::TempDir, folder: &str) -> String {
        super::super::policy::project_for(Some(work.path().join(folder).to_str().unwrap())).0
    }

    /// Queue one pending entry: `source` is the adapter, `hash` its session
    /// hash, in `folder`'s project, at `path`.
    fn pend(s: &DaemonShared, source: &str, hash: &str, folder: &str, path: &str) -> uuid::Uuid {
        let entry_id = uuid::Uuid::new_v4();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                QueueEntry {
                    entry_id,
                    session_hash: hash.to_string(),
                    source: source.to_string(),
                    project_key: folder.to_string(),
                    project_label: "seed".to_string(),
                    path: PathBuf::from(path),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    ..Default::default()
                },
                500,
            )
            .unwrap();
        entry_id
    }

    fn receive(s: &DaemonShared, raw: serde_json::Value) {
        receive_catalogue(&s.mission_catalogue, &raw, Utc::now()).expect("catalogue is read");
    }

    /// No catalogue, an expired one and a refused one are all unknown: the
    /// field is absent from every row, never 0. A refusal empties a slot
    /// that held a live catalogue rather than leaving it in place.
    #[test]
    fn mission_fit_is_absent_without_a_live_catalogue() {
        let (_dir, s, work) = seeded();
        let ask = key_under(&work, "ask-repo");
        pend(&s, "claude-code", "sha256:a", &ask, "/s/claude-ask-1.jsonl");

        assert_eq!(fit_of(&list_pending(&s), "sha256:a"), None, "no slot");

        let stale = Utc::now()
            - chrono::TimeDelta::from_std(MISSION_CATALOGUE_MAX_AGE).unwrap()
            - chrono::TimeDelta::minutes(1);
        receive_catalogue(&s.mission_catalogue, &catalogue(), stale).unwrap();
        assert_eq!(fit_of(&list_pending(&s), "sha256:a"), None, "expired");

        receive(&s, catalogue());
        assert_eq!(fit_of(&list_pending(&s), "sha256:a"), Some(1), "live");

        let refused = receive_catalogue(
            &s.mission_catalogue,
            &json!({"schema_version": 2}),
            Utc::now(),
        );
        assert_eq!(refused, Err(CatalogueError::SchemaUnsupported));
        assert!(s.mission_catalogue.lock().unwrap().is_none());
        assert_eq!(fit_of(&list_pending(&s), "sha256:a"), None, "refused");
    }

    /// A live catalogue with no missions is a known zero.
    #[test]
    fn mission_fit_is_zero_for_a_live_empty_catalogue() {
        let (_dir, s, work) = seeded();
        let ask = key_under(&work, "ask-repo");
        pend(&s, "claude-code", "sha256:a", &ask, "/s/claude-ask-1.jsonl");
        receive(&s, json!({"schema_version": 1, "missions": []}));
        assert_eq!(fit_of(&list_pending(&s), "sha256:a"), Some(0));
    }

    /// An entry from a switched-off adapter, or in a Never folder, fits
    /// nothing, while an Ask me Claude Code entry fits the one matched
    /// mission its criteria meet. The Python mission's only folder is
    /// Never, so it is never matched; `claude-3` asks for three sessions
    /// and only two are readable.
    #[test]
    fn mission_fit_reads_entries_through_the_same_gates() {
        let (_dir, s, work) = seeded();
        let (ask, never) = (key_under(&work, "ask-repo"), key_under(&work, "never-repo"));
        pend(
            &s,
            "claude-code",
            "sha256:ask",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        pend(&s, "codex", "sha256:codex", &ask, "/s/codex-ask.jsonl");
        pend(
            &s,
            "claude-code",
            "sha256:never",
            &never,
            "/s/claude-never.jsonl",
        );
        receive(&s, catalogue());
        let listed = list_pending(&s);
        assert_eq!(fit_of(&listed, "sha256:ask"), Some(1), "{listed}");
        assert_eq!(fit_of(&listed, "sha256:codex"), Some(0), "adapter off");
        assert_eq!(fit_of(&listed, "sha256:never"), Some(0), "Never folder");
    }

    /// Under a Never contribution override nothing is read, so every entry
    /// fits nothing -- a known zero, since the catalogue is live.
    #[test]
    fn mission_fit_under_a_never_override_is_zero_everywhere() {
        let (_dir, s, work) = seeded();
        let ask = key_under(&work, "ask-repo");
        pend(
            &s,
            "claude-code",
            "sha256:ask",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        receive(
            &s,
            json!({"schema_version": 1, "missions": [{"mission_id": "any"}]}),
        );
        assert_eq!(fit_of(&list_pending(&s), "sha256:ask"), Some(1));
        s.policy
            .lock()
            .unwrap()
            .set_contribution_override(ProjectMode::Ignore, Utc::now(), None)
            .unwrap();
        assert_eq!(fit_of(&list_pending(&s), "sha256:ask"), Some(0));
    }

    /// OWNER DECISION V3: an entry whose criteria a mission meets does not
    /// fit it while the readable sessions fall short of its `min_sessions`.
    #[test]
    fn mission_fit_needs_the_mission_matched_not_only_the_criteria_met() {
        let (_dir, s, work) = seeded();
        let ask = key_under(&work, "ask-repo");
        pend(
            &s,
            "claude-code",
            "sha256:ask",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        receive(
            &s,
            json!({"schema_version": 1, "missions": [
                {"mission_id": "claude-3", "criteria": {"tools": ["claude-code"], "min_sessions": 3}},
            ]}),
        );
        assert_eq!(fit_of(&list_pending(&s), "sha256:ask"), Some(0));
        receive(
            &s,
            json!({"schema_version": 1, "missions": [
                {"mission_id": "claude-2", "criteria": {"tools": ["claude-code"], "min_sessions": 2}},
            ]}),
        );
        assert_eq!(fit_of(&list_pending(&s), "sha256:ask"), Some(1));
    }

    /// A pending entry for a session that is kept, or whose upload was
    /// withdrawn, fits nothing: the same path-based M2 exclusion
    /// `readable_sessions` applies to the cwd cache, with no second filter.
    #[test]
    fn mission_fit_excludes_kept_and_withdrawn_sessions() {
        let (_dir, s, work) = seeded();
        let ask = key_under(&work, "ask-repo");
        let one = json!({"schema_version": 1, "missions": [
            {"mission_id": "claude", "criteria": {"tools": ["claude-code"]}},
        ]});
        // An earlier keep of the session at claude-ask-1, and a withdrawn
        // upload of the one at claude-ask-2; each has a later pending entry.
        let kept = pend(
            &s,
            "claude-code",
            "sha256:kept",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        s.queue.lock().unwrap().keep(kept).unwrap();
        let submission_id = uuid::Uuid::new_v4();
        let uploaded = pend(
            &s,
            "claude-code",
            "sha256:up",
            &ask,
            "/s/claude-ask-2.jsonl",
        );
        {
            let mut queue = s.queue.lock().unwrap();
            queue.set_state(uploaded, QueueState::Uploaded, None);
            queue.set_submission_id(uploaded, submission_id);
        }
        HistoryCache::save(&s.store, &[history_record(submission_id, STATUS_WITHDRAWN)]).unwrap();
        pend(
            &s,
            "claude-code",
            "sha256:kept-again",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        pend(
            &s,
            "claude-code",
            "sha256:withdrawn-again",
            &ask,
            "/s/claude-ask-2.jsonl",
        );
        pend(
            &s,
            "claude-code",
            "sha256:fresh",
            &ask,
            "/s/claude-fresh.jsonl",
        );
        // A third Claude Code session in the Ask me folder stays readable,
        // so the mission is matched and the fresh entry is the control.
        s.state.lock().unwrap().cwd_cache.insert(
            "/s/claude-fresh.jsonl".to_string(),
            CwdCacheEntry {
                size_bytes: 1,
                modified_at: Utc::now(),
                cwd: Some(work.path().join("ask-repo").to_str().unwrap().to_string()),
                project_key: Some(ask.clone()),
                tool: Some("claude-code".to_string()),
                adapter: Some("claude-code".to_string()),
            },
        );
        receive(&s, one);
        let listed = list_pending(&s);
        assert_eq!(fit_of(&listed, "sha256:fresh"), Some(1), "{listed}");
        assert_eq!(fit_of(&listed, "sha256:kept-again"), Some(0), "{listed}");
        assert_eq!(
            fit_of(&listed, "sha256:withdrawn-again"),
            Some(0),
            "{listed}"
        );
    }

    /// Counts only: no mission id or title reaches the wire, and no other
    /// rendering of an entry (`list_kept`) carries the field.
    #[test]
    fn mission_fit_puts_no_mission_on_the_wire() {
        let (_dir, s, work) = seeded();
        let ask = key_under(&work, "ask-repo");
        pend(
            &s,
            "claude-code",
            "sha256:ask",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        let kept = pend(
            &s,
            "claude-code",
            "sha256:kept",
            &ask,
            "/s/claude-ask-9.jsonl",
        );
        s.queue.lock().unwrap().keep(kept).unwrap();
        receive(&s, catalogue());
        let listed = list_pending(&s);
        assert_eq!(fit_of(&listed, "sha256:ask"), Some(1));
        let text = listed.to_string();
        for named in [
            "rust-claude",
            "Rust with Claude",
            "python",
            "Python work",
            "claude-3",
            "Three Claude sessions",
        ] {
            assert!(!text.contains(named), "{named} is on the wire: {text}");
        }
        let kept_rows = handle_request(
            &s,
            &Request {
                id: 9,
                method: "list_kept".to_string(),
                params: json!({}),
            },
        )
        .result
        .unwrap();
        assert!(
            kept_rows["kept"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| row.get("mission_fit").is_none()),
            "{kept_rows}"
        );
    }

    /// Folder languages are looked at once per catalogue and then read from
    /// the slot's cache: a marker removed after the first look changes
    /// nothing until the catalogue is replaced, which drops the cache.
    #[test]
    fn mission_fit_caches_folder_languages_per_catalogue() {
        let (_dir, s, work) = seeded();
        let ask = key_under(&work, "ask-repo");
        pend(
            &s,
            "claude-code",
            "sha256:ask",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        let rust = json!({"schema_version": 1, "missions": [
            {"mission_id": "rust", "criteria": {"languages": ["rust"]}},
        ]});
        receive(&s, rust.clone());
        assert_eq!(fit_of(&list_pending(&s), "sha256:ask"), Some(1));
        std::fs::remove_file(work.path().join("ask-repo").join("Cargo.toml")).unwrap();
        assert_eq!(fit_of(&list_pending(&s), "sha256:ask"), Some(1), "cached");
        receive(&s, rust);
        assert_eq!(fit_of(&list_pending(&s), "sha256:ask"), Some(0), "re-read");
    }

    /// `mission_matches` neither reads nor writes the slot: it answers from
    /// its parameter alone, and a live slot is left exactly as it was (M2).
    #[test]
    fn mission_matches_neither_reads_nor_writes_the_slot() {
        let (_dir, s, _work) = seeded();
        let answer = ask(&s, json!({"catalogue": catalogue()})).result.unwrap();
        assert!(s.mission_catalogue.lock().unwrap().is_none(), "not written");
        receive(&s, json!({"schema_version": 1, "missions": []}));
        let before = format!("{:?}", s.mission_catalogue.lock().unwrap());
        assert_eq!(
            ask(&s, json!({"catalogue": catalogue()})).result.unwrap(),
            answer
        );
        assert_eq!(format!("{:?}", s.mission_catalogue.lock().unwrap()), before);
    }

    /// The languages the live slot has cached, by folder key.
    fn cached_languages(s: &DaemonShared) -> BTreeMap<String, BTreeSet<String>> {
        s.mission_catalogue
            .lock()
            .unwrap()
            .as_ref()
            .expect("live catalogue")
            .folder_languages
            .clone()
    }

    /// Kristi's #1285 review, finding 1 (and #1298, poldsam 1): a waiting
    /// entry's folder is probed for language markers only when
    /// `readable_sessions` would let the entry through. Under a languages
    /// catalogue, an entry in a Never folder and an entry whose adapter is
    /// off (codex, in a folder no readable session ran in) each leave their
    /// folder unprobed, while an Ask me entry with its adapter on is probed.
    #[test]
    fn waiting_entries_in_never_folders_or_off_adapters_are_not_language_probed() {
        let (_dir, s, work) = seeded();
        let off_dir = work.path().join("off-repo");
        std::fs::create_dir_all(&off_dir).unwrap();
        std::fs::write(off_dir.join("pyproject.toml"), "").unwrap();
        let (ask, never, off) = (
            key_under(&work, "ask-repo"),
            key_under(&work, "never-repo"),
            key_under(&work, "off-repo"),
        );
        pend(
            &s,
            "claude-code",
            "sha256:ask",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        pend(
            &s,
            "claude-code",
            "sha256:never",
            &never,
            "/s/claude-never.jsonl",
        );
        pend(&s, "codex", "sha256:off", &off, "/s/codex-off.jsonl");
        receive(&s, catalogue());
        let listed = list_pending(&s);
        assert_eq!(fit_of(&listed, "sha256:never"), Some(0), "{listed}");
        assert_eq!(fit_of(&listed, "sha256:off"), Some(0), "{listed}");
        let cached = cached_languages(&s);
        assert!(
            cached.contains_key(&ask),
            "Ask me folder probed: {cached:?}"
        );
        assert!(
            !cached.contains_key(&never),
            "Never folder probed: {cached:?}"
        );
        assert!(
            !cached.contains_key(&off),
            "off adapter's folder probed: {cached:?}"
        );
    }

    /// Under a Never contribution override every folder is Never, so no
    /// waiting entry's folder is probed, whatever its own mode.
    #[test]
    fn no_waiting_entry_is_language_probed_under_a_never_override() {
        let (_dir, s, work) = seeded();
        let ask = key_under(&work, "ask-repo");
        pend(
            &s,
            "claude-code",
            "sha256:ask",
            &ask,
            "/s/claude-ask-1.jsonl",
        );
        s.policy
            .lock()
            .unwrap()
            .set_contribution_override(ProjectMode::Ignore, Utc::now(), None)
            .unwrap();
        receive(&s, catalogue());
        list_pending(&s);
        let cached = cached_languages(&s);
        assert!(cached.is_empty(), "probed under Never: {cached:?}");
    }
}
