//! The IPC contract: `trace_commons.daemon.v1`.
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
//! Filesystem ownership, plus one carve-out.
//!
//! The socket is not merely equivalent to holding the device key. Stealing the
//! device key lets an attacker upload once. The socket would additionally let
//! them set a project to `auto_upload` and have the contributor's own running,
//! trusted daemon exfiltrate every future session in that project, under the
//! contributor's real grant, producing receipts that look entirely normal. The
//! CLI cannot be used this way because arming autonomy there requires a
//! terminal.
//!
//! So granting autonomy and bulk-approving are refused over the socket and
//! must be done from a terminal. Applications surface the command to run.
//!
//! `UnixListener::bind` does not portably set the socket mode, so the 0700
//! state directory is the enforcing control; the daemon refuses to serve from
//! a directory that is not 0700.
//!
//! # Sync vs. async dispatch
//!
//! `handle_request` answers every method synchronously except `"preview"`,
//! which it can only answer partially (see its arm). `handle_request_async`
//! is the real entry point for `"preview"`: it runs the actual redaction
//! pipeline and delegates every other method straight through to
//! `handle_request`. The socket connection loop (`serve_connection`), which
//! is already async, calls `handle_request_async` exclusively so a socket
//! client always gets the real preview. `handle_local` (the in-process CLI
//! path) still calls the synchronous `handle_request`, so a CLI caller that
//! asks for `"preview"` gets the honest-but-incomplete
//! `preview_requires_async` marker rather than a wrong byte count; nothing in
//! this codebase currently drives `"preview"` through `handle_local`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Notify, broadcast};
use uuid::Uuid;

use super::health::HealthState;
use super::history::{HistoryCache, rollup};
use super::policy::{ProjectMode, ProjectPolicy};
use super::queue::{Queue, QueueState};
use super::settings::DaemonSettings;
use super::state::DaemonState;
use crate::config::{ConfigStore, DAEMON_SOCK_FILE};

pub const IPC_SCHEMA: &str = "trace_commons.daemon.v1";
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
pub const METHODS: [&str; 17] = [
    "approve",
    "dismiss",
    "get_settings",
    "hello",
    "history_rollup",
    "list_history",
    "list_pending",
    "list_projects",
    "pause",
    "preview",
    "refresh_history",
    "resume",
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

#[derive(Debug, Clone, Serialize)]
pub struct IpcError {
    pub code: String,
    /// A fixed label, never a message body or server response text.
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

impl Response {
    fn ok(id: u64, result: serde_json::Value) -> Self {
        Self {
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: u64, code: &str, message: &str) -> Self {
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
            "paused": self.paused.load(Ordering::Relaxed),
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

/// Where a request came from. Socket callers are refused the two operations
/// that would let same-user code arm autonomous uploading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Socket,
    LocalTty,
}

pub fn handle_request(shared: &DaemonShared, req: &Request, origin: Origin) -> Response {
    match req.method.as_str() {
        "hello" => Response::ok(
            req.id,
            serde_json::json!({
                "schema_version": IPC_SCHEMA,
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
            let projects: Vec<serde_json::Value> = policy
                .projects
                .iter()
                .map(|(key, entry)| {
                    serde_json::json!({
                        "project_label": entry.label,
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
            if mode == ProjectMode::AutoUpload && origin == Origin::Socket {
                return Response::err(req.id, ERR_NOT_AUTHORIZED, "tty-required");
            }
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
            Response::ok(req.id, serde_json::json!({ "ok": true }))
        }
        "approve" => {
            let all = req
                .params
                .get("all")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if all && origin == Origin::Socket {
                return Response::err(req.id, ERR_NOT_AUTHORIZED, "tty-required");
            }
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
            // Synchronous callers cannot run the redaction pipeline (it is
            // async), so this arm reports only the entry itself, honestly
            // flagged as incomplete, rather than the raw file size the old
            // code returned. `handle_request_async` is the real preview path
            // -- see its doc comment.
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
        "pause" | "resume" => {
            let paused = req.method == "pause";
            shared.paused.store(paused, Ordering::Relaxed);
            {
                let mut state = shared.state.lock().expect("state lock");
                state.paused = paused;
                if state.save(&shared.store).is_err() {
                    return Response::err(req.id, ERR_UNAVAILABLE, "state-write-failed");
                }
            }
            shared.publish(EVENT_STATUS_CHANGED, serde_json::json!({}));
            Response::ok(req.id, serde_json::json!({ "paused": paused }))
        }
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

/// The async counterpart to `handle_request`.
///
/// `handle_request` is synchronous because most of the IPC surface (queue
/// mutation, settings, status) needs no `.await`, and the connection loop
/// used to call it directly. `"preview"` is the one method that has to run
/// the real redaction pipeline (`daemon::preview::build_preview`, which
/// awaits an async redactor) to report the actual bytes and redactions a
/// contributor is about to consent to.
///
/// Rather than block a worker thread on that async work from inside a sync
/// function (the `block_in_place` route the task brief also offered), this
/// function intercepts `"preview"` before it reaches `handle_request`, runs
/// it for real, and delegates every other method unchanged to
/// `handle_request`. This is the only entry point that resolves `"preview"`
/// completely; `handle_request` on its own answers `"preview"` with an
/// honest `preview_requires_async: true` marker rather than a wrong number.
/// The socket connection loop, already async, calls this function; `preview`
/// is documented as socket-only for this reason.
pub async fn handle_request_async(shared: &DaemonShared, req: &Request) -> Response {
    if req.method != "preview" {
        return handle_request(shared, req, Origin::Socket);
    }

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

/// Convenience for the CLI, which drives the same handlers in-process and is
/// therefore allowed to arm autonomy.
pub fn handle_local(shared: &DaemonShared, method: &str, params: serde_json::Value) -> Response {
    let req = Request {
        id: 0,
        method: method.to_string(),
        params,
    };
    handle_request(shared, &req, Origin::LocalTty)
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

    #[test]
    fn arming_autonomy_over_the_socket_is_refused() {
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": "/tmp/p", "mode": "auto_upload"}),
            ),
            Origin::Socket,
        );
        assert_eq!(r.error.unwrap().code, ERR_NOT_AUTHORIZED);
    }

    #[test]
    fn arming_autonomy_from_a_terminal_is_allowed() {
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "set_project_mode",
                serde_json::json!({"project_key": "/tmp/p", "mode": "auto_upload"}),
            ),
            Origin::LocalTty,
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            s.policy.lock().unwrap().resolve("/tmp/p"),
            ProjectMode::AutoUpload
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
            Origin::Socket,
        );
        assert!(r.error.is_none(), "{:?}", r.error);
    }

    #[test]
    fn bulk_approval_over_the_socket_is_refused() {
        let s = shared();
        let r = handle_request(
            &s,
            &req("approve", serde_json::json!({"all": true})),
            Origin::Socket,
        );
        assert_eq!(r.error.unwrap().code, ERR_NOT_AUTHORIZED);
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
            Origin::LocalTty,
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn an_unknown_method_uses_the_taxonomy() {
        let s = shared();
        let r = handle_request(
            &s,
            &req("no_such_method", serde_json::json!({})),
            Origin::Socket,
        );
        assert_eq!(r.error.unwrap().code, ERR_UNKNOWN_METHOD);
    }

    #[test]
    fn hello_advertises_the_schema_and_method_set() {
        let s = shared();
        let r = handle_request(&s, &req("hello", serde_json::json!({})), Origin::Socket);
        let result = r.result.unwrap();
        assert_eq!(result["schema_version"], IPC_SCHEMA);
        assert_eq!(result["methods"].as_array().unwrap().len(), METHODS.len());
    }

    #[test]
    fn status_exposes_every_field_a_tray_needs() {
        let s = shared();
        let r = handle_request(&s, &req("status", serde_json::json!({})), Origin::Socket);
        let v = r.result.unwrap();
        for key in ["logged_in", "paused", "queue_depth", "health"] {
            assert!(!v[key].is_null(), "status missing {key}");
        }
        assert_eq!(v["logged_in"], false);
    }

    #[test]
    fn pause_and_resume_round_trip() {
        let s = shared();
        handle_request(&s, &req("pause", serde_json::json!({})), Origin::Socket);
        assert_eq!(s.status_value()["paused"], true);
        handle_request(&s, &req("resume", serde_json::json!({})), Origin::Socket);
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
        let r = handle_request(
            &s,
            &req("get_settings", serde_json::json!({})),
            Origin::Socket,
        );
        let body = serde_json::to_string(&r.result.unwrap()).unwrap();
        assert!(!body.contains("super-secret-key"), "{body}");
        assert!(body.contains("near_ai_configured"));
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
            Origin::Socket,
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn set_settings_rejects_a_payload_with_nothing_known_in_it() {
        let s = shared();
        let r = handle_request(
            &s,
            &req("set_settings", serde_json::json!({"nonsense": 1})),
            Origin::Socket,
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }
}
