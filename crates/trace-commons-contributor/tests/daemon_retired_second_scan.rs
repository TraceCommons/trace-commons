//! The client-side NEAR AI second scan is retired (owner ruling, 2026-11:
//! "I retire the second scan completely"; on an old config: "Ignore it,
//! local only").
//!
//! A machine that still carries the old settings -- `pii_filter = "near-ai"`
//! in `contributor.json`, a `near_ai` credential in `daemon-settings.json`,
//! even the first-use notice marker -- must upload with local scrubbing
//! only. Nothing it holds may send message text to a NEAR AI classifier,
//! no upload may wait on the retired notice, a grant made before the scan
//! retired must still compare fail-closed, and the retired acknowledge
//! method must change nothing.
//!
//! The legacy state is written as raw JSON on disk, the way an older build
//! left it, and read back through the real loaders: the fields no longer
//! exist on the structs, so a struct literal could not express it.

#![cfg(unix)]
// Drives the daemon through `DaemonShared` and the upload pass, both of which
// assume the unix IPC build.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::{Json, Router, routing::post};
use chrono::Utc;

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig, DAEMON_SETTINGS_FILE,
};
use trace_commons_contributor::daemon::grant_terms::{GrantTerms, VOID_FILTER};
use trace_commons_contributor::daemon::ipc::{self, DaemonShared};
use trace_commons_contributor::daemon::queue::QueueState;
use trace_commons_contributor::identity::DeviceIdentity;

/// The marker file an older build wrote once the first-use notice had been
/// shown. Named literally: the constant is private to the config module.
const LEGACY_NOTICE_MARKER: &str = "near-ai-notice-shown";

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

/// A NEAR AI classify endpoint that counts every request and redacts
/// nothing. The count is the assertion: it must stay at zero.
fn counting_classifier(calls: Arc<AtomicUsize>) -> Router {
    Router::new().route(
        "/privacy/classify",
        post(move || {
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({ "data": [{ "spans": [] }] }))
            }
        }),
    )
}

struct Harness {
    _dir: tempfile::TempDir,
    store_dir: std::path::PathBuf,
    shared: Arc<DaemonShared>,
    claude_root: std::path::PathBuf,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
    classify_calls: Arc<AtomicUsize>,
}

/// Add `key: value` to the JSON object in `path`, in place. Writing over the
/// existing file keeps its 0600 mode.
fn add_key(path: &std::path::Path, key: &str, value: serde_json::Value) {
    let mut body: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    body.as_object_mut().unwrap().insert(key.into(), value);
    std::fs::write(path, serde_json::to_vec_pretty(&body).unwrap()).unwrap();
}

impl Harness {
    /// A machine an older build configured for the second scan. With
    /// `notice_marker`, the old first-use notice had also been shown, so
    /// the old build would have sent text to the classifier straight away.
    async fn legacy(notice_marker: bool) -> Self {
        let received = Arc::new(Mutex::new(Vec::new()));
        let classify_calls = Arc::new(AtomicUsize::new(0));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let classifier = spawn(counting_classifier(classify_calls.clone())).await;

        let dir = tempfile::tempdir().unwrap();
        let store_dir = dir.path().join("state");
        let store = ConfigStore::open(store_dir.clone()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg: ContributorConfig = serde_json::from_value(serde_json::json!({
            "schema_version": CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
            "issuer_url": issuer,
            "ingest_url": ingest,
            "audience": "trace-commons-upload",
            "tenant_id": "tenant-abc",
            "instance_id": "instance-1",
            "user_subject": "alice",
            "device_key_id": device.device_key_id,
            "consent_scopes": ["debugging_evaluation"],
            "consent_scopes_chosen": true,
            "allowed_hosts": "127.0.0.1",
        }))
        .unwrap();
        store.save_config(&cfg).unwrap();
        add_key(
            &store.dir().join("contributor.json"),
            "pii_filter",
            serde_json::json!("near-ai"),
        );

        let claude_root = dir.path().join("projects");
        std::fs::create_dir_all(&claude_root).unwrap();
        {
            let shared = DaemonShared::load(store.clone()).unwrap();
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
            s.save(&shared.store).unwrap();
        }
        add_key(
            &store.daemon_path(DAEMON_SETTINGS_FILE),
            "near_ai",
            serde_json::json!({
                "api_key": "legacy-test-key",
                "base_url": classifier,
                "model": null,
            }),
        );
        if notice_marker {
            std::fs::write(store.dir().join(LEGACY_NOTICE_MARKER), b"").unwrap();
        }

        let shared = Arc::new(DaemonShared::load(store).unwrap());
        Self {
            _dir: dir,
            store_dir,
            shared,
            claude_root,
            received,
            classify_calls,
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

    async fn discover(&self) {
        for _ in 0..2 {
            trace_commons_contributor::daemon::watcher::tick(&self.shared, Self::now())
                .await
                .unwrap();
        }
    }

    fn only_entry(&self) -> trace_commons_contributor::daemon::queue::QueueEntry {
        self.shared.queue.lock().unwrap().all()[0].clone()
    }

    /// Discover one session, preview it, approve it, and run an upload pass.
    async fn contribute_one(&self, id: &str) {
        self.write_session("myproj", id);
        self.discover().await;
        let entry_id = self.only_entry().entry_id;
        ipc::open_preview(&self.shared, entry_id).await.unwrap();
        let resp = ipc::handle_local(
            &self.shared,
            "approve",
            serde_json::json!({ "entry_id": entry_id.to_string() }),
        );
        assert!(resp.error.is_none(), "{:?}", resp.error);
        trace_commons_contributor::daemon::drain_approved_for_test(&self.shared, Self::now())
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn an_old_near_ai_config_uploads_with_local_scrubbing_only() {
    // The old build had shown its notice, so it would have classified
    // straight away. Now the setting is gone: the session uploads, and the
    // classifier the old settings point at hears nothing.
    let h = Harness::legacy(true).await;
    h.contribute_one("8a8a8a8a-8a8a-8a8a-8a8a-8a8a8a8a8a8a")
        .await;

    assert_eq!(
        h.classify_calls.load(Ordering::SeqCst),
        0,
        "no message text may be sent to a NEAR AI classifier from the client"
    );
    let received = h.received.lock().unwrap().clone();
    assert_eq!(received.len(), 1, "entry is {:?}", h.only_entry());
    assert_eq!(h.only_entry().state, QueueState::Uploaded);
    let wire = serde_json::to_string(&received[0]).unwrap();
    assert!(
        !wire.contains("privacy-filter-near-ai"),
        "the envelope must not claim a NEAR AI scan that never ran"
    );
}

#[tokio::test]
async fn no_upload_is_held_for_the_retired_notice() {
    // No marker: the old build refused every upload until the first-use
    // notice was acknowledged. That notice is retired with the scan, so
    // nothing waits for it.
    let h = Harness::legacy(false).await;
    h.contribute_one("8b8b8b8b-8b8b-8b8b-8b8b-8b8b8b8b8b8b")
        .await;

    assert_eq!(
        h.only_entry().state,
        QueueState::Uploaded,
        "an upload must never be held for the retired notice"
    );
    assert_eq!(h.received.lock().unwrap().len(), 1);
    assert_eq!(h.classify_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn stored_grants_compare_fail_closed_after_the_scan_retires() {
    let h = Harness::legacy(true).await;
    let now = GrantTerms::in_force(&h.shared).expect("enrolled");

    // A grant made with no filter at all still holds: the retired settings
    // left on disk do not turn into a filter term, so nobody is re-asked.
    let mut no_filter = now.clone();
    no_filter.pii_filter = None;
    no_filter.classifier_present = false;
    no_filter.classifier_base_url = None;
    no_filter.classifier_model = None;
    assert_eq!(
        now.widening_from(&no_filter),
        Vec::<&str>::new(),
        "a grant made without a filter must still hold"
    );

    // A grant made under the scan voids, once, as a filter change: the
    // removed scan scrubs less than what the contributor granted.
    let mut under_scan = no_filter.clone();
    under_scan.pii_filter = Some("near-ai".into());
    under_scan.classifier_present = true;
    assert_eq!(now.widening_from(&under_scan), vec![VOID_FILTER]);
}

#[tokio::test]
async fn acknowledging_the_retired_notice_is_an_unknown_method() {
    let h = Harness::legacy(false).await;
    let resp = ipc::handle_local(
        &h.shared,
        "acknowledge_near_ai_notice",
        serde_json::json!({}),
    );
    let err = resp
        .error
        .expect("the retired acknowledge method must not answer ok");
    assert_eq!(err.code, ipc::ERR_UNKNOWN_METHOD);
    assert_eq!(err.message, "unknown-method");

    let actions: Vec<String> = trace_commons_contributor::daemon::audit::load(&h.shared.store)
        .unwrap()
        .into_iter()
        .map(|e| e.action)
        .collect();
    assert!(
        !actions.iter().any(|a| a == "near-ai-notice-acknowledged"),
        "a retired method writes no audit row: {actions:?}"
    );
    assert!(
        !h.store_dir.join(LEGACY_NOTICE_MARKER).exists(),
        "a retired method creates no marker"
    );
}
