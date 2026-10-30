//! Renewing an expired account session on a NEAR AI-provisioned account,
//! against one loopback stub that serves the NEAR AI token exchange, the
//! provisioning ceremony and the payout status route. Every token, id and
//! tenant here is synthetic.

use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine;
use chrono::Utc;
use serde_json::{Value, json};

use super::*;
use crate::daemon::ipc::{Request, Response};
use crate::daemon::nearai_onboarding::{PROVISION_FINISH_PATH, PROVISION_START_PATH};
use crate::daemon::settings::NearAiSession;

const ACCOUNT: &str = "6f1d2c3b-4a59-4e6f-8a7b-9c0d1e2f3a4b";
const NONCE: [u8; 32] = [7; 32];
const EXPIRED: &str = "tcn1_bmVhcmFp.expired-session";

fn tenant() -> String {
    format!("nearai-{}", "ab".repeat(32))
}

fn renewed_token() -> String {
    format!(
        "tcn1_{}.renewed-session",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tenant())
    )
}

/// What the stub answers the finish with, and what it saw.
struct Commons {
    /// Overrides the finish answer's tenant.
    tenant: Mutex<String>,
    /// Overrides the finish answer's account.
    account: Mutex<String>,
    /// Refuses every finish.
    refuse: Mutex<bool>,
    starts: Mutex<u32>,
    finishes: Mutex<u32>,
    refreshes: Mutex<u32>,
    /// What NEAR AI answers the token exchange with: 200 unless set.
    refresh_status: Mutex<u16>,
    /// How long the commons takes to answer the start, in milliseconds.
    start_delay_ms: Mutex<u64>,
    /// The bearer each payout status read presented.
    status_bearers: Mutex<Vec<String>>,
    /// Every other account route asked, with the bearer it was asked with.
    feature_calls: Mutex<Vec<(String, String)>>,
    /// The device key the ceremony must be signed by.
    device_public_key: String,
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned()
}

struct Stub {
    origin: String,
    state: Arc<Commons>,
}

async fn spawn(device_public_key: String) -> Stub {
    let state = Arc::new(Commons {
        tenant: Mutex::new(tenant()),
        account: Mutex::new(ACCOUNT.to_string()),
        refuse: Mutex::new(false),
        starts: Mutex::new(0),
        finishes: Mutex::new(0),
        refreshes: Mutex::new(0),
        refresh_status: Mutex::new(200),
        start_delay_ms: Mutex::new(0),
        status_bearers: Mutex::new(Vec::new()),
        feature_calls: Mutex::new(Vec::new()),
        device_public_key,
    });
    let expires = Utc::now().timestamp() + 300;
    let challenge: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let app = Router::new()
        .route(
            "/v1/users/me/access-tokens",
            post({
                let state = Arc::clone(&state);
                move || async move {
                    let mut n = state.refreshes.lock().unwrap();
                    *n += 1;
                    let status = *state.refresh_status.lock().unwrap();
                    if status != 200 {
                        // NEAR AI's refusal of a spent, revoked or
                        // agent-mismatched refresh token: a bare status.
                        return (StatusCode::from_u16(status).unwrap(), Json(json!({})));
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "access_token": format!("near-access-{n}"),
                            "refresh_token": format!("rt-synthetic-rotated-{n}"),
                            "refresh_token_expiration": (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
                        })),
                    )
                }
            }),
        )
        .route(
            PROVISION_START_PATH,
            post({
                let (state, challenge) = (Arc::clone(&state), Arc::clone(&challenge));
                move |Json(body): Json<Value>| async move {
                    *state.starts.lock().unwrap() += 1;
                    let delay = *state.start_delay_ms.lock().unwrap();
                    if delay > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                    }
                    assert_eq!(body["device_public_key"], state.device_public_key.as_str());
                    *challenge.lock().unwrap() = body["code_challenge"].as_str().unwrap().into();
                    Json(json!({
                        "ceremony_id": "renewal-ceremony",
                        "nonce": base64::engine::general_purpose::STANDARD.encode(NONCE),
                        "expires_at": expires,
                    }))
                }
            }),
        )
        .route(
            PROVISION_FINISH_PATH,
            post({
                let (state, challenge) = (Arc::clone(&state), Arc::clone(&challenge));
                move |Json(body): Json<Value>| async move {
                    *state.finishes.lock().unwrap() += 1;
                    // The ceremony a returning device runs is the first one,
                    // signed by the enrolled device key.
                    let public = base64::engine::general_purpose::STANDARD
                        .decode(&state.device_public_key)
                        .unwrap();
                    let device: [u8; 32] = public.clone().try_into().unwrap();
                    let preimage =
                        trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
                            &NONCE,
                            "renewal-ceremony",
                            &device,
                            &challenge.lock().unwrap(),
                            expires,
                        );
                    let signature = base64::engine::general_purpose::STANDARD
                        .decode(body["device_signature"].as_str().unwrap())
                        .unwrap();
                    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public)
                        .verify(&preimage, &signature)
                        .unwrap();
                    assert!(body["access_token"].as_str().unwrap().starts_with("near-access-"));
                    if *state.refuse.lock().unwrap() {
                        return (StatusCode::BAD_REQUEST, Json(json!({"error": "refused"})));
                    }
                    let device_key_id =
                        trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
                            &device,
                        );
                    (
                        StatusCode::OK,
                        Json(json!({
                            "access_token": renewed_token(),
                            "token_type": "Bearer",
                            "expires_in_secs": 43200,
                            "account_id": state.account.lock().unwrap().clone(),
                            "tenant_id": state.tenant.lock().unwrap().clone(),
                            "device_key_id": device_key_id,
                            "anchor_hash": format!("sha256:{}", "cd".repeat(32)),
                        })),
                    )
                }
            }),
        )
        .route(
            "/v1/account/payout-rails",
            get({
                let state = Arc::clone(&state);
                move |headers: HeaderMap| async move {
                    let presented = bearer(&headers);
                    state.status_bearers.lock().unwrap().push(presented.clone());
                    if presented != renewed_token() {
                        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})));
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "contract_version":
                                trace_commons_protocol::payout_link::PAYOUT_LINK_CONTRACT_VERSION,
                            "rails": [{
                                "rail_id": "near_ai", "mode": "live", "linked": false,
                                "recipient_ref": null, "state": null, "version": 0,
                                "request_state": null, "unlink_requested": false,
                                "recipient_unavailable": false,
                            }],
                        })),
                    )
                }
            }),
        )
        .fallback({
            let state = Arc::clone(&state);
            move |uri: axum::http::Uri, headers: HeaderMap| async move {
                let path = uri.path().to_string();
                let presented = bearer(&headers);
                state
                    .feature_calls
                    .lock()
                    .unwrap()
                    .push((path.clone(), presented.clone()));
                if presented != renewed_token() {
                    return (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"})));
                }
                feature_answer(&path)
            }
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Stub { origin, state }
}

/// What each feature's account route answers a renewed session with.
fn feature_answer(path: &str) -> (StatusCode, Json<Value>) {
    let body = match path {
        p if p.starts_with("/v1/account/traces/") && p.ends_with("/withdraw") => {
            json!({"distribution_reach": "not_distributed"})
        }
        "/v1/account/credit-summary" => json!({
            "points": {"earned_this_period": 7, "lifetime_earned": 30},
            "currency": {"code": "USD", "earned_this_period": "0.70"},
            "posture": {
                "settlement": "disabled",
                "graded": false,
                "explanation": "Credit is recorded but not settled.",
            },
            "period": {"start": "2026-08-01T00:00:00Z", "end": "2026-08-31T00:00:00Z"},
            "pending_review": 1,
        }),
        "/v1/account/activity-missions/status" => json!({
            "schema_version":1,"kind":"trace_activity","policy_sha256":"a".repeat(64),
            "observed_at":"2026-10-02T00:00:00Z","source":"account_contributed_submissions",
            "qualification":"accepted","coverage_starts_on":"2026-10-01","month_starts_on":"2026-10-01",
            "monthly_contributions":2,"daily":null,"completed_days":null,"current_streak":null,
            "level":null,"levels_configured":false,"badges":null,"rewards_enabled":false,
            "credit_points_pending":null,"credit_condition":"mission_credit_ledger_unavailable"
        }),
        "/v1/account/inference-connection/offers" => json!({
            "contract_version": "inference-connection-v1",
            "inference_connection_selection_required": true,
            "description": "server copy",
            "offers": [],
        }),
        "/v1/account/contribution-status" => {
            json!({"authority": "bounded", "ready": true, "policy_version": "v1"})
        }
        _ => return (StatusCode::NOT_FOUND, Json(json!({"error": "not_found"}))),
    };
    (StatusCode::OK, Json(body))
}

struct Fixture {
    shared: Arc<DaemonShared>,
    _dir: tempfile::TempDir,
    stub: Stub,
    api: CloudApi,
}

#[derive(Clone, Copy, PartialEq)]
enum Session {
    /// The record the 12-hour session leaves once it has run out.
    Expired,
    /// Signed out: no record at all.
    Absent,
}

impl Fixture {
    /// Enrolled through a NEAR AI login in a `nearai-` tenant, with the
    /// NEAR AI login retained.
    async fn new(session: Session) -> Self {
        Self::with_tenant(session, &tenant()).await
    }

    async fn with_tenant(session: Session, tenant_id: &str) -> Self {
        let (dir, store) = crate::config::tests_support::temp_store();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        let identity = DeviceIdentity::load_or_generate(&shared.store).unwrap();
        let stub = spawn(identity.public_key_b64.clone()).await;
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = stub.origin.clone();
        cfg.tenant_id = tenant_id.to_string();
        cfg.device_key_id = identity.device_key_id.clone();
        cfg.user_subject = identity.device_key_id.clone();
        shared.store.save_config(&cfg).unwrap();
        if session == Session::Expired {
            let record = json!({
                "access_token": EXPIRED,
                "expires_at": Utc::now() - chrono::Duration::hours(1),
                "account_id": ACCOUNT,
                "ingest_origin": stub.origin,
            });
            shared
                .store
                .write_daemon_file(
                    crate::config::ACCOUNT_SESSION_FILE,
                    &serde_json::to_vec(&record).unwrap(),
                )
                .unwrap();
        }
        {
            let now = Utc::now();
            let mut settings = shared.settings.lock().unwrap();
            settings.near_ai_session = Some(NearAiSession {
                refresh_token: "rt-synthetic-renewal".into(),
                refresh_token_expires_at: Some(now + chrono::Duration::hours(1)),
                stored_at: now,
                user_agent: "Mozilla/5.0 Test".into(),
            });
            settings.save_for_test(&shared.store).unwrap();
        }
        let api = CloudApi::for_test(&stub.origin).unwrap();
        // The route a feature call renews through: this stub, for this
        // state directory only.
        route_for_test(&shared.store, &stub.origin);
        Self {
            shared,
            _dir: dir,
            stub,
            api,
        }
    }

    fn via(&self) -> RenewVia<'_> {
        RenewVia::at(
            &self.api,
            super::super::native_identity::client(&self.stub.origin, "unauthenticated").unwrap(),
        )
    }

    fn ceremonies(&self) -> (u32, u32) {
        (
            *self.stub.state.starts.lock().unwrap(),
            *self.stub.state.finishes.lock().unwrap(),
        )
    }

    fn config(&self) -> Vec<u8> {
        std::fs::read(self.shared.store.daemon_path("contributor.json")).unwrap()
    }

    async fn payout_status(&self) -> Response {
        let req = Request {
            id: 3,
            method: "payout_rails_status".into(),
            params: json!({}),
        };
        super::super::payout_link::status(&self.shared, &req, Some(&self.via())).await
    }
}

impl Fixture {
    async fn call(&self, method: &str, params: Value) -> Response {
        let req = Request {
            id: 9,
            method: method.into(),
            params,
        };
        crate::daemon::ipc::handle_request_async(&self.shared, &req).await
    }

    /// The bearers the feature routes under `prefix` were asked with.
    fn feature_bearers(&self, prefix: &str) -> Vec<String> {
        self.stub
            .state
            .feature_calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(path, _)| path.starts_with(prefix))
            .map(|(_, bearer)| bearer.clone())
            .collect()
    }

    /// As the running daemon is: a request can start work that outlives
    /// it, on the daemon's own runtime.
    fn as_running_daemon(&self) {
        self.shared.adopt_runtime();
        self.shared.adopt_self();
    }

    fn refuse_near_ai_login(&self) {
        *self.stub.state.refresh_status.lock().unwrap() = 401;
    }

    fn renewal_log_text(&self) -> String {
        self.shared
            .store
            .read_daemon_file(RENEWAL_LOG_FILE)
            .unwrap()
            .map(|body| String::from_utf8(body).unwrap())
            .unwrap_or_default()
    }

    fn renewal_log(&self) -> Vec<Value> {
        self.renewal_log_text()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    async fn status(&self) -> Value {
        self.call("account_session_status", json!({}))
            .await
            .result
            .expect("status answered")
    }

    fn enable_missions(&self) {
        let mut settings = self.shared.settings.lock().unwrap();
        settings.missions_enabled = true;
        settings.save_for_test(&self.shared.store).unwrap();
    }
}

fn refusal(response: &Response) -> Option<&str> {
    response.error.as_ref().map(|error| error.message.as_str())
}

#[tokio::test]
async fn an_expired_session_on_a_near_ai_account_is_renewed_for_the_payout_status() {
    let fx = Fixture::new(Session::Expired).await;
    let config_before = fx.config();

    let response = fx.payout_status().await;

    assert_eq!(refusal(&response), None, "payout_rails_status refused");
    let rails = &response.result.as_ref().unwrap()["rails"];
    assert_eq!(rails[0]["rail_id"], "near_ai");
    assert_eq!(fx.ceremonies(), (1, 1));
    assert_eq!(
        *fx.stub.state.status_bearers.lock().unwrap(),
        vec![renewed_token()],
        "the expired token is never presented; the renewed one is"
    );
    // Only the session changed: the config is the one enrolled.
    assert_eq!(fx.config(), config_before);
    let loaded = crate::account_auth::try_load_session_with_snapshot(&fx.shared.store)
        .unwrap()
        .expect("a usable session is stored");
    assert_eq!(loaded.session.access_token, renewed_token());
    assert_eq!(loaded.session.account_id, ACCOUNT);
    assert!(loaded.session.expires_at > Utc::now() + chrono::Duration::hours(11));
    // The record's other fields are kept.
    let raw: Value = serde_json::from_slice(
        &super::super::commons_credentials::load(&fx.shared.store, Kind::Account)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(raw["ingest_origin"], fx.stub.origin.as_str());
}

#[tokio::test]
async fn a_signed_out_account_is_not_signed_back_in() {
    let fx = Fixture::new(Session::Absent).await;

    let response = fx.payout_status().await;

    assert_eq!(refusal(&response), Some("account-session-required"));
    assert_eq!(fx.ceremonies(), (0, 0), "nothing is asked of the commons");
    assert_eq!(*fx.stub.state.refreshes.lock().unwrap(), 0);
    assert!(fx.stub.state.status_bearers.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_account_in_another_tenant_is_not_renewed_this_way() {
    // A passkey account signs in again with its passkey.
    let fx = Fixture::with_tenant(Session::Expired, "tenant-passkey").await;

    let response = fx.payout_status().await;

    assert_eq!(refusal(&response), Some("account-session-required"));
    assert_eq!(fx.ceremonies(), (0, 0));
}

#[tokio::test]
async fn a_refused_renewal_stores_nothing_and_is_not_retried_at_once() {
    let fx = Fixture::new(Session::Expired).await;
    *fx.stub.state.refuse.lock().unwrap() = true;
    let config_before = fx.config();

    let first = fx.payout_status().await;
    let second = fx.payout_status().await;

    assert_eq!(refusal(&first), Some("account-session-required"));
    assert_eq!(refusal(&second), Some("account-session-required"));
    assert_eq!(
        fx.ceremonies(),
        (1, 1),
        "one attempt for one expired record"
    );
    assert_eq!(fx.config(), config_before);
    assert!(
        crate::account_auth::try_load_session_with_snapshot(&fx.shared.store)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn an_answer_for_another_account_or_tenant_is_not_stored() {
    for (tenant_id, account) in [
        (format!("nearai-{}", "cd".repeat(32)), ACCOUNT.to_string()),
        (tenant(), uuid::Uuid::new_v4().to_string()),
    ] {
        let fx = Fixture::new(Session::Expired).await;
        *fx.stub.state.tenant.lock().unwrap() = tenant_id;
        *fx.stub.state.account.lock().unwrap() = account;

        let response = fx.payout_status().await;

        assert_eq!(refusal(&response), Some("account-session-required"));
        assert_eq!(fx.ceremonies(), (1, 1));
        assert!(
            crate::account_auth::try_load_session_with_snapshot(&fx.shared.store)
                .unwrap()
                .is_none()
        );
    }
}

#[tokio::test]
async fn two_calls_that_arrive_together_renew_once() {
    let fx = Fixture::new(Session::Expired).await;
    let via = fx.via();

    let (a, b) = tokio::join!(
        account_session(&fx.shared, Some(&via)),
        account_session(&fx.shared, Some(&via)),
    );

    assert_eq!(a.unwrap().unwrap().session.access_token, renewed_token());
    assert_eq!(b.unwrap().unwrap().session.access_token, renewed_token());
    assert_eq!(fx.ceremonies(), (1, 1));
}

#[tokio::test]
async fn without_a_renewal_route_an_expired_session_is_still_required() {
    let fx = Fixture::new(Session::Expired).await;

    assert!(account_session(&fx.shared, None).await.unwrap().is_none());
    assert_eq!(fx.ceremonies(), (0, 0));
}

#[tokio::test]
async fn a_new_near_ai_sign_in_lifts_the_wait_after_a_refused_renewal() {
    let fx = Fixture::new(Session::Expired).await;
    *fx.stub.state.refuse.lock().unwrap() = true;
    assert_eq!(
        refusal(&fx.payout_status().await),
        Some("account-session-required")
    );

    // The person signs in to NEAR AI again: a new login, not the one the
    // refused attempt spent.
    {
        let now = Utc::now() + chrono::Duration::seconds(1);
        let mut settings = fx.shared.settings.lock().unwrap();
        settings.near_ai_session = Some(NearAiSession {
            refresh_token: "rt-synthetic-signed-in-again".into(),
            refresh_token_expires_at: Some(now + chrono::Duration::hours(1)),
            stored_at: now,
            user_agent: "Mozilla/5.0 Test".into(),
        });
        settings.save_for_test(&fx.shared.store).unwrap();
    }
    *fx.stub.state.refuse.lock().unwrap() = false;

    let response = fx.payout_status().await;

    assert_eq!(refusal(&response), None);
    assert_eq!(fx.ceremonies(), (2, 2));
}

// Every feature that presents the account session renews it the same way
// (#1603's follow-ups): withdrawal, Trace Credit, activity missions, the
// inference connection, the public run and account contributions.

/// Each feature method, its params, and the route prefix it presents the
/// session to.
fn feature_methods() -> Vec<(&'static str, Value, &'static str)> {
    let submission = uuid::Uuid::new_v4().to_string();
    vec![
        (
            "withdraw",
            json!({"submission_id": submission}),
            "/v1/account/traces/",
        ),
        (
            "commons_credit_summary",
            json!({}),
            "/v1/account/credit-summary",
        ),
        (
            "activity_missions_status",
            json!({}),
            "/v1/account/activity-missions/status",
        ),
        (
            "inference_connection_offers",
            json!({}),
            "/v1/account/inference-connection/offers",
        ),
        (
            "history_detail",
            json!({"submission_id": submission}),
            "/v1/account/traces/",
        ),
        (
            "account_contribution_status",
            json!({}),
            "/v1/account/contribution-status",
        ),
    ]
}

#[tokio::test]
async fn every_feature_renews_an_expired_session_and_presents_the_new_one() {
    for (method, params, route) in feature_methods() {
        let fx = Fixture::new(Session::Expired).await;
        fx.enable_missions();
        let config_before = fx.config();

        let response = fx.call(method, params).await;

        assert_ne!(
            refusal(&response),
            Some("account-session-required"),
            "{method} was refused for want of a session"
        );
        assert_eq!(fx.ceremonies(), (1, 1), "{method} renewed once");
        let bearers = fx.feature_bearers(route);
        assert!(!bearers.is_empty(), "{method} never reached {route}");
        assert!(
            bearers.iter().all(|b| *b == renewed_token()),
            "{method} presented the expired session"
        );
        assert_eq!(fx.config(), config_before, "{method} rewrote the config");
        match method {
            "withdraw" => {
                assert_eq!(response.result.as_ref().unwrap()["withdrawn"], true);
            }
            "commons_credit_summary" => {
                assert_eq!(response.result.as_ref().unwrap()["points_state"], "known");
            }
            // The stub has no such trace; the point is that it was asked.
            "history_detail" => {
                assert_eq!(refusal(&response), Some("session-detail-not-found"));
            }
            _ => assert_eq!(refusal(&response), None, "{method} refused"),
        }
    }
}

#[tokio::test]
async fn every_feature_leaves_a_signed_out_account_signed_out() {
    for (method, params, _) in feature_methods() {
        let fx = Fixture::new(Session::Absent).await;
        fx.enable_missions();

        let _ = fx.call(method, params).await;

        assert_eq!(fx.ceremonies(), (0, 0), "{method} signed the account in");
        assert_eq!(*fx.stub.state.refreshes.lock().unwrap(), 0, "{method}");
        assert!(
            fx.stub.state.feature_calls.lock().unwrap().is_empty(),
            "{method} presented a session"
        );
    }
}

#[tokio::test]
async fn the_session_status_reports_signed_in_once_it_is_renewed() {
    let fx = Fixture::new(Session::Expired).await;

    let first = fx.call("account_session_status", json!({})).await;
    let second = fx.call("account_session_status", json!({})).await;

    for response in [&first, &second] {
        let status = response.result.as_ref().expect("status answered");
        assert_eq!(status["state"], "known");
        assert_eq!(status["signed_in"], true);
        let expires: chrono::DateTime<Utc> =
            serde_json::from_value(status["expires_at"].clone()).unwrap();
        assert!(expires > Utc::now() + chrono::Duration::hours(11));
    }
    assert_eq!(
        fx.ceremonies(),
        (1, 1),
        "a status read renews only an expired session"
    );
}

#[tokio::test]
async fn the_session_status_of_a_signed_out_or_passkey_account_renews_nothing() {
    for fx in [
        Fixture::new(Session::Absent).await,
        Fixture::with_tenant(Session::Expired, "tenant-passkey").await,
    ] {
        let status = fx.call("account_session_status", json!({})).await;

        assert_eq!(status.result.as_ref().unwrap()["signed_in"], false);
        assert_eq!(fx.ceremonies(), (0, 0));
    }
}

#[tokio::test]
async fn a_refused_renewal_is_not_retried_by_another_feature() {
    let fx = Fixture::new(Session::Expired).await;
    fx.enable_missions();
    *fx.stub.state.refuse.lock().unwrap() = true;

    let status = fx.call("account_session_status", json!({})).await;
    assert_eq!(fx.ceremonies(), (1, 1), "the status read attempted it");
    let missions = fx.call("activity_missions_status", json!({})).await;
    let payout = fx.payout_status().await;

    assert_eq!(status.result.as_ref().unwrap()["signed_in"], false);
    assert_eq!(refusal(&missions), Some("account-session-required"));
    assert_eq!(refusal(&payout), Some("account-session-required"));
    assert_eq!(
        fx.ceremonies(),
        (1, 1),
        "neither retried during the backoff"
    );
}

#[tokio::test]
async fn features_that_arrive_together_renew_once() {
    let fx = Fixture::new(Session::Expired).await;
    fx.enable_missions();

    let (status, credit, missions, offers, payout) = tokio::join!(
        fx.call("account_session_status", json!({})),
        fx.call("commons_credit_summary", json!({})),
        fx.call("activity_missions_status", json!({})),
        fx.call("inference_connection_offers", json!({})),
        fx.payout_status(),
    );

    assert_eq!(status.result.as_ref().unwrap()["signed_in"], true);
    assert_eq!(credit.result.as_ref().unwrap()["points_state"], "known");
    assert_eq!(refusal(&missions), None);
    assert_eq!(refusal(&offers), None);
    assert_eq!(refusal(&payout), None);
    assert_eq!(fx.ceremonies(), (1, 1));
}

/// Automatic upload's admission read (R3) runs on the supervisor's pass and
/// never waits on a ceremony: the pass asks for a renewal off the pass, and
/// the next pass reads the renewed session.
#[tokio::test]
async fn the_pass_renews_off_the_pass_and_only_once_per_wait() {
    let fx = Fixture::new(Session::Expired).await;

    let renewal = renew_off_pass(&fx.shared).expect("a renewal is started");
    // A second pass while it runs, or soon after, starts nothing.
    assert!(renew_off_pass(&fx.shared).is_none());
    renewal.await.unwrap();

    assert_eq!(fx.ceremonies(), (1, 1));
    assert_eq!(
        crate::account_auth::try_load_session_with_snapshot(&fx.shared.store)
            .unwrap()
            .unwrap()
            .session
            .access_token,
        renewed_token()
    );
}

#[tokio::test]
async fn the_admission_read_asks_for_a_renewal_when_it_finds_no_session() {
    let fx = Fixture::new(Session::Expired).await;
    let key = super::super::policy::project_key_for(Some(&super::super::test_paths::abs(
        "tmp/armed-project",
    )));
    fx.shared
        .policy
        .lock()
        .unwrap()
        .set_mode(
            &key,
            super::super::policy::ProjectMode::AutoUpload,
            Utc::now(),
        )
        .unwrap();

    super::super::account_admission::refresh(&fx.shared, Utc::now(), false).await;

    assert!(fx.shared.account_admission.take_session_missing());
    assert!(
        !fx.shared.account_admission.take_session_missing(),
        "taken once"
    );
}

// The staging failure (2026-10-24): an expired session on a NEAR AI
// account, the NEAR AI login still stored, and NEAR AI refusing it. The
// commons saw a start and never a finish, and nothing on the Mac said why.

#[tokio::test]
async fn a_refused_near_ai_login_stops_after_the_start_and_asks_for_a_near_ai_sign_in() {
    let fx = Fixture::new(Session::Expired).await;
    fx.refuse_near_ai_login();

    let payout = fx.payout_status().await;

    assert_eq!(refusal(&payout), Some("account-session-required"));
    assert_eq!(
        fx.ceremonies(),
        (1, 0),
        "start answered, finish never sent: what the staging commons logged"
    );
    let status = fx.status().await;
    assert_eq!(status["signed_in"], false);
    assert_eq!(status["renewal"], "failed");
    assert_eq!(status["renewal_failure"], "near_ai_sign_in_required");
    assert_eq!(status["sign_in"], "near_ai");
    assert_eq!(fx.ceremonies(), (1, 0), "not retried silently");
    let log = fx.renewal_log();
    assert_eq!(log.len(), 1, "one attempt, one line");
    assert_eq!(log[0]["outcome"], "failed");
    assert_eq!(log[0]["failure"], "near_ai_sign_in_required");
    assert_eq!(log[0]["trigger"], "automatic");
    assert!(log[0]["at"].is_string());
}

#[tokio::test]
async fn each_failure_is_logged_by_its_class_and_never_by_its_secrets() {
    let fx = Fixture::new(Session::Expired).await;
    *fx.stub.state.refuse.lock().unwrap() = true;

    let _ = fx.payout_status().await;

    assert_eq!(fx.ceremonies(), (1, 1));
    let log = fx.renewal_log();
    assert_eq!(log[0]["failure"], "commons_refused");
    let text = fx.renewal_log_text();
    for secret in [
        "rt-synthetic",
        "near-access-",
        "tcn1_",
        ACCOUNT,
        fx.stub.origin.as_str(),
        tenant().as_str(),
    ] {
        assert!(!text.contains(secret), "the renewal log carries {secret}");
    }
}

#[tokio::test]
async fn a_renewal_is_logged_when_it_succeeds() {
    let fx = Fixture::new(Session::Expired).await;

    assert_eq!(refusal(&fx.payout_status().await), None);

    let log = fx.renewal_log();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0]["outcome"], "renewed");
    assert!(log[0]["failure"].is_null());
}

// The freeze: the status read waited on the whole ceremony.

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_renewal_does_not_hold_the_status_read() {
    let fx = Fixture::new(Session::Expired).await;
    fx.as_running_daemon();
    *fx.stub.state.start_delay_ms.lock().unwrap() = 6_000;

    let mut reads = Vec::new();
    for _ in 0..2 {
        let started = std::time::Instant::now();
        reads.push(fx.status().await);
        let waited = started.elapsed();
        // The read's own wait, plus slack for a loaded test machine; the
        // renewal takes six seconds.
        assert!(
            waited < READ_WAIT + std::time::Duration::from_millis(1_500),
            "a status read took {waited:?} behind a renewal"
        );
    }
    for status in &reads {
        assert_eq!(status["state"], "known");
        assert_eq!(status["signed_in"], false);
        assert_eq!(status["renewal"], "renewing");
    }
    // The renewal carries on without anyone waiting for it.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let status = fx.status().await;
        if status["signed_in"] == true {
            assert!(status["renewal"].is_null());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the renewal never finished"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(fx.ceremonies(), (1, 1), "the reads started one renewal");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_renewal_does_not_hold_the_home_reads() {
    let fx = Fixture::new(Session::Expired).await;
    fx.as_running_daemon();
    fx.enable_missions();
    // The renewal stalls far longer than the bound below, so the bound can
    // leave room for a loaded CI runner (2.96s seen against the old 2.5s
    // bound) and still fail any read that waited for the renewal.
    *fx.stub.state.start_delay_ms.lock().unwrap() = 20_000;

    let started = std::time::Instant::now();
    let (payout, credit, missions) = tokio::join!(
        fx.call("payout_rails_status", json!({})),
        fx.call("commons_credit_summary", json!({})),
        fx.call("activity_missions_status", json!({})),
    );
    let waited = started.elapsed();

    assert!(
        waited < READ_WAIT + std::time::Duration::from_secs(5),
        "the Home reads took {waited:?} behind a renewal"
    );
    assert_eq!(refusal(&payout), Some("account-session-required"));
    assert_ne!(credit.result.as_ref().unwrap()["points_state"], "known");
    assert_eq!(refusal(&missions), Some("account-session-required"));
}

// "No obvious way to sign in": Sign in again, for a NEAR AI account.

async fn sign_in_again(fx: &Fixture) -> Response {
    fx.call("near_ai_account_sign_in", json!({})).await
}

#[tokio::test]
async fn sign_in_again_renews_even_during_the_backoff() {
    let fx = Fixture::new(Session::Expired).await;
    *fx.stub.state.refuse.lock().unwrap() = true;
    assert_eq!(
        refusal(&fx.payout_status().await),
        Some("account-session-required")
    );
    *fx.stub.state.refuse.lock().unwrap() = false;

    let response = sign_in_again(&fx).await;

    assert_eq!(refusal(&response), None);
    let answer = response.result.unwrap();
    assert_eq!(answer["signed_in"], true);
    assert!(answer.get("access_token").is_none());
    assert_eq!(fx.ceremonies(), (2, 2), "the person's click runs it again");
    let status = fx.status().await;
    assert_eq!(status["signed_in"], true);
    assert!(
        status["renewal"].is_null(),
        "the earlier failure is cleared"
    );
    assert_eq!(fx.renewal_log().last().unwrap()["trigger"], "sign_in");
}

#[tokio::test]
async fn sign_in_again_signs_a_signed_out_near_ai_account_in() {
    let fx = Fixture::new(Session::Absent).await;
    assert_eq!(fx.status().await["sign_in"], "near_ai");
    let config_before = fx.config();

    let response = sign_in_again(&fx).await;

    assert_eq!(refusal(&response), None);
    assert_eq!(fx.ceremonies(), (1, 1));
    let loaded = crate::account_auth::try_load_session_with_snapshot(&fx.shared.store)
        .unwrap()
        .expect("a session is stored");
    assert_eq!(loaded.session.access_token, renewed_token());
    assert_eq!(loaded.session.account_id, ACCOUNT);
    assert_eq!(fx.config(), config_before, "the enrollment is untouched");
}

#[tokio::test]
async fn sign_in_again_with_a_refused_near_ai_login_asks_for_a_near_ai_sign_in() {
    let fx = Fixture::new(Session::Expired).await;
    fx.refuse_near_ai_login();

    let response = sign_in_again(&fx).await;

    assert_eq!(refusal(&response), Some("near_ai_sign_in_required"));
    assert_eq!(fx.ceremonies(), (1, 0));
    assert!(
        crate::account_auth::try_load_session_with_snapshot(&fx.shared.store)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn sign_in_again_without_a_near_ai_login_asks_for_one_and_spends_nothing() {
    let fx = Fixture::new(Session::Expired).await;
    fx.shared.settings.lock().unwrap().near_ai_session = None;

    let response = sign_in_again(&fx).await;

    assert_eq!(refusal(&response), Some("near_ai_sign_in_required"));
    assert_eq!(fx.ceremonies(), (0, 0));
}

#[tokio::test]
async fn sign_in_again_is_not_offered_to_a_passkey_account() {
    let fx = Fixture::with_tenant(Session::Expired, "tenant-passkey").await;

    assert!(fx.status().await["sign_in"].is_null());
    let response = sign_in_again(&fx).await;

    assert_eq!(
        refusal(&response),
        Some("near_ai_sign_in_not_near_ai_account")
    );
    assert_eq!(fx.ceremonies(), (0, 0));
}

#[tokio::test]
async fn enrolling_an_enrolled_device_is_still_refused() {
    // Sign in again is the path back; enrollment stays a one-time step.
    let fx = Fixture::new(Session::Expired).await;

    let response = fx
        .call(
            "near_ai_account_enroll",
            json!({"ingest_url": "https://commons.example"}),
        )
        .await;

    assert_eq!(refusal(&response), Some("near_ai_enroll_already_enrolled"));
    assert_eq!(fx.ceremonies(), (0, 0));
}

#[tokio::test]
async fn a_revoked_key_runs_no_further_ceremony_and_a_new_key_is_not_held_back() {
    let fx = Fixture::new(Session::Expired).await;
    let enrolled = fx
        .shared
        .store
        .load_config()
        .unwrap()
        .unwrap()
        .device_key_id;
    super::remember_if_revoked(
        &fx.shared.store,
        super::FAILURE_DEVICE_KEY_REVOKED,
        &enrolled,
    );

    let response = sign_in_again(&fx).await;

    assert_eq!(refusal(&response), Some(super::FAILURE_DEVICE_KEY_REVOKED));
    assert_eq!(fx.ceremonies(), (0, 0), "no start, no finish");
    assert!(!super::may_renew(&fx.shared));
    assert_eq!(
        super::last_failure(&fx.shared),
        Some(super::FAILURE_DEVICE_KEY_REVOKED)
    );

    // Sign-out keeps the device key, so the key stays revoked.
    let _ = fx.call("account_sign_out", json!({})).await;
    assert!(super::key_revoked(&fx.shared.store), "sign-out keeps it");

    // A late answer for an old key, after the device enrolled a new one,
    // does not hold the new key back.
    super::remember_if_revoked(
        &fx.shared.store,
        super::FAILURE_DEVICE_KEY_REVOKED,
        "an-older-key",
    );
    assert!(!super::key_revoked(&fx.shared.store));
}
