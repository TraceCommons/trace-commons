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
//! ## The one capability that differs between the two modes
//!
//! `preview` over the socket is a **summary** -- counts, sizes, and the
//! redacted opening prompt -- by deliberate design of the contract. The full
//! redacted body is available only in-process, through
//! `daemon::ipc::open_preview`. So the shell can show "Exactly what would be
//! sent" and search the transcript **only when it hosts the loop**. Attached
//! to a daemon it did not start, it cannot, and it says so rather than
//! pretending. See `docs/superpowers/plans/linux-shell-report.md`.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon;
use trace_commons_contributor::daemon::ipc::DaemonShared;

/// How the shell reaches the daemon.
pub enum Backend {
    /// A daemon was already running -- almost always the systemd user unit.
    /// Every call is one request over the control socket.
    Attached { store: ConfigStore },
    /// Nothing held the lock, so this process took it and runs the loop
    /// itself. Calls are answered in-process.
    Hosting(Box<Hosting>),
}

pub struct Hosting {
    rt: tokio::runtime::Runtime,
    shared: Arc<DaemonShared>,
    /// Kept alive for the life of the process: dropping it would release
    /// the exclusive lock while the loop is still mutating the queue.
    _embedded: daemon::EmbeddedDaemon,
}

impl Backend {
    /// Attach to a running daemon, or start one in-process.
    ///
    /// The running-daemon check comes first, and a lost race on the lock
    /// falls back to attaching rather than failing: between the check and
    /// the `try_lock`, the user unit may have come up.
    pub fn open(dir: std::path::PathBuf) -> Result<Self> {
        let store = ConfigStore::open(dir.clone())?;
        if daemon::client::is_running(&store) {
            return Ok(Backend::Attached { store });
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        match rt.block_on(daemon::start_embedded(store)) {
            Ok(embedded) => {
                let shared = Arc::clone(&embedded.shared);
                let loop_shared = Arc::clone(&shared);
                rt.spawn(async move {
                    let _ = daemon::run_supervisor(loop_shared, false).await;
                });
                Ok(Backend::Hosting(Box::new(Hosting {
                    rt,
                    shared,
                    _embedded: embedded,
                })))
            }
            // Somebody took the lock in between. That is not an error: it is
            // the other half of the arbitration.
            Err(_) => Ok(Backend::Attached {
                store: ConfigStore::open(dir)?,
            }),
        }
    }

    pub fn hosts_the_loop(&self) -> bool {
        matches!(self, Backend::Hosting(_))
    }

    /// One request, one result. Blocking: callers run this off the GTK
    /// thread (see `worker`), because a `preview` can run the whole
    /// redaction pipeline and, under an external privacy filter, a network
    /// round trip.
    pub fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let response = match self {
            Backend::Attached { store } => {
                match daemon::client::try_call(store, method, &params)? {
                    Some(r) => r,
                    // The daemon was there when we attached and is not now.
                    // A fixed label, and the window's own not-running state
                    // handles the rest.
                    None => bail!("daemon-not-running"),
                }
            }
            Backend::Hosting(h) => daemon::ipc::handle_local(&h.shared, method, params),
        };
        if let Some(err) = response.error {
            // `error.message` is a fixed label by contract, so forwarding it
            // cannot leak a path or a token.
            bail!("{}", err.message);
        }
        response
            .result
            .ok_or_else(|| anyhow!("daemon answered with neither a result nor an error"))
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
                Ok((serde_json::from_value(value)?, None))
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
        }
    }

    /// A blocking iterator over daemon events, for the thread that keeps the
    /// window live. Returns event names only; the shell re-reads state
    /// rather than trusting event payloads, exactly as `resync_required`
    /// requires it to be able to do anyway.
    pub fn events(&self) -> Result<Box<dyn EventStream>> {
        match self {
            Backend::Attached { store } => Ok(Box::new(SocketEvents::open(store)?)),
            Backend::Hosting(h) => Ok(Box::new(BroadcastEvents {
                rx: h.shared.events.subscribe(),
            })),
        }
    }
}

pub trait EventStream: Send {
    /// Blocks until the next event, or returns `None` when the stream ends.
    fn next(&mut self) -> Option<String>;
}

struct BroadcastEvents {
    rx: tokio::sync::broadcast::Receiver<trace_commons_contributor::daemon::ipc::Event>,
}

impl EventStream for BroadcastEvents {
    fn next(&mut self) -> Option<String> {
        loop {
            match self.rx.blocking_recv() {
                Ok(event) => return Some(event.event),
                // Lagged: the shell's answer to a missed event is the same
                // as its answer to `resync_required` -- re-read everything.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    return Some("resync_required".to_string());
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}

/// A persistent subscription over the control socket. One connection for the
/// life of the window, separate from the one-shot request connections, so a
/// slow `preview` cannot stall the event stream.
struct SocketEvents {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
}

impl SocketEvents {
    fn open(store: &ConfigStore) -> Result<Self> {
        use std::io::Write;
        use trace_commons_contributor::config::DAEMON_SOCK_FILE;

        let mut stream =
            std::os::unix::net::UnixStream::connect(store.daemon_path(DAEMON_SOCK_FILE))?;
        stream.write_all(b"{\"id\":1,\"method\":\"subscribe\",\"params\":{}}\n")?;
        stream.flush().ok();
        Ok(Self {
            reader: std::io::BufReader::new(stream),
        })
    }
}

impl EventStream for SocketEvents {
    fn next(&mut self) -> Option<String> {
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
                return Some(name.to_string());
            }
        }
    }
}
