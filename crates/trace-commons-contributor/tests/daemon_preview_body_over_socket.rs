//! The redacted preview body, over the socket, for a client that is not the
//! process hosting the daemon.
//!
//! Before `preview_body` existed, the body was reachable only through
//! `ipc::open_preview`, which takes `&DaemonShared`. Only the process holding
//! the daemon lock has one, so on the recommended Linux arrangement -- a
//! systemd-managed daemon with the window as a socket client -- the window
//! could not obtain a body at all, and its "Search" and "Exactly what would
//! be sent" surfaces were dead. Loading a second `DaemonShared` is not a
//! workaround: it rewrites the queue file and sweeps the pinned envelopes the
//! running daemon still needs.
//!
//! Every test here talks to the daemon the way that window does: a bare unix
//! socket, JSON per line, no access to the daemon's memory.

#![cfg(unix)]
// The daemon's IPC transport is a unix socket here and a named pipe on
// Windows, so this file's fixtures are unix-only. Without this gate the
// whole test target fails to COMPILE on Windows -- which is why the
// contributor crate's suite had never run there at all, not merely skipped.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use trace_commons_contributor::config::{ConfigStore, ContributorConfig};
use trace_commons_contributor::daemon::ipc::{
    DaemonShared, ERR_BAD_PARAMS, ERR_BODY_DIGEST_REQUIRED, ERR_UNAVAILABLE, ERR_UNKNOWN_ENTRY_ID,
    MAX_PREVIEW_BODY_CHUNK_BYTES, bind, open_preview, serve,
};
use trace_commons_contributor::daemon::queue::{Queue, QueueEntry, QueueState, entry_id_for};
use trace_commons_contributor::daemon::settings::DaemonSettings;
use trace_commons_contributor::identity::DeviceIdentity;
use trace_commons_contributor::source::TraceSource;
use trace_commons_contributor::source::claude_code::ClaudeCodeSource;

/// Planted in the fixture session. Redaction must remove it, so a body that
/// still carries it is not a redacted body.
const PLANTED_SECRET: &str = "sk-fake-fixture-secret-1234";
/// Also planted, inside the session's own working directory, so the "no
/// filesystem path" assertion has something real to catch.
const PLANTED_PATH: &str = "/Users/testuser/code/myproj/src/main.rs";

/// A running daemon plus a socket client that holds nothing but the path.
struct Harness {
    _dir: tempfile::TempDir,
    store_dir: std::path::PathBuf,
    /// The hosting process's own handle. Tests use it only to stand in for
    /// "the process holding the lock"; the socket client never sees it.
    shared: Arc<DaemonShared>,
    entry_id: uuid::Uuid,
}

impl Harness {
    /// `message` is the single user message the fixture session carries.
    async fn start(message: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("state");
        let store = ConfigStore::open(store_dir.clone()).unwrap();

        let sessions_root = dir.path().join("sessions/projects");
        let project = sessions_root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        let line = serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": message },
            "cwd": "/Users/testuser/code/myproj",
            "timestamp": "2026-08-08T10:00:00Z",
            "version": "2.0.1",
            "sessionId": "11111111-1111-1111-1111-111111111111",
            "uuid": "a1",
        });
        std::fs::write(
            project.join("11111111-1111-1111-1111-111111111111.jsonl"),
            format!("{line}\n"),
        )
        .unwrap();
        let src = ClaudeCodeSource::new(sessions_root.clone());
        let session_ref = TraceSource::discover(&src).unwrap().remove(0);

        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = ContributorConfig {
            schema_version: trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION
                .into(),
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
        settings.claude_source = Some(
            trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                path: sessions_root,
            },
        );
        settings.save(&store).unwrap();

        let entry_id = entry_id_for("preview-body-test-hash");
        let mut queue = Queue::new();
        queue
            .upsert(
                QueueEntry {
                    entry_id,
                    session_hash: "preview-body-test-hash".into(),
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
                    observed_modified_at: None,
                },
                100,
            )
            .unwrap();
        queue.save(&store).unwrap();

        let shared = Arc::new(DaemonShared::load(store).unwrap());
        let listener = {
            let store = ConfigStore::open(store_dir.clone()).unwrap();
            bind(&store).await.unwrap()
        };
        let serving = Arc::clone(&shared);
        tokio::spawn(async move {
            let _ = serve(listener, serving).await;
        });

        Self {
            _dir: dir,
            store_dir,
            shared,
            entry_id,
        }
    }

    async fn client(&self) -> Client {
        let stream = UnixStream::connect(self.store_dir.join("daemon.sock"))
            .await
            .unwrap();
        let (r, w) = stream.into_split();
        Client {
            reader: BufReader::new(r),
            writer: w,
            next_id: 1,
        }
    }
}

/// A socket client with no access to `DaemonShared` -- everything a GTK
/// window connected to a systemd-hosted daemon has.
struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
    next_id: u64,
}

impl Client {
    async fn call(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let id = self.next_id;
        self.next_id += 1;
        let line = serde_json::json!({ "id": id, "method": method, "params": params }).to_string();
        self.writer.write_all(line.as_bytes()).await.unwrap();
        self.writer.write_all(b"\n").await.unwrap();
        self.writer.flush().await.unwrap();
        let mut reply = String::new();
        self.reader.read_line(&mut reply).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(value["id"], id, "responses must echo the request id");
        value
    }

    /// Page the whole body the way the contract tells a client to, and
    /// return it reassembled together with the `total_bytes` the daemon
    /// reported. Asserts the frame never exceeds the line cap.
    async fn read_whole_body(&mut self, entry_id: uuid::Uuid) -> (String, usize) {
        let mut body = String::new();
        let mut offset: u64 = 0;
        let mut digest: Option<String> = None;
        let mut total;
        loop {
            let mut params = serde_json::json!({
                "entry_id": entry_id.to_string(),
                "offset": offset,
            });
            if let Some(d) = &digest {
                params["body_digest"] = serde_json::Value::String(d.clone());
            }
            let resp = self.call("preview_body", params).await;
            assert!(resp["error"].is_null(), "{resp}");
            let r = &resp["result"];
            total = r["total_bytes"].as_u64().unwrap() as usize;
            let chunk = r["chunk"].as_str().unwrap();
            assert!(
                chunk.len() <= MAX_PREVIEW_BODY_CHUNK_BYTES,
                "a chunk must respect the frame budget"
            );
            body.push_str(chunk);
            digest = Some(r["body_digest"].as_str().unwrap().to_string());
            match r["next_offset"].as_u64() {
                Some(next) => {
                    assert!(next > offset, "paging must make progress");
                    offset = next;
                }
                None => break,
            }
        }
        (body, total)
    }
}

#[tokio::test]
async fn a_socket_client_that_does_not_hold_the_lock_gets_the_redacted_body() {
    // The gap itself: this is the only path a systemd-hosted deployment's
    // window has, and it used to have no answer at all.
    let h = Harness::start(&format!("deploy with key {PLANTED_SECRET}")).await;
    let mut c = h.client().await;
    let (body, total) = c.read_whole_body(h.entry_id).await;

    assert!(!body.is_empty());
    assert_eq!(
        body.len(),
        total,
        "total_bytes must describe the whole body"
    );
    assert!(
        !body.contains(PLANTED_SECRET),
        "the body a client receives is the post-redaction one"
    );
    // It is a real transcript, not a stub: the redacted event is in there.
    assert!(body.contains("user_message"), "{body}");
}

#[tokio::test]
async fn the_socket_body_is_byte_identical_to_the_hosting_processs_body() {
    // Two surfaces onto one artifact. If they can disagree, an app that
    // searched one and displayed the other is showing a contributor a
    // transcript nobody redacted.
    let h = Harness::start(&format!("deploy with key {PLANTED_SECRET}")).await;
    let (_summary, hosted) = open_preview(&h.shared, h.entry_id).await.unwrap();

    let mut c = h.client().await;
    let (over_socket, _total) = c.read_whole_body(h.entry_id).await;

    assert_eq!(
        over_socket, hosted,
        "the socket body must be the same bytes the hosting process returns"
    );
}

#[tokio::test]
async fn an_unknown_entry_id_is_refused_with_a_fixed_label() {
    let h = Harness::start(&format!("deploy with key {PLANTED_SECRET}")).await;
    let mut c = h.client().await;
    let resp = c
        .call(
            "preview_body",
            serde_json::json!({ "entry_id": uuid::Uuid::new_v4().to_string() }),
        )
        .await;
    assert!(resp["result"].is_null());
    assert_eq!(resp["error"]["code"], ERR_BAD_PARAMS);
    assert_eq!(resp["error"]["message"], ERR_UNKNOWN_ENTRY_ID);
    // A fixed label, not a description of what was asked for.
    assert!(!resp.to_string().contains("/Users/"));
}

#[tokio::test]
async fn a_body_larger_than_one_frame_is_paged_and_reassembles_exactly() {
    // A redacted envelope may approach 1.5 MB while a socket line is capped
    // at 1 MiB. A client that silently received the first page and believed
    // it had the trace would report a confident, false "0 matches".
    let long = "lorem ipsum dolor sit amet ".repeat(16_000); // ~432 KB
    let h = Harness::start(&long).await;
    let (_summary, hosted) = open_preview(&h.shared, h.entry_id).await.unwrap();
    assert!(
        hosted.len() > MAX_PREVIEW_BODY_CHUNK_BYTES,
        "fixture must exceed one frame: {} bytes",
        hosted.len()
    );

    let mut c = h.client().await;
    // Count the pages explicitly rather than trusting the helper.
    let mut pages = 0;
    let mut body = String::new();
    let mut offset: u64 = 0;
    let mut digest: Option<String> = None;
    loop {
        let mut params = serde_json::json!({
            "entry_id": h.entry_id.to_string(),
            "offset": offset,
        });
        if let Some(d) = &digest {
            params["body_digest"] = serde_json::Value::String(d.clone());
        }
        let resp = c.call("preview_body", params).await;
        assert!(resp["error"].is_null(), "{resp}");
        let r = &resp["result"];
        body.push_str(r["chunk"].as_str().unwrap());
        digest = Some(r["body_digest"].as_str().unwrap().to_string());
        pages += 1;
        match r["next_offset"].as_u64() {
            Some(next) => offset = next,
            None => break,
        }
    }
    assert!(pages > 1, "a body this size must take more than one page");
    assert_eq!(body, hosted, "the pages must reassemble to the whole body");
}

#[tokio::test]
async fn a_continuation_page_without_an_anchor_is_refused() {
    // Fail-closed: an unanchored continuation is indistinguishable from a
    // page of a body that no longer exists, and splicing two of those is
    // exactly how a search reports a clean trace it never read.
    let h = Harness::start(&format!("deploy with key {PLANTED_SECRET}")).await;
    let mut c = h.client().await;
    let resp = c
        .call(
            "preview_body",
            serde_json::json!({ "entry_id": h.entry_id.to_string(), "offset": 8 }),
        )
        .await;
    assert_eq!(resp["error"]["code"], ERR_BAD_PARAMS);
    assert_eq!(resp["error"]["message"], ERR_BODY_DIGEST_REQUIRED);

    // And a wrong anchor is refused rather than answered from some other
    // body.
    let resp = c
        .call(
            "preview_body",
            serde_json::json!({
                "entry_id": h.entry_id.to_string(),
                "offset": 8,
                "body_digest": "sha256:0000",
            }),
        )
        .await;
    assert_eq!(resp["error"]["code"], ERR_UNAVAILABLE);
    assert_eq!(resp["error"]["message"], "preview-body-changed");
}

#[tokio::test]
async fn the_body_never_contains_a_filesystem_path() {
    // The socket's standing rule, which the preview exemption does not
    // relax: a path is not trace content a contributor consented to send,
    // and no client may be handed one to render or log.
    let h = Harness::start(&format!("see {PLANTED_PATH} and key {PLANTED_SECRET}")).await;
    let mut c = h.client().await;
    let (body, _total) = c.read_whole_body(h.entry_id).await;

    assert!(!body.contains(PLANTED_PATH), "{body}");
    assert!(!body.contains("/Users/testuser"), "{body}");
    assert!(!body.contains(".jsonl"), "{body}");
}
