//! The daemon IPC contract, exercised over a real unix socket.
//!
//! These tests are the executable half of `docs/contributor-daemon-ipc-v1_1.md`.
//! Three native applications will be written against this framing, so the
//! properties asserted here -- id correlation, snapshot-before-delta, the
//! authorization carve-out, and behaviour on malformed input -- are the ones
//! that must not drift.

#![cfg(unix)]
// The daemon's IPC transport is a unix socket here and a named pipe on
// Windows, so this file's fixtures are unix-only. Without this gate the
// whole test target fails to COMPILE on Windows -- which is why the
// contributor crate's suite had never run there at all, not merely skipped.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::ipc::{
    DaemonShared, ERR_BAD_PARAMS, ERR_UNKNOWN_METHOD, EVENT_PREVIEW_READY, EVENT_SNAPSHOT,
    IPC_SCHEMA, METHODS, bind, serve,
};
use trace_commons_contributor::daemon::policy::project_id_for;
use trace_commons_contributor::daemon::preview_scheduler::{
    self, MAX_PREVIEW_SESSION_BYTES, STATE_QUEUED, STATE_READY,
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
async fn a_client_can_set_a_mode_using_only_what_this_socket_gave_it() {
    // The gap a real SwiftUI client hit. Paths never cross this socket, so
    // a GUI holds `project_label` and nothing else -- and a label is not an
    // admissible `project_key`. `list_projects` and `list_pending` now also
    // carry `project_id`, an opaque daemon-issued handle, and
    // `set_project_mode` accepts it. Nothing in this test names a path
    // after the first (terminal-style) call, which is the point.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    let dir = tempfile::tempdir().unwrap();
    let key = std::fs::canonicalize(dir.path()).unwrap();
    let key = key.to_string_lossy();
    c.send(&format!(
        r#"{{"id":1,"method":"set_project_mode","params":{{"project_key":"{key}","mode":"notify_only"}}}}"#,
    ))
    .await;
    assert!(c.recv_json().await["error"].is_null());

    c.send(r#"{"id":2,"method":"list_projects"}"#).await;
    let listed = c.recv_json().await;
    let row = listed["result"]["projects"][0].clone();
    let project_id = row["project_id"]
        .as_str()
        .unwrap_or_else(|| panic!("list_projects must carry an id a client can name: {listed}"))
        .to_string();
    // `project_path` is the one path this row may carry, for display; the
    // point of the test is that a client can act on the row WITHOUT it, so
    // it asserts the id is what the rest of the exchange uses rather than
    // that no path exists. See `ipc::display_path` for the bound.
    assert_eq!(
        row["project_path"].as_str(),
        Some(key.as_ref()),
        "the row must render the project directory: {listed}"
    );
    assert!(
        !project_id.contains('/'),
        "the handle a client names must not be a path: {project_id}"
    );

    c.send(&format!(
        r#"{{"id":3,"method":"set_project_mode","params":{{"project_id":"{project_id}","mode":"ignore"}}}}"#,
    ))
    .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");

    c.send(r#"{"id":4,"method":"list_projects"}"#).await;
    let listed = c.recv_json().await;
    assert_eq!(
        listed["result"]["projects"][0]["mode"], "ignore",
        "{listed}"
    );
}

#[tokio::test]
async fn an_unrecognized_project_id_is_refused_with_a_fixed_label() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(
        r#"{"id":1,"method":"set_project_mode","params":{"project_id":"proj_0123456789abcdef","mode":"auto_upload"}}"#,
    )
    .await;
    let resp = c.recv_json().await;
    assert_eq!(resp["error"]["code"], ERR_BAD_PARAMS, "{resp}");
    assert_eq!(
        resp["error"]["message"], "project-id-unrecognized",
        "{resp}"
    );

    c.send(r#"{"id":2,"method":"list_projects"}"#).await;
    let listed = c.recv_json().await;
    assert_eq!(
        listed["result"]["projects"].as_array().unwrap().len(),
        0,
        "a refused call must record nothing: {listed}"
    );
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
async fn approve_reports_the_undo_window_the_document_promises() {
    // Three application teams build the undo countdown from the contract
    // document alone, so the fields it promises have to be on the response
    // shape itself -- including the "nothing to undo" case, where a client
    // must be able to tell `hold_until: null` from a missing key.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":6,"method":"approve","params":{"all":true}}"#)
        .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
    let result = &resp["result"];
    assert_eq!(result["approved"], 0, "{result}");
    assert_eq!(
        result["hold_secs"], 10,
        "the documented default hold: {result}"
    );
    assert!(
        result.get("hold_until").is_some() && result["hold_until"].is_null(),
        "an approval of nothing reports the key with a null deadline, so a \
         client offers no undo rather than inventing one: {result}"
    );
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
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: None,
    };
    store.save_config(&cfg).unwrap();

    let mut settings = DaemonSettings::load(&store).unwrap();
    settings.claude_source = Some(
        trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
            path: sessions_root.clone(),
        },
    );
    settings.save(&store).unwrap();

    let entry_id = entry_id_for("preview-test-hash");
    let mut queue = Queue::new();
    queue
        .upsert(
            QueueEntry {
                entry_id,
                session_hash: "preview-test-hash".into(),
                source: "claude-code".into(),
                declared_source: None,
                project_key: "/Users/testuser/code/myproj".into(),
                session_cwd: None,
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
                approved_verdict: None,
                approved_correction: None,
                approved_inputs: None,
                previewed_envelope_digest: None,
                approved_at: None,
                subagent_count: 0,
                subagents_dropped: 0,
                observed_modified_at: None,
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
    // Pinned, not `Utc::now()`. See the comment below the rebuild for why the
    // instant has to be one this test can name again afterwards.
    let rebuild_now = chrono::DateTime::parse_from_rfc3339("2026-09-01T00:00:00Z")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let raw_contribution =
        trace_commons_contributor::envelope::build_raw_contribution(&transcript, &cfg, rebuild_now);
    let envelope =
        trace_commons_contributor::envelope::redact_to_envelope(&redactor, raw_contribution)
            .await
            .unwrap();

    // The daemon stamped its own instant and does not report it, so the
    // rebuild above cannot use the same one. That matters for a size
    // comparison because chrono serializes `DateTime<Utc>` with as many
    // fractional-second digits as the value needs -- none, 3, 6, or 9 --
    // so one timestamp renders at four different lengths depending on which
    // nanosecond it was built in:
    //
    //   2026-09-01T00:00:00Z            20 bytes
    //   2026-09-01T00:00:00.123Z        24 bytes
    //   2026-09-01T00:00:00.123456Z     27 bytes
    //   2026-09-01T00:00:00.123456789Z  30 bytes
    //
    // Nothing else about the instant changes the length -- the date, the hour
    // and the `Z` are fixed width -- so comparing one draw against another was
    // a coin flip, and this test failed on the runs where the two instants
    // happened to land on different precisions, always with an off-by-3-or-10
    // byte count that read like a real accounting bug.
    //
    // Rather than loosen the assertion to a tolerance, enumerate the four
    // whole-envelope renderings the daemon's instant could have produced and
    // require the reported size to be exactly one of them. That still fails
    // if `would_send_bytes` goes back to echoing the raw file size, which is
    // the regression this test exists for and which differs by far more than
    // ten bytes.
    //
    // The instant is pinned above so it can be recognized again here: it
    // reaches `created_at` and also, via `raw_events_for`, the `timestamp` of
    // every event the transcript did not date itself. Those all move
    // together, exactly as they would have inside the daemon -- shifting only
    // `created_at` would model a rebuild the daemon never performs.
    let expected_sizes: Vec<u64> = [0, 123_000_000, 123_456_000, 123_456_789]
        .into_iter()
        .map(|nanos| {
            use chrono::Timelike as _;
            let shifted = rebuild_now.with_nanosecond(nanos).expect("nanos in range");
            let mut candidate = envelope.clone();
            if candidate.created_at == rebuild_now {
                candidate.created_at = shifted;
            }
            for event in &mut candidate.events {
                if event.timestamp == rebuild_now {
                    event.timestamp = shifted;
                }
            }
            trace_commons_contributor::envelope::envelope_size(&candidate).unwrap() as u64
        })
        .collect();
    assert!(
        expected_sizes.contains(&would_send),
        "would_send_bytes must equal the real redacted envelope's serialized size; \
         got {would_send}, expected one of {expected_sizes:?} (the same envelope \
         at each fractional-second precision chrono can emit)"
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

/// A config carrying whatever public profile the test needs, written into a
/// live daemon's state directory. The daemon reads the config on each
/// profile call, so this may be written after it starts.
fn write_config(store_dir: &std::path::Path, display_handle: Option<&str>) {
    let store = ConfigStore::open(store_dir.to_path_buf()).unwrap();
    let mut cfg = trace_commons_contributor::config::ContributorConfig {
        schema_version: trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
        issuer_url: "http://issuer.invalid".into(),
        ingest_url: "http://ingest.invalid".into(),
        audience: "trace-commons-upload".into(),
        tenant_id: "tenant-abc".into(),
        instance_id: "instance-1".into(),
        user_subject: "alice".into(),
        device_key_id: "device-1".into(),
        consent_scopes: vec!["debugging_evaluation".into()],
        pii_filter: None,
        allowed_hosts: None,
        display_handle: display_handle.map(str::to_string),
        public_bio: display_handle.map(|_| "Ships billing systems by day.".to_string()),
        public_since: display_handle.map(|_| chrono::Utc::now()),
        witness: None,
    };
    cfg.consent_scopes.push("public_attribution".into());
    store.save_config(&cfg).unwrap();
}

#[tokio::test]
async fn get_public_profile_reports_the_handle_this_device_published() {
    // The settings profile panel's whole data source. There is no
    // `GET /v1/community/profile`, so if the daemon does not report the
    // locally cached handle the panel renders empty for a contributor who
    // is on the roster.
    let h = TestDaemon::start().await;
    write_config(&h.store_dir, Some("manian"));
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"get_public_profile"}"#).await;
    let r = c.recv_json().await;
    assert!(r["error"].is_null(), "{r}");
    let result = &r["result"];
    assert_eq!(result["on_roster"], true, "{result}");
    assert_eq!(result["handle"], "manian", "{result}");
    assert!(!result["bio"].is_null(), "{result}");
    assert!(!result["public_since"].is_null(), "{result}");
    // No origin for a public profile crosses this socket, so the field is
    // present and null rather than a fabricated URL a client would link to.
    assert!(
        result.get("public_url").is_some() && result["public_url"].is_null(),
        "{result}"
    );
}

#[tokio::test]
async fn get_public_profile_reports_off_the_roster_before_a_handle_is_claimed() {
    let h = TestDaemon::start().await;
    write_config(&h.store_dir, None);
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"get_public_profile"}"#).await;
    let r = c.recv_json().await;
    assert_eq!(r["result"]["on_roster"], false, "{r}");
    assert!(r["result"]["handle"].is_null(), "{r}");
}

#[tokio::test]
async fn get_public_profile_without_an_enrollment_says_so() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"get_public_profile"}"#).await;
    let r = c.recv_json().await;
    assert_eq!(r["error"]["code"], "unavailable", "{r}");
    assert_eq!(r["error"]["message"], "not-logged-in", "{r}");
}

#[tokio::test]
async fn set_public_profile_refuses_an_omitted_bio_rather_than_erasing_one() {
    // The server upserts `bio = excluded.bio`, so the PUT replaces the whole
    // profile. A client that omits `bio` on a handle rename would silently
    // clear a published bio, which is why the daemon refuses instead of
    // guessing. This is checked before anything touches the network.
    let h = TestDaemon::start().await;
    write_config(&h.store_dir, Some("manian"));
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"set_public_profile","params":{"handle":"manian"}}"#)
        .await;
    let r = c.recv_json().await;
    assert_eq!(r["error"]["code"], ERR_BAD_PARAMS, "{r}");
    assert_eq!(r["error"]["message"], "bio-required-or-null", "{r}");
}

#[tokio::test]
async fn set_public_profile_applies_the_shared_handle_rules() {
    // The refusal comes from `trace_commons_protocol::community_handle`, the
    // same code the server validates with. A handle this daemon accepts and
    // the server then refuses is the drift these labels exist to prevent.
    let h = TestDaemon::start().await;
    write_config(&h.store_dir, None);
    let mut c = h.connect().await;
    for (params, label) in [
        (r#"{"handle":"ab","bio":null}"#, "handle-too-short"),
        (r#"{"handle":"admin","bio":null}"#, "handle-reserved"),
        (
            r#"{"handle":"foo--bar","bio":null}"#,
            "handle-consecutive-separators",
        ),
        (
            r#"{"handle":"foo bar","bio":null}"#,
            "handle-invalid-character",
        ),
        (r#"{"handle":"manian","bio":42}"#, "bio-invalid"),
    ] {
        c.send(&format!(
            r#"{{"id":1,"method":"set_public_profile","params":{params}}}"#
        ))
        .await;
        let r = c.recv_json().await;
        assert_eq!(r["error"]["code"], ERR_BAD_PARAMS, "{r}");
        assert_eq!(r["error"]["message"], label, "{r}");
    }
}

#[tokio::test]
async fn public_profile_calls_without_an_enrollment_never_reach_the_network() {
    // Fail closed with the label that tells a shell what is actually
    // missing, rather than attempting a call that cannot be authenticated.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"set_public_profile","params":{"handle":"manian","bio":null}}"#)
        .await;
    let r = c.recv_json().await;
    assert_eq!(r["error"]["message"], "not-logged-in", "{r}");

    c.send(r#"{"id":2,"method":"clear_public_profile"}"#).await;
    let r = c.recv_json().await;
    assert_eq!(r["error"]["message"], "not-logged-in", "{r}");
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

/// A daemon with one pending entry over a fixture session that carries a
/// user message, an assistant message, and a tool call -- enough events for
/// a turn index to be about something. Returns the harness pieces a socket
/// client needs and nothing the daemon holds in memory.
async fn daemon_with_a_multi_event_entry() -> (tempfile::TempDir, std::path::PathBuf, uuid::Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("state");
    let store = ConfigStore::open(store_dir.clone()).unwrap();

    let sessions_root = dir.path().join("sessions/projects");
    let project = sessions_root.join("-Users-testuser-code-myproj");
    std::fs::create_dir_all(&project).unwrap();
    let user = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": "please list the files"},
        "cwd": "/Users/testuser/code/myproj",
        "timestamp": "2026-08-08T10:00:00Z",
        "version": "2.0.1",
        "sessionId": "33333333-3333-3333-3333-333333333333",
        "uuid": "a1",
    });
    let assistant = serde_json::json!({
        "type": "assistant",
        "message": {"role": "assistant", "content": [
            {"type": "text", "text": "Reading the directory."},
            {"type": "tool_use", "name": "Read", "input": {"path": "src/main.rs"}},
        ]},
        "cwd": "/Users/testuser/code/myproj",
        "timestamp": "2026-08-08T10:00:01Z",
        "version": "2.0.1",
        "sessionId": "33333333-3333-3333-3333-333333333333",
        "uuid": "a2",
    });
    std::fs::write(
        project.join("33333333-3333-3333-3333-333333333333.jsonl"),
        format!("{user}\n{assistant}\n"),
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
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: None,
    };
    store.save_config(&cfg).unwrap();

    let mut settings = DaemonSettings::load(&store).unwrap();
    settings.claude_source = Some(
        trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
            path: sessions_root.clone(),
        },
    );
    settings.save(&store).unwrap();

    let entry_id = entry_id_for("turn-index-test-hash");
    let mut queue = Queue::new();
    queue
        .upsert(
            QueueEntry {
                entry_id,
                session_hash: "turn-index-test-hash".into(),
                source: "claude-code".into(),
                declared_source: None,
                project_key: "/Users/testuser/code/myproj".into(),
                session_cwd: None,
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
                approved_verdict: None,
                approved_correction: None,
                approved_inputs: None,
                previewed_envelope_digest: None,
                approved_at: None,
                // A single-file session: no delegated transcripts, nothing
                // dropped to fit the budget. This fixture is about the turn
                // index, not about grouping.
                subagent_count: 0,
                subagents_dropped: 0,
                observed_modified_at: None,
            },
            100,
        )
        .unwrap();
    queue.save(&store).unwrap();

    let shared = Arc::new(DaemonShared::load(store).unwrap());
    // The preview pool, exactly as `daemon::start_embedded` starts it.
    // Without it a `preview_request` here would queue and never complete,
    // and this fixture would be testing a daemon that does not exist.
    let runner: Arc<dyn preview_scheduler::PreviewJobRunner> = Arc::new(
        preview_scheduler::DaemonPreviewRunner::new(Arc::clone(&shared)),
    );
    preview_scheduler::spawn_workers(Arc::clone(&shared.previews), runner);
    let listener = bind_store(&store_dir).await;
    tokio::spawn(async move {
        let _ = serve(listener, shared).await;
    });
    (dir, store_dir, entry_id)
}

/// Single-entry daemon like `daemon_with_a_multi_event_entry`, except the
/// session content carries a private email address the deterministic
/// redactor's `private_email` regex catches unconditionally (no network, no
/// enrolled `near_ai` needed). This is the fixture for tests that need
/// `approve`'s `redactions` / `flagged` counts to be real, checkable values
/// rather than merely present -- an all-benign session (like the
/// multi-event fixture) always reports `redactions: {}`, `flagged: 0`,
/// which cannot tell an inert fold from a correct empty one.
async fn daemon_with_a_redactable_entry() -> (tempfile::TempDir, std::path::PathBuf, uuid::Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("state");
    let store = ConfigStore::open(store_dir.clone()).unwrap();

    let sessions_root = dir.path().join("sessions/projects");
    let project = sessions_root.join("-Users-testuser-code-myproj");
    std::fs::create_dir_all(&project).unwrap();
    let user = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": "please email fixture-user@example.com about the deploy"},
        "cwd": "/Users/testuser/code/myproj",
        "timestamp": "2026-08-08T10:00:00Z",
        "version": "2.0.1",
        "sessionId": "66666666-6666-6666-6666-666666666666",
        "uuid": "a1",
    });
    std::fs::write(
        project.join("66666666-6666-6666-6666-666666666666.jsonl"),
        format!("{user}\n"),
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
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: None,
    };
    store.save_config(&cfg).unwrap();

    let mut settings = DaemonSettings::load(&store).unwrap();
    settings.claude_source = Some(
        trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
            path: sessions_root.clone(),
        },
    );
    settings.save(&store).unwrap();

    let entry_id = entry_id_for("redactable-fixture-hash");
    let mut queue = Queue::new();
    queue
        .upsert(
            QueueEntry {
                entry_id,
                session_hash: "redactable-fixture-hash".into(),
                source: "claude-code".into(),
                declared_source: None,
                project_key: "/Users/testuser/code/myproj".into(),
                session_cwd: None,
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
                approved_verdict: None,
                approved_correction: None,
                approved_inputs: None,
                previewed_envelope_digest: None,
                approved_at: None,
                subagent_count: 0,
                subagents_dropped: 0,
                observed_modified_at: None,
            },
            100,
        )
        .unwrap();
    queue.save(&store).unwrap();

    let shared = Arc::new(DaemonShared::load(store).unwrap());
    // The preview pool, exactly as `daemon::start_embedded` starts it.
    // Without it a `preview_request` here would queue and never complete,
    // and this fixture would be testing a daemon that does not exist.
    let runner: Arc<dyn preview_scheduler::PreviewJobRunner> = Arc::new(
        preview_scheduler::DaemonPreviewRunner::new(Arc::clone(&shared)),
    );
    preview_scheduler::spawn_workers(Arc::clone(&shared.previews), runner);
    let listener = bind_store(&store_dir).await;
    tokio::spawn(async move {
        let _ = serve(listener, shared).await;
    });
    (dir, store_dir, entry_id)
}

async fn connect_to(store_dir: &std::path::Path) -> Client {
    let stream = UnixStream::connect(store_dir.join("daemon.sock"))
        .await
        .unwrap();
    let (r, w) = stream.into_split();
    Client {
        reader: BufReader::new(r),
        writer: w,
    }
}

/// Read the whole body through `preview_body`, following `next_offset` to
/// the end, and return it with the digest the daemon reported. This is the
/// flow a client is required to use, and the turn index is only meaningful
/// against what it produces.
async fn read_whole_body(c: &mut Client, entry_id: uuid::Uuid) -> (String, String) {
    let mut body = String::new();
    let mut offset = Some(0u64);
    let mut digest = String::new();
    let mut id = 1u64;
    while let Some(next) = offset {
        let anchor = if next == 0 {
            String::new()
        } else {
            format!(r#","body_digest":"{digest}""#)
        };
        c.send(&format!(
            r#"{{"id":{id},"method":"preview_body","params":{{"entry_id":"{entry_id}","offset":{next}{anchor}}}}}"#
        ))
        .await;
        let r = c.recv_json().await;
        assert!(r["error"].is_null(), "{r}");
        body.push_str(r["result"]["chunk"].as_str().unwrap());
        digest = r["result"]["body_digest"].as_str().unwrap().to_string();
        offset = r["result"]["next_offset"].as_u64();
        id += 1;
    }
    (body, digest)
}

#[tokio::test]
async fn preview_turns_indexes_the_body_preview_body_returns() {
    // The contract that makes the transcript surface possible without
    // re-rendering it: every offset is a boundary in the body the client is
    // already holding, so a separator drawn there lands between two events
    // rather than inside one.
    let (_dir, store_dir, entry_id) = daemon_with_a_multi_event_entry().await;
    let mut c = connect_to(&store_dir).await;
    let (body, body_digest) = read_whole_body(&mut c, entry_id).await;

    c.send(&format!(
        r#"{{"id":90,"method":"preview_turns","params":{{"entry_id":"{entry_id}","body_digest":"{body_digest}"}}}}"#
    ))
    .await;
    let r = c.recv_json().await;
    assert!(r["error"].is_null(), "{r}");
    let result = &r["result"];
    assert_eq!(result["body_digest"], body_digest.as_str(), "{result}");
    assert!(
        result["envelope_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256:")
    );

    let turns = result["turns"].as_array().unwrap();
    assert_eq!(result["turn_count"].as_u64().unwrap() as usize, turns.len());
    assert!(turns.len() >= 3, "the fixture has three events: {result}");
    let mut covered = 0usize;
    for (i, turn) in turns.iter().enumerate() {
        assert_eq!(turn["index"].as_u64().unwrap() as usize, i, "{turn}");
        let offset = turn["byte_offset"].as_u64().unwrap() as usize;
        let len = turn["byte_len"].as_u64().unwrap() as usize;
        assert!(offset >= covered, "turns must not overlap: {turn}");
        // Re-wrapped as an array because a turn may span more than one
        // element: parsing at all is the assertion that the span begins and
        // ends on element boundaries of the body the client is rendering.
        let slice = &body[offset..offset + len];
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&format!("[{slice}]"))
            .unwrap_or_else(|e| panic!("turn {i} is not a run of whole events: {e}"));
        assert_eq!(
            parsed[0]["event_type"], turn["role"],
            "the label must be the event type in the bytes it points at"
        );
        covered = offset + len;
    }
    assert!(covered <= body.len());
    // The index is labels and offsets. No redacted text rides along on it.
    assert!(!r.to_string().contains("please list the files"), "{r}");
}

#[tokio::test]
async fn preview_turns_refuses_an_unanchored_or_mis_anchored_request() {
    // Offsets against the wrong body are not stale, they are wrong, and
    // wrong invisibly: a separator drawn over the wrong text still looks
    // like a transcript. So the anchor is required from the first call, and
    // a digest that does not match is refused rather than indexed.
    let (_dir, store_dir, entry_id) = daemon_with_a_multi_event_entry().await;
    let mut c = connect_to(&store_dir).await;

    c.send(&format!(
        r#"{{"id":1,"method":"preview_turns","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;
    let r = c.recv_json().await;
    assert_eq!(r["error"]["code"], ERR_BAD_PARAMS, "{r}");
    assert_eq!(
        r["error"]["message"],
        trace_commons_contributor::daemon::ipc::ERR_BODY_DIGEST_REQUIRED,
        "{r}"
    );

    c.send(&format!(
        r#"{{"id":2,"method":"preview_turns","params":{{"entry_id":"{entry_id}","body_digest":"sha256:0000"}}}}"#
    ))
    .await;
    let r = c.recv_json().await;
    assert_eq!(
        r["error"]["code"],
        trace_commons_contributor::daemon::ipc::ERR_UNAVAILABLE,
        "{r}"
    );
    assert_eq!(
        r["error"]["message"],
        trace_commons_contributor::daemon::ipc::ERR_PREVIEW_BODY_CHANGED,
        "{r}"
    );

    // An entry the caller does not hold is refused under the same fixed
    // label the rest of the preview surface uses.
    let unknown = uuid::Uuid::new_v4();
    c.send(&format!(
        r#"{{"id":3,"method":"preview_turns","params":{{"entry_id":"{unknown}","body_digest":"sha256:0000"}}}}"#
    ))
    .await;
    let r = c.recv_json().await;
    assert_eq!(r["error"]["code"], ERR_BAD_PARAMS, "{r}");
    assert_eq!(
        r["error"]["message"],
        trace_commons_contributor::daemon::ipc::ERR_UNKNOWN_ENTRY_ID,
        "{r}"
    );
}

/// Approve one entry over the socket and return the daemon's result value.
async fn approve_one(c: &mut Client, entry_id: uuid::Uuid) -> serde_json::Value {
    c.send(&format!(
        r#"{{"id":1,"method":"approve","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
    resp["result"].clone()
}

#[tokio::test]
async fn approving_without_a_preview_still_pins_an_envelope() {
    // The uploader rebuilds and compares against the pin; an approval with
    // no pin fail-closes into a re-offer, so an unpinned approval is not a
    // submission at all. One-click submit approves things nobody opened, so
    // approve has to build the artifact preview used to.
    let (_dir, store_dir, entry_id) = daemon_with_a_multi_event_entry().await;
    let store = ConfigStore::open(store_dir.clone()).unwrap();
    assert!(
        Queue::load(&store)
            .unwrap()
            .get(entry_id)
            .expect("entry")
            .previewed_envelope_digest
            .is_none(),
        "fixture must start unpinned or this test proves nothing"
    );

    let mut c = connect_to(&store_dir).await;
    let result = approve_one(&mut c, entry_id).await;
    assert_eq!(result["approved"], 1, "{result}");

    let queue = Queue::load(&store).unwrap();
    let entry = queue.get(entry_id).expect("entry");
    assert_eq!(entry.state, QueueState::Approved, "{:?}", entry.state);
    assert!(
        entry.previewed_envelope_digest.is_some(),
        "approve must build and pin an envelope when no preview ran"
    );
}

#[tokio::test]
async fn an_unpreviewed_approval_pins_the_bytes_that_were_persisted() {
    // A digest with no envelope behind it is the same fail-closed re-offer
    // as no digest at all: the uploader loads the stored artifact and
    // checks it against the pin.
    let (_dir, store_dir, entry_id) = daemon_with_a_multi_event_entry().await;
    let store = ConfigStore::open(store_dir.clone()).unwrap();
    let mut c = connect_to(&store_dir).await;
    approve_one(&mut c, entry_id).await;

    let queue = Queue::load(&store).unwrap();
    let pinned = queue
        .get(entry_id)
        .expect("entry")
        .previewed_envelope_digest
        .clone()
        .expect("pinned");
    let saved = trace_commons_contributor::daemon::approved_envelope::load(&store, entry_id)
        .expect("load")
        .expect("an envelope must be on disk, not only a digest");
    assert_eq!(
        trace_commons_contributor::daemon::preview::envelope_digest(&saved).expect("digest"),
        pinned,
        "the pinned digest must name the bytes actually persisted"
    );
}

#[tokio::test]
async fn an_approval_whose_envelope_cannot_be_stored_is_not_reported_as_approved() {
    // The hole a build returning `Ok` hides. `pin_previewed_envelope`
    // declines silently when the envelope cannot be written, so the build
    // succeeds and the entry stays unpinned -- and an unpinned entry is not
    // refused at upload: `approved_envelope_for` returns `Ok(None)` and
    // `submit` builds a fresh envelope from that `None` and sends it. An
    // approval reported as successful here would mean bytes nobody was
    // shown going out. So the pin is re-checked before the entry is
    // approved, and an entry that could not be pinned stays `Pending`.
    //
    // The write is made to fail by planting a directory where the stored
    // envelope's file has to go: the atomic rename onto it cannot succeed,
    // and nothing else on the store is disturbed.
    let (_dir, store_dir, entry_id) = daemon_with_a_multi_event_entry().await;
    let store = ConfigStore::open(store_dir.clone()).unwrap();
    std::fs::create_dir_all(
        store.daemon_path(
            &trace_commons_contributor::daemon::approved_envelope::file_name(entry_id),
        ),
    )
    .unwrap();

    let mut c = connect_to(&store_dir).await;
    c.send(&format!(
        r#"{{"id":1,"method":"approve","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
    assert_eq!(
        resp["result"]["approved"], 0,
        "an entry with no artifact behind it must not be counted as approved: {resp}"
    );
    // Not approved is only half of it. The response promises that
    // `approved` plus `skipped` accounts for every id the call was asked to
    // act on, and this is the only path that reaches the pin re-check's own
    // skip. Without this assertion, deleting that `skipped.push` leaves the
    // entry counted nowhere and the whole contract test file still green.
    let skipped = resp["result"]["skipped"]
        .as_array()
        .expect("skipped must be an array");
    assert_eq!(
        skipped.len(),
        1,
        "the entry this call was asked to act on must show up somewhere in \
         the response, not vanish from both counts: {resp}"
    );
    assert_eq!(
        skipped[0]["entry_id"].as_str(),
        Some(entry_id.to_string()).as_deref(),
        "{resp}"
    );
    assert_eq!(
        skipped[0]["reason_label"].as_str(),
        Some("not-pinned"),
        "an entry still pending whose pin did not stick is the transient, \
         retryable case: {resp}"
    );

    let queue = Queue::load(&store).unwrap();
    let entry = queue.get(entry_id).expect("entry");
    assert!(
        entry.previewed_envelope_digest.is_none(),
        "the fixture must actually have prevented the pin, or this test \
         proves nothing"
    );
    assert_eq!(
        entry.state,
        QueueState::Pending,
        "an entry that could not be pinned stays pending rather than \
         becoming an approval the uploader would satisfy by building and \
         sending something nobody saw"
    );
}

/// An enrolled daemon with real session files in two projects: two pending
/// entries in `proj-a`, one in `proj-b`. Built for tests that approve a
/// whole project and must be able to tell "everything in that project" from
/// "everything else".
struct EnrolledDaemon {
    _dir: tempfile::TempDir,
    store_dir: std::path::PathBuf,
    client: tokio::sync::Mutex<Client>,
}

impl EnrolledDaemon {
    fn store(&self) -> ConfigStore {
        ConfigStore::open(self.store_dir.clone()).unwrap()
    }
}

/// Send one request over the daemon's socket and return the WHOLE response
/// frame, error and all, for a test that is asserting on a refusal.
async fn call_raw(
    daemon: &EnrolledDaemon,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let mut c = daemon.client.lock().await;
    c.send(&serde_json::json!({"id": 1, "method": method, "params": params}).to_string())
        .await;
    c.recv_json().await
}

/// Send one request over the daemon's socket and return its `result`.
/// Panics with the response on an error, so a test that only wants a happy
/// path does not have to check for one itself.
async fn call(
    daemon: &EnrolledDaemon,
    method: &str,
    params: serde_json::Value,
) -> serde_json::Value {
    let mut c = daemon.client.lock().await;
    c.send(&serde_json::json!({"id": 1, "method": method, "params": params}).to_string())
        .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{method} failed: {resp}");
    resp["result"].clone()
}

/// The queue as currently persisted to disk -- not a socket snapshot, so a
/// caller can assert on `QueueEntry` fields the wire format does not carry
/// (like `state` as an enum rather than a string).
async fn queue_entries(daemon: &EnrolledDaemon) -> Vec<QueueEntry> {
    Queue::load(&daemon.store()).unwrap().all().to_vec()
}

/// The project id of a pending entry in `proj-a`, the project this fixture
/// puts two entries in.
async fn first_pending_project_id(daemon: &EnrolledDaemon) -> String {
    let entries = queue_entries(daemon).await;
    let e = entries
        .iter()
        .find(|e| e.state == QueueState::Pending && e.project_label == "proj-a")
        .expect("fixture must seed a pending entry in proj-a");
    project_id_for(&e.project_key)
}

fn write_fixture_session(project_dir: &std::path::Path, session_id: &str, cwd: &str) {
    std::fs::create_dir_all(project_dir).unwrap();
    let user = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": "please list the files"},
        "cwd": cwd,
        "timestamp": "2026-08-08T10:00:00Z",
        "version": "2.0.1",
        "sessionId": session_id,
        "uuid": "a1",
    });
    std::fs::write(
        project_dir.join(format!("{session_id}.jsonl")),
        format!("{user}\n"),
    )
    .unwrap();
}

async fn enrolled_daemon_with_sessions_in_two_projects() -> (EnrolledDaemon, ConfigStore) {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("state");
    let store = ConfigStore::open(store_dir.clone()).unwrap();

    let sessions_root = dir.path().join("sessions/projects");
    let proj_a_dir = sessions_root.join("-Users-testuser-code-proj-a");
    let proj_b_dir = sessions_root.join("-Users-testuser-code-proj-b");
    write_fixture_session(
        &proj_a_dir,
        "11111111-1111-1111-1111-111111111111",
        "/Users/testuser/code/proj-a",
    );
    write_fixture_session(
        &proj_a_dir,
        "22222222-2222-2222-2222-222222222222",
        "/Users/testuser/code/proj-a",
    );
    write_fixture_session(
        &proj_b_dir,
        "33333333-3333-3333-3333-333333333333",
        "/Users/testuser/code/proj-b",
    );

    let src = ClaudeCodeSource::new(sessions_root.clone());
    let refs = TraceSource::discover(&src).unwrap();
    let ref_for = |needle: &str| {
        refs.iter()
            .find(|r| r.path.to_string_lossy().contains(needle))
            .unwrap_or_else(|| panic!("no discovered session for {needle}"))
            .clone()
    };
    let a1 = ref_for("11111111-1111-1111-1111-111111111111");
    let a2 = ref_for("22222222-2222-2222-2222-222222222222");
    let b1 = ref_for("33333333-3333-3333-3333-333333333333");

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
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: None,
    };
    store.save_config(&cfg).unwrap();

    let mut settings = DaemonSettings::load(&store).unwrap();
    settings.claude_source = Some(
        trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
            path: sessions_root.clone(),
        },
    );
    settings.save(&store).unwrap();

    let mut queue = Queue::new();
    let mut seed =
        |session_hash: &str,
         project_key: &str,
         project_label: &str,
         session_ref: &trace_commons_contributor::source::SessionRef| {
            queue
                .upsert(
                    QueueEntry {
                        entry_id: entry_id_for(session_hash),
                        session_hash: session_hash.into(),
                        source: "claude-code".into(),
                        declared_source: None,
                        project_key: project_key.into(),
                        session_cwd: None,
                        project_label: project_label.into(),
                        path: session_ref.path.clone(),
                        size_bytes: session_ref.size_bytes,
                        discovered_at: chrono::Utc::now(),
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
                    100,
                )
                .unwrap();
        };
    seed(
        "two-project-fixture-a1",
        "/Users/testuser/code/proj-a",
        "proj-a",
        &a1,
    );
    seed(
        "two-project-fixture-a2",
        "/Users/testuser/code/proj-a",
        "proj-a",
        &a2,
    );
    seed(
        "two-project-fixture-b1",
        "/Users/testuser/code/proj-b",
        "proj-b",
        &b1,
    );
    queue.save(&store).unwrap();

    let shared = Arc::new(DaemonShared::load(store).unwrap());
    let listener = bind_store(&store_dir).await;
    tokio::spawn(async move {
        let _ = serve(listener, shared).await;
    });
    let client = connect_to(&store_dir).await;

    let daemon = EnrolledDaemon {
        _dir: dir,
        store_dir: store_dir.clone(),
        client: tokio::sync::Mutex::new(client),
    };
    let store = ConfigStore::open(store_dir).unwrap();
    (daemon, store)
}

#[tokio::test]
async fn approving_a_project_takes_that_project_and_no_other() {
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let target = first_pending_project_id(&daemon).await;

    let v = call(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": target }),
    )
    .await;
    assert!(
        v["approved"].as_u64().unwrap_or(0) > 0,
        "nothing approved: {v}"
    );

    for e in queue_entries(&daemon).await {
        let want = project_id_for(&e.project_key) == target;
        assert_eq!(
            e.state == QueueState::Approved,
            want,
            "entry in {} should{} be approved",
            e.project_label,
            if want { "" } else { " not" }
        );
    }
}

#[tokio::test]
async fn approving_a_known_project_with_nothing_pending_is_not_an_error() {
    // A client can race a sweep, or click twice. Zero approved is an
    // outcome, not a fault -- and it must stay distinguishable from the
    // refusal an id naming no project at all now gets (below).
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let target = first_pending_project_id(&daemon).await;
    let first = call(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": target.clone() }),
    )
    .await;
    assert!(
        first["approved"].as_u64().unwrap_or(0) > 0,
        "the fixture must leave this project with nothing pending, or the \
         second call below proves nothing: {first}"
    );

    let again = call_raw(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": target }),
    )
    .await;
    assert!(
        again["error"].is_null(),
        "a project the daemon knows, with nothing left pending, is a \
         success reporting zero -- not a refusal: {again}"
    );
    assert_eq!(again["result"]["approved"].as_u64(), Some(0), "{again}");
    assert_eq!(
        again["result"]["skipped"].as_array().map(Vec::len),
        Some(0),
        "{again}"
    );
}

#[tokio::test]
async fn approving_a_project_id_the_daemon_does_not_know_is_refused() {
    // Consistency with `set_project_mode`, which refuses the same input on
    // the same socket with the same fixed label. A handle the caller never
    // received is a client bug, and answering it `approved: 0` is
    // indistinguishable from "that project had nothing to send" -- a shell
    // holding a typo'd or stale id would render a success toast forever.
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let v = call_raw(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": "proj_0000000000000000" }),
    )
    .await;
    assert_eq!(v["error"]["code"], ERR_BAD_PARAMS, "{v}");
    assert_eq!(v["error"]["message"], "project-id-unrecognized", "{v}");

    for e in queue_entries(&daemon).await {
        assert_eq!(
            e.state,
            QueueState::Pending,
            "a refused call must approve nothing"
        );
    }
}

#[tokio::test]
async fn approve_reports_counts_a_client_can_show_without_asking_again() {
    // The one-click flow never calls `preview`: the toast it renders --
    // "Sent -- scrubbing removed N things, M flagged. [Undo]" -- has to come
    // entirely off this response. Asserts actual values, not just shape: a
    // fold that never runs and an empty-but-present `redactions: {}` are
    // indistinguishable to `.is_object()` / `.is_u64()` checks alone, and a
    // fixture with nothing to redact (like the plain multi-event one) makes
    // that gap invisible. This fixture plants a private email the
    // deterministic redactor always catches, so the counts below are the
    // one real thing that fixture would produce.
    let (_dir, store_dir, entry_id) = daemon_with_a_redactable_entry().await;
    let mut c = connect_to(&store_dir).await;
    let v = approve_one(&mut c, entry_id).await;
    assert_eq!(v["approved"].as_u64(), Some(1), "{v}");
    assert_eq!(
        v["redactions"]["private_email"].as_u64(),
        Some(1),
        "the planted email must show up in the redaction counts: {v}"
    );
    assert_eq!(
        v["flagged"].as_u64(),
        Some(1),
        "a session with a PII label present must count as flagged: {v}"
    );
    assert!(v["skipped"].is_array(), "{v}");
    assert_eq!(v["skipped"].as_array().unwrap().len(), 0, "{v}");
}

/// An enrolled daemon with two pending entries in one project: one ordinary
/// session, and one whose sole event carries content past
/// `MAX_ENVELOPE_BYTES`. `build_preview` does not size-check the raw
/// contribution -- only the stored artifact does -- so the build itself
/// succeeds; `approve` catches the oversize before attempting to pin it and
/// reports the entry `skipped` (`envelope-too-large`) rather than letting it
/// vanish from both `approved` and `skipped`. Built for the "every entry
/// attempted is accounted for" guarantee.
async fn enrolled_daemon_with_one_good_and_one_oversized_session() -> (EnrolledDaemon, ConfigStore)
{
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("state");
    let store = ConfigStore::open(store_dir.clone()).unwrap();

    let sessions_root = dir.path().join("sessions/projects");
    let project_dir = sessions_root.join("-Users-testuser-code-proj-a");
    std::fs::create_dir_all(&project_dir).unwrap();

    write_fixture_session(
        &project_dir,
        "44444444-4444-4444-4444-444444444444",
        "/Users/testuser/code/proj-a",
    );

    // Past `MAX_ENVELOPE_BYTES` (16_000_000). Preview builds this fine --
    // the size guard `approve` checks (`summary.would_send_bytes`) mirrors
    // the one `approved_envelope::save` would otherwise apply, not
    // `build_preview` itself -- so this is what drives the entry into the
    // `envelope-too-large` skip rather than a build-time (`preview-failed`)
    // one.
    let oversized_content = "x".repeat(trace_commons_contributor::envelope::MAX_ENVELOPE_BYTES + 1);
    let oversized = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": oversized_content},
        "cwd": "/Users/testuser/code/proj-a",
        "timestamp": "2026-08-08T10:00:00Z",
        "version": "2.0.1",
        "sessionId": "55555555-5555-5555-5555-555555555555",
        "uuid": "a1",
    });
    std::fs::write(
        project_dir.join("55555555-5555-5555-5555-555555555555.jsonl"),
        format!("{oversized}\n"),
    )
    .unwrap();

    let src = ClaudeCodeSource::new(sessions_root.clone());
    let refs = TraceSource::discover(&src).unwrap();
    let ref_for = |needle: &str| {
        refs.iter()
            .find(|r| r.path.to_string_lossy().contains(needle))
            .unwrap_or_else(|| panic!("no discovered session for {needle}"))
            .clone()
    };
    let good = ref_for("44444444-4444-4444-4444-444444444444");
    let oversized_ref = ref_for("55555555-5555-5555-5555-555555555555");

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
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: None,
    };
    store.save_config(&cfg).unwrap();

    let mut settings = DaemonSettings::load(&store).unwrap();
    settings.claude_source = Some(
        trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
            path: sessions_root.clone(),
        },
    );
    settings.save(&store).unwrap();

    let mut queue = Queue::new();
    let mut seed =
        |session_hash: &str, session_ref: &trace_commons_contributor::source::SessionRef| {
            queue
                .upsert(
                    QueueEntry {
                        entry_id: entry_id_for(session_hash),
                        session_hash: session_hash.into(),
                        source: "claude-code".into(),
                        declared_source: None,
                        project_key: "/Users/testuser/code/proj-a".into(),
                        session_cwd: None,
                        project_label: "proj-a".into(),
                        path: session_ref.path.clone(),
                        size_bytes: session_ref.size_bytes,
                        discovered_at: chrono::Utc::now(),
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
                    100,
                )
                .unwrap();
        };
    seed("oversized-fixture-good", &good);
    seed("oversized-fixture-oversized", &oversized_ref);
    queue.save(&store).unwrap();

    let shared = Arc::new(DaemonShared::load(store).unwrap());
    let listener = bind_store(&store_dir).await;
    tokio::spawn(async move {
        let _ = serve(listener, shared).await;
    });
    let client = connect_to(&store_dir).await;

    let daemon = EnrolledDaemon {
        _dir: dir,
        store_dir: store_dir.clone(),
        client: tokio::sync::Mutex::new(client),
    };
    let store = ConfigStore::open(store_dir).unwrap();
    (daemon, store)
}

#[tokio::test]
async fn a_partial_batch_accounts_for_every_entry_it_was_given() {
    let (daemon, _store) = enrolled_daemon_with_one_good_and_one_oversized_session().await;
    let v = call(&daemon, "approve", serde_json::json!({ "all": true })).await;
    let approved = v["approved"].as_u64().expect("approved");
    let skipped = v["skipped"].as_array().expect("skipped");
    assert_eq!(
        approved + skipped.len() as u64,
        2,
        "every entry attempted must be accounted for: {v}"
    );
    assert_eq!(approved, 1, "{v}");
    assert_eq!(skipped.len(), 1, "{v}");
    for s in skipped {
        let label = s["reason_label"].as_str().expect("label");
        // The build itself succeeds -- preview does not size-check the raw
        // contribution, only the stored artifact does -- so the pin was
        // refused for size and `approve` repeats that same measurement to
        // recognise why, and gives it the permanent
        // `envelope-too-large` label rather than the generic, transient
        // `not-pinned` the pin re-check would otherwise apply. Either way
        // this is exactly the hole the pin re-check exists to close: an
        // entry that built but never got pinned must not vanish from the
        // response.
        assert_eq!(
            label, "envelope-too-large",
            "the oversized session's fixed label: {v}"
        );
        assert!(
            !label.contains('/'),
            "a reason label must not carry a path: {label}"
        );
        assert!(s["entry_id"].is_string(), "{v}");
    }
}

#[tokio::test]
async fn approving_an_already_approved_entry_is_reported_not_dropped() {
    // The deterministic repro for the hole `Queue::approve` returning
    // `false` opened one branch past the pin re-check: no timing games
    // needed, just approve the same entry twice. The pin survives the
    // first approval (`approve` never clears it), so the second call
    // passes the pin re-check and only then finds the entry is not
    // `Pending` any more.
    let (_dir, store_dir, entry_id) = daemon_with_a_multi_event_entry().await;
    let mut c = connect_to(&store_dir).await;

    let first = approve_one(&mut c, entry_id).await;
    assert_eq!(first["approved"].as_u64(), Some(1), "{first}");
    assert_eq!(first["skipped"].as_array().unwrap().len(), 0, "{first}");

    let second = approve_one(&mut c, entry_id).await;
    assert_eq!(
        second["approved"].as_u64(),
        Some(0),
        "an already-approved entry cannot be approved again: {second}"
    );
    let skipped = second["skipped"].as_array().expect("skipped");
    assert_eq!(
        skipped.len(),
        1,
        "the entry this call was asked to act on must show up somewhere \
         in the response: {second}"
    );
    assert_eq!(
        skipped[0]["entry_id"].as_str(),
        Some(entry_id.to_string()).as_deref(),
        "{second}"
    );
    assert_eq!(
        skipped[0]["reason_label"].as_str(),
        Some("not-pending"),
        "{second}"
    );
}

#[tokio::test]
async fn approve_with_an_id_the_caller_never_held_is_refused_like_preview() {
    // Consistency with `handle_preview`, which answers the same fixed
    // label for the same input: an id the caller never held is a client
    // bug, not something for `approve` to fold into a labelled skip of a
    // call that otherwise ran.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    let unknown = uuid::Uuid::new_v4();
    c.send(&format!(
        r#"{{"id":1,"method":"approve","params":{{"entry_id":"{unknown}"}}}}"#
    ))
    .await;
    let resp = c.recv_json().await;
    assert_eq!(resp["error"]["code"], ERR_BAD_PARAMS, "{resp}");
    assert_eq!(
        resp["error"]["message"],
        trace_commons_contributor::daemon::ipc::ERR_UNKNOWN_ENTRY_ID,
        "{resp}"
    );
}

#[tokio::test]
async fn an_unpinned_entry_that_is_no_longer_pending_is_labelled_not_pending() {
    // The two labels a client codes different retry logic against.
    // `not-pinned` is documented transient -- "retry is expected to work" --
    // and that is only true while the entry is still `pending`. An entry
    // that was never previewed and has since left `pending` (dismissed
    // here) can never be pinned by a retry: every pin path refuses a
    // non-`pending` entry, so a shell that trusts the table loops forever.
    // It gets `not-pending`, whose documented advice is to refresh queue
    // state instead.
    let (_dir, store_dir, entry_id) = daemon_with_a_multi_event_entry().await;
    let store = ConfigStore::open(store_dir.clone()).unwrap();
    let mut c = connect_to(&store_dir).await;

    c.send(&format!(
        r#"{{"id":1,"method":"dismiss","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
    let entry = Queue::load(&store).unwrap().get(entry_id).cloned();
    let entry = entry.expect("entry");
    assert_eq!(
        entry.state,
        QueueState::Refused,
        "the fixture must have moved the entry off pending"
    );
    assert!(
        entry.previewed_envelope_digest.is_none(),
        "the fixture must leave the entry unpinned, or this test exercises \
         the other branch"
    );

    let result = approve_one(&mut c, entry_id).await;
    assert_eq!(result["approved"].as_u64(), Some(0), "{result}");
    let skipped = result["skipped"].as_array().expect("skipped");
    assert_eq!(skipped.len(), 1, "{result}");
    assert_eq!(
        skipped[0]["reason_label"].as_str(),
        Some("not-pending"),
        "an unpinned entry that is no longer pending cannot be fixed by a \
         retry and must not be labelled transient: {result}"
    );
}

#[tokio::test]
async fn approving_a_whole_project_is_recorded_in_the_local_audit_log() {
    // The terminal-only restriction on bulk approval was removed and this
    // log is what replaced it (see `daemon::audit`). A tray click that
    // approves a whole project unattended is the same class of act as
    // approving the whole queue, so it leaves the same record. An empty
    // match writes nothing: nothing was approved, and a shell polling a
    // drained project would otherwise rotate real records out of a capped
    // log.
    use trace_commons_contributor::daemon::audit;

    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let target = first_pending_project_id(&daemon).await;
    assert!(
        audit::load(&daemon.store()).unwrap().is_empty(),
        "the fixture must start with an empty log"
    );

    let v = call(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": target.clone() }),
    )
    .await;
    assert_eq!(
        v["approved"].as_u64(),
        Some(2),
        "the fixture puts two pending entries in this project: {v}"
    );

    let entries = audit::load(&daemon.store()).unwrap();
    assert_eq!(
        entries.len(),
        1,
        "a project-wide approval must leave exactly one record"
    );
    assert_eq!(entries[0].action, "bulk-approved");
    assert_eq!(entries[0].detail.as_deref(), Some("2"));
    assert_eq!(
        entries[0].project_label.as_deref(),
        Some("proj-a"),
        "the label is derived from the key the daemon holds, never from \
         the caller's string"
    );

    // Nothing left pending in that project: a second call approves nothing
    // and records nothing.
    let again = call(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": target }),
    )
    .await;
    assert_eq!(again["approved"].as_u64(), Some(0), "{again}");
    assert_eq!(
        audit::load(&daemon.store()).unwrap().len(),
        1,
        "an approval that matched nothing must not append a record"
    );
}

#[tokio::test]
async fn undo_leaves_no_pin_behind_and_the_next_submit_rebuilds() {
    // The spec's Undo property, end to end over the socket: one-click
    // Submit pins an artifact nobody previewed, Undo withdraws it, and
    // nothing of that approval survives -- no state, and no pin.
    //
    // The pin is the half that matters here. Left behind, a second Submit
    // finds the entry already pinned, skips the rebuild, and approves the
    // artifact built at the FIRST click: stale bytes if the session grew
    // meanwhile, and `redactions: {}` / `flagged: 0` both times, so the
    // contributor is shown nothing either way. Cleared, the second Submit
    // rebuilds from the session as it now stands -- which is why this test
    // asserts the second approval's counts are real.
    let (_dir, store_dir, entry_id) = daemon_with_a_redactable_entry().await;
    let store = ConfigStore::open(store_dir.clone()).unwrap();
    let mut c = connect_to(&store_dir).await;

    let first = approve_one(&mut c, entry_id).await;
    assert_eq!(first["approved"].as_u64(), Some(1), "{first}");
    assert_eq!(
        first["redactions"]["private_email"].as_u64(),
        Some(1),
        "{first}"
    );
    let pinned = Queue::load(&store)
        .unwrap()
        .get(entry_id)
        .expect("entry")
        .previewed_envelope_digest
        .clone();
    assert!(
        pinned.is_some(),
        "the first Submit must pin, or the undo below proves nothing"
    );

    c.send(&format!(
        r#"{{"id":2,"method":"cancel","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;
    let undo = c.recv_json().await;
    assert!(undo["error"].is_null(), "{undo}");

    let entry = Queue::load(&store).unwrap().get(entry_id).cloned();
    let entry = entry.expect("entry");
    assert_eq!(entry.state, QueueState::Pending, "{:?}", entry.state);
    assert_eq!(
        entry.previewed_envelope_digest, None,
        "undo withdraws the approval, so the binding to the bytes it \
         covered cannot survive it"
    );

    // A second Submit is a fresh approval of a freshly built artifact, and
    // reports its counts like any other unpreviewed approval.
    let second = approve_one(&mut c, entry_id).await;
    assert_eq!(second["approved"].as_u64(), Some(1), "{second}");
    assert_eq!(
        second["redactions"]["private_email"].as_u64(),
        Some(1),
        "the second Submit must rebuild, not silently re-approve the first \
         click's artifact with nothing to report: {second}"
    );
    assert!(
        Queue::load(&store)
            .unwrap()
            .get(entry_id)
            .expect("entry")
            .previewed_envelope_digest
            .is_some(),
        "{second}"
    );
}

#[tokio::test]
async fn cancelling_a_project_undoes_that_projects_approved_entries_and_no_other() {
    // The gap this closes: with no `project_id` selector on `cancel`, a
    // shell offering Undo on a batch approval had to derive "the ids I saw
    // pending, minus the ones reported skipped" and issue one `cancel` per
    // id -- racy, and guesswork the daemon can remove outright. This is
    // that selector's basic contract, mirroring
    // `approving_a_project_takes_that_project_and_no_other`.
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let target = first_pending_project_id(&daemon).await;

    let approved = call(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": target.clone() }),
    )
    .await;
    assert_eq!(
        approved["approved"].as_u64(),
        Some(2),
        "the fixture puts two pending entries in proj-a: {approved}"
    );

    let canceled = call(
        &daemon,
        "cancel",
        serde_json::json!({ "project_id": target.clone() }),
    )
    .await;
    assert_eq!(canceled["canceled"].as_u64(), Some(2), "{canceled}");

    for e in queue_entries(&daemon).await {
        if project_id_for(&e.project_key) == target {
            assert_eq!(
                e.state,
                QueueState::Pending,
                "every entry cancel matched must be returned to pending"
            );
        } else {
            assert_eq!(
                e.state,
                QueueState::Pending,
                "the fixture's other project was never approved, so its \
                 own entry must simply remain untouched"
            );
        }
    }
}

#[tokio::test]
async fn cancelling_a_project_leaves_its_still_pending_entries_alone() {
    // There is nothing for Undo to undo about an entry that was never
    // approved. A project-wide `cancel` must select only `Approved`
    // entries -- selecting `Pending` ones too would be indistinguishable
    // from this test's fixture at the "how many got canceled" level, since
    // `Queue::cancel` itself refuses a non-`Approved` entry; what this
    // guards is that such entries are not even attempted, and are left
    // exactly as they were.
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let target = first_pending_project_id(&daemon).await;

    // Approve only ONE of the two pending entries in this project, so the
    // other stays `Pending` -- the case a selector that also picked up
    // `Pending` entries would get wrong.
    let one_id = queue_entries(&daemon)
        .await
        .into_iter()
        .find(|e| e.state == QueueState::Pending && project_id_for(&e.project_key) == target)
        .expect("fixture must have a pending entry in the target project")
        .entry_id;
    let approved = call(
        &daemon,
        "approve",
        serde_json::json!({ "entry_id": one_id.to_string() }),
    )
    .await;
    assert_eq!(approved["approved"].as_u64(), Some(1), "{approved}");

    let still_pending = queue_entries(&daemon)
        .await
        .into_iter()
        .find(|e| e.state == QueueState::Pending && project_id_for(&e.project_key) == target)
        .expect("the fixture's second entry in this project must still be pending")
        .entry_id;

    let canceled = call(
        &daemon,
        "cancel",
        serde_json::json!({ "project_id": target }),
    )
    .await;
    assert_eq!(
        canceled["canceled"].as_u64(),
        Some(1),
        "only the one approved entry should be undone: {canceled}"
    );

    let entries = queue_entries(&daemon).await;
    assert_eq!(
        entries.iter().find(|e| e.entry_id == one_id).unwrap().state,
        QueueState::Pending,
        "the approved entry must be undone"
    );
    assert_eq!(
        entries
            .iter()
            .find(|e| e.entry_id == still_pending)
            .unwrap()
            .state,
        QueueState::Pending,
        "the never-approved entry must be left exactly as it was, not \
         touched by a selector that should only ever see `approved` \
         entries"
    );

    // The audit row's count is taken at selection time (see `approve`'s own
    // audit comment), so a selector that over-picked the still-`pending`
    // entry would claim 2 canceled here even though `Queue::cancel` itself
    // refuses to act on it and only 1 entry actually moved. Checking this
    // catches that over-selection even though it would be invisible in the
    // response's own `canceled` count above.
    use trace_commons_contributor::daemon::audit;
    let entries = audit::load(&daemon.store()).unwrap();
    assert_eq!(
        entries.last().unwrap().detail.as_deref(),
        Some("1"),
        "the audit row must record exactly the one entry actually \
         eligible to be canceled, not every entry in the project"
    );
}

#[tokio::test]
async fn cancelling_an_unknown_project_id_is_refused() {
    // Consistency with `approve`'s own `project_id` selector, refused on
    // this same socket with this same fixed label: a handle the caller
    // never received is a client bug, and answering `canceled: 0` would be
    // indistinguishable from "that project had nothing to undo".
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let v = call_raw(
        &daemon,
        "cancel",
        serde_json::json!({ "project_id": "proj_0000000000000000" }),
    )
    .await;
    assert_eq!(v["error"]["code"], ERR_BAD_PARAMS, "{v}");
    assert_eq!(v["error"]["message"], "project-id-unrecognized", "{v}");
}

#[tokio::test]
async fn cancelling_a_known_project_with_nothing_approved_is_not_an_error() {
    // A known project with nothing to undo is a success reporting zero --
    // distinguishable from an id that names no project at all, above.
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let target = first_pending_project_id(&daemon).await;

    let v = call_raw(
        &daemon,
        "cancel",
        serde_json::json!({ "project_id": target }),
    )
    .await;
    assert!(
        v["error"].is_null(),
        "a project the daemon knows, with nothing approved, is a success: {v}"
    );
    assert_eq!(v["result"]["canceled"].as_u64(), Some(0), "{v}");
}

#[tokio::test]
async fn cancelling_a_project_clears_the_pin_on_every_entry_it_undoes() {
    // The property `cancel_clears_the_pin_so_the_next_approval_rebuilds`
    // guards for the single-`entry_id` form must hold for the batch form
    // too: an undone entry left pinned would make the next Submit approve
    // stale, already-built bytes and report empty counts either time.
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let target = first_pending_project_id(&daemon).await;

    let approved = call(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": target.clone() }),
    )
    .await;
    assert_eq!(approved["approved"].as_u64(), Some(2), "{approved}");
    for e in queue_entries(&daemon).await {
        if project_id_for(&e.project_key) == target {
            assert!(
                e.previewed_envelope_digest.is_some(),
                "the fixture must actually pin something, or this proves \
                 nothing"
            );
        }
    }

    let canceled = call(
        &daemon,
        "cancel",
        serde_json::json!({ "project_id": target.clone() }),
    )
    .await;
    assert_eq!(canceled["canceled"].as_u64(), Some(2), "{canceled}");

    for e in queue_entries(&daemon).await {
        if project_id_for(&e.project_key) == target {
            assert_eq!(
                e.previewed_envelope_digest, None,
                "an undone approval must leave no pin behind"
            );
        }
    }
}

#[tokio::test]
async fn cancelling_a_project_is_recorded_in_the_local_audit_log() {
    // Undoing a batch is the same class of act as approving one -- bulk,
    // unattended, previously terminal-only -- so it gets the same
    // visibility `approving_a_whole_project_is_recorded_in_the_local_audit_log`
    // documents for `approve`. An empty match writes nothing.
    use trace_commons_contributor::daemon::audit;

    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let target = first_pending_project_id(&daemon).await;

    let approved = call(
        &daemon,
        "approve",
        serde_json::json!({ "project_id": target.clone() }),
    )
    .await;
    assert_eq!(approved["approved"].as_u64(), Some(2), "{approved}");
    assert_eq!(
        audit::load(&daemon.store()).unwrap().len(),
        1,
        "the approval itself must have recorded one row"
    );

    let canceled = call(
        &daemon,
        "cancel",
        serde_json::json!({ "project_id": target.clone() }),
    )
    .await;
    assert_eq!(canceled["canceled"].as_u64(), Some(2), "{canceled}");

    let entries = audit::load(&daemon.store()).unwrap();
    assert_eq!(
        entries.len(),
        2,
        "a project-wide cancel must append exactly one more record"
    );
    assert_eq!(entries[1].action, "bulk-canceled");
    assert_eq!(entries[1].detail.as_deref(), Some("2"));
    assert_eq!(
        entries[1].project_label.as_deref(),
        Some("proj-a"),
        "the label is derived from the key the daemon holds, never from \
         the caller's string"
    );

    // Nothing left approved in that project: a second call cancels
    // nothing and records nothing.
    let again = call(
        &daemon,
        "cancel",
        serde_json::json!({ "project_id": target }),
    )
    .await;
    assert_eq!(again["canceled"].as_u64(), Some(0), "{again}");
    assert_eq!(
        audit::load(&daemon.store()).unwrap().len(),
        2,
        "a cancel that matched nothing must not append a record"
    );
}

/// The scheduled preview path end to end over the socket: the request
/// returns without building anything, the event carries the real summary,
/// and the second request is answered from cache.
#[tokio::test]
async fn preview_request_returns_promptly_and_the_event_carries_the_real_summary() {
    let (_dir, store_dir, entry_id) = daemon_with_a_redactable_entry().await;
    let mut c = connect_to(&store_dir).await;

    c.send(r#"{"id":1,"method":"subscribe"}"#).await;
    assert_eq!(c.recv_json().await["id"], 1);
    let snapshot = c.recv_json().await;
    assert_eq!(snapshot["event"], EVENT_SNAPSHOT);

    c.send(&format!(
        r#"{{"id":2,"method":"preview_request","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;

    // The response and the event arrive on the same connection, and which
    // lands first is a race the contract does not fix. Read frames until
    // both have been seen.
    let mut request_state: Option<String> = None;
    let mut ready: Option<serde_json::Value> = None;
    while request_state.is_none() || ready.is_none() {
        let frame = c.recv_json().await;
        if frame["id"] == 2 {
            assert!(frame["error"].is_null(), "{frame}");
            request_state = Some(frame["result"]["state"].as_str().unwrap().to_string());
        } else if frame["event"] == EVENT_PREVIEW_READY {
            ready = Some(frame);
        }
    }
    assert_eq!(
        request_state.unwrap(),
        STATE_QUEUED,
        "the first request must queue, never build on the connection's time"
    );

    let ready = ready.unwrap();
    assert!(ready["id"].is_null(), "push frames carry no id: {ready}");
    let data = &ready["data"];
    assert_eq!(data["entry_id"], entry_id.to_string());
    assert_eq!(data["state"], STATE_READY);
    let summary = &data["summary"];
    let would_send = summary["would_send_bytes"].as_u64().expect("a real size");
    assert!(would_send > 0, "the summary is the real build, not a stub");
    assert_eq!(summary["enrolled"], true);
    // No digest, and that is the contract rather than an omission. A card
    // build never produces one: the digest costs a second full
    // serialization plus a `serde_json::Value` tree of the whole redacted
    // envelope, and a card would discard both. Asserted as absent so a
    // future change cannot quietly put the cost back.
    assert!(
        summary["envelope_digest"].is_null(),
        "a card summary carries no digest: {summary}"
    );
    let fingerprint = summary["input_fingerprint"]
        .as_str()
        .expect("the configuration fingerprint still rides along");
    assert!(
        fingerprint.starts_with("sha256:"),
        "the fingerprint is a hash of the config, not of the envelope: {fingerprint}"
    );
    let redactions: u64 = summary["redactions"]
        .as_object()
        .expect("redaction counts")
        .values()
        .filter_map(|v| v.as_u64())
        .sum();
    assert!(redactions > 0, "the fixture plants an address to redact");
    assert!(
        !ready.to_string().contains("fixture-user@example.com"),
        "the event must never carry unredacted trace content"
    );

    // Second request: answered from the cache, with the same build.
    c.send(&format!(
        r#"{{"id":3,"method":"preview_request","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;
    let again = c.recv_json().await;
    assert_eq!(again["id"], 3);
    assert_eq!(again["result"]["state"], STATE_READY);
    // The whole summary, not one field: a cache hit replays the same build,
    // and comparing the object proves it for every field at once rather
    // than for whichever one this test happened to name.
    assert_eq!(
        &again["result"]["summary"], summary,
        "a cache hit replays the same build"
    );
    assert_eq!(again["result"]["summary"]["would_send_bytes"], would_send);

    // And no second `preview_ready` follows a cache hit: there was no job.
    c.send(r#"{"id":4,"method":"status"}"#).await;
    let next = c.recv_json().await;
    assert_eq!(
        next["id"], 4,
        "a cache hit publishes no event, so status answers next: {next}"
    );
}

/// `preview_visible` and `preview_cancel` are cheap, idempotent, and never
/// error on a no-op -- a shell calls them on every scroll.
#[tokio::test]
async fn preview_visible_and_cancel_are_safe_to_call_repeatedly() {
    let (_dir, store_dir, entry_id) = daemon_with_a_redactable_entry().await;
    let mut c = connect_to(&store_dir).await;

    c.send(&format!(
        r#"{{"id":1,"method":"preview_visible","params":{{"entry_ids":["{entry_id}"]}}}}"#
    ))
    .await;
    let resp = c.recv_json().await;
    assert!(resp["error"].is_null(), "{resp}");
    assert_eq!(resp["result"]["visible"], 1);

    c.send(r#"{"id":2,"method":"preview_visible","params":{"entry_ids":[]}}"#)
        .await;
    assert_eq!(c.recv_json().await["result"]["visible"], 0);

    c.send(r#"{"id":3,"method":"preview_visible","params":{"entry_ids":["not-a-uuid"]}}"#)
        .await;
    let bad = c.recv_json().await;
    assert_eq!(bad["error"]["code"], ERR_BAD_PARAMS);
    assert_eq!(bad["error"]["message"], "entry-ids-invalid");

    // Nothing scheduled: a cancel is a no-op, not an error.
    c.send(&format!(
        r#"{{"id":4,"method":"preview_cancel","params":{{"entry_id":"{entry_id}"}}}}"#
    ))
    .await;
    let cancelled = c.recv_json().await;
    assert!(cancelled["error"].is_null(), "{cancelled}");
    assert_eq!(cancelled["result"]["dropped"], false);
    assert_eq!(cancelled["result"]["entry_id"], entry_id.to_string());
}

/// The admission cap is the daemon's, not a test constant: a shell reading
/// `too_large` must be able to trust that no would-send number accompanies
/// it.
#[test]
fn the_admission_cap_admits_the_measured_claude_corpus_and_refuses_the_codex_tail() {
    // Asked of a real scheduler built the way the daemon builds it, so this
    // tracks the shipped policy rather than restating a constant.
    let sched = trace_commons_contributor::daemon::preview_scheduler::PreviewScheduler::default();
    assert_eq!(sched.admission_cap(), MAX_PREVIEW_SESSION_BYTES);
    // The corpus behind the incident: Claude sessions topped out at
    // 29.8 MB, Codex rollouts at 367.5 MB against a 3.5 MB mean.
    assert!(
        sched.admits(29_800_000),
        "the largest measured Claude session must still be previewable"
    );
    assert!(
        sched.admits(3_500_000),
        "the mean Codex rollout must be previewable"
    );
    assert!(
        !sched.admits(367_500_000),
        "the largest measured Codex rollout must not be parsed to draw a card"
    );
}
