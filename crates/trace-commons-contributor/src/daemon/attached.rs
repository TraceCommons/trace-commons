//! A persistent client for a daemon this process did *not* start.
//!
//! [`client::try_call`] is one request, one response, one connection. That is
//! the right shape for a CLI verb and the wrong one for a window: a shell
//! attached to a running daemon needs the event stream, and the event stream
//! is pushed down the same connection that carried `subscribe`. Opening a
//! fresh connection per call would answer every request correctly and never
//! deliver a single event.
//!
//! So this holds one connection open, writes requests to it, and runs a
//! reader that demultiplexes what comes back: a frame carrying `id` is the
//! answer to a request somebody is waiting for, and a frame carrying `event`
//! is a push with no waiter. `ipc.rs` states that contract at the top of the
//! file -- "pushed events share the connection and a client with two calls in
//! flight must be able to tell which answer is which" -- and this is the
//! client half of it.
//!
//! WHY THIS EXISTS AT ALL. `start_embedded` fails with
//! [`StartFailure::AlreadyRunning`] when another process holds `daemon.lock`,
//! and a shell that treats that as a dead end tells the contributor the
//! watcher is not running while their watcher is running. The daemon they
//! want is up; this is how a shell reaches it.
//!
//! # UNIX ONLY, AND NOT AN OVERSIGHT
//!
//! This holds one connection open and reads it from a background thread
//! while other threads write requests to it. On Unix that is a `UnixStream`,
//! where the two directions are independent.
//!
//! The Windows endpoint is a named pipe opened as a plain `std::fs::File` --
//! a synchronous handle. A blocking read parked on it does not run in
//! parallel with a write to it, so the first real round trip never
//! completes: the CI job for that platform ran one subscribe past sixty
//! seconds and was killed at forty-five minutes. `client::try_call` is
//! unaffected because it writes and then reads, never both at once.
//!
//! Making this work on Windows needs overlapped I/O -- a tokio
//! `NamedPipeClient` -- not a `cfg`. No Windows or GTK shell calls attach
//! today, so [`AttachedDaemon::connect`] reports
//! [`AttachError::UnsupportedTransport`] there rather than shipping a client
//! that deadlocks on first use.
//!
//! WHAT AN ATTACHED CLIENT MAY NOT DO. It must not stop the daemon. The
//! process on the other end may be a `trace-commons-contributor daemon` under
//! a service manager, or another window; a shell that did not start it does
//! not get to end it. `call` refuses `"shutdown"` with
//! [`AttachError::StopRefused`] rather than forwarding it, so the refusal
//! holds no matter which layer asks.

#[cfg(unix)]
use std::collections::HashMap;
#[cfg(unix)]
use std::io::{BufRead, BufReader, Write};
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(unix)]
use std::sync::{Arc, Mutex, mpsc};

#[cfg(unix)]
use super::client::{PlatformStream, connect_for_attach};
use super::ipc::{Event, Response};
use crate::config::ConfigStore;

/// The daemon method an attached client refuses to forward.
///
/// Matched here rather than left to the daemon because the daemon would
/// honour it: the socket has no notion of which client started the process,
/// so the refusal can only live on this side.
#[cfg(unix)]
const METHOD_SHUTDOWN: &str = "shutdown";

#[derive(Debug, thiserror::Error)]
pub enum AttachError {
    /// Nothing is listening on this store's endpoint.
    #[error("no daemon is listening")]
    NotListening,
    /// The daemon accepted the connection and then stopped answering.
    #[error("the attached daemon closed the connection")]
    Disconnected,
    /// Asking an attached client to stop a daemon it did not start.
    #[error("an attached shell may not stop a daemon it did not start")]
    StopRefused,
    /// This platform's daemon endpoint cannot carry a held-open connection.
    /// See the module doc: the Windows named pipe is a synchronous handle.
    #[error("this platform's daemon transport cannot be attached to")]
    UnsupportedTransport,
    #[error("attached daemon transport: {0}")]
    Transport(String),
}

/// A live connection to a daemon running in another process.
#[cfg(unix)]
pub struct AttachedDaemon {
    /// The write half. Serialised because two threads may call at once and a
    /// request frame is only meaningful as a whole line.
    tx: Mutex<PlatformStream>,
    /// Waiters keyed by request id, handed their `Response` by the reader.
    pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Response>>>>,
    /// Pushed frames go here. `None` until `subscribe` installs a sink, so a
    /// client that never subscribes costs nothing to feed.
    sink: Arc<Mutex<Option<Box<dyn Fn(Event) + Send + 'static>>>>,
    next_id: AtomicU64,
    closed: Arc<AtomicBool>,
}

#[cfg(unix)]
impl AttachedDaemon {
    /// Attach to the daemon listening on `store`'s endpoint.
    ///
    /// `Err(NotListening)` means no daemon is there -- including the case of a
    /// socket file left behind by a crashed one, which `connect` already
    /// treats as nothing running.
    pub fn connect(store: &ConfigStore) -> Result<Self, AttachError> {
        let stream = connect_for_attach(store).ok_or(AttachError::NotListening)?;
        let reader_stream = stream
            .try_clone()
            .map_err(|e| AttachError::Transport(e.to_string()))?;
        // `connect` installs a 60-second read timeout, which is right for a
        // one-shot CLI call and fatal here: this reader is parked on the
        // event stream, where sixty quiet seconds is normal and a timeout
        // would end the subscription for good. Clear it on the reader only --
        // the write half keeps its timeout.
        #[cfg(unix)]
        reader_stream
            .set_read_timeout(None)
            .map_err(|e| AttachError::Transport(e.to_string()))?;

        let pending: Arc<Mutex<HashMap<u64, mpsc::Sender<Response>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let sink: Arc<Mutex<Option<Box<dyn Fn(Event) + Send + 'static>>>> =
            Arc::new(Mutex::new(None));
        let closed = Arc::new(AtomicBool::new(false));

        let reader_pending = Arc::clone(&pending);
        let reader_sink = Arc::clone(&sink);
        let reader_closed = Arc::clone(&closed);
        std::thread::Builder::new()
            .name("tc-attached-reader".into())
            .spawn(move || {
                let mut lines = BufReader::new(reader_stream).lines();
                for line in lines.by_ref() {
                    let Ok(line) = line else { break };
                    if line.trim().is_empty() {
                        continue;
                    }
                    // A frame is a response or a push, never both. Response
                    // first: it carries `id`, which a push never does.
                    if let Ok(response) = serde_json::from_str::<Response>(&line) {
                        let waiter = reader_pending.lock().unwrap().remove(&response.id);
                        if let Some(waiter) = waiter {
                            let _ = waiter.send(response);
                        }
                        continue;
                    }
                    if let Ok(event) = serde_json::from_str::<Event>(&line) {
                        if let Some(sink) = reader_sink.lock().unwrap().as_ref() {
                            sink(event);
                        }
                    }
                }
                // The connection is gone. Release every waiter rather than
                // leaving them to their timeouts: a shell blocked forever on
                // a daemon that exited is the failure this whole module is
                // meant to stop producing.
                reader_closed.store(true, Ordering::SeqCst);
                reader_pending.lock().unwrap().clear();
            })
            .map_err(|e| AttachError::Transport(e.to_string()))?;

        Ok(Self {
            tx: Mutex::new(stream),
            pending,
            sink,
            next_id: AtomicU64::new(1),
            closed,
        })
    }

    /// Whether the connection has dropped.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Send one request and wait for its answer.
    pub fn call(&self, method: &str, params: &serde_json::Value) -> Result<Response, AttachError> {
        if method == METHOD_SHUTDOWN {
            return Err(AttachError::StopRefused);
        }
        if self.is_closed() {
            return Err(AttachError::Disconnected);
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);

        let mut line = serde_json::to_string(&serde_json::json!({
            "id": id,
            "method": method,
            "params": params,
        }))
        .map_err(|e| AttachError::Transport(e.to_string()))?;
        line.push('\n');
        {
            let mut stream = self.tx.lock().unwrap();
            if let Err(e) = stream.write_all(line.as_bytes()) {
                self.pending.lock().unwrap().remove(&id);
                return Err(AttachError::Transport(e.to_string()));
            }
            let _ = stream.flush();
        }

        // The reader clears `pending` when the connection drops, which drops
        // this sender and ends the wait -- so a disconnect surfaces as
        // `Disconnected` rather than as a hang.
        rx.recv().map_err(|_| AttachError::Disconnected)
    }

    /// Stop delivering pushed events.
    ///
    /// The connection stays open and `call` keeps working: this drops the
    /// sink only. There is no "unsubscribe" on the wire, and inventing one
    /// would be a protocol change; a host that no longer wants events wants
    /// its callback to stop being invoked, which is exactly this.
    pub fn clear_sink(&self) {
        *self.sink.lock().unwrap() = None;
    }

    /// Install the sink pushed events are delivered to, then subscribe.
    ///
    /// Ordering matters: the daemon answers `subscribe` and then begins
    /// pushing, and `ipc.rs` sends a `snapshot` to the connection that just
    /// subscribed. Installing the sink after the call would drop it.
    pub fn subscribe<F>(&self, sink: F) -> Result<Response, AttachError>
    where
        F: Fn(Event) + Send + 'static,
    {
        *self.sink.lock().unwrap() = Some(Box::new(sink));
        self.call("subscribe", &serde_json::json!({}))
    }
}

/// The same surface on a platform whose endpoint cannot be held open.
///
/// A type rather than a `cfg` at every call site: the FFI reads one field and
/// branches once, and it should not have to know which platforms can attach.
/// Nothing constructs this -- `connect` is the only way in and it always
/// refuses -- so the other methods exist to satisfy the shape and are
/// unreachable.
#[cfg(not(unix))]
pub struct AttachedDaemon {
    _never: std::convert::Infallible,
}

#[cfg(not(unix))]
impl AttachedDaemon {
    pub fn connect(_store: &ConfigStore) -> Result<Self, AttachError> {
        Err(AttachError::UnsupportedTransport)
    }

    pub fn is_closed(&self) -> bool {
        match self._never {}
    }

    pub fn clear_sink(&self) {
        match self._never {}
    }

    pub fn call(
        &self,
        _method: &str,
        _params: &serde_json::Value,
    ) -> Result<Response, AttachError> {
        match self._never {}
    }

    pub fn subscribe<F>(&self, _sink: F) -> Result<Response, AttachError>
    where
        F: Fn(Event) + Send + 'static,
    {
        match self._never {}
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::daemon::start_embedded;

    /// The whole point: a second starter is refused the lock, and attaches
    /// instead of reporting a daemon that is plainly running as absent.
    #[tokio::test(flavor = "multi_thread")]
    async fn attaches_to_a_daemon_this_process_did_not_start() {
        let dir = tempfile::tempdir().unwrap();
        let store_a = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let embedded = start_embedded(store_a).await.unwrap();

        let store_b = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let attached = tokio::task::spawn_blocking(move || {
            let attached = AttachedDaemon::connect(&store_b).unwrap();
            let response = attached.call("status", &serde_json::json!({})).unwrap();
            assert!(
                response.error.is_none(),
                "attached status returned an error frame: {:?}",
                response.error
            );
            assert!(
                response.result.is_some(),
                "attached status carried no result"
            );
            attached
        })
        .await
        .unwrap();

        assert!(!attached.is_closed());
        drop(attached);
        embedded.close();
    }

    /// Subscribing must deliver the snapshot the socket sends to a client
    /// that just subscribed -- the frame the in-process `tc_subscribe` path
    /// never receives.
    #[tokio::test(flavor = "multi_thread")]
    async fn subscribing_delivers_the_snapshot_push() {
        let dir = tempfile::tempdir().unwrap();
        let store_a = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let embedded = start_embedded(store_a).await.unwrap();

        let store_b = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let seen = tokio::task::spawn_blocking(move || {
            let attached = AttachedDaemon::connect(&store_b).unwrap();
            let (tx, rx) = mpsc::channel();
            attached
                .subscribe(move |event| {
                    let _ = tx.send(event.event);
                })
                .unwrap();
            rx.recv_timeout(std::time::Duration::from_secs(10))
        })
        .await
        .unwrap();

        assert_eq!(
            seen.ok().as_deref(),
            Some(super::super::ipc::EVENT_SNAPSHOT),
            "subscribing over the socket did not deliver a snapshot push"
        );
        embedded.close();
    }

    /// An attached shell may not stop a daemon it did not start, and the
    /// refusal is this client's, not the daemon's -- so it holds even though
    /// the socket would happily honour `shutdown`.
    #[tokio::test(flavor = "multi_thread")]
    async fn refuses_to_stop_a_daemon_it_did_not_start() {
        let dir = tempfile::tempdir().unwrap();
        let store_a = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let embedded = start_embedded(store_a).await.unwrap();

        let store_b = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let still_up = tokio::task::spawn_blocking(move || {
            let attached = AttachedDaemon::connect(&store_b).unwrap();
            let err = attached
                .call("shutdown", &serde_json::json!({}))
                .expect_err("an attached client forwarded shutdown");
            assert!(matches!(err, AttachError::StopRefused), "got {err:?}");
            // The daemon is still answering, which is the fact the refusal
            // is for -- asserting the error alone would pass just as well
            // against a client that had killed it.
            attached
                .call("status", &serde_json::json!({}))
                .unwrap()
                .error
                .is_none()
        })
        .await
        .unwrap();

        assert!(
            still_up,
            "the daemon stopped answering after a refused stop"
        );
        embedded.close();
    }

    #[test]
    fn attaching_where_nothing_listens_is_not_listening() {
        let (_d, store) = crate::config::tests_support::temp_store();
        assert!(matches!(
            AttachedDaemon::connect(&store),
            Err(AttachError::NotListening)
        ));
    }
}
