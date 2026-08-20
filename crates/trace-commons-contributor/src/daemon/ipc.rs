//! The IPC contract: `trace_commons.daemon.v1_1` (v1 clients remain
//! supported; see `SUPPORTED_VERSIONS`).
//!
//! This is the surface the native menu-bar and window applications are built
//! against, so it is versioned and frozen rather than allowed to drift. It
//! serves both surfaces: the tray needs `status`, `list_pending`, `approve`,
//! `pause`/`resume` and the event stream; the window additionally needs
//! `preview`, `list_history`, `history_rollup`, `list_projects` and settings.
//!
//! Framing is JSON, one message per line, over a unix domain socket. Every
//! request carries an `id` and every response echoes it, because responses and
//! pushed events share the connection and a client with two calls in flight
//! must be able to tell which answer is which. Pushed events carry `event` and
//! never an `id`.
//!
//! # Authorization
//!
//! Filesystem ownership: the 0700 state directory is the sole access control
//! on the socket, since `UnixListener::bind` does not portably set the
//! socket's own mode; the daemon refuses to serve from a directory that is
//! not 0700.
//!
//! Two operations -- arming a project for `auto_upload` and bulk-approving
//! the whole queue -- used to be refused over the socket and required a
//! terminal. That restriction is gone: the reasoning behind it does not
//! survive scrutiny. Same-user code execution that can reach this socket can
//! already read `~/.claude/projects` directly and send it anywhere, and can
//! install its own persistent watcher -- the daemon confers neither the read
//! nor the persistence a real attacker needs. Routing exfiltration through it
//! would in fact be strictly worse for an attacker: rate-limited, capped,
//! redacted, PII-filtered, and delivered to a server they cannot read from.
//!
//! What replaces the restriction is visibility, not gatekeeping: both
//! operations append a local, hash-only audit entry (`daemon::audit`) that a
//! contributor can read to see when autonomy was granted and when a bulk
//! approval happened. This is user-facing visibility, not a security
//! control, and is not claimed to be one -- but it is written fail-closed:
//! an audited action whose entry cannot be persisted is rolled back and the
//! call returns `audit-write-failed`, because a change that stands with no
//! record of it is exactly what removing the restriction was not supposed
//! to make possible. See `daemon::audit`.
//!
//! # What crosses this socket
//!
//! No path, token, invite code, claim, device key, or trace content
//! appears in any response, error string, or pushed event. `error.message`
//! is a fixed label. Queue entries carry `project_label`, never
//! `project_key` or `path`. Project labels are derived by the daemon from
//! the key and are never a string a caller supplied.
//!
//! **The preview exemption.** `"preview"`'s `opening_prompt`,
//! `"preview_body"`'s `chunk`, and the redacted body `open_preview` returns
//! to the C ABI, *are* trace content, deliberately. A contributor cannot
//! consent to sending something they cannot see, so preview is the one
//! interface allowed to carry it -- bounded to post-redaction content, only
//! for an `entry_id` the caller already holds, and never onward into a log
//! line, an audit entry, a history record, notification text, or a receipt.
//! Everywhere else in this module the rule is absolute.
//!
//! `"preview_body"` is the *same* carve-out reaching the same body over the
//! socket, not a second one. It exists because the body used to be
//! reachable only through `open_preview`, which takes `&DaemonShared` and so
//! can only be called by the process holding the daemon lock. On the
//! recommended Linux arrangement -- a systemd-managed daemon with the window
//! as a socket client -- that is never the window, so "search this trace for
//! my client's name" and "show me exactly what would be sent" were not slow
//! or awkward there, they were impossible. Loading a second `DaemonShared`
//! is not the workaround it looks like: it rewrites the queue file and
//! sweeps the pinned envelopes the running daemon is still holding.
//!
//! # Sync vs. async dispatch
//!
//! Most of this surface needs no `.await` and is answered by the synchronous
//! `handle_request`. A few methods do real async work -- `"preview"` runs the
//! redaction pipeline to report actual bytes and redactions, `"enroll"`
//! registers this device with an issuer over the network -- and
//! `handle_request` cannot run either of those to completion; its arms for
//! them (where present) return an honest partial or deferred answer rather
//! than a wrong one.
//!
//! `handle_request_async` is the complete dispatcher: it answers the async
//! methods for real and delegates everything else, unchanged, to
//! `handle_request`. There are exactly two real entry points, and both go
//! through it:
//!
//! - The socket connection loop (`serve_connection`), already async, calls
//!   `handle_request_async` directly.
//! - `handle_local` (the in-process CLI path, wired in
//!   `src/bin/trace-commons-contributor.rs`) is itself synchronous, so it
//!   runs `handle_request_async` to completion via `block_on_ipc`, a
//!   scoped-OS-thread blocking wrapper. It does this for *every* method, not
//!   only the async ones -- a per-method special case here was tried once
//!   already and is exactly how a socket caller and a CLI caller ended up
//!   able to get different answers to the same request. Routing every method
//!   through the one real dispatcher removes that failure mode by
//!   construction: a method added to `handle_request_async` is automatically
//!   answered identically by both callers, with nothing to remember to update
//!   here.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use anyhow::bail;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(unix)]
use tokio::net::UnixListener;
use tokio::sync::{Notify, broadcast};
use uuid::Uuid;

use super::audit::{self, AuditEntry};
use super::enroll;
use super::health::HealthState;
use super::history::{HistoryCache, rollup};
use super::policy::{
    ERR_PROJECT_ID_UNRECOGNIZED, ERR_PROJECT_KEY_UNRECOGNIZED, ProjectMode, ProjectPolicy,
    UNKNOWN_PROJECT_KEY, disambiguated_label, known_keys, project_id_for, project_key_for_id,
    project_key_is_admissible,
};
use super::queue::{Queue, QueueState};
use super::settings::DaemonSettings;
use super::state::DaemonState;
use crate::config::ConfigStore;
#[cfg(unix)]
use crate::config::DAEMON_SOCK_FILE;

pub const IPC_SCHEMA: &str = "trace_commons.daemon.v1_1";
/// Every schema version a client may declare compatibility with. `hello`
/// reports this so a v1 client (built before the seven methods below existed
/// and before the terminal-only gate was dropped) can keep talking to this
/// daemon: every v1 method keeps its v1 request and response shape, so a v1
/// client that ignores unfamiliar methods and fields works unmodified.
pub const SUPPORTED_VERSIONS: [&str; 2] = ["trace_commons.daemon.v1", "trace_commons.daemon.v1_1"];
/// Longest accepted request line. Anything larger is a malformed client, not a
/// real request.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

/// The largest slice of a redacted preview body `preview_body` will put in
/// one frame, and the cap it silently applies to a larger `limit`.
///
/// Sized against [`MAX_LINE_BYTES`], not against the body. A redacted
/// envelope may approach `MAX_ENVELOPE_BYTES` (1.5 MB), so a whole body does
/// not reliably fit one 1 MiB line and `preview_body` pages. The chunk still
/// has to survive JSON string escaping on the way out: `serde_json` passes
/// non-ASCII UTF-8 through unescaped but expands a control byte to `\u00XX`,
/// six bytes for one, so a pathological 128 KiB chunk serializes to at most
/// 768 KiB and the frame stays comfortably inside the line cap with the
/// response's own fields on top.
pub const MAX_PREVIEW_BODY_CHUNK_BYTES: usize = 128 * 1024;

pub const ERR_UNKNOWN_METHOD: &str = "unknown_method";
pub const ERR_BAD_PARAMS: &str = "bad_params";
pub const ERR_NOT_AUTHORIZED: &str = "not_authorized";
pub const ERR_BUSY: &str = "busy";
pub const ERR_UNAVAILABLE: &str = "unavailable";

/// `preview_body` refused because the body it resolved is not the one the
/// caller has been reading: the `body_digest` from the caller's first page
/// does not match. Splicing two pages of two different bodies together
/// would produce a transcript nobody ever redacted, and a search over it
/// would be answering about text that does not exist. Restart from
/// `offset: 0`.
pub const ERR_PREVIEW_BODY_CHANGED: &str = "preview-body-changed";
/// A continuation page (`offset > 0`) arrived without the `body_digest` the
/// first page returned. Required, not optional: without it the daemon
/// cannot tell a continuation of the body the caller holds from a page of a
/// different one, and paging is the whole reason this method exists.
pub const ERR_BODY_DIGEST_REQUIRED: &str = "body-digest-required";
/// The fixed label every `preview_body` refusal for an entry the caller
/// does not hold -- unknown id, or an id that is not in the queue -- comes
/// back under. Identical to `preview`'s, deliberately: the two must not be
/// distinguishable.
pub const ERR_UNKNOWN_ENTRY_ID: &str = "unknown-entry-id";

/// `quiesce` gave up waiting for in-flight uploads to finish. The caller
/// leaves the update staged and tries again later; the swap never forces its
/// way past active work, because a half-uploaded trace is not an acceptable
/// cost for an update.
pub const ERR_QUIESCE_TIMEOUT: &str = "quiesce-timeout";

/// How long `quiesce` waits for in-flight uploads by default.
pub const DEFAULT_QUIESCE_TIMEOUT_SECS: u64 = 60;
/// The longest a caller may ask `quiesce` to park uploads for.
pub const MAX_QUIESCE_TIMEOUT_SECS: u64 = 300;
/// How often the drain is re-checked while waiting.
const QUIESCE_POLL_MS: u64 = 200;

/// Every method this version answers. `hello` reports this list, and the
/// contract document is checked against it by test.
pub const METHODS: [&str; 32] = [
    "acknowledge_near_ai_notice",
    "approve",
    "cancel",
    "clear_public_profile",
    "consent_options",
    "dismiss",
    "enroll",
    "get_public_profile",
    "get_settings",
    "hello",
    "history_rollup",
    "list_audit",
    "list_history",
    "list_pending",
    "list_projects",
    "pause",
    "preview",
    "preview_body",
    "preview_turns",
    "queue_outcome_counts",
    "quiesce",
    "refresh_history",
    "resume",
    "set_consent_scopes",
    "set_project_mode",
    "set_public_profile",
    "set_settings",
    "shutdown",
    "status",
    "subscribe",
    "withdraw",
    "withdraw_bulk",
];

pub const EVENT_SNAPSHOT: &str = "snapshot";
pub const EVENT_QUEUE_CHANGED: &str = "queue_changed";
pub const EVENT_STATUS_CHANGED: &str = "status_changed";
pub const EVENT_DIGEST_DUE: &str = "digest_due";
pub const EVENT_RESYNC_REQUIRED: &str = "resync_required";

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcError {
    pub code: String,
    /// A fixed label, never a message body or server response text.
    pub message: String,
}

/// `Deserialize` as well as `Serialize`: `daemon::client` parses a running
/// daemon's reply back into this exact type, so the CLI's view of a
/// response and the socket's wire shape are the same definition rather
/// than two that can drift.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub id: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

impl Response {
    /// `pub`, not `pub(crate)`: `trace-commons-contributor-ffi` builds
    /// error frames for failures it must synthesize itself (a malformed
    /// `params_json`, a null pointer) rather than ones `handle_local`
    /// produces, and needs the exact wire shape a real dispatcher response
    /// serializes to -- constructing it this way, rather than hand-rolling
    /// an equivalent `serde_json::json!`, is what keeps the two from
    /// drifting apart.
    pub fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    /// See `ok`'s doc comment on why this is `pub`.
    pub fn err(id: u64, code: &str, message: &str) -> Self {
        Self {
            id,
            result: None,
            error: Some(IpcError {
                code: code.to_string(),
                message: message.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Event {
    pub event: String,
    pub data: serde_json::Value,
}

/// Everything the daemon's loops and its IPC server share.
pub struct DaemonShared {
    pub store: ConfigStore,
    pub queue: Mutex<Queue>,
    pub policy: Mutex<ProjectPolicy>,
    pub state: Mutex<DaemonState>,
    pub settings: Mutex<DaemonSettings>,
    pub health: Mutex<HealthState>,
    pub paused: AtomicBool,
    /// Uploads are parked for an update swap.
    ///
    /// Deliberately *not* `paused`. Pause is the contributor's own setting
    /// and is persisted in `daemon-state.json`; an update that set it would
    /// be rewriting their preference, and a crash between quiescing and
    /// swapping would leave the daemon paused forever with nothing to say
    /// why. This flag is in-memory only and dies with the process, which is
    /// exactly the lifetime an update swap needs: after the swap there is a
    /// new process and nothing left to un-quiesce.
    pub quiesced: AtomicBool,
    pub shutdown: AtomicBool,
    /// Wakes the supervisor immediately on a shutdown request. Without it the
    /// daemon would not notice until its next poll, which is a minute away --
    /// long enough for a logout to give up waiting and leave it running.
    ///
    /// Notified with `notify_one`, which stores a permit when nobody is
    /// waiting yet. `notify_waiters` would drop the request on the floor if it
    /// arrived while the supervisor was mid-scan, which is exactly when a
    /// long poll makes it most likely to arrive.
    pub shutdown_signal: Arc<Notify>,
    pub events: broadcast::Sender<Event>,
}

impl DaemonShared {
    pub fn load(store: ConfigStore) -> Result<Self> {
        let mut queue = Queue::load(&store)?;
        // `Uploading` is a transient, in-pass claim. A daemon that died
        // mid-upload leaves entries in it, and nothing else would ever move
        // them out: never uploaded, never offered again. Re-sending is safe
        // -- the receipts file dedups by session hash, so a session that
        // did reach the server comes back `AlreadySubmitted`.
        if queue.release_in_flight() {
            queue.save(&store)?;
        }
        // One-time upgrade: retire entries that stand for a single subagent
        // transcript.
        //
        // Those entries were minted when each `<uuid>/subagents/*.jsonl`
        // file was discovered as a session in its own right. Discovery no
        // longer yields those paths, so `find_session` cannot resolve them:
        // an approved one would fail with `session-file-vanished` and a
        // pending one would sit in the queue until it aged out. Both are
        // safe and both are confusing, and leaving them would keep offering
        // a fragment whose opening prompt was written by the parent agent
        // rather than by the contributor. Superseding says what actually
        // happened -- the conversation each belongs to is offered whole
        // instead -- and releases the stored preview envelope on the next
        // sweep.
        if regroup_subagent_entries(&mut queue) {
            queue.save(&store)?;
        }
        // Sweep stored preview envelopes on the way up. A daemon that died
        // between resolving an entry and sweeping, or one whose queue file
        // was replaced underneath it, would otherwise leave redacted trace
        // content on disk with no entry that needs it.
        let _ = super::approved_envelope::sweep(&store, &queue.pinned_entry_ids());
        let policy = ProjectPolicy::load(&store)?;
        let state = DaemonState::load(&store)?;
        let settings = DaemonSettings::load(&store)?;
        let (events, _) = broadcast::channel(256);
        let paused = state.paused;
        Ok(Self {
            store,
            queue: Mutex::new(queue),
            policy: Mutex::new(policy),
            state: Mutex::new(state),
            settings: Mutex::new(settings),
            health: Mutex::new(HealthState::default()),
            paused: AtomicBool::new(paused),
            quiesced: AtomicBool::new(false),
            shutdown: AtomicBool::new(false),
            shutdown_signal: Arc::new(Notify::new()),
            events,
        })
    }

    pub fn publish(&self, event: &str, data: serde_json::Value) {
        // A send with no subscribers is not an error: the daemon runs happily
        // with no application attached.
        let _ = self.events.send(Event {
            event: event.to_string(),
            data,
        });
    }

    fn logged_in(&self) -> bool {
        super::uploader::enrollment_is_live(&self.store)
    }

    /// Whether the daemon is currently paused, accounting for a timed pause
    /// that has lapsed.
    ///
    /// An elapsed timed pause auto-clears here (and persists the clear)
    /// rather than leaving the daemon paused until an explicit `resume`: an
    /// app-side timer would die with the app and silently fail to un-pause
    /// it otherwise.
    pub fn is_paused(&self, now: chrono::DateTime<Utc>) -> bool {
        if !self.paused.load(Ordering::Relaxed) {
            return false;
        }
        let mut state = self.state.lock().expect("state lock");
        if let Some(until) = state.paused_until {
            if now >= until {
                state.paused_until = None;
                state.paused = false;
                let _ = state.save(&self.store);
                drop(state);
                self.paused.store(false, Ordering::Relaxed);
                self.publish(EVENT_STATUS_CHANGED, serde_json::json!({}));
                return false;
            }
        }
        // Read `state.paused` rather than returning `true` unconditionally:
        // two concurrent readers both pass the atomic check above, but only
        // one of them actually clears the lapsed pause (it wins the `state`
        // lock first); the other must not report a pause that its sibling
        // call just resolved.
        state.paused
    }

    /// The tray's whole world in one object.
    pub fn status_value(&self) -> serde_json::Value {
        let queue = self.queue.lock().expect("queue lock");
        let health = self.health.lock().expect("health lock");
        let cfg = self.store.load_config().ok().flatten();
        serde_json::json!({
            "schema_version": IPC_SCHEMA,
            "logged_in": self.logged_in(),
            "tenant_id": cfg.as_ref().map(|c| c.tenant_id.clone()),
            "consent_scopes": cfg.as_ref().map(|c| c.consent_scopes.clone()).unwrap_or_default(),
            "paused": self.is_paused(Utc::now()),
            "queue_depth": queue.pending().len(),
            "next_digest_at": self.next_digest_at(),
            "health": {
                "last_error_label": health.last_error_label,
                "since": health.since,
            },
        })
    }

    fn next_digest_at(&self) -> Option<chrono::DateTime<Utc>> {
        let state = self.state.lock().expect("state lock");
        let settings = self.settings.lock().expect("settings lock");
        state
            .last_digest_at
            .map(|t| t + chrono::Duration::seconds(settings.digest_interval_secs as i64))
    }

    fn snapshot_value(&self) -> serde_json::Value {
        let pending: Vec<serde_json::Value> = {
            let queue = self.queue.lock().expect("queue lock");
            queue.pending().iter().map(|e| entry_value(e)).collect()
        };
        serde_json::json!({
            "pending": pending,
            "status": self.status_value(),
        })
    }
}

/// Recompute every queue entry's `project_label` against the current
/// known-key set (every configured project plus every project already in
/// the queue) and rewrite any that changed. Returns whether anything
/// changed, so the caller knows whether to persist and publish.
///
/// This is the single implementation of "what does a project collide
/// with, and should this entry's label change" -- both `watcher::tick`
/// (after every poll) and `set_project_mode` (immediately after a policy
/// edit) call it, so a queue entry's stored label can never be computed by
/// two different pieces of logic that drift apart. It takes already-locked
/// guards rather than locking `DaemonShared` itself, since every caller
/// already holds (or is about to take) both locks at the point it needs
/// this.
pub fn relabel_queue_entries(policy: &ProjectPolicy, queue: &mut Queue) -> bool {
    let known = known_keys(policy, queue.all().iter().map(|e| e.project_key.clone()));
    let updates: Vec<(Uuid, String)> = queue
        .all()
        .iter()
        .filter_map(|e| {
            let fresh = disambiguated_label(&e.project_key, &known);
            (fresh != e.project_label).then_some((e.entry_id, fresh))
        })
        .collect();
    let changed = !updates.is_empty();
    for (entry_id, label) in updates {
        queue.set_project_label(entry_id, label);
    }
    changed
}

/// The wire shape of a queue entry.
///
/// `path` and `project_key` are deliberately absent: both are local
/// filesystem paths, and applications render `project_label`.
///
/// `project_id` is the opaque handle that makes the label useful for more
/// than rendering: it is the only thing a socket client can pass back to
/// `set_project_mode` to arm or silence the project this entry came from.
/// It is a hash of the key, so it carries no path component (see
/// `policy::project_id_for`).
/// Supersede every live queue entry whose path sits under a `subagents/`
/// directory, because such a path is no longer a session the daemon can
/// discover. Returns whether anything changed.
///
/// Matched on the path shape rather than on the source name: it is the
/// layout, not the adapter, that stopped being addressable. Entries in a
/// terminal state are left exactly as they are -- they are history, and a
/// record of what was uploaded must not be rewritten by an upgrade.
fn regroup_subagent_entries(queue: &mut Queue) -> bool {
    let stale: Vec<uuid::Uuid> = queue
        .all()
        .iter()
        .filter(|e| {
            matches!(
                e.state,
                super::queue::QueueState::Pending | super::queue::QueueState::Approved
            ) && e
                .path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == "subagents")
        })
        .map(|e| e.entry_id)
        .collect();
    if stale.is_empty() {
        return false;
    }
    for entry_id in stale {
        queue.set_state(
            entry_id,
            super::queue::QueueState::Superseded,
            Some("regrouped-under-parent".to_string()),
        );
    }
    true
}

pub fn entry_value(e: &super::queue::QueueEntry) -> serde_json::Value {
    serde_json::json!({
        "entry_id": e.entry_id,
        "session_hash": e.session_hash,
        "source": e.source,
        "project_id": project_id_for(&e.project_key),
        "project_label": e.project_label,
        "size_bytes": e.size_bytes,
        "discovered_at": e.discovered_at,
        "state": e.state,
        "reason_label": e.reason_label,
        "attempts": e.attempts,
        "retry_after": e.retry_after,
        "submission_id": e.submission_id,
        // Additive, and the reason the card can be honest about its own
        // extent: one entry can stand for a conversation plus a hundred
        // delegated transcripts, which is material to the consent decision
        // rather than decoration. `subagents_dropped` is non-zero only when
        // the conversation was trimmed to fit the byte budget, and a card
        // showing it is the difference between a trimmed trace and a
        // silently partial one. No ordinal is exposed: there is no "1 of 3"
        // to expose, because nothing in the format supplies one.
        "subagent_count": e.subagent_count,
        "subagents_dropped": e.subagents_dropped,
    })
}

pub fn handle_request(shared: &DaemonShared, req: &Request) -> Response {
    match req.method.as_str() {
        "hello" => Response::ok(
            req.id,
            serde_json::json!({
                "schema_version": IPC_SCHEMA,
                "supported_versions": SUPPORTED_VERSIONS,
                "methods": METHODS,
                "events": [
                    EVENT_SNAPSHOT, EVENT_QUEUE_CHANGED, EVENT_STATUS_CHANGED,
                    EVENT_DIGEST_DUE, EVENT_RESYNC_REQUIRED,
                ],
                "max_line_bytes": MAX_LINE_BYTES,
            }),
        ),
        "status" => Response::ok(req.id, shared.status_value()),
        "list_pending" => {
            let queue = shared.queue.lock().expect("queue lock");
            let entries: Vec<serde_json::Value> =
                queue.pending().iter().map(|e| entry_value(e)).collect();
            Response::ok(req.id, serde_json::json!({ "pending": entries }))
        }
        // Every project the daemon knows about -- configured *and* merely
        // discovered -- with the mode actually in force for each.
        //
        // It used to report `policy.projects` alone, which meant a project
        // the daemon had seen but the contributor had never ruled on was
        // invisible here. That is precisely the set an onboarding "which of
        // these should never be uploaded" screen has to show: a project
        // becomes configured only by being ruled on, so listing only
        // configured projects lists only the ones already decided. A
        // contributor could not exclude their employer's repository before
        // anything was sent, because the screen could not name it.
        //
        // A discovered row carries `configured: false` and `added_at:
        // null`; its `mode` is the effective one, which for an unruled
        // project is the notify-only default. Nothing new crosses the
        // socket: the label and the id are the same two daemon-derived
        // fields the queue entry for that project already carries.
        //
        // `is_unresolved_bucket` marks the row holding sessions whose
        // working directory had no usable final segment. Clients show it
        // with a permanent note that these can never be armed -- which is
        // enforcement they are REPORTING, not performing: `Policy` refuses
        // `auto_upload` for this key independently of any client.
        //
        // The daemon says so explicitly because it is the only side that
        // knows it for free. A client deriving it would have to re-implement
        // `project_id_for`'s hash to compare ids, and a client matching on
        // `project_label` would break the day that string is reworded --
        // which every shell does to it, because the raw label is a slug no
        // contributor should read. Clients MUST NOT recognise this row by
        // label.
        "list_projects" => {
            let policy = shared.policy.lock().expect("policy lock");
            let queue = shared.queue.lock().expect("queue lock");
            let known = known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()));
            let discovered: std::collections::BTreeSet<String> = queue
                .all()
                .iter()
                .map(|e| e.project_key.clone())
                .filter(|key| !policy.projects.contains_key(key))
                .collect();
            let projects: Vec<serde_json::Value> = policy
                .projects
                .iter()
                .map(|(key, entry)| {
                    serde_json::json!({
                        "project_id": project_id_for(key),
                        "project_label": disambiguated_label(key, &known),
                        "mode": policy.resolve(key),
                        "added_at": entry.added_at,
                        "configured": true,
                        "is_unresolved_bucket": key == UNKNOWN_PROJECT_KEY,
                    })
                })
                .chain(discovered.iter().map(|key| {
                    serde_json::json!({
                        "project_id": project_id_for(key),
                        "project_label": disambiguated_label(key, &known),
                        "mode": policy.resolve(key),
                        "added_at": serde_json::Value::Null,
                        "configured": false,
                        "is_unresolved_bucket": key == UNKNOWN_PROJECT_KEY,
                    })
                }))
                .collect();
            Response::ok(req.id, serde_json::json!({ "projects": projects }))
        }
        // Two ways to name a project, for two different callers.
        //
        // `project_id` is for anything that learned about the project over
        // this socket -- a queue entry or a `list_projects` row. It is the
        // only identifier such a caller holds, because keys are paths and
        // paths do not cross this socket. Without it this method was
        // unreachable from every GUI: a label is not an admissible key, and
        // the only writer of `policy.projects` is this method itself, so
        // there was no way in.
        //
        // `project_key` is for a caller standing in a terminal, where the
        // human types the path: `daemon project <path> --mode ignore`. That
        // flow must keep working *before* the project's first session,
        // which is exactly when the daemon has no id to offer for it -- it
        // cannot mint one for a project it has never discovered. So both
        // are supported, deliberately, rather than one replacing the other.
        //
        // `project_id` wins when both are sent.
        "set_project_mode" => {
            let id_param = req.params.get("project_id").and_then(|v| v.as_str());
            let key_param = req.params.get("project_key").and_then(|v| v.as_str());
            if id_param.is_none() && key_param.is_none() {
                return Response::err(req.id, ERR_BAD_PARAMS, "project_id-or-project_key-required");
            }
            let mode: ProjectMode = match req
                .params
                .get("mode")
                .cloned()
                .map(serde_json::from_value::<ProjectMode>)
            {
                Some(Ok(m)) => m,
                _ => return Response::err(req.id, ERR_BAD_PARAMS, "mode-invalid"),
            };
            // A `label` param is accepted on the wire for compatibility with
            // older clients and then IGNORED. It used to be stored verbatim
            // and echoed back by `list_projects` and written into
            // `daemon-audit.jsonl`, so any socket client could inject a
            // path, a token, or a transcript fragment into both of the
            // sinks this crate's label-only rule exists to protect. The
            // label is now derived from the key inside `set_mode`.
            // Lock order is policy before queue, as everywhere else.
            let mut policy = shared.policy.lock().expect("policy lock");
            let (key, audit_label) = {
                let queue = shared.queue.lock().expect("queue lock");
                let known = known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()));
                // Whichever way the project was named, what comes out of
                // here is a key the daemon itself already holds or has just
                // corroborated on disk -- never a caller's string. That is
                // what keeps the derived label, and so `list_projects` and
                // `daemon-audit.jsonl`, un-injectable.
                let key = match id_param {
                    Some(id) => match project_key_for_id(id, &known) {
                        Some(key) => key,
                        None => {
                            return Response::err(
                                req.id,
                                ERR_BAD_PARAMS,
                                ERR_PROJECT_ID_UNRECOGNIZED,
                            );
                        }
                    },
                    None => {
                        let key = key_param.unwrap_or_default();
                        if !project_key_is_admissible(key, &known) {
                            return Response::err(
                                req.id,
                                ERR_BAD_PARAMS,
                                ERR_PROJECT_KEY_UNRECOGNIZED,
                            );
                        }
                        key.to_string()
                    }
                };
                let label = disambiguated_label(&key, &known);
                (key, label)
            };

            // The audit entry goes down FIRST, before anything is armed,
            // the way `acknowledge_near_ai_notice` does it.
            //
            // The reverse order looked equivalent and was not. It saved the
            // policy, then appended, then on an append failure restored the
            // in-memory policy and wrote it back best-effort -- but the
            // disk-full or permissions failure that broke the append breaks
            // that write back just as reliably, and the daemon loads its
            // policy from disk on restart. The fail-closed guarantee did not
            // survive a reboot: autonomy stayed armed on disk with no record
            // of it ever having been armed. Recording first means there is
            // nothing to roll back, so nothing that has to succeed twice.
            //
            // This is visibility, not a security control -- see
            // `daemon::audit` -- but it is the *only* visibility there is
            // here, and the terminal-only restriction it replaced was
            // itself a visibility mechanism.
            //
            // Both locks are dropped for the append itself: it is a
            // whole-file read-modify-write on a synchronous socket handler,
            // and the queue lock in particular is contended with the upload
            // pass. The policy lock is retaken immediately after; a
            // concurrent `set_project_mode` can only interleave two
            // record-then-arm sequences, never produce an armed policy with
            // no record.
            if mode == ProjectMode::AutoUpload {
                drop(policy);
                if let Err(_e) = audit::append(
                    &shared.store,
                    &AuditEntry {
                        at: Utc::now(),
                        action: "armed-auto-upload".to_string(),
                        project_label: Some(audit_label),
                        detail: None,
                    },
                ) {
                    return Response::err(req.id, ERR_UNAVAILABLE, "audit-write-failed");
                }
                policy = shared.policy.lock().expect("policy lock");
            }

            if let Err(e) = policy.set_mode(&key, mode, Utc::now()) {
                return Response::err(req.id, ERR_BAD_PARAMS, &one_line_label(&e.to_string()));
            }
            if let Err(_e) = policy.save(&shared.store) {
                return Response::err(req.id, ERR_UNAVAILABLE, "policy-write-failed");
            }
            // A newly-configured project can turn a previously-unique queue
            // label into a collision (or vice versa) immediately -- e.g.
            // configuring the client's "api" the moment after "api" was
            // queued bare from the contributor's own repo. Relabel now
            // rather than leaving the queue to lag until the next poll,
            // which would leave two same-basename projects briefly
            // indistinguishable in the one place uploads are approved from.
            let relabelled = {
                let mut queue = shared.queue.lock().expect("queue lock");
                if relabel_queue_entries(&policy, &mut queue) {
                    if let Err(_e) = queue.save(&shared.store) {
                        return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
                    }
                    true
                } else {
                    false
                }
            };
            drop(policy);
            if relabelled {
                shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
            }
            Response::ok(req.id, serde_json::json!({ "ok": true }))
        }
        "dismiss" => {
            let id = match parse_entry_id(&req.params) {
                Ok(id) => id,
                Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
            };
            let mut queue = shared.queue.lock().expect("queue lock");
            queue.set_state(
                id,
                QueueState::Refused,
                Some("dismissed-by-contributor".to_string()),
            );
            if let Err(_e) = queue.save(&shared.store) {
                return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
            }
            drop(queue);
            shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
            Response::ok(req.id, serde_json::json!({ "ok": true }))
        }
        "preview" => {
            // This synchronous handler cannot run the redaction pipeline (it
            // is async), so this arm reports only the entry itself,
            // honestly flagged as incomplete, rather than the raw file size
            // the old code returned. Real callers never see this: every real
            // entry point (the socket loop, and the CLI via `handle_local`)
            // runs `handle_request_async` instead, which answers `"preview"`
            // for real -- see the module doc's "Sync vs. async dispatch"
            // section.
            let id = match parse_entry_id(&req.params) {
                Ok(id) => id,
                Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
            };
            let queue = shared.queue.lock().expect("queue lock");
            match queue.get(id) {
                Some(e) => Response::ok(
                    req.id,
                    serde_json::json!({
                        "entry": entry_value(e),
                        "preview_requires_async": true,
                    }),
                ),
                None => Response::err(req.id, ERR_BAD_PARAMS, ERR_UNKNOWN_ENTRY_ID),
            }
        }
        // Resolving a body may have to run the redaction pipeline, which is
        // async. Same treatment as `"enroll"`: an honest refusal here rather
        // than a partial answer. No real caller reaches it -- see the module
        // doc's "Sync vs. async dispatch" section.
        "preview_body" => Response::err(req.id, ERR_UNAVAILABLE, "preview-body-requires-async"),
        // Waiting for a drain is async by nature; the synchronous dispatcher
        // cannot do it and says so rather than claiming a quiesce it did not
        // perform. See the module doc's "Sync vs. async dispatch" section.
        "quiesce" => Response::err(req.id, ERR_UNAVAILABLE, "quiesce-requires-async"),
        // The turn index is resolved from the same envelope as the body, by
        // the same async path, so it refuses here for the same reason.
        "preview_turns" => Response::err(req.id, ERR_UNAVAILABLE, "preview-turns-requires-async"),
        "pause" => {
            // An optional timed pause, persisted so it survives a restart of
            // either the daemon or the app that requested it -- an app-side
            // timer alone would die with the app and silently fail to
            // resume the daemon.
            let until = match req.params.get("until").and_then(|v| v.as_str()) {
                Some(s) => match s.parse::<chrono::DateTime<Utc>>() {
                    Ok(dt) if dt > Utc::now() => Some(dt),
                    // A deadline that has already passed would publish a
                    // pause event for a pause the very next status call (or
                    // is_paused check) clears -- reject it up front rather
                    // than accept a pause that is a lie the instant it's
                    // acknowledged.
                    Ok(_) => return Response::err(req.id, ERR_BAD_PARAMS, "until-in-the-past"),
                    Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "until-invalid"),
                },
                None => None,
            };
            shared.paused.store(true, Ordering::Relaxed);
            {
                let mut state = shared.state.lock().expect("state lock");
                state.paused = true;
                state.paused_until = until;
                if state.save(&shared.store).is_err() {
                    return Response::err(req.id, ERR_UNAVAILABLE, "state-write-failed");
                }
            }
            shared.publish(EVENT_STATUS_CHANGED, serde_json::json!({}));
            Response::ok(
                req.id,
                serde_json::json!({ "paused": true, "paused_until": until }),
            )
        }
        "resume" => {
            shared.paused.store(false, Ordering::Relaxed);
            {
                let mut state = shared.state.lock().expect("state lock");
                state.paused = false;
                state.paused_until = None;
                if state.save(&shared.store).is_err() {
                    return Response::err(req.id, ERR_UNAVAILABLE, "state-write-failed");
                }
            }
            shared.publish(EVENT_STATUS_CHANGED, serde_json::json!({}));
            Response::ok(req.id, serde_json::json!({ "paused": false }))
        }
        "cancel" => {
            let id = match parse_entry_id(&req.params) {
                Ok(id) => id,
                Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
            };
            let mut queue = shared.queue.lock().expect("queue lock");
            if queue.cancel(id).is_err() {
                return Response::err(req.id, ERR_BAD_PARAMS, "not-cancelable");
            }
            if let Err(_e) = queue.save(&shared.store) {
                return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
            }
            drop(queue);
            shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
            Response::ok(req.id, serde_json::json!({ "ok": true }))
        }
        "list_audit" => {
            // Same cap as `list_history`: the log is append-by-whole-file
            // rewrite and otherwise unbounded.
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .min(1000) as usize;
            match audit::load(&shared.store) {
                Ok(mut entries) => {
                    // Newest first, matching `list_history`'s convention.
                    entries.reverse();
                    entries.truncate(limit);
                    Response::ok(req.id, serde_json::json!({ "entries": entries }))
                }
                Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "audit-read-failed"),
            }
        }
        // A count of `reason_label` across every entry currently on the
        // queue, whatever its state -- no state filter is applied, it is
        // simply whichever entries currently carry a label (in practice
        // that's dismissed, refused, expired, and superseded entries, since
        // nothing else sets one). These labels are already computed by the
        // queue and uploader; this is the first surface that rolls them up.
        //
        // Deliberately NOT named `eligibility_reasons`: every source of a
        // `reason_label` applies to an entry that already exists in the
        // queue. It cannot explain the sessions an app most needs explained
        // -- ones `watcher::tick` discarded before an entry was ever
        // created, via a bare `continue` on a non-`Eligible` verdict or an
        // `Ignore`-mode project. Answering "I finished a session, why is
        // nothing pending?" needs a different, not-yet-built method; this
        // name is chosen so that one can be added later without a contract
        // break.
        "queue_outcome_counts" => {
            let queue = shared.queue.lock().expect("queue lock");
            let mut counts: std::collections::BTreeMap<&str, u64> =
                std::collections::BTreeMap::new();
            for e in queue.all() {
                if let Some(label) = e.reason_label.as_deref() {
                    *counts.entry(label).or_insert(0) += 1;
                }
            }
            Response::ok(req.id, serde_json::json!({ "reasons": counts }))
        }
        "consent_options" => Response::ok(req.id, enroll::consent_options()),
        "set_consent_scopes" => enroll::handle_set_consent_scopes(shared, req),
        "acknowledge_near_ai_notice" => enroll::handle_acknowledge_near_ai_notice(shared, req),
        // Real network I/O; only handled for real by `handle_request_async`
        // (via `handle_local`'s `block_on_ipc`, or the socket loop). See the
        // module doc's "Sync vs. async dispatch" section.
        "enroll" => Response::err(req.id, ERR_UNAVAILABLE, "enroll-requires-async"),
        "list_history" => {
            let limit = req
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .unwrap_or(50)
                .min(1000) as usize;
            match HistoryCache::load(&shared.store) {
                Ok(records) => {
                    let page: Vec<_> = records.into_iter().take(limit).collect();
                    Response::ok(req.id, serde_json::json!({ "history": page }))
                }
                Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "history-read-failed"),
            }
        }
        "history_rollup" => match HistoryCache::load(&shared.store) {
            Ok(records) => {
                let now = Utc::now();
                let mut body = serde_json::to_value(rollup(&records, now))
                    .unwrap_or_else(|_| serde_json::json!({}));
                // The public roster standing rides on this answer as an
                // additive `community` object rather than on a method of its
                // own: History is the one screen that draws it, and it
                // already asks for this. A client that ignores the field is
                // unaffected.
                //
                // No network call here. The poller
                // (`daemon::refresh_community`) owns the fetch and this
                // serves what it last cached -- and serves nothing at all
                // when there is no standing, or when the cached one has
                // aged past the roster's withdrawal bound. The field is
                // then absent rather than null-filled, because a client
                // that receives no standing must draw no section, and a
                // null-filled object is a set of claims about someone's
                // public standing that this daemon never received.
                let standing = {
                    let state = shared.state.lock().expect("state lock");
                    state.community.clone()
                };
                if let Some(standing) = standing.filter(|s| s.is_fresh(now)) {
                    if let (Some(object), Ok(value)) =
                        (body.as_object_mut(), serde_json::to_value(&standing))
                    {
                        object.insert("community".to_string(), value);
                    }
                }
                Response::ok(req.id, body)
            }
            Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "history-read-failed"),
        },
        "refresh_history" => {
            // The poller owns the network. This only asks it to run sooner,
            // and says so rather than queueing an unbounded number of asks.
            Response::ok(req.id, serde_json::json!({ "requested": true }))
        }
        "get_settings" => {
            let settings = shared.settings.lock().expect("settings lock");
            Response::ok(req.id, redacted_settings(&settings))
        }
        "set_settings" => {
            let mut settings = shared.settings.lock().expect("settings lock");
            // `apply_settings_object` is the same validation
            // `tc_daemon_start_with_settings` (the C ABI's pre-start
            // settings override) uses, so there is one definition of "a
            // valid settings object" for both. See its doc for why an
            // unrecognized key is rejected rather than ignored.
            match super::settings::apply_settings_object(&mut settings, &req.params) {
                Ok(false) => Response::err(req.id, ERR_BAD_PARAMS, "no-known-setting-supplied"),
                Ok(true) => {
                    if let Err(_e) = settings.save(&shared.store) {
                        return Response::err(req.id, ERR_UNAVAILABLE, "settings-write-failed");
                    }
                    Response::ok(req.id, redacted_settings(&settings))
                }
                Err(label) => Response::err(req.id, ERR_BAD_PARAMS, label),
            }
        }
        "shutdown" => {
            shared.shutdown.store(true, Ordering::Relaxed);
            shared.shutdown_signal.notify_one();
            Response::ok(req.id, serde_json::json!({ "stopping": true }))
        }
        // subscribe is handled by the connection loop, which owns the stream.
        "subscribe" => Response::ok(req.id, serde_json::json!({ "subscribed": true })),
        // Real network I/O when an account session exists to make the call
        // with (it never does today -- see `daemon::withdraw`'s module doc);
        // only handled for real by `handle_request_async`, same as
        // `"enroll"` above.
        "withdraw" => Response::err(req.id, ERR_UNAVAILABLE, "withdraw-requires-async"),
        "withdraw_bulk" => Response::err(req.id, ERR_UNAVAILABLE, "withdraw-requires-async"),
        // Claiming and withdrawing a public handle both call the server, so
        // like `"withdraw"` above they are only answered for real by
        // `handle_request_async`. Reading the profile back is a local cache
        // read (there is no server read-back to make -- see
        // `daemon::profile`), so it is complete here.
        "set_public_profile" => Response::err(req.id, ERR_UNAVAILABLE, "profile-requires-async"),
        "clear_public_profile" => Response::err(req.id, ERR_UNAVAILABLE, "profile-requires-async"),
        "get_public_profile" => super::profile::handle_get_public_profile(shared, req),
        _ => Response::err(req.id, ERR_UNKNOWN_METHOD, "unknown-method"),
    }
}

/// The complete dispatcher: answers the async methods (`"approve"`,
/// `"preview"`, `"preview_body"`, `"preview_turns"`, `"quiesce"`, `"enroll"`,
/// `"withdraw"`, `"withdraw_bulk"`, `"set_public_profile"`,
/// `"clear_public_profile"`) for real and delegates every other method,
/// unchanged, to the synchronous `handle_request`. See the module doc's
/// "Sync vs. async dispatch" section for why this is the only place that
/// decides which methods are async, and why both real callers (the socket
/// loop and `handle_local`) always go through this function rather than
/// `handle_request` directly.
pub async fn handle_request_async(shared: &DaemonShared, req: &Request) -> Response {
    match req.method.as_str() {
        "approve" => handle_approve(shared, req).await,
        "preview" => handle_preview(shared, req).await,
        "preview_body" => handle_preview_body(shared, req).await,
        "quiesce" => handle_quiesce(shared, req).await,
        "preview_turns" => handle_preview_turns(shared, req).await,
        "enroll" => enroll::handle_enroll(shared, req).await,
        "withdraw" => super::withdraw::handle_withdraw(shared, req).await,
        "withdraw_bulk" => super::withdraw::handle_withdraw_bulk(shared, req).await,
        "set_public_profile" => super::profile::handle_set_public_profile(shared, req).await,
        "clear_public_profile" => super::profile::handle_clear_public_profile(shared, req).await,
        _ => handle_request(shared, req),
    }
}

/// The timeout `quiesce` will actually honour.
///
/// A caller cannot park uploads for a week, and a caller that asks for zero
/// gets the default rather than an instant refusal.
fn clamp_quiesce_timeout(requested: Option<u64>) -> u64 {
    match requested {
        Some(0) | None => DEFAULT_QUIESCE_TIMEOUT_SECS,
        Some(n) => n.min(MAX_QUIESCE_TIMEOUT_SECS),
    }
}

/// Park the upload queue and wait for anything already in flight to finish.
///
/// The flag is set first, so nothing new is claimed while the wait runs, and
/// then in-flight work is allowed to complete on its own terms. On timeout
/// the flag is cleared and the caller is refused: the update stays staged and
/// retries later. There is no forced path -- a half-uploaded trace is not an
/// acceptable cost for an update.
async fn handle_quiesce(shared: &DaemonShared, req: &Request) -> Response {
    let requested = match req.params.get("timeout_secs") {
        None => None,
        Some(v) => match v.as_u64() {
            Some(n) => Some(n),
            None => return Response::err(req.id, ERR_BAD_PARAMS, "timeout-secs-invalid"),
        },
    };
    let timeout = std::time::Duration::from_secs(clamp_quiesce_timeout(requested));

    shared.quiesced.store(true, Ordering::Relaxed);
    let started = std::time::Instant::now();
    loop {
        let in_flight = {
            let queue = shared.queue.lock().expect("queue lock");
            queue.all().iter().any(|e| e.state == QueueState::Uploading)
        };
        if !in_flight {
            return Response::ok(
                req.id,
                serde_json::json!({
                    "quiesced": true,
                    "waited_ms": started.elapsed().as_millis() as u64,
                }),
            );
        }
        if started.elapsed() >= timeout {
            shared.quiesced.store(false, Ordering::Relaxed);
            return Response::err(req.id, ERR_BUSY, ERR_QUIESCE_TIMEOUT);
        }
        tokio::time::sleep(std::time::Duration::from_millis(QUIESCE_POLL_MS)).await;
    }
}

/// Run the real, async redaction pipeline for one queue entry and report the
/// actual bytes and redactions a contributor is about to consent to.
///
/// `handle_request` cannot run this (it is synchronous) and answers
/// `"preview"` on its own with an honest `preview_requires_async: true`
/// marker rather than a wrong byte count; only `handle_request_async`
/// resolves it completely.
async fn handle_approve(shared: &DaemonShared, req: &Request) -> Response {
    let all = req
        .params
        .get("all")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // Read before the queue lock is taken, so the settings lock is
    // never held under it.
    //
    // What is being approved is not just a session: it is that
    // session under the consent scopes and the
    // envelope-determining configuration in force right now. Both
    // are recorded on the entry so the uploader can refuse if
    // either moves before it sends. An approval with no readable
    // config records neither, which the uploader treats as
    // "unknown, re-ask" -- fail-closed.
    let cfg = shared.store.load_config().ok().flatten();
    let scopes = cfg
        .as_ref()
        .map(|c| c.consent_scopes.clone())
        .unwrap_or_default();
    // One instant for the whole call, so `approve: {"all": true}`
    // holds every entry it approved for the same window and reports
    // one deadline that is true of all of them -- rather than a
    // deadline that happens to describe the first entry and expires
    // early for the rest.
    let approved_at = Utc::now();
    let approval_hold_secs = shared
        .settings
        .lock()
        .expect("settings lock")
        .approval_hold_secs;
    // `None`, not `Some("")`, when there is no readable config:
    // every call site expresses "unknown" the same way, and the
    // uploader treats it as "re-ask" -- fail-closed.
    let inputs = cfg.as_ref().map(|c| {
        let near_ai = shared
            .settings
            .lock()
            .expect("settings lock")
            .near_ai
            .clone();
        super::preview::input_fingerprint(c, near_ai.as_ref())
    });
    let project_id = req.params.get("project_id").and_then(|v| v.as_str());
    // Three mutually exclusive selectors; `all` wins over `project_id` wins
    // over `entry_id` when more than one is sent -- same precedence rule as
    // `set_project_mode` above.
    let ids: Vec<Uuid> = {
        let queue = shared.queue.lock().expect("queue lock");
        if all {
            queue.pending().iter().map(|e| e.entry_id).collect()
        } else if let Some(pid) = project_id {
            // Only `Pending`: an entry already approved has had its terms
            // fixed, and a project-wide call must not silently re-pin them.
            queue
                .pending()
                .iter()
                .filter(|e| project_id_for(&e.project_key) == pid)
                .map(|e| e.entry_id)
                .collect()
        } else {
            match parse_entry_id(&req.params) {
                Ok(id) => {
                    // An id the caller never held is a client bug, not
                    // something to fold into the skip accounting below --
                    // refused up front, the same way `preview` refuses the
                    // same input, rather than reported as a labelled skip
                    // of a call that otherwise ran. `all` and `project_id`
                    // cannot reach this branch: their ids are read from
                    // the queue itself a few lines above, so every id they
                    // produce already names a real entry at the moment of
                    // selection.
                    if queue.get(id).is_none() {
                        return Response::err(req.id, ERR_BAD_PARAMS, ERR_UNKNOWN_ENTRY_ID);
                    }
                    vec![id]
                }
                Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
            }
        }
    };
    if all {
        // A local, label-only record that the whole queue was
        // bulk-approved, written BEFORE anything is approved --
        // same ordering, and the same reason, as
        // `set_project_mode`: a rollback that has to write to the
        // disk that just refused a write is not a rollback. This is
        // visibility, not a security control (see `daemon::audit`),
        // but it is the only visibility there is for a call that
        // used to require a terminal.
        //
        // Written before the envelope builds below, not merely before
        // the approve loop: those builds persist artifacts of their
        // own, and "the record could not be written, so nothing
        // happened" has to stay true of everything this call does.
        //
        // The count is of entries eligible to be approved when the
        // queue was read. It is an upper bound: an entry no artifact
        // can be built for is skipped below, and `approve` can refuse
        // one that moved. The record says the whole queue was
        // bulk-approved at all, which is what it exists for.
        if let Err(_e) = audit::append(
            &shared.store,
            &AuditEntry {
                at: Utc::now(),
                action: "bulk-approved".to_string(),
                project_label: None,
                detail: Some(ids.len().to_string()),
            },
        ) {
            return Response::err(req.id, ERR_UNAVAILABLE, "audit-write-failed");
        }
    }
    // Entries nobody previewed have no artifact behind them. Build one now.
    //
    // What is at stake if this is not done, or is done and does not stick:
    // an entry with no pin is not refused at upload. `approved_envelope_for`
    // returns `Ok(None)` for a missing pin, and `submit` treats `None` as
    // "build one" -- so the uploader silently constructs a fresh envelope
    // and sends it. Approving from the tray without this would mean sending
    // bytes no contributor was ever shown, reported back as a success.
    //
    // The build is async and must not run under the queue lock, so the
    // entries are cloned out under a short lock of their own and the lock
    // is retaken for the approve loop below.
    let unpinned: Vec<(Uuid, super::queue::QueueEntry)> = {
        let queue = shared.queue.lock().expect("queue lock");
        ids.iter()
            .filter_map(|id| queue.all().iter().find(|e| e.entry_id == *id))
            .filter(|e| e.previewed_envelope_digest.is_none())
            .map(|e| (e.entry_id, e.clone()))
            .collect()
    };
    // Fixed labels, one per entry that could not be given an artifact.
    // Nothing here is approved: sending something the contributor was never
    // shown is worse than a refusal they can see.
    let mut skipped: Vec<(Uuid, &'static str)> = Vec::new();
    // What the response's toast is built from: redaction counts summed by
    // category across every entry approve itself built a preview for (an
    // entry that was already previewed before this call contributes
    // nothing here -- its preview response already told the caller this),
    // and how many of those builds carried a PII label. Counts and labels
    // only, per the hash-only rule -- never the text a redaction removed.
    let mut redactions: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let mut flagged: u64 = 0;
    for (id, entry) in unpinned {
        // An unenrolled build is never pinned -- it is a placeholder
        // -identity artifact, not the one an upload would send -- so there
        // is nothing for approve to do with it. This is an optimisation of
        // the pin re-check below, which would skip such an entry anyway,
        // and it exists only so the caller gets the specific label rather
        // than a generic one.
        if cfg.is_none() {
            skipped.push((id, "not-enrolled"));
            continue;
        }
        match build_and_pin_preview(shared, id, &entry, cfg.as_ref()).await {
            Ok((summary, _body, _envelope)) => {
                // `build_preview` does not size-check the raw contribution
                // (only `submit`'s path does); `approved_envelope::save`
                // does, on exactly this measurement. Checked here, before
                // the pin is even attempted, so an oversized session gets
                // its own label instead of the generic `not-pinned` the
                // pin re-check below would otherwise give it -- this one
                // is permanent, and retrying the same session can never
                // succeed, which `not-pinned`'s other causes do not imply.
                if summary.would_send_bytes > crate::envelope::MAX_ENVELOPE_BYTES {
                    skipped.push((id, "envelope-too-large"));
                    continue;
                }
                for (category, count) in &summary.redactions {
                    *redactions.entry(category.clone()).or_insert(0) += count;
                }
                if !summary.pii_labels_present.is_empty() {
                    flagged += 1;
                }
            }
            Err((_code, label)) => skipped.push((id, label)),
        }
    }
    let skipped_ids: std::collections::HashSet<Uuid> = skipped.iter().map(|(id, _)| *id).collect();
    let mut queue = shared.queue.lock().expect("queue lock");
    let mut approved_ids = Vec::new();
    for id in &ids {
        let id = *id;
        if skipped_ids.contains(&id) {
            continue;
        }
        // The pin is re-checked here, under the lock that approves, rather
        // than inferred from the build returning `Ok`. `build_and_pin_preview`
        // is `Ok` whenever the *build* succeeded, and `pin_previewed_envelope`
        // declines silently when the envelope could not be written, when the
        // queue write failed, or when the entry left `Pending` while the
        // build was running. Trusting `Ok` would approve an entry with no
        // artifact behind it, which is the one thing the uploader does not
        // catch. An entry that is still unpinned here is left `Pending` for
        // the contributor to approve again.
        if queue
            .get(id)
            .is_none_or(|e| e.previewed_envelope_digest.is_none())
        {
            // The build reported `Ok` (or this entry had a stale pin to
            // begin with) but nothing is pinned now: `pin_previewed_envelope`
            // declined to write because the queue write failed or the
            // entry left `Pending` while the build was running, or (for
            // `all`/`project_id`) the entry was removed from the queue
            // entirely in that same window. All three are transient --
            // unlike `envelope-too-large` above, retrying is expected to
            // work once the race that caused this has passed. Either way
            // the entry is left `Pending`, same as any other unpinned
            // entry -- but it must not vanish from the response. An entry
            // counted in neither `approved` nor `skipped` is exactly the
            // silent hole the pin re-check above this loop exists to
            // close for the uploader; the caller deserves the same
            // guarantee.
            skipped.push((id, "not-pinned"));
            continue;
        }
        if queue.approve(id, &scopes, inputs.as_deref(), Some(approved_at)) {
            approved_ids.push(id);
        } else {
            // `Queue::approve` refuses anything not `Pending`, and this
            // entry just passed the pin check above under the same held
            // lock -- so it exists and nothing else can have touched it
            // since. The only way it still lands here is that its state
            // was already something other than `Pending` when this call
            // started: `previewed_envelope_digest` is never cleared by
            // `approve` or by the terminal states `cancel` moves an entry
            // through, so an entry that was approved (or otherwise moved
            // off `Pending`) earlier keeps looking pinned forever. The
            // deterministic repro is approving the same `entry_id` twice
            // in a row. Reported, not dropped, for the same reason as
            // `not-pinned`: an id this call was asked to act on must show
            // up somewhere in the response.
            skipped.push((id, "not-pending"));
        }
    }
    let approved = approved_ids.len();
    // The deadline the daemon will actually honour, taken from an
    // entry it just wrote rather than recomputed here, so a client
    // counting down against it is counting down against the same
    // value `drain_approved` compares. `null` when nothing was
    // approved or the hold is configured off -- a client must then
    // offer no undo, rather than invent one.
    let hold_until = approved_ids
        .first()
        .and_then(|id| queue.get(*id))
        .and_then(|e| e.hold_until(approval_hold_secs));
    if let Err(_e) = queue.save(&shared.store) {
        // The approvals exist only in memory and would not survive a
        // restart; a queue that disagrees with its own file is worse
        // than no approval. `cancel` refuses anything past
        // `Approved`, and these were set `Approved` a few lines ago
        // under this same lock, so no upload pass can have claimed
        // one.
        for id in approved_ids {
            let _ = queue.cancel(id);
        }
        return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
    }
    drop(queue);
    shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
    // The signal the contributor sees instead of a preview: "Sent --
    // scrubbing removed N things, M flagged." Counts and labels only -- a
    // redaction count names a category, never the text it removed, and a
    // skip reason is a fixed label, never a path or trace content.
    Response::ok(
        req.id,
        serde_json::json!({
            "approved": approved,
            "hold_secs": approval_hold_secs,
            "hold_until": hold_until,
            "flagged": flagged,
            "redactions": redactions,
            "skipped": skipped
                .iter()
                .map(|(id, label)| serde_json::json!({
                    "entry_id": id,
                    "reason_label": label,
                }))
                .collect::<Vec<_>>(),
        }),
    )
}

async fn handle_preview(shared: &DaemonShared, req: &Request) -> Response {
    let id = match parse_entry_id(&req.params) {
        Ok(id) => id,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    let entry = {
        let queue = shared.queue.lock().expect("queue lock");
        match queue.get(id) {
            Some(e) => e.clone(),
            None => return Response::err(req.id, ERR_BAD_PARAMS, ERR_UNKNOWN_ENTRY_ID),
        }
    };
    // No enrollment is not a refusal. Preview does no network I/O and needs
    // neither the daemon's lock nor its running loop, so requiring a config
    // here was incidental -- and it forced anyone who wanted to *see* what
    // would be sent to enrol first, which is the wrong way round. Without a
    // config the pipeline builds the same placeholder-identity,
    // deterministic-only envelope the CLI's unenrolled `--dry-run` builds,
    // and the response says so. See `preview::build_preview`.
    let cfg = shared.store.load_config().ok().flatten();

    match build_and_pin_preview(shared, id, &entry, cfg.as_ref()).await {
        Ok((summary, _body, _envelope)) => {
            Response::ok(
                req.id,
                serde_json::json!({
                    "entry": entry_value(&entry),
                    "would_send_bytes": summary.would_send_bytes,
                    "raw_session_bytes": summary.raw_session_bytes,
                    "event_count": summary.event_count,
                    "opening_prompt": summary.opening_prompt,
                    "redactions": summary.redactions,
                    "pii_labels_present": summary.pii_labels_present,
                    "consent_scopes": summary.consent_scopes,
                    "residual_risk": summary.residual_risk,
                    // Hashes, not content: what the contributor is being shown,
                    // and the configuration that produced it. An app can hold
                    // these to confirm the entry it later approves is the one
                    // it displayed.
                    "envelope_digest": summary.envelope_digest,
                    "input_fingerprint": summary.input_fingerprint,
                    // False when this device is not enrolled: the summary
                    // describes a placeholder-identity, deterministic-only
                    // build, and neither hash above is bindable to a later
                    // approval.
                    "enrolled": summary.enrolled,
                }),
            )
        }
        Err((code, label)) => Response::err(req.id, code, label),
    }
}

/// Build the redacted envelope for one queue entry, pin the entry to it, and
/// hand back the summary, the redacted body, and the envelope.
///
/// The one place the preview pipeline is driven. `handle_preview` (the
/// socket's summary), `open_preview` (the C ABI's in-process full preview),
/// and `handle_preview_body` (the socket's body, when there is no stored
/// envelope to read instead) all go through it, so the summary one surface
/// reports and the body another returns always describe the same build.
/// Errors are `(code, fixed label)` -- no path, no entry content -- and the
/// callers that need a bare label discard the code.
async fn build_and_pin_preview(
    shared: &DaemonShared,
    entry_id: Uuid,
    entry: &super::queue::QueueEntry,
    cfg: Option<&crate::config::ContributorConfig>,
) -> Result<
    (
        super::preview::PreviewSummary,
        String,
        trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
    ),
    (&'static str, &'static str),
> {
    let (near_ai, claude_source, codex_source) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.near_ai.clone(),
            s.claude_source.clone(),
            s.codex_source.clone(),
        )
    };
    let sources = crate::source::all_sources(claude_source, codex_source, None);
    let (source, session_ref) =
        super::find_session(&sources, entry).ok_or((ERR_BAD_PARAMS, "session-file-vanished"))?;
    let (summary, body, envelope) =
        super::preview::build_preview(&shared.store, cfg, near_ai, source, &session_ref)
            .await
            .map_err(|_| (ERR_UNAVAILABLE, "preview-failed"))?;
    // An unenrolled preview is never pinned: it was built from a placeholder
    // identity, so it is not the artifact any later approval would send.
    if summary.enrolled {
        pin_previewed_envelope(shared, entry_id, &summary, &envelope);
    }
    Ok((summary, body, envelope))
}

/// Full preview -- summary *and* redacted body -- for one queue entry, for a
/// caller that already holds `shared` directly rather than issuing a
/// request/response frame. This is what the C ABI's `tc_preview_open` uses.
///
/// A socket client reaches the same body through `"preview_body"`
/// (`handle_preview_body`, below), which pages it under the 1 MiB line cap.
/// That method exists because this function's `&DaemonShared` is only
/// available to the process holding the daemon lock -- which, on a
/// systemd-hosted daemon with the window as a socket client, is never the
/// window. Errors are fixed labels, matching every other surface at this
/// boundary -- no path, no entry content.
pub async fn open_preview(
    shared: &DaemonShared,
    entry_id: Uuid,
) -> Result<(super::preview::PreviewSummary, String), &'static str> {
    let entry = {
        let queue = shared.queue.lock().expect("queue lock");
        queue.get(entry_id).cloned().ok_or(ERR_UNKNOWN_ENTRY_ID)?
    };
    // As with the socket's `"preview"`: no enrollment yields a
    // placeholder-identity, deterministic-only preview rather than a
    // refusal, and `summary.enrolled` says which one this is.
    let cfg = shared.store.load_config().map_err(|_| "not-logged-in")?;
    // Same pinning as the socket's `"preview"`: the entry now holds the
    // artifact this caller was shown, so an approval that follows covers
    // that artifact and nothing else.
    let (summary, body, _envelope) = build_and_pin_preview(shared, entry_id, &entry, cfg.as_ref())
        .await
        .map_err(|(_code, label)| label)?;
    Ok((summary, body))
}

/// The redacted preview body for one queue entry, over the socket, in pages.
///
/// # Why this exists
///
/// `open_preview` needs `&DaemonShared`, so only the process holding the
/// daemon lock can call it. On the recommended Linux arrangement the daemon
/// is a systemd unit and the window is a socket client, so the window is
/// never that process: without this method its "search" and "exactly what
/// would be sent" surfaces cannot work at all. Search in particular is the
/// affordance that lets a contributor under an NDA check in seconds whether
/// a trace names their client, and it was dead on the platform's primary
/// deployment.
///
/// # Paging, and why the body is not searched here
///
/// A redacted envelope may approach `MAX_ENVELOPE_BYTES`, above the 1 MiB
/// `MAX_LINE_BYTES` frame, so the body is paged: `offset` in, `chunk` plus
/// `next_offset` out, `next_offset: null` at the end. Nothing is ever
/// silently truncated -- a client that believed it had searched a whole
/// trace when it had searched the first megabyte would report a confident,
/// false "0 matches", which is the exact failure this affordance exists to
/// prevent.
///
/// The daemon ships the body and does not search it. A server-side matcher
/// would have to reproduce the client's own notion of a match (case folding,
/// word boundaries, how an event boundary is spanned) and would still have
/// to ship surrounding text for the client to render, so the client would
/// end up holding the body anyway -- but with a second matcher to keep in
/// step with the one displaying results. One body, one text, one search:
/// what the contributor searched is what the contributor is looking at.
///
/// The property that must survive either choice is that **a client can never
/// report a trace clean when it could not actually look**, and paging is the
/// only thing that could quietly break it. Two things hold it up: the client
/// is told `total_bytes` and can refuse to report a result until it has
/// received `[0, total_bytes)`, and a continuation page must carry the
/// `body_digest` of the page it continues. A body that changed underneath a
/// paging client is refused with [`ERR_PREVIEW_BODY_CHANGED`] rather than
/// spliced -- which matters because a rebuild is not reproducible: event ids
/// are minted per build, and under an LLM-backed privacy filter the
/// redaction spans move too.
///
/// # Where the body comes from
///
/// A previewed, pinned entry has its envelope on disk, and that stored
/// artifact is what is read -- the same bytes the upload will send, so
/// paging is stable across calls and identical to what `open_preview`
/// returns. Only an entry with no stored envelope runs the pipeline (which
/// pins it, exactly as `preview` does). An entry that *is* pinned but whose
/// bytes are missing or unusable is refused with
/// `approved-envelope-unavailable` rather than rebuilt: a rebuild would show
/// a contributor something other than what they approved.
///
/// Trace content, under the preview exemption in this module's doc: only for
/// an entry the caller already holds, post-redaction only, and never onward.
async fn handle_preview_body(shared: &DaemonShared, req: &Request) -> Response {
    let id = match parse_entry_id(&req.params) {
        Ok(id) => id,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    let offset = match req.params.get("offset") {
        None => 0usize,
        Some(v) => match v.as_u64() {
            Some(n) => n as usize,
            None => return Response::err(req.id, ERR_BAD_PARAMS, "offset-invalid"),
        },
    };
    let limit = match req.params.get("limit") {
        None => MAX_PREVIEW_BODY_CHUNK_BYTES,
        Some(v) => match v.as_u64() {
            // A larger ask is capped, not refused: the cap is a framing
            // limit, and a client that asks for the whole body in one go is
            // making a reasonable request the transport cannot grant.
            Some(n) if n > 0 => (n as usize).min(MAX_PREVIEW_BODY_CHUNK_BYTES),
            _ => return Response::err(req.id, ERR_BAD_PARAMS, "limit-invalid"),
        },
    };
    let expected_digest = match req.params.get("body_digest") {
        None => None,
        Some(v) => match v.as_str() {
            Some(s) => Some(s.to_string()),
            None => return Response::err(req.id, ERR_BAD_PARAMS, "body-digest-invalid"),
        },
    };
    // Fail-closed rather than best-effort: an unanchored continuation is
    // indistinguishable from a continuation of a body that no longer exists.
    if offset > 0 && expected_digest.is_none() {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_BODY_DIGEST_REQUIRED);
    }

    let (body, envelope_digest, enrolled) = match resolve_preview_body(shared, id).await {
        Ok(v) => v,
        Err((code, label)) => return Response::err(req.id, code, label),
    };
    let body_digest = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
    if let Some(expected) = expected_digest {
        if expected != body_digest {
            return Response::err(req.id, ERR_UNAVAILABLE, ERR_PREVIEW_BODY_CHANGED);
        }
    }

    let total = body.len();
    if offset > total || !body.is_char_boundary(offset) {
        return Response::err(req.id, ERR_BAD_PARAMS, "offset-invalid");
    }
    let mut end = offset.saturating_add(limit).min(total);
    // The body is UTF-8 and `chunk` is a JSON string, so a page may not
    // split a character. Walk the end down to a boundary; if that leaves no
    // progress at all (a `limit` smaller than the character it lands in),
    // walk up instead, so a paging client can never stall.
    while end > offset && !body.is_char_boundary(end) {
        end -= 1;
    }
    if end == offset && offset < total {
        end = offset + 1;
        while end < total && !body.is_char_boundary(end) {
            end += 1;
        }
    }
    let next_offset = (end < total).then_some(end);

    Response::ok(
        req.id,
        serde_json::json!({
            "entry_id": id,
            "total_bytes": total,
            "offset": offset,
            "chunk": &body[offset..end],
            "next_offset": next_offset,
            // The token that anchors the next page to this body, and the
            // digest of the envelope the body came from -- the same value
            // `preview` reports, so an app can tie the body it is showing to
            // the summary it displayed.
            "body_digest": body_digest,
            "envelope_digest": envelope_digest,
            "enrolled": enrolled,
            "max_chunk_bytes": MAX_PREVIEW_BODY_CHUNK_BYTES,
        }),
    )
}

/// An index of the turns in the redacted preview body: where each one starts
/// inside the body and what to label it. **An overlay, never a replacement.**
///
/// # Why this is an index and not a rendered transcript
///
/// The transcript surface a contributor approves from is titled "exactly
/// what would be sent", and that is meant literally: what it shows is
/// `preview_body`'s bytes, the same bytes the upload sends. Re-rendering
/// those events as prose turns would drop everything that has no prose form
/// -- `structured_payload`, `token_counts`, `latency_ms`, `cost_usd`,
/// `failure_modes` -- and so would show *less* than the artifact under a
/// heading promising the whole of it. So the daemon does not re-render. It
/// says where the turns begin in the body the client already has, and the
/// client draws separators there over text it renders verbatim.
///
/// `preview::turns_of` computes the offsets from `preview::body_of`'s own
/// output, so there is exactly one definition of how events map to bytes,
/// and one test asserts each span re-parses to the event it claims.
///
/// # Anchoring
///
/// `body_digest` is **required**, on the first call and every call, and is
/// the same anchoring rule `preview_body`'s continuation pages use. An index
/// is a set of offsets into a specific string; against any other string it
/// is not merely stale but wrong, and wrong in the invisible way -- a
/// separator drawn over the wrong text still looks like a transcript. A
/// rebuilt envelope is a different artifact (event ids are minted per build,
/// and an LLM-backed privacy filter does not reproduce its own spans), so a
/// mismatch is refused with [`ERR_PREVIEW_BODY_CHANGED`] exactly as a
/// mis-anchored page is, and the correct response is the same: re-read the
/// body from `offset: 0` and ask again with the digest it returns.
///
/// # Framing
///
/// Unpaged, and it fits: a turn serializes to well under 100 bytes, and an
/// envelope is capped at `MAX_ENVELOPE_BYTES` (1.5 MB) while one
/// pretty-printed event costs upwards of 170 of those bytes, so the index
/// stays a fraction of the 1 MiB line cap even for an envelope at the
/// ceiling. If that ceiling ever rises materially, this has to page the way
/// `preview_body` does rather than truncate -- a truncated index is a
/// transcript with turns silently missing from the end.
///
/// The index itself carries no redacted trace text -- an event-type label,
/// the tool name the envelope already records as metadata, and byte offsets.
/// It is still only served for an entry the caller already holds, under the
/// same rule as the rest of the preview surface, because the shape of a
/// transcript is itself something a contributor has not offered anyone.
async fn handle_preview_turns(shared: &DaemonShared, req: &Request) -> Response {
    let id = match parse_entry_id(&req.params) {
        Ok(id) => id,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    let expected_digest = match req.params.get("body_digest") {
        // Fail-closed, and required from the first call: an index is only
        // meaningful against the body the caller is holding.
        None => return Response::err(req.id, ERR_BAD_PARAMS, ERR_BODY_DIGEST_REQUIRED),
        Some(v) => match v.as_str() {
            Some(s) => s.to_string(),
            None => return Response::err(req.id, ERR_BAD_PARAMS, "body-digest-invalid"),
        },
    };

    let (envelope, envelope_digest, _enrolled) = match resolve_preview_envelope(shared, id).await {
        Ok(v) => v,
        Err((code, label)) => return Response::err(req.id, code, label),
    };
    let body = match super::preview::body_of(&envelope) {
        Ok(b) => b,
        Err(_) => return Response::err(req.id, ERR_UNAVAILABLE, "preview-failed"),
    };
    let body_digest = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
    if expected_digest != body_digest {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_PREVIEW_BODY_CHANGED);
    }
    let turns = match super::preview::turns_of(&envelope) {
        Ok(t) => t,
        Err(_) => {
            return Response::err(
                req.id,
                ERR_UNAVAILABLE,
                super::preview::REASON_TURN_INDEX_FAILED,
            );
        }
    };

    Response::ok(
        req.id,
        serde_json::json!({
            "entry_id": id,
            "body_digest": body_digest,
            "envelope_digest": envelope_digest,
            "turn_count": turns.len(),
            "turns": turns,
        }),
    )
}

/// The turn index for one entry, for a caller that already holds `shared`
/// directly rather than issuing a request/response frame -- the C ABI's
/// `tc_preview_turns_json`. Anchored by the same rule as the socket method:
/// the caller passes the digest of the body it is showing, and a body that
/// is not that one is refused rather than indexed.
///
/// Returns the same JSON object `"preview_turns"` puts in its `result`, so
/// the two surfaces cannot describe the same entry differently.
pub async fn open_preview_turns(
    shared: &DaemonShared,
    entry_id: Uuid,
    expected_body_digest: &str,
) -> Result<String, &'static str> {
    let (envelope, envelope_digest, _enrolled) = resolve_preview_envelope(shared, entry_id)
        .await
        .map_err(|(_code, label)| label)?;
    let body = super::preview::body_of(&envelope).map_err(|_| "preview-failed")?;
    let body_digest = format!("sha256:{:x}", Sha256::digest(body.as_bytes()));
    if expected_body_digest != body_digest {
        return Err(ERR_PREVIEW_BODY_CHANGED);
    }
    let turns = super::preview::turns_of(&envelope)
        .map_err(|_| super::preview::REASON_TURN_INDEX_FAILED)?;
    serde_json::to_string(&serde_json::json!({
        "entry_id": entry_id,
        "body_digest": body_digest,
        "envelope_digest": envelope_digest,
        "turn_count": turns.len(),
        "turns": turns,
    }))
    .map_err(|_| "turns-serialize-failed")
}

/// The redacted body for one entry, plus its envelope digest and whether the
/// build behind it was an enrolled one. See `handle_preview_body` for which
/// of the two sources is used and why.
async fn resolve_preview_body(
    shared: &DaemonShared,
    entry_id: Uuid,
) -> Result<(String, String, bool), (&'static str, &'static str)> {
    let (envelope, digest, enrolled) = resolve_preview_envelope(shared, entry_id).await?;
    let body =
        super::preview::body_of(&envelope).map_err(|_| (ERR_UNAVAILABLE, "preview-failed"))?;
    Ok((body, digest, enrolled))
}

/// The redacted envelope one preview surface is describing, resolved once so
/// the body and the turn index over it can never come from two different
/// builds. `handle_preview_body` documents which of the two sources is used
/// and why a pinned-but-missing artifact is refused rather than rebuilt.
async fn resolve_preview_envelope(
    shared: &DaemonShared,
    entry_id: Uuid,
) -> Result<
    (
        trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
        String,
        bool,
    ),
    (&'static str, &'static str),
> {
    let entry = {
        let queue = shared.queue.lock().expect("queue lock");
        queue
            .get(entry_id)
            .cloned()
            .ok_or((ERR_BAD_PARAMS, ERR_UNKNOWN_ENTRY_ID))?
    };
    match super::approved_envelope::load(&shared.store, entry_id) {
        Ok(Some(envelope)) => {
            let digest = super::preview::envelope_digest(&envelope)
                .map_err(|_| (ERR_UNAVAILABLE, "preview-failed"))?;
            // Only an enrolled preview is ever stored.
            Ok((envelope, digest, true))
        }
        // Pinned, but the bytes are not there. Refuse rather than rebuild:
        // a rebuild is a different artifact from the one this entry is
        // pinned to, and showing it as "what would be sent" would be false.
        Ok(None) if entry.previewed_envelope_digest.is_some() => Err((
            ERR_UNAVAILABLE,
            super::preview::REASON_APPROVED_ENVELOPE_UNAVAILABLE,
        )),
        Ok(None) => {
            let cfg = shared.store.load_config().ok().flatten();
            let (summary, _body, envelope) =
                build_and_pin_preview(shared, entry_id, &entry, cfg.as_ref()).await?;
            Ok((envelope, summary.envelope_digest, summary.enrolled))
        }
        Err(_) => Err((
            ERR_UNAVAILABLE,
            super::preview::REASON_APPROVED_ENVELOPE_UNAVAILABLE,
        )),
    }
}

/// Store the redacted envelope a preview just built and pin the entry to
/// it, so an upload that follows sends exactly those bytes rather than
/// building a second envelope.
///
/// The order matters and is the whole contract: the bytes go down first,
/// and the entry is only pinned once they are on disk. "Pinned" is what the
/// uploader reads as "the approved bytes exist"; a pin recorded without
/// them turns an ordinary upload into a fail-closed re-offer.
///
/// Best effort, deliberately. A preview is a read, and a state directory
/// that cannot take the write should still let the contributor *see* what
/// would be sent. An unpinned entry falls back to the pipeline building the
/// envelope at upload time under the input fingerprint the approval
/// records -- which is where every entry stood before any of this existed,
/// and is still fail-closed. It is never no check at all.
///
/// The queue lock is held across both writes so an entry cannot change
/// state underneath them. Previewing an entry that is no longer `Pending`
/// must not touch the stored bytes at all: an already-approved entry is
/// pinned to the artifact it was approved as, and overwriting or deleting
/// that would revoke a live approval for no reason.
fn pin_previewed_envelope(
    shared: &DaemonShared,
    entry_id: Uuid,
    summary: &super::preview::PreviewSummary,
    envelope: &trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
) {
    let mut queue = shared.queue.lock().expect("queue lock");
    if queue.get(entry_id).map(|e| e.state) != Some(QueueState::Pending) {
        return;
    }
    if super::approved_envelope::save(&shared.store, entry_id, envelope).is_err() {
        return;
    }
    if queue.record_previewed_envelope(entry_id, &summary.envelope_digest) {
        // A failed queue write leaves the pin in memory and the bytes on
        // disk -- consistent with each other, and the next queue save
        // persists it. Nothing is removed here: the bytes are what the
        // in-memory pin refers to.
        let _ = queue.save(&shared.store);
    }
}

/// Settings as returned over IPC: the privacy-filter credential is reported
/// as present or absent, never echoed.
fn redacted_settings(s: &DaemonSettings) -> serde_json::Value {
    let mut v = serde_json::to_value(s).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = v.as_object_mut() {
        let configured = s.near_ai.is_some();
        obj.remove("near_ai");
        obj.insert(
            "near_ai_configured".to_string(),
            serde_json::Value::Bool(configured),
        );
        // claude_root / codex_root are local filesystem paths. entry_value
        // is scrupulous about never putting a path on the wire; this
        // serialized-wholesale settings blob was not, and leaked one
        // whenever either root was overridden from the conventional
        // location. Report presence only.
        //
        // `*_root_configured` stays true only for a source pointed at a
        // folder. A source declared OFF is answered but has no folder, so
        // reporting it as configured would tell a settings screen to print
        // "sessions folder set" about an agent the contributor said they do
        // not use. The mode carries that distinction, and carries no path.
        let mode_of = |d: &Option<crate::daemon::settings::SourceDeclaration>| match d {
            Some(crate::daemon::settings::SourceDeclaration::Watch { .. }) => "watch",
            Some(crate::daemon::settings::SourceDeclaration::Off) => "off",
            None => "unset",
        };
        let claude_mode = mode_of(&s.claude_source);
        let codex_mode = mode_of(&s.codex_source);
        obj.remove("claude_root");
        obj.remove("codex_root");
        obj.remove("claude_source");
        obj.remove("codex_source");
        obj.insert(
            "claude_root_configured".to_string(),
            serde_json::Value::Bool(claude_mode == "watch"),
        );
        obj.insert(
            "codex_root_configured".to_string(),
            serde_json::Value::Bool(codex_mode == "watch"),
        );
        obj.insert(
            "claude_source_mode".to_string(),
            serde_json::Value::String(claude_mode.to_string()),
        );
        obj.insert(
            "codex_source_mode".to_string(),
            serde_json::Value::String(codex_mode.to_string()),
        );
    }
    v
}

fn parse_entry_id(params: &serde_json::Value) -> Result<Uuid, &'static str> {
    params
        .get("entry_id")
        .and_then(|v| v.as_str())
        .ok_or("entry_id-required")?
        .parse()
        .map_err(|_| "entry_id-invalid")
}

/// Collapse an internal error string to a single-line label. Nothing
/// multi-line or free-form crosses the socket.
fn one_line_label(s: &str) -> String {
    s.split_whitespace()
        .take(6)
        .collect::<Vec<_>>()
        .join("-")
        .replace(':', "")
}

/// The kernel's limit on a unix socket path, conservatively the smallest of
/// the common values (macOS allows 104 bytes, Linux 108).
#[cfg(unix)]
const MAX_SOCKET_PATH_BYTES: usize = 104;

/// Bind the daemon socket, refusing unless the state directory is private.
#[cfg(unix)]
pub async fn bind(store: &ConfigStore) -> Result<UnixListener> {
    ensure_private_dir(store.dir())?;
    let path = store.daemon_path(DAEMON_SOCK_FILE);

    // The kernel truncates rather than explains, and the resulting error names
    // a constant most people have never heard of. Say what is actually wrong
    // and what to do about it.
    // The message names the length and the fix, but not the path: this
    // error is returned to `daemon run`, which under a service manager
    // writes it to the journal, and a state-directory path there carries
    // the OS username. The length plus the file name is enough to act on.
    let len = path.as_os_str().len();
    if len >= MAX_SOCKET_PATH_BYTES {
        bail!(
            "the daemon socket path is {len} bytes, over the {MAX_SOCKET_PATH_BYTES}-byte \
             kernel limit for unix sockets (the path is your state directory plus \
             {DAEMON_SOCK_FILE}). Use a shorter state directory, \
             e.g. TRACE_COMMONS_CONTRIBUTOR_DIR=~/.config/trace-commons"
        );
    }

    // A socket left behind by a crashed daemon would block binding. The
    // single-instance lock, not this file, is what prevents two daemons.
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(&path).context("binding the daemon socket in the state directory")
}

/// The 0700 directory is the access control for the socket, because
/// `UnixListener::bind` does not portably apply a mode to the socket itself.
///
/// Checking the mode is sufficient: a directory at 0700 belonging to someone
/// else is not writable by this process, so binding a socket inside it fails
/// on its own. Only a directory that is both ours and private gets served.
#[cfg(unix)]
fn ensure_private_dir(dir: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta =
        std::fs::metadata(dir).context("reading permissions of the daemon state directory")?;
    let mode = meta.permissions().mode() & 0o777;
    if mode != 0o700 {
        bail!(
            "refusing to serve from a state directory that is not 0700 \
             (found {mode:o}): the directory is the only access control on \
             the daemon socket"
        );
    }
    Ok(())
}

/// Serve connections until shutdown is requested.
#[cfg(unix)]
pub async fn serve(listener: UnixListener, shared: Arc<DaemonShared>) -> Result<()> {
    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(pair) => pair,
            Err(_) => continue,
        };
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            let _ = serve_connection(stream, shared).await;
        });
    }
}

/// Serve one client connection.
///
/// Generic over the stream so the unix-socket and Windows named-pipe
/// transports share one implementation: the protocol, the framing, and the
/// error taxonomy are identical on both, and three applications are built
/// against one contract document. Only the listening and connecting ends
/// differ per platform.
pub async fn serve_connection<S>(stream: S, shared: Arc<DaemonShared>) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);
    let mut subscription: Option<broadcast::Receiver<Event>> = None;
    let mut line = String::new();

    loop {
        line.clear();
        tokio::select! {
            read = reader.read_line(&mut line) => {
                let n = match read {
                    Ok(0) => return Ok(()),
                    Ok(n) => n,
                    Err(_) => return Ok(()),
                };
                if n > MAX_LINE_BYTES {
                    let resp = Response::err(0, ERR_BAD_PARAMS, "line-too-long");
                    write_json(&mut write_half, &resp).await?;
                    return Ok(());
                }
                let req: Request = match serde_json::from_str(line.trim()) {
                    Ok(r) => r,
                    Err(_) => {
                        let resp = Response::err(0, ERR_BAD_PARAMS, "malformed-request");
                        write_json(&mut write_half, &resp).await?;
                        return Ok(());
                    }
                };
                let is_subscribe = req.method == "subscribe";
                let resp = handle_request_async(&shared, &req).await;
                write_json(&mut write_half, &resp).await?;
                if is_subscribe && resp.error.is_none() {
                    // Snapshot first, so an application never has to race the
                    // event stream against a separate list call.
                    subscription = Some(shared.events.subscribe());
                    let snap = Event {
                        event: EVENT_SNAPSHOT.to_string(),
                        data: shared.snapshot_value(),
                    };
                    write_json(&mut write_half, &snap).await?;
                }
            }
            event = async {
                match subscription.as_mut() {
                    Some(rx) => rx.recv().await,
                    // No subscription: park forever rather than spinning.
                    None => std::future::pending().await,
                }
            } => {
                match event {
                    Ok(ev) => write_json(&mut write_half, &ev).await?,
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        let ev = Event {
                            event: EVENT_RESYNC_REQUIRED.to_string(),
                            data: serde_json::json!({}),
                        };
                        write_json(&mut write_half, &ev).await?;
                    }
                    Err(broadcast::error::RecvError::Closed) => return Ok(()),
                }
            }
        }
    }
}

async fn write_json<W, T>(w: &mut W, value: &T) -> Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    let mut body = serde_json::to_vec(value).context("serializing ipc frame")?;
    body.push(b'\n');
    w.write_all(&body).await.context("writing ipc frame")?;
    w.flush().await.context("flushing ipc frame")?;
    Ok(())
}

/// Convenience for the CLI, which drives the same handlers in-process.
///
/// Every method, not only the async ones, is answered through
/// `handle_request_async` via `block_on_ipc` -- see the module doc's "Sync
/// vs. async dispatch" section for why routing everything through the one
/// real dispatcher, rather than special-casing individual methods here, is
/// what guarantees a CLI caller and a socket caller can never get different
/// answers to the same request.
pub fn handle_local(shared: &DaemonShared, method: &str, params: serde_json::Value) -> Response {
    let req = Request {
        id: 0,
        method: method.to_string(),
        params,
    };
    block_on_ipc(shared, &req)
}

/// Run `handle_request_async` to completion from a synchronous caller.
///
/// The CLI binary is itself async (`#[tokio::main]`, multi-thread flavor),
/// so a call from it executes on a tokio worker thread -- but plenty of test
/// callers of `handle_local` run inside a default (current-thread)
/// `#[tokio::test]`, and some might not be inside any runtime at all. Both
/// `tokio::task::block_in_place` (needs the multi-thread flavor) and
/// building a second `Runtime` and calling `.block_on()` on the *same*
/// thread (tokio refuses to re-enter a runtime context on one thread) would
/// panic in one of those cases. A scoped OS thread sidesteps all of it: it
/// carries no tokio context of its own, so a throwaway current-thread
/// runtime on it can always `block_on` the real `handle_request_async`, and
/// `std::thread::scope` lets it borrow `shared`/`req` without requiring
/// `'static`.
fn block_on_ipc(shared: &DaemonShared, req: &Request) -> Response {
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(_) => return Response::err(req.id, ERR_UNAVAILABLE, "runtime-unavailable"),
                };
                rt.block_on(handle_request_async(shared, req))
            })
            .join()
            .unwrap_or_else(|_| Response::err(req.id, ERR_UNAVAILABLE, "ipc-thread-panicked"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;
    use crate::daemon::policy::UNKNOWN_PROJECT_KEY;

    fn shared() -> DaemonShared {
        let (_d, store) = temp_store();
        // Leak the tempdir for the lifetime of the test process; the store
        // borrows its path.
        std::mem::forget(_d);
        DaemonShared::load(store).unwrap()
    }

    fn req(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    fn at(s: &str) -> chrono::DateTime<Utc> {
        s.parse().unwrap()
    }

    /// A real directory on this machine whose canonical path is an
    /// admissible project key. `set_project_mode` no longer accepts a key
    /// the daemon cannot corroborate, so tests name directories that exist
    /// -- exactly as the CLI's `daemon project <path>` does.
    ///
    /// The tempdir is leaked for the lifetime of the test process: the key
    /// must stay resolvable for as long as the daemon under test might
    /// re-validate it.
    fn tmp_project(basename: &str) -> String {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join(basename);
        std::fs::create_dir_all(&p).unwrap();
        std::mem::forget(d);
        std::fs::canonicalize(&p)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn arming_autonomy_over_the_socket_is_now_allowed() {
        // The terminal-only gate is removed: same-user code that can reach
        // this socket can already read the session files directly and
        // install its own watcher, so this call grants it neither the read
        // nor the persistence it would need to exfiltrate anything, and
        // would in fact be a worse channel for an attacker than doing it
        // itself (rate-limited, capped, redacted, delivered somewhere it
        // cannot read back). See the module doc's "Authorization" section.
        let s = shared();
        let key = tmp_project("p");
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": key, "mode": "auto_upload"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            s.policy.lock().unwrap().resolve(&key),
            ProjectMode::AutoUpload
        );
    }

    #[test]
    fn arming_autonomy_appends_an_audit_entry() {
        // The audit log is what replaced the removed gate: not a control,
        // but a local record a contributor can read to see when autonomy
        // was granted.
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": tmp_project("p"), "mode": "auto_upload"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        let entries = audit::load(&s.store).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "armed-auto-upload");
        assert_eq!(entries[0].project_label.as_deref(), Some("p"));
    }

    #[test]
    fn set_project_mode_relabels_the_queue_immediately_with_no_intervening_tick() {
        // Regression for the round-1 residual: a queue entry's stored label
        // must not lag a policy edit until the next poll. Everything here
        // goes through `handle_request` / direct queue seeding -- `tick` is
        // never called -- so any staleness can only come from
        // `set_project_mode` itself failing to relabel the queue.
        let s = shared();

        // "work/api" is configured and already has a queue entry, seeded
        // directly (as if a session had been queued for it earlier while
        // its basename was still unique).
        let work_api = tmp_project("api");
        let client_api = tmp_project("api");
        handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": work_api, "mode": "notify_only"}),
            ),
        );
        let entry_id = uuid::Uuid::new_v4();
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(
                    super::super::queue::QueueEntry {
                        entry_id,
                        session_hash: "sha256:seed".to_string(),
                        source: "claude-code".to_string(),
                        project_key: work_api.clone(),
                        project_label: "api".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Pending,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
                        approved_scopes: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                    },
                    500,
                )
                .unwrap();
        }

        // A colliding project shows up via a policy edit -- no tick runs.
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": client_api, "mode": "ignore"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);

        let queue_label = {
            let queue = s.queue.lock().unwrap();
            queue.get(entry_id).unwrap().project_label.clone()
        };

        let list = handle_request(&s, &req("list_projects", serde_json::json!({})));
        let projects = list.result.unwrap()["projects"].clone();
        let work_row = projects
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["mode"] == serde_json::json!("notify_only"))
            .expect("work/api row");
        let list_label = work_row["project_label"].as_str().unwrap().to_string();

        assert_eq!(
            queue_label, list_label,
            "queue and list_projects must agree immediately, with no tick in between"
        );
        assert!(
            list_label.starts_with("api ("),
            "expected a collision suffix, got {list_label}"
        );
    }

    #[test]
    fn setting_notify_only_over_the_socket_is_allowed() {
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": tmp_project("p"), "mode": "notify_only"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
    }

    #[tokio::test]
    async fn bulk_approval_over_the_socket_is_now_allowed_and_appends_an_audit_entry() {
        // As with arming autonomy, the terminal-only gate on bulk approval
        // is removed for the same reason: it restricted nothing an attacker
        // with same-user code execution did not already have. The audit
        // entry is the replacement -- visibility, not a control.
        let s = shared();
        let r = handle_request_async(&s, &req("approve", serde_json::json!({"all": true}))).await;
        assert!(r.error.is_none(), "{:?}", r.error);
        let entries = audit::load(&s.store).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "bulk-approved");
        assert_eq!(entries[0].project_label, None);
    }

    #[tokio::test]
    async fn a_single_entry_approval_leaves_the_audit_log_empty() {
        // Only the "approve all" bulk action is consequential enough to
        // audit; approving one entry at a time is the default, always-was
        // path and does not need a new log entry per click.
        let s = shared();
        let entry_id = uuid::Uuid::new_v4();
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(
                    super::super::queue::QueueEntry {
                        entry_id,
                        session_hash: "sha256:seed".to_string(),
                        source: "claude-code".to_string(),
                        project_key: "/tmp/p".to_string(),
                        project_label: "p".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Pending,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
                        approved_scopes: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                    },
                    500,
                )
                .unwrap();
        }
        let r = handle_request_async(
            &s,
            &req(
                "approve",
                serde_json::json!({"entry_id": entry_id.to_string()}),
            ),
        )
        .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(audit::load(&s.store).unwrap().is_empty());
    }

    #[test]
    fn a_caller_supplied_label_never_reaches_list_projects_or_the_audit_log() {
        // `set_project_mode` used to store whatever `label` a socket client
        // sent and hand it straight to `list_projects` and to
        // `daemon-audit.jsonl` -- the two sinks the label-only rule exists
        // to protect. The label is now derived from the key; the param is
        // accepted and ignored.
        let s = shared();
        let key = tmp_project("myproj");
        let injected = "ghp_fakeinjectedtoken/and/a/path";
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({
                    "project_key": key,
                    "label": injected,
                    "mode": "auto_upload",
                }),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);

        let list = handle_request(&s, &req("list_projects", serde_json::json!({})));
        let projects = serde_json::to_string(&list.result.unwrap()).unwrap();
        assert!(
            !projects.contains("ghp_fakeinjectedtoken"),
            "a caller-supplied label reached list_projects: {projects}"
        );
        assert!(
            projects.contains("\"myproj\""),
            "the label must be derived from the key: {projects}"
        );

        let audit_text = serde_json::to_string(&audit::load(&s.store).unwrap()).unwrap();
        assert!(
            !audit_text.contains("ghp_fakeinjectedtoken"),
            "a caller-supplied label reached the audit log: {audit_text}"
        );
        assert_eq!(
            audit::load(&s.store).unwrap()[0].project_label.as_deref(),
            Some("myproj")
        );
    }

    /// Seed one pending queue entry for `project_key`, the way a poll that
    /// discovered a session would, without running the watcher.
    fn seed_entry(s: &DaemonShared, project_key: &str) -> uuid::Uuid {
        let entry_id = uuid::Uuid::new_v4();
        let mut queue = s.queue.lock().unwrap();
        queue
            .upsert(
                super::super::queue::QueueEntry {
                    entry_id,
                    session_hash: format!("sha256:{entry_id}"),
                    source: "claude-code".to_string(),
                    project_key: project_key.to_string(),
                    project_label: super::super::policy::project_label_for(project_key),
                    path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    state: QueueState::Pending,
                    reason_label: None,
                    attempts: 0,
                    retry_after: None,
                    submission_id: None,
                    approved_scopes: None,
                    approved_inputs: None,
                    previewed_envelope_digest: None,
                    approved_at: None,
                    subagent_count: 0,
                    subagents_dropped: 0,
                },
                500,
            )
            .unwrap();
        entry_id
    }

    fn projects_of(s: &DaemonShared) -> Vec<serde_json::Value> {
        handle_request(s, &req("list_projects", serde_json::json!({})))
            .result
            .unwrap()["projects"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn a_project_id_from_list_pending_is_accepted_by_set_project_mode() {
        // The gap this closes. A socket client sees `project_label` and
        // never `project_key`, and a label is not an admissible key -- so
        // before the id existed, a GUI holding a queue entry had no way to
        // say anything at all about the project it came from.
        let s = shared();
        let key = tmp_project("p");
        seed_entry(&s, &key);

        let pending = handle_request(&s, &req("list_pending", serde_json::json!({})))
            .result
            .unwrap()["pending"]
            .as_array()
            .unwrap()
            .clone();
        let project_id = pending[0]["project_id"].as_str().unwrap().to_string();
        assert!(
            pending[0].get("project_key").is_none(),
            "a key must never cross the wire: {:?}",
            pending[0]
        );

        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_id": project_id, "mode": "ignore"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(s.policy.lock().unwrap().resolve(&key), ProjectMode::Ignore);
    }

    #[test]
    fn a_project_id_from_list_projects_is_accepted_by_set_project_mode() {
        let s = shared();
        let key = tmp_project("p");
        seed_entry(&s, &key);

        let row = projects_of(&s)[0].clone();
        let project_id = row["project_id"].as_str().unwrap().to_string();
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_id": project_id, "mode": "auto_upload"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            s.policy.lock().unwrap().resolve(&key),
            ProjectMode::AutoUpload
        );
    }

    #[test]
    fn an_unknown_project_id_is_refused_with_a_fixed_label_and_records_nothing() {
        let s = shared();
        let unknown = super::super::policy::project_id_for("/Users/z/never/seen");
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_id": unknown, "mode": "auto_upload"}),
            ),
        );
        let err = r.error.expect("an unknown id must be refused");
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, ERR_PROJECT_ID_UNRECOGNIZED);
        assert!(s.policy.lock().unwrap().projects.is_empty());
        assert!(
            audit::load(&s.store).unwrap().is_empty(),
            "a refused call must record nothing"
        );
        assert!(projects_of(&s).is_empty());
    }

    #[test]
    fn a_real_canonical_path_is_still_accepted_before_the_project_is_ever_seen() {
        // The CLI's pre-discovery flow: `daemon project <path> --mode
        // ignore` for a project whose first session has not happened, so no
        // id exists for it and none can. This is why the id supplements the
        // key rather than replacing it.
        let s = shared();
        let key = tmp_project("employer-repo");
        assert!(
            projects_of(&s).is_empty(),
            "the project must be genuinely unknown for this test to mean anything"
        );
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": key, "mode": "ignore"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(s.policy.lock().unwrap().resolve(&key), ProjectMode::Ignore);
    }

    #[test]
    fn a_project_id_is_stable_across_a_daemon_restart_and_a_rebuilt_policy_file() {
        // Ids are derived, never stored, so there is nothing for a restart
        // or a from-scratch policy file to lose. A client that cached an id
        // yesterday can still use it today.
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let open = || crate::config::ConfigStore::open(state.clone()).unwrap();
        let key = tmp_project("p");

        let first = {
            let s = DaemonShared::load(open()).unwrap();
            let r = handle_request(
                &s,
                &req(
                    "set_project_mode",
                    serde_json::json!({"project_key": key, "mode": "ignore"}),
                ),
            );
            assert!(r.error.is_none(), "{:?}", r.error);
            projects_of(&s)[0]["project_id"]
                .as_str()
                .unwrap()
                .to_string()
        };

        // A second daemon over the same state directory: a restart.
        let s = DaemonShared::load(open()).unwrap();
        assert_eq!(projects_of(&s)[0]["project_id"].as_str().unwrap(), first);

        // And a policy file rebuilt from scratch, holding the same project.
        let mut rebuilt = ProjectPolicy::new();
        rebuilt
            .set_mode(&key, ProjectMode::NotifyOnly, Utc::now())
            .unwrap();
        rebuilt.save(&open()).unwrap();
        let s = DaemonShared::load(open()).unwrap();
        let row = projects_of(&s)[0].clone();
        assert_eq!(row["project_id"].as_str().unwrap(), first);
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_id": first, "mode": "ignore"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
    }

    #[test]
    fn a_project_id_never_carries_a_path_component() {
        let s = shared();
        let key = tmp_project("acme-secret-client");
        seed_entry(&s, &key);
        let wire = format!(
            "{}{}",
            serde_json::to_string(&handle_request(
                &s,
                &req("list_pending", serde_json::json!({}))
            ))
            .unwrap(),
            serde_json::to_string(&projects_of(&s)).unwrap()
        );
        let id = super::super::policy::project_id_for(&key);
        assert!(wire.contains(&id), "the id must be on the wire: {wire}");
        assert!(
            !id.contains("acme") && !id.contains("secret") && !id.contains('/'),
            "the id leaked a path component: {id}"
        );
        // Only segments long enough that a coincidental match is implausible.
        //
        // The id is a prefix plus 16 hex characters, and a temp path on macOS
        // contains short components that are themselves valid hex -- a real
        // one is `/private/var/folders/d8/...`. Asserting the id does not
        // contain "d8" fails about one run in seventeen purely because two
        // hex characters agree, which is not a leak.
        //
        // A security test that cries wolf at that rate is worse than no test:
        // it teaches everyone to re-run it, and a real leak is then waved
        // through with the same shrug. Two separate agents hit this flake on
        // 2026-08-10 while working on unrelated changes. Four characters puts
        // a coincidental hit at roughly one in sixteen thousand while still
        // catching any segment big enough to identify anybody.
        const MIN_DISTINGUISHING_LEN: usize = 4;
        for segment in std::path::Path::new(&key)
            .parent()
            .unwrap()
            .components()
            .filter_map(|c| match c {
                std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .filter(|segment| segment.len() >= MIN_DISTINGUISHING_LEN)
        {
            assert!(
                !id.contains(&segment),
                "the id leaked the path segment {segment}"
            );
        }
    }

    #[test]
    fn list_projects_reports_a_discovered_but_unconfigured_project() {
        // Onboarding's "which of these should never be uploaded" screen
        // needs exactly this set: a project is configured only once it has
        // been ruled on, so listing only configured projects lists only the
        // decisions already made and never the one the contributor is being
        // asked to make.
        let s = shared();
        let key = tmp_project("employer-repo");
        seed_entry(&s, &key);

        let rows = projects_of(&s);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["project_label"], serde_json::json!("employer-repo"));
        assert_eq!(rows[0]["configured"], serde_json::json!(false));
        assert!(rows[0]["added_at"].is_null());
        assert_eq!(
            rows[0]["mode"],
            serde_json::json!("notify_only"),
            "an unruled project reports the effective default"
        );

        // Ruling on it makes it configured, and does not duplicate the row.
        let id = rows[0]["project_id"].as_str().unwrap().to_string();
        handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_id": id, "mode": "ignore"}),
            ),
        );
        let rows = projects_of(&s);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["configured"], serde_json::json!(true));
        assert_eq!(rows[0]["mode"], serde_json::json!("ignore"));
    }

    #[test]
    fn list_projects_marks_only_the_unresolvable_bucket() {
        // The flag exists so a shell never has to re-derive `project_id_for`
        // to know which row this is, and never matches on `project_label` --
        // which every client rewords, because the raw label is a slug.
        let s = shared();
        let ordinary = tmp_project("employer-repo");
        seed_entry(&s, &ordinary);
        seed_entry(&s, UNKNOWN_PROJECT_KEY);

        let rows = projects_of(&s);
        assert_eq!(rows.len(), 2, "{rows:?}");

        let bucket: Vec<&serde_json::Value> = rows
            .iter()
            .filter(|r| r["is_unresolved_bucket"] == serde_json::json!(true))
            .collect();
        assert_eq!(bucket.len(), 1, "exactly one row is the bucket: {rows:?}");

        // And it is the right one. Checked through the id the daemon minted
        // for the key, not through the label, so this test cannot pass by
        // agreeing with a display string.
        assert_eq!(
            bucket[0]["project_id"],
            serde_json::json!(project_id_for(UNKNOWN_PROJECT_KEY))
        );

        let ordinary_row = rows
            .iter()
            .find(|r| r["project_id"] == serde_json::json!(project_id_for(&ordinary)))
            .expect("the ordinary project is listed");
        assert_eq!(
            ordinary_row["is_unresolved_bucket"],
            serde_json::json!(false),
            "an ordinary project must never be explained as unresolvable"
        );
    }

    #[test]
    fn the_unresolvable_flag_survives_being_ruled_on() {
        // A contributor can silence the bucket even though it can never be
        // armed. Ignoring it moves it from discovered to configured, and the
        // marker has to hold across that -- otherwise the row loses its
        // explanation exactly when someone has interacted with it.
        let s = shared();
        seed_entry(&s, UNKNOWN_PROJECT_KEY);

        let rows = projects_of(&s);
        let id = rows[0]["project_id"].as_str().unwrap().to_string();
        assert_eq!(rows[0]["is_unresolved_bucket"], serde_json::json!(true));
        assert_eq!(rows[0]["configured"], serde_json::json!(false));

        handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_id": id, "mode": "ignore"}),
            ),
        );

        let rows = projects_of(&s);
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0]["configured"], serde_json::json!(true));
        assert_eq!(rows[0]["mode"], serde_json::json!("ignore"));
        assert_eq!(
            rows[0]["is_unresolved_bucket"],
            serde_json::json!(true),
            "the marker is a property of the key, not of whether it is configured"
        );
    }

    #[test]
    fn nothing_a_client_sends_reaches_list_projects_or_the_audit_log_via_an_id() {
        // The original injection fix must survive the new entry point: the
        // id path resolves to a key the daemon already holds, so the label
        // is still derived and a caller's strings still reach neither sink.
        let s = shared();
        let key = tmp_project("myproj");
        seed_entry(&s, &key);
        let id = super::super::policy::project_id_for(&key);
        let injected = "ghp_fakeinjectedtoken/and/a/path";
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({
                    "project_id": id,
                    "project_key": injected,
                    "label": injected,
                    "mode": "auto_upload",
                }),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);

        let listed = serde_json::to_string(&projects_of(&s)).unwrap();
        assert!(
            !listed.contains("ghp_fakeinjectedtoken"),
            "a caller-supplied string reached list_projects: {listed}"
        );
        assert!(listed.contains("\"myproj\""), "{listed}");
        let audit_text = serde_json::to_string(&audit::load(&s.store).unwrap()).unwrap();
        assert!(
            !audit_text.contains("ghp_fakeinjectedtoken"),
            "a caller-supplied string reached the audit log: {audit_text}"
        );
        assert_eq!(
            audit::load(&s.store).unwrap()[0].project_label.as_deref(),
            Some("myproj")
        );
    }

    #[test]
    fn naming_a_project_neither_way_is_a_bad_params_error() {
        let s = shared();
        let r = handle_request(
            &s,
            &req("set_project_mode", serde_json::json!({"mode": "ignore"})),
        );
        let err = r.error.expect("must be refused");
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, "project_id-or-project_key-required");
    }

    /// Make the audit log unappendable: `audit::load` reads the file as
    /// UTF-8 and fails on bytes that are not, so every subsequent `append`
    /// fails too. Stands in for a disk-full, permissions, or corruption
    /// failure without needing any of those.
    fn break_the_audit_log(store: &ConfigStore) {
        store
            .write_daemon_file(crate::config::DAEMON_AUDIT_FILE, &[0xff, 0xfe, 0xff])
            .unwrap();
    }

    #[test]
    fn arming_autonomy_is_rolled_back_when_its_audit_entry_cannot_be_written() {
        // The audit entry is the stated replacement for a removed
        // terminal-only restriction. A best-effort append reduced a
        // disk-full or permissions failure to a warning while the call
        // still returned success, silently defeating the whole replacement.
        let s = shared();
        let key = tmp_project("p");
        break_the_audit_log(&s.store);

        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": key, "mode": "auto_upload"}),
            ),
        );
        let err = r.error.expect("an unwritable audit log must fail the call");
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, "audit-write-failed");
        assert_eq!(
            s.policy.lock().unwrap().resolve(&key),
            ProjectMode::NotifyOnly,
            "autonomy must not stand without a record of it"
        );
        // And the rollback is durable, not only in memory.
        let on_disk = ProjectPolicy::load(&s.store).unwrap();
        assert_eq!(on_disk.resolve(&key), ProjectMode::NotifyOnly);
    }

    #[test]
    fn a_notify_only_change_still_succeeds_with_an_unwritable_audit_log() {
        // Only the consequential actions are audited, so only they are
        // gated on the audit succeeding. Setting notify_only writes no
        // entry and must not be collateral damage.
        let s = shared();
        let key = tmp_project("p");
        break_the_audit_log(&s.store);
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": key, "mode": "notify_only"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
    }

    #[tokio::test]
    async fn bulk_approval_is_rolled_back_when_its_audit_entry_cannot_be_written() {
        let s = shared();
        let entry_id = uuid::Uuid::new_v4();
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(
                    super::super::queue::QueueEntry {
                        entry_id,
                        session_hash: "sha256:seed".to_string(),
                        source: "claude-code".to_string(),
                        project_key: "/tmp/p".to_string(),
                        project_label: "p".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Pending,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
                        approved_scopes: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                    },
                    500,
                )
                .unwrap();
        }
        break_the_audit_log(&s.store);

        let r = handle_request_async(&s, &req("approve", serde_json::json!({"all": true}))).await;
        let err = r.error.expect("an unwritable audit log must fail the call");
        assert_eq!(err.message, "audit-write-failed");
        let state = s.queue.lock().unwrap().get(entry_id).unwrap().state;
        assert_eq!(
            state,
            QueueState::Pending,
            "an unrecorded bulk approval must not stand"
        );
    }

    #[test]
    fn a_project_key_the_daemon_cannot_corroborate_is_refused() {
        // Deriving the label from the key is not enough on its own: the
        // basename of an attacker-chosen key is still an attacker-chosen
        // string. A key must be the unknown-cwd sentinel, one the daemon
        // already knows, or a real local directory.
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({
                    "project_key": "/nonexistent-xyz/ghp_fakeinjectedtoken",
                    "mode": "auto_upload",
                }),
            ),
        );
        let err = r.error.expect("an unrecognized key must be refused");
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, ERR_PROJECT_KEY_UNRECOGNIZED);
        assert!(
            s.policy.lock().unwrap().projects.is_empty(),
            "a refused key must not be recorded"
        );
        assert!(audit::load(&s.store).unwrap().is_empty());
    }

    #[test]
    fn a_key_already_known_to_the_daemon_stays_settable() {
        // A project the daemon discovered on a queued session must remain
        // configurable even if its directory has since been deleted --
        // otherwise the contributor loses the ability to say "ignore this"
        // about exactly the sessions already sitting in their queue.
        let s = shared();
        let gone = "/nonexistent-xyz/oldproj";
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(
                    super::super::queue::QueueEntry {
                        entry_id: uuid::Uuid::new_v4(),
                        session_hash: "sha256:known".to_string(),
                        source: "claude-code".to_string(),
                        project_key: gone.to_string(),
                        project_label: "oldproj".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Pending,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
                        approved_scopes: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                    },
                    500,
                )
                .unwrap();
        }
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": gone, "mode": "ignore"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(s.policy.lock().unwrap().resolve(gone), ProjectMode::Ignore);
    }

    #[test]
    fn the_unknown_bucket_cannot_be_armed_even_from_a_terminal() {
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": UNKNOWN_PROJECT_KEY, "mode": "auto_upload"}),
            ),
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    // `bind`'s socket-path-length refusal and `ensure_private_dir`'s 0700
    // check are both properties of the unix-socket transport specifically:
    // Windows has no socket path to overflow and no directory-mode access
    // control (see `win_pipe.rs`, whose DACL plays that role instead), so
    // there is no Windows equivalent of either function to test here.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_bind_failure_never_names_a_local_path() {
        // These errors are returned to `daemon run`, which under a service
        // manager writes them to the journal -- where a state-directory
        // path carries the OS username.
        let deep = std::env::temp_dir().join("a".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&deep, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let store = ConfigStore::open(deep.clone()).unwrap();
        let err = bind(&store).await.unwrap_err();
        let text = format!("{err:#}");
        assert!(!text.contains(&*deep.to_string_lossy()), "{text}");
        assert!(
            text.contains("kernel limit"),
            "the message must still say what to do: {text}"
        );
        let _ = std::fs::remove_dir_all(&deep);
    }

    #[cfg(unix)]
    #[test]
    fn a_state_directory_permissions_failure_never_names_a_local_path() {
        let missing = std::env::temp_dir().join("trace-commons-no-such-dir-xyz");
        let _ = std::fs::remove_dir_all(&missing);
        let err = ensure_private_dir(&missing).unwrap_err();
        let text = format!("{err:#}");
        assert!(!text.contains("trace-commons-no-such-dir-xyz"), "{text}");
    }

    #[test]
    fn an_unknown_method_uses_the_taxonomy() {
        let s = shared();
        let r = handle_request(&s, &req("no_such_method", serde_json::json!({})));
        assert_eq!(r.error.unwrap().code, ERR_UNKNOWN_METHOD);
    }

    #[test]
    fn hello_advertises_the_schema_and_method_set() {
        let s = shared();
        let r = handle_request(&s, &req("hello", serde_json::json!({})));
        let result = r.result.unwrap();
        assert_eq!(result["schema_version"], IPC_SCHEMA);
        assert_eq!(result["methods"].as_array().unwrap().len(), METHODS.len());
    }

    #[test]
    fn status_exposes_every_field_a_tray_needs() {
        let s = shared();
        let r = handle_request(&s, &req("status", serde_json::json!({})));
        let v = r.result.unwrap();
        for key in ["logged_in", "paused", "queue_depth", "health"] {
            assert!(!v[key].is_null(), "status missing {key}");
        }
        assert_eq!(v["logged_in"], false);
    }

    #[test]
    fn pause_and_resume_round_trip() {
        let s = shared();
        handle_request(&s, &req("pause", serde_json::json!({})));
        assert_eq!(s.status_value()["paused"], true);
        handle_request(&s, &req("resume", serde_json::json!({})));
        assert_eq!(s.status_value()["paused"], false);
    }

    #[test]
    fn settings_never_echo_the_privacy_filter_credential() {
        let s = shared();
        s.settings.lock().unwrap().near_ai = Some(crate::envelope::NearAiSettings {
            api_key: "super-secret-key".into(),
            base_url: None,
            model: None,
        });
        let r = handle_request(&s, &req("get_settings", serde_json::json!({})));
        let body = serde_json::to_string(&r.result.unwrap()).unwrap();
        assert!(!body.contains("super-secret-key"), "{body}");
        assert!(body.contains("near_ai_configured"));
    }

    #[test]
    fn get_settings_never_carries_a_local_filesystem_path() {
        // The wholesale-serialized settings blob used to leak claude_root /
        // codex_root verbatim whenever either was overridden from the
        // conventional location -- exactly what entry_value is scrupulous
        // about avoiding for queue entries.
        let s = shared();
        {
            let mut settings = s.settings.lock().unwrap();
            settings.claude_source = Some(crate::daemon::settings::SourceDeclaration::Watch {
                path: std::path::PathBuf::from("/Users/z/.claude/projects"),
            });
            settings.codex_source = Some(crate::daemon::settings::SourceDeclaration::Watch {
                path: std::path::PathBuf::from("/Users/z/.codex/sessions"),
            });
        }
        let r = handle_request(&s, &req("get_settings", serde_json::json!({})));
        let result = r.result.unwrap();
        let body = serde_json::to_string(&result).unwrap();
        assert!(!body.contains('/'), "path leaked to the wire: {body}");
        assert_eq!(result["claude_root_configured"], true);
        assert_eq!(result["codex_root_configured"], true);
    }

    #[test]
    fn a_queue_entry_on_the_wire_carries_no_local_path() {
        use crate::daemon::queue::{QueueEntry, entry_id_for};
        let e = QueueEntry {
            entry_id: entry_id_for("sha256:aa"),
            session_hash: "sha256:aa".into(),
            source: "claude-code".into(),
            project_key: "/Users/z/code/secret-client-project".into(),
            project_label: "secret-client-project".into(),
            path: "/Users/z/.claude/projects/x/s.jsonl".into(),
            size_bytes: 10,
            discovered_at: Utc::now(),
            state: QueueState::Pending,
            reason_label: None,
            attempts: 0,
            retry_after: None,
            submission_id: None,
            approved_scopes: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
            approved_at: None,
            subagent_count: 0,
            subagents_dropped: 0,
        };
        let body = serde_json::to_string(&entry_value(&e)).unwrap();
        assert!(
            !body.contains("/Users/z"),
            "path leaked to the wire: {body}"
        );
        assert!(body.contains("secret-client-project"));
    }

    #[test]
    fn the_upgrade_retires_entries_that_stand_for_a_lone_subagent_transcript() {
        // Discovery no longer yields a `subagents/` path, so these entries
        // are unreachable: an approved one would fail `session-file-vanished`
        // and a pending one would sit until it aged out. Worse, each still
        // offers a fragment whose opening prompt was written by the parent
        // agent. Say what happened instead, and leave the top-level entry --
        // and anything already resolved -- exactly as it was.
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let open = || crate::config::ConfigStore::open(state.clone()).unwrap();

        let seed =
            |hash: &str, path: &str, entry_state: QueueState| super::super::queue::QueueEntry {
                entry_id: super::super::queue::entry_id_for(hash),
                session_hash: hash.to_string(),
                source: "claude-code".to_string(),
                project_key: "/tmp/p".to_string(),
                project_label: "p".to_string(),
                path: std::path::PathBuf::from(path),
                size_bytes: 1,
                discovered_at: Utc::now(),
                state: entry_state,
                reason_label: None,
                attempts: 0,
                retry_after: None,
                submission_id: None,
                approved_scopes: None,
                approved_inputs: None,
                previewed_envelope_digest: None,
                approved_at: None,
                subagent_count: 0,
                subagents_dropped: 0,
            };
        let mut queue = Queue::new();
        queue
            .upsert(
                seed(
                    "sha256:top",
                    "/p/-Users-z-proj/aaa.jsonl",
                    QueueState::Pending,
                ),
                500,
            )
            .unwrap();
        queue
            .upsert(
                seed(
                    "sha256:sub",
                    "/p/-Users-z-proj/aaa/subagents/agent-1.jsonl",
                    QueueState::Pending,
                ),
                500,
            )
            .unwrap();
        queue
            .upsert(
                seed(
                    "sha256:subapproved",
                    "/p/-Users-z-proj/aaa/subagents/agent-2.jsonl",
                    QueueState::Approved,
                ),
                500,
            )
            .unwrap();
        queue
            .upsert(
                seed(
                    "sha256:subdone",
                    "/p/-Users-z-proj/aaa/subagents/agent-3.jsonl",
                    QueueState::Uploaded,
                ),
                500,
            )
            .unwrap();
        queue.save(&open()).unwrap();

        let s = DaemonShared::load(open()).unwrap();
        let q = s.queue.lock().unwrap();
        let by_hash = |h: &str| q.all().iter().find(|e| e.session_hash == h).unwrap();
        assert_eq!(by_hash("sha256:top").state, QueueState::Pending);
        assert_eq!(by_hash("sha256:sub").state, QueueState::Superseded);
        assert_eq!(
            by_hash("sha256:sub").reason_label.as_deref(),
            Some("regrouped-under-parent")
        );
        assert_eq!(by_hash("sha256:subapproved").state, QueueState::Superseded);
        assert_eq!(
            by_hash("sha256:subdone").state,
            QueueState::Uploaded,
            "an upload already recorded must not be rewritten by an upgrade"
        );
    }

    #[test]
    fn the_queue_card_reports_how_many_delegated_transcripts_it_covers() {
        // A card standing for a hundred delegated transcripts has to say so:
        // the extent of what is being sent is part of the consent decision,
        // not decoration. No ordinal is exposed -- nothing in the format
        // supplies one.
        let e = super::super::queue::QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            session_hash: "sha256:aa".to_string(),
            source: "claude-code".to_string(),
            project_key: "/tmp/p".to_string(),
            project_label: "p".to_string(),
            path: std::path::PathBuf::from("/tmp/s.jsonl"),
            size_bytes: 1,
            discovered_at: Utc::now(),
            state: QueueState::Pending,
            reason_label: None,
            attempts: 0,
            retry_after: None,
            submission_id: None,
            approved_scopes: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
            approved_at: None,
            subagent_count: 114,
            subagents_dropped: 2,
        };
        let v = entry_value(&e);
        assert_eq!(v["subagent_count"], 114);
        assert_eq!(v["subagents_dropped"], 2);
        let body = serde_json::to_string(&v).unwrap();
        assert!(!body.contains("/tmp/s.jsonl"), "path leaked: {body}");
    }

    #[test]
    fn a_bad_entry_id_is_a_param_error_not_a_panic() {
        let s = shared();
        let r = handle_request(
            &s,
            &req("dismiss", serde_json::json!({"entry_id": "not-a-uuid"})),
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn set_settings_rejects_a_payload_with_nothing_known_in_it() {
        let s = shared();
        let r = handle_request(&s, &req("set_settings", serde_json::json!({"nonsense": 1})));
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    fn seed_entry_in_state(s: &DaemonShared, state: QueueState) -> Uuid {
        let entry_id = uuid::Uuid::new_v4();
        let mut queue = s.queue.lock().unwrap();
        queue
            .upsert(
                super::super::queue::QueueEntry {
                    entry_id,
                    session_hash: format!("sha256:{entry_id}"),
                    source: "claude-code".to_string(),
                    project_key: "/tmp/p".to_string(),
                    project_label: "p".to_string(),
                    path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                    size_bytes: 1,
                    discovered_at: Utc::now(),
                    state: QueueState::Pending,
                    reason_label: None,
                    attempts: 0,
                    retry_after: None,
                    submission_id: None,
                    approved_scopes: None,
                    approved_inputs: None,
                    previewed_envelope_digest: None,
                    approved_at: None,
                    subagent_count: 0,
                    subagents_dropped: 0,
                },
                500,
            )
            .unwrap();
        queue.set_state(entry_id, state, None);
        entry_id
    }

    fn seed_approved_entry(s: &DaemonShared) -> Uuid {
        seed_entry_in_state(s, QueueState::Approved)
    }

    #[test]
    fn acknowledging_the_near_ai_notice_clears_the_blocking_health_label() {
        // Without this an app-only contributor (never touching the CLI,
        // which shows the same notice on stdout) is stuck forever.
        let s = shared();
        s.health.lock().unwrap().fail(
            crate::daemon::health::LABEL_NEAR_AI_NOTICE_PENDING,
            Utc::now(),
        );
        let r = handle_request(
            &s,
            &req("acknowledge_near_ai_notice", serde_json::json!({})),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(s.store.near_ai_notice_shown());
        assert!(s.health.lock().unwrap().ok());
    }

    #[test]
    fn cancel_returns_an_approved_entry_to_pending() {
        let s = shared();
        let id = seed_approved_entry(&s);
        let r = handle_request(
            &s,
            &req("cancel", serde_json::json!({"entry_id": id.to_string()})),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            s.queue.lock().unwrap().get(id).unwrap().state,
            QueueState::Pending
        );
    }

    #[test]
    fn cancel_refuses_once_the_upload_is_in_flight() {
        let s = shared();
        let id = seed_entry_in_state(&s, QueueState::Uploading);
        let r = handle_request(
            &s,
            &req("cancel", serde_json::json!({"entry_id": id.to_string()})),
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn cancel_of_an_unknown_entry_is_a_param_error() {
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "cancel",
                serde_json::json!({"entry_id": uuid::Uuid::new_v4().to_string()}),
            ),
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn a_timed_pause_is_persisted_so_it_survives_a_restart() {
        // An app-side timer would die with the app and silently un-pause.
        let s = shared();
        let until = "2030-01-01T00:00:00Z";
        handle_request(&s, &req("pause", serde_json::json!({"until": until})));
        assert_eq!(
            s.state.lock().unwrap().paused_until.map(|t| t.to_rfc3339()),
            Some(until.parse::<chrono::DateTime<Utc>>().unwrap().to_rfc3339())
        );
    }

    #[test]
    fn pause_rejects_a_deadline_already_in_the_past() {
        // Accepting it would publish a pause event for a pause the very next
        // status call (or is_paused check) clears -- a lie the instant it's
        // acknowledged.
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "pause",
                serde_json::json!({"until": "2020-01-01T00:00:00Z"}),
            ),
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
        assert!(!s.paused.load(Ordering::Relaxed));
    }

    #[test]
    fn a_lapsed_timed_pause_clears_itself_when_checked() {
        // A deadline that was in the future when set, and has since passed
        // (unlike the request-time validation above, which only catches a
        // deadline that was already past when submitted).
        let s = shared();
        handle_request(
            &s,
            &req(
                "pause",
                serde_json::json!({"until": "2030-01-01T00:00:00Z"}),
            ),
        );
        assert!(s.is_paused(at("2029-12-31T00:00:00Z")));
        assert!(
            !s.is_paused(at("2030-06-01T00:00:00Z")),
            "an elapsed pause is not a pause"
        );
        assert_eq!(s.status_value()["paused"], false);
    }

    #[test]
    fn a_lapsed_timed_pause_publishes_status_changed() {
        let s = shared();
        handle_request(
            &s,
            &req(
                "pause",
                serde_json::json!({"until": "2030-01-01T00:00:00Z"}),
            ),
        );
        let mut rx = s.events.subscribe();
        assert!(!s.is_paused(at("2030-06-01T00:00:00Z")));
        let ev = rx.try_recv().expect("no status_changed event published");
        assert_eq!(ev.event, EVENT_STATUS_CHANGED);
    }

    #[test]
    fn an_untimed_pause_never_lapses_on_its_own() {
        let s = shared();
        handle_request(&s, &req("pause", serde_json::json!({})));
        assert_eq!(s.status_value()["paused"], true);
    }

    #[test]
    fn an_invalid_until_is_a_param_error() {
        let s = shared();
        let r = handle_request(
            &s,
            &req("pause", serde_json::json!({"until": "not-a-timestamp"})),
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn list_audit_reads_back_what_set_project_mode_appended() {
        let s = shared();
        handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": tmp_project("p"), "mode": "auto_upload"}),
            ),
        );
        let r = handle_request(&s, &req("list_audit", serde_json::json!({})));
        let entries = r.result.unwrap()["entries"].as_array().unwrap().clone();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["action"], "armed-auto-upload");
    }

    #[test]
    fn list_audit_honors_a_limit_and_reports_the_most_recent_entries() {
        // The log is append-by-whole-file-rewrite and otherwise unbounded,
        // same reason list_history caps.
        let s = shared();
        for key in [tmp_project("a"), tmp_project("b"), tmp_project("c")] {
            handle_request(
                &s,
                &req(
                    "set_project_mode",
                    serde_json::json!({"project_key": key, "mode": "auto_upload"}),
                ),
            );
        }
        let r = handle_request(&s, &req("list_audit", serde_json::json!({"limit": 2})));
        let entries = r.result.unwrap()["entries"].as_array().unwrap().clone();
        assert_eq!(entries.len(), 2, "{entries:?}");
        // Newest first: the last-armed project ("c") comes back before "b".
        assert_eq!(entries[0]["project_label"], "c");
        assert_eq!(entries[1]["project_label"], "b");
    }

    #[test]
    fn list_audit_caps_an_oversize_limit_at_one_thousand() {
        let s = shared();
        handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": tmp_project("p"), "mode": "auto_upload"}),
            ),
        );
        let r = handle_request(
            &s,
            &req("list_audit", serde_json::json!({"limit": 999_999})),
        );
        // Never panics or misbehaves on an absurd limit; still bounded.
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.result.unwrap()["entries"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn consent_change_and_notice_acknowledgement_both_appear_in_list_audit() {
        // Both are at least as consequential as arming auto-upload and were
        // previously silent.
        let s = shared();
        s.store
            .save_config(&crate::config::ContributorConfig {
                schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
                issuer_url: "https://issuer.invalid".to_string(),
                ingest_url: "https://ingest.invalid".to_string(),
                audience: "aud".to_string(),
                tenant_id: "tenant-1".to_string(),
                instance_id: "instance-1".to_string(),
                user_subject: "alice".to_string(),
                device_key_id: "sha256:aa".to_string(),
                consent_scopes: vec!["debugging_evaluation".to_string()],
                pii_filter: None,
                allowed_hosts: None,
                display_handle: None,
                public_bio: None,
                public_since: None,
            })
            .unwrap();
        handle_request(
            &s,
            &req(
                "set_consent_scopes",
                serde_json::json!({"scopes": ["model_training"]}),
            ),
        );
        handle_request(
            &s,
            &req("acknowledge_near_ai_notice", serde_json::json!({})),
        );

        let r = handle_request(&s, &req("list_audit", serde_json::json!({})));
        let entries = r.result.unwrap()["entries"].as_array().unwrap().clone();
        let actions: Vec<String> = entries
            .iter()
            .map(|e| e["action"].as_str().unwrap().to_string())
            .collect();
        assert!(
            actions.contains(&"consent-scopes-changed".to_string()),
            "{actions:?}"
        );
        assert!(
            actions.contains(&"near-ai-notice-acknowledged".to_string()),
            "{actions:?}"
        );
    }

    #[test]
    fn queue_outcome_counts_counts_reason_labels_already_on_the_queue() {
        let s = shared();
        let id = seed_entry_in_state(&s, QueueState::Pending);
        s.queue.lock().unwrap().set_state(
            id,
            QueueState::Expired,
            Some("expired-without-decision".to_string()),
        );
        let r = handle_request(&s, &req("queue_outcome_counts", serde_json::json!({})));
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.result.unwrap()["reasons"]["expired-without-decision"], 1);
    }

    #[test]
    fn consent_options_is_reachable_over_the_dispatcher() {
        let s = shared();
        let r = handle_request(&s, &req("consent_options", serde_json::json!({})));
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            r.result.unwrap()["scopes"].as_array().unwrap().len(),
            crate::consent::VALID_SCOPES.len()
        );
    }

    #[tokio::test]
    async fn handle_local_and_handle_request_async_answer_an_async_method_identically() {
        // Regression guard: an async method must be answered the same way
        // whether it's reached through the socket path
        // (`handle_request_async`) or the CLI path (`handle_local`). This
        // plan already hit the failure mode once, where an async method was
        // wired into only one of the two dispatchers and a CLI caller
        // silently got a degraded answer.
        let s = shared();
        let via_async = handle_request_async(&s, &req("enroll", serde_json::json!({}))).await;
        let via_local = handle_local(&s, "enroll", serde_json::json!({}));
        assert_eq!(
            via_async.result, via_local.result,
            "{via_async:?} vs {via_local:?}"
        );
        assert_eq!(
            via_async.error.map(|e| e.code),
            via_local.error.map(|e| e.code)
        );
    }

    #[tokio::test]
    async fn quiesce_parks_the_queue_when_nothing_is_in_flight() {
        let s = shared();
        let r = handle_request_async(&s, &req("quiesce", serde_json::json!({}))).await;
        let v = r.result.expect("quiesce should succeed with an idle queue");
        assert_eq!(v["quiesced"], true);
        assert!(s.quiesced.load(Ordering::Relaxed), "the flag must be set");
    }

    #[tokio::test]
    async fn quiesce_times_out_rather_than_forcing_its_way_past_an_upload() {
        let s = shared();
        let entry_id = uuid::Uuid::new_v4();
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(
                    super::super::queue::QueueEntry {
                        entry_id,
                        session_hash: "sha256:seed".to_string(),
                        source: "claude-code".to_string(),
                        project_key: "/tmp/p".to_string(),
                        project_label: "p".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Uploading,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
                        approved_scopes: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                    },
                    500,
                )
                .unwrap();
        }
        let r =
            handle_request_async(&s, &req("quiesce", serde_json::json!({"timeout_secs": 1}))).await;
        let err = r.error.expect("an in-flight upload must not be abandoned");
        assert_eq!(err.code, ERR_BUSY);
        assert_eq!(err.message, ERR_QUIESCE_TIMEOUT);
        // A failed quiesce must leave the daemon working: the update stays
        // staged and retries, rather than parking uploads indefinitely.
        assert!(!s.quiesced.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn quiesce_completes_once_the_in_flight_upload_finishes() {
        let s = std::sync::Arc::new(shared());
        let entry_id = uuid::Uuid::new_v4();
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(
                    super::super::queue::QueueEntry {
                        entry_id,
                        session_hash: "sha256:seed".to_string(),
                        source: "claude-code".to_string(),
                        project_key: "/tmp/p".to_string(),
                        project_label: "p".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Uploading,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
                        approved_scopes: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                    },
                    500,
                )
                .unwrap();
        }
        let finisher = std::sync::Arc::clone(&s);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let mut queue = finisher.queue.lock().unwrap();
            queue.set_state(entry_id, QueueState::Uploaded, None);
        });
        let r = handle_request_async(&s, &req("quiesce", serde_json::json!({"timeout_secs": 10})))
            .await;
        assert_eq!(r.result.expect("drained")["quiesced"], true);
    }

    #[test]
    fn a_synchronous_quiesce_is_refused_rather_than_answered_wrongly() {
        let s = shared();
        let r = handle_request(&s, &req("quiesce", serde_json::json!({})));
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, "quiesce-requires-async");
    }

    #[tokio::test]
    async fn an_absurd_quiesce_timeout_is_capped_rather_than_honoured() {
        let s = shared();
        let r = handle_request_async(
            &s,
            &req("quiesce", serde_json::json!({"timeout_secs": 999_999})),
        )
        .await;
        // The queue is idle, so this returns immediately; the point is that a
        // caller cannot ask the daemon to park uploads for a week.
        assert_eq!(r.result.expect("idle")["quiesced"], true);
        assert_eq!(
            clamp_quiesce_timeout(Some(999_999)),
            MAX_QUIESCE_TIMEOUT_SECS
        );
        assert_eq!(clamp_quiesce_timeout(None), DEFAULT_QUIESCE_TIMEOUT_SECS);
        assert_eq!(clamp_quiesce_timeout(Some(0)), DEFAULT_QUIESCE_TIMEOUT_SECS);
        assert_eq!(clamp_quiesce_timeout(Some(5)), 5);
    }
}
