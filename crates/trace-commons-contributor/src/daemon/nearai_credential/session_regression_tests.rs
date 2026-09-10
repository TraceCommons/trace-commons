// INTEGRATION: register `#[cfg(test)] mod session_regression_tests;` in
// daemon/nearai_credential/mod.rs. The existing pub(super) exchange and
// cfg(test) CloudApi::for_test visibility suffice. This module uses only
// synthetic credentials, temporary settings, and an ephemeral loopback server.

use crate::config::tests_support::temp_store;
use crate::daemon::ipc::{DaemonShared, Request};
use crate::daemon::nearai_credential::api::CloudApi;
use crate::daemon::nearai_credential::loopback::SessionTokens;
use crate::daemon::nearai_credential::{exchange, handle_forget};
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const DEADLINE: Duration = Duration::from_secs(15);

#[test]
fn status_reports_inference_and_cloud_session_independently() {
    let fixture = Fixture::new();
    fixture.connect("status");
    let request = Request {
        id: 1,
        method: "near_ai_credential_status".into(),
        params: json!({}),
    };
    let status = crate::daemon::nearai_credential::handle_status(&fixture.shared, &request);
    let body = status.result.expect("status reply");
    assert_eq!(body["state"], "present");
    assert_eq!(body["session_state"], "present");
    fixture.shared.settings.lock().unwrap().near_ai_session = None;
    let body = crate::daemon::nearai_credential::handle_status(&fixture.shared, &request)
        .result
        .unwrap();
    assert_eq!(body["state"], "present");
    assert_eq!(body["session_state"], "absent");
}

struct Fixture {
    shared: Arc<DaemonShared>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let (dir, store) = temp_store();
        Self {
            shared: Arc::new(DaemonShared::load(store).expect("load isolated daemon")),
            _dir: dir,
        }
    }

    // A completed synthetic connection is installed in both stores. This
    // isolates refresh ownership from the browser ceremony and proxy startup.
    fn connect(&self, account: &str) -> NearAiSession {
        let now = Utc::now();
        let session = NearAiSession {
            refresh_token: format!("rt-synthetic-{account}"),
            refresh_token_expires_at: Some(now + chrono::Duration::hours(1)),
            stored_at: now,
        };
        let mut settings = self.shared.settings.lock().expect("fixture settings lock");
        settings.near_ai_session = Some(session.clone());
        settings.near_ai_inference = Some(NearAiInferenceCredential {
            key: format!("sk-synthetic-{account}"),
            key_id: format!("key-{account}"),
            key_prefix: "sk-synthetic".to_owned(),
            organization_id: format!("org-{account}"),
            workspace_id: format!("workspace-{account}"),
            minted_at: now,
        });
        settings
            .save_for_test(&self.shared.store)
            .expect("save synthetic connection");
        session
    }

    fn persisted(&self) -> DaemonSettings {
        DaemonSettings::load_with_cloud_credentials(&self.shared.store)
            .expect("reload isolated settings")
    }

    fn retained(&self) -> NearAiSession {
        let memory = self
            .shared
            .settings
            .lock()
            .expect("fixture settings lock")
            .near_ai_session
            .clone();
        let disk = self.persisted().near_ai_session;
        assert!(
            memory == disk,
            "memory and disk must retain the same session"
        );
        memory.expect("a connected session must remain")
    }
}

struct HeldRefresh {
    entered: oneshot::Receiver<()>,
    release: oneshot::Sender<()>,
}

impl HeldRefresh {
    async fn wait(self) -> oneshot::Sender<()> {
        timeout(DEADLINE, self.entered)
            .await
            .expect("refresh did not reach the response barrier")
            .expect("refresh response barrier closed early");
        self.release
    }
}

struct ResponseGate {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

struct ServerTokens {
    refresh: String,
    access: String,
    rotations: usize,
}

struct ServerState {
    tokens: Mutex<ServerTokens>,
    first_response: Mutex<Option<ResponseGate>>,
    shared: Arc<DaemonShared>,
    balance_reads: AtomicUsize,
}

struct LocalCloud {
    api: CloudApi,
    state: Arc<ServerState>,
    server: JoinHandle<()>,
}

impl LocalCloud {
    async fn start(fixture: &Fixture, initial: &NearAiSession) -> (Self, HeldRefresh) {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        let state = Arc::new(ServerState {
            tokens: Mutex::new(ServerTokens {
                refresh: initial.refresh_token.clone(),
                access: String::new(),
                rotations: 0,
            }),
            first_response: Mutex::new(Some(ResponseGate {
                entered: entered_tx,
                release: release_rx,
            })),
            shared: Arc::clone(&fixture.shared),
            balance_reads: AtomicUsize::new(0),
        });
        let router = Router::new()
            .route("/v1/users/me/access-tokens", post(refresh))
            .route("/v1/organizations/org-first/usage/balance", get(balance))
            .with_state(Arc::clone(&state));
        let listener = timeout(DEADLINE, tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .expect("local listener timed out")
            .expect("bind local listener");
        let address = listener.local_addr().expect("read local listener address");
        let api = CloudApi::for_test(&format!("http://{address}"))
            .expect("construct loopback-only test client");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve synthetic Cloud responses");
        });
        (
            Self { api, state, server },
            HeldRefresh {
                entered: entered_rx,
                release: release_tx,
            },
        )
    }

    fn assert_retained_is_live(&self, fixture: &Fixture) {
        let retained = fixture.retained();
        let tokens = self.state.tokens.lock().expect("synthetic token lock");
        assert!(
            retained.refresh_token == tokens.refresh,
            "the retained refresh token must still be accepted by the server"
        );
        assert!(retained.refresh_token_expires_at.is_some());
    }
}

impl Drop for LocalCloud {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

async fn refresh(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let response = {
        let mut tokens = state.tokens.lock().expect("synthetic token lock");
        if bearer(&headers) != Some(tokens.refresh.as_str()) {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "retired"})));
        }
        tokens.rotations += 1;
        tokens.refresh = format!("rt-synthetic-rotation-{}", tokens.rotations);
        tokens.access = format!("access-synthetic-{}", tokens.rotations);
        json!({
            "access_token": tokens.access,
            "refresh_token": tokens.refresh,
            "refresh_token_expiration": Utc::now() + chrono::Duration::hours(1)
        })
    };
    let gate = state
        .first_response
        .lock()
        .expect("response gate lock")
        .take();
    if let Some(gate) = gate {
        // The old refresh token is already retired. Delay only delivery of
        // the successful response, precisely where stale persistence races.
        gate.entered.send(()).expect("notify response barrier");
        timeout(DEADLINE, gate.release)
            .await
            .expect("held response was never released")
            .expect("held response controller disappeared");
    }
    (StatusCode::OK, Json(response))
}

async fn balance(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let (refresh, access) = {
        let tokens = state.tokens.lock().expect("synthetic token lock");
        (tokens.refresh.clone(), tokens.access.clone())
    };
    if bearer(&headers) != Some(access.as_str()) {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "access"})));
    }
    let memory = state
        .shared
        .settings
        .lock()
        .expect("fixture settings lock")
        .near_ai_session
        .clone();
    let disk = DaemonSettings::load_with_cloud_credentials(&state.shared.store)
        .expect("read settings before balance is served")
        .near_ai_session;
    if memory != disk || disk.is_none_or(|session| session.refresh_token != refresh) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "rotation_not_persisted"})),
        );
    }
    state.balance_reads.fetch_add(1, Ordering::SeqCst);
    (
        StatusCode::OK,
        Json(json!({
            "total_spent": 0, "remaining": 100, "spend_limit": 100,
            "total_requests": 0, "total_tokens": 0
        })),
    )
}

#[tokio::test]
async fn refresh_in_flight_cannot_restore_a_forgotten_session() {
    let fixture = Fixture::new();
    let initial = fixture.connect("first");
    let (cloud, held) = LocalCloud::start(&fixture, &initial).await;
    let (result, ()) = timeout(DEADLINE, async {
        tokio::join!(exchange(&fixture.shared, &cloud.api, &initial), async {
            let release = held.wait().await;
            let response = handle_forget(
                &fixture.shared,
                &Request {
                    id: 1,
                    method: "near_ai_credential_forget".to_owned(),
                    params: json!({}),
                },
            );
            assert!(
                response.error.is_none(),
                "forget must succeed at the barrier"
            );
            assert!(fixture.persisted().near_ai_session.is_none());
            release.send(()).expect("release stale refresh response");
        })
    })
    .await
    .expect("forget race timed out");

    let disk = fixture.persisted();
    assert!(
        disk.near_ai_inference.is_none(),
        "forget must remove the key"
    );
    assert!(
        disk.near_ai_session.is_none(),
        "a stale refresh must not restore a forgotten session on disk"
    );
    assert!(
        fixture
            .shared
            .settings
            .lock()
            .unwrap()
            .near_ai_session
            .is_none(),
        "a stale refresh must not restore a forgotten session in memory"
    );
    assert!(
        result.is_err(),
        "discard access from the forgotten connection"
    );
    // handle_forget leaves memory's inference key for proxy reconciliation;
    // its documented synchronous contract clears the retained session here.
}

#[tokio::test]
async fn refresh_in_flight_cannot_overwrite_a_replacement_account() {
    let fixture = Fixture::new();
    let initial = fixture.connect("first");
    let (cloud, held) = LocalCloud::start(&fixture, &initial).await;
    let (result, replacement) = timeout(DEADLINE, async {
        tokio::join!(exchange(&fixture.shared, &cloud.api, &initial), async {
            let release = held.wait().await;
            fixture.connect("replacement");
            let replacement = fixture.persisted();
            release.send(()).expect("release prior account response");
            replacement
        })
    })
    .await
    .expect("account replacement race timed out");

    let disk = fixture.persisted();
    let memory = fixture
        .shared
        .settings
        .lock()
        .expect("fixture settings lock");
    assert!(
        disk.near_ai_session == replacement.near_ai_session,
        "a stale refresh must not replace the new account's session on disk"
    );
    assert!(
        memory.near_ai_session == replacement.near_ai_session,
        "a stale refresh must not replace the new account's session in memory"
    );
    assert!(disk.near_ai_inference == replacement.near_ai_inference);
    assert!(memory.near_ai_inference == replacement.near_ai_inference);
    assert!(
        result.is_err(),
        "discard access from the replaced connection"
    );
}

#[tokio::test]
async fn rotation_is_persisted_before_returned_access_is_used() {
    let fixture = Fixture::new();
    let initial = fixture.connect("first");
    let (cloud, held) = LocalCloud::start(&fixture, &initial).await;
    let (result, ()) = timeout(DEADLINE, async {
        tokio::join!(exchange(&fixture.shared, &cloud.api, &initial), async {
            held.wait().await.send(()).expect("release valid rotation");
        })
    })
    .await
    .expect("valid rotation timed out");
    let access_token = result.expect("valid rotation must succeed").access_token;
    cloud.assert_retained_is_live(&fixture);
    let retained = fixture.retained();
    assert!(retained.refresh_token != initial.refresh_token);
    let balance = timeout(
        DEADLINE,
        cloud.api.organization_balance(
            &SessionTokens {
                access_token,
                refresh_token: retained.refresh_token,
            },
            "org-first",
        ),
    )
    .await
    .expect("balance request timed out")
    .expect("balance must observe the persisted rotation");
    assert_eq!(balance.remaining, Some(100));
    assert_eq!(cloud.state.balance_reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn overlapping_rotations_leave_the_retained_session_usable() {
    let fixture = Fixture::new();
    let initial = fixture.connect("first");
    let (cloud, held) = LocalCloud::start(&fixture, &initial).await;
    let (first, second, ()) = timeout(DEADLINE, async {
        tokio::join!(
            exchange(&fixture.shared, &cloud.api, &initial),
            exchange(&fixture.shared, &cloud.api, &initial),
            async {
                // Do not require both HTTP calls to arrive: a correct client
                // may serialize exchanges or reject the stale second caller.
                held.wait()
                    .await
                    .send(())
                    .expect("release overlapping rotation");
            }
        )
    })
    .await
    .expect("overlapping rotations timed out");
    assert!(
        first.is_ok() || second.is_ok(),
        "one valid rotation must survive"
    );
    cloud.assert_retained_is_live(&fixture);

    // Prove the retained token remains usable through the real client path,
    // rather than accepting equality between two stale local copies.
    let retained = fixture.retained();
    timeout(DEADLINE, exchange(&fixture.shared, &cloud.api, &retained))
        .await
        .expect("next retained-session exchange timed out")
        .expect("the retained session must support another exchange");
    cloud.assert_retained_is_live(&fixture);
}

#[cfg(unix)]
#[tokio::test]
async fn failed_rotation_save_withholds_access_and_preserves_memory() {
    use std::os::unix::fs::PermissionsExt;

    struct RestorePermissions(std::path::PathBuf, std::fs::Permissions);
    impl Drop for RestorePermissions {
        fn drop(&mut self) {
            std::fs::set_permissions(&self.0, self.1.clone()).unwrap();
        }
    }

    let fixture = Fixture::new();
    let initial = fixture.connect("first");
    let before = fixture.persisted();
    let dir = fixture.shared.store.dir();
    let _restore = RestorePermissions(
        dir.to_path_buf(),
        std::fs::metadata(dir).unwrap().permissions(),
    );
    let (cloud, held) = LocalCloud::start(&fixture, &initial).await;
    let (result, ()) = timeout(DEADLINE, async {
        tokio::join!(exchange(&fixture.shared, &cloud.api, &initial), async {
            let release = held.wait().await;
            // The existing settings stay readable, but a new atomic file
            // cannot be created after Cloud has already rotated the token.
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o500)).unwrap();
            assert!(std::fs::File::create(dir.join("write-must-fail")).is_err());
            assert!(DaemonSettings::load_with_cloud_credentials(&fixture.shared.store).is_ok());
            release.send(()).unwrap();
        })
    })
    .await
    .expect("failed publication timed out");

    assert_eq!(cloud.state.tokens.lock().unwrap().rotations, 1);
    assert!(
        result.is_err(),
        "unpersisted access must not escape exchange"
    );
    assert_eq!(fixture.persisted().near_ai_session, before.near_ai_session);
    assert_eq!(
        fixture.shared.settings.lock().unwrap().near_ai_session,
        before.near_ai_session
    );
    assert_eq!(cloud.state.balance_reads.load(Ordering::SeqCst), 0);
}
