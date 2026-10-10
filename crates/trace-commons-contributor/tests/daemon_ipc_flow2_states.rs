//! The Flow 2 states (#1118 K3), over a real unix socket: the scrub state
//! and second-look reasons on a queue entry, the unsure-span index over the
//! preview body, and the "Leaves this Mac" field list.
//!
//! The executable half of the `scrub` / `second_look`, `preview_unsure_spans`
//! and `leaves_this_mac` sections of `docs/contributor-daemon-ipc-v1_1.md`.

#![cfg(unix)]
// The daemon's IPC transport is a unix socket here and a named pipe on
// Windows; these fixtures are unix-only, exactly as `daemon_ipc_contract.rs`.

use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::ipc::{
    DaemonShared, ERR_BAD_PARAMS, ERR_BODY_DIGEST_REQUIRED, ERR_PREVIEW_BODY_CHANGED,
    ERR_UNAVAILABLE, ERR_UNKNOWN_ENTRY_ID, bind, serve,
};
use trace_commons_contributor::daemon::preview_scheduler;
use trace_commons_contributor::daemon::queue::{Queue, QueueEntry, entry_id_for};
use trace_commons_contributor::daemon::second_look::{
    REASON_LOOKS_UNSURE, REASON_NOTHING_MATCHED, REASON_TRIMMED_TO_FIT, SCRUB_NOT_YET_SCRUBBED,
    SCRUB_SCRUBBED,
};
use trace_commons_contributor::daemon::settings::DaemonSettings;
use trace_commons_contributor::daemon::unsure_spans::LABEL_LOOKS_LIKE_EMAIL;
use trace_commons_contributor::identity::DeviceIdentity;
use trace_commons_contributor::source::TraceSource;
use trace_commons_contributor::source::claude_code::ClaudeCodeSource;

struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(store_dir: &std::path::Path) -> Self {
        let stream = UnixStream::connect(store_dir.join("daemon.sock"))
            .await
            .unwrap();
        let (r, w) = stream.into_split();
        Client {
            reader: BufReader::new(r),
            writer: w,
        }
    }

    async fn call(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let line = serde_json::json!({ "id": 1, "method": method, "params": params });
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        self.writer.flush().await.unwrap();
        let mut reply = String::new();
        self.reader.read_line(&mut reply).await.unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("bad frame {reply:?}: {e}"))
    }

    /// The whole body through `preview_body`, following `next_offset`, with
    /// the digest the daemon reported for it.
    async fn whole_body(&mut self, entry_id: uuid::Uuid) -> (String, String) {
        let mut body = String::new();
        let mut digest = String::new();
        let mut offset = Some(0u64);
        while let Some(next) = offset {
            let mut params = serde_json::json!({ "entry_id": entry_id, "offset": next });
            if next > 0 {
                params["body_digest"] = serde_json::Value::from(digest.clone());
            }
            let r = self.call("preview_body", params).await;
            assert!(r["error"].is_null(), "{r}");
            body.push_str(r["result"]["chunk"].as_str().unwrap());
            digest = r["result"]["body_digest"].as_str().unwrap().to_string();
            offset = r["result"]["next_offset"].as_u64();
        }
        (body, digest)
    }

    async fn pending_entry(&mut self, entry_id: uuid::Uuid) -> serde_json::Value {
        let r = self.call("list_pending", serde_json::json!({})).await;
        r["result"]["pending"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["entry_id"] == entry_id.to_string())
            .cloned()
            .unwrap_or_else(|| panic!("entry not pending: {r}"))
    }
}

/// An enrolled daemon with one queued claude-code session whose single user
/// message is `content`, serving on a real socket.
async fn daemon_with_one_message(
    content: &str,
    subagents_dropped: u32,
) -> (tempfile::TempDir, std::path::PathBuf, uuid::Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let store_dir = dir.path().join("state");
    let store = ConfigStore::open(store_dir.clone()).unwrap();

    let session = "77777777-7777-7777-7777-777777777777";
    let sessions_root = dir.path().join("sessions/projects");
    let project = sessions_root.join("-Users-testuser-code-orchard-api");
    std::fs::create_dir_all(&project).unwrap();
    let user = serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": content},
        "cwd": "/Users/testuser/code/orchard-api",
        "timestamp": "2026-09-12T10:00:00Z",
        "version": "2.0.1",
        "sessionId": session,
        "uuid": "a1",
    });
    std::fs::write(
        project.join(format!("{session}.jsonl")),
        format!("{user}\n"),
    )
    .unwrap();
    let src = ClaudeCodeSource::new(sessions_root.clone());
    let session_ref = TraceSource::discover(&src).unwrap().remove(0);

    let device = DeviceIdentity::load_or_generate(&store).unwrap();
    let cfg = trace_commons_contributor::config::ContributorConfig {
        inference_receipt_endpoint: None,
        consent_scopes_chosen: Some(true),
        witness_origin: None,
        inference_receipt_check_attestation: false,
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

    let entry_id = entry_id_for("flow2-states-fixture-hash");
    let mut queue = Queue::new();
    queue
        .upsert(
            QueueEntry {
                entry_id,
                session_hash: "flow2-states-fixture-hash".into(),
                source: "claude-code".into(),
                project_key: "/Users/testuser/code/orchard-api".into(),
                project_label: "orchard-api".into(),
                path: session_ref.path.clone(),
                size_bytes: session_ref.size_bytes,
                discovered_at: chrono::Utc::now(),
                subagents_dropped,
                ..Default::default()
            },
            100,
        )
        .unwrap();
    queue.save(&store).unwrap();

    let shared = Arc::new(DaemonShared::load(store).unwrap());
    let runner: Arc<dyn preview_scheduler::PreviewJobRunner> = Arc::new(
        preview_scheduler::DaemonPreviewRunner::new(Arc::clone(&shared)),
    );
    preview_scheduler::spawn_workers(Arc::clone(&shared.previews), runner);
    let listener = bind(&ConfigStore::open(store_dir.clone()).unwrap())
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = serve(listener, shared).await;
    });
    (dir, store_dir, entry_id)
}

#[tokio::test]
async fn an_unpreviewed_entry_is_not_yet_scrubbed_and_never_zero_marks() {
    let (_dir, store_dir, entry_id) = daemon_with_one_message("please list the files", 0).await;
    let mut c = Client::connect(&store_dir).await;

    let before = c.pending_entry(entry_id).await;
    assert_eq!(before["scrub"], SCRUB_NOT_YET_SCRUBBED, "{before}");
    assert!(
        before.get("marks").is_none(),
        "no count before anything counted: {before}"
    );
    assert_eq!(
        before["second_look"],
        serde_json::json!([]),
        "an unscrubbed session is not 'nothing matched': {before}"
    );

    // A card describes its own build -- none matched -- but pins nothing,
    // so the ENTRY is still not scrubbed: no count describes the bytes an
    // approval would send.
    let r = c
        .call("preview", serde_json::json!({ "entry_id": entry_id }))
        .await;
    assert!(r["error"].is_null(), "{r}");
    assert_eq!(r["result"]["scrub"], SCRUB_SCRUBBED, "{r}");
    assert_eq!(r["result"]["marks"], 0, "{r}");
    assert_eq!(
        r["result"]["second_look"],
        serde_json::json!([REASON_NOTHING_MATCHED])
    );
    let after_card = c.pending_entry(entry_id).await;
    assert_eq!(after_card["scrub"], SCRUB_NOT_YET_SCRUBBED, "{after_card}");

    // Opening the body pins the envelope, and the count goes down with it.
    let _ = c.whole_body(entry_id).await;
    let after = c.pending_entry(entry_id).await;
    assert_eq!(after["scrub"], SCRUB_SCRUBBED, "{after}");
    assert_eq!(after["marks"], 0, "{after}");
    assert_eq!(
        after["second_look"],
        serde_json::json!([REASON_NOTHING_MATCHED]),
        "{after}"
    );
}

#[tokio::test]
async fn a_scrubbed_entry_reports_its_marks_and_is_not_flagged() {
    let (_dir, store_dir, entry_id) =
        daemon_with_one_message("please email fixture-user@example.com about it", 0).await;
    let mut c = Client::connect(&store_dir).await;
    let _ = c.whole_body(entry_id).await;
    let after = c.pending_entry(entry_id).await;
    assert_eq!(after["scrub"], SCRUB_SCRUBBED, "{after}");
    assert!(after["marks"].as_u64().unwrap() >= 1, "{after}");
    assert!(after["content_marks"].as_u64().unwrap() >= 1, "{after}");
    assert_eq!(after["second_look"], serde_json::json!([]), "{after}");
}

#[tokio::test]
async fn an_entry_whose_only_marks_are_paths_is_nothing_matched() {
    let (_dir, store_dir, entry_id) = daemon_with_one_message(
        "open /Users/testuser/code/orchard-api/src/main.rs please",
        0,
    )
    .await;
    let mut c = Client::connect(&store_dir).await;
    let _ = c.whole_body(entry_id).await;
    let after = c.pending_entry(entry_id).await;
    assert!(after["marks"].as_u64().unwrap() >= 1, "{after}");
    assert_eq!(after["content_marks"], 0, "{after}");
    assert_eq!(
        after["second_look"],
        serde_json::json!([REASON_NOTHING_MATCHED]),
        "path removals alone are not a match: {after}"
    );
}

#[tokio::test]
async fn a_trimmed_entry_is_trimmed_to_fit_before_and_after_a_preview() {
    let (_dir, store_dir, entry_id) =
        daemon_with_one_message("please email fixture-user@example.com about it", 2).await;
    let mut c = Client::connect(&store_dir).await;
    let before = c.pending_entry(entry_id).await;
    assert_eq!(before["scrub"], SCRUB_NOT_YET_SCRUBBED, "{before}");
    assert_eq!(
        before["second_look"],
        serde_json::json!([REASON_TRIMMED_TO_FIT]),
        "{before}"
    );
    let _ = c.whole_body(entry_id).await;
    let after = c.pending_entry(entry_id).await;
    assert_eq!(after["scrub"], SCRUB_SCRUBBED, "the preview ran: {after}");
    assert_eq!(
        after["second_look"],
        serde_json::json!([REASON_TRIMMED_TO_FIT]),
        "{after}"
    );
}

#[tokio::test]
async fn an_unmatched_email_is_hinted_at_exact_body_offsets_without_its_text() {
    // Planted in the session, before redaction. Bracket-obfuscated, so the
    // deterministic email pass (which needs an `@`) leaves it in the body:
    // a real survivor, not one added to a finished envelope.
    let survivor = "ops [at] acme [dot] io";
    let (_dir, store_dir, entry_id) =
        daemon_with_one_message(&format!("Use the staging key from {survivor} for it."), 0).await;
    let mut c = Client::connect(&store_dir).await;
    let (body, digest) = c.whole_body(entry_id).await;
    assert!(
        body.contains(survivor),
        "the fixture must survive scrubbing"
    );

    let r = c
        .call(
            "preview_unsure_spans",
            serde_json::json!({ "entry_id": entry_id, "body_digest": digest }),
        )
        .await;
    assert!(r["error"].is_null(), "{r}");
    let result = &r["result"];
    assert_eq!(result["body_digest"], digest.as_str());
    assert_eq!(result["spans_truncated"], false);
    let spans = result["spans"].as_array().unwrap();
    assert_eq!(result["span_count"].as_u64().unwrap() as usize, spans.len());
    let email = spans
        .iter()
        .find(|s| s["label"] == LABEL_LOOKS_LIKE_EMAIL)
        .unwrap_or_else(|| panic!("no email hint: {result}"));
    let offset = email["byte_offset"].as_u64().unwrap() as usize;
    let len = email["byte_len"].as_u64().unwrap() as usize;
    assert_eq!(&body[offset..offset + len], survivor);
    // Offsets and labels only. The shell holds the body; the hint never
    // repeats it.
    assert!(!r.to_string().contains("acme"), "{r}");

    // And the entry is held for it: an unsure span is a second-look reason.
    let entry = c.pending_entry(entry_id).await;
    assert_eq!(entry["unsure_spans"], 1, "{entry}");
    assert!(
        entry["second_look"]
            .as_array()
            .unwrap()
            .contains(&serde_json::Value::from(REASON_LOOKS_UNSURE)),
        "{entry}"
    );
}

#[tokio::test]
async fn a_matched_email_is_not_hinted() {
    let (_dir, store_dir, entry_id) =
        daemon_with_one_message("please email fixture-user@example.com about it", 0).await;
    let mut c = Client::connect(&store_dir).await;
    let (body, digest) = c.whole_body(entry_id).await;
    assert!(
        !body.contains("fixture-user@example.com"),
        "the fixture email must be redacted"
    );
    let r = c
        .call(
            "preview_unsure_spans",
            serde_json::json!({ "entry_id": entry_id, "body_digest": digest }),
        )
        .await;
    assert!(r["error"].is_null(), "{r}");
    assert_eq!(r["result"]["span_count"], 0, "{r}");
}

#[tokio::test]
async fn unsure_spans_refuse_an_unanchored_stale_or_unknown_request() {
    let (_dir, store_dir, entry_id) = daemon_with_one_message("please list the files", 0).await;
    let mut c = Client::connect(&store_dir).await;

    let r = c
        .call(
            "preview_unsure_spans",
            serde_json::json!({ "entry_id": entry_id }),
        )
        .await;
    assert_eq!(r["error"]["code"], ERR_BAD_PARAMS, "{r}");
    assert_eq!(r["error"]["message"], ERR_BODY_DIGEST_REQUIRED, "{r}");

    let r = c
        .call(
            "preview_unsure_spans",
            serde_json::json!({ "entry_id": entry_id, "body_digest": "sha256:0000" }),
        )
        .await;
    assert_eq!(r["error"]["code"], ERR_UNAVAILABLE, "{r}");
    assert_eq!(r["error"]["message"], ERR_PREVIEW_BODY_CHANGED, "{r}");

    let r = c
        .call(
            "preview_unsure_spans",
            serde_json::json!({ "entry_id": uuid::Uuid::new_v4(), "body_digest": "sha256:0000" }),
        )
        .await;
    assert_eq!(r["error"]["code"], ERR_BAD_PARAMS, "{r}");
    assert_eq!(r["error"]["message"], ERR_UNKNOWN_ENTRY_ID, "{r}");
}

#[tokio::test]
async fn preview_turns_says_what_leaves_this_mac_from_the_envelope() {
    use trace_commons_contributor::consent_copy::{
        LEAVES_FOLDER_IN_CONVERSATION, LEAVES_METADATA_NEVER,
    };
    let (_dir, store_dir, entry_id) = daemon_with_one_message("please list the files", 0).await;
    let mut c = Client::connect(&store_dir).await;
    let (_body, digest) = c.whole_body(entry_id).await;
    let r = c
        .call(
            "preview_turns",
            serde_json::json!({ "entry_id": entry_id, "body_digest": digest }),
        )
        .await;
    assert!(r["error"].is_null(), "{r}");
    let leaves = &r["result"]["leaves_this_mac"];
    let fields: Vec<&str> = leaves["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    for expected in [
        "conversation",
        "tool",
        "timing",
        "outcome",
        "uses",
        "tenant",
        "redaction-summary",
        "trace-ids",
        "folder-fingerprint",
    ] {
        assert!(fields.contains(&expected), "{expected} missing: {leaves}");
    }
    assert!(!fields.contains(&"other"), "{leaves}");
    assert_eq!(leaves["turn_count"], r["result"]["turn_count"]);
    assert!(leaves["would_send_bytes"].as_u64().unwrap() > 0);
    assert_eq!(leaves["folder_named_in_conversation"], false, "{leaves}");
    let line = leaves["line"].as_str().unwrap();
    assert!(line.ends_with(LEAVES_METADATA_NEVER), "{line}");
    assert!(!line.contains(LEAVES_FOLDER_IN_CONVERSATION), "{line}");
    // The folder's name is not among what leaves, so it is not claimed.
    assert!(!r.to_string().contains("orchard"), "{r}");
}

#[tokio::test]
async fn a_conversation_that_names_the_folder_is_not_promised_otherwise() {
    use trace_commons_contributor::consent_copy::{
        LEAVES_FOLDER_IN_CONVERSATION, LEAVES_METADATA_NEVER,
    };
    // Only absolute paths are scrubbed; a relative one keeps the folder name.
    let (_dir, store_dir, entry_id) =
        daemon_with_one_message("cd orchard-api && cat ../orchard-api/src/main.rs", 0).await;
    let mut c = Client::connect(&store_dir).await;
    let (body, digest) = c.whole_body(entry_id).await;
    assert!(
        body.contains("orchard-api"),
        "the fixture must name the folder"
    );
    let r = c
        .call(
            "preview_turns",
            serde_json::json!({ "entry_id": entry_id, "body_digest": digest }),
        )
        .await;
    assert!(r["error"].is_null(), "{r}");
    let leaves = &r["result"]["leaves_this_mac"];
    assert_eq!(leaves["folder_named_in_conversation"], true, "{leaves}");
    assert_eq!(leaves["folder_named_in_metadata"], false, "{leaves}");
    let line = leaves["line"].as_str().unwrap();
    assert!(line.contains(LEAVES_METADATA_NEVER), "{line}");
    assert!(line.ends_with(LEAVES_FOLDER_IN_CONVERSATION), "{line}");
}
