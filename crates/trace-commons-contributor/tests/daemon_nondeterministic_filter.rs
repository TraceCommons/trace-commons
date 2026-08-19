//! The consent path must complete when the redaction step does not
//! reproduce itself.
//!
//! This is the configuration the pilot actually runs: `pii_filter =
//! "near-ai"`, an LLM-backed privacy filter. An LLM does not return
//! identical spans for identical text, so redacting the same session twice
//! yields two different envelopes.
//!
//! The design this replaced pinned a digest of the previewed envelope and
//! re-derived it immediately before the upload. Against a deterministic
//! local redactor that is invisible; against this one it made previewed
//! entries **permanently unuploadable** -- preview pins D1, the upload
//! rebuilds and gets D2, the entry is refused `envelope-changed-after
//! -approval` and re-offered with the pin cleared, the next preview pins D3
//! which will not reproduce either. It failed closed, so nothing unsafe
//! shipped, and the primary consent path never completed. Every existing
//! test used the deterministic redactor, which is why nothing caught it.
//!
//! So the daemon stores the envelope a preview built and uploads exactly
//! those bytes. The tests here drive that against a classifier that really
//! does move between calls.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::{Json, Router, routing::post};
use chrono::Utc;

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};
use trace_commons_contributor::daemon::ipc::{self, DaemonShared};
use trace_commons_contributor::daemon::queue::QueueState;
use trace_commons_contributor::envelope::NearAiSettings;
use trace_commons_contributor::identity::DeviceIdentity;

async fn spawn(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

fn stub_issuer() -> Router {
    Router::new().route(
        "/v1/trace-upload-claim",
        post(|| async {
            Json(serde_json::json!({
                "access_token": "stub-claim-jwt",
                "token_type": "Bearer",
                "expires_at": Utc::now() + chrono::Duration::seconds(300),
                "expires_in": 300,
                "consent_scopes": ["debugging_evaluation"],
                "allowed_uses": ["debugging", "evaluation"],
            }))
        }),
    )
}

fn stub_ingest(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
    Router::new().route(
        "/v1/traces",
        post(move |Json(body): Json<serde_json::Value>| {
            let received = received.clone();
            async move {
                received.lock().unwrap().push(body);
                Json(serde_json::json!({
                    "status": "accepted",
                    "credit_points_pending": 1.0,
                    "explanation": []
                }))
            }
        }),
    )
}

/// The synthetic values `run_privacy_filter_canary` plants and then insists
/// are gone. A filter that leaves any of them in fails the batch canary, so
/// the stub has to actually remove them however else it behaves.
const CANARY_VALUES: &[&str] = &[
    "trace-canary.person@example.invalid",
    "tc_canary_secret_0123456789abcdef",
    "/tmp/trace_canary_private/path.txt",
];

/// A privacy classifier that removes the canary values correctly and is
/// otherwise **deliberately non-deterministic**: how much of an ordinary
/// field it redacts depends on `generation`, which the test bumps. Two
/// redactions of byte-identical text under byte-identical configuration
/// therefore produce different output, which is what an LLM-backed filter
/// does in production and what no test in this crate could previously
/// express.
///
/// Spans are codepoint offsets, per the NEAR AI contract.
fn jittery_privacy_filter(generation: Arc<AtomicUsize>) -> Router {
    Router::new().route(
        "/privacy/classify",
        post(move |Json(body): Json<serde_json::Value>| {
            let generation = generation.clone();
            async move {
                let input = body
                    .get("input")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let chars: Vec<char> = input.chars().collect();
                let mut spans = Vec::new();
                for value in CANARY_VALUES {
                    if let Some(byte_start) = input.find(value) {
                        let start = input[..byte_start].chars().count();
                        spans.push(serde_json::json!({
                            "category": "private_name",
                            "start": start,
                            "end": start + value.chars().count(),
                            "score": 0.99,
                        }));
                    }
                }
                if spans.is_empty() && chars.len() > 4 {
                    // The jitter: the same field, classified twice, comes
                    // back with a differently sized span.
                    let end = 1 + generation.load(Ordering::SeqCst) % 3;
                    spans.push(serde_json::json!({
                        "category": "private_name",
                        "start": 0,
                        "end": end.min(chars.len()),
                        "score": 0.9,
                    }));
                }
                Json(serde_json::json!({ "data": [{ "spans": spans }] }))
            }
        }),
    )
}

struct Harness {
    _dir: tempfile::TempDir,
    shared: Arc<DaemonShared>,
    claude_root: std::path::PathBuf,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    generation: Arc<AtomicUsize>,
}

impl Harness {
    async fn new() -> Self {
        let received = Arc::new(Mutex::new(Vec::new()));
        let generation = Arc::new(AtomicUsize::new(0));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let filter = spawn(jittery_privacy_filter(generation.clone())).await;

        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("state")).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        store
            .save_config(&ContributorConfig {
                schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
                issuer_url: issuer,
                ingest_url: ingest,
                audience: "trace-commons-upload".into(),
                tenant_id: "tenant-abc".into(),
                instance_id: "instance-1".into(),
                user_subject: "alice".into(),
                device_key_id: device.device_key_id,
                consent_scopes: vec!["debugging_evaluation".into()],
                pii_filter: Some("near-ai".into()),
                allowed_hosts: Some("127.0.0.1".into()),
                display_handle: None,
                public_bio: None,
                public_since: None,
            })
            .unwrap();

        let claude_root = dir.path().join("projects");
        std::fs::create_dir_all(&claude_root).unwrap();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        {
            let mut s = shared.settings.lock().unwrap();
            s.claude_source = Some(
                trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                    path: claude_root.clone(),
                },
            );
            s.codex_source = Some(
                trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                    path: dir.path().join("codex"),
                },
            );
            s.near_ai = Some(NearAiSettings {
                api_key: "test-key".into(),
                base_url: Some(filter),
                model: None,
            });
        }
        // The first-use disclosure is a separate gate; acknowledge it so
        // these tests are about the filter's behaviour, not the notice.
        shared.store.ensure_near_ai_notice_shown().unwrap();
        Self {
            _dir: dir,
            shared,
            claude_root,
            received,
            generation,
        }
    }

    fn write_session(&self, project: &str, id: &str) {
        let project_dir = self
            .claude_root
            .join(format!("-Users-testuser-code-{project}"));
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(
            project_dir.join(format!("{id}.jsonl")),
            format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"fix the parser please\"}},\
                 \"cwd\":\"/Users/testuser/code/{project}\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"{id}\",\"uuid\":\"a1\"}}\n"
            ),
        )
        .unwrap();
    }

    fn now() -> chrono::DateTime<Utc> {
        "2030-01-01T00:00:00Z".parse().unwrap()
    }

    /// Two watch passes: eligibility needs a stable size.
    async fn discover(&self) {
        for _ in 0..2 {
            trace_commons_contributor::daemon::watcher::tick(&self.shared, Self::now())
                .await
                .unwrap();
        }
    }

    async fn upload_pass(&self) -> anyhow::Result<()> {
        trace_commons_contributor::daemon::drain_approved_for_test(&self.shared, Self::now()).await
    }

    fn only_entry(&self) -> trace_commons_contributor::daemon::queue::QueueEntry {
        self.shared.queue.lock().unwrap().all()[0].clone()
    }

    /// Move the filter on, so the next redaction of the same text differs
    /// from the last one.
    fn advance_filter(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn a_previewed_entry_uploads_even_though_the_filter_never_repeats_itself() {
    let h = Harness::new().await;
    h.write_session("myproj", "7a7a7a7a-7a7a-7a7a-7a7a-7a7a7a7a7a7a");
    h.discover().await;
    let entry_id = h.only_entry().entry_id;

    // Preview once, move the filter, preview again. Same session bytes,
    // same config, different artifact: this is the premise the digest
    // design could not survive, asserted rather than assumed.
    let (first, first_body) = ipc::open_preview(&h.shared, entry_id).await.unwrap();
    h.advance_filter();
    let (second, second_body) = ipc::open_preview(&h.shared, entry_id).await.unwrap();
    assert_ne!(
        first_body, second_body,
        "the stub filter must actually be non-deterministic for this test \
         to mean anything"
    );
    assert_ne!(first.envelope_digest, second.envelope_digest);

    // Move it again, so a rebuild at upload time would match neither
    // preview. Under the digest design this is the refusal; here it is
    // irrelevant, because nothing rebuilds.
    h.advance_filter();

    let resp = ipc::handle_local(
        &h.shared,
        "approve",
        serde_json::json!({ "entry_id": entry_id.to_string() }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);

    h.upload_pass().await.unwrap();

    let received = h.received.lock().unwrap().clone();
    assert_eq!(
        received.len(),
        1,
        "the primary consent path -- preview, approve, upload -- must \
         complete under a non-deterministic privacy filter; entry is {:?}",
        h.only_entry()
    );
    assert_eq!(h.only_entry().state, QueueState::Uploaded);

    // And the bytes on the wire are the ones the contributor was shown,
    // not a third redaction nobody saw.
    let previewed_events: serde_json::Value = serde_json::from_str(&second_body).unwrap();
    assert_eq!(
        received[0].get("events"),
        Some(&previewed_events),
        "the upload must carry exactly the envelope the last preview showed"
    );
}

#[tokio::test]
async fn a_resolved_entry_leaves_no_redacted_content_on_disk() {
    // The stored envelope is redacted trace content at rest. It is bounded
    // to the approval it belongs to: once the entry is resolved, it goes.
    let h = Harness::new().await;
    h.write_session("myproj", "7b7b7b7b-7b7b-7b7b-7b7b-7b7b7b7b7b7b");
    h.discover().await;
    let entry_id = h.only_entry().entry_id;

    ipc::open_preview(&h.shared, entry_id).await.unwrap();
    let stored = h
        .shared
        .store
        .daemon_path(&trace_commons_contributor::daemon::approved_envelope::file_name(entry_id));
    assert!(stored.exists(), "a preview stores what it showed");

    let resp = ipc::handle_local(
        &h.shared,
        "approve",
        serde_json::json!({ "entry_id": entry_id.to_string() }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);
    h.upload_pass().await.unwrap();

    assert_eq!(h.only_entry().state, QueueState::Uploaded);
    assert!(
        !stored.exists(),
        "an uploaded entry must not leave its redacted envelope on disk"
    );
}
