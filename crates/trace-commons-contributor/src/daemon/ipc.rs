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
use std::sync::{Arc, Mutex, RwLock};

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
use super::preview_scheduler::{PreviewKey, PreviewScheduler, RequestState};
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
/// `approve`'s `outcome` named something other than `worked`, `partly` or
/// `failed`. Refused, not coerced to `Unknown`, so a typo does not silently
/// discard the contributor's answer -- the same rule the `--outcome` flag
/// applies.
pub const ERR_BAD_VERDICT: &str = "outcome-invalid";
/// `approve`'s `correction` was not a string.
pub const ERR_BAD_CORRECTION: &str = "correction-invalid";
/// `approve` carried a correction without a `partly` or `failed` outcome.
///
/// The shells only show the field for those two verdicts, and the same rule
/// is enforced here rather than trusted to them. A run the contributor has
/// just called successful has nothing to correct, and refusing the
/// combination halves the surface for correction-shaped credit farming --
/// see the S5 design note.
pub const ERR_CORRECTION_NEEDS_VERDICT: &str = "correction-needs-outcome";
/// `approve` carried a correction longer than
/// `envelope::MAX_CORRECTION_CHARS`.
pub const ERR_CORRECTION_TOO_LONG: &str = "correction-too-long";
/// `approve` carried a correction with `all` or `project_id`.
///
/// A correction is written about one session. Applying one string to a whole
/// batch would attach an explanation to sessions it was not written about,
/// and every one of them would carry it into the corpus as the
/// contributor's own words.
pub const ERR_CORRECTION_NEEDS_ENTRY: &str = "correction-needs-entry-id";
/// The label an entry is skipped under when credential detection fired on
/// the correction the contributor wrote for it.
///
/// Nothing is approved and nothing is sent. The contributor is told to
/// remove the credential and, because it has already been typed, to rotate
/// it -- masking it and sending it on would leave a live credential
/// transmitted and its owner unaware. The label names the condition and
/// never the text or the match.
pub const REASON_CORRECTION_CREDENTIAL: &str = crate::envelope::REASON_CORRECTION_CREDENTIAL;

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
/// A slice rather than a fixed-size array: `serde` implements `Serialize`
/// for arrays only up to 32 elements, and `hello` serializes this list
/// directly. The length is still checked against the contract document by
/// test.
pub const METHODS: &[&str] = &[
    "acknowledge_near_ai_notice",
    "approve",
    "cancel",
    "clear_public_profile",
    "consent_options",
    "discover_routing",
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
    "preview_cancel",
    "preview_request",
    "preview_turns",
    "preview_visible",
    "probe_routing",
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
/// One entry's scheduled preview finished. Carries the same object
/// `preview_request` returns for a cache hit: `entry_id`, `state`, and
/// whichever of `summary` / `raw_session_bytes` / `code` that state has.
///
/// Published for every job the scheduler completes and delivers, including
/// refusals -- a shell that showed a spinner needs to be told the answer is
/// "too large" just as much as it needs to be told the summary. It is *not*
/// published for a job that was cancelled while it ran; see
/// `preview_scheduler::PreviewScheduler::cancel`.
pub const EVENT_PREVIEW_READY: &str = "preview_ready";

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
    /// The daemon-wide bound on concurrent preview work.
    ///
    /// `Arc` rather than a plain field because the worker pool outlives any
    /// one request and is spawned from `daemon::start_embedded` with only
    /// this handle -- the pool must not hold an `Arc<DaemonShared>` through
    /// the scheduler, or the two would keep each other alive forever.
    pub previews: Arc<PreviewScheduler>,
    /// The routing overlay's one long-lived instance, built from the
    /// declaration in settings and refreshed on the poll tick.
    ///
    /// `None` is the majority case -- no proxy declared. Settings keep
    /// describing the *declaration* (`DaemonSettings::source_roots` stays
    /// bare); this is the single place the *instance* lives, because a
    /// ledger built per call would hand every caller a cold, empty snapshot
    /// -- see [`Self::source_roots_with_routing`].
    ///
    /// Behind an `RwLock` because the declaration can change while the
    /// daemon runs: `set_settings` rebuilds this in place (see
    /// [`Self::rebuild_routing`]). Telling a contributor to restart the
    /// daemon because they typed a port number is the friction that makes a
    /// feature feel broken. Readers take the lock only long enough to clone
    /// the `Arc` out -- nothing holds it across an await, and nothing holds
    /// it while doing I/O.
    routing: RwLock<Option<Arc<crate::routing::ironwire::IronWireLedger>>>,
    /// Whether the last state this daemon reported for `routing` was
    /// "has rows". Compared against on every refresh so a transition is
    /// reported once, not on every poll -- see [`Self::routing_transition`].
    routing_had_rows: AtomicBool,
}

/// `status.routing.state`: the contributor never declared a proxy.
///
/// The state `IronWireLedger::has_rows` cannot express. A daemon holding no
/// ledger and a daemon holding a ledger that has read nothing both report
/// "no rows", and collapsing them tells a contributor who declared a proxy
/// that everything is fine when the declaration never took.
pub const ROUTING_NOT_DECLARED: &str = "not_declared";

/// `status.routing.state`: a proxy is declared and the daemon holds a
/// ledger, but no row has arrived yet.
///
/// **Not an error.** A proxy installed this morning reports this, and so
/// does one whose declaration was changed a second ago -- a rebuilt ledger
/// starts cold by construction. A shell that renders this as a fault will
/// be wrong on both. `routing.last_refresh_at` is what separates "the proxy
/// answered and had nothing" from "nothing has answered yet".
pub const ROUTING_AWAITING_ROWS: &str = "awaiting_rows";

/// `status.routing.state`: a proxy is declared and rows have been read.
pub const ROUTING_ROWS_SEEN: &str = "rows_seen";

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
        // Release the pins on stored previews nobody is waiting on before
        // sweeping, so a store left over from an earlier version of the
        // daemon -- previews written for entries the contributor never
        // asked about, all of them still pending -- drains on the first
        // start rather than waiting for a tick.
        if !super::approved_envelope::release_stale_pins(&store, &mut queue).is_empty() {
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
        // Built here from the declaration this settings file carries at
        // startup. A later edit does not wait for a restart:
        // `set_settings` rebuilds the instance in place.
        let routing = RwLock::new(super::settings::ironwire_ledger_for(
            settings.ironwire.as_ref(),
        ));
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
            previews: Arc::new(PreviewScheduler::default()),
            routing,
            routing_had_rows: AtomicBool::new(false),
        })
    }

    /// Source roots with the daemon's live routing ledger attached.
    ///
    /// Settings describe the declaration; the daemon owns the instance.
    /// Building a ledger per call would hand every caller its own cold
    /// snapshot, which is the defect this helper exists to prevent -- see
    /// `DaemonSettings::source_roots`, which stays bare on purpose.
    pub(crate) fn source_roots_with_routing(&self) -> crate::source::SourceRoots {
        let roots = {
            let s = self.settings.lock().expect("settings lock");
            s.source_roots(&self.store)
        };
        let ledger = self
            .routing_ledger()
            .map(|l| l as Arc<dyn crate::routing::RoutingLedger>);
        roots.with_routing(ledger)
    }

    /// The routing ledger the daemon currently holds, if any.
    ///
    /// Every reader goes through here rather than touching the lock, so the
    /// guard is never held for longer than the clone -- in particular never
    /// across the await in [`Self::refresh_routing`], where holding it would
    /// block `set_settings` for the length of a network call.
    pub(crate) fn routing_ledger(&self) -> Option<Arc<crate::routing::ironwire::IronWireLedger>> {
        self.routing
            .read()
            .expect("routing lock")
            .as_ref()
            .map(Arc::clone)
    }

    /// Rebuild the held ledger from a declaration, replacing whatever was
    /// there.
    ///
    /// The rebuilt instance starts **cold** -- an empty snapshot until the
    /// next refresh. That is correct and it is not an error: it is exactly
    /// the "declared, nothing seen yet" state `status_value` reports, and
    /// the same state a machine whose proxy was installed this morning is
    /// in. It also resets the transition state, so the first refresh after
    /// a declaration change logs the new ledger's data state rather than
    /// comparing it against the old one's.
    pub(crate) fn rebuild_routing(
        &self,
        declaration: Option<&super::settings::IronWireDeclaration>,
    ) {
        let rebuilt = super::settings::ironwire_ledger_for(declaration);
        *self.routing.write().expect("routing lock") = rebuilt;
        self.routing_had_rows.store(false, Ordering::Relaxed);
    }

    /// Refresh the routing ledger, if the contributor declared one, and
    /// report a data-state transition.
    ///
    /// Infallible by construction: `IronWireLedger::refresh` never returns
    /// an error and carries its own short timeout, so awaiting it here
    /// cannot fail or stall the poll tick that calls this.
    pub(crate) async fn refresh_routing(&self) {
        let Some(ledger) = self.routing_ledger() else {
            return;
        };
        ledger.refresh().await;
        if let Some(has_rows) = self.routing_transition(ledger.has_rows()) {
            // Hash-only by construction: `has_rows` is a bool, and nothing
            // else about the ledger -- port, token, row contents -- appears
            // here. A machine whose proxy was installed today legitimately
            // logs `has_rows=false` once; that is not an error.
            tracing::info!(has_rows, "routing ledger data state changed");
        }
    }

    /// Whether `has_rows` differs from the state last reported for the
    /// routing ledger, updating the reported state either way.
    ///
    /// Returns the new state only on an actual change, so a caller logs
    /// once per transition rather than once per poll -- the same shape as
    /// `HealthState::fail`/`resolve`, but for a condition that is not an
    /// error and must never be treated as one.
    fn routing_transition(&self, has_rows: bool) -> Option<bool> {
        let previous = self.routing_had_rows.swap(has_rows, Ordering::Relaxed);
        (previous != has_rows).then_some(has_rows)
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

    /// Today's volume budget, and how much approved work it is holding.
    ///
    /// Read-only: `budget_snapshot` rolls the day on a copy, so polling
    /// `status` cannot move the daemon's own counters.
    ///
    /// Approved entries never appear on `list_pending` -- that method
    /// returns `Pending` only -- so a shell has no row it could annotate.
    /// A count and a byte total on `status` are the only place the
    /// condition can be told, which is why it lands here rather than on a
    /// queue entry.
    pub fn daily_budget(&self, now: chrono::DateTime<Utc>) -> super::uploader::DailyBudget {
        let approved: Vec<super::queue::QueueEntry> = {
            let queue = self.queue.lock().expect("queue lock");
            queue
                .all()
                .iter()
                .filter(|e| e.state == super::queue::QueueState::Approved)
                .cloned()
                .collect()
        };
        let state = self.state.lock().expect("state lock");
        let settings = self.settings.lock().expect("settings lock");
        super::uploader::budget_snapshot(&approved, &state, &settings, now)
    }

    /// The tray's whole world in one object.
    pub fn status_value(&self) -> serde_json::Value {
        let now = Utc::now();
        // Taken before the queue lock below, and released with it, because
        // `daily_budget` takes the queue, state, and settings locks itself.
        let budget = self.daily_budget(now);
        // Taken before the locks below for the same reason as `budget`:
        // this reads the routing lock, and taking locks in one order
        // everywhere is what keeps that safe.
        let routing = self.routing_value();
        let queue = self.queue.lock().expect("queue lock");
        let health = self.health.lock().expect("health lock");
        let cfg = self.store.load_config().ok().flatten();
        serde_json::json!({
            "schema_version": IPC_SCHEMA,
            "logged_in": self.logged_in(),
            "tenant_id": cfg.as_ref().map(|c| c.tenant_id.clone()),
            "consent_scopes": cfg.as_ref().map(|c| c.consent_scopes.clone()).unwrap_or_default(),
            "paused": self.is_paused(now),
            "queue_depth": queue.pending().len(),
            "next_digest_at": self.next_digest_at(),
            "health": {
                "last_error_label": health.last_error_label,
                "since": health.since,
            },
            // Additive. The daily cap is enforced whatever else is wrong,
            // and `health` can only carry one label at a time, so this is
            // reported independently of it -- see `DailyBudget`.
            "daily_budget": {
                "bytes_today": budget.bytes_today,
                "max_bytes_per_day": budget.max_bytes_per_day,
                "bytes_remaining": budget.bytes_remaining,
                "uploads_today": budget.uploads_today,
                "max_uploads_per_day": budget.max_uploads_per_day,
                "uploads_remaining": budget.uploads_remaining,
                "resets_at": budget.resets_at,
                "blocked": budget.blocked(),
                "blocked_entries": budget.blocked_entries,
                "blocked_bytes": budget.blocked_bytes,
            },
            // Additive, and three-valued on purpose. `health` reports
            // faults; none of these three is one, so like `daily_budget`
            // this sits beside it rather than inside it.
            "routing": routing,
        })
    }

    /// The `routing` sub-object of [`Self::status_value`].
    ///
    /// Three states, because the two a boolean can carry are the defect:
    /// "no proxy declared" and "declared but reading nothing" are different
    /// situations with different answers, and only the second is worth
    /// telling a contributor about. `has_rows` alone cannot tell them apart
    /// -- the distinction lives in the `Option` on shared state, which is
    /// why this reads the held instance rather than the settings blob.
    ///
    /// `last_refresh_at` is reported alongside because `has_rows` is a poor
    /// health signal on its own: it says data exists, not that the proxy
    /// answers now. A proxy that died an hour ago still has rows.
    fn routing_value(&self) -> serde_json::Value {
        let Some(ledger) = self.routing_ledger() else {
            return serde_json::json!({
                "state": ROUTING_NOT_DECLARED,
                "last_refresh_at": serde_json::Value::Null,
            });
        };
        serde_json::json!({
            "state": if ledger.has_rows() { ROUTING_ROWS_SEEN } else { ROUTING_AWAITING_ROWS },
            "last_refresh_at": ledger.last_refresh_at(),
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
        // Beside `source`, never replacing it: a consumer uses the adapter
        // name to ask for the same session again, while this is what the
        // conversation says it came from. See `QueueEntry::declared_source`.
        "declared_source": e.declared_source,
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
            // Ignoring a project clears what it already has waiting. Doing
            // it here rather than in the UI means Settings, onboarding and
            // the CLI all get it: before this, ignoring from Settings left
            // the contributor staring at the cards they had just declined.
            //
            // Pending only. See `refuse_pending_for_project`.
            //
            // Leaving `Ignore` undoes exactly that, and only that: see
            // `clear_project_ignored`, which is what makes the
            // confirmation's "You can undo this in Settings" true for a
            // *finished* session -- the ordinary case, and the one the
            // ignore was aimed at. It is the same arm because the two are
            // one setting, and every route that can set it (Settings,
            // onboarding, the CLI, the Waiting screen) must get both halves.
            //
            // The policy is already saved at this point, so a `queue.save`
            // failure below leaves disk disagreeing with memory: the project
            // is durably `Ignore` while its entries are still durably
            // `Pending`, and a restart brings the cleared cards back. The
            // error is reported and the daemon keeps the in-memory truth, so
            // the contributor sees the right thing until then. Ordering the
            // two writes the other way does not help -- the relabel below
            // reads the *new* policy, so the queue cannot be written first --
            // and a real fix wants both files under one atomic write, which
            // the store does not offer.
            let (queue_changed, purged) = {
                let mut queue = shared.queue.lock().expect("queue lock");
                let purged = if mode == ProjectMode::Ignore {
                    queue.refuse_pending_for_project(&key)
                } else {
                    0
                };
                let restored = if mode == ProjectMode::Ignore {
                    0
                } else {
                    queue.clear_project_ignored(&key)
                };
                let relabelled = relabel_queue_entries(&policy, &mut queue);
                if relabelled || purged > 0 || restored > 0 {
                    if let Err(_e) = queue.save(&shared.store) {
                        return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
                    }
                }
                (relabelled || restored > 0, purged)
            };
            drop(policy);
            if queue_changed || purged > 0 {
                shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
            }
            Response::ok(req.id, serde_json::json!({ "ok": true, "purged": purged }))
        }
        "dismiss" => {
            let id = match parse_entry_id(&req.params) {
                Ok(id) => id,
                Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
            };
            // A dismissed entry is never previewed again, so drop any
            // scheduled preview for it. If one is already building this
            // cannot stop it mid-parse -- see `PreviewScheduler::cancel` --
            // but the result is discarded rather than delivered or cached,
            // which is the part that matters for a trace just declined.
            shared.previews.cancel(id);
            let mut queue = shared.queue.lock().expect("queue lock");
            queue.set_state(
                id,
                QueueState::Refused,
                Some(super::queue::REASON_DISMISSED.to_string()),
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
        // The probe opens a loopback connection to the proxy, which the
        // synchronous dispatcher cannot do. It refuses rather than
        // reporting a state it never checked -- a probe that answered
        // without asking would be exactly the silence this method exists
        // to end.
        "probe_routing" => Response::err(req.id, ERR_UNAVAILABLE, "probe-routing-requires-async"),
        // Unlike the probe, discovery opens no connection: it reads one
        // small file the proxy left on disk. So it answers here, on the
        // synchronous path, and a shell can call it before it has anything
        // to declare.
        "discover_routing" => handle_discover_routing(req),
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
            // `project_id` is the batch form of `cancel`, mirroring
            // `approve`'s selector shape so an Undo of a project-wide
            // approval is one call with the same argument -- the daemon
            // decides which entries it covers, rather than the shell
            // deriving "what I saw pending minus what was reported
            // skipped" and racing the queue to cancel them one at a time.
            // `project_id` and `entry_id` are mutually exclusive;
            // `project_id` wins when both are sent, the same precedence
            // `approve` uses between its own selectors.
            let project_id = req.params.get("project_id").and_then(|v| v.as_str());
            if let Some(pid) = project_id {
                // Unrecognized is refused exactly as `approve` refuses it
                // for the same selector: a handle the caller never
                // received is a client bug, and answering `canceled: 0`
                // would be indistinguishable from "that project had
                // nothing to cancel". Lock order is policy before queue,
                // as everywhere else.
                let policy = shared.policy.lock().expect("policy lock");
                let mut queue = shared.queue.lock().expect("queue lock");
                let known = known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()));
                let Some(key) = project_key_for_id(pid, &known) else {
                    return Response::err(req.id, ERR_BAD_PARAMS, ERR_PROJECT_ID_UNRECOGNIZED);
                };
                // Only `Approved`: those are the entries an Undo would be
                // undoing. A `Pending` entry has nothing to cancel, and a
                // project-wide call must not touch it. Matched by the id
                // `entry_value` publishes, never `project_label`, same as
                // `approve`.
                let ids: Vec<Uuid> = queue
                    .all()
                    .iter()
                    .filter(|e| e.project_key == key && e.state == QueueState::Approved)
                    .map(|e| e.entry_id)
                    .collect();
                let project_audit_label = disambiguated_label(&key, &known);
                // Same ordering as `approve`'s `bulk-approved` row: written
                // before anything is canceled, so a rollback never has to
                // write to the disk that just refused a write. Undoing a
                // batch is the same class of act as approving one -- bulk,
                // unattended, previously terminal-only -- so it gets the
                // same visibility. A selector that matched nothing writes
                // no row, for the same reason `approve` writes none: no
                // entries were canceled, and a shell polling an empty
                // project would rotate real records out of a capped log.
                if !ids.is_empty() {
                    if let Err(_e) = audit::append(
                        &shared.store,
                        &AuditEntry {
                            at: Utc::now(),
                            action: "bulk-canceled".to_string(),
                            project_label: Some(project_audit_label),
                            detail: Some(ids.len().to_string()),
                        },
                    ) {
                        return Response::err(req.id, ERR_UNAVAILABLE, "audit-write-failed");
                    }
                }
                let mut canceled = 0usize;
                for id in &ids {
                    // Selection and cancellation happen under one
                    // continuous hold of the queue lock, so every id
                    // selected above is still `Approved` when `cancel`
                    // runs on it; nothing else can have moved it.
                    if queue.cancel(*id).is_ok() {
                        canceled += 1;
                    }
                }
                if let Err(_e) = queue.save(&shared.store) {
                    return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
                }
                drop(queue);
                shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
                return Response::ok(req.id, serde_json::json!({ "canceled": canceled }));
            }
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
            // Captured before the write so the ledger is rebuilt only when
            // the declaration actually moved. Rebuilding on every settings
            // write would blank a warm snapshot whenever a contributor
            // touched an unrelated slider.
            let declared_before = settings.ironwire.clone();
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
                    // Apply the proxy declaration to this running daemon.
                    // Without this the contributor would have to restart the
                    // daemon to make a port they just typed take effect,
                    // which reads as the feature being broken. Done after
                    // the save so a declaration that takes effect is always
                    // one that survives a restart too.
                    if settings.ironwire != declared_before {
                        shared.rebuild_routing(settings.ironwire.as_ref());
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
        "preview_request" => handle_preview_request(shared, req),
        "preview_visible" => handle_preview_visible(shared, req),
        "preview_cancel" => handle_preview_cancel(shared, req),
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
/// `"preview"`, `"preview_body"`, `"preview_turns"`, `"probe_routing"`,
/// `"quiesce"`, `"enroll"`,
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
        "probe_routing" => handle_probe_routing(req).await,
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
    // Validated up front, before anything is approved and before the
    // bulk-approval audit row is written. A refused call must approve
    // nothing and leave no record of a batch that did not happen.
    //
    // Refused rather than ignored: a contributor who answered meant to say
    // something, and coercing a typo to `Unknown` would silently discard
    // the answer. Same rule the `--outcome` flag applies.
    let verdict = match req.params.get("outcome") {
        None => None,
        Some(v) => {
            let Some(name) = v.as_str() else {
                return Response::err(req.id, ERR_BAD_PARAMS, ERR_BAD_VERDICT);
            };
            if crate::envelope::ContributorVerdict::parse(name).is_none() {
                return Response::err(req.id, ERR_BAD_PARAMS, ERR_BAD_VERDICT);
            }
            Some(name.to_string())
        }
    };
    // Validated in the same place and for the same reason as the verdict:
    // before anything is approved, before the audit row, and refused rather
    // than coerced.
    //
    // Whitespace is not a correction, so it is normalised away here and the
    // call proceeds as an ordinary uncorrected approval. Everything else is
    // a refusal the caller has to see: a correction the daemon silently
    // dropped is worse than one it declined, because the contributor was
    // shown a caption promising their words would be stored as written.
    let correction = match req.params.get("correction") {
        None => None,
        Some(v) => {
            let Some(text) = v.as_str() else {
                return Response::err(req.id, ERR_BAD_PARAMS, ERR_BAD_CORRECTION);
            };
            let text = text.trim();
            if text.is_empty() {
                None
            } else {
                if text.chars().count() > crate::envelope::MAX_CORRECTION_CHARS {
                    return Response::err(req.id, ERR_BAD_PARAMS, ERR_CORRECTION_TOO_LONG);
                }
                Some(text.to_string())
            }
        }
    };
    if correction.is_some() {
        if !matches!(verdict.as_deref(), Some("partly") | Some("failed")) {
            return Response::err(req.id, ERR_BAD_PARAMS, ERR_CORRECTION_NEEDS_VERDICT);
        }
        if all || req.params.get("project_id").is_some() {
            return Response::err(req.id, ERR_BAD_PARAMS, ERR_CORRECTION_NEEDS_ENTRY);
        }
    }
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
    //
    // `project_audit_label` is the display label of the project a
    // `project_id` call resolved to, for the audit row below. It is derived
    // from the key the daemon itself holds, never from the caller's string
    // -- the same rule `set_project_mode` follows, and the reason
    // `daemon-audit.jsonl` cannot be injected into.
    let (ids, project_audit_label): (Vec<Uuid>, Option<String>) = if all {
        let queue = shared.queue.lock().expect("queue lock");
        (queue.pending().iter().map(|e| e.entry_id).collect(), None)
    } else if let Some(pid) = project_id {
        // An id naming no project the daemon knows is refused, exactly as
        // `set_project_mode` refuses it on this same socket with this same
        // handle, and for the same reason an unknown `entry_id` is refused
        // below: a handle the caller never received is a client bug, and
        // answering it `approved: 0` is indistinguishable from "that
        // project had nothing pending" -- a shell holding a typo'd or stale
        // id would render "Sent 0 sessions" and never learn otherwise.
        //
        // "Known" is policy plus every project in the queue in ANY state,
        // so a project whose entries were all just approved or swept is
        // still recognised and still answers `approved: 0`. The genuinely
        // empty case therefore stays a success, distinguishable from an id
        // that names no project at all.
        //
        // Lock order is policy before queue, as everywhere else.
        let policy = shared.policy.lock().expect("policy lock");
        let queue = shared.queue.lock().expect("queue lock");
        let known = known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()));
        let Some(key) = project_key_for_id(pid, &known) else {
            return Response::err(req.id, ERR_BAD_PARAMS, ERR_PROJECT_ID_UNRECOGNIZED);
        };
        // Only `Pending`: an entry already approved has had its terms
        // fixed, and a project-wide call must not silently re-pin them.
        let ids = queue
            .pending()
            .iter()
            .filter(|e| e.project_key == key)
            .map(|e| e.entry_id)
            .collect();
        // The unknown-cwd sentinel resolves here like any other project.
        // Approving what is already in that bucket is an ordinary consent
        // decision about entries the contributor can see; it is *arming*
        // the bucket that `set_mode` refuses, which is a standing grant.
        (ids, Some(disambiguated_label(&key, &known)))
    } else {
        let queue = shared.queue.lock().expect("queue lock");
        (
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
            },
            None,
        )
    };
    // A local, label-only record that a batch was bulk-approved, written
    // BEFORE anything is approved -- same ordering, and the same reason, as
    // `set_project_mode`: a rollback that has to write to the disk that just
    // refused a write is not a rollback. This is visibility, not a security
    // control (see `daemon::audit`), but it is the only visibility there is
    // for a call that used to require a terminal.
    //
    // Both bulk selectors are recorded, not only `all`. A tray click that
    // approves a whole project unattended is the same class of act as
    // approving the whole queue -- bulk, unattended, previously
    // terminal-only -- and leaving it unrecorded would drain the queue with
    // nothing in `daemon-audit.jsonl` to show for it. A single `entry_id`
    // approval is the always-was, one-click-at-a-time path and stays
    // unaudited.
    //
    // Written before the envelope builds below, not merely before the
    // approve loop: those builds persist artifacts of their own, and "the
    // record could not be written, so nothing happened" has to stay true of
    // everything this call does.
    //
    // The count is of entries eligible to be approved when the queue was
    // read. It is an upper bound: an entry no artifact can be built for is
    // skipped below, and `approve` can refuse one that moved. The record
    // says the batch was bulk-approved at all, which is what it exists for.
    //
    // A project selector that matched nothing writes no row: nothing was
    // approved and, the selection having been taken under the queue lock,
    // nothing could have been -- a row would record an act that did not
    // happen, and a shell polling a project with an empty queue would
    // rotate real records out of a capped log. `all` keeps writing
    // unconditionally: it names the queue rather than a subset of it, and
    // "the whole queue was bulk-approved" is a statement about the request,
    // which `bulk_approval_over_the_socket_is_now_allowed_and_appends_an_audit_entry`
    // fixes in place.
    if all || (project_audit_label.is_some() && !ids.is_empty()) {
        if let Err(_e) = audit::append(
            &shared.store,
            &AuditEntry {
                at: Utc::now(),
                action: "bulk-approved".to_string(),
                project_label: project_audit_label,
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
    //
    // A correction forces a rebuild even for an entry that was previewed:
    // the pinned artifact was built before the contributor had written
    // anything, so it carries neither the correction nor the
    // `correction_included` declaration that enrols it for the PII backstop,
    // and credential detection has never run over the text. Re-pinning here
    // is what makes the approval cover the bytes the correction is part of.
    // `correction` is only ever `Some` for a single `entry_id`, refused
    // above otherwise, so this rebuilds exactly one entry.
    let unpinned: Vec<(Uuid, super::queue::QueueEntry)> = {
        let queue = shared.queue.lock().expect("queue lock");
        ids.iter()
            .filter_map(|id| queue.all().iter().find(|e| e.entry_id == *id))
            .filter(|e| e.previewed_envelope_digest.is_none() || correction.is_some())
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
    //
    // The "already previewed contributes nothing" half of that stops being
    // true for a corrected entry, which is rebuilt above whatever its pin
    // said. Its counts are reported again here, and that is the right
    // answer rather than a double count: the artifact is not the one the
    // preview described, so what this toast names is what the approval
    // actually covers.
    let mut redactions: std::collections::BTreeMap<String, u32> = std::collections::BTreeMap::new();
    let mut flagged: u64 = 0;
    // The entries whose refusal has to outlive this response. See the size
    // check below, and the write under the queue lock further down.
    let mut too_large: Vec<Uuid> = Vec::new();
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
        match build_and_pin_preview(shared, id, &entry, cfg.as_ref(), correction.as_deref()).await {
            Ok((summary, _body, _envelope)) => {
                // `build_preview` does not size-check the raw contribution
                // (only `submit`'s path does); `approved_envelope::save`
                // does, on exactly this measurement -- so the pin
                // `build_and_pin_preview` just attempted has already been
                // refused for size, and this entry is unpinned. Repeating
                // the measurement here is what gives an oversized session
                // its own label instead of the generic `not-pinned` the pin
                // re-check below would otherwise give it -- this one is
                // permanent, and retrying the same session can never
                // succeed, which `not-pinned`'s other causes do not imply.
                if summary.would_send_bytes > crate::envelope::MAX_ENVELOPE_BYTES {
                    skipped.push((id, super::queue::REASON_TOO_LARGE));
                    // Reported *and* recorded. Reporting alone left the
                    // entry `Pending`, which is the state that means "still
                    // waiting on the contributor" -- so the watcher kept
                    // finding a live offer at this path and the card sat
                    // there for a session no click could ever get past this
                    // very check. A decision that can never come out
                    // differently for these bytes is a decision, and it
                    // belongs on the entry like every other one.
                    too_large.push(id);
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
    // Written under the same lock that approves, and saved by the same
    // `queue.save` below, so the refusal and the approvals in this batch
    // land together or not at all.
    //
    // Guarded on `Pending`, which is what `set_state` does not check for
    // itself: the build ran without the lock held, and an entry that was
    // cancelled, dismissed or superseded in that window has a newer
    // decision on it than this one.
    //
    // `Refused` is the right terminal state and `REASON_TOO_LARGE` the
    // right label, but they are deliberately *not* `REASON_DISMISSED`:
    // `Queue::dismissed_at_path` suppresses a whole conversation forever,
    // and that is reserved for a contributor saying no. This is the
    // pipeline's verdict on one envelope built under one set of consent
    // scopes -- narrower scopes can yield a smaller envelope from the same
    // conversation -- so it binds to the entry, and a session that has
    // moved on is offered again exactly as it is after any other pipeline
    // refusal.
    for id in &too_large {
        if queue.get(*id).map(|e| e.state) == Some(QueueState::Pending) {
            queue.set_state(
                *id,
                QueueState::Refused,
                Some(super::queue::REASON_TOO_LARGE.to_string()),
            );
        }
    }
    let mut approved_ids = Vec::new();
    for id in &ids {
        let id = *id;
        if skipped_ids.contains(&id) {
            continue;
        }
        // The pin is re-checked here, under the lock that approves, rather
        // than inferred from the build returning `Ok`. `build_and_pin_preview`
        // is `Ok` whenever the *build* succeeded, and `pin_previewed_envelope`
        // declines silently when the envelope could not be written, and when
        // the entry was not `Pending` by the time the pin was attempted.
        // Trusting `Ok` would approve an entry with no
        // artifact behind it, which is the one thing the uploader does not
        // catch. An entry that is still unpinned here is left `Pending` for
        // the contributor to approve again.
        if queue
            .get(id)
            .is_none_or(|e| e.previewed_envelope_digest.is_none())
        {
            // The build reported `Ok` (or this entry had a stale pin to
            // begin with) but nothing is pinned now: `pin_previewed_envelope`
            // declined to write because `approved_envelope::save` failed, or
            // because the entry was no longer `Pending` when the pin was
            // attempted; or (for `all`/`project_id`) the entry was removed
            // from the queue entirely in that same window. (A failed queue
            // *save* inside `pin_previewed_envelope` does not land here: it
            // leaves the pin in memory, which is what this check reads.)
            //
            // Which label that deserves depends on where the entry stands
            // now, and the two documented labels promise different things
            // to a client. `not-pinned` is documented transient -- retry is
            // expected to work once the race passed -- and that is only
            // true while the entry is still `Pending`. An entry that is
            // both unpinned and no longer `Pending` (dismissed, expired,
            // superseded, or approved through some other path without ever
            // being previewed) can never be approved by a retry: every pin
            // path refuses a non-`Pending` entry, so retrying loops
            // forever. That case gets `not-pending`, whose documented
            // advice -- refresh queue state rather than retry blindly -- is
            // the correct one for it. An entry gone from the queue entirely
            // is reported `not-pinned`: it was `Pending` when it was
            // selected, and its disappearance is the transient race.
            //
            // Either way the entry is left as it stands -- but it must not
            // vanish from the response. An entry counted in neither
            // `approved` nor `skipped` is exactly the silent hole the pin
            // re-check above this loop exists to close for the uploader;
            // the caller deserves the same guarantee.
            let label = match queue.get(id).map(|e| e.state) {
                Some(QueueState::Pending) | None => "not-pinned",
                Some(_) => "not-pending",
            };
            skipped.push((id, label));
            continue;
        }
        if queue.approve(
            id,
            &scopes,
            inputs.as_deref(),
            verdict.as_deref(),
            correction.as_deref(),
            Some(approved_at),
        ) {
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

/// The socket's `"preview"` handler -- the queue-card summary.
///
/// This is called once per row shown in the queue (the app fires it for
/// every entry `awaitingDecision` at once), so it deliberately does the
/// *least* work of any preview surface: [`preview::build_preview_card`]
/// skips both the envelope digest and the pretty-printed body, neither of
/// which a card renders. It also does not pin the entry -- see that
/// function's doc for why skipping the digest makes that the only safe
/// choice, and why nothing downstream relies on a card load to have
/// happened: the preview sheet (`open_preview`, below) and an on-demand
/// rebuild inside `handle_approve` are the two paths that actually pin, and
/// either one runs regardless of whether a card was ever loaded for the
/// entry.
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
    let near_ai = {
        let s = shared.settings.lock().expect("settings lock");
        s.near_ai.clone()
    };
    let source_roots = shared.source_roots_with_routing();
    let sources = crate::source::all_sources(&source_roots);
    let Some((source, session_ref)) = super::find_session(&sources, &entry) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "session-file-vanished");
    };

    match super::preview::build_preview_card(cfg.as_ref(), near_ai, source, &session_ref).await {
        Ok(summary) => {
            // The card shape lives in `preview_card_value`, shared with the
            // scheduler's ready event so the two cannot drift. `entry` is
            // added only here: this response describes an entry the caller
            // just named, while a cached summary outlives that state.
            let mut value = preview_card_value(&summary);
            value["entry"] = entry_value(&entry);
            Response::ok(req.id, value)
        }
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "preview-failed"),
    }
}

/// The summary object both `preview` and the scheduler report.
///
/// One definition, so the blocking method and the scheduled one cannot
/// describe the same build differently. It deliberately does **not** carry
/// `entry`: the scheduler caches this object and a queue entry's state
/// changes underneath it, so embedding one would let a cached preview
/// assert a stale state. `preview` adds a freshly read `entry` on top; the
/// scheduler's callers already hold entries from `list_pending` and the
/// `snapshot` event.
/// The card shape, without the entry.
///
/// Two surfaces render it -- the socket's `preview` response, which adds
/// `entry` on top, and the scheduler's `preview_ready` event, which
/// deliberately omits it because a cached summary outlives the entry state
/// it was built beside. They share this function so the field set cannot
/// drift between them.
///
/// There is no `envelope_digest` here, and that is the point: a card build
/// never produces one. See `preview::build_preview_card`.
pub(crate) fn preview_card_value(
    summary: &super::preview::PreviewCardSummary,
) -> serde_json::Value {
    serde_json::json!({
        "would_send_bytes": summary.would_send_bytes,
        "raw_session_bytes": summary.raw_session_bytes,
        "event_count": summary.event_count,
        "opening_prompt": summary.opening_prompt,
        "redactions": summary.redactions,
        "pii_labels_present": summary.pii_labels_present,
        "consent_scopes": summary.consent_scopes,
        "residual_risk": summary.residual_risk,
        // The configuration fingerprint -- cheap, a hash of the config
        // rather than of the envelope -- so it still rides along. There is
        // no `envelope_digest`: this build never made one and never pinned
        // the entry.
        "input_fingerprint": summary.input_fingerprint,
        // False when this device is not enrolled: the summary describes a
        // placeholder-identity, deterministic-only build, and the
        // fingerprint above is not bindable to a later approval.
        "enrolled": summary.enrolled,
    })
}

/// The fingerprint of every input other than the session bytes that decides
/// what a preview says.
///
/// Part of the scheduler's cache key, so a device that changes its consent
/// scopes, its identity, or its privacy-filter settings does not keep being
/// shown cards built under the old configuration.
///
/// An unenrolled device gets a fixed label rather than the placeholder
/// config's fingerprint. Hashing the placeholder would be hashing a
/// constant, and the value that matters -- that enrolling invalidates every
/// unenrolled preview -- holds either way.
pub(crate) fn preview_config_fingerprint(shared: &DaemonShared) -> String {
    let cfg = shared.store.load_config().ok().flatten();
    let near_ai = {
        let s = shared.settings.lock().expect("settings lock");
        s.near_ai.clone()
    };
    match cfg {
        Some(c) => super::preview::input_fingerprint(&c, near_ai.as_ref()),
        None => "unenrolled".to_string(),
    }
}

/// Ask the scheduler for a preview and return promptly.
///
/// The difference from `preview` is the whole point: this never runs the
/// redaction pipeline on the connection's time. It answers from cache, or
/// says the work is queued and lets the `preview_ready` event carry the
/// result. A shell drawing a list calls this once per card and blocks on
/// nothing.
fn handle_preview_request(shared: &DaemonShared, req: &Request) -> Response {
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
    let key = PreviewKey::for_entry(
        &entry.path,
        entry.size_bytes,
        preview_config_fingerprint(shared),
    );
    match shared.previews.request(id, key, entry.size_bytes) {
        RequestState::Cached(outcome) => Response::ok(req.id, outcome.to_value(id)),
        RequestState::Queued => Response::ok(
            req.id,
            serde_json::json!({
                "entry_id": id.to_string(),
                "state": super::preview_scheduler::STATE_QUEUED,
            }),
        ),
        RequestState::Running => Response::ok(
            req.id,
            serde_json::json!({
                "entry_id": id.to_string(),
                "state": super::preview_scheduler::STATE_RUNNING,
            }),
        ),
    }
}

/// Declare which entries are on screen, so their previews are built first.
///
/// Wholesale replacement of the visible set, and cheap enough to call on
/// every scroll: it takes one lock and moves no work. Visibility decides
/// order, never membership -- an entry that scrolls away keeps its place in
/// the queue until someone calls `preview_cancel`.
fn handle_preview_visible(shared: &DaemonShared, req: &Request) -> Response {
    let Some(raw) = req.params.get("entry_ids") else {
        return Response::err(req.id, ERR_BAD_PARAMS, "entry-ids-required");
    };
    let Some(list) = raw.as_array() else {
        return Response::err(req.id, ERR_BAD_PARAMS, "entry-ids-invalid");
    };
    let mut ids = Vec::with_capacity(list.len());
    for item in list {
        match item.as_str().and_then(|s| Uuid::parse_str(s).ok()) {
            Some(id) => ids.push(id),
            None => return Response::err(req.id, ERR_BAD_PARAMS, "entry-ids-invalid"),
        }
    }
    let visible = shared.previews.set_visible(ids);
    Response::ok(req.id, serde_json::json!({ "visible": visible }))
}

/// Drop a scheduled preview.
///
/// `dropped: false` means there was nothing to drop -- already finished,
/// already cancelled, or never requested. It is not an error: a shell that
/// cancels on every card leaving the viewport will hit that case constantly
/// and has nothing to do about it.
fn handle_preview_cancel(shared: &DaemonShared, req: &Request) -> Response {
    let id = match parse_entry_id(&req.params) {
        Ok(id) => id,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    let dropped = shared.previews.cancel(id);
    Response::ok(
        req.id,
        serde_json::json!({ "entry_id": id.to_string(), "dropped": dropped }),
    )
}

/// Build the redacted envelope for one queue entry, pin the entry to it, and
/// hand back the summary, the redacted body, and the envelope.
///
/// This is the pinning path -- `open_preview` (the C ABI's in-process full
/// preview, behind the preview sheet), `handle_preview_body` (the socket's
/// body, when there is no stored envelope to read instead), and
/// `handle_approve`'s on-demand rebuild for an entry no card or sheet ever
/// pinned all go through it. `handle_preview` (the socket's card summary,
/// above) deliberately does **not**: see its doc comment and
/// `preview::build_preview_card` for why a card load skips both the digest
/// and the pin. Errors are `(code, fixed label)` -- no path, no entry
/// content -- and the callers that need a bare label discard the code.
/// `correction` is the contributor's written correction for this entry, when
/// the call that is building this artifact carried one. It is folded in
/// before redaction runs, so the pinned bytes are the ones the correction is
/// part of -- and so credential detection gets to refuse before anything is
/// pinned. Only `handle_approve` ever passes one.
async fn build_and_pin_preview(
    shared: &DaemonShared,
    entry_id: Uuid,
    entry: &super::queue::QueueEntry,
    cfg: Option<&crate::config::ContributorConfig>,
    correction: Option<&str>,
) -> Result<
    (
        super::preview::PreviewSummary,
        String,
        trace_commons_protocol::trace_contribution::TraceContributionEnvelope,
    ),
    (&'static str, &'static str),
> {
    let near_ai = {
        let s = shared.settings.lock().expect("settings lock");
        s.near_ai.clone()
    };
    let source_roots = shared.source_roots_with_routing();
    let sources = crate::source::all_sources(&source_roots);
    let (source, session_ref) =
        super::find_session(&sources, entry).ok_or((ERR_BAD_PARAMS, "session-file-vanished"))?;
    let (summary, body, envelope) = super::preview::build_preview_with_correction(
        &shared.store,
        cfg,
        near_ai,
        source,
        &session_ref,
        correction,
    )
    .await
    .map_err(|e| {
        // The one refusal a contributor can act on gets its own label so a
        // shell can say what happened and what to do about it. Compared
        // against the fixed label `redact_to_envelope` produces, never
        // rendered from the error, so no text can ride out on this path.
        if e.to_string() == REASON_CORRECTION_CREDENTIAL {
            (ERR_BAD_PARAMS, REASON_CORRECTION_CREDENTIAL)
        } else {
            (ERR_UNAVAILABLE, "preview-failed")
        }
    })?;
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
    let (summary, body, _envelope) =
        build_and_pin_preview(shared, entry_id, &entry, cfg.as_ref(), None)
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
                build_and_pin_preview(shared, entry_id, &entry, cfg.as_ref(), None).await?;
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
        let gemini_mode = mode_of(&s.gemini_source);
        obj.remove("claude_root");
        obj.remove("codex_root");
        obj.remove("claude_source");
        obj.remove("codex_source");
        obj.remove("gemini_source");
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
        // No `gemini_root_configured`: the `*_root_configured` pair exists
        // for shells written before `*_source_mode` did, and none of them
        // knows about this source. A new key would only be a second
        // spelling of the mode.
        obj.insert(
            "gemini_source_mode".to_string(),
            serde_json::Value::String(gemini_mode.to_string()),
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

/// How long a probe waits for the proxy before calling it unreachable.
///
/// A loopback call to a process on the same machine, and one a human is
/// waiting on with a settings dialog open. Short for both reasons.
const PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// `probe_routing`: the proxy answered and the token was accepted.
pub const PROBE_REACHABLE: &str = "reachable";
/// `probe_routing`: the token could not be used. Carries `token_path`, the
/// absolute path that was tried -- never the token.
///
/// Covers both halves of the same contributor-fixable mistake: no readable
/// `control.token` at the resolved path, and a proxy that answered and
/// refused the one that was there. A GUI daemon never sees `IRONWIRE_HOME`,
/// so it reads `~/.ironwire/control.token` whatever the contributor set in
/// a shell -- which produces a missing file on one machine and a stale
/// token on another. Both are fixed by naming the directory, so both name
/// the path.
pub const PROBE_TOKEN_UNREADABLE: &str = "token_unreadable";
/// `probe_routing`: nothing usable answered on the port. Carries `port`.
///
/// Also the answer when something answered but did not serve the ledger --
/// a 404 or a 500. The contributor's actionable fact there is still the
/// port: the usual cause is a number naming some other local service.
pub const PROBE_UNREACHABLE: &str = "unreachable";

/// Ask the declared proxy whether it is there, and say what was found.
///
/// The counterpart to the rule `routing/mod.rs` lives by. On the submission
/// path absence and failure are deliberately the same state, because a
/// proxy that vanished must never cost anyone a trace. **Declaring is the
/// other path, and it must answer**: as `main` stood, a contributor could
/// name a wrong port or an unreachable token, get no error and no
/// indicator, and have every trace silently carry no routing data.
///
/// Nothing here touches daemon state and nothing here can affect a
/// submission: it takes no `DaemonShared`, it runs only when a human asks,
/// and its whole effect is the answer it returns.
///
/// Three outcomes, each distinguishable by the caller, and **none of them
/// ever carries the token**. `IronWireLedger`'s hand-written `Debug` keeps
/// the token out of logs because it is a credential for an API that can
/// rewrite the contributor's agent configuration; the same reasoning holds
/// at the IPC boundary, where the answer crosses a socket to a shell.
///
/// The token *directory* is a different thing and is the point: the path is
/// what makes the failure fixable.
/// Answer what a running IronWire says about itself, so the app does not
/// have to ask a contributor for it.
///
/// The declaring flow's counterpart to `probe_routing`. The probe checks a
/// port and a token the contributor already named; this reports the port
/// and token path a *running* proxy published, before there is anything
/// declared to check.
///
/// Result shape, and the reason for it:
///
/// ```json
/// { "found": true, "port": 8463, "token_path": "/home/x/.ironwire/control.token" }
/// { "found": false }
/// ```
///
/// A boolean rather than a named outcome. There is exactly one distinction
/// to draw here -- a pointer was read, or it was not -- and every reason it
/// was not (never installed, not running, a version that does not publish,
/// a file this reader will not act on) is the same fact to the caller and
/// the same next step for the contributor: type the port. A vocabulary of
/// outcome strings would invite a caller to match on one, and a string that
/// is a prefix of another is how a shell comes to treat "unreachable" as
/// "reachable".
///
/// **Never carries a token.** `token_path` is a path, for display beside
/// the port; the daemon opens it itself, at call time, when it builds a
/// reader. Discovery is advisory throughout: nothing here writes settings,
/// touches daemon state, or can affect a submission.
fn handle_discover_routing(req: &Request) -> Response {
    let Some(pointer) = super::ironwire_pointer::read_pointer() else {
        return Response::ok(req.id, serde_json::json!({ "found": false }));
    };
    let mut result = serde_json::Map::new();
    result.insert("found".to_string(), serde_json::Value::Bool(true));
    result.insert("port".to_string(), serde_json::json!(pointer.port));
    if let Some(token_path) = pointer.token_path.as_ref() {
        result.insert(
            "token_path".to_string(),
            serde_json::json!(token_path.to_string_lossy()),
        );
    }
    Response::ok(req.id, serde_json::Value::Object(result))
}

async fn handle_probe_routing(req: &Request) -> Response {
    let port = match req.params.get("port").and_then(serde_json::Value::as_u64) {
        // Port 0 is not a port a proxy listens on; it is the ask-the-kernel
        // sentinel, and accepting it would probe whatever it resolved to.
        Some(port) if port > 0 && port <= u64::from(u16::MAX) => port as u16,
        _ => return Response::err(req.id, ERR_BAD_PARAMS, "port-invalid"),
    };
    let token_dir = match req.params.get("token_dir") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => match value.as_str() {
            // An empty string is not a directory. Refused rather than
            // treated as absent, because falling through to the
            // environment would answer about a path the caller did not ask
            // about.
            Some(dir) if !dir.is_empty() => Some(std::path::PathBuf::from(dir)),
            _ => return Response::err(req.id, ERR_BAD_PARAMS, "token-dir-invalid"),
        },
    };

    // The same resolution the reader uses, through the same function, so a
    // probe cannot send a contributor to fix a file nothing reads.
    let Some(path) = super::settings::ironwire_token_path(token_dir.as_deref()) else {
        // Nothing resolved at all: no declared directory, no
        // `IRONWIRE_HOME`, no discoverable home. There is no path to name,
        // and `token_path` is absent rather than empty.
        return Response::ok(
            req.id,
            serde_json::json!({ "outcome": PROBE_TOKEN_UNREADABLE }),
        );
    };
    // Lexical, not a canonicalization: `std::path::absolute` touches no
    // filesystem and resolves a relative path against the same working
    // directory `fs::read_to_string` would, so what is reported and what is
    // read stay the same file.
    let reported = std::path::absolute(&path).unwrap_or_else(|_| path.clone());
    let unreadable = || {
        Response::ok(
            req.id,
            serde_json::json!({
                "outcome": PROBE_TOKEN_UNREADABLE,
                "token_path": reported.to_string_lossy(),
            }),
        )
    };

    let token = match std::fs::read_to_string(&path) {
        Ok(token) => token.trim().to_string(),
        Err(_) => return unreadable(),
    };
    if token.is_empty() {
        // A file that exists with nothing in it. There is no credential to
        // send, so this is the same fixable state as no file at all.
        return unreadable();
    }

    let Ok(client) = reqwest::Client::builder().build() else {
        // The platform trust store would not load. Not a fact about the
        // proxy, and saying "unreachable" would send the contributor to
        // check a port that is fine.
        return Response::err(req.id, ERR_UNAVAILABLE, "probe-client-unavailable");
    };
    let response = client
        .get(format!("http://127.0.0.1:{port}/_ironwire/log?limit=1"))
        .timeout(PROBE_TIMEOUT)
        .header("authorization", format!("Bearer {token}"))
        .send()
        .await;
    let Ok(response) = response else {
        return Response::ok(
            req.id,
            serde_json::json!({ "outcome": PROBE_UNREACHABLE, "port": port }),
        );
    };
    let status = response.status();
    if status.is_success() {
        return Response::ok(req.id, serde_json::json!({ "outcome": PROBE_REACHABLE }));
    }
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return unreadable();
    }
    Response::ok(
        req.id,
        serde_json::json!({ "outcome": PROBE_UNREACHABLE, "port": port }),
    )
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

    /// A daemon whose settings never declared a proxy builds no ledger and
    /// attempts no connection.
    #[test]
    fn no_proxy_declared_builds_no_ledger() {
        assert!(shared().routing_ledger().is_none());
    }

    /// `source_roots_with_routing` is the one insertion point: bare roots
    /// with no ledger, decorated roots when the daemon holds one.
    #[test]
    fn source_roots_with_routing_reflects_whether_a_ledger_is_held() {
        let s = shared();
        assert!(
            !s.source_roots_with_routing().is_routed(),
            "no ledger held, so the roots must stay bare"
        );

        let s = shared();
        *s.routing.write().unwrap() = Some(Arc::new(
            crate::routing::ironwire::IronWireLedger::new(8463, "t".to_string()),
        ));
        assert!(
            s.source_roots_with_routing().is_routed(),
            "a held ledger must be attached"
        );
    }

    /// A refresh against a port nothing is listening on must not fail or
    /// hang the caller -- the same guarantee `IronWireLedger::refresh` makes
    /// on its own, exercised here through the daemon's own entry point.
    #[tokio::test]
    async fn a_refresh_failure_leaves_the_daemon_running_and_the_overlay_empty() {
        // Bind to get a genuinely free loopback port, then drop the
        // listener so nothing answers on it.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let s = shared();
        *s.routing.write().unwrap() = Some(Arc::new(
            crate::routing::ironwire::IronWireLedger::new(port, "t".to_string()),
        ));
        s.refresh_routing().await;
        assert!(
            !s.routing_ledger().unwrap().has_rows(),
            "an unreachable proxy leaves the snapshot empty, not the daemon down"
        );
        assert!(
            s.source_roots_with_routing().is_routed(),
            "the ledger stays attached even though it has nothing to say"
        );
    }

    /// `has_rows` only gets reported when it changes, not on every poll.
    #[test]
    fn routing_state_reports_only_on_transition() {
        let s = shared();
        assert_eq!(
            s.routing_transition(false),
            None,
            "still empty is not a transition"
        );
        assert_eq!(
            s.routing_transition(true),
            Some(true),
            "empty to reading is a transition"
        );
        assert_eq!(
            s.routing_transition(true),
            None,
            "still reading is not a transition"
        );
        assert_eq!(
            s.routing_transition(false),
            Some(false),
            "reading to empty is a transition"
        );
    }

    /// The full loop: a mock IronWire server, the ledger the daemon owns,
    /// a real session file on disk, and `source_roots_with_routing`'s
    /// output actually run through `all_sources` and `load`. This is the
    /// assertion Task 5 could only pin with a test-only accessor -- here the
    /// routing row is asserted on the transcript a real adapter produced,
    /// not on whether a ledger happens to be reachable.
    #[tokio::test]
    async fn a_refreshed_ledger_reaches_a_loaded_transcript() {
        let claude_root = tempfile::tempdir().unwrap();
        let project_dir = claude_root.path().join("proj");
        std::fs::create_dir_all(&project_dir).unwrap();
        let session_path = project_dir.join("sess-1.jsonl");
        std::fs::write(
            &session_path,
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"hi\"},\
             \"cwd\":\"/x/proj\",\"timestamp\":\"2026-08-08T10:00:00Z\",\
             \"version\":\"2.0.1\",\"sessionId\":\"sess-1\",\"uuid\":\"a1\"}\n",
        )
        .unwrap();

        let router = axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "exchanges": [{
                        "started_at": "2026-08-08T10:05:00Z",
                        "client_session_id": "sess-1",
                        "facade": "anthropic",
                        "backend": "claude-sub",
                        "rung": "same_model",
                        "attempts": 1,
                        "status": 200
                    }]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let s = shared();
        {
            let mut settings = s.settings.lock().unwrap();
            settings.claude_source = Some(crate::daemon::settings::SourceDeclaration::Watch {
                path: claude_root.path().to_path_buf(),
            });
        }
        *s.routing.write().unwrap() = Some(Arc::new(
            crate::routing::ironwire::IronWireLedger::new(port, "t".to_string()),
        ));

        // This is the seam this task installs: refresh, then build sources
        // off the daemon's own held instance.
        s.refresh_routing().await;
        assert!(
            s.routing_ledger().unwrap().has_rows(),
            "the mock server's row must have reached the snapshot"
        );

        let roots = s.source_roots_with_routing();
        let sources = crate::source::all_sources(&roots);
        let claude = sources
            .iter()
            .find(|src| src.name() == crate::source::SOURCE_CLAUDE_CODE)
            .expect("the claude source is present");
        let refs = claude.discover().expect("discovers the fixture");
        let session_ref = refs
            .into_iter()
            .find(|r| r.path == session_path)
            .expect("the written session was discovered");
        let transcript = claude.load(&session_ref).expect("loads");
        assert_eq!(
            transcript.routing.len(),
            1,
            "the refreshed row reached the loaded transcript"
        );
    }

    /// Every source declaration carries a local filesystem path, and this
    /// blob is serialized wholesale. A source added to `DaemonSettings`
    /// without a matching removal here would put that path on the wire.
    #[test]
    fn the_settings_blob_reports_source_modes_and_never_a_source_path() {
        let settings = DaemonSettings {
            claude_source: Some(crate::daemon::settings::SourceDeclaration::Watch {
                path: std::path::PathBuf::from("/declared/claude"),
            }),
            codex_source: Some(crate::daemon::settings::SourceDeclaration::Off),
            gemini_source: Some(crate::daemon::settings::SourceDeclaration::Watch {
                path: std::path::PathBuf::from("/declared/gemini"),
            }),
            ..Default::default()
        };
        let v = redacted_settings(&settings);
        let rendered = v.to_string();
        for path in ["/declared/claude", "/declared/gemini"] {
            assert!(!rendered.contains(path), "a source path reached the wire");
        }
        assert_eq!(v["claude_source_mode"], "watch");
        assert_eq!(v["codex_source_mode"], "off");
        assert_eq!(v["gemini_source_mode"], "watch");
        assert!(v.get("gemini_source").is_none());
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
                        declared_source: None,
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
                        approved_verdict: None,
                        approved_correction: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                        observed_modified_at: None,
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
                        declared_source: None,
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
                        approved_verdict: None,
                        approved_correction: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                        observed_modified_at: None,
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

    #[tokio::test]
    async fn an_approval_carries_its_verdict_to_the_entry() {
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
                        declared_source: None,
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
                        approved_verdict: None,
                        approved_correction: None,
                        approved_inputs: None,
                        // Already pinned, so `handle_approve` does not try to
                        // build a real preview for a path that does not
                        // exist -- this test is about the verdict, not the
                        // envelope pipeline.
                        previewed_envelope_digest: Some("sha256:preview".to_string()),
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                        observed_modified_at: None,
                    },
                    500,
                )
                .unwrap();
        }

        let r = handle_request_async(
            &s,
            &req(
                "approve",
                serde_json::json!({"entry_id": entry_id.to_string(), "outcome": "partly"}),
            ),
        )
        .await;
        assert!(r.error.is_none(), "approve should succeed: {:?}", r.error);

        let queue = s.queue.lock().unwrap();
        assert_eq!(
            queue.get(entry_id).unwrap().approved_verdict.as_deref(),
            Some("partly")
        );
    }

    /// A bulk approval applies one verdict to every entry it covers. This is a
    /// coverage-over-precision tradeoff taken deliberately; see the spec.
    #[tokio::test]
    async fn a_bulk_approval_applies_its_verdict_to_every_entry() {
        let s = shared();
        {
            let mut queue = s.queue.lock().unwrap();
            for n in 0..2u8 {
                queue
                    .upsert(
                        super::super::queue::QueueEntry {
                            entry_id: uuid::Uuid::new_v4(),
                            session_hash: format!("sha256:seed{n}"),
                            source: "claude-code".to_string(),
                            declared_source: None,
                            project_key: "/tmp/p".to_string(),
                            project_label: "p".to_string(),
                            path: std::path::PathBuf::from(format!("/tmp/seed{n}.jsonl")),
                            size_bytes: 1,
                            discovered_at: Utc::now(),
                            state: QueueState::Pending,
                            reason_label: None,
                            attempts: 0,
                            retry_after: None,
                            submission_id: None,
                            approved_scopes: None,
                            approved_verdict: None,
                            approved_correction: None,
                            approved_inputs: None,
                            // Already pinned, for the same reason as the
                            // single-entry test above.
                            previewed_envelope_digest: Some(format!("sha256:preview{n}")),
                            approved_at: None,
                            subagent_count: 0,
                            subagents_dropped: 0,
                            observed_modified_at: None,
                        },
                        500,
                    )
                    .unwrap();
            }
        }

        let r = handle_request_async(
            &s,
            &req(
                "approve",
                serde_json::json!({"all": true, "outcome": "worked"}),
            ),
        )
        .await;
        assert!(r.error.is_none(), "{:?}", r.error);

        let queue = s.queue.lock().unwrap();
        for e in queue.all() {
            assert_eq!(e.approved_verdict.as_deref(), Some("worked"));
        }
    }

    /// A typo must not silently submit the run as `Unknown`. Same rule the
    /// `--outcome` flag applies, at the IPC boundary.
    #[tokio::test]
    async fn an_unrecognised_verdict_is_refused_and_approves_nothing() {
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
                        declared_source: None,
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
                        approved_verdict: None,
                        approved_correction: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                        observed_modified_at: None,
                    },
                    500,
                )
                .unwrap();
        }

        let r = handle_request_async(
            &s,
            &req(
                "approve",
                serde_json::json!({"entry_id": entry_id.to_string(), "outcome": "sucess"}),
            ),
        )
        .await;

        assert!(r.error.is_some(), "an unknown verdict must be refused");
        let queue = s.queue.lock().unwrap();
        assert_eq!(
            queue.get(entry_id).unwrap().state,
            QueueState::Pending,
            "a refused call must approve nothing"
        );
    }

    /// A pinned, pending entry with a path that does not exist. Enough for
    /// the correction refusals below, every one of which returns before any
    /// envelope is built.
    fn seed_pinned_entry(s: &DaemonShared) -> Uuid {
        let entry_id = Uuid::new_v4();
        let mut queue = s.queue.lock().unwrap();
        queue
            .upsert(
                super::super::queue::QueueEntry {
                    entry_id,
                    session_hash: format!("sha256:{entry_id}"),
                    source: "claude-code".to_string(),
                    declared_source: None,
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
                    approved_verdict: None,
                    approved_correction: None,
                    approved_inputs: None,
                    previewed_envelope_digest: Some("sha256:preview".to_string()),
                    approved_at: None,
                    subagent_count: 0,
                    subagents_dropped: 0,
                    observed_modified_at: None,
                },
                500,
            )
            .unwrap();
        entry_id
    }

    /// The verdict gate, enforced by the daemon rather than trusted to the
    /// shells. A run the contributor has just called successful has nothing
    /// to correct, and the field is not shown for it in any client.
    #[tokio::test]
    async fn a_correction_is_refused_without_a_partly_or_failed_outcome() {
        let s = shared();
        let entry_id = seed_pinned_entry(&s);

        for outcome in [Some("worked"), None] {
            let mut params = serde_json::json!({
                "entry_id": entry_id.to_string(),
                "correction": "it edited the wrong config file",
            });
            if let Some(name) = outcome {
                params["outcome"] = serde_json::Value::String(name.to_string());
            }
            let r = handle_request_async(&s, &req("approve", params)).await;
            let err = r.error.expect("a correction without a verdict is refused");
            assert_eq!(err.message, ERR_CORRECTION_NEEDS_VERDICT);
        }

        let queue = s.queue.lock().unwrap();
        assert_eq!(queue.get(entry_id).unwrap().state, QueueState::Pending);
    }

    /// One correction cannot be attached to a batch: it was written about
    /// one session, and every other session in the batch would carry it into
    /// the corpus as the contributor's own words about work it does not
    /// describe.
    #[tokio::test]
    async fn a_correction_is_refused_on_a_bulk_selector() {
        let s = shared();
        seed_pinned_entry(&s);

        for selector in [
            serde_json::json!({"all": true}),
            serde_json::json!({"project_id": "some-project"}),
        ] {
            let mut params = selector;
            params["outcome"] = serde_json::Value::String("failed".to_string());
            params["correction"] = serde_json::Value::String("it did the wrong thing".to_string());
            let r = handle_request_async(&s, &req("approve", params)).await;
            let err = r.error.expect("a bulk correction is refused");
            assert_eq!(err.message, ERR_CORRECTION_NEEDS_ENTRY);
        }
    }

    #[tokio::test]
    async fn a_correction_of_the_wrong_type_or_past_the_cap_is_refused() {
        let s = shared();
        let entry_id = seed_pinned_entry(&s);

        let r = handle_request_async(
            &s,
            &req(
                "approve",
                serde_json::json!({
                    "entry_id": entry_id.to_string(),
                    "outcome": "failed",
                    "correction": 7,
                }),
            ),
        )
        .await;
        assert_eq!(
            r.error.expect("a non-string correction is refused").message,
            ERR_BAD_CORRECTION
        );

        let too_long = "x".repeat(crate::envelope::MAX_CORRECTION_CHARS + 1);
        let r = handle_request_async(
            &s,
            &req(
                "approve",
                serde_json::json!({
                    "entry_id": entry_id.to_string(),
                    "outcome": "failed",
                    "correction": too_long,
                }),
            ),
        )
        .await;
        assert_eq!(
            r.error.expect("an oversized correction is refused").message,
            ERR_CORRECTION_TOO_LONG
        );

        let queue = s.queue.lock().unwrap();
        assert_eq!(queue.get(entry_id).unwrap().state, QueueState::Pending);
    }

    /// Whitespace is not a correction. It is normalised away rather than
    /// refused, so a contributor who tabbed through the field and typed
    /// nothing gets exactly the 0.5.0 behaviour: an ordinary approval, no
    /// correction recorded, and no rebuild of the artifact they were shown.
    #[tokio::test]
    async fn a_whitespace_only_correction_approves_as_if_none_was_written() {
        let s = shared();
        let entry_id = seed_pinned_entry(&s);

        let r = handle_request_async(
            &s,
            &req(
                "approve",
                serde_json::json!({
                    "entry_id": entry_id.to_string(),
                    "correction": "   \n  ",
                }),
            ),
        )
        .await;
        assert!(r.error.is_none(), "should approve: {:?}", r.error);

        let queue = s.queue.lock().unwrap();
        let e = queue.get(entry_id).unwrap();
        assert_eq!(e.state, QueueState::Approved);
        assert_eq!(e.approved_correction, None);
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
                    declared_source: None,
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
                    approved_verdict: None,
                    approved_correction: None,
                    approved_inputs: None,
                    previewed_envelope_digest: None,
                    approved_at: None,
                    subagent_count: 0,
                    subagents_dropped: 0,
                    observed_modified_at: None,
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
                        declared_source: None,
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
                        approved_verdict: None,
                        approved_correction: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                        observed_modified_at: None,
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
                        declared_source: None,
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
                        approved_verdict: None,
                        approved_correction: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                        observed_modified_at: None,
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

    /// An approved queue entry of a given size, for budget assertions.
    /// `n` only distinguishes it from its siblings -- the queue dedupes on
    /// the session hash, so a fixture that reused one would silently
    /// collapse fourteen entries into one.
    fn approved_entry_n(n: usize, size_bytes: u64) -> super::super::queue::QueueEntry {
        super::super::queue::QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            session_hash: format!("sha256:{n:04x}"),
            source: "claude-code".to_string(),
            declared_source: None,
            project_key: "/tmp/p".to_string(),
            project_label: "p".to_string(),
            path: std::path::PathBuf::from("/tmp/s.jsonl"),
            size_bytes,
            observed_modified_at: None,
            discovered_at: Utc::now(),
            state: QueueState::Approved,
            reason_label: None,
            attempts: 0,
            retry_after: None,
            submission_id: None,
            approved_scopes: None,
            approved_verdict: None,
            approved_correction: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
            approved_at: None,
            subagent_count: 0,
            subagents_dropped: 0,
        }
    }

    fn approved_entry(size_bytes: u64) -> super::super::queue::QueueEntry {
        approved_entry_n(0, size_bytes)
    }

    #[test]
    fn status_reports_the_daily_budget_with_the_real_numbers() {
        // The condition that made a working app look broken: the byte
        // budget all but spent, 14 approved entries waiting, and a health
        // slot occupied by something else entirely -- so the cap was
        // invisible. `daily_budget` is reported independently of `health`
        // for exactly that reason, and this pins the numbers rather than
        // the presence of a key.
        let s = shared();
        {
            let mut state = s.state.lock().unwrap();
            state.day_bucket = Some(Utc::now().format("%Y-%m-%d").to_string());
            state.bytes_today = 204_659_969;
            state.uploads_today = 12;
        }
        {
            let mut q = s.queue.lock().unwrap();
            let max = s.settings.lock().unwrap().max_queue_entries;
            for n in 0..14 {
                q.upsert(approved_entry_n(n, 14_900_000), max).unwrap();
            }
        }
        // Something else already holds the single health slot, exactly as
        // `queue-full` did on the machine this came from.
        s.health
            .lock()
            .unwrap()
            .fail(crate::daemon::health::LABEL_QUEUE_FULL, Utc::now());

        let r = handle_request(&s, &req("status", serde_json::json!({})));
        let v = r.result.unwrap();
        let b = &v["daily_budget"];
        assert_eq!(v["health"]["last_error_label"], "queue-full");
        assert_eq!(b["blocked"], true);
        assert_eq!(b["blocked_entries"], 14);
        assert_eq!(b["blocked_bytes"], 14_900_000u64 * 14);
        assert_eq!(b["bytes_today"], 204_659_969u64);
        assert_eq!(b["max_bytes_per_day"], 209_715_200u64);
        assert_eq!(b["bytes_remaining"], 5_055_231u64);
        assert_eq!(b["uploads_today"], 12);
        assert_eq!(b["max_uploads_per_day"], 50);
        assert_eq!(b["uploads_remaining"], 38);
        assert!(!b["resets_at"].is_null());
    }

    #[test]
    fn status_reports_an_unspent_budget_as_not_blocking_anything() {
        let s = shared();
        {
            let mut q = s.queue.lock().unwrap();
            let max = s.settings.lock().unwrap().max_queue_entries;
            q.upsert(approved_entry(1_024), max).unwrap();
        }
        let v = handle_request(&s, &req("status", serde_json::json!({})))
            .result
            .unwrap();
        assert_eq!(v["daily_budget"]["blocked"], false);
        assert_eq!(v["daily_budget"]["blocked_entries"], 0);
        assert_eq!(v["daily_budget"]["bytes_today"], 0);
        assert_eq!(v["daily_budget"]["bytes_remaining"], 209_715_200u64);
    }

    #[test]
    fn polling_status_does_not_hand_back_a_fresh_budget() {
        // `budget_snapshot` rolls the day on a copy. If it rolled the real
        // one, a client polling `status` across midnight -- or a state file
        // left from yesterday -- would silently reset the counters the cap
        // is enforced against.
        let s = shared();
        {
            let mut state = s.state.lock().unwrap();
            state.day_bucket = Some("2020-01-01".to_string());
            state.bytes_today = 204_659_969;
            state.uploads_today = 12;
        }
        let _ = handle_request(&s, &req("status", serde_json::json!({})));
        let state = s.state.lock().unwrap();
        assert_eq!(state.bytes_today, 204_659_969);
        assert_eq!(state.uploads_today, 12);
        assert_eq!(state.day_bucket.as_deref(), Some("2020-01-01"));
    }

    #[test]
    fn a_stale_day_bucket_reports_todays_budget_as_untouched() {
        // The counters on disk belong to a day that has passed, so what
        // `status` reports is a full budget rather than yesterday's spend.
        let s = shared();
        {
            let mut state = s.state.lock().unwrap();
            state.day_bucket = Some("2020-01-01".to_string());
            state.bytes_today = 209_715_200;
            state.uploads_today = 50;
        }
        {
            let mut q = s.queue.lock().unwrap();
            let max = s.settings.lock().unwrap().max_queue_entries;
            q.upsert(approved_entry(14_900_000), max).unwrap();
        }
        let v = handle_request(&s, &req("status", serde_json::json!({})))
            .result
            .unwrap();
        assert_eq!(v["daily_budget"]["bytes_today"], 0);
        assert_eq!(v["daily_budget"]["blocked"], false);
    }

    #[test]
    fn the_daily_budget_carries_no_identifier_of_any_kind() {
        // Counts and timestamps only: no entry id, no hash, no path.
        let s = shared();
        {
            let mut q = s.queue.lock().unwrap();
            let max = s.settings.lock().unwrap().max_queue_entries;
            q.upsert(approved_entry(1_024), max).unwrap();
        }
        let v = handle_request(&s, &req("status", serde_json::json!({})))
            .result
            .unwrap();
        let body = serde_json::to_string(&v["daily_budget"]).unwrap();
        assert!(!body.contains("sha256"), "{body}");
        assert!(!body.contains('/'), "{body}");
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
            declared_source: None,
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
            approved_verdict: None,
            approved_correction: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
            approved_at: None,
            subagent_count: 0,
            subagents_dropped: 0,
            observed_modified_at: None,
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
                declared_source: None,
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
                approved_verdict: None,
                approved_correction: None,
                approved_inputs: None,
                previewed_envelope_digest: None,
                approved_at: None,
                subagent_count: 0,
                subagents_dropped: 0,
                observed_modified_at: None,
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

    /// One pending queue card, for the `entry_value` tests below.
    fn card_entry() -> super::super::queue::QueueEntry {
        super::super::queue::QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            session_hash: "sha256:aa".to_string(),
            source: "claude-code".to_string(),
            declared_source: None,
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
            approved_verdict: None,
            approved_correction: None,
            approved_inputs: None,
            previewed_envelope_digest: None,
            approved_at: None,
            subagent_count: 114,
            subagents_dropped: 2,
            observed_modified_at: None,
        }
    }

    /// The origin has to cross the IPC boundary, not merely exist on the ref.
    ///
    /// The desktop apps read this JSON and nothing else. An equivalent
    /// hand-off is exactly what broke while `declared_source` was being
    /// added to `SessionRef`, so it is asserted at the boundary rather than
    /// one layer below it.
    #[test]
    fn an_entry_reports_both_its_adapter_and_its_declared_origin() {
        let mut e = card_entry();
        e.source = "trajectory".to_string();
        e.declared_source = Some("antigravity".to_string());

        let v = entry_value(&e);
        assert_eq!(
            v["source"], "trajectory",
            "the adapter that loads it must stay reportable"
        );
        assert_eq!(v["declared_source"], "antigravity");
    }

    /// A native session declares nothing and must not grow an empty label.
    #[test]
    fn an_entry_with_no_declared_origin_reports_null() {
        let v = entry_value(&card_entry());
        assert_eq!(v["source"], "claude-code");
        assert!(
            v["declared_source"].is_null(),
            "absent must serialize as null, not as an empty string"
        );
    }

    #[test]
    fn the_queue_card_reports_how_many_delegated_transcripts_it_covers() {
        // A card standing for a hundred delegated transcripts has to say so:
        // the extent of what is being sent is part of the consent decision,
        // not decoration. No ordinal is exposed -- nothing in the format
        // supplies one.
        let e = card_entry();
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

    #[test]
    fn set_settings_raises_the_daily_caps_persists_and_the_next_upload_pass_sees_it() {
        // The scenario this exists for: a contributor whose byte budget was
        // already spent, with approved traces waiting, raises the cap
        // through `set_settings` -- no restart, no hand-edited file.
        let s = shared();
        {
            let mut state = s.state.lock().unwrap();
            state.uploads_today =
                super::super::settings::DaemonSettings::default().max_uploads_per_day;
            state.bytes_today = 204_659_969;
        }
        assert!(
            !super::super::uploader::cap_check(
                &s.state.lock().unwrap(),
                1,
                &s.settings.lock().unwrap(),
            ),
            "sanity: the default budget is exhausted before the raise"
        );

        let r = handle_request(
            &s,
            &req(
                "set_settings",
                serde_json::json!({"max_uploads_per_day": 200, "max_bytes_per_day": 2_147_483_648u64}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        let body = r.result.unwrap();
        assert_eq!(body["max_uploads_per_day"], 200);
        assert_eq!(body["max_bytes_per_day"], 2_147_483_648u64);

        // Live: the in-memory settings the next upload pass reads are
        // already updated, with no restart.
        assert!(
            super::super::uploader::cap_check(
                &s.state.lock().unwrap(),
                1,
                &s.settings.lock().unwrap(),
            ),
            "the raised cap must be visible to the very next cap check"
        );

        // Persisted: reloading from disk (what a restart does) must agree
        // with the live value, or a restart would silently revert what the
        // contributor just did.
        let reloaded = super::super::settings::DaemonSettings::load(&s.store).unwrap();
        assert_eq!(reloaded.max_uploads_per_day, 200);
        assert_eq!(reloaded.max_bytes_per_day, 2_147_483_648);
    }

    #[test]
    fn set_settings_accepts_ironwire_and_persists_it() {
        // Before this, `ironwire` was not in `apply_settings_object`'s
        // whitelist, so the only way to declare the proxy overlay was
        // hand-editing settings.json. `set_settings` now persists the
        // declaration the same way. This test asserts the live settings
        // value and the persisted file; that the overlay itself activates
        // without a restart is
        // `a_declaration_change_takes_effect_without_a_restart`.
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "set_settings",
                serde_json::json!({"ironwire": {"mode": "watch", "port": 8463}}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(
            s.settings.lock().unwrap().ironwire,
            Some(super::super::settings::IronWireDeclaration::Watch {
                port: 8463,
                token_dir: None
            })
        );

        // Persisted: a restart must see the same declaration.
        let reloaded = super::super::settings::DaemonSettings::load(&s.store).unwrap();
        assert_eq!(
            reloaded.ironwire,
            Some(super::super::settings::IronWireDeclaration::Watch {
                port: 8463,
                token_dir: None
            })
        );

        // null turns it back off.
        let r = handle_request(
            &s,
            &req("set_settings", serde_json::json!({"ironwire": null})),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(s.settings.lock().unwrap().ironwire, None);
    }

    #[test]
    fn set_settings_rejects_a_daily_cap_above_the_ceiling() {
        let s = shared();
        let r = handle_request(
            &s,
            &req(
                "set_settings",
                serde_json::json!({"max_uploads_per_day": 1_000_001u64}),
            ),
        );
        let err = r.error.expect("an out-of-range cap must be refused");
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, "settings-invalid-value");
        // Unchanged: the default is untouched by the rejected write.
        assert_eq!(
            s.settings.lock().unwrap().max_uploads_per_day,
            super::super::settings::DaemonSettings::default().max_uploads_per_day
        );
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
                    declared_source: None,
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
                    approved_verdict: None,
                    approved_correction: None,
                    approved_inputs: None,
                    previewed_envelope_digest: None,
                    approved_at: None,
                    subagent_count: 0,
                    subagents_dropped: 0,
                    observed_modified_at: None,
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
                        declared_source: None,
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
                        approved_verdict: None,
                        approved_correction: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                        observed_modified_at: None,
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
                        declared_source: None,
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
                        approved_verdict: None,
                        approved_correction: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                        subagent_count: 0,
                        subagents_dropped: 0,
                        observed_modified_at: None,
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

    // --- probe_routing -------------------------------------------------

    /// A `probe_routing` request naming `port`, and a token directory when
    /// the test needs the answer to be hermetic.
    ///
    /// Every probe test passes one: with `token_dir` absent the resolution
    /// falls through to `IRONWIRE_HOME` and then `~/.ironwire`, so a test
    /// that omitted it would read whatever the machine running it happens
    /// to have and would pass or fail for reasons that are not about this
    /// code.
    fn probe_request(port: u16, token_dir: Option<&std::path::Path>) -> Request {
        let mut params = serde_json::Map::new();
        params.insert("port".to_string(), serde_json::json!(port));
        if let Some(dir) = token_dir {
            params.insert(
                "token_dir".to_string(),
                serde_json::json!(dir.to_string_lossy()),
            );
        }
        Request {
            id: 7,
            method: "probe_routing".to_string(),
            params: serde_json::Value::Object(params),
        }
    }

    /// A directory holding a `control.token` with this exact content.
    fn token_dir_holding(token: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("control.token"), token).expect("write token");
        dir
    }

    /// Serve `router` on a free loopback port and return the port.
    async fn serve_on_loopback(router: axum::Router) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        port
    }

    #[tokio::test]
    async fn a_probe_against_a_dead_port_reports_unreachable_with_the_port() {
        // Bind and drop, so the port is real and definitely closed. The
        // token is readable, so nothing but the connection can explain the
        // answer.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);

        let dir = token_dir_holding("a-readable-token");
        let response = handle_probe_routing(&probe_request(port, Some(dir.path()))).await;

        let result = response
            .result
            .expect("a probe answers with a result, never an IPC error");
        assert_eq!(result["outcome"], PROBE_UNREACHABLE);
        assert_eq!(
            result["port"],
            serde_json::json!(port),
            "the port that was tried is the actionable fact"
        );
    }

    #[tokio::test]
    async fn a_probe_that_cannot_read_the_token_names_the_path_it_tried() {
        // An empty directory: the resolution succeeds, the read does not.
        // This is the failure a GUI contributor actually hits, and today it
        // is silently identical to "off".
        let dir = tempfile::tempdir().expect("tempdir");
        let expected = dir.path().join("control.token");
        assert!(
            !expected.exists(),
            "the fixture must not accidentally hold a token"
        );

        // A live proxy on the port, so "unreachable" cannot be the answer
        // and the outcome can only be about the token.
        let port = serve_on_loopback(axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(|| async { r#"{"enabled":true,"exchanges":[]}"# }),
        ))
        .await;

        let response = handle_probe_routing(&probe_request(port, Some(dir.path()))).await;
        let result = response
            .result
            .expect("a probe answers with a result, never an IPC error");
        assert_eq!(result["outcome"], PROBE_TOKEN_UNREADABLE);
        let reported = result["token_path"]
            .as_str()
            .expect("the path that was tried is carried, not merely the failure");
        assert!(
            std::path::Path::new(reported).is_absolute(),
            "a relative path is not something a contributor can act on: {reported}"
        );
        assert_eq!(
            std::path::Path::new(reported),
            expected,
            "the reported path must be the file the reader would actually read"
        );
    }

    #[tokio::test]
    async fn a_probe_never_returns_the_token() {
        // A reachable proxy that accepts the token: the outcome that has
        // most reason to echo it back.
        const SECRET: &str = "c0ntrol-token-that-must-never-be-echoed";
        let dir = token_dir_holding(SECRET);
        let port = serve_on_loopback(axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(|| async { r#"{"enabled":true,"exchanges":[]}"# }),
        ))
        .await;

        let response = handle_probe_routing(&probe_request(port, Some(dir.path()))).await;
        let wire = serde_json::to_string(&response).expect("a response serializes");
        assert!(
            !wire.contains(SECRET),
            "the token is a credential for an API that can rewrite agent configs: {wire}"
        );
        // Field equality, not a substring of the frame: "unreachable"
        // contains "reachable", so a substring check here would call a
        // failed probe a successful one and quietly stop testing the
        // outcome this test claims to cover.
        assert_eq!(
            response
                .result
                .expect("a result, never an IPC error")
                .get("outcome")
                .and_then(serde_json::Value::as_str),
            Some(PROBE_REACHABLE),
            "the test must have exercised the outcome it claims to: {wire}"
        );
    }

    #[tokio::test]
    async fn a_successful_probe_reports_reachable() {
        const TOKEN: &str = "the-right-token";
        let dir = token_dir_holding(TOKEN);
        // The proxy checks the header, so "reachable" also means the token
        // was accepted rather than merely that something answered.
        let port = serve_on_loopback(axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(|req: axum::extract::Request| async move {
                let authorized = req
                    .headers()
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    == Some(concat!("Bearer ", "the-right-token"));
                if authorized {
                    axum::response::Response::builder()
                        .header("content-type", "application/json")
                        .body(axum::body::Body::from(r#"{"enabled":true,"exchanges":[]}"#))
                        .expect("response builds")
                } else {
                    axum::response::Response::builder()
                        .status(401)
                        .body(axum::body::Body::empty())
                        .expect("response builds")
                }
            }),
        ))
        .await;

        let response = handle_probe_routing(&probe_request(port, Some(dir.path()))).await;
        let result = response
            .result
            .expect("a probe answers with a result, never an IPC error");
        assert_eq!(result["outcome"], PROBE_REACHABLE);
        assert_eq!(
            result.get("token_path"),
            None,
            "nothing about the token belongs in a successful answer"
        );
    }

    /// A proxy that answers but rejects the token is the *likely* GUI
    /// failure: a daemon that never saw `IRONWIRE_HOME` read a stale
    /// `~/.ironwire/control.token` and got a live proxy's 401. The
    /// contributor fixes it by naming the directory, so the answer carries
    /// the path, not the port.
    #[tokio::test]
    async fn a_proxy_that_refuses_the_token_reports_it_unreadable_with_the_path() {
        const STALE: &str = "a-stale-token";
        let dir = token_dir_holding(STALE);
        let expected = dir.path().join("control.token");
        let port = serve_on_loopback(axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(|| async {
                axum::response::Response::builder()
                    .status(401)
                    .body(axum::body::Body::empty())
                    .expect("response builds")
            }),
        ))
        .await;

        let response = handle_probe_routing(&probe_request(port, Some(dir.path()))).await;
        let wire = serde_json::to_string(&response).expect("a response serializes");
        assert!(!wire.contains(STALE), "the refused token is still a token");
        let result = response.result.expect("a result, never an IPC error");
        assert_eq!(result["outcome"], PROBE_TOKEN_UNREADABLE);
        assert_eq!(
            result["token_path"].as_str().map(std::path::Path::new),
            Some(expected.as_path())
        );
    }

    /// An empty `control.token` is unreadable in the only sense that
    /// matters: there is no credential in it to send.
    #[tokio::test]
    async fn an_empty_token_file_is_treated_as_unreadable() {
        let dir = token_dir_holding("   \n");
        let response = handle_probe_routing(&probe_request(9, Some(dir.path()))).await;
        let result = response.result.expect("a result, never an IPC error");
        assert_eq!(result["outcome"], PROBE_TOKEN_UNREADABLE);
        assert_eq!(
            result["token_path"].as_str().map(std::path::Path::new),
            Some(dir.path().join("control.token").as_path())
        );
    }

    #[tokio::test]
    async fn a_probe_without_a_usable_port_is_refused() {
        let dir = token_dir_holding("t");
        for params in [
            serde_json::json!({}),
            serde_json::json!({"port": 0}),
            serde_json::json!({"port": 70000}),
            serde_json::json!({"port": "8463"}),
        ] {
            let req = Request {
                id: 7,
                method: "probe_routing".to_string(),
                params,
            };
            let error = handle_probe_routing(&req)
                .await
                .error
                .expect("a probe with no usable port is refused");
            assert_eq!(error.code, ERR_BAD_PARAMS);
            assert_eq!(error.message, "port-invalid");
        }
        // And a token_dir that is not a string is refused rather than
        // silently falling through to the environment.
        let req = Request {
            id: 7,
            method: "probe_routing".to_string(),
            params: serde_json::json!({"port": 8463, "token_dir": 5}),
        };
        let error = handle_probe_routing(&req)
            .await
            .error
            .expect("a non-string token_dir is refused");
        assert_eq!(error.code, ERR_BAD_PARAMS);
        assert_eq!(error.message, "token-dir-invalid");
        drop(dir);
    }

    /// The probe does network I/O, so the synchronous dispatcher says so
    /// rather than answering something it did not perform -- the same
    /// pattern `quiesce` and `preview_body` follow.
    #[test]
    fn the_sync_dispatcher_refuses_probe_routing_as_async_only() {
        let response = handle_request(&shared(), &probe_request(8463, None));
        let error = response.error.expect("the sync path cannot answer a probe");
        assert_eq!(error.code, ERR_UNAVAILABLE);
        assert_eq!(error.message, "probe-routing-requires-async");
    }

    /// The wiring, not the handler. Without the entry in
    /// `handle_request_async` every test above still passes and the method
    /// is dead on the socket -- the async dispatcher is the only path a
    /// real client's request takes, so it is the one that has to answer.
    #[tokio::test]
    async fn the_async_dispatcher_answers_a_probe_for_real() {
        let dir = tempfile::tempdir().expect("tempdir");
        let response =
            handle_request_async(&shared(), &probe_request(8463, Some(dir.path()))).await;
        assert!(
            response.error.is_none(),
            "the async dispatcher must answer rather than refuse: {:?}",
            response.error
        );
        assert_eq!(
            response
                .result
                .expect("a real answer")
                .get("outcome")
                .and_then(serde_json::Value::as_str),
            Some(PROBE_TOKEN_UNREADABLE),
            "an empty token directory is the outcome a real dispatch must reach"
        );
    }

    /// The constants are what the tests above compare against, so a change
    /// to one of their *values* would move the wire contract and every one
    /// of those tests would still pass. These literals are the ones written
    /// into `docs/contributor-daemon-ipc-v1_1.md`, which is what a shell is
    /// built against.
    #[test]
    fn the_probe_outcome_names_are_the_documented_wire_values() {
        assert_eq!(PROBE_REACHABLE, "reachable");
        assert_eq!(PROBE_TOKEN_UNREADABLE, "token_unreadable");
        assert_eq!(PROBE_UNREACHABLE, "unreachable");
    }

    #[test]
    fn probe_routing_is_advertised() {
        assert!(
            METHODS.contains(&"probe_routing"),
            "a method hello does not advertise is a method no shell will call"
        );
    }

    // --- discover_routing ----------------------------------------------

    fn discover_request() -> Request {
        Request {
            id: 11,
            method: "discover_routing".to_string(),
            params: serde_json::Value::Null,
        }
    }

    /// A pointer on disk naming a token file that holds `token`. Returns the
    /// tempdir, which must outlive the assertions.
    fn pointer_dir(port: u16, token: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let token_path = dir.path().join("control.token");
        std::fs::write(&token_path, token).expect("write token");
        std::fs::write(
            dir.path().join("endpoint.json"),
            format!(
                r#"{{"control_url":"http://127.0.0.1:{port}","token_path":"{}"}}"#,
                token_path.display()
            ),
        )
        .expect("write pointer");
        dir
    }

    #[test]
    fn discovery_reports_what_a_running_proxy_published() {
        let dir = pointer_dir(9143, "a-secret-token");
        let _at = super::super::ironwire_pointer::test_support::PointerAt::set(
            &dir.path().join("endpoint.json"),
        );

        let result = handle_discover_routing(&discover_request())
            .result
            .expect("discovery answers with a result, never an IPC error");

        assert_eq!(result["found"], serde_json::json!(true));
        assert_eq!(result["port"], serde_json::json!(9143));
        assert_eq!(
            result["token_path"],
            serde_json::json!(dir.path().join("control.token").to_string_lossy()),
        );
    }

    /// The rule the whole feature hangs on. A machine without IronWire is
    /// the ordinary case, and it must answer -- not error -- so the app can
    /// fall back to asking.
    #[test]
    fn no_pointer_is_answered_as_not_found_rather_than_as_an_error() {
        let _none = super::super::ironwire_pointer::test_support::PointerAt::none();

        let response = handle_discover_routing(&discover_request());

        assert!(
            response.error.is_none(),
            "a machine without IronWire is not an error: {:?}",
            response.error
        );
        let result = response.result.expect("a real answer");
        assert_eq!(result["found"], serde_json::json!(false));
        assert!(
            result.get("port").is_none(),
            "there is no port to report, and reporting one would be invented"
        );
    }

    /// The pointer names a token path; the answer must carry the path and
    /// never the credential at it. This answer crosses a socket to a shell.
    #[test]
    fn discovery_never_carries_the_token() {
        let dir = pointer_dir(9143, "a-secret-token");
        let _at = super::super::ironwire_pointer::test_support::PointerAt::set(
            &dir.path().join("endpoint.json"),
        );

        let response = handle_discover_routing(&discover_request());
        let wire = serde_json::to_string(&response.result.expect("a result")).unwrap();

        assert!(
            !wire.contains("a-secret-token"),
            "the token must not appear anywhere in the answer: {wire}"
        );
    }

    /// Answered by the synchronous dispatcher, which is the path a shell's
    /// request actually takes for a method that opens no connection.
    #[test]
    fn the_sync_dispatcher_answers_discovery_for_real() {
        let _none = super::super::ironwire_pointer::test_support::PointerAt::none();

        let response = handle_request(&shared(), &discover_request());

        assert!(
            response.error.is_none(),
            "discovery must answer on the sync path: {:?}",
            response.error
        );
        assert_eq!(response.result.expect("a result")["found"], false);
    }

    #[test]
    fn discover_routing_is_advertised() {
        assert!(
            METHODS.contains(&"discover_routing"),
            "a method hello does not advertise is a method no shell will call"
        );
    }

    // --- hot swap and the routing status block -------------------------

    /// A declared proxy whose `control.token` is readable, so
    /// `ironwire_ledger_for` actually builds a ledger. Returns the
    /// directory, which must outlive the assertions.
    fn declared_token_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join(super::super::settings::IRONWIRE_TOKEN_FILE),
            "tok\n",
        )
        .unwrap();
        dir
    }

    fn watch_params(port: u16, dir: &std::path::Path) -> serde_json::Value {
        serde_json::json!({
            "ironwire": {
                "mode": "watch",
                "port": port,
                "token_dir": dir.to_string_lossy(),
            }
        })
    }

    /// The point of the hot swap: a contributor who declares the proxy in
    /// the app must get enrichment on the next poll, not after a restart.
    /// Nothing here reloads `DaemonShared`.
    #[test]
    fn a_declaration_change_takes_effect_without_a_restart() {
        let dir = declared_token_dir();
        let s = shared();
        assert!(
            !s.source_roots_with_routing().is_routed(),
            "nothing declared yet, so the roots start bare"
        );

        let r = handle_request(&s, &req("set_settings", watch_params(8463, dir.path())));
        assert!(r.error.is_none(), "{:?}", r.error);

        assert!(
            s.source_roots_with_routing().is_routed(),
            "the declaration must take effect on this same daemon instance"
        );
        assert!(
            s.routing_ledger().is_some(),
            "the rebuilt instance is the one the daemon holds"
        );
    }

    /// `null` means off, and off must actually stop reading.
    #[test]
    fn clearing_the_declaration_drops_the_ledger() {
        let dir = declared_token_dir();
        let s = shared();
        let r = handle_request(&s, &req("set_settings", watch_params(8463, dir.path())));
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(s.routing_ledger().is_some(), "declared, so a ledger exists");

        let r = handle_request(
            &s,
            &req("set_settings", serde_json::json!({"ironwire": null})),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(
            s.routing_ledger().is_none(),
            "off must drop the instance, not just the declaration"
        );
        assert!(
            !s.source_roots_with_routing().is_routed(),
            "a dropped ledger must stop reaching the loaded transcript"
        );
    }

    /// A settings write that does not touch the declaration must not throw
    /// away a warm snapshot. Rebuilding on every `set_settings` would blank
    /// the overlay whenever a contributor moved an unrelated slider.
    #[test]
    fn an_unrelated_settings_change_keeps_the_same_ledger() {
        let dir = declared_token_dir();
        let s = shared();
        handle_request(&s, &req("set_settings", watch_params(8463, dir.path())));
        let before = s.routing_ledger().expect("declared");

        let r = handle_request(
            &s,
            &req(
                "set_settings",
                serde_json::json!({"max_uploads_per_day": 7}),
            ),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        let after = s.routing_ledger().expect("still declared");
        assert!(
            Arc::ptr_eq(&before, &after),
            "an unrelated edit must leave the live instance alone"
        );
    }

    /// Cold start is not an error state and must not be reported as one.
    #[test]
    fn a_rebuilt_ledger_reports_declared_but_nothing_seen() {
        let dir = declared_token_dir();
        let s = shared();
        handle_request(&s, &req("set_settings", watch_params(8463, dir.path())));

        let routing = s.status_value()["routing"].clone();
        assert_eq!(
            routing["state"], ROUTING_AWAITING_ROWS,
            "a freshly rebuilt ledger is declared, not broken: {routing}"
        );
        assert_eq!(
            routing["last_refresh_at"],
            serde_json::Value::Null,
            "nothing has refreshed yet"
        );
    }

    /// The state `has_rows()` alone cannot express.
    #[test]
    fn status_reports_not_declared_when_no_proxy_is_declared() {
        let s = shared();
        let routing = s.status_value()["routing"].clone();
        assert_eq!(routing["state"], ROUTING_NOT_DECLARED, "{routing}");
        assert_eq!(routing["last_refresh_at"], serde_json::Value::Null);
    }

    /// The third state, and the one that proves `last_refresh_at` reports a
    /// refresh that actually reached the proxy rather than one that was
    /// merely attempted.
    #[tokio::test]
    async fn status_reports_rows_seen_after_a_refresh_that_reached_the_proxy() {
        let router = axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "exchanges": [{
                        "started_at": "2026-08-08T10:05:00Z",
                        "client_session_id": "sess-1",
                        "facade": "anthropic",
                        "backend": "claude-sub",
                        "rung": "same_model",
                        "attempts": 1,
                        "status": 200
                    }]
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let dir = declared_token_dir();
        let s = shared();
        handle_request(&s, &req("set_settings", watch_params(port, dir.path())));
        assert_eq!(
            s.status_value()["routing"]["state"],
            ROUTING_AWAITING_ROWS,
            "declared but not yet refreshed"
        );

        s.refresh_routing().await;

        let routing = s.status_value()["routing"].clone();
        assert_eq!(routing["state"], ROUTING_ROWS_SEEN, "{routing}");
        assert!(
            routing["last_refresh_at"].as_str().is_some(),
            "a refresh that reached the proxy stamps a time: {routing}"
        );
    }

    /// An unreachable proxy leaves `last_refresh_at` null: the timestamp
    /// exists so a contributor can tell "the proxy answered and had nothing"
    /// from "the proxy has not answered at all", and stamping it on a failed
    /// attempt would erase that difference.
    #[tokio::test]
    async fn an_unreachable_proxy_stamps_no_refresh_time() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let dir = declared_token_dir();
        let s = shared();
        handle_request(&s, &req("set_settings", watch_params(port, dir.path())));
        s.refresh_routing().await;

        let routing = s.status_value()["routing"].clone();
        assert_eq!(routing["state"], ROUTING_AWAITING_ROWS, "{routing}");
        assert_eq!(
            routing["last_refresh_at"],
            serde_json::Value::Null,
            "an attempt that never reached the proxy is not a refresh"
        );
    }

    /// The whole point of the block is that a shell reads it off the socket.
    /// Unit-testing `status_value` alone would not catch a `status` method
    /// that never returns it -- the failure mode this plan already hit once
    /// with async dispatch.
    #[tokio::test]
    async fn the_routing_block_reaches_a_shell_through_the_real_dispatcher() {
        let dir = declared_token_dir();
        let s = shared();

        let response = handle_request_async(&s, &req("status", serde_json::json!({}))).await;
        let result = response.result.expect("status answers");
        assert_eq!(
            result["routing"]["state"], ROUTING_NOT_DECLARED,
            "{result:?}"
        );

        let response =
            handle_request_async(&s, &req("set_settings", watch_params(8463, dir.path()))).await;
        assert!(response.error.is_none(), "{:?}", response.error);

        let response = handle_request_async(&s, &req("status", serde_json::json!({}))).await;
        let result = response.result.expect("status answers");
        assert_eq!(
            result["routing"]["state"], ROUTING_AWAITING_ROWS,
            "the hot swap must be visible over the socket: {result:?}"
        );
    }

    /// The state names are what every assertion above compares against, so a
    /// change to one of their *values* would move the wire contract with
    /// every one of those tests still green. These literals are the ones
    /// written into `docs/contributor-daemon-ipc-v1_1.md`.
    ///
    /// They are also deliberately chosen so that none is a substring of
    /// another: a shell (or a test) that reached for `contains` rather than
    /// equality would still be answering the right question.
    #[test]
    fn the_routing_state_names_are_the_documented_wire_values() {
        assert_eq!(ROUTING_NOT_DECLARED, "not_declared");
        assert_eq!(ROUTING_AWAITING_ROWS, "awaiting_rows");
        assert_eq!(ROUTING_ROWS_SEEN, "rows_seen");
        for (a, b) in [
            (ROUTING_NOT_DECLARED, ROUTING_AWAITING_ROWS),
            (ROUTING_AWAITING_ROWS, ROUTING_ROWS_SEEN),
            (ROUTING_ROWS_SEEN, ROUTING_NOT_DECLARED),
        ] {
            assert!(!a.contains(b) && !b.contains(a), "{a} and {b} overlap");
        }
    }
}
