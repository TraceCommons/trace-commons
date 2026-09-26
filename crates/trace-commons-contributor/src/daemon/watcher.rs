//! The poll loop: stat the session roots, decide what is finished, and route
//! it by project policy.
//!
//! Polling rather than filesystem notification is deliberate. The quiescence
//! window is half an hour, so a sixty-second poll costs nothing in
//! responsiveness, and it avoids a watch dependency plus the per-platform
//! behaviour differences that come with one.
//!
//! Resolving a session's working directory means reading into the file, so
//! results are cached against the file's size and mtime. Without that cache a
//! laptop would re-read every session file every minute.
//!
//! The same reasoning governs the expensive step, `TraceSource::load`, which
//! reads, parses and hashes a whole session group. A session already sitting
//! in the queue is `Eligible` on every poll -- eligibility is decided from
//! prior *uploads*, and a `Pending` entry has never uploaded -- so the pass
//! used to load all of them, every minute, and throw the result away when
//! `replace_live_at_path` found the entry already tracked. It asks
//! `Queue::unchanged_offer_at_path` first now, comparing the whole
//! observation (group size and group mtime) against what the queue entry was
//! built from, and only loads when something moved.
//!
//! Two smaller per-poll costs were measured on the same corpus (81
//! claude-code groups over 1,044 files, 3,069 codex sessions, 11.7 GB) with
//! a release build, and only one of them was worth changing:
//!
//! - `source.discover()` walks both trees every tick. That is 4.9 ms for
//!   claude-code and 9.0 ms for codex once the head-peek memos are warm --
//!   14 ms of a sixty-second tick, 0.02% duty -- and 154 ms on the first
//!   pass of a process, when those memos are cold. It stats rather than
//!   reads, and the memos (capped at 8192 entries each) have room for
//!   several times this corpus. Left as a full walk: incremental discovery
//!   would have to keep its own view of which directories moved, and the
//!   failure mode of getting that wrong -- a session never noticed, or a
//!   subagent that lands under an already-seen session directory never
//!   noticed -- costs far more than the 14 ms it would save.
//! - `state.save()` at the end of the tick re-serialized and rewrote the
//!   whole `DaemonState` unconditionally: 1.24 MB, ~0.85 ms to serialize
//!   and ~6-10 ms to write and `fsync`, every sixty seconds, around 1.8 GB
//!   of writes a day, for bytes identical to the ones already on disk. That
//!   one is now elided when nothing moved; see `DaemonState::save`.
//!
//! The trajectory source is not watched: trajectory files have no
//! conventional local store to poll, so they stay a deliberate `submit
//! --trajectory` action.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::eligibility::{Eligibility, Observation, armed_settle_elapsed, evaluate};
use super::health;
use super::ipc::{DaemonShared, EVENT_QUEUE_CHANGED};
use super::policy::{ProjectMode, disambiguated_label, known_keys, project_for};
use super::queue::{QueueEntry, QueueState, entry_id_for};
use super::state::CwdCacheEntry;
use crate::source::{SessionRef, TraceSource, all_sources};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    pub observed: usize,
    pub queued: usize,
    /// Entries handed straight to the uploader because their project is
    /// opted in.
    pub auto_ready: usize,
    /// Entries whose project is opted in but whose session has not been
    /// quiet for `ARMED_SETTLE_SECS` yet, so they stay `Pending` for now.
    /// Counted apart from `auto_ready` because it is a wait, not a refusal:
    /// the same entry becomes `auto_ready` on a later poll.
    pub armed_not_settled: usize,
    pub ignored: usize,
    /// Sessions skipped because the contributor dismissed them. Distinct
    /// from `ignored`, which is a standing decision about a whole project.
    pub dismissed: usize,
    /// Sessions that reached `TraceSource::load` and could not be read, for
    /// any reason. Every one of these used to be a bare `continue`.
    pub unloadable: usize,
    /// Unattended approvals made this pass that the automatic-contribution
    /// gate would have refused, had it been enforced. Counted within
    /// `auto_ready`, not apart from it. While the gate ships unenforced this
    /// is how far today's behaviour is from what the gate will allow; see
    /// `automatic_gate`.
    pub gate_would_refuse: usize,
    /// Sessions in armed folders that an enforced gate is holding waiting
    /// instead of approving. A level rather than an event: an entry the gate
    /// holds is counted again on every pass that sees it, so this is how
    /// much armed work is waiting on the gate now.
    ///
    /// `None` on a scoped pass. That pass visits only the sessions some
    /// paths resolved to, so its count says nothing about the rest of the
    /// corpus; reporting it as a level would drop to zero on every unrelated
    /// file change while work is still held. Only a full pass measures it,
    /// the same rule `finish_pass` applies to retracting health conditions.
    /// `Some(0)` while the gate is unenforced. See `automatic_gate`.
    pub gate_blocked: Option<usize>,
    /// The subset of `unloadable` the source declined by name over its own
    /// byte budget, rather than failed to read. See
    /// `source::SessionTooLarge` for why the two are counted apart.
    pub too_large: usize,
}

/// One pass over the session roots.
///
/// Returns what it saw rather than acting on uploads itself: the caller owns
/// the submit pipeline, which needs an async context and mutable state this
/// function deliberately does not hold.
///
/// The pass itself (`tick_blocking`) is synchronous filesystem scanning and
/// hashing with no `.await` point of its own -- `source.discover()`,
/// `std::fs::metadata`, `source.load()` are all blocking calls. On a
/// multi-thread runtime with few worker threads (in the limit, one -- the
/// C ABI's `tc_daemon_start` builds a runtime sized by
/// `TOKIO_WORKER_THREADS`, which a 1-vCPU host or an explicit override can
/// set to 1), running that whole pass inline previously monopolized the
/// sole worker for the scan's entire duration: the socket server, every
/// `tc_subscribe` delivery, and even a reentrant `tc_daemon_stop`'s own
/// wait on the supervisor's `JoinHandle` all starved behind it. `tick`
/// moves the pass off whichever worker is currently running it via
/// `super::run_blocking` (see its doc) -- callers under a `current_thread`
/// runtime (the default `#[tokio::test]` flavor, which every test in this
/// module's suite uses) run the pass inline instead, since
/// `block_in_place` panics there.
pub async fn tick(shared: &DaemonShared, now: DateTime<Utc>) -> Result<TickReport> {
    // `is_paused` also auto-clears a timed pause that has lapsed, so an
    // elapsed `pause {until}` resumes ticking on its own rather than needing
    // an explicit `resume` from whichever app set the timer -- cheap enough
    // to check before deciding whether `block_in_place` is even worth it.
    if shared.is_paused(now) {
        return Ok(TickReport::default());
    }
    super::run_blocking(|| tick_blocking(shared, now))
}

/// The actual pass; see `tick`'s doc for why it is a plain synchronous
/// function rather than `async fn` (it never awaited anything -- every step
/// is a blocking filesystem or lock operation) and why `tick` runs it
/// through `run_blocking`.
fn tick_blocking(shared: &DaemonShared, now: DateTime<Utc>) -> Result<TickReport> {
    let max_queue_entries = {
        let s = shared.settings.lock().expect("settings lock");
        s.max_queue_entries
    };
    let source_roots = shared.source_roots_with_routing();
    tick_over(
        shared,
        now,
        all_sources(&source_roots),
        source_roots.source_identities(),
        max_queue_entries,
    )
}

/// The pass itself, over an explicit source list.
///
/// Split out from `tick_blocking` only so tests can hand it a source that
/// counts its own `load` calls: "this poll did not re-read anything" is a
/// claim about how often `TraceSource::load` runs, and nothing observable
/// from the queue alone can prove it.
///
/// `source_identities` must come from the same `SourceRoots` as `sources`:
/// the automatic grant records each source under its name and root, and a
/// root read separately could name one a `set_settings` has since swapped
/// in, so root A's listing would be recorded as root B's and B's history
/// would read as new. See `PassContext::source_key`.
///
/// This is the full-scan path: it asks every source what it has. The scoped
/// path (`tick_over_paths`) visits a supplied set of sessions instead. Both
/// run the *same* per-session body (`visit_session`) and the *same* epilogue
/// (`finish_pass`); neither is duplicated, because a copy of either would
/// drift, and what it would drift on is what the daemon decides to upload.
fn tick_over(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    sources: Vec<Box<dyn TraceSource>>,
    source_identities: SourceIdentities,
    max_queue_entries: usize,
) -> Result<TickReport> {
    release_stale_holds(shared, now);
    let ctx = PassContext::read(shared, now, max_queue_entries, source_identities);
    // Before any session is visited, so a project whose grant was just
    // voided is already ask-first when its sessions are looked at -- and
    // against the same config snapshot the pass uses, so a widening written
    // between two reads cannot be missed by the sweep and used by the pass.
    sweep_grants(shared, &ctx);
    let mut out = PassOutcome::default();

    // Read before anything is listed: a grant given while discovery walks the
    // disk is recorded by a later pass, from a listing taken after it.
    let grant = shared.policy.lock().expect("policy lock").grant_id();
    let discovered: Vec<(&dyn TraceSource, Vec<SessionRef>)> = sources
        .iter()
        .filter_map(|source| source.discover().ok().map(|refs| (source.as_ref(), refs)))
        .collect();
    // Before any session is visited, so nothing on disk in a source recorded
    // now can be armed by the grant. A source whose discovery failed is not
    // recorded, and arms nothing until a pass records it.
    if let Some(grant) = grant {
        record_sources_for_grant(shared, &ctx, grant, &discovered);
    }
    for (source, refs) in &discovered {
        for session_ref in refs {
            visit_session(shared, &ctx, *source, session_ref, &mut out);
        }
    }

    let report = finish_pass(shared, out, true)?;
    report_gate(shared, &ctx.gate, &report);
    Ok(report)
}

/// Record what is on disk for the Flow 1 grant `grant`, per source: each
/// source this full pass discovered that the grant has not recorded yet --
/// the first pass after the grant, a harness connected or re-rooted since,
/// or one whose discovery failed before -- has every session and the project
/// each belongs to recorded, eligible or not. Only a full pass may do this; a
/// scoped pass sees only what changed. See `policy::AutomaticGrant`.
fn record_sources_for_grant(
    shared: &DaemonShared,
    ctx: &PassContext,
    grant: DateTime<Utc>,
    discovered: &[(&dyn TraceSource, Vec<SessionRef>)],
) {
    for (source, refs) in discovered {
        let key = ctx.source_key(source.name());
        if !shared
            .policy
            .lock()
            .expect("policy lock")
            .needs_source_record(grant, &key)
        {
            continue;
        }
        let mut sessions = std::collections::BTreeSet::new();
        let mut projects = std::collections::BTreeSet::new();
        for session_ref in refs {
            sessions.insert(session_ref.path.to_string_lossy().to_string());
            projects.insert(project_key_of(shared, *source, session_ref));
        }
        let mut policy = shared.policy.lock().expect("policy lock");
        if policy.record_source(grant, &key, sessions, projects)
            && policy.save(&shared.store).is_err()
        {
            tracing::warn!("could not persist what was on disk for the automatic grant");
        }
    }
}

/// The project a session belongs to, through the same cwd cache the pass
/// uses.
fn project_key_of(
    shared: &DaemonShared,
    source: &dyn TraceSource,
    session_ref: &SessionRef,
) -> String {
    let modified_at = session_ref.group_modified_at.or_else(|| {
        std::fs::metadata(&session_ref.path)
            .and_then(|m| m.modified())
            .ok()
            .map(DateTime::<Utc>::from)
    });
    let cwd = match modified_at {
        Some(modified_at) => {
            let obs = Observation {
                path: session_ref.path.clone(),
                size_bytes: session_ref.size_bytes,
                modified_at,
            };
            resolve_cwd(shared, source, session_ref, &obs)
        }
        None => session_ref.cwd.clone(),
    };
    super::policy::project_for(cwd.as_deref()).0
}

/// Arm `project_key` under the Flow 1 grant when this session is the first
/// sign of a project discovered after it (K3). The arming writes an explicit
/// policy entry, the terms it is granted under, and an audit row -- recorded
/// first, as `set_project_mode` does, so there is never an armed project
/// with no record of how it was armed. Returns the mode now in force.
fn arm_by_default(
    shared: &DaemonShared,
    ctx: &PassContext,
    source: &dyn TraceSource,
    project_key: &str,
    session_path: &Path,
) -> ProjectMode {
    let path = session_path.to_string_lossy();
    let source_key = ctx.source_key(source.name());
    let Some(terms) = ctx.grant_terms.clone() else {
        return ProjectMode::NotifyOnly;
    };
    let label = {
        let policy = shared.policy.lock().expect("policy lock");
        if !policy.arms_by_default(project_key, &path, &source_key) {
            return ProjectMode::NotifyOnly;
        }
        super::policy::project_label_for(
            super::policy::display_path_for_key(project_key)
                .as_deref()
                .unwrap_or(project_key),
        )
    };
    let entry = super::audit::AuditEntry {
        at: ctx.now,
        action: "armed-by-default".to_string(),
        project_label: Some(label),
        detail: None,
    };
    if super::audit::append(&shared.store, &entry).is_err() {
        tracing::warn!("could not record arming a new project; it asks first");
        return ProjectMode::NotifyOnly;
    }
    let mut policy = shared.policy.lock().expect("policy lock");
    // Re-checked: the grant or the project may have changed while the audit
    // row was written.
    if !policy.arms_by_default(project_key, &path, &source_key)
        || policy.arm_by_grant(project_key, ctx.now, terms).is_err()
    {
        return ProjectMode::NotifyOnly;
    }
    if policy.save(&shared.store).is_err() {
        tracing::warn!("could not persist arming a new project");
    }
    ProjectMode::AutoUpload
}

/// Whether a session waits for the contributor even though its project is
/// armed: the automatic grant armed the project and the session was on disk
/// at a grant. Arming nothing already on disk has to cover approval, not only
/// arming -- a pre-grant session can come to read as a project the grant
/// armed later (its recorded cwd changed, or it gained one after sitting in
/// the unknown bucket). Both unattended approval sites ask this.
fn held_back_from_the_grant(shared: &DaemonShared, project_key: &str, session_path: &Path) -> bool {
    shared
        .policy
        .lock()
        .expect("policy lock")
        .holds_back_unattended(project_key, &session_path.to_string_lossy())
}

/// Maps a path something happened at to the session that owns it, without
/// walking the corpus.
///
/// A path is not a session: a claude-code write under
/// `<uuid>/subagents/<agent>.jsonl` belongs to the parent conversation, which
/// is the group-address rule `discover` already implements and which must not
/// be re-derived here. The lookup is therefore source-specific and belongs
/// beside `discover` on `TraceSource`; until it lands there, the caller
/// supplies it and this module stays agnostic about how it is answered.
/// Returning `None` means "this path is not this source's, or names nothing".
pub type SessionAt<'a> = &'a dyn Fn(&dyn TraceSource, &Path) -> Option<SessionRef>;

/// One pass over an explicit set of session paths.
///
/// The sibling of `tick`: same paused check, same off-worker dispatch, same
/// per-session work and same epilogue -- only the question "which sessions?"
/// is answered differently. Nothing about *what* is decided changes; see
/// `visit_session`.
pub async fn tick_paths(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    paths: &[PathBuf],
    session_at: SessionAt<'_>,
) -> Result<TickReport> {
    if shared.is_paused(now) {
        return Ok(TickReport::default());
    }
    let max_queue_entries = {
        let s = shared.settings.lock().expect("settings lock");
        s.max_queue_entries
    };
    let source_roots = shared.source_roots_with_routing();
    super::run_blocking(|| {
        tick_over_paths(
            shared,
            now,
            all_sources(&source_roots),
            source_roots.source_identities(),
            max_queue_entries,
            paths,
            session_at,
        )
    })
}

/// The scoped pass, over an explicit source list; `tick_over`'s sibling.
///
/// Visits exactly the sessions the supplied paths resolve to and never calls
/// `discover`, which is the entire point: the full walk is what costs, and it
/// costs whether or not anything moved.
///
/// Two paths that resolve to the same session (a conversation and one of its
/// delegated transcripts, say, both written in the same burst) are visited
/// once. A second visit in the same pass would record a second observation of
/// a byte count that has not moved between them, which is precisely the
/// signal `eligibility::evaluate` reads as "stable".
fn tick_over_paths(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    sources: Vec<Box<dyn TraceSource>>,
    source_identities: SourceIdentities,
    max_queue_entries: usize,
    paths: &[PathBuf],
    session_at: SessionAt<'_>,
) -> Result<TickReport> {
    release_stale_holds(shared, now);
    let ctx = PassContext::read(shared, now, max_queue_entries, source_identities);
    // Before any session is visited, so a project whose grant was just
    // voided is already ask-first when its sessions are looked at -- and
    // against the same config snapshot the pass uses, so a widening written
    // between two reads cannot be missed by the sweep and used by the pass.
    sweep_grants(shared, &ctx);
    let mut out = PassOutcome::default();
    let mut visited: HashSet<PathBuf> = HashSet::new();

    for path in paths {
        for source in &sources {
            let Some(session_ref) = session_at(source.as_ref(), path) else {
                continue;
            };
            if !visited.insert(session_ref.path.clone()) {
                break;
            }
            visit_session(shared, &ctx, source.as_ref(), &session_ref, &mut out);
            break;
        }
    }

    let report = finish_pass(shared, out, false)?;
    report_gate(shared, &ctx.gate, &report);
    Ok(report)
}

/// Release holds whose cause has gone, before any session is visited.
///
/// Today the only hold is for token-distribution review, which exists only
/// while `token_distributions_contribution` is on. Checked every pass rather
/// than when the setting changes, so no route to turning it off -- the
/// socket, a config edit, a restart -- can leave the holds behind.
fn release_stale_holds(shared: &DaemonShared, now: DateTime<Utc>) {
    let token_review_on = {
        let s = shared.settings.lock().expect("settings lock");
        s.token_distributions_contribution
    };
    if token_review_on {
        return;
    }
    let released = {
        let mut queue = shared.queue.lock().expect("queue lock");
        let released = queue
            .release_holds_for_reason(super::queue::REASON_TOKEN_DISTRIBUTION_REVIEW_REQUIRED, now);
        if released > 0 && queue.save(&shared.store).is_err() {
            tracing::warn!("could not persist released holds");
        }
        released
    };
    if released > 0 {
        shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
    }
}

// Lets a test exercise the enforced gate while it ships unenforced.
//
// Thread-local rather than a shared flag because tests run in parallel and a
// global would leak between them; each `#[tokio::test]` runs on its own
// current-thread runtime. Compiled out of every non-test build.
#[cfg(test)]
thread_local! {
    static ENFORCE_GATE_FOR_TEST: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn gate_enforced() -> bool {
    #[cfg(test)]
    if ENFORCE_GATE_FOR_TEST.with(std::cell::Cell::get) {
        return true;
    }
    super::automatic_gate::ENFORCED
}

/// Void every standing grant the terms in force no longer cover.
///
/// R6 of the connect-and-forget design; see `grant_terms` for what widens a
/// grant. A voided project returns to ask-first: nothing more from it is
/// approved on the contributor's behalf until they arm it again, which
/// records the new terms. Approvals already made under the old terms are
/// stopped by the uploader, which re-derives `input_fingerprint` before every
/// send -- every widening this rule checks is also a change to that
/// fingerprint -- and revokes them to `Pending`, where the watcher, the
/// project no longer armed, leaves them for the contributor.
///
/// Void first and record second. Arming records first so that a failed
/// write blocks it; voiding is the safe direction, so it must happen even
/// when the audit cannot be written, and a failed write is logged instead.
/// Audit and log carry labels only.
fn sweep_grants(shared: &DaemonShared, ctx: &PassContext) {
    let Some(current) = ctx.grant_terms.as_ref() else {
        return;
    };
    let now = ctx.now;
    let sweep = {
        let mut policy = shared.policy.lock().expect("policy lock");
        let sweep = policy.sweep_grants(current, now);
        // Fixed labels, never the error: its context can carry a path.
        if sweep.changed() && policy.save(&shared.store).is_err() {
            tracing::warn!("could not persist the grant sweep");
        }
        sweep
    };
    for voided in &sweep.voided {
        let entry = super::audit::AuditEntry {
            at: now,
            action: "auto-upload-voided".to_string(),
            project_label: Some(voided.project_label.clone()),
            detail: Some(voided.reasons.join(",")),
        };
        if super::audit::append(&shared.store, &entry).is_err() {
            tracing::warn!("could not record a voided grant");
        }
        tracing::info!(
            reasons = ?voided.reasons,
            "an automatic-contribution grant was voided; the project now asks first"
        );
    }
    if let Some(reasons) = &sweep.automatic_grant_voided {
        let entry = super::audit::AuditEntry {
            at: now,
            action: "automatic-grant-voided".to_string(),
            project_label: None,
            detail: Some(reasons.join(",")),
        };
        if super::audit::append(&shared.store, &entry).is_err() {
            tracing::warn!("could not record a voided automatic grant");
        }
        tracing::info!(
            reasons = ?reasons,
            "the automatic-contribution grant was voided; new projects ask first"
        );
    }
    if !sweep.voided.is_empty() || sweep.automatic_grant_voided.is_some() {
        shared.publish(super::ipc::EVENT_STATUS_CHANGED, serde_json::json!({}));
    }
}

/// Each source's name and root, as `SourceRoots::source_identities` gives
/// them.
type SourceIdentities = std::collections::BTreeMap<&'static str, String>;

/// What a pass reads once, up front, and hands to every session it visits.
struct PassContext {
    now: DateTime<Utc>,
    max_queue_entries: usize,
    consent_scopes: Vec<String>,
    approval_inputs: Option<String>,
    /// The signup flag, read once per pass with everything else. Off for an
    /// invited contributor, whose entries carry no eligibility at all -- see
    /// `QueueEntry::eligibility`.
    admission_evidence: bool,
    /// Whether an unattended approval may happen this pass. Evaluated once,
    /// with the config, because every requirement it checks today is about
    /// the contributor rather than a particular session.
    gate: super::automatic_gate::GateVerdict,
    /// The grant terms in force, from the same config and settings this pass
    /// reads. `None` without a config. See `sweep_grants`.
    grant_terms: Option<super::grant_terms::GrantTerms>,
    /// Each source's name and root, for the automatic grant's per-source
    /// record: from the same `SourceRoots` the pass's sources were built
    /// from, never a second read of the settings. See `tick_over`.
    source_identities: SourceIdentities,
}

impl PassContext {
    /// A source as the automatic grant records it: its name and its root, so
    /// a harness pointed at another root is recorded afresh.
    fn source_key(&self, name: &str) -> String {
        match self.source_identities.get(name) {
            Some(root) => format!("{name} {root}"),
            None => name.to_string(),
        }
    }

    fn read(
        shared: &DaemonShared,
        now: DateTime<Utc>,
        max_queue_entries: usize,
        source_identities: SourceIdentities,
    ) -> Self {
        // The terms an auto-approval would be given under: the consent scopes,
        // and a fingerprint of everything else outside the session file that
        // determines the envelope. See `QueueEntry::approved_scopes` /
        // `approved_inputs`.
        //
        // Read once per pass rather than once per eligible candidate. It was
        // per-candidate, which meant re-reading and re-parsing the contributor
        // config file once for every session in the corpus on every poll -- and
        // a tick is a snapshot of the settings already (`max_queue_entries` and
        // the source declarations are taken once, above), so there is nothing
        // for a mid-pass re-read to be more correct about.
        let cfg = shared.store.load_config().ok().flatten();
        let consent_scopes = cfg
            .as_ref()
            .map(|c| c.consent_scopes.clone())
            .unwrap_or_default();
        let (near_ai, attested_bodies) = {
            let s = shared.settings.lock().expect("settings lock");
            (s.near_ai.clone(), s.ironwire_attested_bodies)
        };
        let approval_inputs = cfg.as_ref().map(|c| {
            crate::daemon::preview::input_fingerprint(c, near_ai.as_ref(), attested_bodies)
        });
        let grant_terms = cfg.as_ref().map(|c| {
            super::grant_terms::GrantTerms::current(
                c,
                near_ai.as_ref(),
                attested_bodies,
                &super::grant_terms::env_filter_backend(),
            )
        });
        let admission_evidence = cfg
            .as_ref()
            .and_then(|c| c.witness.as_ref())
            .is_some_and(|w| w.admission_evidence);
        let gate = super::automatic_gate::evaluate(cfg.as_ref(), gate_enforced());
        Self {
            now,
            max_queue_entries,
            consent_scopes,
            approval_inputs,
            admission_evidence,
            gate,
            grant_terms,
            source_identities,
        }
    }
}

/// What a pass accumulates across the sessions it visits.
#[derive(Default)]
struct PassOutcome {
    report: TickReport,
    /// Whether anything the queue holds moved, and so whether the epilogue
    /// has a save and a publish to do.
    changed: bool,
    /// Whether this pass met a session its source declines to read at all.
    /// Separate from `report.too_large` only so the epilogue reads as the
    /// condition it is testing rather than as a count.
    too_large: bool,
    unsupported_export_version: bool,
    /// Sessions the gate held this pass. Kept out of `report` until the
    /// epilogue knows whether the pass was exhaustive; see
    /// `TickReport::gate_blocked`.
    gate_blocked: usize,
}

/// Everything one session costs: observe, evaluate, ask the queue, and load
/// and hash only if all three still warrant it.
///
/// The single implementation of the per-session body. Both entry points call
/// it and neither has a copy; the locks it takes, and the order it takes them
/// in, are the ones the poll path has always taken.
fn visit_session(
    shared: &DaemonShared,
    ctx: &PassContext,
    source: &dyn TraceSource,
    session_ref: &SessionRef,
    out: &mut PassOutcome,
) {
    out.report.observed += 1;
    let Ok(meta) = std::fs::metadata(&session_ref.path) else {
        return;
    };
    let Ok(modified) = meta.modified() else {
        return;
    };
    // Size and mtime come from the `SessionRef`, not from a re-stat
    // of `path`, because a ref can cover more than one file. A
    // claude-code session's delegated transcripts live beside it
    // under `<uuid>/subagents/`, and `path` deliberately stays the
    // parent file so the queue and the upload state keep one stable
    // address per conversation. Judging quiescence on the parent's
    // own mtime would therefore make a subagent that is still being
    // written completely invisible: the daemon would call the group
    // finished mid-delegation, and would never re-offer a
    // conversation that gained forty transcripts after the parent
    // went quiet. `group_modified_at` is `None` for every
    // single-file source, which is exactly the old behaviour.
    let obs = Observation {
        path: session_ref.path.clone(),
        size_bytes: session_ref.size_bytes,
        modified_at: session_ref
            .group_modified_at
            .unwrap_or_else(|| DateTime::<Utc>::from(modified)),
    };

    let (previous_size, prior) = {
        let state = shared.state.lock().expect("state lock");
        (
            state.previous_size(&obs.path),
            state.prior_upload(&obs.path).cloned(),
        )
    };

    let verdict = {
        let settings = shared.settings.lock().expect("settings lock");
        evaluate(&obs, previous_size, prior.as_ref(), ctx.now, &settings)
    };

    // Record the observation regardless, so the next poll can judge
    // size stability.
    {
        let mut state = shared.state.lock().expect("state lock");
        state.observe(&obs.path, obs.size_bytes);
    }

    if verdict != Eligibility::Eligible {
        return;
    }

    // Eligibility cannot tell "already offered and unchanged" from
    // "never offered": it is decided from the observation, the
    // previous poll's size and any prior *upload*, and a session
    // sitting in the queue `Pending` has never uploaded, so it has
    // no prior and comes back `Eligible` forever. Ask the queue
    // before paying for a load. `unchanged_offer_at_path` compares
    // the whole observation -- group size and group mtime -- and
    // deliberately declines whenever a live offer here was built
    // from a different one, so a grown session (a new delegated
    // transcript included) still reaches `replace_live_at_path` and
    // still supersedes.
    //
    // The same lock answers the other half of "can this load
    // produce anything?": a session the queue holds no offer for at
    // all, because the queue is at `ctx.max_queue_entries`. See
    // `Queue::load_can_land` -- and note it answers `true`
    // whenever a live entry sits at this path, so a grown session
    // with a live card still reaches the load and still
    // supersedes, full queue or not.
    //
    // The same lock also answers the question that outranks both: has the
    // contributor already said no to this conversation? See
    // `Queue::dismissed_at_path`. It is checked here, in front of the
    // load, rather than after it, because a declined session someone keeps
    // working in would otherwise be read, parsed and group-hashed on every
    // poll for the rest of its life for a result nothing may act on.
    let (dismissed, already_offered, can_land) = {
        let queue = shared.queue.lock().expect("queue lock");
        (
            queue.dismissed_at_path(&obs.path),
            queue
                .unchanged_offer_at_path(&obs.path, obs.size_bytes, obs.modified_at)
                .map(|e| {
                    (
                        e.entry_id,
                        e.project_key.clone(),
                        e.state,
                        e.held_for_review(),
                    )
                }),
            queue.load_can_land(&obs.path, ctx.max_queue_entries),
        )
    };
    // Ahead of every other verdict, including the standing opt-in below: a
    // project's `auto_upload` is a standing yes to sessions the contributor
    // has not ruled on, never an override of one they have declined.
    if dismissed {
        out.report.dismissed += 1;
        return;
    }
    if let Some((entry_id, project_key, state, held_for_review)) = already_offered {
        // The project key is taken from the entry rather than
        // re-derived, which also skips `resolve_cwd`: the entry's
        // key came from this same unchanged content, and resolving
        // it again is another per-poll lock and cache probe over
        // the whole corpus for an answer that cannot have moved.
        let mode = {
            let policy = shared.policy.lock().expect("policy lock");
            policy.resolve(&project_key)
        };
        if mode == ProjectMode::Ignore {
            out.report.ignored += 1;
            return;
        }
        // Armed, but not yet settled: leave it `Pending` and look again next
        // poll. The entry stays in the queue meanwhile, so a contributor who
        // opens the app can still approve it by hand -- arming is a standing
        // yes to sending finished work unattended, not a refusal to let them
        // act sooner. See `eligibility::ARMED_SETTLE_SECS`.
        if mode == ProjectMode::AutoUpload
            && state == QueueState::Pending
            && !armed_settle_elapsed(obs.modified_at, ctx.now)
        {
            out.report.armed_not_settled += 1;
            return;
        }
        // The one thing the discarded dedup path did do: re-apply a
        // project's standing opt-in to an entry that has since been
        // put back to `Pending` (by a supersede, or by the
        // consent-scope guard). `approve` only moves `Pending`, so
        // it can never resurrect a dismissed, expired or uploaded
        // entry. Preserved here so skipping the load costs nothing
        // but the load.
        // Through the automatic-contribution gate, like every other approval
        // made on the contributor's behalf. See `automatic_gate`. A session
        // the grant holds back would not be approved either way, so it is
        // not counted as one the gate holds; nor is one held for a person.
        let would_approve = mode == ProjectMode::AutoUpload
            && state == QueueState::Pending
            && !held_for_review
            && !held_back_from_the_grant(shared, &project_key, &obs.path);
        if would_approve && ctx.gate.blocks() {
            out.gate_blocked += 1;
        } else if would_approve {
            let mut queue = shared.queue.lock().expect("queue lock");
            if queue.approve_unattended(
                entry_id,
                &ctx.consent_scopes,
                ctx.approval_inputs.as_deref(),
            ) {
                out.changed = true;
                out.report.auto_ready += 1;
                if ctx.gate.would_refuse() {
                    out.report.gate_would_refuse += 1;
                }
            }
        }
        return;
    }

    // A full queue with no live entry at this path: whatever the
    // load produced, `replace_live_at_path` would refuse it
    // `queue-full` and the work would be discarded. Refuse it here
    // instead, before the read, the parse and the group hash, and
    // raise the same health label the refusal below raises -- the
    // contributor's queue is genuinely full and sessions are going
    // unoffered, which is the same condition either way.
    if !can_land {
        let mut health = shared.health.lock().expect("health lock");
        health.fail(health::LABEL_QUEUE_FULL, ctx.now);
        return;
    }

    let cwd = resolve_cwd(shared, source, session_ref, &obs);
    let (project_key, project_path) = project_for(cwd.as_deref());
    let mode = {
        let policy = shared.policy.lock().expect("policy lock");
        policy.resolve(&project_key)
    };
    let mode = if mode == ProjectMode::NotifyOnly {
        arm_by_default(shared, ctx, source, &project_key, &obs.path)
    } else {
        mode
    };
    if mode == ProjectMode::Ignore {
        out.report.ignored += 1;
        return;
    }

    // Hashing reads the whole group, so it happens only here: for a
    // session the queue has no unchanged offer for and could still
    // hold the result of. Everything already offered at this exact
    // observation, and everything a full queue could not have
    // taken, was skipped above.
    // A failed load used to be a bare `continue`, and that was the whole
    // defect: `source::codex` declines an oversized rollout *by name*,
    // saying in its own comment that the refusal is "named rather than
    // silent", and this line then made it silent. The session never
    // entered the queue, no counter moved, nothing was written down, and no
    // shell could ever tell the contributor why a conversation they
    // finished simply does not exist as far as the tool is concerned.
    //
    // Stable size and unsupported-export-version refusals raise standing
    // health labels. Other load errors increment the error count without
    // pinning a transient IO or parse failure on daemon health. A complete
    // pass clears a refusal label only when it observes no such refusal.
    // Neither path logs file paths or imported content.
    let transcript = match source.load(session_ref) {
        Ok(t) => t,
        Err(err) => {
            out.report.unloadable += 1;
            if let Some(too_large) = err.downcast_ref::<crate::source::SessionTooLarge>() {
                out.report.too_large += 1;
                out.too_large = true;
                tracing::warn!(
                    refusal = too_large.label,
                    declared_bytes = too_large.declared_bytes,
                    budget_bytes = too_large.budget_bytes,
                    "declined a session larger than its source will read"
                );
                let mut health = shared.health.lock().expect("health lock");
                health.fail(health::LABEL_SESSION_TOO_LARGE, ctx.now);
            } else if err
                .downcast_ref::<crate::source::opencode::UnsupportedExportVersion>()
                .is_some()
            {
                out.unsupported_export_version = true;
                shared
                    .health
                    .lock()
                    .expect("health lock")
                    .fail(health::LABEL_OPENCODE_EXPORT_VERSION_UNSUPPORTED, ctx.now);
            } else {
                // No `err` in the field set: an IO error's `Display` is
                // free to carry the path it failed on, and a log line is
                // not a place a path may appear.
                tracing::debug!("a session could not be read this pass");
            }
            return;
        }
    };

    // Collision detection must see every project the daemon knows
    // about -- both configured policy entries and projects already
    // sitting in the queue -- so a collision is visible as soon as
    // either colliding project has a queue entry. The end-of-tick
    // relabel pass below is what makes this symmetric across the
    // whole colliding set, since this per-entry snapshot alone can
    // still miss a project discovered later in the same pass.
    let known = {
        let policy = shared.policy.lock().expect("policy lock");
        let queue = shared.queue.lock().expect("queue lock");
        known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()))
    };

    // Two independent restrictions on arming, and a fresh entry has to clear
    // both. They arrived from different directions -- the settle window from
    // #515, the staging exclusion from the trajectory-import work -- and each
    // one dropped is a session sent unattended that should not have been, so
    // they are ANDed rather than either replacing the other.

    // A staged trajectory is never armed, whatever the project mode says.
    //
    // The daemon's only trajectory scope is the staging directory (see
    // `DaemonSettings::source_roots`), so a trajectory ref reaching this
    // point IS an import. It was invisible to this daemon until the staging
    // scope existed, and auto-uploading on first sight would send something
    // the contributor may not remember importing, with no prompt. They
    // armed a watched source they had declared; this is not one.
    //
    // The check is on the adapter rather than on `declared_source` on
    // purpose: it must hold for every staged trajectory, including one a
    // contributor dropped in by hand, not only for the ones that name
    // themselves.
    let from_staging = session_ref.source == crate::source::SOURCE_TRAJECTORY;

    // A fresh entry from an armed project is queued `Pending` until it has
    // settled, not `Approved` on sight. The next poll promotes it once the
    // window has elapsed (site above), and a session that grows in the
    // meantime supersedes this entry -- free, where the same growth after an
    // upload would cost one of three re-uploads and a duplicate penalty.
    // Through the automatic-contribution gate. `armed` decides both paths
    // below that approve on the contributor's behalf -- a fresh entry created
    // `Approved`, and an already-queued one re-approved -- so gating it here
    // gates both. See `automatic_gate`.
    let would_arm = mode == ProjectMode::AutoUpload
        && !from_staging
        && armed_settle_elapsed(obs.modified_at, ctx.now)
        && !held_back_from_the_grant(shared, &project_key, &obs.path);
    let armed = would_arm && !ctx.gate.blocks();
    // What the gate held back, counted so that enforcing it cannot stop an
    // armed folder without saying so.
    let gate_held = would_arm && ctx.gate.blocks();

    // Two questions off one transcript load. The mark is answered for every
    // contributor; the eligibility verdict is a derivation from it that stays
    // silent unless the signup flag applies. Both are free here and nowhere
    // else -- see `QueueEntry::attestation`.
    let attestation = super::attestation_mark::evaluate(
        &transcript.routing,
        transcript.attested_call.as_deref(),
        transcript.attested_refusal,
    );
    let eligibility = super::contribution_eligibility::evaluate(
        ctx.admission_evidence,
        &transcript.routing,
        transcript.attested_call.as_deref(),
        transcript.attested_refusal,
    );

    let entry = QueueEntry {
        entry_id: entry_id_for(&transcript.session_hash),
        // Follows `armed`, because the `state` below is `Approved` on the
        // same condition: a settled session in an armed project is approved
        // here on first sight, without ever being `Pending`. That is the
        // backlog case retraction exists for, so recording it as the
        // contributor's own would make `retract_unattended_for_project` skip
        // exactly the entries it is meant to reach.
        approved_unattended: armed,
        session_hash: transcript.session_hash.clone(),
        source: session_ref.source.to_string(),
        declared_source: session_ref.declared_source.clone(),
        project_key: project_key.clone(),
        // The unfolded spelling of the same directory, for rendering only.
        project_path: project_path.clone(),
        // The raw recorded cwd, which `project_key_for` normalized away.
        session_cwd: cwd.clone(),
        project_label: disambiguated_label(&project_key, project_path.as_deref(), &known),
        path: obs.path.clone(),
        size_bytes: obs.size_bytes,
        discovered_at: ctx.now,
        state: if armed {
            // Opted in, so it needs no decision; the uploader picks it
            // up on its next pass.
            QueueState::Approved
        } else {
            QueueState::Pending
        },
        reason_label: None,
        attempts: 0,
        retry_after: None,
        submission_id: None,
        approved_scopes: armed.then(|| ctx.consent_scopes.clone()),
        // A fresh entry has no answer to give yet, armed or not: it is
        // either newly discovered (`Pending`) or auto-approved without
        // a contributor ever seeing it, so there is no verdict to
        // record.
        approved_verdict: None,
        approved_correction: None,
        // `None` when the config could not be read, which the
        // uploader treats as "unknown, re-ask": fail-closed.
        approved_inputs: armed.then(|| ctx.approval_inputs.clone()).flatten(),
        // An armed project's sessions are never previewed, so there
        // is no shown artifact to pin to. The input fingerprint is
        // the guard that applies to them.
        previewed_envelope_digest: None,
        // No post-approval hold on a standing opt-in: it is a
        // decision taken in advance, separately audited, with no
        // click to take back and no client counting down for it.
        // See `Queue::approve`.
        approved_at: None,
        subagent_count: transcript.subagent_count,
        subagents_dropped: transcript.subagents_dropped,
        // The observation this entry is made of, so the next poll
        // can recognize it without reading the group again. See
        // `QueueEntry::observed_modified_at`.
        observed_modified_at: Some(obs.modified_at),
        // Free here and nowhere else. The load above already joined this
        // session's ledger hops and, where a body store is configured,
        // already ran the full attested check; recording what they said
        // costs two labels. A list that asked the question instead would
        // pay for a re-read and re-hash of every captured body in the
        // queue, every time anything called it.
        eligibility: eligibility.map(|v| v.state.to_string()),
        eligibility_reason: eligibility.and_then(|v| v.reason).map(str::to_string),
        attestation: Some(attestation.state.to_string()),
        attestation_reason: attestation.reason.map(str::to_string),
        attested_inference: None,
    };
    let entry_id = entry.entry_id;

    let mut queue = shared.queue.lock().expect("queue lock");
    // Add the new offer and retire any earlier one for this same
    // session in a single step. Without the retirement, a
    // conversation that gains a delegated transcript accumulates a
    // card per delegation -- `upsert` dedups on hash, and the hash
    // is precisely what moved. Without the atomicity, a `queue-full`
    // between the two would retire the old offer and never land the
    // replacement, leaving the conversation with no live card at
    // all. See `Queue::replace_live_at_path`.
    match queue.replace_live_at_path(entry, ctx.max_queue_entries) {
        Ok(outcome) => {
            if outcome.superseded > 0 {
                out.changed = true;
            }
            if outcome.inserted {
                out.changed = true;
                if armed {
                    out.report.auto_ready += 1;
                    if ctx.gate.would_refuse() {
                        out.report.gate_would_refuse += 1;
                    }
                } else {
                    out.report.queued += 1;
                    if gate_held {
                        out.gate_blocked += 1;
                    }
                }
                // A new entry passed the capacity check: there is
                // space in the queue.
                let mut health = shared.health.lock().expect("health lock");
                health.resolve(health::LABEL_QUEUE_FULL);
            } else {
                // Dedup path: re-observing an already-queued session.
                // The insert deliberately never rewrites an existing
                // entry, which used to mean a standing opt-in simply
                // stopped applying to an entry that had been put
                // back to `Pending` since it was created -- by
                // `supersede`, or by the consent-scope guard. The
                // entry sat `Pending` until it aged out, in a
                // project the contributor had explicitly armed.
                // Re-apply the standing decision here, which is the
                // one place that knows both the entry and the mode
                // in force.
                //
                // `Queue::approve` only moves `Pending`, so this can
                // never resurrect a dismissed-and-refused, expired,
                // or already-uploaded entry.
                // `ctx.approval_inputs` is passed through as `Option`,
                // not flattened to `""`: the insert path above
                // records `None` for "the config could not be read",
                // and this path recording `Some("")` for the same
                // condition made two spellings of "unknown". Both
                // fail closed, but the uploader should only have one
                // shape to recognize. `None` for `approved_at`,
                // matching the fresh-entry path above: a standing
                // opt-in is not held.
                if armed
                    && queue.approve_unattended(
                        entry_id,
                        &ctx.consent_scopes,
                        ctx.approval_inputs.as_deref(),
                    )
                {
                    out.changed = true;
                    out.report.auto_ready += 1;
                    if ctx.gate.would_refuse() {
                        out.report.gate_would_refuse += 1;
                    }
                } else if gate_held
                    && queue
                        .get(entry_id)
                        .is_some_and(|e| e.state == QueueState::Pending && !e.held_for_review())
                {
                    // Held for a person, it waits on them, not on the gate.
                    out.gate_blocked += 1;
                }
                // This path returns Ok without checking capacity, so
                // it does not prove space is available. Do not
                // retract queue-full.
            }
        }
        Err(_) => {
            let mut health = shared.health.lock().expect("health lock");
            health.fail(health::LABEL_QUEUE_FULL, ctx.now);
        }
    }
}

/// Say what the gate refused, or would have refused.
///
/// Only when it mattered, so a daemon with nothing armed logs nothing. What
/// the gate holds is a level, so it is logged when a full pass finds it
/// changed -- the count or the unmet reasons -- rather than on every poll:
/// one session held for a day would otherwise log the same line every poll
/// interval, and a count-only comparison would leave the last line showing
/// stale reasons. The reasons are labels, never paths or content.
fn report_gate(
    shared: &DaemonShared,
    gate: &super::automatic_gate::GateVerdict,
    report: &TickReport,
) -> Option<HeldLog> {
    let reasons: Vec<&'static str> = gate.unmet.iter().map(|u| u.reason).collect();
    let mut logged = None;
    if let Some(held) = report.gate_blocked {
        // What the last "held" line said: the count and the reasons, so a
        // change of reasons at the same count is logged too. Nothing held has
        // no reasons worth comparing.
        let now = (
            held,
            if held > 0 {
                reasons.clone()
            } else {
                Vec::new()
            },
        );
        let before = std::mem::replace(
            &mut *shared
                .gate_held_logged
                .lock()
                .unwrap_or_else(|e| e.into_inner()),
            now.clone(),
        );
        if now != before && held > 0 {
            tracing::info!(
                held,
                unmet = ?reasons,
                "the automatic-contribution gate is holding sessions in armed folders for the contributor"
            );
            logged = Some(HeldLog::Holding);
        } else if now != before {
            tracing::info!("the automatic-contribution gate is no longer holding any sessions");
            logged = Some(HeldLog::Released);
        }
    }
    if report.gate_would_refuse > 0 {
        tracing::info!(
            approvals = report.gate_would_refuse,
            unmet = ?reasons,
            "approved on the contributor's behalf; the automatic-contribution gate would have refused these"
        );
    }
    logged
}

/// Which "held" line `report_gate` wrote, if any. Returned so the log-on-change
/// rule can be tested without capturing log output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeldLog {
    Holding,
    Released,
}

/// Everything a pass owes once it has visited its sessions: the relabel pass,
/// the queue save and envelope sweep, the change publish, and the state save.
///
/// The single implementation of the epilogue, and it runs once per pass, not
/// once per session -- `queue.save` and `state.save` each rewrite a whole
/// file, and the publish is a wake-up for every subscribed shell.
fn finish_pass(shared: &DaemonShared, out: PassOutcome, exhaustive: bool) -> Result<TickReport> {
    let PassOutcome {
        mut report,
        mut changed,
        too_large,
        unsupported_export_version,
        gate_blocked,
    } = out;
    report.gate_blocked = exhaustive.then_some(gate_blocked);

    // Retract the unreadable-session flag only from a pass that asked every
    // source what it has and found nothing it could not read. A scoped pass
    // visits the handful of sessions some paths resolved to, so it is
    // entitled to *raise* the flag -- it saw one -- and never to clear it:
    // one readable session says nothing about the rest of the corpus, and a
    // status indicator that blinks off every time an unrelated file is
    // written is not one anybody will trust. A condition that cannot
    // retract itself masks every lower-precedence one forever, which is why
    // this is here at all.
    if exhaustive && !too_large {
        let mut health = shared.health.lock().expect("health lock");
        health.resolve(health::LABEL_SESSION_TOO_LARGE);
    }

    if exhaustive && !unsupported_export_version {
        shared
            .health
            .lock()
            .expect("health lock")
            .resolve(health::LABEL_OPENCODE_EXPORT_VERSION_UNSUPPORTED);
    }

    // Relabel pass: `Queue::upsert` never rewrites an existing entry, so a
    // project that was unique when its entry was first queued would
    // otherwise keep a bare label forever even after a colliding project
    // later gets its own entry (or is configured in policy). Recomputing
    // every entry's label against the final known-key set for this tick
    // means a collision is always visible on *every* member of the
    // colliding set, not just whichever was processed second.
    if relabel_queue(shared) {
        changed = true;
    }

    // Release stored preview envelopes nobody is waiting on: pending
    // previews the contributor never acted on, and, if the store is over
    // its ceiling, the oldest pending previews until it is not. This runs
    // whether or not the tick found anything, because a queue that has
    // stopped changing is exactly the state in which stale previews
    // accumulate -- every entry pending, nothing resolving, the files kept
    // forever. Releasing a pin is a change the sweep below acts on, so it
    // sets `changed`.
    {
        let mut queue = shared.queue.lock().expect("queue lock");
        if !crate::daemon::approved_envelope::release_stale_pins(&shared.store, &mut queue)
            .is_empty()
        {
            changed = true;
        }
    }

    if changed {
        {
            let queue = shared.queue.lock().expect("queue lock");
            queue.save(&shared.store)?;
            // An expired or superseded entry keeps no redacted trace
            // content on disk. Best-effort: a file that will not delete
            // must not fail a poll.
            let _ =
                crate::daemon::approved_envelope::sweep(&shared.store, &queue.pinned_entry_ids());
        }
        shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
    }
    {
        let mut state = shared.state.lock().expect("state lock");
        state.save(&shared.store)?;
    }
    Ok(report)
}

/// End-of-tick pass: recompute every queue entry's `project_label` against
/// the tick's final known-key set and rewrite any that changed.
///
/// `Queue::upsert` deliberately never touches an existing entry, so without
/// this pass an entry queued while its basename was still unique would keep
/// a bare label forever, even after a colliding project shows up in a later
/// tick. The actual relabeling logic lives in `ipc::relabel_queue_entries`,
/// which `set_project_mode` also calls (immediately after a policy edit,
/// rather than waiting for the next poll) -- this wrapper only owns taking
/// the locks tick() needs anyway.
fn relabel_queue(shared: &DaemonShared) -> bool {
    let policy = shared.policy.lock().expect("policy lock");
    let mut queue = shared.queue.lock().expect("queue lock");
    super::ipc::relabel_queue_entries(&policy, &mut queue)
}

/// The session's working directory, from cache when the file has not changed.
fn resolve_cwd(
    shared: &DaemonShared,
    source: &dyn TraceSource,
    session_ref: &SessionRef,
    obs: &Observation,
) -> Option<String> {
    let key = obs.path.to_string_lossy().to_string();
    {
        let state = shared.state.lock().expect("state lock");
        if let Some(hit) = state.cwd_cache.get(&key) {
            if hit.size_bytes == obs.size_bytes && hit.modified_at == obs.modified_at {
                return hit.cwd.clone();
            }
        }
    }
    // Discovery may already know it; otherwise this reads the file.
    let cwd = session_ref
        .cwd
        .clone()
        .or_else(|| source.load(session_ref).ok().and_then(|t| t.cwd));
    let mut state = shared.state.lock().expect("state lock");
    state.cwd_cache.insert(
        key,
        CwdCacheEntry {
            size_bytes: obs.size_bytes,
            modified_at: obs.modified_at,
            cwd: cwd.clone(),
        },
    );
    cwd
}

#[cfg(test)]
mod tests {
    use super::super::policy::project_key_for;
    use super::*;
    use crate::config::ConfigStore;
    use crate::daemon::policy::ProjectMode;
    use crate::daemon::test_paths::{abs, abs_json, json_escaped};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A real source with a counter around its `load`.
    ///
    /// "The poll did not re-read anything" is a claim about how many times
    /// `TraceSource::load` ran, and nothing observable from the queue can
    /// prove it -- a pass that loads every session and then discards the
    /// result leaves a queue identical to one that loaded nothing. Wrapping
    /// the genuine adapter rather than faking one keeps discovery, grouping
    /// and hashing exactly as they are in production.
    struct CountingSource {
        inner: Box<dyn TraceSource>,
        loads: Arc<AtomicUsize>,
        /// The other half of the claim, for the scoped path: "this pass did
        /// not walk the corpus" is a claim about `discover`, and a scoped
        /// pass that quietly walked would be indistinguishable from one that
        /// did not by anything the queue shows.
        discovers: Arc<AtomicUsize>,
    }

    impl TraceSource for CountingSource {
        fn name(&self) -> &'static str {
            self.inner.name()
        }
        fn discover(&self) -> Result<Vec<SessionRef>> {
            self.discovers.fetch_add(1, Ordering::SeqCst);
            self.inner.discover()
        }
        fn load(&self, r: &SessionRef) -> Result<crate::source::SessionTranscript> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            self.inner.load(r)
        }
    }

    use crate::daemon::test_support::at;

    /// A daemon whose session roots are a tempdir, so a test never reads the
    /// developer's real transcripts.
    struct WatcherFixture {
        _dir: tempfile::TempDir,
        shared: DaemonShared,
        claude_root: PathBuf,
    }

    impl WatcherFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = ConfigStore::open(dir.path().join("state")).unwrap();
            let claude_root = dir.path().join("projects");
            let codex_root = dir.path().join("codex-sessions");
            std::fs::create_dir_all(&claude_root).unwrap();
            std::fs::create_dir_all(&codex_root).unwrap();
            let shared = DaemonShared::load(store).unwrap();
            {
                let mut s = shared.settings.lock().unwrap();
                s.claude_source = Some(crate::daemon::settings::SourceDeclaration::Watch {
                    path: claude_root.clone(),
                });
                s.codex_source =
                    Some(crate::daemon::settings::SourceDeclaration::Watch { path: codex_root });
            }
            Self {
                _dir: dir,
                shared,
                claude_root,
            }
        }

        /// Write a session and backdate it so it reads as quiescent.
        fn write_session(&self, project: &str, name: &str, extra_events: usize) -> PathBuf {
            let project_dir = self
                .claude_root
                .join(format!("-Users-testuser-code-{project}"));
            std::fs::create_dir_all(&project_dir).unwrap();
            let path = project_dir.join(format!("{name}.jsonl"));
            let cwd = abs_json(&format!("Users/testuser/code/{project}"));
            let mut body = format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"hello\"}},\
                 \"cwd\":\"{cwd}\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"{name}\",\"uuid\":\"a1\"}}\n"
            );
            for i in 0..extra_events {
                body.push_str(&format!(
                    "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"more {i}\"}},\
                     \"cwd\":\"{cwd}\",\
                     \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                     \"sessionId\":\"{name}\",\"uuid\":\"b{i}\"}}\n"
                ));
            }
            std::fs::write(&path, body).unwrap();
            path
        }

        /// Like `write_session`, but the session's cwd is an explicit full
        /// path rather than derived from `project`. Used to simulate two
        /// distinct projects that happen to share a basename (e.g. two
        /// checkouts both named `api`), which `write_session` alone cannot
        /// produce since it always uses the same parent directory.
        fn write_session_with_cwd(&self, dir_name: &str, cwd: &str, name: &str) -> PathBuf {
            // Escaped here rather than at each call site, so callers pass a
            // real path and not a JSON fragment.
            let cwd = json_escaped(cwd);
            let project_dir = self.claude_root.join(dir_name);
            std::fs::create_dir_all(&project_dir).unwrap();
            let path = project_dir.join(format!("{name}.jsonl"));
            let body = format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"hello\"}},\
                 \"cwd\":\"{cwd}\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"{name}\",\"uuid\":\"a1\"}}\n"
            );
            std::fs::write(&path, body).unwrap();
            path
        }

        /// Write a delegated transcript under `<session>/subagents/`,
        /// stamped with the parent's `sessionId` so it verifies as a member.
        fn write_subagent(&self, project: &str, session: &str, agent: &str) -> PathBuf {
            let subagents = self
                .claude_root
                .join(format!("-Users-testuser-code-{project}"))
                .join(session)
                .join("subagents");
            std::fs::create_dir_all(&subagents).unwrap();
            let path = subagents.join(format!("{agent}.jsonl"));
            let cwd = abs_json(&format!("Users/testuser/code/{project}"));
            std::fs::write(
                &path,
                format!(
                    "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"delegated\"}},\
                     \"cwd\":\"{cwd}\",\
                     \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                     \"sessionId\":\"{session}\",\"uuid\":\"s1\"}}\n"
                ),
            )
            .unwrap();
            path
        }

        /// Shrink the queue cap, so a test can reach "the queue is full"
        /// with two sessions instead of five hundred.
        fn set_max_queue_entries(&self, max: usize) {
            self.shared.settings.lock().unwrap().max_queue_entries = max;
        }

        /// Set a mode for the project a `write_session` fixture records.
        ///
        /// Goes through `project_key_for` rather than using the recorded
        /// cwd verbatim, because that is what the watcher does: the key is
        /// the NORMALIZED directory, and a helper that set policy under the
        /// raw spelling would be setting it for a project nothing ever
        /// resolves to.
        fn set_mode(&self, project: &str, mode: ProjectMode) {
            self.set_mode_for_key(&abs(&format!("Users/testuser/code/{project}")), mode);
        }

        /// Like `set_mode`, but for an explicit recorded cwd rather than
        /// one derived from `/Users/testuser/code/{project}` -- needed for
        /// projects written via `write_session_with_cwd`.
        fn set_mode_for_key(&self, key: &str, mode: ProjectMode) {
            self.shared
                .policy
                .lock()
                .unwrap()
                .set_mode(
                    &project_key_for(Some(key)),
                    mode,
                    at("2026-08-08T12:00:00Z"),
                )
                .unwrap();
        }

        /// Like `set_mode`, but through the real IPC arm rather than
        /// setting the policy directly. `set_mode` above only exercises the
        /// policy layer -- the queue purge on `Ignore` lives in
        /// `ipc::handle_request`'s `set_project_mode` arm, so a test that
        /// wants to prove the purge happens has to go in this door.
        fn set_mode_via_ipc(&self, project: &str, mode: ProjectMode) {
            let req = super::super::ipc::Request {
                id: 1,
                method: "set_project_mode".to_string(),
                params: serde_json::json!({
                    "project_key": project_key_for(Some(&abs(&format!("Users/testuser/code/{project}")))),
                    "mode": mode,
                }),
            };
            let resp = super::super::ipc::handle_request(&self.shared, &req);
            assert!(resp.error.is_none(), "{:?}", resp.error);
        }

        /// Turn the signup flag on, which is what makes a queue entry
        /// carry an eligibility at all.
        fn admitted_on_evidence(&self) {
            let cfg: crate::config::ContributorConfig = serde_json::from_value(serde_json::json!({
                "schema_version": crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
                "issuer_url": "https://issuer.example",
                "ingest_url": "https://ingest.example",
                "audience": "upload",
                "tenant_id": format!("near-{}", "ab".repeat(32)),
                "instance_id": "",
                "user_subject": "device",
                "device_key_id": "device",
                "consent_scopes": ["debugging_evaluation"],
                "witness": {
                    "url": "https://witness.example",
                    "signing_address": format!("0x{}", "ab".repeat(20)),
                    "expected_measurements": [format!("mrtd={}", "ab".repeat(48))],
                    "admission_evidence": true,
                },
            }))
            .unwrap();
            self.shared.store.save_config(&cfg).unwrap();
        }

        fn queue_len(&self) -> usize {
            self.shared.queue.lock().unwrap().all().len()
        }

        fn states(&self) -> Vec<QueueState> {
            self.shared
                .queue
                .lock()
                .unwrap()
                .all()
                .iter()
                .map(|e| e.state)
                .collect()
        }

        /// Two ticks: the first records a size, the second can confirm it is
        /// stable. Eligibility deliberately never fires on a first sighting.
        async fn settle(&self, now: DateTime<Utc>) -> TickReport {
            tick(&self.shared, now).await.unwrap();
            tick(&self.shared, now).await.unwrap()
        }

        /// One pass -- the same `tick_over` `tick` runs -- over sources that
        /// count their `load` calls.
        fn tick_counted(&self, now: DateTime<Utc>, loads: &Arc<AtomicUsize>) -> TickReport {
            let (max_queue_entries, source_roots) = {
                let s = self.shared.settings.lock().unwrap();
                (s.max_queue_entries, s.source_roots(&self.shared.store))
            };
            let sources = all_sources(&source_roots)
                .into_iter()
                .map(|inner| {
                    Box::new(CountingSource {
                        inner,
                        loads: loads.clone(),
                        discovers: Arc::new(AtomicUsize::new(0)),
                    }) as Box<dyn TraceSource>
                })
                .collect();
            tick_over(
                &self.shared,
                now,
                sources,
                source_roots.source_identities(),
                max_queue_entries,
            )
            .unwrap()
        }

        /// One *scoped* pass -- the same `tick_over_paths` `tick_paths` runs
        /// -- over sources that count both their `load` and their `discover`
        /// calls.
        ///
        /// The path-to-session lookup stands in for what the source layer
        /// still owes this path (`TraceSource::session_at`). It is answered
        /// from a second, uncounted set of adapters, so the counters the
        /// tests assert on only ever see what the pass itself did.
        fn tick_paths_counted(
            &self,
            now: DateTime<Utc>,
            loads: &Arc<AtomicUsize>,
            discovers: &Arc<AtomicUsize>,
            paths: &[PathBuf],
        ) -> TickReport {
            let (max_queue_entries, source_roots) = {
                let s = self.shared.settings.lock().unwrap();
                (s.max_queue_entries, s.source_roots(&self.shared.store))
            };
            let sources = all_sources(&source_roots)
                .into_iter()
                .map(|inner| {
                    Box::new(CountingSource {
                        inner,
                        loads: loads.clone(),
                        discovers: discovers.clone(),
                    }) as Box<dyn TraceSource>
                })
                .collect();
            let known: Vec<(&'static str, SessionRef)> = all_sources(&source_roots)
                .iter()
                .flat_map(|s| {
                    let name = s.name();
                    s.discover()
                        .unwrap_or_default()
                        .into_iter()
                        .map(move |r| (name, r))
                        .collect::<Vec<_>>()
                })
                .collect();
            let session_at = |source: &dyn TraceSource, path: &Path| {
                known
                    .iter()
                    .find(|(name, r)| *name == source.name() && r.path == path)
                    .map(|(_, r)| r.clone())
            };
            tick_over_paths(
                &self.shared,
                now,
                sources,
                source_roots.source_identities(),
                max_queue_entries,
                paths,
                &session_at,
            )
            .unwrap()
        }

        /// `settle`, scoped: two scoped passes, since eligibility never fires
        /// on a first sighting no matter which path made the observation.
        fn settle_paths(
            &self,
            now: DateTime<Utc>,
            loads: &Arc<AtomicUsize>,
            discovers: &Arc<AtomicUsize>,
            paths: &[PathBuf],
        ) -> TickReport {
            self.tick_paths_counted(now, loads, discovers, paths);
            self.tick_paths_counted(now, loads, discovers, paths)
        }

        /// The queue-changed events a pass published, drained.
        fn drain_changed(
            rx: &mut tokio::sync::broadcast::Receiver<crate::daemon::ipc::Event>,
        ) -> usize {
            let mut n = 0;
            while let Ok(ev) = rx.try_recv() {
                if ev.event == EVENT_QUEUE_CHANGED {
                    n += 1;
                }
            }
            n
        }

        /// `settle`, counted.
        fn settle_counted(&self, now: DateTime<Utc>, loads: &Arc<AtomicUsize>) -> TickReport {
            self.tick_counted(now, loads);
            self.tick_counted(now, loads)
        }

        /// Append to an existing session file, i.e. the conversation
        /// continued after it was offered.
        fn append_to_session(&self, path: &std::path::Path, project: &str, name: &str) {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
            let cwd = abs_json(&format!("Users/testuser/code/{project}"));
            for i in 0..40 {
                writeln!(
                    f,
                    "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"later {i}\"}},\
                     \"cwd\":\"{cwd}\",\
                     \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                     \"sessionId\":\"{name}\",\"uuid\":\"c{i}\"}}"
                )
                .unwrap();
            }
        }
    }

    fn loads() -> Arc<AtomicUsize> {
        Arc::new(AtomicUsize::new(0))
    }

    fn count(c: &Arc<AtomicUsize>) -> usize {
        c.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn a_quiesced_session_is_queued_for_a_notify_only_project() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 1, "{report:?}");
        assert_eq!(f.queue_len(), 1);
        assert_eq!(f.states(), vec![QueueState::Pending]);
    }

    #[tokio::test]
    async fn a_session_still_being_written_is_not_queued() {
        // Only one tick, so size stability was never confirmed.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let report = tick(&f.shared, at("2030-01-01T00:00:00Z")).await.unwrap();
        assert_eq!(report.queued, 0);
        assert_eq!(f.queue_len(), 0);
    }

    #[tokio::test]
    async fn a_recently_written_session_is_not_queued() {
        // The fixture file's mtime is genuinely now, so judging it against
        // the present clock is exactly the live-session case.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let report = f.settle(Utc::now()).await;
        assert_eq!(report.queued, 0, "a live session must not be offered");
    }

    #[tokio::test]
    async fn an_ignored_project_is_never_queued() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::Ignore);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 0);
        assert_eq!(report.ignored, 1);
        assert_eq!(f.queue_len(), 0);
    }

    #[tokio::test]
    async fn an_opted_in_project_is_queued_already_approved() {
        // Opting the project in is the decision; the entry needs no second one.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.auto_ready, 1, "{report:?}");
        assert_eq!(report.queued, 0);
        assert_eq!(f.states(), vec![QueueState::Approved]);
    }

    /// Arming is a standing yes to sending *finished* work unattended, not
    /// to sending whatever has been quiet for half an hour. The fixture's
    /// mtime is genuinely now, so an hour later is past quiescence and well
    /// inside the settle window -- exactly a session paused over lunch.
    #[tokio::test]
    async fn an_opted_in_session_is_not_approved_before_it_settles() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        let report = f.settle(Utc::now() + chrono::Duration::hours(1)).await;
        assert_eq!(report.auto_ready, 0, "{report:?}");
        assert_eq!(
            f.states(),
            vec![QueueState::Pending],
            "an unsettled armed session waits in the queue rather than being sent"
        );
    }

    /// ...and it is a wait, not a refusal: the same entry is promoted once
    /// the window elapses, with no new discovery and no second decision.
    #[tokio::test]
    async fn an_opted_in_session_is_approved_once_it_settles() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(Utc::now() + chrono::Duration::hours(1)).await;
        assert_eq!(f.states(), vec![QueueState::Pending]);

        let report = tick(&f.shared, Utc::now() + chrono::Duration::hours(25))
            .await
            .unwrap();
        assert_eq!(report.auto_ready, 1, "{report:?}");
        assert_eq!(f.states(), vec![QueueState::Approved]);
        assert_eq!(f.queue_len(), 1, "promotion must not mint a second entry");
    }

    /// The wait is counted, and counted apart from `auto_ready`, so an
    /// operator reading a tick report can tell "holding" from "nothing
    /// armed here".
    #[tokio::test]
    async fn an_unsettled_armed_session_is_reported_as_waiting() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(Utc::now() + chrono::Duration::hours(1)).await;
        let report = tick(&f.shared, Utc::now() + chrono::Duration::hours(2))
            .await
            .unwrap();
        assert_eq!(report.armed_not_settled, 1, "{report:?}");
        assert_eq!(report.auto_ready, 0, "{report:?}");
    }

    #[tokio::test]
    async fn a_backlog_approved_on_first_sight_is_marked_unattended() {
        // The case the flag exists for, and the one `push_for_test` unit
        // tests cannot reach: an armed project's settled session is created
        // `Approved` directly by `visit_session`, never passing through
        // `Pending` or `approve_unattended`. Reviewed as High on #997
        // because the entry was being recorded as the contributor's own.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        // Past ARMED_SETTLE_SECS, so it is approved rather than held.
        let report = f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        assert_eq!(report.auto_ready, 1, "{report:?}");

        let entries = f.shared.queue.lock().unwrap().all().to_vec();
        let e = entries.first().expect("one entry");
        assert_eq!(e.state, QueueState::Approved, "armed and settled");
        assert!(
            e.approved_unattended,
            "nobody decided this; excluding the project must be able to retract it"
        );
    }

    #[tokio::test]
    async fn a_session_offered_before_the_project_was_armed_is_not_marked() {
        // The other direction. An entry that reached `Approved` because the
        // contributor approved it stays theirs, so exclusion leaves it alone.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let id = f.shared.queue.lock().unwrap().all()[0].entry_id;
        assert!(
            f.shared
                .queue
                .lock()
                .unwrap()
                .approve(id, &[], None, None, None, None)
        );

        let e = f.shared.queue.lock().unwrap().all()[0].clone();
        assert_eq!(e.state, QueueState::Approved);
        assert!(!e.approved_unattended, "the contributor decided this one");
    }

    #[tokio::test]
    async fn a_manual_approval_after_an_unattended_one_is_not_retractable() {
        // Reviewed as Medium on #997: the flag was never cleared, so an
        // auto-approval that was revoked and then approved by hand stayed
        // marked unattended and could be retracted as if nobody decided it.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        let id = f.shared.queue.lock().unwrap().all()[0].entry_id;
        assert!(f.shared.queue.lock().unwrap().all()[0].approved_unattended);

        {
            let mut q = f.shared.queue.lock().unwrap();
            assert!(q.revoke_approval(id, "scopes-changed"));
            assert!(
                !q.all()[0].approved_unattended,
                "revocation clears it with the other terms of approval"
            );
            assert!(q.approve(id, &[], None, None, None, None));
        }

        let mut q = f.shared.queue.lock().unwrap();
        assert!(!q.all()[0].approved_unattended);
        let key = q.all()[0].project_key.clone();
        assert_eq!(
            q.retract_unattended_for_project(&key),
            0,
            "the contributor approved this one by hand; it is not retractable"
        );
    }

    /// The loop this closes, driven through the watcher.
    ///
    /// With token distributions on, the uploader revokes an unattended
    /// approval with `token-distribution-review-required`, which only a
    /// person's review of that session can satisfy. The watcher used to
    /// re-approve it on the next poll without asking why it was revoked, the
    /// uploader revoked it again, and the session never uploaded.
    #[tokio::test]
    async fn a_session_held_for_a_person_is_not_re_approved_on_their_behalf() {
        let f = WatcherFixture::new();
        // The hold only exists while token distributions are on; with the
        // setting off it is released at the start of the next pass.
        f.shared
            .settings
            .lock()
            .unwrap()
            .token_distributions_contribution = true;
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        let id = f.shared.queue.lock().unwrap().all()[0].entry_id;

        // What `drain_approved` does with the uploader's ApprovalStale.
        assert!(f.shared.queue.lock().unwrap().revoke_approval(
            id,
            crate::daemon::queue::REASON_TOKEN_DISTRIBUTION_REVIEW_REQUIRED
        ));

        f.settle(Utc::now() + chrono::Duration::hours(31)).await;

        let e = f.shared.queue.lock().unwrap().all()[0].clone();
        assert_eq!(e.state, QueueState::Pending, "held, not re-approved");
        assert!(e.held_for_review());
    }

    /// Reviewed on #1010: a hold outlived its cause. Turning token
    /// distributions off now releases it on the next pass, and the standing
    /// opt-in applies again.
    #[tokio::test]
    async fn turning_token_distributions_off_releases_the_hold() {
        let f = WatcherFixture::new();
        f.shared
            .settings
            .lock()
            .unwrap()
            .token_distributions_contribution = true;
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        let id = f.shared.queue.lock().unwrap().all()[0].entry_id;
        assert!(f.shared.queue.lock().unwrap().revoke_approval(
            id,
            crate::daemon::queue::REASON_TOKEN_DISTRIBUTION_REVIEW_REQUIRED
        ));
        assert!(f.shared.queue.lock().unwrap().all()[0].held_for_review());

        f.shared
            .settings
            .lock()
            .unwrap()
            .token_distributions_contribution = false;
        f.settle(Utc::now() + chrono::Duration::hours(31)).await;

        let e = f.shared.queue.lock().unwrap().all()[0].clone();
        assert!(!e.held_for_review(), "the hold's cause is gone");
        assert_eq!(
            e.state,
            QueueState::Approved,
            "the standing opt-in applies again"
        );
    }

    /// The hold is narrow. A revocation the standing opt-in can satisfy is
    /// still re-applied: re-approving under changed scopes is what arming
    /// means.
    #[tokio::test]
    async fn a_scopes_changed_revocation_is_still_re_approved() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        let id = f.shared.queue.lock().unwrap().all()[0].entry_id;
        assert!(
            f.shared
                .queue
                .lock()
                .unwrap()
                .revoke_approval(id, crate::daemon::queue::REASON_SCOPES_CHANGED)
        );

        f.settle(Utc::now() + chrono::Duration::hours(31)).await;

        let e = f.shared.queue.lock().unwrap().all()[0].clone();
        assert_eq!(e.state, QueueState::Approved);
        assert!(!e.held_for_review());
    }

    /// Report-only, which is how the gate ships: the approval goes ahead as
    /// before, and is counted as one the gate would have refused.
    #[tokio::test]
    async fn the_unenforced_gate_approves_as_before_and_counts_what_it_would_refuse() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        let report = f.settle(Utc::now() + chrono::Duration::hours(30)).await;

        assert_eq!(report.auto_ready, 1, "{report:?}");
        assert_eq!(report.gate_would_refuse, 1, "{report:?}");
        assert_eq!(report.gate_blocked, Some(0), "unenforced, it holds nothing");
        let e = f.shared.queue.lock().unwrap().all()[0].clone();
        assert_eq!(e.state, QueueState::Approved);
    }

    /// Enforced, every route to an unattended approval stops: a fresh
    /// settled session is created waiting, not approved, and stays waiting
    /// when it is seen again. Nobody passes the gate today, so this is what
    /// switching it on without K5 would do to every armed folder.
    #[tokio::test]
    async fn the_enforced_gate_stops_every_approval_on_the_contributors_behalf() {
        ENFORCE_GATE_FOR_TEST.with(|c| c.set(true));
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);

        let first = f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        let again = f.settle(Utc::now() + chrono::Duration::hours(31)).await;
        ENFORCE_GATE_FOR_TEST.with(|c| c.set(false));

        assert_eq!(first.auto_ready, 0, "{first:?}");
        assert_eq!(again.auto_ready, 0, "{again:?}");
        // Not silently: each pass counts what the gate is holding.
        assert_eq!(first.gate_blocked, Some(1), "{first:?}");
        assert_eq!(again.gate_blocked, Some(1), "{again:?}");
        let e = f.shared.queue.lock().unwrap().all()[0].clone();
        assert_eq!(e.state, QueueState::Pending, "waits for the contributor");
        assert!(!e.approved_unattended);
    }

    /// A session held for a person is not one the gate is holding. With the
    /// gate enforced, a session in an armed project is counted as held by
    /// it; once that session is held for review instead (#1010), no gate
    /// verdict would approve it on the contributor's behalf, so counting it
    /// would put a session in the gate's "holding" line that only a person
    /// can release. The same rule `held_back_from_the_grant` already follows.
    #[tokio::test]
    async fn a_session_held_for_a_person_is_not_counted_as_held_by_the_gate() {
        ENFORCE_GATE_FOR_TEST.with(|c| c.set(true));
        let f = WatcherFixture::new();
        // The hold exists only while token distributions are on.
        f.shared
            .settings
            .lock()
            .unwrap()
            .token_distributions_contribution = true;
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        let gated = f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        let id = f.shared.queue.lock().unwrap().all()[0].entry_id;
        f.shared.queue.lock().unwrap().set_state(
            id,
            QueueState::Pending,
            Some(crate::daemon::queue::REASON_TOKEN_DISTRIBUTION_REVIEW_REQUIRED.to_string()),
        );
        let held = f.settle(Utc::now() + chrono::Duration::hours(31)).await;
        ENFORCE_GATE_FOR_TEST.with(|c| c.set(false));

        assert_eq!(gated.gate_blocked, Some(1), "{gated:?}");
        assert_eq!(held.gate_blocked, Some(0), "{held:?}");
        let e = f.shared.queue.lock().unwrap().all()[0].clone();
        assert!(e.held_for_review(), "still waiting on a person");
    }

    /// What the gate holds is a level only a full pass can measure. A scoped
    /// pass over an unrelated session must not report the held work as
    /// gone, or a health condition read from it would clear on every file
    /// change while the session is still waiting.
    #[tokio::test]
    async fn only_a_full_pass_reports_what_the_gate_is_holding() {
        ENFORCE_GATE_FOR_TEST.with(|c| c.set(true));
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        let full = f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        let unrelated = f.write_session("other", "22222222-2222-2222-2222-222222222222", 0);
        let (l, d) = (loads(), loads());
        let scoped = f.settle_paths(
            Utc::now() + chrono::Duration::hours(31),
            &l,
            &d,
            std::slice::from_ref(&unrelated),
        );
        ENFORCE_GATE_FOR_TEST.with(|c| c.set(false));

        assert_eq!(full.gate_blocked, Some(1), "{full:?}");
        assert_eq!(scoped.gate_blocked, None, "{scoped:?}");
    }

    /// The "holding" line is written when what the gate holds changes: the
    /// count, or the reasons it holds for. The same pair on the next poll
    /// logs nothing; a new reason at the same count logs again, so the last
    /// line never shows stale reasons. A scoped pass measures nothing and
    /// logs nothing.
    #[tokio::test]
    async fn the_held_line_is_logged_when_the_count_or_the_reasons_change() {
        use super::super::automatic_gate::{
            GateVerdict, REASON_ACCOUNT_ALLOWANCE_SPENT, REASON_ADMISSION_PER_SESSION,
            REASON_NO_SCOPE, Requirement, Unmet,
        };
        let f = WatcherFixture::new();
        let verdict = |unmet: &[(Requirement, &'static str)]| GateVerdict {
            unmet: unmet
                .iter()
                .map(|&(requirement, reason)| Unmet {
                    requirement,
                    reason,
                })
                .collect(),
            enforced: true,
        };
        let held = |n| TickReport {
            gate_blocked: n,
            ..TickReport::default()
        };
        let r3 = verdict(&[(Requirement::R3Admission, REASON_ADMISSION_PER_SESSION)]);
        let r3_spent = verdict(&[(Requirement::R3Admission, REASON_ACCOUNT_ALLOWANCE_SPENT)]);
        let r7 = verdict(&[(Requirement::R7Scope, REASON_NO_SCOPE)]);

        let log = |g: &GateVerdict, r: &TickReport| report_gate(&f.shared, g, r);
        assert_eq!(log(&r3, &held(Some(2))), Some(HeldLog::Holding));
        assert_eq!(log(&r3, &held(Some(2))), None, "unchanged: not every poll");
        assert_eq!(
            log(&r3, &held(None)),
            None,
            "a scoped pass measures nothing"
        );
        assert_eq!(
            log(&r3_spent, &held(Some(2))),
            Some(HeldLog::Holding),
            "same count, new reason"
        );
        assert_eq!(
            log(&r7, &held(Some(2))),
            Some(HeldLog::Holding),
            "same count, R3 lifted and R7 unmet"
        );
        assert_eq!(log(&r7, &held(Some(3))), Some(HeldLog::Holding));
        assert_eq!(log(&r7, &held(Some(0))), Some(HeldLog::Released));
        // Nothing held: a change of reasons alone is not news, since the
        // unenforced gate reports would-refuse approvals on their own line.
        assert_eq!(log(&r3, &held(Some(0))), None);
        assert_eq!(log(&r3, &held(Some(1))), Some(HeldLog::Holding));
    }

    fn grant_test_cfg(scopes: &[&str]) -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            inference_receipt_endpoint: None,
            inference_receipt_check_attestation: false,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.invalid".to_string(),
            ingest_url: "https://ingest.invalid".to_string(),
            audience: "aud".to_string(),
            tenant_id: "tenant-1".to_string(),
            instance_id: "instance-1".to_string(),
            user_subject: "alice".to_string(),
            device_key_id: "sha256:aa".to_string(),
            consent_scopes: scopes.iter().map(|s| s.to_string()).collect(),
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        }
    }

    fn the_only_project(f: &WatcherFixture) -> crate::daemon::policy::ProjectEntry {
        let policy = f.shared.policy.lock().unwrap();
        assert_eq!(policy.projects.len(), 1);
        policy.projects.values().next().unwrap().clone()
    }

    /// R6, through the watcher. A grant is baselined on the first pass, then
    /// a change that widens what leaves -- here the scopes gaining
    /// `model_training` -- voids it: the project asks first, a new session is
    /// not approved on the contributor's behalf, and the void is recorded.
    #[tokio::test]
    async fn widening_the_terms_voids_the_grant_and_stops_new_approvals() {
        let f = WatcherFixture::new();
        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation"]))
            .unwrap();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);

        let first = f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        assert_eq!(first.auto_ready, 1, "{first:?}");
        assert!(the_only_project(&f).armed_under.is_some(), "baselined");

        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation", "model_training"]))
            .unwrap();
        f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        let second = f.settle(Utc::now() + chrono::Duration::hours(31)).await;

        assert_eq!(second.auto_ready, 0, "{second:?}");
        let project = the_only_project(&f);
        assert_eq!(
            project.mode,
            ProjectMode::NotifyOnly,
            "the project asks first"
        );
        assert!(project.armed_under.is_none());
        let audit = crate::daemon::audit::load(&f.shared.store).unwrap();
        let voided = audit
            .iter()
            .find(|e| e.action == "auto-upload-voided")
            .expect("the void is recorded");
        assert_eq!(voided.detail.as_deref(), Some("scopes-widened"));

        // And the contributor is told, not only the audit: the notice is on
        // `status` for every shell, and survives a restart.
        let status = f.shared.status_value();
        let voids = status["grant_voids"].as_array().expect("grant_voids");
        assert_eq!(voids.len(), 1, "{voids:?}");
        assert_eq!(voids[0]["kind"], "project");
        assert_eq!(voids[0]["reasons"], serde_json::json!(["scopes-widened"]));
        let persisted = crate::daemon::policy::ProjectPolicy::load(&f.shared.store).unwrap();
        assert_eq!(persisted.grant_voids.len(), 1, "saved with the void");
    }

    /// Arm and baseline a project under `cfg`, apply `widen`, run a pass,
    /// and require that the grant was voided with `label`.
    async fn assert_widening_voids(
        cfg: crate::config::ContributorConfig,
        widen: impl FnOnce(&WatcherFixture),
        label: &str,
    ) {
        let f = WatcherFixture::new();
        f.shared.store.save_config(&cfg).unwrap();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        assert!(the_only_project(&f).armed_under.is_some(), "baselined");

        widen(&f);
        f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        f.settle(Utc::now() + chrono::Duration::hours(31)).await;

        let project = the_only_project(&f);
        assert_eq!(project.mode, ProjectMode::NotifyOnly, "{label}");
        assert!(project.armed_under.is_none(), "{label}");
        let audit = crate::daemon::audit::load(&f.shared.store).unwrap();
        let voided = audit
            .iter()
            .find(|e| e.action == "auto-upload-voided")
            .unwrap_or_else(|| panic!("{label}: the void is recorded"));
        assert_eq!(voided.detail.as_deref(), Some(label));
    }

    /// Turning attested bodies on sends trace bodies to a party that did not
    /// see them when the project was armed.
    #[tokio::test]
    async fn turning_attested_bodies_on_voids_the_grant() {
        assert_widening_voids(
            grant_test_cfg(&["debugging_evaluation"]),
            |f| f.shared.settings.lock().unwrap().ironwire_attested_bodies = true,
            crate::daemon::grant_terms::VOID_ATTESTED_BODIES,
        )
        .await;
    }

    /// Pointing the client at a different witness changes who vouches for
    /// what leaves.
    #[tokio::test]
    async fn changing_the_witness_voids_the_grant() {
        let witness = |url: &str| crate::config::WitnessSettings {
            admission_evidence: false,
            url: url.to_string(),
            signing_address: "0x0000000000000000000000000000000000000001".to_string(),
            expected_measurements: Vec::new(),
        };
        let mut cfg = grant_test_cfg(&["debugging_evaluation"]);
        cfg.witness = Some(witness("https://witness-a.invalid"));
        let mut moved = cfg.clone();
        moved.witness = Some(witness("https://witness-b.invalid"));
        assert_widening_voids(
            cfg,
            move |f| f.shared.store.save_config(&moved).unwrap(),
            crate::daemon::grant_terms::VOID_WITNESS,
        )
        .await;
    }

    /// Narrowing is covered by the grant: removing a scope keeps the project
    /// armed and approving.
    #[tokio::test]
    async fn narrowing_the_terms_keeps_the_grant() {
        let f = WatcherFixture::new();
        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation", "benchmark_only"]))
            .unwrap();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;

        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation"]))
            .unwrap();
        f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        let second = f.settle(Utc::now() + chrono::Duration::hours(31)).await;

        assert_eq!(second.auto_ready, 1, "{second:?}");
        assert_eq!(the_only_project(&f).mode, ProjectMode::AutoUpload);
    }

    fn grant_automatic(f: &WatcherFixture) {
        let req = super::super::ipc::Request {
            id: 1,
            method: "grant_automatic".to_string(),
            params: serde_json::json!({}),
        };
        let resp = super::super::ipc::handle_request(&f.shared, &req);
        assert!(resp.error.is_none(), "{:?}", resp.error);
    }

    fn mode_of(f: &WatcherFixture, project: &str) -> (ProjectMode, bool) {
        let key = project_key_for(Some(&abs(&format!("Users/testuser/code/{project}"))));
        let policy = f.shared.policy.lock().unwrap();
        (policy.resolve(&key), policy.projects.contains_key(&key))
    }

    /// K3: under the Flow 1 grant a project first seen after it is armed,
    /// with an explicit policy entry, the terms it is granted under, and an
    /// audit row. A project already on disk at the grant keeps asking, and
    /// so does its new session.
    #[tokio::test]
    async fn the_grant_arms_projects_discovered_after_it_and_nothing_already_on_disk() {
        let f = WatcherFixture::new();
        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation"]))
            .unwrap();
        f.write_session("old", "11111111-1111-1111-1111-111111111111", 0);
        grant_automatic(&f);

        // The first full pass records what is on disk and arms nothing.
        let first = f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        assert_eq!(first.auto_ready, 0, "{first:?}");
        assert_eq!(mode_of(&f, "old"), (ProjectMode::NotifyOnly, false));

        f.write_session("old", "22222222-2222-2222-2222-222222222222", 0);
        f.write_session("new", "33333333-3333-3333-3333-333333333333", 0);
        let second = f.settle(Utc::now() + chrono::Duration::hours(31)).await;

        assert_eq!(mode_of(&f, "new"), (ProjectMode::AutoUpload, true));
        assert_eq!(
            mode_of(&f, "old"),
            (ProjectMode::NotifyOnly, false),
            "a project on disk at the grant asks, for its new sessions too"
        );
        assert_eq!(second.auto_ready, 1, "{second:?}");
        let key = project_key_for(Some(&abs("Users/testuser/code/new")));
        assert!(
            f.shared.policy.lock().unwrap().projects[&key]
                .armed_under
                .is_some(),
            "armed under recorded terms"
        );
        let audit = crate::daemon::audit::load(&f.shared.store).unwrap();
        assert!(audit.iter().any(|e| e.action == "automatic-granted"));
        let armed: Vec<_> = audit
            .iter()
            .filter(|e| e.action == "armed-by-default")
            .collect();
        assert_eq!(armed.len(), 1);
        assert_eq!(armed[0].project_label.as_deref(), Some("new"));
    }

    /// A project the grant arms by default is still an unattended approval,
    /// so it goes through the gate: enforced, the new project is armed (the
    /// grant is the contributor's), but its session waits and is counted as
    /// held. The project on disk at the grant asks, and is not counted.
    #[tokio::test]
    async fn the_enforced_gate_holds_a_session_in_a_project_the_grant_armed() {
        ENFORCE_GATE_FOR_TEST.with(|c| c.set(true));
        let f = WatcherFixture::new();
        f.shared.store.save_config(&grant_test_cfg(&[])).unwrap();
        f.write_session("old", "11111111-1111-1111-1111-111111111111", 0);
        grant_automatic(&f);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        f.write_session("old", "22222222-2222-2222-2222-222222222222", 0);
        f.write_session("new", "33333333-3333-3333-3333-333333333333", 0);
        let second = f.settle(Utc::now() + chrono::Duration::hours(31)).await;
        let third = f.settle(Utc::now() + chrono::Duration::hours(32)).await;
        ENFORCE_GATE_FOR_TEST.with(|c| c.set(false));

        assert_eq!(mode_of(&f, "new"), (ProjectMode::AutoUpload, true));
        assert_eq!(second.auto_ready, 0, "{second:?}");
        assert_eq!(second.gate_blocked, Some(1), "{second:?}");
        assert_eq!(third.gate_blocked, Some(1), "{third:?}");
        let queue = f.shared.queue.lock().unwrap();
        assert!(
            queue
                .all()
                .iter()
                .all(|e| e.state == QueueState::Pending && !e.approved_unattended),
            "nothing approved on the contributor's behalf"
        );
    }

    /// K4: a re-grant after logout arms nothing already on disk. Logout
    /// wipes the policy, so a folder set to Never loses that decision; the
    /// re-grant must still count it as on disk and ask, for its new
    /// sessions as well, rather than treat it as newly discovered.
    #[tokio::test]
    async fn a_re_grant_after_logout_arms_nothing_already_on_disk() {
        let f = WatcherFixture::new();
        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation"]))
            .unwrap();
        f.write_session("never", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("never", ProjectMode::Ignore);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;

        // What logout leaves of the policy: nothing.
        *f.shared.policy.lock().unwrap() = crate::daemon::policy::ProjectPolicy::new();
        grant_automatic(&f);
        f.settle(Utc::now() + chrono::Duration::hours(31)).await;
        f.write_session("never", "22222222-2222-2222-2222-222222222222", 0);
        let after = f.settle(Utc::now() + chrono::Duration::hours(32)).await;

        assert_eq!(after.auto_ready, 0, "{after:?}");
        assert_eq!(mode_of(&f, "never"), (ProjectMode::NotifyOnly, false));
    }

    /// A harness connected after the grant -- the normal onboarding order --
    /// is recorded on its first discovery, so its history is on disk rather
    /// than new: nothing in it is armed or sent. (Zaki's reproduction on
    /// #1031.)
    #[tokio::test]
    async fn a_harness_connected_after_the_grant_arms_nothing_already_on_disk() {
        let f = WatcherFixture::new();
        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation"]))
            .unwrap();
        f.write_session("old", "11111111-1111-1111-1111-111111111111", 0);
        let watch = f.shared.settings.lock().unwrap().claude_source.clone();
        f.shared.settings.lock().unwrap().claude_source =
            Some(crate::daemon::settings::SourceDeclaration::Off);
        grant_automatic(&f);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;
        f.shared.settings.lock().unwrap().claude_source = watch; // connect the harness
        let after = f.settle(Utc::now() + chrono::Duration::hours(31)).await;
        assert_eq!(
            (after.auto_ready, mode_of(&f, "old")),
            (0, (ProjectMode::NotifyOnly, false)),
            "{after:?}"
        );

        // And a project genuinely new since then is still armed.
        f.write_session("new", "22222222-2222-2222-2222-222222222222", 0);
        f.settle(Utc::now() + chrono::Duration::hours(32)).await;
        assert_eq!(mode_of(&f, "new"), (ProjectMode::AutoUpload, true));
    }

    /// A session on disk at the grant is not approved unattended when it
    /// comes to read as a project the grant armed later. (Zaki's second
    /// reproduction on #1031: the session's first cwd changes.)
    #[tokio::test]
    async fn a_pre_grant_session_is_not_sent_under_a_project_the_grant_armed() {
        let f = WatcherFixture::new();
        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation"]))
            .unwrap();
        let pre = f.write_session("old", "11111111-1111-1111-1111-111111111111", 0);
        grant_automatic(&f);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;

        f.write_session_with_cwd(
            "-Users-testuser-code-old",
            &abs("Users/testuser/code/new"),
            "11111111-1111-1111-1111-111111111111",
        );
        f.write_session("new", "33333333-3333-3333-3333-333333333333", 0);
        for h in 31..35 {
            f.settle(Utc::now() + chrono::Duration::hours(h)).await;
        }

        assert_eq!(mode_of(&f, "new"), (ProjectMode::AutoUpload, true));
        let queue = f.shared.queue.lock().unwrap();
        for e in queue.all().iter().filter(|e| e.path == pre) {
            assert_ne!(
                e.state,
                QueueState::Approved,
                "a session on disk at the grant was approved unattended under {}",
                e.project_key
            );
        }
        assert!(
            queue
                .all()
                .iter()
                .any(|e| e.path != pre && e.state == QueueState::Approved),
            "the new session is approved"
        );
    }

    /// A pass records each source under the root it discovered, not under
    /// whatever root the settings name by the time the pass reads them. A
    /// harness re-rooted between the two reads must not have root A's listing
    /// recorded as root B's: root B's history would then read as already
    /// recorded, and its pre-grant sessions as new.
    #[tokio::test]
    async fn a_root_changed_mid_pass_is_not_recorded_under_the_new_root() {
        let mut f = WatcherFixture::new();
        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation"]))
            .unwrap();
        f.write_session("a-old", "11111111-1111-1111-1111-111111111111", 0);
        let root_a = f.claude_root.clone();
        let root_b = f._dir.path().join("projects-b");
        f.claude_root = root_b.clone();
        let pre_b = f.write_session("b-old", "22222222-2222-2222-2222-222222222222", 0);
        f.claude_root = root_a;
        grant_automatic(&f);
        f.settle(Utc::now() + chrono::Duration::hours(30)).await;

        // The pass's sources come from one read of the settings; the harness
        // is re-rooted before the pass reads them again.
        let (max_queue_entries, roots_a) = {
            let s = f.shared.settings.lock().unwrap();
            (s.max_queue_entries, s.source_roots(&f.shared.store))
        };
        f.shared.settings.lock().unwrap().claude_source =
            Some(crate::daemon::settings::SourceDeclaration::Watch {
                path: root_b.clone(),
            });
        tick_over(
            &f.shared,
            Utc::now() + chrono::Duration::hours(31),
            all_sources(&roots_a),
            roots_a.source_identities(),
            max_queue_entries,
        )
        .unwrap();
        for h in 32..36 {
            f.settle(Utc::now() + chrono::Duration::hours(h)).await;
        }

        assert_eq!(
            mode_of(&f, "b-old"),
            (ProjectMode::NotifyOnly, false),
            "root B's pre-grant project was armed"
        );
        let queue = f.shared.queue.lock().unwrap();
        for e in queue.all().iter().filter(|e| e.path == pre_b) {
            assert_ne!(
                e.state,
                QueueState::Approved,
                "a session on disk at the grant under root B was approved unattended"
            );
        }
    }

    /// Only a full pass records what is on disk; before it, the grant arms
    /// nothing, however the session arrives.
    #[tokio::test]
    async fn the_grant_arms_nothing_before_a_full_pass_has_recorded_the_disk() {
        let f = WatcherFixture::new();
        f.shared
            .store
            .save_config(&grant_test_cfg(&["debugging_evaluation"]))
            .unwrap();
        grant_automatic(&f);
        assert!(
            f.shared
                .policy
                .lock()
                .unwrap()
                .automatic_grant
                .as_ref()
                .unwrap()
                .recorded_sources
                .is_empty(),
            "a scoped pass records nothing"
        );
        let path = f.write_session("fresh", "11111111-1111-1111-1111-111111111111", 0);
        let session_at: SessionAt<'_> =
            &|source, p| source.discover().ok()?.into_iter().find(|r| r.path == p);
        let now = Utc::now() + chrono::Duration::hours(30);
        tick_paths(&f.shared, now, std::slice::from_ref(&path), session_at)
            .await
            .unwrap();
        tick_paths(&f.shared, now, &[path], session_at)
            .await
            .unwrap();
        assert_eq!(mode_of(&f, "fresh"), (ProjectMode::NotifyOnly, false));
        assert!(
            f.shared
                .policy
                .lock()
                .unwrap()
                .automatic_grant
                .as_ref()
                .unwrap()
                .recorded_sources
                .is_empty(),
            "a scoped pass records nothing"
        );
    }

    #[tokio::test]
    async fn repeated_ticks_do_not_duplicate_an_entry() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        tick(&f.shared, at("2030-01-01T00:01:00Z")).await.unwrap();
        tick(&f.shared, at("2030-01-01T00:02:00Z")).await.unwrap();
        assert_eq!(f.queue_len(), 1);
    }

    #[tokio::test]
    async fn a_paused_daemon_does_no_work_at_all() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        // A real `pause` call always sets both together; `is_paused` trusts
        // `state.paused` once it has taken the state lock (needed so it can
        // return a non-stale answer to a reader that loses a race against a
        // lapsing timed pause -- see `DaemonShared::is_paused`), so the
        // fixture must keep both in sync too.
        f.shared.paused.store(true, Ordering::Relaxed);
        f.shared.state.lock().unwrap().paused = true;
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report, TickReport::default());
        assert_eq!(f.queue_len(), 0);
    }

    #[tokio::test]
    async fn a_lapsed_timed_pause_resumes_ticking_on_its_own() {
        // An app-side timer dies with the app; the daemon must notice the
        // pause has lapsed itself rather than waiting for an explicit
        // `resume` that might never come.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.shared.paused.store(true, Ordering::Relaxed);
        f.shared.state.lock().unwrap().paused_until = Some(at("2029-12-31T00:00:00Z"));
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 1, "{report:?}");
        assert!(
            !f.shared.paused.load(Ordering::Relaxed),
            "the lapsed pause should have cleared itself"
        );
    }

    #[tokio::test]
    async fn sessions_from_several_projects_are_all_offered() {
        let f = WatcherFixture::new();
        f.write_session("alpha", "11111111-1111-1111-1111-111111111111", 0);
        f.write_session("beta", "22222222-2222-2222-2222-222222222222", 0);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 2, "{report:?}");
    }

    #[tokio::test]
    async fn the_queue_and_state_are_persisted_after_a_tick() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert!(
            f.shared
                .store
                .read_daemon_file(crate::config::DAEMON_STATE_FILE)
                .unwrap()
                .is_some()
        );
        let reloaded = crate::daemon::queue::Queue::load(&f.shared.store).unwrap();
        assert_eq!(reloaded.all().len(), 1);
    }

    #[tokio::test]
    async fn a_queued_entry_records_a_label_and_a_hash() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let queue = f.shared.queue.lock().unwrap();
        let e = &queue.all()[0];
        assert_eq!(e.project_label, "proj");
        assert!(e.session_hash.starts_with("sha256:"));
        assert_eq!(e.source, "claude-code");
        assert!(e.size_bytes > 0);
    }

    #[tokio::test]
    async fn colliding_project_basenames_both_get_suffixed_in_the_same_tick() {
        // Two different repositories both called `api`; one might be the
        // client's. Both must render suffixed once the collision is known --
        // not just whichever one happened to be processed second -- or an
        // operator who has learned "unsuffixed = normal" will misread the
        // bare one as the safe default roughly half the time.
        let f = WatcherFixture::new();
        f.write_session_with_cwd(
            "-Users-testuser-work-api",
            abs("Users/testuser/work/api").as_str(),
            "11111111-1111-1111-1111-111111111111",
        );
        f.write_session_with_cwd(
            "-Users-testuser-client-api",
            abs("Users/testuser/client/api").as_str(),
            "22222222-2222-2222-2222-222222222222",
        );
        f.settle(at("2030-01-01T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 2, "{:?}", queue.all());
        let by_key: std::collections::BTreeMap<String, String> = queue
            .all()
            .iter()
            .map(|e| (e.project_key.clone(), e.project_label.clone()))
            .collect();
        let labels: Vec<&str> = by_key.values().map(String::as_str).collect();
        assert_ne!(
            labels[0], labels[1],
            "colliding projects must render distinct labels: {labels:?}"
        );
        for label in &labels {
            assert!(
                label.starts_with("api ("),
                "expected every colliding member suffixed, got {label}"
            );
            assert!(
                !label.contains("work") && !label.contains("client") && !label.contains('/'),
                "label must not leak a path segment: {label}"
            );
        }
    }

    #[tokio::test]
    async fn a_bare_label_is_relabelled_once_a_collision_appears_in_a_later_tick() {
        // Project queued first, alone, with a unique basename -- correctly
        // bare. A colliding project only shows up afterwards. Because
        // `Queue::upsert` never rewrites an existing entry, only the
        // end-of-tick relabel pass can fix the first entry's now-stale bare
        // label.
        let f = WatcherFixture::new();
        f.write_session_with_cwd(
            "-Users-testuser-work-api",
            abs("Users/testuser/work/api").as_str(),
            "11111111-1111-1111-1111-111111111111",
        );
        f.settle(at("2030-01-01T00:00:00Z")).await;
        {
            let queue = f.shared.queue.lock().unwrap();
            assert_eq!(queue.all().len(), 1);
            assert_eq!(queue.all()[0].project_label, "api");
        }

        // A second, colliding project shows up in a later tick.
        f.write_session_with_cwd(
            "-Users-testuser-client-api",
            abs("Users/testuser/client/api").as_str(),
            "22222222-2222-2222-2222-222222222222",
        );
        f.settle(at("2030-01-01T00:10:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 2, "{:?}", queue.all());
        let first = queue
            .all()
            .iter()
            .find(|e| {
                e.project_key == project_key_for(Some(abs("Users/testuser/work/api").as_str()))
            })
            .unwrap();
        let second = queue
            .all()
            .iter()
            .find(|e| {
                e.project_key == project_key_for(Some(abs("Users/testuser/client/api").as_str()))
            })
            .unwrap();
        assert!(
            first.project_label.starts_with("api ("),
            "the first-queued entry must be relabelled once it collides, got {}",
            first.project_label
        );
        assert!(second.project_label.starts_with("api ("));
        assert_ne!(first.project_label, second.project_label);
    }

    #[tokio::test]
    async fn a_unique_basename_stays_bare_and_is_untouched_by_the_relabel_pass() {
        let f = WatcherFixture::new();
        f.write_session("solo", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        tick(&f.shared, at("2030-01-01T00:03:00Z")).await.unwrap();
        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 1);
        assert_eq!(queue.all()[0].project_label, "solo");
    }

    #[tokio::test]
    async fn list_projects_and_the_queue_render_the_same_label_when_a_collision_exists() {
        let f = WatcherFixture::new();
        f.write_session_with_cwd(
            "-Users-testuser-work-api",
            abs("Users/testuser/work/api").as_str(),
            "11111111-1111-1111-1111-111111111111",
        );
        f.write_session_with_cwd(
            "-Users-testuser-client-api",
            abs("Users/testuser/client/api").as_str(),
            "22222222-2222-2222-2222-222222222222",
        );
        f.settle(at("2030-01-01T00:00:00Z")).await;

        // Configure both projects in policy too (with distinct modes so the
        // `list_projects` rows can be told apart, since that surface
        // deliberately never echoes the project key).
        f.set_mode_for_key(
            abs("Users/testuser/work/api").as_str(),
            ProjectMode::NotifyOnly,
        );
        f.set_mode_for_key(
            abs("Users/testuser/client/api").as_str(),
            ProjectMode::Ignore,
        );

        let resp = crate::daemon::ipc::handle_request(
            &f.shared,
            &crate::daemon::ipc::Request {
                id: 1,
                method: "list_projects".to_string(),
                params: serde_json::json!({}),
            },
        );
        let projects = resp.result.unwrap()["projects"].clone();
        let work_row = projects
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["mode"] == serde_json::json!("notify_only"))
            .expect("work/api row");
        let list_label = work_row["project_label"].as_str().unwrap();

        let queue = f.shared.queue.lock().unwrap();
        let queue_entry = queue
            .all()
            .iter()
            .find(|e| {
                e.project_key == project_key_for(Some(abs("Users/testuser/work/api").as_str()))
            })
            .unwrap();
        assert_eq!(
            list_label, queue_entry.project_label,
            "the same project key must render identically on both surfaces"
        );
        assert!(
            list_label.starts_with("api ("),
            "expected a collision suffix, got {list_label}"
        );
    }

    #[tokio::test]
    async fn a_session_and_its_subagents_are_offered_as_one_card() {
        // The whole point: 911 files describing 69 conversations became 911
        // cards. One conversation is one decision.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.write_subagent("proj", session, "agent-a");
        f.write_subagent("proj", session, "agent-b");
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 1, "{report:?}");
        assert_eq!(f.queue_len(), 1);
        let queue = f.shared.queue.lock().unwrap();
        let e = &queue.all()[0];
        assert_eq!(e.subagent_count, 2, "the card must state its own extent");
        assert_eq!(e.subagents_dropped, 0);
        assert!(e.path.ends_with(format!("{session}.jsonl")), "{:?}", e.path);
    }

    #[tokio::test]
    async fn a_subagent_still_being_written_holds_the_whole_group_back() {
        // The trap this change exists to avoid. `Observation` used to come
        // from a re-stat of the parent file, so a subagent appearing or
        // growing was invisible: the daemon would call a conversation
        // finished while a delegate was mid-write. The parent here is
        // deliberately old; only the member is fresh.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.write_subagent("proj", session, "agent-a");
        let report = f.settle(Utc::now()).await;
        assert_eq!(
            report.queued, 0,
            "a group with a live delegate must not be offered: {report:?}"
        );
    }

    #[tokio::test]
    async fn a_new_subagent_supersedes_the_offer_it_invalidates() {
        // Membership is part of the description a contributor consents to.
        // When it moves, the old offer dies and a fresh one is made -- one
        // card, not one card per delegation.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let first_hash = {
            let queue = f.shared.queue.lock().unwrap();
            assert_eq!(queue.all().len(), 1);
            queue.all()[0].session_hash.clone()
        };

        f.write_subagent("proj", session, "agent-a");
        f.settle(at("2030-01-02T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 2, "{:?}", queue.all());
        let old = queue
            .all()
            .iter()
            .find(|e| e.session_hash == first_hash)
            .unwrap();
        assert_eq!(old.state, QueueState::Superseded);
        assert_eq!(
            old.reason_label.as_deref(),
            Some(crate::daemon::queue::REASON_CHANGED)
        );
        assert_eq!(queue.pending().len(), 1, "exactly one live offer");
        let fresh = queue.pending()[0];
        assert_ne!(fresh.session_hash, first_hash, "the hash must have moved");
        assert_eq!(fresh.subagent_count, 1);
    }

    #[tokio::test]
    async fn a_new_subagent_releases_the_preview_the_old_offer_was_pinned_to() {
        // The end of the same story: the artifact a contributor was shown is
        // stored on disk and pinned to the entry that was offered. Once that
        // offer is superseded the stored bytes describe a conversation
        // nobody will now be asked about, so the sweep must delete them --
        // an entry the contributor never resolved must not leave redacted
        // trace content lying around.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;

        let entry_id = {
            let queue = f.shared.queue.lock().unwrap();
            queue.all()[0].entry_id
        };
        // Exactly what `preview` does: build the envelope, store it, pin the
        // entry to it.
        let src = crate::source::claude_code::ClaudeCodeSource::new(f.claude_root.clone());
        let session_ref = src.discover().unwrap().remove(0);
        let (summary, _body, envelope) =
            crate::daemon::preview::build_preview(&f.shared.store, None, None, &src, &session_ref)
                .await
                .unwrap();
        crate::daemon::approved_envelope::save(&f.shared.store, entry_id, &envelope).unwrap();
        {
            let mut queue = f.shared.queue.lock().unwrap();
            assert!(queue.record_previewed_envelope(entry_id, &summary.envelope_digest, None));
        }
        assert!(
            crate::daemon::approved_envelope::load(&f.shared.store, entry_id)
                .unwrap()
                .is_some(),
            "the shown artifact should be on disk before the change"
        );

        f.write_subagent("proj", session, "agent-a");
        f.settle(at("2030-01-02T00:00:00Z")).await;

        {
            let queue = f.shared.queue.lock().unwrap();
            assert_eq!(queue.get(entry_id).unwrap().state, QueueState::Superseded);
        }
        assert!(
            crate::daemon::approved_envelope::load(&f.shared.store, entry_id)
                .unwrap()
                .is_none(),
            "the superseded offer's stored preview must be swept"
        );
    }

    #[tokio::test]
    async fn an_approved_group_is_superseded_when_a_subagent_lands() {
        // The same rule, one state later: an approval covers a description,
        // and a delegate arriving after it is a different description. The
        // approval must not carry over.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.states(), vec![QueueState::Approved]);

        f.write_subagent("proj", session, "agent-a");
        f.settle(at("2030-01-02T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        let superseded = queue
            .all()
            .iter()
            .filter(|e| e.state == QueueState::Superseded)
            .count();
        assert_eq!(superseded, 1, "{:?}", queue.all());
    }

    #[tokio::test]
    async fn an_unqueued_eligible_session_is_loaded_exactly_once() {
        // The baseline the skip is measured against: settling a fresh
        // session costs one load -- nothing on the first sighting (still
        // `Unstable`), one on the poll that actually offers it. Two would
        // mean `resolve_cwd` is reading the file as well as `load`.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let c = loads();
        let report = f.settle_counted(at("2030-01-01T00:00:00Z"), &c);
        assert_eq!(report.queued, 1, "{report:?}");
        assert_eq!(
            count(&c),
            1,
            "settling one session must cost exactly one load"
        );
    }

    #[tokio::test]
    async fn a_queued_session_that_has_not_moved_is_never_loaded_again() {
        // The bug. `eligibility::evaluate` only knows about prior *uploads*,
        // so a session sitting `Pending` came back `Eligible` on every poll
        // and the pass read, parsed and hashed the whole group again --
        // 11 GB of transcripts a minute on the machine that reported this --
        // only for `replace_live_at_path` to find it already tracked.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1);

        let c = loads();
        for minute in 1..=5 {
            f.tick_counted(at(&format!("2030-01-01T00:0{minute}:00Z")), &c);
        }
        assert_eq!(
            count(&c),
            0,
            "five polls over an unchanged queued session must load nothing"
        );
        assert_eq!(f.queue_len(), 1, "and must not disturb the queue");
        assert_eq!(f.states(), vec![QueueState::Pending]);
    }

    #[tokio::test]
    async fn a_corpus_of_queued_sessions_costs_nothing_on_a_later_poll() {
        // The property that actually matters at scale: the reported machine
        // had 498 entries in the queue and re-hashed the lot every sixty
        // seconds. N queued sessions, zero loads on the next poll.
        let f = WatcherFixture::new();
        for i in 0..30u32 {
            f.write_session(
                "proj",
                &format!("1111111{i:02}-1111-1111-1111-111111111111"),
                0,
            );
        }
        let first = loads();
        let report = f.settle_counted(at("2030-01-01T00:00:00Z"), &first);
        assert_eq!(report.queued, 30, "{report:?}");
        assert_eq!(
            count(&first),
            30,
            "each session is loaded once, when it is offered"
        );

        let later = loads();
        f.tick_counted(at("2030-01-01T00:01:00Z"), &later);
        assert_eq!(
            count(&later),
            0,
            "a poll over 30 unchanged queued sessions must load nothing"
        );
        assert_eq!(f.queue_len(), 30);
    }

    #[tokio::test]
    async fn a_dismissed_session_is_not_re_offered_after_it_grows() {
        // "Not this one" is a decision about the conversation, not about the
        // byte range it happened to have when the card was drawn. The
        // dismissal used to be recorded against the content hash alone: the
        // next few keystrokes in the same session produced a new hash, the
        // watcher had nothing telling it the contributor had already said
        // no, and `replace_live_at_path` pushed a fresh `Pending` card for
        // the very session that had just been declined.
        let f = WatcherFixture::new();
        let name = "11111111-1111-1111-1111-111111111111";
        let path = f.write_session("proj", name, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.states(), vec![QueueState::Pending]);
        let entry_id = f.shared.queue.lock().unwrap().pending()[0].entry_id;

        let r = crate::daemon::ipc::handle_request(
            &f.shared,
            &serde_json::from_value(serde_json::json!({
                "id": 1,
                "method": "dismiss",
                "params": { "entry_id": entry_id.to_string() },
            }))
            .unwrap(),
        );
        assert!(r.error.is_none(), "{r:?}");
        assert!(
            f.shared.queue.lock().unwrap().pending().is_empty(),
            "the dismissal must take the card off the pending list at once"
        );

        f.append_to_session(&path, "proj", name);
        f.settle(at("2030-01-02T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert!(
            !queue
                .all()
                .iter()
                .any(|e| matches!(e.state, QueueState::Pending | QueueState::Approved)),
            "a declined session must not come back the moment it grows: {:?}",
            queue.all()
        );
    }

    #[tokio::test]
    async fn a_dismissed_session_is_not_re_offered_by_a_standing_opt_in() {
        // The same decision, in an armed project. Arming a project is a
        // standing "yes" to sessions the contributor has not ruled on; it is
        // not an override of one they have explicitly declined. So the
        // re-offer must not come back as `Approved` either -- that one needs
        // no second click and the uploader would send it unattended.
        let f = WatcherFixture::new();
        let name = "11111111-1111-1111-1111-111111111111";
        let path = f.write_session("proj", name, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let entry_id = f.shared.queue.lock().unwrap().pending()[0].entry_id;
        {
            let mut queue = f.shared.queue.lock().unwrap();
            queue.set_state(
                entry_id,
                QueueState::Refused,
                Some(crate::daemon::queue::REASON_DISMISSED.to_string()),
            );
        }

        f.set_mode("proj", ProjectMode::AutoUpload);
        f.append_to_session(&path, "proj", name);
        f.settle(at("2030-01-02T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert!(
            !queue
                .all()
                .iter()
                .any(|e| matches!(e.state, QueueState::Pending | QueueState::Approved)),
            "{:?}",
            queue.all()
        );
    }

    #[tokio::test]
    async fn ignoring_a_project_clears_what_is_already_waiting() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.shared.queue.lock().unwrap().pending().len(), 1);

        f.set_mode_via_ipc("proj", ProjectMode::Ignore);

        let queue = f.shared.queue.lock().unwrap();
        assert!(queue.pending().is_empty(), "{:?}", queue.all());
        assert_eq!(
            queue.all()[0].reason_label.as_deref(),
            Some(crate::daemon::queue::REASON_PROJECT_IGNORED)
        );
    }

    #[tokio::test]
    async fn un_ignoring_a_project_lets_its_sessions_be_offered_again() {
        // The confirmation copy promises this is undoable in Settings. Without
        // this test that promise is unverified.
        let f = WatcherFixture::new();
        let path = f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        f.set_mode_via_ipc("proj", ProjectMode::Ignore);
        assert!(f.shared.queue.lock().unwrap().pending().is_empty());

        f.set_mode_via_ipc("proj", ProjectMode::NotifyOnly);
        f.append_to_session(&path, "proj", "22222222-2222-2222-2222-222222222222");
        f.settle(at("2030-01-02T00:00:00Z")).await;

        assert!(
            !f.shared.queue.lock().unwrap().pending().is_empty(),
            "un-ignoring must let the watcher offer the project again"
        );
    }

    #[tokio::test]
    async fn un_ignoring_a_project_re_offers_a_session_that_never_changes() {
        // The ordinary case, and the one the confirmation copy is actually
        // about: the sessions cleared by an ignore are *finished*. The work
        // is done, the files will never be written to again. If undo only
        // works for a session that happens to grow afterwards, "You can undo
        // this in Settings" is false for every trace it was written for.
        let f = WatcherFixture::new();
        f.write_session("proj", "33333333-3333-3333-3333-333333333333", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.shared.queue.lock().unwrap().pending().len(), 1);

        f.set_mode_via_ipc("proj", ProjectMode::Ignore);
        assert!(f.shared.queue.lock().unwrap().pending().is_empty());

        f.set_mode_via_ipc("proj", ProjectMode::NotifyOnly);
        // Deliberately no `append_to_session`: the file is untouched from
        // here on, exactly as a finished session would be.
        f.settle(at("2030-01-02T00:00:00Z")).await;

        assert!(
            !f.shared.queue.lock().unwrap().pending().is_empty(),
            "un-ignoring must re-offer a session that never changes again: {:?}",
            f.shared.queue.lock().unwrap().all()
        );
    }

    #[tokio::test]
    async fn a_dismissed_session_costs_no_further_reads() {
        // The skip belongs in front of `source.load`, not after it. A
        // conversation the contributor declined and then kept working in
        // would otherwise be read, parsed and group-hashed on every poll
        // for the rest of its life, for a result the queue throws away.
        let f = WatcherFixture::new();
        let name = "11111111-1111-1111-1111-111111111111";
        let path = f.write_session("proj", name, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let entry_id = f.shared.queue.lock().unwrap().pending()[0].entry_id;
        {
            let mut queue = f.shared.queue.lock().unwrap();
            queue.set_state(
                entry_id,
                QueueState::Refused,
                Some(crate::daemon::queue::REASON_DISMISSED.to_string()),
            );
        }

        f.append_to_session(&path, "proj", name);
        let c = loads();
        f.settle_counted(at("2030-01-02T00:00:00Z"), &c);
        assert_eq!(count(&c), 0, "a declined session must not be re-read");
    }

    /// Enrol the fixture, so `approve` has a config to build an envelope
    /// against. Without one it skips every entry `not-enrolled` and never
    /// reaches the size check at all.
    fn enrol(f: &WatcherFixture) {
        let device = crate::identity::DeviceIdentity::load_or_generate(&f.shared.store).unwrap();
        let cfg = crate::config::ContributorConfig {
            inference_receipt_endpoint: None,
            inference_receipt_check_attestation: false,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        };
        f.shared.store.save_config(&cfg).unwrap();
    }

    /// One event carrying more than `MAX_ENVELOPE_BYTES`, so the envelope
    /// `approve` builds cannot be stored and the entry can never be
    /// approved.
    fn write_oversized_session(f: &WatcherFixture, project: &str, name: &str) -> PathBuf {
        let project_dir = f
            .claude_root
            .join(format!("-Users-testuser-code-{project}"));
        std::fs::create_dir_all(&project_dir).unwrap();
        let path = project_dir.join(format!("{name}.jsonl"));
        let content = "x".repeat(crate::envelope::MAX_ENVELOPE_BYTES + 1);
        let event = serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": content},
            "cwd": abs(&format!("Users/testuser/code/{project}")),
            "timestamp": "2026-08-08T10:00:00Z",
            "version": "2.0.1",
            "sessionId": name,
            "uuid": "a1",
        });
        std::fs::write(&path, format!("{event}\n")).unwrap();
        path
    }

    async fn approve_entry(f: &WatcherFixture, entry_id: uuid::Uuid) -> serde_json::Value {
        let r = crate::daemon::ipc::handle_request_async(
            &f.shared,
            &serde_json::from_value(serde_json::json!({
                "id": 1,
                "method": "approve",
                "params": { "entry_id": entry_id.to_string() },
            }))
            .unwrap(),
        )
        .await;
        assert!(r.error.is_none(), "{r:?}");
        r.result.expect("approve result")
    }

    #[tokio::test]
    async fn an_oversized_session_is_refused_once_and_not_offered_again() {
        // `approve` recognises an envelope past `MAX_ENVELOPE_BYTES` and
        // reports it `skipped`, but used to persist nothing: the entry
        // stayed `Pending`, so the card sat in the queue for a session no
        // click could ever get past the size guard, and every later poll
        // found it still there. The refusal has to be written down, on the
        // entry, for the same reason every other decision is.
        let f = WatcherFixture::new();
        enrol(&f);
        let name = "11111111-1111-1111-1111-111111111111";
        write_oversized_session(&f, "proj", name);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.states(), vec![QueueState::Pending]);
        let entry_id = f.shared.queue.lock().unwrap().pending()[0].entry_id;

        let v = approve_entry(&f, entry_id).await;
        assert_eq!(v["approved"].as_u64(), Some(0), "{v}");
        let skipped = v["skipped"].as_array().expect("skipped");
        assert_eq!(skipped.len(), 1, "{v}");
        assert_eq!(
            skipped[0]["reason_label"].as_str(),
            Some(crate::daemon::queue::REASON_TOO_LARGE),
            "the wire label the shells already translate must not change: {v}"
        );

        assert!(
            f.shared.queue.lock().unwrap().pending().is_empty(),
            "a refusal nothing can retry must take the card off the pending list"
        );

        // And it must stay off it. This is the half the response alone
        // could not deliver: the watcher re-observes the same quiescent
        // session on every poll, and an entry left `Pending` is an offer
        // that comes back forever.
        f.settle(at("2030-01-02T00:00:00Z")).await;
        let queue = f.shared.queue.lock().unwrap();
        assert!(
            !queue
                .all()
                .iter()
                .any(|e| matches!(e.state, QueueState::Pending | QueueState::Approved)),
            "an oversized session must not be re-offered: {:?}",
            queue.all()
        );
        let refused = queue
            .all()
            .iter()
            .find(|e| e.state == QueueState::Refused)
            .expect("the refusal is recorded on the entry");
        assert_eq!(
            refused.reason_label.as_deref(),
            Some(crate::daemon::queue::REASON_TOO_LARGE)
        );
    }

    #[tokio::test]
    async fn an_oversized_refusal_is_not_a_dismissal_and_a_grown_session_is_offered_again() {
        // The deliberate difference from `dismiss`. A dismissal is the
        // contributor's decision about a conversation, so it suppresses the
        // path forever; this is the pipeline's verdict on one set of bytes
        // under one set of consent scopes, and those bytes are not the last
        // word -- narrower scopes can produce a smaller envelope from the
        // same conversation. So it is recorded on the entry, like every
        // other pipeline refusal, and a session that has moved on is still
        // offered.
        let f = WatcherFixture::new();
        enrol(&f);
        let name = "11111111-1111-1111-1111-111111111111";
        let path = write_oversized_session(&f, "proj", name);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let entry_id = f.shared.queue.lock().unwrap().pending()[0].entry_id;
        approve_entry(&f, entry_id).await;
        assert!(f.shared.queue.lock().unwrap().pending().is_empty());

        f.append_to_session(&path, "proj", name);
        f.settle(at("2030-01-03T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(
            queue.pending().len(),
            1,
            "an oversized refusal must not condemn the conversation: {:?}",
            queue.all()
        );
    }

    /// A real source whose `load` always fails, so a test can choose which
    /// *kind* of failure the pass sees.
    ///
    /// Wrapping the genuine adapter rather than faking one keeps discovery,
    /// grouping and eligibility exactly as they are in production: the only
    /// thing that differs is the one call this is about.
    struct RefusingSource {
        inner: Box<dyn TraceSource>,
        err: fn() -> anyhow::Error,
    }

    impl TraceSource for RefusingSource {
        fn name(&self) -> &'static str {
            self.inner.name()
        }
        fn discover(&self) -> Result<Vec<SessionRef>> {
            self.inner.discover()
        }
        fn load(&self, _r: &SessionRef) -> Result<crate::source::SessionTranscript> {
            Err((self.err)())
        }
    }

    /// The refusal `source::codex` raises for a rollout past its byte
    /// budget, built here rather than by writing a 64 MB file: this is a
    /// test about what the *watcher* does with the refusal, and the refusal
    /// itself is pinned in `source::codex`'s own suite.
    fn too_large() -> anyhow::Error {
        crate::source::SessionTooLarge {
            label: "rollout-too-large",
            declared_bytes: 70_000_000,
            budget_bytes: 64_000_000,
        }
        .into()
    }

    /// A read that failed because of what the machine was doing at that
    /// instant, not because of what the session is.
    fn transient_io() -> anyhow::Error {
        std::io::Error::from(std::io::ErrorKind::PermissionDenied).into()
    }

    impl WatcherFixture {
        /// One pass over sources that refuse every `load` with `err`.
        fn tick_refusing(&self, now: DateTime<Utc>, err: fn() -> anyhow::Error) -> TickReport {
            let (max_queue_entries, source_roots) = {
                let s = self.shared.settings.lock().unwrap();
                (s.max_queue_entries, s.source_roots(&self.shared.store))
            };
            let sources = all_sources(&source_roots)
                .into_iter()
                .map(|inner| Box::new(RefusingSource { inner, err }) as Box<dyn TraceSource>)
                .collect();
            tick_over(
                &self.shared,
                now,
                sources,
                source_roots.source_identities(),
                max_queue_entries,
            )
            .unwrap()
        }

        fn health_label(&self) -> Option<String> {
            self.shared.health.lock().unwrap().last_error_label.clone()
        }
    }

    #[tokio::test]
    async fn a_session_refused_by_name_is_counted_and_flagged() {
        // The bug: the pass used to drop `source.load`'s error on the floor
        // with a bare `continue`. `source::codex` declines an oversized
        // rollout *by name*, and its comment says so deliberately -- and
        // then nothing counted it, nothing recorded it, and no shell could
        // ever say why a session the contributor finished never appeared.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let now = at("2030-01-01T00:00:00Z");
        f.tick_refusing(now, too_large);
        let r = f.tick_refusing(now, too_large);

        assert_eq!(r.unloadable, 1, "the failure must be counted: {r:?}");
        assert_eq!(r.too_large, 1, "and classified: {r:?}");
        assert_eq!(
            f.health_label().as_deref(),
            Some(health::LABEL_SESSION_TOO_LARGE),
            "a refusal that recurs on every poll must reach the status surface"
        );
    }

    #[tokio::test]
    async fn a_transient_read_failure_is_counted_but_raises_no_standing_flag() {
        // The other half of the classification. A read that failed because
        // of what the machine was doing at that instant will very likely
        // succeed on the next poll sixty seconds later, and a status flag
        // that a single blip pins on a healthy daemon is worse than no flag
        // at all. It is still counted -- nothing is discarded unseen.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let now = at("2030-01-01T00:00:00Z");
        f.tick_refusing(now, transient_io);
        let r = f.tick_refusing(now, transient_io);

        assert_eq!(r.unloadable, 1, "{r:?}");
        assert_eq!(r.too_large, 0, "an IO blip is not a verdict: {r:?}");
        assert_eq!(
            f.health_label(),
            None,
            "one failed read must not flag the daemon"
        );
    }

    #[tokio::test]
    async fn unsupported_export_version_is_visible_and_resolves_after_repair() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let now = at("2030-01-01T00:00:00Z");
        let refusal = || crate::source::opencode::UnsupportedExportVersion.into();
        f.tick_refusing(now, refusal);
        f.tick_refusing(now, refusal);
        assert_eq!(
            f.health_label().as_deref(),
            Some(health::LABEL_OPENCODE_EXPORT_VERSION_UNSUPPORTED)
        );
        f.settle(at("2030-01-02T00:00:00Z")).await;
        assert_eq!(f.health_label(), None);
    }

    #[tokio::test]
    async fn a_full_pass_that_reads_everything_retracts_the_flag() {
        // A condition that cannot retract itself masks every
        // lower-precedence one forever, so the flag has to come off when
        // the corpus is readable again -- and only a pass that asked every
        // source what it has is entitled to say so.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let now = at("2030-01-01T00:00:00Z");
        f.tick_refusing(now, too_large);
        f.tick_refusing(now, too_large);
        assert_eq!(
            f.health_label().as_deref(),
            Some(health::LABEL_SESSION_TOO_LARGE)
        );

        f.settle(at("2030-01-02T00:00:00Z")).await;
        assert_eq!(
            f.health_label(),
            None,
            "the session loads now; the flag must not outlive the condition"
        );
    }

    #[tokio::test]
    async fn a_pipeline_refusal_is_not_a_dismissal_and_still_re_offers() {
        // `Refused` is also what the pipeline records when it will not send
        // a trace -- a residual secret, an unavailable privacy filter. That
        // is the daemon's verdict on some bytes, not the contributor's on
        // the conversation, so a grown session must still be offered again.
        // Only the dismissal label suppresses the re-offer.
        let f = WatcherFixture::new();
        let name = "11111111-1111-1111-1111-111111111111";
        let path = f.write_session("proj", name, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let entry_id = f.shared.queue.lock().unwrap().pending()[0].entry_id;
        {
            let mut queue = f.shared.queue.lock().unwrap();
            queue.set_state(
                entry_id,
                QueueState::Refused,
                Some("residual-secret".to_string()),
            );
        }

        f.append_to_session(&path, "proj", name);
        f.settle(at("2030-01-02T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.pending().len(), 1, "{:?}", queue.all());
    }

    #[tokio::test]
    async fn a_queued_session_that_grew_is_loaded_again_and_supersedes() {
        // The skip must not swallow the supersede path: the offer describes
        // content, the content moved, so the old offer dies and a fresh one
        // is made.
        let f = WatcherFixture::new();
        let name = "11111111-1111-1111-1111-111111111111";
        let path = f.write_session("proj", name, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let first_hash = {
            let queue = f.shared.queue.lock().unwrap();
            queue.all()[0].session_hash.clone()
        };

        f.append_to_session(&path, "proj", name);
        let c = loads();
        f.settle_counted(at("2030-01-02T00:00:00Z"), &c);
        assert_eq!(count(&c), 1, "a grown session must be re-read exactly once");

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 2, "{:?}", queue.all());
        let old = queue
            .all()
            .iter()
            .find(|e| e.session_hash == first_hash)
            .unwrap();
        assert_eq!(old.state, QueueState::Superseded);
        assert_eq!(
            old.reason_label.as_deref(),
            Some(crate::daemon::queue::REASON_CHANGED)
        );
        assert_eq!(queue.pending().len(), 1, "exactly one live offer");
        assert_ne!(queue.pending()[0].session_hash, first_hash);
    }

    #[tokio::test]
    async fn a_new_subagent_reloads_even_though_the_parent_files_own_stat_is_unchanged() {
        // The trap a naive pre-check falls into. A delegated transcript
        // lands beside the session under `<uuid>/subagents/`; the parent
        // file itself is untouched, so a check against the parent's own
        // stat would call this "unchanged" and never re-offer the
        // conversation. The observation compared is the group's, which is
        // why `SessionRef::size_bytes` and `group_modified_at` exist.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        let parent = f.write_session("proj", session, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let first_hash = {
            let queue = f.shared.queue.lock().unwrap();
            queue.all()[0].session_hash.clone()
        };
        let before = std::fs::metadata(&parent).unwrap();

        f.write_subagent("proj", session, "agent-a");
        let after = std::fs::metadata(&parent).unwrap();
        assert_eq!(
            before.len(),
            after.len(),
            "the parent file must be untouched"
        );
        assert_eq!(
            before.modified().unwrap(),
            after.modified().unwrap(),
            "the parent file's own mtime must be untouched -- that is the whole point"
        );

        let c = loads();
        f.settle_counted(at("2030-01-02T00:00:00Z"), &c);
        assert_eq!(count(&c), 1, "the group grew, so it must be re-read");

        let queue = f.shared.queue.lock().unwrap();
        let old = queue
            .all()
            .iter()
            .find(|e| e.session_hash == first_hash)
            .unwrap();
        assert_eq!(old.state, QueueState::Superseded);
        assert_eq!(queue.pending().len(), 1);
        assert_eq!(queue.pending()[0].subagent_count, 1);
    }

    #[tokio::test]
    async fn a_standing_opt_in_still_reaches_an_entry_the_skip_path_finds() {
        // The one thing the discarded per-poll work did do: re-apply a
        // project's standing `auto_upload` to an entry that has since been
        // put back to `Pending`. Skipping the load must not skip that, or
        // arming a project would stop applying to entries already offered.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.states(), vec![QueueState::Pending]);

        f.set_mode("proj", ProjectMode::AutoUpload);
        let c = loads();
        let report = f.tick_counted(at("2030-01-01T00:01:00Z"), &c);
        assert_eq!(count(&c), 0, "re-approving must not cost a re-read");
        assert_eq!(report.auto_ready, 1, "{report:?}");
        assert_eq!(f.states(), vec![QueueState::Approved]);
    }

    #[tokio::test]
    async fn an_ignored_project_with_a_queued_entry_is_still_reported_ignored() {
        // The skip path reports the same way the load path did, from the
        // entry's own project key rather than a re-resolved cwd.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        f.set_mode("proj", ProjectMode::Ignore);
        let c = loads();
        let report = f.tick_counted(at("2030-01-01T00:01:00Z"), &c);
        assert_eq!(report.ignored, 1, "{report:?}");
        assert_eq!(count(&c), 0);
    }

    #[tokio::test]
    async fn a_queue_entry_records_the_observation_it_was_built_from() {
        // The comparison data lives on the entry, so it cannot drift from
        // the queue it describes.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let queue = f.shared.queue.lock().unwrap();
        let e = &queue.all()[0];
        let observed = e
            .observed_modified_at
            .expect("the entry must record its mtime");
        assert!(
            queue
                .unchanged_offer_at_path(&e.path, e.size_bytes, observed)
                .is_some(),
            "the recorded observation must be the one the next poll compares against"
        );
    }

    /// Overwrite the daemon state file with bytes a save would never
    /// produce, so a later write is detectable by the sentinel being gone.
    /// The in-memory state stays authoritative -- the file is only read at
    /// startup -- so this observes writes without disturbing the tick.
    fn plant_state_sentinel(f: &WatcherFixture) {
        std::fs::write(
            f.shared.store.daemon_path(crate::config::DAEMON_STATE_FILE),
            b"SENTINEL",
        )
        .unwrap();
    }

    fn state_sentinel_survived(f: &WatcherFixture) -> bool {
        std::fs::read(f.shared.store.daemon_path(crate::config::DAEMON_STATE_FILE)).unwrap()
            == b"SENTINEL"
    }

    #[tokio::test]
    async fn a_tick_that_moved_nothing_does_not_rewrite_the_state_file() {
        // The second remaining per-poll cost. Every tick re-serialized the
        // whole `DaemonState` and wrote it with an fsync -- 1.24 MB a minute
        // on the reported machine -- even when the pass changed nothing.
        // `observe` runs for every path on every poll, which is why this is
        // asserted against writes rather than against a dirty flag: the
        // bookkeeping is touched either way, and what must not happen is
        // the write.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1);

        plant_state_sentinel(&f);
        for minute in 1..=3 {
            tick(&f.shared, at(&format!("2030-01-01T00:0{minute}:00Z")))
                .await
                .unwrap();
        }
        assert!(
            state_sentinel_survived(&f),
            "three idle polls must not rewrite the state file"
        );
    }

    #[tokio::test]
    async fn a_tick_that_saw_something_new_does_rewrite_the_state_file() {
        // The other half: eliding the write must not lose an observation.
        // A session first sighted on this tick has to be on disk, or a
        // restart would treat it as never seen and the size-stability check
        // would start over.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        plant_state_sentinel(&f);

        let second = "22222222-2222-2222-2222-222222222222";
        let path = f.write_session("proj", second, 0);
        tick(&f.shared, at("2030-01-01T00:01:00Z")).await.unwrap();
        assert!(
            !state_sentinel_survived(&f),
            "a first sighting must be persisted"
        );
        let reloaded = crate::daemon::state::DaemonState::load(&f.shared.store).unwrap();
        assert!(
            reloaded.previous_size(&path).is_some(),
            "the reloaded state must carry the new session's observation"
        );
    }

    #[tokio::test]
    async fn a_restart_after_idle_polls_still_knows_what_was_offered() {
        // The risk the elision has to be measured against: state that was
        // never written is state a restart cannot see. After idle polls
        // skipped their writes, a daemon reloading from disk must still
        // find the observations and must not re-offer the session it
        // already has a card for.
        let f = WatcherFixture::new();
        let path = f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        for minute in 1..=3 {
            tick(&f.shared, at(&format!("2030-01-01T00:0{minute}:00Z")))
                .await
                .unwrap();
        }

        let reloaded = crate::daemon::state::DaemonState::load(&f.shared.store).unwrap();
        let live = f.shared.state.lock().unwrap();
        assert_eq!(
            reloaded.previous_size(&path),
            live.previous_size(&path),
            "the file must agree with memory about the last observation"
        );
        assert_eq!(
            reloaded.last_observation, live.last_observation,
            "no observation may be lost to a skipped write"
        );
        assert_eq!(reloaded.cwd_cache, live.cwd_cache);
        assert_eq!(reloaded.prior_uploads, live.prior_uploads);
    }

    #[tokio::test]
    async fn queue_full_is_retracted_when_a_new_entry_passes_capacity_check() {
        // When a genuinely new entry is inserted, it passed the capacity check,
        // so queue-full can be safely retracted: space is available.
        let f = WatcherFixture::new();
        // Set queue-full manually to simulate prior failure
        {
            let mut health = f.shared.health.lock().unwrap();
            health.fail(health::LABEL_QUEUE_FULL, at("2026-08-08T12:00:00Z"));
        }
        assert!(!{ f.shared.health.lock().unwrap().ok() });
        // Write a new session and settle it (two ticks to pass eligibility check)
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        // After settling, a new entry was inserted, which proves capacity check passed
        assert_eq!(f.queue_len(), 1);
        // queue-full should be retracted
        assert!({ f.shared.health.lock().unwrap().ok() });
    }

    #[tokio::test]
    async fn queue_full_survives_when_only_dedup_reobservation_occurs() {
        // When only dedup re-observation occurs (session is already queued),
        // Queue::upsert returns Ok(()) BEFORE checking capacity. Do not retract
        // queue-full, as there is no evidence of available space.
        let f = WatcherFixture::new();
        // Write a session and settle it so it is queued
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1);
        // Set queue-full manually
        {
            let mut health = f.shared.health.lock().unwrap();
            health.fail(health::LABEL_QUEUE_FULL, at("2026-08-08T12:00:00Z"));
        }
        assert!(!{ f.shared.health.lock().unwrap().ok() });
        // Tick again: the same session is re-observed (dedup path)
        tick(&f.shared, at("2030-01-02T00:00:00Z")).await.unwrap();
        // Still exactly one entry (dedup did not insert a duplicate)
        assert_eq!(f.queue_len(), 1);
        // queue-full should SURVIVE because dedup does not check capacity
        assert!(
            !{ f.shared.health.lock().unwrap().ok() },
            "queue-full must persist on dedup re-observation"
        );
    }
    #[tokio::test]
    async fn a_session_a_full_queue_cannot_hold_is_never_loaded() {
        // The bug. With a corpus larger than `max_queue_entries` -- 3,152
        // sessions against a cap of 500 on the machine that reported this
        // -- every session the queue has no room for is eligible, has no
        // unchanged offer to be skipped by, and so was read, parsed and
        // hashed in full on every sixty-second poll, purely to be refused
        // `queue-full` afterwards. 74.6% of one core, on an idle app.
        let f = WatcherFixture::new();
        f.set_max_queue_entries(1);
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1, "the queue is now at capacity");

        f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        let c = loads();
        for minute in 1..=3 {
            f.tick_counted(at(&format!("2030-01-01T00:0{minute}:00Z")), &c);
        }
        assert_eq!(
            count(&c),
            0,
            "a session the full queue cannot hold must not be read at all"
        );
        assert_eq!(f.queue_len(), 1, "and nothing lands, exactly as before");
        assert!(
            !{ f.shared.health.lock().unwrap().ok() },
            "the contributor must still be told the queue is full"
        );
    }

    #[tokio::test]
    async fn a_grown_session_with_a_live_card_still_loads_and_supersedes_at_capacity() {
        // The trap the capacity pre-check must not fall into. A naive "the
        // queue is full, skip" would mean a conversation that grew could
        // never supersede its own stale card once the queue filled up, and
        // the contributor would be left looking at an offer describing
        // content that has moved on. `replace_live_at_path` frees the slot
        // it is about to reuse, so this load can land and must happen.
        let f = WatcherFixture::new();
        f.set_max_queue_entries(1);
        let name = "11111111-1111-1111-1111-111111111111";
        let path = f.write_session("proj", name, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let first_hash = {
            let queue = f.shared.queue.lock().unwrap();
            queue.all()[0].session_hash.clone()
        };
        assert_eq!(f.queue_len(), 1, "the queue is at capacity");

        f.append_to_session(&path, "proj", name);
        let c = loads();
        f.settle_counted(at("2030-01-02T00:00:00Z"), &c);
        assert_eq!(
            count(&c),
            1,
            "a grown session with a live card must still be re-read at capacity"
        );

        let queue = f.shared.queue.lock().unwrap();
        let old = queue
            .all()
            .iter()
            .find(|e| e.session_hash == first_hash)
            .expect("the first offer is still on record");
        assert_eq!(old.state, QueueState::Superseded, "{:?}", queue.all());
        assert_eq!(queue.pending().len(), 1, "exactly one live offer");
        assert_ne!(
            queue.pending()[0].session_hash,
            first_hash,
            "and it describes the session as it now stands"
        );
    }

    #[tokio::test]
    async fn a_queue_with_room_still_loads_and_offers_every_session() {
        // The other side of the check: below the cap nothing changes, and a
        // second session is read once and queued.
        let f = WatcherFixture::new();
        f.set_max_queue_entries(10);
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1);

        f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        let c = loads();
        let report = f.settle_counted(at("2030-01-01T00:01:00Z"), &c);
        assert_eq!(report.queued, 1, "{report:?}");
        assert_eq!(count(&c), 1, "the new session is read exactly once");
        assert_eq!(f.queue_len(), 2);
        assert!(
            { f.shared.health.lock().unwrap().ok() },
            "a queue with room must not report queue-full"
        );
    }

    // --- the scoped entry point -------------------------------------------
    //
    // Same body, same epilogue, different answer to "which sessions?". These
    // four pin the difference: what it visits, that it decides the same
    // thing, that the epilogue is a per-pass event and not a per-session one,
    // and that an empty batch is genuinely free.

    #[tokio::test]
    async fn a_scoped_pass_visits_only_the_supplied_sessions_and_never_walks_the_corpus() {
        let f = WatcherFixture::new();
        let target = f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        f.write_session("other", "33333333-3333-3333-3333-333333333333", 0);

        let (l, d) = (loads(), loads());
        let report = f.settle_paths(
            at("2030-01-01T00:00:00Z"),
            &l,
            &d,
            std::slice::from_ref(&target),
        );

        assert_eq!(count(&d), 0, "the scoped pass must not call discover");
        assert_eq!(report.observed, 1, "one supplied path, one session visited");
        assert_eq!(f.queue_len(), 1);
        assert_eq!(
            f.shared.queue.lock().unwrap().all()[0].path,
            target,
            "the queued entry is the supplied session, not a corpus neighbour"
        );
    }

    #[tokio::test]
    async fn a_scoped_pass_reaches_the_same_decision_as_the_full_scan() {
        // Same session, same clock, same policy; only the route differs. The
        // per-session body is shared, so this is really a test that the
        // scoped path feeds it the same observation -- in particular the same
        // `previous_size`, which is what eligibility turns on.
        let name = "11111111-1111-1111-1111-111111111111";
        let now = at("2030-01-01T00:00:00Z");

        let scanned = WatcherFixture::new();
        let scanned_path = scanned.write_session("proj", name, 0);
        scanned.settle(now).await;

        let scoped = WatcherFixture::new();
        let scoped_path = scoped.write_session("proj", name, 0);
        let (l, d) = (loads(), loads());
        scoped.settle_paths(now, &l, &d, std::slice::from_ref(&scoped_path));

        let decision = |f: &WatcherFixture| {
            f.shared
                .queue
                .lock()
                .unwrap()
                .all()
                .iter()
                .map(|e| {
                    (
                        e.session_hash.clone(),
                        e.state,
                        e.project_label.clone(),
                        e.size_bytes,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(decision(&scanned).len(), 1);
        assert_eq!(decision(&scoped), decision(&scanned));

        // `observed_modified_at` is a real file mtime, and the two fixtures
        // wrote their copies milliseconds apart, so it is compared against
        // each fixture's own file rather than across them: what matters is
        // that both routes recorded the observation they judged.
        let recorded = |f: &WatcherFixture, path: &Path| {
            let want = DateTime::<Utc>::from(std::fs::metadata(path).unwrap().modified().unwrap());
            f.shared.queue.lock().unwrap().all()[0].observed_modified_at == Some(want)
        };
        assert!(recorded(&scanned, &scanned_path));
        assert!(recorded(&scoped, &scoped_path));
    }

    #[tokio::test]
    async fn a_scoped_pass_runs_the_epilogue_once_for_the_whole_batch() {
        // `queue.save` and `state.save` each rewrite a whole file and the
        // publish wakes every attached shell, so the epilogue belongs to the
        // pass, not to the session. Three sessions landing together must
        // still be one publish.
        let f = WatcherFixture::new();
        let a = f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let b = f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        let c = f.write_session("other", "33333333-3333-3333-3333-333333333333", 0);

        let mut rx = f.shared.events.subscribe();
        let (l, d) = (loads(), loads());
        let report = f.settle_paths(at("2030-01-01T00:00:00Z"), &l, &d, &[a, b, c]);

        assert_eq!(report.queued, 3);
        assert_eq!(f.queue_len(), 3);
        assert_eq!(
            WatcherFixture::drain_changed(&mut rx),
            1,
            "three sessions in one pass are one queue-changed publish"
        );
    }

    #[tokio::test]
    async fn an_empty_scoped_pass_does_nothing_and_publishes_nothing() {
        // The debounce will hand this path an empty batch whenever every
        // dirty session is still inside its window. That must cost nothing:
        // no publish for the shells to redraw on, and no state rewrite.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        // Settle first: the elision compares against what this process last
        // wrote, so a state file that has never been written is rewritten by
        // any pass at all, scoped or not.
        f.settle(at("2030-01-01T00:00:00Z")).await;

        let mut rx = f.shared.events.subscribe();
        plant_state_sentinel(&f);
        let (l, d) = (loads(), loads());
        let report = f.tick_paths_counted(at("2030-01-01T00:00:00Z"), &l, &d, &[]);

        assert_eq!(report, TickReport::default());
        assert_eq!(f.queue_len(), 1, "the settled entry is untouched");
        assert_eq!(count(&l), 0);
        assert_eq!(count(&d), 0);
        assert_eq!(WatcherFixture::drain_changed(&mut rx), 0);
        assert!(
            state_sentinel_survived(&f),
            "an empty batch moved nothing and must not rewrite the state file"
        );
    }

    /// The entry records its eligibility at the one moment the answer is
    /// free -- the load the watcher had already paid for -- rather than
    /// leaving a list to ask for it later.
    ///
    /// A session with no inference hops is the case the whole surface exists
    /// for: everything a contributor recorded before they started having
    /// their model calls answered here. It is permanent, and the entry says
    /// so before anybody presses anything.
    #[tokio::test]
    async fn a_queued_session_with_no_hops_records_a_permanent_ineligibility() {
        use crate::daemon::contribution_eligibility as ce;
        let f = WatcherFixture::new();
        f.admitted_on_evidence();
        f.write_session("repo", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        let entry = queue.all().first().expect("one entry").clone();
        assert_eq!(
            entry.eligibility.as_deref(),
            Some(ce::STATE_INELIGIBLE_PERMANENT)
        );
        assert_eq!(
            entry.eligibility_reason.as_deref(),
            Some(ce::REASON_NO_CALL)
        );
    }

    /// And an invited contributor's entry records nothing at all, so the
    /// wire has nothing to render. The queue is the same queue; the only
    /// difference is that nobody is being asked a question they do not have.
    #[tokio::test]
    async fn an_invited_contributors_entry_records_no_eligibility() {
        let f = WatcherFixture::new();
        f.write_session("repo", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        let entry = queue.all().first().expect("one entry").clone();
        assert_eq!(entry.eligibility, None);
        assert_eq!(entry.eligibility_reason, None);
    }
}
