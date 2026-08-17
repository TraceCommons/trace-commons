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
    let serialized = serde_json::to_string(&listed).unwrap();
    assert!(
        !serialized.contains(key.as_ref()),
        "a path crossed the socket: {serialized}"
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
                approved_inputs: None,
                previewed_envelope_digest: None,
                approved_at: None,
                subagent_count: 0,
                subagents_dropped: 0,
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
    };
    store.save_config(&cfg).unwrap();

    let mut settings = DaemonSettings::load(&store).unwrap();
    settings.claude_root = Some(sessions_root.clone());
    settings.save(&store).unwrap();

    let entry_id = entry_id_for("turn-index-test-hash");
    let mut queue = Queue::new();
    queue
        .upsert(
            QueueEntry {
                entry_id,
                session_hash: "turn-index-test-hash".into(),
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
                approved_inputs: None,
                previewed_envelope_digest: None,
                approved_at: None,
                // A single-file session: no delegated transcripts, nothing
                // dropped to fit the budget. This fixture is about the turn
                // index, not about grouping.
                subagent_count: 0,
                subagents_dropped: 0,
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
