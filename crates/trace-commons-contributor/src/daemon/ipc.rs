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
//! control, and is not claimed to be one.
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

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, broadcast};
use uuid::Uuid;

use super::audit::{self, AuditEntry};
use super::enroll;
use super::health::HealthState;
use super::history::{HistoryCache, rollup};
use super::policy::{ProjectMode, ProjectPolicy, disambiguated_label, known_keys};
use super::queue::{Queue, QueueState};
use super::settings::DaemonSettings;
use super::state::DaemonState;
use crate::config::{ConfigStore, DAEMON_SOCK_FILE};

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

pub const ERR_UNKNOWN_METHOD: &str = "unknown_method";
pub const ERR_BAD_PARAMS: &str = "bad_params";
pub const ERR_NOT_AUTHORIZED: &str = "not_authorized";
pub const ERR_BUSY: &str = "busy";
pub const ERR_UNAVAILABLE: &str = "unavailable";

/// Every method this version answers. `hello` reports this list, and the
/// contract document is checked against it by test.
pub const METHODS: [&str; 24] = [
    "acknowledge_near_ai_notice",
    "approve",
    "cancel",
    "consent_options",
    "dismiss",
    "enroll",
    "get_settings",
    "hello",
    "history_rollup",
    "list_audit",
    "list_history",
    "list_pending",
    "list_projects",
    "pause",
    "preview",
    "queue_outcome_counts",
    "refresh_history",
    "resume",
    "set_consent_scopes",
    "set_project_mode",
    "set_settings",
    "shutdown",
    "status",
    "subscribe",
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
        let queue = Queue::load(&store)?;
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
pub fn entry_value(e: &super::queue::QueueEntry) -> serde_json::Value {
    serde_json::json!({
        "entry_id": e.entry_id,
        "session_hash": e.session_hash,
        "source": e.source,
        "project_label": e.project_label,
        "size_bytes": e.size_bytes,
        "discovered_at": e.discovered_at,
        "state": e.state,
        "reason_label": e.reason_label,
        "attempts": e.attempts,
        "retry_after": e.retry_after,
        "submission_id": e.submission_id,
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
        "list_projects" => {
            let policy = shared.policy.lock().expect("policy lock");
            let queue = shared.queue.lock().expect("queue lock");
            let known = known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()));
            let projects: Vec<serde_json::Value> = policy
                .projects
                .iter()
                .map(|(key, entry)| {
                    serde_json::json!({
                        "project_label": disambiguated_label(key, &known),
                        "mode": policy.resolve(key),
                        "added_at": entry.added_at,
                    })
                })
                .collect();
            Response::ok(req.id, serde_json::json!({ "projects": projects }))
        }
        "set_project_mode" => {
            let Some(key) = req.params.get("project_key").and_then(|v| v.as_str()) else {
                return Response::err(req.id, ERR_BAD_PARAMS, "project_key-required");
            };
            let mode: ProjectMode = match req
                .params
                .get("mode")
                .cloned()
                .map(serde_json::from_value::<ProjectMode>)
            {
                Some(Ok(m)) => m,
                _ => return Response::err(req.id, ERR_BAD_PARAMS, "mode-invalid"),
            };
            let label = req
                .params
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(key);
            let mut policy = shared.policy.lock().expect("policy lock");
            if let Err(e) = policy.set_mode(key, label, mode, Utc::now()) {
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
            let mut queue = shared.queue.lock().expect("queue lock");
            if relabel_queue_entries(&policy, &mut queue) {
                if let Err(_e) = queue.save(&shared.store) {
                    return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
                }
                shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
            }
            if mode == ProjectMode::AutoUpload {
                // A local, label-only record that autonomy was armed for
                // this project. This is visibility, not a security control
                // -- see `daemon::audit`.
                let known = known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()));
                let audit_label = disambiguated_label(key, &known);
                drop(queue);
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
                    tracing::warn!("failed to append daemon audit entry");
                }
            }
            Response::ok(req.id, serde_json::json!({ "ok": true }))
        }
        "approve" => {
            let all = req
                .params
                .get("all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let mut queue = shared.queue.lock().expect("queue lock");
            let ids: Vec<Uuid> = if all {
                queue.pending().iter().map(|e| e.entry_id).collect()
            } else {
                match parse_entry_id(&req.params) {
                    Ok(id) => vec![id],
                    Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
                }
            };
            let mut approved = 0;
            for id in ids {
                if queue.get(id).map(|e| e.state) == Some(QueueState::Pending) {
                    queue.set_state(id, QueueState::Approved, None);
                    approved += 1;
                }
            }
            if let Err(_e) = queue.save(&shared.store) {
                return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
            }
            drop(queue);
            shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
            if all {
                // A local, label-only record that the whole queue was
                // bulk-approved. This is visibility, not a security control
                // -- see `daemon::audit`.
                if let Err(_e) = audit::append(
                    &shared.store,
                    &AuditEntry {
                        at: Utc::now(),
                        action: "bulk-approved".to_string(),
                        project_label: None,
                        detail: Some(approved.to_string()),
                    },
                ) {
                    tracing::warn!("failed to append daemon audit entry");
                }
            }
            Response::ok(req.id, serde_json::json!({ "approved": approved }))
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
                None => Response::err(req.id, ERR_BAD_PARAMS, "unknown-entry-id"),
            }
        }
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
            Ok(records) => Response::ok(
                req.id,
                serde_json::to_value(rollup(&records, Utc::now()))
                    .unwrap_or_else(|_| serde_json::json!({})),
            ),
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
            let mut changed = false;
            if let Some(v) = req.params.get("quiescence_secs").and_then(|v| v.as_u64()) {
                settings.quiescence_secs = v;
                changed = true;
            }
            if let Some(v) = req
                .params
                .get("digest_interval_secs")
                .and_then(|v| v.as_u64())
            {
                settings.digest_interval_secs = v;
                changed = true;
            }
            if let Some(v) = req
                .params
                .get("local_notifications")
                .and_then(|v| v.as_bool())
            {
                settings.local_notifications = v;
                changed = true;
            }
            if !changed {
                return Response::err(req.id, ERR_BAD_PARAMS, "no-known-setting-supplied");
            }
            if let Err(_e) = settings.save(&shared.store) {
                return Response::err(req.id, ERR_UNAVAILABLE, "settings-write-failed");
            }
            Response::ok(req.id, redacted_settings(&settings))
        }
        "shutdown" => {
            shared.shutdown.store(true, Ordering::Relaxed);
            shared.shutdown_signal.notify_one();
            Response::ok(req.id, serde_json::json!({ "stopping": true }))
        }
        // subscribe is handled by the connection loop, which owns the stream.
        "subscribe" => Response::ok(req.id, serde_json::json!({ "subscribed": true })),
        _ => Response::err(req.id, ERR_UNKNOWN_METHOD, "unknown-method"),
    }
}

/// The complete dispatcher: answers the async methods (`"preview"`,
/// `"enroll"`) for real and delegates every other method, unchanged, to the
/// synchronous `handle_request`. See the module doc's "Sync vs. async
/// dispatch" section for why this is the only place that decides which
/// methods are async, and why both real callers (the socket loop and
/// `handle_local`) always go through this function rather than
/// `handle_request` directly.
pub async fn handle_request_async(shared: &DaemonShared, req: &Request) -> Response {
    match req.method.as_str() {
        "preview" => handle_preview(shared, req).await,
        "enroll" => enroll::handle_enroll(shared, req).await,
        _ => handle_request(shared, req),
    }
}

/// Run the real, async redaction pipeline for one queue entry and report the
/// actual bytes and redactions a contributor is about to consent to.
///
/// `handle_request` cannot run this (it is synchronous) and answers
/// `"preview"` on its own with an honest `preview_requires_async: true`
/// marker rather than a wrong byte count; only `handle_request_async`
/// resolves it completely.
async fn handle_preview(shared: &DaemonShared, req: &Request) -> Response {
    let id = match parse_entry_id(&req.params) {
        Ok(id) => id,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    let entry = {
        let queue = shared.queue.lock().expect("queue lock");
        match queue.get(id) {
            Some(e) => e.clone(),
            None => return Response::err(req.id, ERR_BAD_PARAMS, "unknown-entry-id"),
        }
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    let (near_ai, claude_root, codex_root) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.near_ai.clone(),
            s.claude_root.clone(),
            s.codex_root.clone(),
        )
    };
    let sources = crate::source::all_sources(claude_root, codex_root, None);
    let Some((source, session_ref)) = super::find_session(&sources, &entry) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "session-file-vanished");
    };

    match super::preview::build_preview(&shared.store, &cfg, near_ai, source, &session_ref).await {
        Ok((summary, _body)) => Response::ok(
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
            }),
        ),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "preview-failed"),
    }
}

/// Full preview -- summary *and* redacted body -- for one queue entry, for a
/// caller that already holds `shared` directly rather than issuing a
/// request/response frame. This is what the C ABI's `tc_preview_open` uses.
///
/// The socket's `"preview"` (`handle_preview`, above) returns the summary
/// only and never the body: per the design's "preview is a local operation,
/// not daemon state" section, a body only ever needs to leave this process
/// through a return value a caller who is already inside it can hold a
/// pointer to, never through the 1 MiB-capped socket. Errors are fixed
/// labels, matching every other surface at this boundary -- no path, no
/// entry content.
pub async fn open_preview(
    shared: &DaemonShared,
    entry_id: Uuid,
) -> Result<(super::preview::PreviewSummary, String), &'static str> {
    let entry = {
        let queue = shared.queue.lock().expect("queue lock");
        queue.get(entry_id).cloned().ok_or("unknown-entry-id")?
    };
    let cfg = shared
        .store
        .load_config()
        .map_err(|_| "not-logged-in")?
        .ok_or("not-logged-in")?;
    let (near_ai, claude_root, codex_root) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.near_ai.clone(),
            s.claude_root.clone(),
            s.codex_root.clone(),
        )
    };
    let sources = crate::source::all_sources(claude_root, codex_root, None);
    let (source, session_ref) =
        super::find_session(&sources, &entry).ok_or("session-file-vanished")?;
    super::preview::build_preview(&shared.store, &cfg, near_ai, source, &session_ref)
        .await
        .map_err(|_| "preview-failed")
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
        let claude_root_configured = s.claude_root.is_some();
        let codex_root_configured = s.codex_root.is_some();
        obj.remove("claude_root");
        obj.remove("codex_root");
        obj.insert(
            "claude_root_configured".to_string(),
            serde_json::Value::Bool(claude_root_configured),
        );
        obj.insert(
            "codex_root_configured".to_string(),
            serde_json::Value::Bool(codex_root_configured),
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
const MAX_SOCKET_PATH_BYTES: usize = 104;

/// Bind the daemon socket, refusing unless the state directory is private.
pub async fn bind(store: &ConfigStore) -> Result<UnixListener> {
    ensure_private_dir(store.dir())?;
    let path = store.daemon_path(DAEMON_SOCK_FILE);

    // The kernel truncates rather than explains, and the resulting error names
    // a constant most people have never heard of. Say what is actually wrong
    // and what to do about it.
    let len = path.as_os_str().len();
    if len >= MAX_SOCKET_PATH_BYTES {
        bail!(
            "the daemon socket path is {len} bytes, over the {MAX_SOCKET_PATH_BYTES}-byte \
             kernel limit for unix sockets:\n  {}\nUse a shorter state directory, \
             e.g. TRACE_COMMONS_CONTRIBUTOR_DIR=~/.config/trace-commons",
            path.display()
        );
    }

    // A socket left behind by a crashed daemon would block binding. The
    // single-instance lock, not this file, is what prevents two daemons.
    let _ = std::fs::remove_file(&path);
    UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))
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
    let meta = std::fs::metadata(dir)
        .with_context(|| format!("reading permissions of {}", dir.display()))?;
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

#[cfg(not(unix))]
fn ensure_private_dir(_dir: &std::path::Path) -> Result<()> {
    bail!("the daemon socket is unix-only in this version")
}

/// Serve connections until shutdown is requested.
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

pub async fn serve_connection(stream: UnixStream, shared: Arc<DaemonShared>) -> Result<()> {
    let (read_half, mut write_half) = stream.into_split();
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

async fn write_json<T: Serialize>(
    w: &mut tokio::net::unix::OwnedWriteHalf,
    value: &T,
) -> Result<()> {
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
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": "/tmp/p", "mode": "auto_upload"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            s.policy.lock().unwrap().resolve("/tmp/p"),
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
                serde_json::json!({"project_key": "/tmp/p", "mode": "auto_upload"}),
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
        handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": "/Users/z/work/api", "mode": "notify_only"}),
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
                        project_key: "/Users/z/work/api".to_string(),
                        project_label: "api".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Pending,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
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
                serde_json::json!({"project_key": "/Users/z/client/api", "mode": "ignore"}),
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
                serde_json::json!({"project_key": "/tmp/p", "mode": "notify_only"}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
    }

    #[test]
    fn bulk_approval_over_the_socket_is_now_allowed_and_appends_an_audit_entry() {
        // As with arming autonomy, the terminal-only gate on bulk approval
        // is removed for the same reason: it restricted nothing an attacker
        // with same-user code execution did not already have. The audit
        // entry is the replacement -- visibility, not a control.
        let s = shared();
        let r = handle_request(&s, &req("approve", serde_json::json!({"all": true})));
        assert!(r.error.is_none(), "{:?}", r.error);
        let entries = audit::load(&s.store).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "bulk-approved");
        assert_eq!(entries[0].project_label, None);
    }

    #[test]
    fn a_single_entry_approval_leaves_the_audit_log_empty() {
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
                    },
                    500,
                )
                .unwrap();
        }
        let r = handle_request(
            &s,
            &req(
                "approve",
                serde_json::json!({"entry_id": entry_id.to_string()}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(audit::load(&s.store).unwrap().is_empty());
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
            settings.claude_root = Some(std::path::PathBuf::from("/Users/z/.claude/projects"));
            settings.codex_root = Some(std::path::PathBuf::from("/Users/z/.codex/sessions"));
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
        };
        let body = serde_json::to_string(&entry_value(&e)).unwrap();
        assert!(
            !body.contains("/Users/z"),
            "path leaked to the wire: {body}"
        );
        assert!(body.contains("secret-client-project"));
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
                serde_json::json!({"project_key": "/tmp/p", "mode": "auto_upload"}),
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
        for key in ["/tmp/a", "/tmp/b", "/tmp/c"] {
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
                serde_json::json!({"project_key": "/tmp/p", "mode": "auto_upload"}),
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
}
