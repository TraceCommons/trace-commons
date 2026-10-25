use super::*;
use crate::{
    account_auth::AccountSession,
    daemon::{cloud_credential_test_support::MemoryBackend, uploader},
};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, Semaphore};

const TENANT: &str = "upload-allowance-test";

#[derive(Clone)]
struct Reply {
    status: StatusCode,
    body: Value,
    rotated: Option<String>,
    stalled: bool,
}

struct ServerState {
    reply: Mutex<Reply>,
    hits: AtomicUsize,
    authorization: Mutex<Option<String>>,
    started: Notify,
    release: Semaphore,
}

struct Server {
    base: String,
    state: Arc<ServerState>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn allowance() -> Value {
    json!({"allowance": {"schema_version": 1, "max_uploads_per_day": 200,
        "max_bytes_per_day": 2_147_483_648u64}})
}

impl Server {
    async fn new() -> Self {
        let state = Arc::new(ServerState {
            reply: Mutex::new(Reply {
                status: StatusCode::OK,
                body: allowance(),
                rotated: None,
                stalled: false,
            }),
            hits: AtomicUsize::new(0),
            authorization: Mutex::new(None),
            started: Notify::new(),
            release: Semaphore::new(0),
        });
        async fn handler(
            State(state): State<Arc<ServerState>>,
            headers: HeaderMap,
        ) -> axum::response::Response {
            let reply = state.reply.lock().unwrap().clone();
            *state.authorization.lock().unwrap() = headers
                .get("authorization")
                .map(|v| v.to_str().unwrap().to_owned());
            state.hits.fetch_add(1, Ordering::SeqCst);
            state.started.notify_one();
            if reply.stalled {
                state.release.acquire().await.unwrap().forget();
            }
            let mut response = (reply.status, Json(reply.body)).into_response();
            if let Some(rotated) = reply.rotated {
                response.headers_mut().insert(
                    trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                    rotated.parse().unwrap(),
                );
                response.headers_mut().insert(
                    trace_commons_protocol::ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER,
                    (Utc::now() + chrono::TimeDelta::hours(12))
                        .to_rfc3339()
                        .parse()
                        .unwrap(),
                );
            }
            response
        }
        let router = Router::new()
            .route("/v1/account/upload-allowance", get(handler))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { base, state, task }
    }

    fn respond(&self, status: StatusCode, body: Value) {
        *self.state.reply.lock().unwrap() = Reply {
            status,
            body,
            rotated: None,
            stalled: false,
        };
    }

    fn hits(&self) -> usize {
        self.state.hits.load(Ordering::SeqCst)
    }
}

struct Daemon {
    _dir: tempfile::TempDir,
    shared: Arc<DaemonShared>,
    credential_reads: Arc<AtomicUsize>,
}

struct CountingBackend {
    inner: MemoryBackend,
    reads: Arc<AtomicUsize>,
}

impl super::super::credential_store::SecretBackend for CountingBackend {
    fn read(
        &self,
        reference: &super::super::credential_store::CredentialReference,
    ) -> Result<Vec<u8>, super::super::credential_store::CredentialError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        super::super::credential_store::SecretBackend::read(&self.inner, reference)
    }
    fn write(
        &self,
        reference: &super::super::credential_store::CredentialReference,
        bytes: &[u8],
    ) -> Result<(), super::super::credential_store::CredentialError> {
        super::super::credential_store::SecretBackend::write(&self.inner, reference, bytes)
    }
    fn delete(
        &self,
        reference: &super::super::credential_store::CredentialReference,
    ) -> Result<(), super::super::credential_store::CredentialError> {
        super::super::credential_store::SecretBackend::delete(&self.inner, reference)
    }
}

fn token(secret: &str) -> String {
    super::super::account_admission::native_token_for_test(TENANT, secret)
}

impl Daemon {
    fn new(base: &str) -> Self {
        let (dir, store) = crate::config::tests_support::temp_store();
        let credential_reads = Arc::new(AtomicUsize::new(0));
        commons_credentials::install_test_backend(
            &store,
            Arc::new(CountingBackend {
                inner: MemoryBackend::default(),
                reads: credential_reads.clone(),
            }),
        );
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = base.to_owned();
        cfg.tenant_id = TENANT.to_owned();
        store.save_config(&cfg).unwrap();
        let session = AccountSession {
            access_token: token("original"),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "allowance-account".to_owned(),
        };
        store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        crate::account_auth::try_load_token(&store)
            .unwrap()
            .unwrap();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        credential_reads.store(0, Ordering::SeqCst);
        Self {
            _dir: dir,
            shared,
            credential_reads,
        }
    }

    fn active(base: &str) -> Self {
        let daemon = Self::new(base);
        daemon
            .shared
            .policy
            .lock()
            .unwrap()
            .set_mode(
                "/tmp/allowance-armed-project",
                super::super::policy::ProjectMode::AutoUpload,
                Utc::now(),
            )
            .unwrap();
        daemon
    }
}

#[cfg(test)]
#[tokio::test]
async fn http_refresh_raises_status_and_enforcement_while_capped_without_an_armed_project() {
    let server = Server::new().await;
    let daemon = Daemon::new(&server.base);
    let shared = &daemon.shared;
    let now = Utc::now();
    {
        let mut usage = shared.state.lock().unwrap();
        usage.roll_day(now);
        usage.uploads_today = 50;
        usage.bytes_today = 209_715_200;
    }
    shared
        .queue
        .lock()
        .unwrap()
        .upsert(
            super::super::queue::QueueEntry {
                entry_id: uuid::Uuid::new_v4(),
                session_hash: "sha256:0000".to_owned(),
                source: "claude-code".to_owned(),
                project_key: "/tmp/allowance-project".to_owned(),
                project_label: "allowance-project".to_owned(),
                path: std::path::PathBuf::from("/tmp/allowance-session.jsonl"),
                size_bytes: 1_024,
                discovered_at: now,
                state: super::super::queue::QueueState::Approved,
                ..Default::default()
            },
            shared.settings.lock().unwrap().max_queue_entries,
        )
        .unwrap();
    assert!(shared.daily_budget(now).blocked());
    assert_eq!(shared.daily_budget(now).blocked_entries, 1);
    assert!(!uploader::cap_check(
        &shared.state.lock().unwrap(),
        1,
        &effective_settings(shared)
    ));
    refresh(shared, now, false).await;
    let budget = shared.status_value_at(now)["daily_budget"].clone();
    assert_eq!(budget["blocked"], false);
    assert_eq!(budget["blocked_entries"], 0);
    assert_eq!(budget["max_uploads_per_day"], 200);
    assert_eq!(budget["max_bytes_per_day"], 2_147_483_648u64);
    assert_eq!(budget["uploads_today"], 50);
    assert_eq!(budget["bytes_today"], 209_715_200);
    assert!(uploader::cap_check(
        &shared.state.lock().unwrap(),
        1,
        &effective_settings(shared)
    ));
    assert_eq!(shared.settings.lock().unwrap().max_uploads_per_day, 50);
    assert_eq!(
        server.state.authorization.lock().unwrap().as_deref(),
        Some(format!("Bearer {}", token("original")).as_str())
    );
    assert_eq!(server.hits(), 1);
    refresh(shared, now + chrono::TimeDelta::minutes(4), false).await;
    assert_eq!(server.hits(), 1);
    refresh(shared, now + chrono::TimeDelta::minutes(5), false).await;
    assert_eq!(server.hits(), 2);

    super::super::settings::apply_settings_object(
        &mut shared.settings.lock().unwrap(),
        &json!({"max_uploads_per_day": 50}),
    )
    .unwrap();
    let budget = shared.status_value_at(now)["daily_budget"].clone();
    assert_eq!(budget["max_uploads_per_day"], 50);
    assert_eq!(budget["max_bytes_per_day"], 2_147_483_648u64);
    assert!(!uploader::cap_check(
        &shared.state.lock().unwrap(),
        1,
        &effective_settings(shared)
    ));
}

#[tokio::test]
async fn failed_and_invalid_replies_retain_the_allowance_but_explicit_null_clears_it() {
    let server = Server::new().await;
    let daemon = Daemon::active(&server.base);
    let mut now = Utc::now();
    refresh(&daemon.shared, now, false).await;
    for (status, body) in [
        (StatusCode::INTERNAL_SERVER_ERROR, allowance()),
        (StatusCode::OK, json!({"allowance": "malformed"})),
        (StatusCode::OK, json!({})),
        (
            StatusCode::OK,
            json!({"allowance": {"schema_version": 2, "max_uploads_per_day": 200, "max_bytes_per_day": 2_147_483_648u64}}),
        ),
        (
            StatusCode::OK,
            json!({"allowance": {"schema_version": 1, "max_uploads_per_day": 1_001, "max_bytes_per_day": 2_147_483_648u64}}),
        ),
        (
            StatusCode::OK,
            json!({"allowance": {"schema_version": 1, "max_uploads_per_day": 200, "max_bytes_per_day": 5_368_709_121u64}}),
        ),
    ] {
        server.respond(status, body);
        now += chrono::TimeDelta::minutes(5);
        refresh(&daemon.shared, now, false).await;
        let effective = effective_settings(&daemon.shared);
        assert_eq!(effective.max_uploads_per_day, 200);
        assert_eq!(effective.max_bytes_per_day, 2_147_483_648);
    }
    server.respond(StatusCode::OK, json!({"allowance": null}));
    refresh(&daemon.shared, now + chrono::TimeDelta::minutes(5), false).await;
    let effective = effective_settings(&daemon.shared);
    assert_eq!(effective.max_uploads_per_day, 50);
    assert_eq!(effective.max_bytes_per_day, 209_715_200);
}

#[tokio::test]
async fn dry_run_never_fetches_and_does_not_delay_the_first_real_refresh() {
    let server = Server::new().await;
    let daemon = Daemon::active(&server.base);
    let now = Utc::now();
    refresh(&daemon.shared, now, true).await;
    assert_eq!(server.hits(), 0);
    assert_eq!(daemon.credential_reads.load(Ordering::SeqCst), 0);
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 50);
    refresh(&daemon.shared, now, false).await;
    assert_eq!(server.hits(), 1);
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 200);
}

#[tokio::test]
async fn rotated_session_is_persisted_without_losing_the_new_allowance() {
    let server = Server::new().await;
    server.state.reply.lock().unwrap().rotated = Some(token("rotated"));
    let daemon = Daemon::active(&server.base);
    refresh(&daemon.shared, Utc::now(), false).await;
    assert_eq!(
        crate::account_auth::try_load_token(&daemon.shared.store).unwrap(),
        Some(token("rotated"))
    );
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 200);
}

#[tokio::test]
async fn configuration_change_during_http_invalidates_the_cached_allowance_and_late_reply() {
    stale_reply_is_discarded(false).await;
}

#[tokio::test]
async fn sign_out_during_http_invalidates_the_cached_allowance_and_late_reply() {
    stale_reply_is_discarded(true).await;
}

async fn stale_reply_is_discarded(sign_out: bool) {
    let server = Server::new().await;
    let daemon = Daemon::active(&server.base);
    let now = Utc::now();
    refresh(&daemon.shared, now, false).await;
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 200);
    // Consume the first request's notification before gating the next one.
    server.state.started.notified().await;
    server.state.reply.lock().unwrap().stalled = true;
    let shared = daemon.shared.clone();
    let request = tokio::spawn(async move {
        refresh(&shared, now + chrono::TimeDelta::minutes(5), false).await;
    });
    tokio::time::timeout(Duration::from_secs(5), server.state.started.notified())
        .await
        .unwrap();
    if sign_out {
        commons_credentials::clear(&daemon.shared.store, &[commons_credentials::Kind::Account])
            .unwrap();
    } else {
        let mut cfg = daemon.shared.store.load_config().unwrap().unwrap();
        cfg.user_subject = "changed-subject".to_owned();
        daemon.shared.store.save_config(&cfg).unwrap();
    }
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 50);
    server.state.release.add_permits(1);
    tokio::time::timeout(Duration::from_secs(5), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 50);
}

#[tokio::test]
async fn transient_credential_read_failure_retains_the_same_lifecycle_allowance() {
    let server = Server::new().await;
    let daemon = Daemon::active(&server.base);
    let now = Utc::now();
    refresh(&daemon.shared, now, false).await;
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 200);
    daemon.shared.account_admission.drop_session();
    // Preserve the account metadata and lifecycle while making the OS-store
    // secret temporarily unreadable, as a locked or unavailable store would.
    commons_credentials::install_test_backend(
        &daemon.shared.store,
        Arc::new(MemoryBackend::default()),
    );
    refresh(&daemon.shared, now + chrono::TimeDelta::minutes(5), false).await;
    assert_eq!(server.hits(), 1);
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 200);
    assert_eq!(
        effective_settings(&daemon.shared).max_bytes_per_day,
        2_147_483_648
    );
}

#[tokio::test]
async fn idle_and_paused_allowance_refresh_never_read_credentials_or_call_http() {
    let server = Server::new().await;
    let idle = Daemon::new(&server.base);
    let now = Utc::now();
    refresh(&idle.shared, now, false).await;
    refresh(&idle.shared, now + chrono::TimeDelta::minutes(5), false).await;
    assert_eq!(server.hits(), 0, "an idle sign-in must expire normally");
    assert_eq!(idle.credential_reads.load(Ordering::SeqCst), 0);

    let paused = Daemon::active(&server.base);
    paused.shared.state.lock().unwrap().paused = true;
    paused.shared.paused.store(true, Ordering::SeqCst);
    refresh(&paused.shared, now, false).await;
    refresh(&paused.shared, now + chrono::TimeDelta::minutes(5), false).await;
    assert_eq!(
        server.hits(),
        0,
        "pause suppresses allowance requests even when armed"
    );
    assert_eq!(paused.credential_reads.load(Ordering::SeqCst), 0);
    paused.shared.state.lock().unwrap().paused = false;
    paused.shared.paused.store(false, Ordering::SeqCst);
    refresh(&paused.shared, now + chrono::TimeDelta::minutes(5), false).await;
    assert_eq!(server.hits(), 1, "skipped refreshes do not delay resuming");
    assert!(paused.credential_reads.load(Ordering::SeqCst) > 0);

    let quiesced = Daemon::active(&server.base);
    quiesced.shared.quiesced.store(true, Ordering::SeqCst);
    refresh(&quiesced.shared, now, false).await;
    assert_eq!(server.hits(), 1, "update quiesce suppresses new requests");
    assert_eq!(quiesced.credential_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn resetting_one_local_cap_follows_server_again_without_resetting_the_other() {
    let server = Server::new().await;
    let daemon = Daemon::active(&server.base);
    let now = Utc::now();
    let mut settings = daemon.shared.settings.lock().unwrap().clone();
    super::super::settings::apply_settings_object(
        &mut settings,
        &json!({"max_uploads_per_day": 7, "max_bytes_per_day": 1_024}),
    )
    .unwrap();
    *daemon.shared.settings.lock().unwrap() = settings;
    refresh(&daemon.shared, now, false).await;
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 7);
    assert_eq!(effective_settings(&daemon.shared).max_bytes_per_day, 1_024);
    super::super::settings::apply_settings_object(
        &mut daemon.shared.settings.lock().unwrap(),
        &json!({"max_uploads_per_day": null}),
    )
    .unwrap();
    assert_eq!(effective_settings(&daemon.shared).max_uploads_per_day, 200);
    assert_eq!(effective_settings(&daemon.shared).max_bytes_per_day, 1_024);
    super::super::settings::apply_settings_object(
        &mut daemon.shared.settings.lock().unwrap(),
        &json!({"max_bytes_per_day": null}),
    )
    .unwrap();
    assert_eq!(
        effective_settings(&daemon.shared).max_bytes_per_day,
        2_147_483_648
    );
}
