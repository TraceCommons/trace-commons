//! The daemon IPC contract, exercised over a real unix socket.
//!
//! These tests are the executable half of `docs/contributor-daemon-ipc-v1.md`.
//! Three native applications will be written against this framing, so the
//! properties asserted here -- id correlation, snapshot-before-delta, the
//! authorization carve-out, and behaviour on malformed input -- are the ones
//! that must not drift.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::ipc::{
    DaemonShared, ERR_BAD_PARAMS, ERR_NOT_AUTHORIZED, ERR_UNKNOWN_METHOD, EVENT_SNAPSHOT,
    IPC_SCHEMA, METHODS, bind, serve,
};

struct TestDaemon {
    _dir: tempfile::TempDir,
    store_dir: std::path::PathBuf,
}

impl TestDaemon {
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("state");
        let store = ConfigStore::open(store_dir.clone()).unwrap();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        let listener = bind_store(&store_dir).await;
        tokio::spawn(async move {
            let _ = serve(listener, shared).await;
        });
        Self {
            _dir: dir,
            store_dir,
        }
    }

    fn socket_path(&self) -> std::path::PathBuf {
        self.store_dir.join("daemon.sock")
    }

    async fn connect(&self) -> Client {
        let stream = UnixStream::connect(self.socket_path()).await.unwrap();
        let (r, w) = stream.into_split();
        Client {
            reader: BufReader::new(r),
            writer: w,
        }
    }
}

async fn bind_store(dir: &std::path::Path) -> tokio::net::UnixListener {
    let store = ConfigStore::open(dir.to_path_buf()).unwrap();
    bind(&store).await.unwrap()
}

struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn send(&mut self, line: &str) {
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.writer.write_all(b"\n").await.unwrap();
        self.writer.flush().await.unwrap();
    }

    async fn recv_json(&mut self) -> serde_json::Value {
        let mut line = String::new();
        self.reader.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad frame {line:?}: {e}"))
    }

    /// Whether the peer closed the connection.
    async fn is_closed(&mut self) -> bool {
        let mut line = String::new();
        matches!(self.reader.read_line(&mut line).await, Ok(0))
    }
}

#[tokio::test]
async fn responses_echo_the_request_id() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":7,"method":"hello"}"#).await;
    let resp = c.recv_json().await;
    assert_eq!(resp["id"], 7);
    assert_eq!(resp["result"]["schema_version"], IPC_SCHEMA);
}

#[tokio::test]
async fn pipelined_requests_are_answered_with_their_own_ids() {
    // A client with two calls in flight must be able to tell the answers
    // apart. This is why every frame carries an id.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":11,"method":"status"}"#).await;
    let first = c.recv_json().await;
    c.send(r#"{"id":22,"method":"list_pending"}"#).await;
    let second = c.recv_json().await;
    assert_eq!(first["id"], 11);
    assert_eq!(second["id"], 22);
}

#[tokio::test]
async fn an_unknown_method_returns_the_taxonomy_code() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"no_such_method"}"#).await;
    let resp = c.recv_json().await;
    assert_eq!(resp["error"]["code"], ERR_UNKNOWN_METHOD);
}

#[tokio::test]
async fn subscribe_sends_a_full_snapshot_before_any_delta() {
    // Without this an application would have to race list_pending against
    // the event stream on every startup.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":2,"method":"subscribe"}"#).await;
    let ack = c.recv_json().await;
    assert_eq!(ack["id"], 2);
    let snapshot = c.recv_json().await;
    assert_eq!(snapshot["event"], EVENT_SNAPSHOT);
    assert!(snapshot["data"]["pending"].is_array());
    assert!(
        snapshot["id"].is_null(),
        "push frames must not carry an id: {snapshot}"
    );
}

#[tokio::test]
async fn arming_autonomy_over_the_socket_is_refused() {
    // Same-user code execution must not be able to turn the contributor's own
    // daemon into a continuous exfiltration channel.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(
        r#"{"id":3,"method":"set_project_mode","params":{"project_key":"/tmp/p","label":"p","mode":"auto_upload"}}"#,
    )
    .await;
    let resp = c.recv_json().await;
    assert_eq!(resp["error"]["code"], ERR_NOT_AUTHORIZED);
    assert_eq!(resp["error"]["message"], "tty-required");
}

#[tokio::test]
async fn setting_notify_only_over_the_socket_is_allowed() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(
        r#"{"id":4,"method":"set_project_mode","params":{"project_key":"/tmp/p","label":"p","mode":"notify_only"}}"#,
    )
    .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
}

#[tokio::test]
async fn bulk_approval_over_the_socket_is_refused() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":5,"method":"approve","params":{"all":true}}"#)
        .await;
    let resp = c.recv_json().await;
    assert_eq!(resp["error"]["code"], ERR_NOT_AUTHORIZED);
}

#[tokio::test]
async fn a_malformed_line_is_rejected_and_closes_the_connection() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send("this is not json").await;
    let resp = c.recv_json().await;
    assert_eq!(resp["error"]["code"], ERR_BAD_PARAMS);
    assert!(
        c.is_closed().await,
        "connection should close after a bad frame"
    );
}

#[tokio::test]
async fn an_oversize_line_is_rejected_rather_than_buffered() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    let huge = "x".repeat(2 * 1024 * 1024);
    c.send(&format!(r#"{{"id":6,"method":"hello","params":"{huge}"}}"#))
        .await;
    let resp = c.recv_json().await;
    assert_eq!(resp["error"]["code"], ERR_BAD_PARAMS);
}

#[tokio::test]
async fn status_exposes_every_state_a_tray_needs() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":8,"method":"status"}"#).await;
    let r = c.recv_json().await;
    for key in ["logged_in", "paused", "queue_depth", "health"] {
        assert!(!r["result"][key].is_null(), "status missing {key}");
    }
    // A daemon with no enrollment must say so rather than looking healthy.
    assert_eq!(r["result"]["logged_in"], false);
}

#[tokio::test]
async fn hello_advertises_exactly_the_documented_method_set() {
    // The contract document and this list are the same contract. Drift
    // between them is exactly what this catches.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"hello"}"#).await;
    let r = c.recv_json().await;
    let mut methods: Vec<String> = r["result"]["methods"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap().to_string())
        .collect();
    methods.sort();
    let mut expected: Vec<String> = METHODS.iter().map(|m| m.to_string()).collect();
    expected.sort();
    assert_eq!(methods, expected);
}

#[tokio::test]
async fn the_daemon_refuses_to_bind_in_a_world_readable_directory() {
    // UnixListener::bind does not set a socket mode, so the directory is the
    // only access control the socket has.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let store = ConfigStore::open(state.clone()).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = bind(&store).await.unwrap_err();
        assert!(
            err.to_string().contains("0700"),
            "expected a permissions refusal, got: {err}"
        );
    }
}

#[tokio::test]
async fn two_clients_are_served_independently() {
    let h = TestDaemon::start().await;
    let mut a = h.connect().await;
    let mut b = h.connect().await;
    a.send(r#"{"id":100,"method":"status"}"#).await;
    b.send(r#"{"id":200,"method":"status"}"#).await;
    assert_eq!(a.recv_json().await["id"], 100);
    assert_eq!(b.recv_json().await["id"], 200);
}
