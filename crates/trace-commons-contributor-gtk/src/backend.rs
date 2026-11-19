//! Connect to the daemon, or be it.
//!
//! On Linux the separate daemon under the systemd user unit is the primary
//! deployment and this application is an optional client over its socket --
//! the inverse of macOS, where the app hosts the loop in-process. Both have
//! to work here: a contributor who only ever runs the app should not have to
//! learn what a user unit is, and a contributor who runs the unit should not
//! have the app fail on the lock.
//!
//! The existing exclusive lock arbitrates, and the arbitration is the whole
//! of the logic below: try to take it, and if somebody else has it, connect
//! to them instead. There is no other coordination and no second lock.
//!
//! Attached previews use the same digest-anchored `preview_body` pages as
//! other socket clients. A summary alone cannot enable contribution review.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon;
use trace_commons_contributor::daemon::ipc::DaemonShared;
use trace_commons_contributor::daemon::settings::SourceDeclaration;

/// How the shell reaches the daemon.
pub enum Backend {
    /// A daemon was already running -- almost always the systemd user unit.
    /// Every call is one request over `socket`: the state directory's
    /// control socket, or, from a Flatpak sandbox, the host daemon's host
    /// link (`daemon::host_link`).
    Attached { socket: std::path::PathBuf },
    /// Nothing held the lock, so this process took it and runs the loop
    /// itself. Calls are answered in-process.
    Hosting(Box<Hosting>),
    /// `TRACE_COMMONS_SAMPLE` named a recorded set: no daemon, no state
    /// directory, no lock, and every answer read from the recordings the
    /// macOS app draws from (see `crate::sample`). Debug builds only; a
    /// release build has no such variant.
    #[cfg(debug_assertions)]
    Sample(crate::sample::SampleBackend),
}

pub struct Hosting {
    rt: tokio::runtime::Runtime,
    shared: Arc<DaemonShared>,
    /// The supervise loop, so [`Backend::stop`] can wait for its pass to
    /// end before the lock is released.
    supervisor: tokio::task::JoinHandle<()>,
    /// Kept alive until [`Backend::stop`] (or the end of the process):
    /// dropping it would release the exclusive lock while the loop is still
    /// mutating the queue.
    embedded: daemon::EmbeddedDaemon,
}

/// How long [`Backend::stop`] waits for the loop's pass, then for the
/// runtime's tasks.
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The label this shell reports when the contributor has not said which
/// session folders to watch.
///
/// The same string the C ABI uses, deliberately: macOS and Windows read it
/// off `tc_daemon_start`, this shell produces it directly, and one label
/// across the product means the three onboarding flows can route on the same
/// fact rather than each inventing a spelling.
pub const ERR_ROOTS_NOT_DECLARED: &str = "roots-not-declared";
/// A declaration named a source this build does not know. Fixed and
/// content-free like every other label here: the input that would identify
/// it is an adapter name, and the failure is refused rather than written.
pub const ERR_UNKNOWN_SOURCE: &str = "unknown-source";

/// Record the two session roots the contributor named, so the next
/// `Backend::open` can start.
///
/// Goes through `apply_settings_object` rather than assigning the fields,
/// for the reason `tc_daemon_start_with_settings` shares `set_settings`'
/// validator: one definition of a valid settings object. This shell links
/// the core directly, so this is that same definition rather than a second
/// implementation of it -- the objection that keeps Swift and C# from
/// writing `daemon-settings.json` themselves does not apply to a caller
/// that is already Rust.
///
/// Refuses an incomplete declaration here rather than writing one and
/// letting the next start refuse: a half-written settings file is a worse
/// thing to leave behind than an unanswered question.
pub fn declare_sources(dir: &std::path::Path, answers: &[(&str, SourceDeclaration)]) -> Result<()> {
    let store = ConfigStore::open(dir.to_path_buf())?;
    let mut settings = daemon::settings::DaemonSettings::load_for_preferences(&store)?;
    // Every answer the screen collected, keyed by adapter name rather than
    // by a list of fields kept here. A source the roots screen can now
    // discover but this function had never heard of would otherwise be
    // rendered, answered, and silently dropped.
    //
    // The `_source` spellings, not the `_root` ones: a path can say where to
    // watch but cannot say "off", and off is the answer this exists to make
    // expressible.
    let mut object = serde_json::Map::new();
    for (source, declaration) in answers {
        let key = daemon::settings::source_settings_key(source)
            .ok_or_else(|| anyhow!(ERR_UNKNOWN_SOURCE))?;
        object.insert(key.to_string(), serde_json::to_value(declaration)?);
    }
    daemon::settings::apply_settings_object(&mut settings, &serde_json::Value::Object(object))
        .map_err(|label| anyhow!(label))?;
    if !daemon::settings::roots_declared(&settings) {
        bail!(ERR_ROOTS_NOT_DECLARED);
    }
    settings.save(&store)?;
    Ok(())
}

/// Declare both sources as folders to watch.
///
/// Kept as the two-paths spelling for callers that have two paths; the
/// window goes through [`declare_sources`], because a folder chooser cannot
/// express "I don't use this agent".
pub fn declare_roots(
    dir: &std::path::Path,
    claude_root: &std::path::Path,
    codex_root: &std::path::Path,
) -> Result<()> {
    declare_sources(
        dir,
        &[
            (
                trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                SourceDeclaration::Watch {
                    path: claude_root.to_path_buf(),
                },
            ),
            (
                trace_commons_contributor::source::SOURCE_CODEX,
                SourceDeclaration::Watch {
                    path: codex_root.to_path_buf(),
                },
            ),
        ],
    )
}

impl Backend {
    /// Attach to a running daemon, or start one in-process.
    ///
    /// The running-daemon check comes first, and a lost race on the lock
    /// falls back to attaching rather than failing: between the check and
    /// the `try_lock`, the user unit may have come up.
    ///
    /// Starting one in-process fails closed on undeclared session roots, on
    /// the rule `daemon::settings::roots_declared` owns. Until this landed
    /// the Linux shell had no such check at all: it started, and an unset
    /// `claude_root` meant the daemon watched the contributor's real
    /// `~/.claude`.
    ///
    /// The ATTACH path is deliberately not gated. A daemon that is already
    /// running was started by `trace-commons-contributor daemon` -- somebody
    /// typing a command on purpose -- and the CLI keeps its own defaults by
    /// design. This is an application-shell posture, and refusing to attach
    /// to a daemon this shell did not start would be a different decision
    /// than the one made here.
    pub fn open(dir: std::path::PathBuf) -> Result<Self> {
        #[cfg(debug_assertions)]
        {
            Self::open_or_sample(dir, std::env::var(crate::sample::SAMPLE_ENV).ok())
        }
        #[cfg(not(debug_assertions))]
        {
            Self::open_daemon(dir)
        }
    }

    /// The sample `sample` names, before anything else is touched -- the
    /// state directory, a running daemon, the lock -- or else the daemon.
    #[cfg(debug_assertions)]
    fn open_or_sample(dir: std::path::PathBuf, sample: Option<String>) -> Result<Self> {
        match crate::sample::SampleBackend::from_value(sample.as_deref()) {
            Some(sample) => Ok(Backend::Sample(sample)),
            None => Self::open_daemon(dir),
        }
    }

    /// The daemon path: a host daemon from a sandbox ([`Backend::open_with`]),
    /// else attach to or start this state directory's daemon.
    fn open_daemon(dir: std::path::PathBuf) -> Result<Self> {
        Self::open_with(
            dir,
            daemon::host_link::runs_sandboxed(),
            daemon::host_link::socket_path(),
        )
    }

    /// [`Backend::open`] with the sandbox fact and the host link's path
    /// given, so the preference order is testable off Linux.
    ///
    /// Inside a Flatpak sandbox, a host daemon -- listening, or installed and
    /// down -- comes first: the sandbox cannot open a host terminal, a daemon
    /// in it stops when the window closes, and a second daemon there would
    /// watch the same sessions.
    /// Without one, the path below runs exactly as outside a sandbox. See
    /// `docs/superpowers/specs/2026-11-17-linux-flatpak-host-daemon-design.md`.
    pub fn open_with(
        dir: std::path::PathBuf,
        sandboxed: bool,
        host_socket: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        Self::open_at(dir, sandboxed, host_socket, None)
    }

    /// [`Backend::open_with`], with the place a hosted daemon takes
    /// `watcher.lock` and serves the host link given (`None`: from the
    /// environment), so a test can see that lock without touching the real
    /// runtime directory.
    pub fn open_at(
        dir: std::path::PathBuf,
        sandboxed: bool,
        host_socket: Option<std::path::PathBuf>,
        site: Option<daemon::HostLinkSite>,
    ) -> Result<Self> {
        let host = || match daemon::host_link::route(
            sandboxed,
            host_socket.as_deref(),
            daemon::host_link::probe,
        ) {
            daemon::host_link::Route::Host(socket) => Some(Backend::Attached { socket }),
            daemon::host_link::Route::Local => None,
        };
        if let Some(attached) = host() {
            return Ok(attached);
        }
        let store = ConfigStore::open(dir.clone())?;
        let local = store.daemon_path(trace_commons_contributor::config::DAEMON_SOCK_FILE);
        if daemon::client::is_running(&store) {
            return Ok(Backend::Attached { socket: local });
        }
        let settings = daemon::settings::DaemonSettings::load(&store)?;
        if !daemon::settings::roots_declared(&settings) {
            bail!(ERR_ROOTS_NOT_DECLARED);
        }
        // This process is about to host the daemon, and a GTK app's stderr
        // is usually unwatched: record panic locations and warn/error
        // callsites (never messages) in the state directory, as the CLI and
        // the FFI handle do. Returns the dev dry-run store when one applies.
        let store = daemon::diagnostics_log::install_for(store)?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        match rt.block_on(daemon::start_embedded_at(store, site)) {
            Ok(embedded) => {
                let shared = Arc::clone(&embedded.shared);
                let loop_shared = Arc::clone(&shared);
                let supervisor = rt.spawn(async move {
                    let _ = daemon::run_supervisor(loop_shared, false).await;
                });
                Ok(Backend::Hosting(Box::new(Hosting {
                    rt,
                    shared,
                    supervisor,
                    embedded,
                })))
            }
            // Somebody took the lock in between. That is not an error: it is
            // the other half of the arbitration. The somebody may hold this
            // state directory's lock, or only the watcher lock: the host
            // daemon (from a sandbox), or a daemon in the sandbox (from the
            // host).
            Err(_) => Ok(Self::attach_after_lost_start(
                local,
                host_socket.as_deref(),
                sandboxed,
            )),
        }
    }

    /// Where to attach after this process lost the start to another daemon:
    /// whichever socket answers, else this state directory's socket, which
    /// shows the window's existing not-running state.
    pub fn attach_after_lost_start(
        local: std::path::PathBuf,
        host_socket: Option<&std::path::Path>,
        sandboxed: bool,
    ) -> Self {
        // In a sandbox, the host part comes first, as in `open_with`.
        if let daemon::host_link::Route::Host(socket) =
            daemon::host_link::route(sandboxed, host_socket, daemon::host_link::probe)
        {
            return Backend::Attached { socket };
        }
        if daemon::client::is_running_at(&local) {
            return Backend::Attached { socket: local };
        }
        // On the host: a daemon that holds the watcher lock but not this
        // state directory's lock serves the host link, so reach it there.
        // A daemon in the sandbox serves no host link and nothing here can
        // reach it; the local socket then gives the window its existing
        // not-running state.
        match host_socket {
            Some(socket) if daemon::client::is_running_at(socket) => Backend::Attached {
                socket: socket.to_path_buf(),
            },
            _ => Backend::Attached { socket: local },
        }
    }

    pub fn hosts_the_loop(&self) -> bool {
        matches!(self, Backend::Hosting(_))
    }

    /// The recorded set this backend answers from, if it is a sample.
    pub fn sample_set(&self) -> Option<&'static str> {
        match self {
            #[cfg(debug_assertions)]
            Backend::Sample(sample) => Some(sample.set()),
            _ => None,
        }
    }

    /// One request, one result. Blocking: callers run this off the GTK
    /// thread (see `worker`), because a `preview` can run the whole
    /// redaction pipeline and, under an external privacy filter, a network
    /// round trip.
    pub fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let response = match self {
            Backend::Attached { socket } => {
                match daemon::client::try_call_at(socket, method, &params)? {
                    Some(r) => r,
                    // The daemon was there when we attached and is not now.
                    // A fixed label, and the window's own not-running state
                    // handles the rest.
                    None => bail!("daemon-not-running"),
                }
            }
            Backend::Hosting(h) => daemon::ipc::handle_local(&h.shared, method, params),
            #[cfg(debug_assertions)]
            Backend::Sample(sample) => {
                let _ = params;
                return sample.call(method);
            }
        };
        response_result(response)
    }

    /// How many times `needle` appears in an entry's PRE-redaction session
    /// text. `None` on any failure.
    ///
    /// A COUNT, never content -- that is the whole bound of the daemon call
    /// behind this, and the reason it is allowed to read unredacted bytes at
    /// all. `None` rather than a `Result` because the one thing the caller
    /// must not do is round a failure off to a clean answer; see
    /// [`crate::original_search::Outcome::Unknown`].
    pub fn search_original(&self, entry_id: &str, needle: &str) -> Option<u32> {
        // One arm for both modes: `handle_local` runs the same async
        // dispatcher the socket does, so a hosting build routes this method
        // without a second implementation to keep in step.
        let value = self
            .call(
                "search_original",
                serde_json::json!({ "entry_id": entry_id, "needle": needle }),
            )
            .ok()?;
        value["matches"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
    }

    /// The redacted body and the summary together, for the preview sheet.
    ///
    /// `Ok(None)` for the body means "this deployment cannot serve it" --
    /// attached to a daemon we do not host -- not "there is none". The sheet
    /// renders that honestly instead of showing an empty transcript.
    pub fn preview(
        &self,
        entry_id: &str,
    ) -> Result<(crate::model::PreviewSummary, Option<String>)> {
        match self {
            Backend::Attached { .. } => {
                let value = self.call("preview", serde_json::json!({ "entry_id": entry_id }))?;
                let summary: crate::model::PreviewSummary = serde_json::from_value(value)?;
                let body = read_preview_body(entry_id, &summary, |params| {
                    self.call("preview_body", params)
                })?;
                Ok((summary, Some(body)))
            }
            Backend::Hosting(h) => {
                let id: uuid::Uuid = entry_id.parse().map_err(|_| anyhow!("entry-id-invalid"))?;
                let shared = Arc::clone(&h.shared);
                let (summary, body) =
                    h.rt.block_on(daemon::ipc::open_preview(&shared, id))
                        .map_err(|label| anyhow!("{label}"))?;
                let summary = serde_json::from_value(serde_json::to_value(&summary)?)?;
                Ok((summary, Some(body)))
            }
            // No transcript is recorded, and none is made up here: the
            // preview is refused, as an unrecorded method is.
            #[cfg(debug_assertions)]
            Backend::Sample(_) => Err(anyhow!(crate::sample::ERR_SAMPLE_UNRECORDED)),
        }
    }

    /// A blocking iterator over daemon events, for the thread that keeps the
    /// window live. Returns event names only, with one narrow exception --
    /// see [`DaemonEvent`] -- and never a payload the shell would have to
    /// trust as current state; the shell re-reads state itself, exactly as
    /// `resync_required` requires it to be able to do anyway.
    /// Stop the daemon this window runs, if it runs one, and release its
    /// locks -- the state directory's and the watcher lock -- so the
    /// background service can take over. The order is the C ABI's
    /// (`tc_daemon_stop`): ask the loop to stop, wait (bounded) for its pass
    /// to end, then close, so nothing writes after the lock is gone. An
    /// attached backend has nothing to stop.
    ///
    /// Whether the locks are released: `false` when the runtime did not end
    /// within [`STOP_WAIT`]. The locks then stay held until it does, and the
    /// caller must not expect another daemon to take them yet. Blocking (up
    /// to twice `STOP_WAIT`); the worker thread calls it, never GTK's.
    pub fn stop(self) -> bool {
        let Backend::Hosting(hosting) = self else {
            return true;
        };
        let Hosting {
            rt,
            shared,
            mut supervisor,
            embedded,
        } = *hosting;
        shared
            .shutdown
            .store(true, std::sync::atomic::Ordering::Relaxed);
        shared.shutdown_signal.notify_one();
        let ended = rt.block_on(async {
            tokio::time::timeout(STOP_WAIT, &mut supervisor)
                .await
                .is_ok()
        });
        if !ended {
            supervisor.abort();
        }
        drop(shared);
        // Every task, blocking work included, ends before the locks go.
        release_after_shutdown(rt, move || embedded.close(), STOP_WAIT)
    }

    /// The socket an attached window talks to; `None` while hosting, and
    /// on a sample.
    pub fn attached_socket(&self) -> Option<&std::path::Path> {
        match self {
            Backend::Attached { socket } => Some(socket),
            Backend::Hosting(_) => None,
            #[cfg(debug_assertions)]
            Backend::Sample(_) => None,
        }
    }

    /// A subscription over a socket named directly.
    pub fn events_at(socket: &std::path::Path) -> Result<Box<dyn EventStream>> {
        Ok(Box::new(SocketEvents::open(socket)?))
    }

    pub fn events(&self) -> Result<Box<dyn EventStream>> {
        match self {
            Backend::Attached { socket } => Ok(Box::new(SocketEvents::open(socket)?)),
            Backend::Hosting(h) => Ok(Box::new(BroadcastEvents {
                rx: h.shared.events.subscribe(),
            })),
            #[cfg(debug_assertions)]
            Backend::Sample(_) => Ok(Box::new(crate::sample::NoEvents)),
        }
    }
}

/// Shut `rt` down, then `release`, on a thread of their own. Whether both
/// finished within `bound`.
///
/// Dropping a runtime waits for every task on it, `spawn_blocking` work
/// included (tokio's blocking pool shuts down with no timeout on drop), so
/// `release` -- which lets go of the daemon's locks -- never runs while
/// something of the old daemon can still write. `shutdown_timeout` gave no
/// such promise: past its bound it returned with blocking work still
/// running. When the bound passes first, this returns `false` and the
/// thread finishes the release whenever the runtime does.
pub(crate) fn release_after_shutdown(
    rt: tokio::runtime::Runtime,
    release: impl FnOnce() + Send + 'static,
    bound: std::time::Duration,
) -> bool {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(rt);
        release();
        let _ = done.send(());
    });
    finished.recv_timeout(bound).is_ok()
}

/// A daemon answer that was an error: its fixed label, and whatever
/// `result` the daemon sent beside it.
///
/// `Display` is the label and nothing else, so every caller that renders or
/// matches `to_string()` sees exactly what it saw before the payload was
/// kept. The payload is for the few handlers that read a structured error on
/// purpose -- an interactive witness review that met a busy witness carries
/// `view.retry_at` there -- and is never rendered as a whole.
#[derive(Debug, Clone, PartialEq)]
pub struct DaemonFailure {
    pub label: String,
    pub result: Option<serde_json::Value>,
}

impl std::fmt::Display for DaemonFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl std::error::Error for DaemonFailure {}

impl From<String> for DaemonFailure {
    fn from(label: String) -> Self {
        Self {
            label,
            result: None,
        }
    }
}

impl DaemonFailure {
    /// Recover the daemon's payload from an error `Backend::call` returned.
    /// Any other failure -- a socket that went away, a malformed answer --
    /// is its label alone.
    pub fn from_error(error: anyhow::Error) -> Self {
        match error.downcast::<DaemonFailure>() {
            Ok(failure) => failure,
            Err(error) => error.to_string().into(),
        }
    }
}

/// One response, one result.
fn response_result(response: daemon::ipc::Response) -> Result<serde_json::Value> {
    if let Some(err) = response.error {
        // `error.message` is a fixed label by contract, so forwarding it
        // cannot leak a path or a token. The `result` beside it is kept, not
        // displayed: see `DaemonFailure`.
        return Err(anyhow::Error::new(DaemonFailure {
            label: err.message,
            result: response.result,
        }));
    }
    response
        .result
        .ok_or_else(|| anyhow!("daemon answered with neither a result nor an error"))
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    use trace_commons_contributor::daemon::ipc::Response;

    fn busy_response() -> Response {
        let mut response = Response::err(7, "unavailable", "witness_saturated");
        response.result = Some(serde_json::json!({
            "view": {"state": "Busy", "retry_at": "2026-09-27T12:00:00Z"}
        }));
        response
    }

    /// An error answer keeps its `result`, and still reads as its label.
    #[test]
    fn an_error_answer_keeps_its_result_beside_the_label() {
        let error = response_result(busy_response()).unwrap_err();
        assert_eq!(error.to_string(), "witness_saturated");
        let failure = DaemonFailure::from_error(error);
        assert_eq!(failure.label, "witness_saturated");
        assert_eq!(
            failure.result.as_ref().map(|r| r["view"]["state"].clone()),
            Some(serde_json::json!("Busy"))
        );
    }

    /// An error with no payload, and a failure that never reached the
    /// daemon, carry no payload: nothing is invented.
    #[test]
    fn a_failure_without_a_payload_is_its_label_alone() {
        let error = response_result(Response::err(1, "unavailable", "preview-failed")).unwrap_err();
        assert_eq!(
            DaemonFailure::from_error(error),
            DaemonFailure {
                label: "preview-failed".into(),
                result: None
            }
        );
        assert_eq!(
            DaemonFailure::from_error(anyhow!("daemon-not-running")),
            DaemonFailure {
                label: "daemon-not-running".into(),
                result: None
            }
        );
    }

    /// A success is unchanged.
    #[test]
    fn a_success_is_its_result() {
        let mut response = Response::err(1, "x", "y");
        response.error = None;
        response.result = Some(serde_json::json!({"ok": true}));
        assert_eq!(
            response_result(response).unwrap(),
            serde_json::json!({"ok": true})
        );
    }
}

/// One daemon event, as this shell's event pump sees it.
///
/// `entry_id` is populated for exactly one event, `preview_ready`, and is
/// `None` for every other. This is a deliberate, narrow exception to "names
/// only", not a reopening of the rule: an entry id is an identifier, not
/// state. Reading it only tells the shell *which* card to re-ask the daemon
/// about -- `App::handle_preview_ready` still calls `preview_request` and
/// still trusts nothing but that fresh answer. The alternative -- forgetting
/// which entry resolved and re-checking every card still outstanding on
/// every `preview_ready` -- is what turned filling a 500-card queue into
/// tens of thousands of redundant requests; see
/// `docs/superpowers/specs/2026-08-20-preview-scheduler-design.md`.
pub struct DaemonEvent {
    pub name: String,
    pub entry_id: Option<String>,
    /// Populated for exactly one event, `digest_due`, and `None` for every
    /// other.
    ///
    /// This is the second narrow exception to "names only", added for the
    /// same shape of reason as `entry_id` and held to the same limit. A
    /// digest can now be about traces that were contributed without ever
    /// being queued, and an armed project queues nothing -- so the shell
    /// cannot recover these numbers by looking at its own entries, the way
    /// it recovers the waiting count. It is counts and labels: no path, no
    /// id, no state the shell then reasons from. Everything else on the
    /// payload stays unread.
    pub digest: Option<DigestFacts>,
}

/// What a `digest_due` event says happened since the last digest.
///
/// Labels, not keys: these are the daemon's own display strings, already
/// reduced from paths, and they go straight into notification text that a
/// desktop environment may persist.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DigestFacts {
    pub contributed: usize,
    pub contributed_projects: Vec<String>,
    pub credit_final: f64,
    pub credit_pending: f64,
}

/// `digest_due`'s `data` is built in `daemon::tick`. Absent fields decode as
/// zero and an empty list, which degrades this to the waiting-only digest
/// that shipped before rather than to a wrong number -- the case that
/// matters when a new shell attaches to an older daemon.
fn digest_facts(name: &str, data: &serde_json::Value) -> Option<DigestFacts> {
    if name != trace_commons_contributor::daemon::ipc::EVENT_DIGEST_DUE {
        return None;
    }
    Some(DigestFacts {
        contributed: data
            .get("contributed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        contributed_projects: data
            .get("contributed_projects")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        credit_final: data
            .get("credit_final")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        credit_pending: data
            .get("credit_pending")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    })
}

/// `preview_ready`'s `data` is always `PreviewOutcome::to_value`, which
/// always carries `entry_id` -- see `daemon::preview_scheduler`. Every other
/// event's `data` is left untouched: this reads the one field this shell
/// ever trusts off an event payload, and only for the one event it is
/// documented to carry it.
fn preview_ready_entry_id(name: &str, data: &serde_json::Value) -> Option<String> {
    if name != trace_commons_contributor::daemon::ipc::EVENT_PREVIEW_READY {
        return None;
    }
    data.get("entry_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

pub trait EventStream: Send {
    /// Blocks until the next event, or returns `None` when the stream ends.
    fn next(&mut self) -> Option<DaemonEvent>;
}

struct BroadcastEvents {
    rx: tokio::sync::broadcast::Receiver<trace_commons_contributor::daemon::ipc::Event>,
}

impl EventStream for BroadcastEvents {
    fn next(&mut self) -> Option<DaemonEvent> {
        match self.rx.blocking_recv() {
            Ok(event) => Some(DaemonEvent {
                entry_id: preview_ready_entry_id(&event.event, &event.data),
                digest: digest_facts(&event.event, &event.data),
                name: event.event,
            }),
            // Lagged: the shell's answer to a missed event is the same as
            // its answer to `resync_required` -- re-read everything.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => Some(DaemonEvent {
                name: "resync_required".to_string(),
                entry_id: None,
                digest: None,
            }),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    }
}

/// The one subscription frame. `feature_flags_changed` is opt-in on a
/// socket (`daemon::ipc::OPT_IN_EVENTS`), so it is named here, as the Mac
/// names it; without it an attached window would never hear that a flag
/// changed. `reengage_due`, the other opt-in, is not: naming it declares
/// this window a renderer of notifications it does not draw. A daemon too
/// old to know the name drops it.
const SUBSCRIBE_FRAME: &str =
    "{\"id\":1,\"method\":\"subscribe\",\"params\":{\"accepts\":[\"feature_flags_changed\"]}}\n";

/// A persistent subscription over the control socket. One connection for the
/// life of the window, separate from the one-shot request connections, so a
/// slow `preview` cannot stall the event stream.
struct SocketEvents {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
}

impl SocketEvents {
    fn open(socket: &std::path::Path) -> Result<Self> {
        use std::io::Write;

        let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
        stream.write_all(SUBSCRIBE_FRAME.as_bytes())?;
        stream.flush().ok();
        Ok(Self {
            reader: std::io::BufReader::new(stream),
        })
    }
}

impl EventStream for SocketEvents {
    fn next(&mut self) -> Option<DaemonEvent> {
        use std::io::BufRead;
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                continue;
            };
            // Events carry no `id`; that is how the contract says to tell
            // them from the `subscribe` response sharing this connection.
            if let Some(name) = value.get("event").and_then(|v| v.as_str()) {
                let data = value.get("data").unwrap_or(&serde_json::Value::Null);
                let entry_id = preview_ready_entry_id(name, data);
                let digest = digest_facts(name, data);
                return Some(DaemonEvent {
                    name: name.to_string(),
                    entry_id,
                    digest,
                });
            }
        }
    }
}

#[cfg(test)]
mod roots_tests {
    use super::*;
    use trace_commons_contributor::daemon::settings::DaemonSettings;

    /// A scratch directory that removes itself.
    ///
    /// Hand-rolled rather than pulling in `tempfile`: this crate is its own
    /// workspace with its own lockfile, so a dev-dependency here is a real
    /// new package edge, and the repo's dependency policy asks for an inline
    /// utility when one fits in a few lines.
    pub(super) struct Scratch(std::path::PathBuf);

    impl Scratch {
        pub(super) fn new(tag: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("tc-gtk-{tag}-{unique}"));
            std::fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }

        pub(super) fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(super) fn declare(dir: &std::path::Path, claude: Option<&str>, codex: Option<&str>) {
        let store = ConfigStore::open(dir.to_path_buf()).unwrap();
        let make = |name: &str| {
            let p = dir.join(name);
            std::fs::create_dir_all(&p).unwrap();
            p
        };
        use trace_commons_contributor::daemon::settings::SourceDeclaration;
        DaemonSettings {
            claude_source: claude
                .map(make)
                .map(|path| SourceDeclaration::Watch { path }),
            codex_source: codex
                .map(make)
                .map(|path| SourceDeclaration::Watch { path }),
            ..Default::default()
        }
        .save(&store)
        .unwrap();
    }

    #[test]
    fn open_refuses_to_host_a_daemon_when_no_roots_are_declared() {
        let dir = Scratch::new("no-roots");
        let message = match Backend::open(dir.path().to_path_buf()) {
            Ok(_) => panic!("undeclared roots must refuse to start"),
            Err(e) => e.to_string(),
        };
        assert_eq!(
            message, ERR_ROOTS_NOT_DECLARED,
            "the label has to be exactly the one the other two shells route on"
        );
    }

    /// The host link's socket in a scratch runtime directory, and the
    /// state directory a sandboxed window would have.
    fn host_link_fixture(tag: &str) -> (Scratch, Scratch, std::path::PathBuf) {
        let state = Scratch::new(&format!("{tag}s"));
        let run = Scratch::new(&format!("{tag}r"));
        let sock = run
            .path()
            .join(trace_commons_contributor::daemon::host_link::HOST_LINK_SOCK_FILE);
        (state, run, sock)
    }

    fn attached_socket(backend: &Backend) -> &std::path::Path {
        match backend {
            Backend::Attached { socket } => socket,
            Backend::Hosting(_) => panic!("expected to attach, not to host"),
            #[cfg(debug_assertions)]
            Backend::Sample(_) => panic!("expected to attach, not a sample"),
        }
    }

    #[test]
    fn in_a_sandbox_a_listening_host_daemon_is_attached_before_anything_else() {
        // Undeclared roots in the sandbox's own state directory: the
        // in-sandbox path would refuse. The host daemon comes first.
        let (state, _run, sock) = host_link_fixture("hl1");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let backend = Backend::open_with(state.path().to_path_buf(), true, Some(sock.clone()))
            .expect("attached to the host daemon");
        assert_eq!(attached_socket(&backend), sock);
        assert!(!backend.hosts_the_loop());
    }

    #[test]
    fn in_a_sandbox_a_stopped_host_daemon_is_attached_not_replaced() {
        // The socket file a stopped host daemon leaves. A second daemon in
        // the sandbox would watch the same sessions; the window's existing
        // not-running state reports this instead.
        let (state, _run, sock) = host_link_fixture("hl2");
        drop(std::os::unix::net::UnixListener::bind(&sock).unwrap());
        // The systemd unit's marker: the host part is installed.
        std::fs::write(
            sock.with_file_name(
                trace_commons_contributor::daemon::host_link::HOST_UNIT_MARKER_FILE,
            ),
            b"",
        )
        .unwrap();
        let backend = Backend::open_with(state.path().to_path_buf(), true, Some(sock.clone()))
            .expect("attached, not an error that would end the process");
        assert_eq!(attached_socket(&backend), sock);
        assert_eq!(
            backend
                .call("status", serde_json::json!({}))
                .unwrap_err()
                .to_string(),
            "daemon-not-running"
        );
    }

    #[test]
    fn in_a_sandbox_a_socket_some_other_host_daemon_left_is_not_a_host_part() {
        // A one-off `daemon run` that crashed, with no unit marker: the
        // window runs its own daemon, here refused only on its roots.
        let (state, _run, sock) = host_link_fixture("hl6");
        drop(std::os::unix::net::UnixListener::bind(&sock).unwrap());
        let message = match Backend::open_with(state.path().to_path_buf(), true, Some(sock)) {
            Ok(_) => panic!("no unit marker: the in-sandbox path"),
            Err(e) => e.to_string(),
        };
        assert_eq!(message, ERR_ROOTS_NOT_DECLARED);
    }

    #[test]
    fn a_lost_start_on_the_host_attaches_to_a_listening_host_link() {
        // The window on the host lost the start to a daemon that took the
        // watcher lock and serves the host link but not this state
        // directory's socket.
        let (state, _run, sock) = host_link_fixture("hl7");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let local = state.path().join("daemon.sock");
        let backend = Backend::attach_after_lost_start(local.clone(), Some(&sock), false);
        assert_eq!(attached_socket(&backend), sock);

        // This state directory's own daemon, when it answers, comes first.
        let _own = std::os::unix::net::UnixListener::bind(&local).unwrap();
        let backend = Backend::attach_after_lost_start(local.clone(), Some(&sock), false);
        assert_eq!(attached_socket(&backend), local);
    }

    #[test]
    fn a_lost_start_with_nothing_reachable_shows_not_running() {
        // A daemon in the Flatpak sandbox holds the watcher lock and serves
        // no host link: nothing here can reach it.
        let (state, _run, sock) = host_link_fixture("hl8");
        let local = state.path().join("daemon.sock");
        let backend = Backend::attach_after_lost_start(local.clone(), Some(&sock), false);
        assert_eq!(attached_socket(&backend), local);
        assert_eq!(
            backend
                .call("status", serde_json::json!({}))
                .unwrap_err()
                .to_string(),
            "daemon-not-running"
        );
    }

    #[test]
    fn in_a_sandbox_without_a_host_part_the_window_runs_as_today() {
        let (state, _run, sock) = host_link_fixture("hl3");
        let message = match Backend::open_with(state.path().to_path_buf(), true, Some(sock)) {
            Ok(_) => panic!("no host part: the in-sandbox path refuses undeclared roots"),
            Err(e) => e.to_string(),
        };
        assert_eq!(message, ERR_ROOTS_NOT_DECLARED);

        let message = match Backend::open_with(state.path().to_path_buf(), true, None) {
            Ok(_) => panic!("no runtime directory: the in-sandbox path"),
            Err(e) => e.to_string(),
        };
        assert_eq!(message, ERR_ROOTS_NOT_DECLARED);
    }

    #[test]
    fn outside_a_sandbox_the_host_link_is_ignored() {
        let (state, _run, sock) = host_link_fixture("hl4");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let message = match Backend::open_with(state.path().to_path_buf(), false, Some(sock)) {
            Ok(_) => panic!("outside a sandbox the order is unchanged"),
            Err(e) => e.to_string(),
        };
        assert_eq!(message, ERR_ROOTS_NOT_DECLARED);
    }

    #[test]
    fn calls_attached_over_the_host_link_reach_the_host_daemon() {
        use std::io::{BufRead, BufReader, Write};
        let (state, _run, sock) = host_link_fixture("hl5");
        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let backend =
            Backend::open_with(state.path().to_path_buf(), true, Some(sock)).expect("attached");
        // The probe in `open_with` connected once; answer the next one.
        let _probe = listener.accept().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            (&stream)
                .write_all(b"{\"id\":1,\"result\":{\"from\":\"host\"}}\n")
                .unwrap();
        });
        let value = backend.call("status", serde_json::json!({})).unwrap();
        server.join().unwrap();
        assert_eq!(value["from"], "host");
    }

    #[test]
    fn open_refuses_when_only_one_root_is_declared() {
        // The fail-open an `||` would have allowed: an unset codex_root is
        // the real ~/.codex, not "no codex source".
        let dir = Scratch::new("half-roots");
        declare(dir.path(), Some("claude"), None);

        let message = match Backend::open(dir.path().to_path_buf()) {
            Ok(_) => panic!("half a declaration must refuse"),
            Err(e) => e.to_string(),
        };
        assert_eq!(message, ERR_ROOTS_NOT_DECLARED);
    }

    #[test]
    fn declare_roots_turns_a_refusal_into_a_start() {
        // The whole exit from the refusal, end to end: refused, the
        // contributor names two folders, and the next open hosts.
        let dir = Scratch::new("declare");
        assert!(Backend::open(dir.path().to_path_buf()).is_err());

        let claude = dir.path().join("claude");
        let codex = dir.path().join("codex");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        declare_roots(dir.path(), &claude, &codex).unwrap();

        let backend = match Backend::open(dir.path().to_path_buf()) {
            Ok(b) => b,
            Err(e) => panic!("after declaring roots the shell must start: {e}"),
        };
        assert!(backend.hosts_the_loop());
    }

    #[test]
    fn declare_roots_survives_a_folder_name_that_would_break_string_building() {
        // These come from a file chooser. A quote or a backslash in a folder
        // name must round-trip, which is why the settings object is built as
        // JSON rather than formatted.
        let dir = Scratch::new("awkward");
        let claude = dir.path().join(r#"He said "hi"\back"#);
        let codex = dir.path().join("codex");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();

        declare_roots(dir.path(), &claude, &codex).unwrap();

        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let settings = DaemonSettings::load(&store).unwrap();
        assert_eq!(
            settings.claude_source.as_ref().and_then(|d| d.path()),
            Some(claude.as_path())
        );
    }

    #[test]
    fn declaring_an_agent_off_is_a_real_answer_that_lets_the_daemon_start() {
        // "I don't use Codex" has to be an answer, not a silence. Before
        // `SourceDeclaration` the only way to express it was to leave the
        // root unset, and an unset root means the real ~/.codex -- so the
        // answer a privacy-conscious contributor is most likely to give was
        // the one that scanned their work.
        let dir = Scratch::new("codex-off");
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(&claude).unwrap();

        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Watch {
                        path: claude.clone(),
                    },
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
            ],
        )
        .unwrap();

        let backend = match Backend::open(dir.path().to_path_buf()) {
            Ok(b) => b,
            Err(e) => panic!("an off declaration is complete and must start: {e}"),
        };
        assert!(backend.hosts_the_loop());
    }

    #[test]
    fn an_agent_declared_off_never_reaches_the_conventional_location() {
        // Asserting on the declaration itself, not on a count: the failure
        // that matters is not "a source went missing" but "a source is
        // rooted at the contributor's home directory".
        let dir = Scratch::new("off-not-home");
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(&claude).unwrap();

        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Watch { path: claude },
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
            ],
        )
        .unwrap();

        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let settings = DaemonSettings::load(&store).unwrap();
        assert_eq!(settings.codex_source, Some(SourceDeclaration::Off));
        assert_eq!(
            settings.codex_source.as_ref().and_then(|d| d.path()),
            None,
            "off must resolve to no directory at all, not to a fallback"
        );
    }

    /// The roots screen renders one row per discovered source, and every
    /// row it renders must be written. A source the screen can show but
    /// this function drops would ask the contributor a question and then
    /// ignore the answer.
    #[test]
    fn every_answered_source_is_written_not_just_the_two_that_gate_the_start() {
        let dir = Scratch::new("every-answer");
        let gemini = dir.path().join("gemini");
        std::fs::create_dir_all(&gemini).unwrap();
        let cline = dir.path().join("cline");
        std::fs::create_dir_all(&cline).unwrap();

        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Off,
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
                (
                    trace_commons_contributor::source::SOURCE_GEMINI_CLI,
                    SourceDeclaration::Watch {
                        path: gemini.clone(),
                    },
                ),
                (
                    trace_commons_contributor::source::SOURCE_CLINE,
                    SourceDeclaration::Watch {
                        path: cline.clone(),
                    },
                ),
            ],
        )
        .unwrap();

        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let settings = DaemonSettings::load(&store).unwrap();
        assert_eq!(
            settings.gemini_source,
            Some(SourceDeclaration::Watch { path: gemini }),
            "the contributor said to watch it; the answer must survive"
        );
        assert_eq!(
            settings.cline_source,
            Some(SourceDeclaration::Watch { path: cline }),
            "the contributor said to watch it; the answer must survive"
        );
    }

    /// A source this build has no settings key for is refused, not written
    /// under a guessed field name.
    #[test]
    fn an_unknown_source_name_is_refused() {
        let dir = Scratch::new("unknown-source");
        let err = declare_sources(dir.path(), &[("not-an-adapter", SourceDeclaration::Off)])
            .expect_err("an unknown source must refuse");
        assert_eq!(err.to_string(), ERR_UNKNOWN_SOURCE);
    }

    #[test]
    fn both_agents_off_is_a_complete_declaration() {
        // Somebody who uses neither agent still has to be able to leave the
        // screen. Refusing here would be the dead end this whole slice
        // exists to remove.
        let dir = Scratch::new("both-off");
        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Off,
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
            ],
        )
        .unwrap();

        let backend = match Backend::open(dir.path().to_path_buf()) {
            Ok(b) => b,
            Err(e) => panic!("two off declarations are still a declaration: {e}"),
        };
        assert!(backend.hosts_the_loop());
    }

    #[test]
    fn an_off_declaration_survives_a_reload_rather_than_reverting_to_unasked() {
        // The regression that would matter most quietly: if `off` failed to
        // round-trip through the settings file it would load back as None,
        // which is "never asked" -- and never-asked is the state that falls
        // back to the real location.
        let dir = Scratch::new("off-round-trip");
        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Off,
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
            ],
        )
        .unwrap();

        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let reloaded = DaemonSettings::load(&store).unwrap();
        assert_eq!(reloaded.claude_source, Some(SourceDeclaration::Off));
        assert_eq!(reloaded.codex_source, Some(SourceDeclaration::Off));
    }

    #[test]
    fn open_hosts_a_daemon_once_both_roots_are_declared() {
        let dir = Scratch::new("both-roots");
        declare(dir.path(), Some("claude"), Some("codex"));

        let backend = match Backend::open(dir.path().to_path_buf()) {
            Ok(b) => b,
            Err(e) => panic!("declared roots must start: {e}"),
        };
        assert!(
            backend.hosts_the_loop(),
            "nothing else held the lock, so this shell should be hosting"
        );
    }

    /// A stop whose runtime still has blocking work after the bound must
    /// not release the lock under it: the release waits for the runtime to
    /// end, and the stop reports that it did not finish.
    #[test]
    fn the_lock_is_released_only_after_every_runtime_task_has_ended() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::<&str>::new()));
        let started = std::sync::Arc::new(std::sync::Barrier::new(2));
        {
            let order = std::sync::Arc::clone(&order);
            let started = std::sync::Arc::clone(&started);
            rt.spawn_blocking(move || {
                started.wait();
                std::thread::sleep(std::time::Duration::from_millis(400));
                order.lock().unwrap().push("blocking-ended");
            });
        }
        started.wait();
        let released = std::sync::Arc::clone(&order);
        let finished = release_after_shutdown(
            rt,
            move || released.lock().unwrap().push("released"),
            std::time::Duration::from_millis(50),
        );
        assert!(!finished, "the stop must report that it did not finish");
        assert!(
            order.lock().unwrap().is_empty(),
            "released while blocking work still ran"
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while order.lock().unwrap().len() < 2 {
            assert!(std::time::Instant::now() < deadline, "never released");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(*order.lock().unwrap(), ["blocking-ended", "released"]);
    }

    #[test]
    fn a_runtime_that_ends_in_time_is_released_at_once() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let released = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&released);
        assert!(release_after_shutdown(
            rt,
            move || flag.store(true, std::sync::atomic::Ordering::SeqCst),
            std::time::Duration::from_secs(5),
        ));
        assert!(released.load(std::sync::atomic::Ordering::SeqCst));
    }

    /// The switch-over to the background service stops the daemon this
    /// window runs, and only after that may another start on the same
    /// state directory: the lock is released, not leaked.
    #[test]
    fn stopping_a_hosted_daemon_releases_its_lock_for_the_next_one() {
        use trace_commons_contributor::config::DAEMON_LOCK_FILE;
        let dir = Scratch::new("stop-host");
        declare(dir.path(), Some("claude"), Some("codex"));
        let backend = Backend::open(dir.path().to_path_buf()).expect("hosts");
        assert!(backend.hosts_the_loop());
        let lock_path = ConfigStore::open(dir.path().to_path_buf())
            .unwrap()
            .daemon_path(DAEMON_LOCK_FILE);
        backend.stop();
        if let Ok(lock) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
        {
            assert!(lock.try_lock().is_ok(), "the stopped daemon kept its lock");
        }
        let again = Backend::open(dir.path().to_path_buf()).expect("hosts again");
        assert!(again.hosts_the_loop());
        again.stop();
    }

    /// The cross-sandbox lock too: a daemon this window hosts on the
    /// platform default state directory holds `watcher.lock` in the runtime
    /// directory, and stopping it releases that lock for the next daemon --
    /// the background service, after a switch-over.
    #[cfg(unix)]
    #[test]
    fn stopping_a_hosted_daemon_releases_the_watcher_lock_for_the_next_one() {
        let dir = Scratch::new("stop-watcher");
        declare(dir.path(), Some("claude"), Some("codex"));
        // A short path: the host link's socket is bound in here.
        let run = std::path::PathBuf::from(format!("/tmp/tc-wl-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&run).unwrap();
        let run = Scratch(run);
        let site = daemon::HostLinkSite {
            runtime: run.path().join("rt"),
            platform_default: dir.path().to_path_buf(),
            sandboxed: false,
            host_unit: false,
        };
        let watcher = site.runtime.join(daemon::host_link::WATCHER_LOCK_FILE);
        let held = |path: &std::path::Path| -> bool {
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .expect("watcher.lock exists");
            matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
        };

        let backend = Backend::open_at(dir.path().to_path_buf(), false, None, Some(site.clone()))
            .expect("hosts");
        assert!(backend.hosts_the_loop());
        assert!(held(&watcher), "a hosted daemon holds watcher.lock");
        assert!(backend.stop(), "the stop finished");
        assert!(!held(&watcher), "the stopped daemon kept watcher.lock");

        let next = Backend::open_at(dir.path().to_path_buf(), false, None, Some(site))
            .expect("the next daemon starts");
        assert!(next.hosts_the_loop());
        assert!(held(&watcher), "the next daemon holds watcher.lock");
        assert!(next.stop());
        assert!(!held(&watcher));
    }
}

/// Join bounded UTF-8 pages only while both the body and envelope anchors hold.
fn read_preview_body(
    entry_id: &str,
    summary: &crate::model::PreviewSummary,
    mut fetch: impl FnMut(serde_json::Value) -> Result<serde_json::Value>,
) -> Result<String> {
    let mut body = String::new();
    let mut digest: Option<String> = None;
    let mut total: Option<usize> = None;
    loop {
        let mut params = serde_json::json!({"entry_id": entry_id, "offset": body.len()});
        if let Some(digest) = &digest {
            params["body_digest"] = digest.clone().into();
        }
        let page = fetch(params)?;
        let chunk = page
            .get("chunk")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow!("preview-body-invalid"))?;
        let page_digest = page
            .get("body_digest")
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("preview-body-invalid"))?;
        let page_total = page
            .get("total_bytes")
            .and_then(serde_json::Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| anyhow!("preview-body-invalid"))?;
        if page_total > trace_commons_contributor::envelope::MAX_ENVELOPE_BYTES
            || total.is_some_and(|expected| expected != page_total)
            || digest
                .as_deref()
                .is_some_and(|expected| expected != page_digest)
            || page
                .get("envelope_digest")
                .and_then(serde_json::Value::as_str)
                != Some(summary.envelope_digest.as_str())
            || page.get("enrolled").and_then(serde_json::Value::as_bool) != Some(summary.enrolled)
            || body.len().saturating_add(chunk.len()) > page_total
        {
            bail!("preview-body-changed");
        }
        total = Some(page_total);
        digest = Some(page_digest.to_string());
        body.push_str(chunk);
        match page.get("next_offset") {
            Some(serde_json::Value::Null) if body.len() == page_total => return Ok(body),
            Some(next)
                if !chunk.is_empty()
                    && body.len() < page_total
                    && next.as_u64() == Some(body.len() as u64) => {}
            _ => bail!("preview-body-incomplete"),
        }
    }
}

/// The feature-flag snapshot over the daemon this shell hosts or attaches
/// to: the read the window makes (`App::refresh_feature_flags`), and the
/// change event an attached window has to ask for.
#[cfg(test)]
mod feature_flag_tests {
    use super::roots_tests::{Scratch, declare};
    use super::*;
    use crate::feature_flags::{self, EVENT_FEATURE_FLAGS_CHANGED};
    use trace_commons_contributor::config::DAEMON_SOCK_FILE;

    fn snapshot_of(backend: &Backend) -> feature_flags::Snapshot {
        let answer = backend
            .call(feature_flags::METHOD_SNAPSHOT, serde_json::json!({}))
            .map_err(|e| e.to_string());
        feature_flags::read_answer(answer, feature_flags::build_identity().as_ref())
            .expect("the daemon's snapshot is read and is this build's")
    }

    /// The frame is one line the daemon takes, naming exactly the change
    /// event and never the notification renderer's.
    #[test]
    fn the_subscription_names_the_change_event_and_nothing_else() {
        assert_eq!(SUBSCRIBE_FRAME.matches('\n').count(), 1);
        let frame: serde_json::Value = serde_json::from_str(SUBSCRIBE_FRAME.trim_end()).unwrap();
        assert_eq!(frame["method"], "subscribe");
        let accepts = &frame["params"]["accepts"];
        assert!(daemon::ipc::subscribe_accepts_valid(accepts));
        assert_eq!(accepts, &serde_json::json!([EVENT_FEATURE_FLAGS_CHANGED]));
    }

    /// The daemon this shell hosts answers a snapshot this shell reads,
    /// about this build's own profile.
    #[test]
    fn a_hosted_daemons_snapshot_is_read_and_is_this_builds() {
        let dir = Scratch::new("ff-read");
        declare(dir.path(), Some("claude"), Some("codex"));
        let backend = Backend::open(dir.path().to_path_buf()).expect("hosts");
        let snapshot = snapshot_of(&backend);
        assert!(snapshot.flags.contains_key("insights_skills"));
        backend.stop();
    }

    /// `feature_flags_changed` is opt-in on a socket: a subscriber that
    /// does not name it never receives it. An attached window's
    /// subscription names it, and a changed flag reaches it.
    #[test]
    fn an_attached_subscription_receives_feature_flag_changes() {
        let dir = Scratch::new("ff-event");
        declare(dir.path(), Some("claude"), Some("codex"));
        let backend = Backend::open(dir.path().to_path_buf()).expect("hosts");
        let snapshot = snapshot_of(&backend);
        let socket = ConfigStore::open(dir.path().to_path_buf())
            .unwrap()
            .daemon_path(DAEMON_SOCK_FILE);
        let mut events = Backend::events_at(&socket).expect("subscribes");
        let (names, received) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Some(event) = events.next() {
                if names.send(event.name).is_err() {
                    return;
                }
            }
        });
        // The subscription is registered when the daemon has read its
        // frame, which this cannot observe; each toggle is a new generation
        // and its own event, so toggle until one arrives.
        let mut arrived = false;
        for attempt in 0..40 {
            backend
                .call(
                    "feature_flags_override_set",
                    serde_json::json!({
                        "id": "insights_skills",
                        "value": attempt % 2 == 0,
                        "environment": snapshot.environment.as_str(),
                        "profile_digest": snapshot.profile_digest,
                    }),
                )
                .expect("a product flag takes a local choice");
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
            while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
                match received.recv_timeout(left) {
                    Ok(name) if name == EVENT_FEATURE_FLAGS_CHANGED => {
                        arrived = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            if arrived {
                break;
            }
        }
        assert!(arrived, "the attached subscription never saw the change");
        backend.stop();
    }
}

#[cfg(test)]
mod native_witness_body_tests {
    use super::*;

    fn summary() -> crate::model::PreviewSummary {
        serde_json::from_value(
            serde_json::json!({"envelope_digest":"witness-sha256:artifact", "enrolled":true}),
        )
        .unwrap()
    }

    fn page(chunk: &str, next: serde_json::Value, digest: &str) -> serde_json::Value {
        serde_json::json!({"chunk":chunk, "next_offset":next, "total_bytes":4,
            "body_digest":digest, "envelope_digest":"witness-sha256:artifact", "enrolled":true})
    }

    #[test]
    fn witness_body_reassembles_exact_bytes_and_passes_continuation_anchor() {
        let mut requests = Vec::new();
        let body = read_preview_body("entry", &summary(), |params| {
            requests.push(params);
            Ok(if requests.len() == 1 {
                page("ab", 2.into(), "same")
            } else {
                page("cd", serde_json::Value::Null, "same")
            })
        })
        .unwrap();
        assert_eq!(body, "abcd");
        assert!(requests[0].get("body_digest").is_none());
        assert_eq!(requests[1]["body_digest"], "same");
        assert_eq!(requests[1]["offset"], 2);
    }

    #[test]
    fn witness_body_rejects_changed_anchor_partial_or_stalled_page() {
        for second in [
            page("cd", serde_json::Value::Null, "changed"),
            page("c", serde_json::Value::Null, "same"),
            page("", 2.into(), "same"),
        ] {
            let mut first = true;
            assert!(
                read_preview_body("entry", &summary(), |_| {
                    Ok(if std::mem::take(&mut first) {
                        page("ab", 2.into(), "same")
                    } else {
                        second.clone()
                    })
                })
                .is_err()
            );
        }
    }
}

#[cfg(test)]
#[cfg(debug_assertions)]
mod sample_tests {
    use super::*;

    fn untouched_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("tc-gtk-sample-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn a_named_sample_opens_without_touching_the_state_directory() {
        let dir = untouched_dir();
        let backend = Backend::open_or_sample(dir.clone(), Some("heldSessions".into())).unwrap();
        assert_eq!(backend.sample_set(), Some("heldSessions"));
        assert!(!backend.hosts_the_loop(), "a sample hosts no loop");
        assert!(!dir.exists(), "the sample created {}", dir.display());
        let status = backend.call("status", serde_json::json!({})).unwrap();
        assert!(status.is_object());
        let refused = backend
            .call("set_settings", serde_json::json!({"x": 1}))
            .unwrap_err();
        assert_eq!(
            DaemonFailure::from_error(refused).label,
            crate::sample::ERR_SAMPLE_UNRECORDED
        );
        assert_eq!(
            backend
                .preview("00000000-0000-0000-0000-000000000000")
                .unwrap_err()
                .to_string(),
            crate::sample::ERR_SAMPLE_UNRECORDED
        );
        assert!(backend.events().unwrap().next().is_none());
        assert!(backend.search_original("x", "y").is_none());
        assert!(!dir.exists());
    }

    #[test]
    fn an_unrecognised_or_empty_name_is_the_default_set() {
        for value in ["", "noSuchSet"] {
            let backend = Backend::open_or_sample(untouched_dir(), Some(value.into())).unwrap();
            assert_eq!(
                backend.sample_set(),
                Some(crate::sample::DEFAULT_SET),
                "{value:?}"
            );
        }
    }

    #[test]
    fn an_unset_variable_opens_the_daemon_path() {
        // The daemon path's first act is to refuse undeclared roots, so a
        // fresh directory shows the daemon path was taken, not the sample.
        let dir = untouched_dir();
        let error = Backend::open_or_sample(dir.clone(), None)
            .err()
            .expect("not a sample");
        assert_eq!(error.to_string(), ERR_ROOTS_NOT_DECLARED);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The sample is impossible in a release build: the variable's name is
    /// spelled only in `sample.rs`, whose declaration, like every reference
    /// to the module and the variant, is under `cfg(debug_assertions)`.
    #[test]
    fn the_sample_is_compiled_only_into_debug_builds() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let lib = std::fs::read_to_string(src.join("lib.rs")).unwrap();
        assert!(lib.contains("#[cfg(debug_assertions)]\npub mod sample;"));
        let mut stack = vec![src.clone()];
        let mut spelled = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                if text.contains(concat!("TRACE_COMMONS_", "SAMPLE\"")) {
                    spelled.push(path.strip_prefix(&src).unwrap().display().to_string());
                }
                // Every line naming the module or the variant sits under the
                // gate, directly or in a gated block or item. A gated test
                // module is gated whole, so its lines are not read.
                let production = text.split(concat!("mod sample", "_tests")).next().unwrap();
                let lines: Vec<&str> = production.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    let code = line.trim_start();
                    if code.starts_with("//") || path.ends_with("sample.rs") {
                        continue;
                    }
                    if code.contains("::sample::")
                        || code.contains("Backend::Sample")
                        || code.starts_with("Sample(")
                    {
                        let gated = lines[..i]
                            .iter()
                            .rev()
                            .take(12)
                            .any(|l| l.contains("cfg(debug_assertions)"));
                        assert!(
                            gated,
                            "{}:{} is not under cfg(debug_assertions)",
                            path.display(),
                            i + 1
                        );
                    }
                }
            }
        }
        assert_eq!(spelled, vec!["sample.rs".to_string()]);
    }
}
