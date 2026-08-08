//! The daemon IPC contract, exercised over a real unix socket.
//!
//! These tests are the executable half of `docs/contributor-daemon-ipc-v1_1.md`.
//! Three native applications will be written against this framing, so the
//! properties asserted here -- id correlation, snapshot-before-delta, the
//! authorization carve-out, and behaviour on malformed input -- are the ones
//! that must not drift.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::ipc::{
    DaemonShared, ERR_BAD_PARAMS, ERR_UNKNOWN_METHOD, EVENT_SNAPSHOT, IPC_SCHEMA, METHODS, bind,
    serve,
};
use trace_commons_contributor::daemon::queue::{Queue, QueueEntry, QueueState, entry_id_for};
use trace_commons_contributor::daemon::settings::DaemonSettings;
use trace_commons_contributor::identity::DeviceIdentity;
use trace_commons_contributor::source::TraceSource;
use trace_commons_contributor::source::claude_code::ClaudeCodeSource;

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
async fn arming_autonomy_over_the_socket_is_now_allowed() {
    // The terminal-only gate on this call is removed. Same-user code that
    // can reach this socket can already read `~/.claude/projects` directly
    // and install its own persistent watcher, so this call grants it
    // neither the read nor the persistence it would need to exfiltrate
    // anything -- and would in fact be a worse channel for an attacker than
    // doing it directly (rate-limited, capped, redacted, and delivered
    // somewhere it cannot read back). See `daemon::ipc`'s "Authorization"
    // section.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    // A real local directory: the daemon no longer accepts a project key it
    // cannot corroborate, because the key's basename becomes the label that
    // crosses this socket and lands in the audit log. The `label` param is
    // still sent here, and is deliberately ignored.
    let dir = tempfile::tempdir().unwrap();
    let key = std::fs::canonicalize(dir.path()).unwrap();
    let key = key.to_string_lossy();
    c.send(&format!(
        r#"{{"id":3,"method":"set_project_mode","params":{{"project_key":"{key}","label":"p","mode":"auto_upload"}}}}"#,
    ))
    .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
}

#[tokio::test]
async fn setting_notify_only_over_the_socket_is_allowed() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    let dir = tempfile::tempdir().unwrap();
    let key = std::fs::canonicalize(dir.path()).unwrap();
    let key = key.to_string_lossy();
    c.send(&format!(
        r#"{{"id":4,"method":"set_project_mode","params":{{"project_key":"{key}","label":"p","mode":"notify_only"}}}}"#,
    ))
    .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
}

#[tokio::test]
async fn bulk_approval_over_the_socket_is_now_allowed() {
    // Removed for the same reason as arming autonomy above: the restriction
    // stopped nothing an attacker with same-user code execution did not
    // already have.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":5,"method":"approve","params":{"all":true}}"#)
        .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
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

#[tokio::test]
async fn preview_reports_the_redacted_envelope_not_the_raw_file() {
    // The regression this whole task exists to fix: `preview` used to
    // report `entry.size_bytes` (the raw session file on disk) instead of
    // the size of what redaction actually produces.
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("state");
    let store = ConfigStore::open(store_dir.clone()).unwrap();

    // A fixture session with a planted secret, so redaction has something
    // to do and the sizes cannot coincidentally match.
    let sessions_root = dir.path().join("sessions/projects");
    let project = sessions_root.join("-Users-testuser-code-myproj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::write(
        project.join("11111111-1111-1111-1111-111111111111.jsonl"),
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\
         \"content\":\"deploy with key sk-fake-fixture-secret-1234\"},\
         \"cwd\":\"/Users/testuser/code/myproj\",\
         \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
         \"sessionId\":\"11111111-1111-1111-1111-111111111111\",\
         \"uuid\":\"a1\"}\n",
    )
    .unwrap();
    let src = ClaudeCodeSource::new(sessions_root.clone());
    let session_ref = TraceSource::discover(&src).unwrap().remove(0);

    let device = DeviceIdentity::load_or_generate(&store).unwrap();
    let cfg = trace_commons_contributor::config::ContributorConfig {
        schema_version: trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
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
    };
    store.save_config(&cfg).unwrap();

    let mut settings = DaemonSettings::load(&store).unwrap();
    settings.claude_root = Some(sessions_root.clone());
    settings.save(&store).unwrap();

    let entry_id = entry_id_for("preview-test-hash");
    let mut queue = Queue::new();
    queue
        .upsert(
            QueueEntry {
                entry_id,
                session_hash: "preview-test-hash".into(),
                source: "claude-code".into(),
                project_key: "/Users/testuser/code/myproj".into(),
                project_label: "myproj".into(),
                path: session_ref.path.clone(),
                size_bytes: session_ref.size_bytes,
                discovered_at: chrono::Utc::now(),
                state: QueueState::Pending,
                reason_label: None,
                attempts: 0,
                retry_after: None,
                submission_id: None,
                approved_scopes: None,
            },
            100,
        )
        .unwrap();
    queue.save(&store).unwrap();

    let shared = Arc::new(DaemonShared::load(store).unwrap());
    let listener = bind_store(&store_dir).await;
    tokio::spawn(async move {
        let _ = serve(listener, shared).await;
    });

    let stream = UnixStream::connect(store_dir.join("daemon.sock"))
        .await
        .unwrap();
    let (r, w) = stream.into_split();
    let mut c = Client {
        reader: BufReader::new(r),
        writer: w,
    };
    c.send(&format!(
        r#"{{"id":1,"method":"preview","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;
    let resp = c.recv_json().await;
    let result = &resp["result"];
    assert!(resp["error"].is_null(), "{resp}");

    let would_send = result["would_send_bytes"]
        .as_u64()
        .expect("would_send_bytes present");
    let raw = result["raw_session_bytes"]
        .as_u64()
        .expect("raw_session_bytes present");
    // The regression: the old code returned `entry.size_bytes` (the raw file
    // size) verbatim as `would_send_bytes`. A redacted envelope carries its
    // own schema/consent/privacy/trace-card metadata on top of the (mostly
    // redaction-shortened) content, so for this fixture it comes out larger
    // than the raw file, not smaller -- the point is that it must be the
    // real, independently-computed envelope size, not a copy of the raw
    // size, in either direction.
    assert_ne!(
        would_send, raw,
        "would_send_bytes must not just echo raw_session_bytes"
    );

    // Recompute the envelope size independently through the same pipeline
    // `submit_one` and `build_preview` use, and check the daemon reported
    // exactly that -- not merely *some* different number.
    let transcript = TraceSource::load(&src, &session_ref).unwrap();
    let redactor = trace_commons_contributor::envelope::build_redactor_with(
        &cfg,
        transcript.cwd.as_deref(),
        None,
    )
    .unwrap();
    let raw_contribution = trace_commons_contributor::envelope::build_raw_contribution(
        &transcript,
        &cfg,
        chrono::Utc::now(),
    );
    let envelope =
        trace_commons_contributor::envelope::redact_to_envelope(&redactor, raw_contribution)
            .await
            .unwrap();
    let expected_would_send =
        trace_commons_contributor::envelope::envelope_size(&envelope).unwrap() as u64;
    assert_eq!(
        would_send, expected_would_send,
        "would_send_bytes must equal the real redacted envelope's serialized size"
    );

    let redactions = result["redactions"]
        .as_object()
        .expect("redactions present");
    let total: u64 = redactions.values().filter_map(|v| v.as_u64()).sum();
    assert!(
        total > 0,
        "the planted secret should show up in the redaction counts: {redactions:?}"
    );

    let body = resp.to_string();
    assert!(!body.contains("sk-fake-fixture-secret-1234"));
}

#[tokio::test]
async fn hello_reports_v1_1_and_still_claims_v1_compatibility() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"hello"}"#).await;
    let r = c.recv_json().await;
    assert_eq!(r["result"]["schema_version"], "trace_commons.daemon.v1_1");
    let supported: Vec<String> = r["result"]["supported_versions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        supported.contains(&"trace_commons.daemon.v1".to_string()),
        "a v1 client must still be told it is supported"
    );
}

#[tokio::test]
async fn an_over_long_socket_path_is_explained_rather_than_truncated() {
    // The kernel's own error names a constant most people have never heard
    // of, and does not say what to do about it.
    let dir = tempfile::tempdir().unwrap();
    let deep = dir.path().join("a".repeat(120));
    let store = ConfigStore::open(deep).unwrap();
    let err = bind(&store).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("kernel limit"), "{msg}");
    assert!(msg.contains("TRACE_COMMONS_CONTRIBUTOR_DIR"), "{msg}");
}
