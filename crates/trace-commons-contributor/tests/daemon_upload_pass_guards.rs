//! Properties of one upload pass (`drain_approved`) that the per-decision
//! unit tests could not see, because each of them is about how the pass as
//! a whole behaves around a single decision.
//!
//! - A fail-closed privacy-filter canary must set a health label, and that
//!   label must survive the abort. `LABEL_CANARY_FAILED` is in
//!   `EXPIRY_BLOCKING_LABELS`, and nothing in production ever set it.
//! - Pause must stop uploads, not just discovery.
//! - `cancel` must be able to cancel, and must stop being able to once the
//!   upload is genuinely in flight.
//! - An approval covers the consent scopes it was given under.

use std::sync::{Arc, Mutex};

use axum::{Json, Router, routing::post};
use chrono::Utc;

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};
use trace_commons_contributor::daemon::health::LABEL_CANARY_FAILED;
use trace_commons_contributor::daemon::ipc::{self, DaemonShared};
use trace_commons_contributor::daemon::policy::ProjectMode;
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

/// A well-formed privacy filter that removes nothing: 200 with an empty
/// span list for every input. This is exactly the shape of a filter outage
/// that fails the batch canary, and it is what a real outage looks like
/// from the client's side.
fn noop_privacy_filter() -> Router {
    Router::new().route(
        "/privacy/classify",
        post(|Json(_): Json<serde_json::Value>| async {
            Json(serde_json::json!({"data": [{"spans": []}]}))
        }),
    )
}

struct Harness {
    _dir: tempfile::TempDir,
    shared: Arc<DaemonShared>,
    claude_root: std::path::PathBuf,
    received: Arc<Mutex<Vec<serde_json::Value>>>,
}

impl Harness {
    async fn new() -> Self {
        Self::build(false).await
    }

    async fn with_broken_privacy_filter() -> Self {
        Self::build(true).await
    }

    async fn build(broken_filter: bool) -> Self {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;

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
                pii_filter: broken_filter.then(|| "near-ai".to_string()),
                allowed_hosts: Some("127.0.0.1".into()),
            })
            .unwrap();

        let claude_root = dir.path().join("projects");
        std::fs::create_dir_all(&claude_root).unwrap();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        // Spawned before the lock is taken: holding a std MutexGuard across
        // an await is a real hazard, not a lint to silence.
        let filter_base = if broken_filter {
            Some(spawn(noop_privacy_filter()).await)
        } else {
            None
        };
        {
            let mut s = shared.settings.lock().unwrap();
            s.claude_root = Some(claude_root.clone());
            s.codex_root = Some(dir.path().join("codex"));
            if let Some(base_url) = filter_base {
                s.near_ai = Some(NearAiSettings {
                    api_key: "test-key".into(),
                    base_url: Some(base_url),
                    model: None,
                });
            }
        }
        if broken_filter {
            // The notice gate is a separate precondition; acknowledge it so
            // the canary is what the pass actually trips on.
            shared.store.ensure_near_ai_notice_shown().unwrap();
        }
        Self {
            _dir: dir,
            shared,
            claude_root,
            received,
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
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"fix the parser\"}},\
                 \"cwd\":\"/Users/testuser/code/{project}\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"{id}\",\"uuid\":\"a1\"}}\n"
            ),
        )
        .unwrap();
    }

    fn opt_in(&self, project: &str) {
        self.shared
            .policy
            .lock()
            .unwrap()
            .set_mode(
                &format!("/Users/testuser/code/{project}"),
                ProjectMode::AutoUpload,
                Utc::now(),
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

    fn states(&self) -> Vec<QueueState> {
        self.shared
            .queue
            .lock()
            .unwrap()
            .all()
            .iter()
            .map(|e| e.state)
            .collect()
    }

    fn only_entry(&self) -> trace_commons_contributor::daemon::queue::QueueEntry {
        self.shared.queue.lock().unwrap().all()[0].clone()
    }
}

// --- I1: a canary failure must be visible, and must suspend expiry --------

#[tokio::test]
async fn a_privacy_filter_canary_failure_sets_the_health_label_that_suspends_expiry() {
    // Before this fix, `LABEL_CANARY_FAILED` was in EXPIRY_BLOCKING_LABELS
    // but was never set by any production code path: the canary failure
    // propagated as an opaque error, the supervisor logged a warning and
    // continued, health said "ok", `blocks_expiry()` was false, and the
    // fourteen-day clock ran straight through a filter outage -- discarding
    // pending traces as "expired-without-decision" that nobody declined.
    let h = Harness::with_broken_privacy_filter().await;
    h.opt_in("myproj");
    h.write_session("myproj", "11111111-1111-1111-1111-111111111111");
    h.discover().await;

    let err = h.upload_pass().await.unwrap_err();
    assert!(
        format!("{err:#}").contains("canary"),
        "the pass must abort on the canary, got: {err:#}"
    );

    assert_eq!(h.received.lock().unwrap().len(), 0, "nothing may be sent");
    let health = h.shared.health.lock().unwrap().clone();
    assert_eq!(
        health.last_error_label.as_deref(),
        Some(LABEL_CANARY_FAILED),
        "a canary failure must be visible in health"
    );
    assert!(
        health.blocks_expiry(),
        "a filter outage must suspend the expiry clock"
    );
}

#[tokio::test]
async fn a_canary_abort_still_persists_the_passs_own_mutations() {
    // The abort used to propagate with `?`, skipping drain_approved's
    // trailing `if changed { save }` entirely -- so the pass's queue and
    // state mutations, and the health label itself, were thrown away with
    // it.
    let h = Harness::with_broken_privacy_filter().await;
    h.opt_in("myproj");
    h.write_session("myproj", "22222222-2222-2222-2222-222222222222");
    h.discover().await;

    assert!(h.upload_pass().await.is_err());

    // The entry was claimed for upload and must not be stranded in
    // `Uploading`, a state nothing else would move it out of.
    assert_eq!(
        h.states(),
        vec![QueueState::Approved],
        "an aborted pass must release its claim, not strand the entry"
    );
}

// --- I2: pause must stop uploads, not just discovery ----------------------

#[tokio::test]
async fn pausing_after_entries_are_approved_still_stops_the_upload() {
    // The old end-to-end test paused before anything was queued, so it only
    // proved discovery stops -- its `queue.all().is_empty()` assertion gives
    // that away. Everything already Approved kept uploading while `status`
    // reported paused.
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "33333333-3333-3333-3333-333333333333");
    h.discover().await;
    assert_eq!(
        h.states(),
        vec![QueueState::Approved],
        "the armed project must have auto-approved before the pause"
    );

    let resp = ipc::handle_local(&h.shared, "pause", serde_json::json!({}));
    assert!(resp.error.is_none(), "{:?}", resp.error);

    h.upload_pass().await.unwrap();

    assert_eq!(
        h.received.lock().unwrap().len(),
        0,
        "pause must mean nothing leaves this machine, not merely that \
         nothing new is discovered"
    );
    assert_eq!(h.states(), vec![QueueState::Approved]);

    // And resuming lets it through, so the pause is a pause and not a drop.
    ipc::handle_local(&h.shared, "resume", serde_json::json!({}));
    h.upload_pass().await.unwrap();
    assert_eq!(h.received.lock().unwrap().len(), 1);
}

// --- I3: cancel must actually cancel -------------------------------------

#[tokio::test]
async fn cancelling_before_the_pass_runs_stops_the_upload() {
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "44444444-4444-4444-4444-444444444444");
    h.discover().await;
    let entry_id = h.only_entry().entry_id;

    let resp = ipc::handle_local(
        &h.shared,
        "cancel",
        serde_json::json!({ "entry_id": entry_id.to_string() }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);

    h.upload_pass().await.unwrap();

    assert_eq!(
        h.received.lock().unwrap().len(),
        0,
        "a cancelled entry must not upload from a stale snapshot of the \
         approved set"
    );
    assert_eq!(h.states(), vec![QueueState::Pending]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelling_mid_upload_is_refused_rather_than_falsely_acknowledged() {
    // The real race, and the one that lied to the contributor: cancel
    // arriving while the upload is genuinely in flight. `drain_approved`
    // snapshotted the approved set and left every entry `Approved` for the
    // whole upload, and nothing in production ever set `Uploading` -- so
    // `cancel` answered `ok: true`, set `Pending`, the upload proceeded from
    // the snapshot regardless, and the terminal `Uploaded` overwrote
    // `Pending`. The contributor was told an upload was cancelled after it
    // had been sent.
    let received = Arc::new(Mutex::new(Vec::new()));
    let arrived = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let issuer = spawn(stub_issuer()).await;
    let ingest = {
        let received = received.clone();
        let arrived = arrived.clone();
        let release = release.clone();
        spawn(Router::new().route(
            "/v1/traces",
            post(move |Json(body): Json<serde_json::Value>| {
                let (received, arrived, release) =
                    (received.clone(), arrived.clone(), release.clone());
                async move {
                    received.lock().unwrap().push(body);
                    arrived.notify_one();
                    release.notified().await;
                    Json(serde_json::json!({
                        "status": "accepted",
                        "credit_points_pending": 1.0,
                        "explanation": []
                    }))
                }
            }),
        ))
        .await
    };

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
            pii_filter: None,
            allowed_hosts: Some("127.0.0.1".into()),
        })
        .unwrap();
    let claude_root = dir.path().join("projects");
    std::fs::create_dir_all(&claude_root).unwrap();
    let shared = Arc::new(DaemonShared::load(store).unwrap());
    {
        let mut s = shared.settings.lock().unwrap();
        s.claude_root = Some(claude_root.clone());
        s.codex_root = Some(dir.path().join("codex"));
    }
    shared
        .policy
        .lock()
        .unwrap()
        .set_mode(
            "/Users/testuser/code/myproj",
            ProjectMode::AutoUpload,
            Utc::now(),
        )
        .unwrap();
    let project_dir = claude_root.join("-Users-testuser-code-myproj");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(
        project_dir.join("99999999-9999-9999-9999-999999999999.jsonl"),
        "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"fix the parser\"},\
         \"cwd\":\"/Users/testuser/code/myproj\",\"timestamp\":\"2026-08-08T10:00:00Z\",\
         \"version\":\"2.0.1\",\"sessionId\":\"99999999-9999-9999-9999-999999999999\",\
         \"uuid\":\"a1\"}\n",
    )
    .unwrap();

    let now: chrono::DateTime<Utc> = "2030-01-01T00:00:00Z".parse().unwrap();
    for _ in 0..2 {
        trace_commons_contributor::daemon::watcher::tick(&shared, now)
            .await
            .unwrap();
    }
    let entry_id = shared.queue.lock().unwrap().all()[0].entry_id;

    let pass = {
        let shared = Arc::clone(&shared);
        tokio::spawn(async move {
            trace_commons_contributor::daemon::drain_approved_for_test(&shared, now)
                .await
                .unwrap();
        })
    };

    // The envelope is now sitting in the ingest handler: the upload is
    // genuinely in flight, and this is the moment a cancel must be refused.
    arrived.notified().await;
    let resp = {
        let shared = Arc::clone(&shared);
        tokio::task::spawn_blocking(move || {
            ipc::handle_local(
                &shared,
                "cancel",
                serde_json::json!({ "entry_id": entry_id.to_string() }),
            )
        })
        .await
        .unwrap()
    };
    release.notify_one();
    pass.await.unwrap();

    assert!(
        resp.error.is_some(),
        "cancel must be refused while the upload is in flight, not \
         acknowledged for an upload that has already been sent"
    );
    assert_eq!(received.lock().unwrap().len(), 1);
    assert_eq!(
        shared.queue.lock().unwrap().get(entry_id).unwrap().state,
        QueueState::Uploaded
    );
}

#[tokio::test]
async fn an_entry_claimed_for_upload_can_no_longer_be_cancelled() {
    // `QueueState::Uploading` had no production writer at all, so `cancel`
    // always answered `ok: true` -- including for an upload that had
    // already been sent.
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "55555555-5555-5555-5555-555555555555");
    h.discover().await;
    let entry_id = h.only_entry().entry_id;

    assert!(
        h.shared.queue.lock().unwrap().claim_for_upload(entry_id),
        "an approved entry must be claimable"
    );

    let resp = ipc::handle_local(
        &h.shared,
        "cancel",
        serde_json::json!({ "entry_id": entry_id.to_string() }),
    );
    assert!(
        resp.error.is_some(),
        "cancel must be refused once the upload is genuinely in flight"
    );
}

// --- I4: an approval covers the scopes it was given under -----------------

#[tokio::test]
async fn widening_consent_scopes_after_approval_revokes_the_approval() {
    // Preview shows one scope set, the contributor approves on the strength
    // of it, someone calls set_consent_scopes, and -- before this fix -- the
    // same trace uploaded under the wider set, because drain_approved
    // rebuilt its SubmitContext from a freshly-read config every pass with
    // nothing coupling it to what was approved.
    let h = Harness::new().await;
    h.write_session("otherproj", "66666666-6666-6666-6666-666666666666");
    h.discover().await;
    let entry_id = h.only_entry().entry_id;

    let resp = ipc::handle_local(
        &h.shared,
        "approve",
        serde_json::json!({ "entry_id": entry_id.to_string() }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);
    assert_eq!(
        h.only_entry().approved_scopes.as_deref(),
        Some(["debugging_evaluation".to_string()].as_slice()),
        "an approval must record the scopes it was given under"
    );

    let resp = ipc::handle_local(
        &h.shared,
        "set_consent_scopes",
        serde_json::json!({ "scopes": ["debugging_evaluation", "model_training"] }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);

    h.upload_pass().await.unwrap();

    assert_eq!(
        h.received.lock().unwrap().len(),
        0,
        "a trace approved under one consent scope set must not be sent \
         under a wider one"
    );
    let entry = h.only_entry();
    assert_eq!(entry.state, QueueState::Pending);
    assert_eq!(
        entry.reason_label.as_deref(),
        Some("consent-scopes-changed-after-approval")
    );
    assert!(entry.approved_scopes.is_none());
}

// --- M4: a standing opt-in must keep applying -----------------------------

#[tokio::test]
async fn an_armed_project_re_approves_an_entry_that_went_back_to_pending() {
    // `Queue::upsert` never rewrites an existing entry, so an entry put
    // back to Pending (by supersede, or by the consent-scope guard above)
    // sat Pending forever in a project the contributor had explicitly
    // armed -- the opt-in silently stopped applying until TTL expiry.
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "77777777-7777-7777-7777-777777777777");
    h.discover().await;
    let entry_id = h.only_entry().entry_id;

    // Put it back to Pending the way the consent-scope guard does.
    h.shared
        .queue
        .lock()
        .unwrap()
        .revoke_approval(entry_id, "consent-scopes-changed-after-approval");
    assert_eq!(h.only_entry().state, QueueState::Pending);

    h.discover().await;

    assert_eq!(
        h.only_entry().state,
        QueueState::Approved,
        "a standing opt-in must keep applying to its project's sessions"
    );
}

#[tokio::test]
async fn re_approving_never_resurrects_an_entry_the_contributor_declined() {
    // The M4 fix must not turn `dismiss` into a no-op in an armed project.
    let h = Harness::new().await;
    h.opt_in("myproj");
    h.write_session("myproj", "88888888-8888-8888-8888-888888888888");
    h.discover().await;
    let entry_id = h.only_entry().entry_id;

    let resp = ipc::handle_local(
        &h.shared,
        "dismiss",
        serde_json::json!({ "entry_id": entry_id.to_string() }),
    );
    assert!(resp.error.is_none(), "{:?}", resp.error);
    assert_eq!(h.only_entry().state, QueueState::Refused);

    h.discover().await;
    h.upload_pass().await.unwrap();

    assert_eq!(h.only_entry().state, QueueState::Refused);
    assert_eq!(h.received.lock().unwrap().len(), 0);
}
