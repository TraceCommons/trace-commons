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
//! # PERSISTENT TRANSPORTS
//!
//! This holds one connection open and reads it from a background thread
//! while other threads write requests to it. On Unix that is a `UnixStream`.
//!
//! On Windows, a Tokio `NamedPipeClient` supplies overlapped I/O. The
//! one-shot `client::try_call` still uses a synchronous file handle because
//! it writes and reads sequentially; this persistent client must read and
//! write concurrently to carry pushed events.
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
use std::net::Shutdown;
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
#[cfg(unix)]
use std::sync::{Arc, Mutex, mpsc};
#[cfg(unix)]
use std::time::Duration;

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
    #[error("the attached daemon did not answer in time")]
    TimedOut,
    /// Asking an attached client to stop a daemon it did not start.
    #[error("an attached shell may not stop a daemon it did not start")]
    StopRefused,
    /// This platform's daemon endpoint cannot carry a held-open connection.
    /// See the module doc for the supported Unix and Windows transports.
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
    sink: Arc<Mutex<Option<Arc<Mutex<Box<dyn Fn(Event) + Send + 'static>>>>>>,
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
        let sink: Arc<Mutex<Option<Arc<Mutex<Box<dyn Fn(Event) + Send + 'static>>>>>> =
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
                        let callback = reader_sink.lock().unwrap().clone();
                        if let Some(sink) = callback {
                            (sink.lock().unwrap())(event);
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
        self.call_with_timeout(method, params, Duration::from_secs(60))
    }

    /// Status checks use a shorter bound so a silent daemon cannot freeze the shell.
    pub fn call_with_timeout(
        &self,
        method: &str,
        params: &serde_json::Value,
        timeout: Duration,
    ) -> Result<Response, AttachError> {
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
        match rx.recv_timeout(timeout) {
            Ok(response) => Ok(response),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(AttachError::Disconnected),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().unwrap().remove(&id);
                self.close();
                Err(AttachError::TimedOut)
            }
        }
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

    /// Close this client connection without sending the daemon's shutdown
    /// method. This is for a shell that is exiting after attaching to a
    /// daemon it did not start; closing the socket wakes the subscription
    /// reader and leaves the daemon running for its owner.
    pub fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        if let Ok(stream) = self.tx.lock() {
            let _ = stream.shutdown(Shutdown::Both);
        }
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
        *self.sink.lock().unwrap() = Some(Arc::new(Mutex::new(Box::new(sink))));
        self.call("subscribe", &serde_json::json!({}))
    }
}

/// Windows implementation. Tokio's named-pipe client uses overlapped I/O,
/// allowing one task to keep the event stream open while accepting writes
/// from synchronous callers.
#[cfg(windows)]
mod windows_attached {
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::Duration;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
    use tokio::sync::mpsc as tokio_mpsc;
    use tokio::time::Instant;
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

    use super::{AttachError, ConfigStore, Event, Response};

    const METHOD_SHUTDOWN: &str = "shutdown";
    const PIPE_OPEN_RETRY_WINDOW: Duration = Duration::from_millis(500);
    const PIPE_OPEN_RETRY_DELAY: Duration = Duration::from_millis(25);

    enum Command {
        Call {
            id: u64,
            line: String,
            waiter: mpsc::Sender<Response>,
        },
        Close,
    }

    /// A live, persistent connection to a daemon running in another process.
    pub struct AttachedDaemon {
        commands: tokio_mpsc::UnboundedSender<Command>,
        sink: Arc<Mutex<Option<Arc<Mutex<Box<dyn Fn(Event) + Send + 'static>>>>>>,
        next_id: AtomicU64,
        closed: Arc<AtomicBool>,
    }

    impl AttachedDaemon {
        /// Attach to the daemon listening on `store`'s named pipe.
        pub fn connect(store: &ConfigStore) -> Result<Self, AttachError> {
            let pipe_name = super::super::win_pipe::pipe_name(store);
            let (commands, command_rx) = tokio_mpsc::unbounded_channel();
            let (ready_tx, ready_rx) = mpsc::channel();
            let sink: Arc<Mutex<Option<Arc<Mutex<Box<dyn Fn(Event) + Send + 'static>>>>>> =
                Arc::new(Mutex::new(None));
            let closed = Arc::new(AtomicBool::new(false));

            let thread_sink = Arc::clone(&sink);
            let thread_closed = Arc::clone(&closed);
            std::thread::Builder::new()
                .name("tc-attached-pipe".into())
                .spawn(move || {
                    let runtime = match tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        Ok(runtime) => runtime,
                        Err(error) => {
                            let _ = ready_tx.send(Err(AttachError::Transport(error.to_string())));
                            return;
                        }
                    };

                    let client = match runtime.block_on(open_pipe(&pipe_name)) {
                        Ok(client) => client,
                        Err(error) => {
                            let attach_error = if error.kind() == std::io::ErrorKind::NotFound {
                                AttachError::NotListening
                            } else {
                                AttachError::Transport(error.to_string())
                            };
                            let _ = ready_tx.send(Err(attach_error));
                            return;
                        }
                    };

                    if ready_tx.send(Ok(())).is_err() {
                        return;
                    }
                    runtime.block_on(run_connection(
                        client,
                        command_rx,
                        thread_sink,
                        thread_closed,
                    ));
                })
                .map_err(|error| AttachError::Transport(error.to_string()))?;

            match ready_rx.recv() {
                Ok(Ok(())) => Ok(Self {
                    commands,
                    sink,
                    next_id: AtomicU64::new(1),
                    closed,
                }),
                Ok(Err(error)) => Err(error),
                Err(error) => Err(AttachError::Transport(error.to_string())),
            }
        }

        /// Whether the connection has dropped.
        pub fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }

        /// Send one request and wait for its answer.
        pub fn call(
            &self,
            method: &str,
            params: &serde_json::Value,
        ) -> Result<Response, AttachError> {
            self.call_with_timeout(method, params, Duration::from_secs(60))
        }

        /// Status checks use a shorter bound so a silent daemon cannot freeze the shell.
        pub fn call_with_timeout(
            &self,
            method: &str,
            params: &serde_json::Value,
            timeout: Duration,
        ) -> Result<Response, AttachError> {
            if method == METHOD_SHUTDOWN {
                return Err(AttachError::StopRefused);
            }
            if self.is_closed() {
                return Err(AttachError::Disconnected);
            }

            let id = self.next_id.fetch_add(1, Ordering::SeqCst);
            let (waiter, response) = mpsc::channel();
            let mut line = serde_json::to_string(&serde_json::json!({
                "id": id,
                "method": method,
                "params": params,
            }))
            .map_err(|error| AttachError::Transport(error.to_string()))?;
            line.push('\n');
            self.commands
                .send(Command::Call { id, line, waiter })
                .map_err(|_| AttachError::Disconnected)?;

            match response.recv_timeout(timeout) {
                Ok(response) => Ok(response),
                Err(mpsc::RecvTimeoutError::Disconnected) => Err(AttachError::Disconnected),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    self.close();
                    Err(AttachError::TimedOut)
                }
            }
        }

        /// Stop delivering pushed events while leaving the connection usable.
        pub fn clear_sink(&self) {
            *self.sink.lock().unwrap() = None;
        }

        /// Close this client without asking the daemon to stop.
        pub fn close(&self) {
            self.closed.store(true, Ordering::SeqCst);
            let _ = self.commands.send(Command::Close);
        }

        /// Install the sink before subscribing so the initial snapshot is delivered.
        pub fn subscribe<F>(&self, sink: F) -> Result<Response, AttachError>
        where
            F: Fn(Event) + Send + 'static,
        {
            *self.sink.lock().unwrap() = Some(Arc::new(Mutex::new(Box::new(sink))));
            self.call("subscribe", &serde_json::json!({}))
        }
    }

    impl Drop for AttachedDaemon {
        fn drop(&mut self) {
            self.close();
        }
    }

    async fn open_pipe(name: &str) -> std::io::Result<NamedPipeClient> {
        let started = Instant::now();
        loop {
            match ClientOptions::new().open(name) {
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                        && started.elapsed() < PIPE_OPEN_RETRY_WINDOW =>
                {
                    tokio::time::sleep(PIPE_OPEN_RETRY_DELAY).await;
                }
                result => return result,
            }
        }
    }

    async fn run_connection(
        client: NamedPipeClient,
        mut commands: tokio_mpsc::UnboundedReceiver<Command>,
        sink: Arc<Mutex<Option<Arc<Mutex<Box<dyn Fn(Event) + Send + 'static>>>>>>,
        closed: Arc<AtomicBool>,
    ) {
        let (read_half, mut write_half) = tokio::io::split(client);
        let mut lines = BufReader::new(read_half).lines();
        let mut pending = HashMap::<u64, mpsc::Sender<Response>>::new();

        loop {
            tokio::select! {
                line = lines.next_line() => {
                    match line {
                        Ok(Some(line)) if !line.trim().is_empty() => {
                            if let Ok(response) = serde_json::from_str::<Response>(&line) {
                                if let Some(waiter) = pending.remove(&response.id) {
                                    let _ = waiter.send(response);
                                }
                            } else if let Ok(event) = serde_json::from_str::<Event>(&line) {
                                let callback = sink.lock().unwrap().clone();
                                if let Some(sink) = callback {
                                    (sink.lock().unwrap())(event);
                                }
                            }
                        }
                        Ok(Some(_)) => {}
                        Ok(None) | Err(_) => break,
                    }
                }
                command = commands.recv() => {
                    match command {
                        Some(Command::Call { id, line, waiter }) => {
                            pending.insert(id, waiter);
                            if write_half.write_all(line.as_bytes()).await.is_err()
                                || write_half.flush().await.is_err()
                            {
                                break;
                            }
                        }
                        Some(Command::Close) | None => break,
                    }
                }
            }
        }

        closed.store(true, Ordering::SeqCst);
        // Dropping pending senders wakes all synchronous callers as disconnected.
    }
}

#[cfg(windows)]
pub use windows_attached::AttachedDaemon;

/// The same surface on an otherwise unsupported platform.
///
/// A type rather than a `cfg` at every call site: the FFI reads one field and
/// branches once, and it should not have to know which platforms can attach.
#[cfg(all(not(unix), not(windows)))]
pub struct AttachedDaemon {
    _never: std::convert::Infallible,
}

#[cfg(all(not(unix), not(windows)))]
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

    pub fn close(&self) {
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

#[cfg(all(test, any(unix, windows)))]
mod tests {
    use super::*;
    use crate::daemon::start_embedded;
    #[cfg(windows)]
    use std::sync::Arc;
    use std::sync::mpsc;

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

    #[tokio::test(flavor = "multi_thread")]
    async fn callback_can_clear_its_sink_without_deadlocking() {
        let dir = tempfile::tempdir().unwrap();
        let store_a = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let embedded = start_embedded(store_a).await.unwrap();

        let store_b = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let cleared = tokio::task::spawn_blocking(move || {
            let attached = Arc::new(AttachedDaemon::connect(&store_b).unwrap());
            let weak = Arc::downgrade(&attached);
            let (tx, rx) = mpsc::channel();
            attached
                .subscribe(move |_| {
                    if let Some(attached) = weak.upgrade() {
                        attached.clear_sink();
                    }
                    let _ = tx.send(());
                })
                .unwrap();
            rx.recv_timeout(std::time::Duration::from_secs(3))
        })
        .await
        .unwrap();

        assert!(
            cleared.is_ok(),
            "event callback deadlocked while clearing its sink"
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
