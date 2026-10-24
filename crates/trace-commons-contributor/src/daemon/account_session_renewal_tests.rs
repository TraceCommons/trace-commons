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
    /// The bearer each payout status read presented.
    status_bearers: Mutex<Vec<String>>,
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
        status_bearers: Mutex::new(Vec::new()),
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
                    Json(json!({
                        "access_token": format!("near-access-{n}"),
                        "refresh_token": format!("rt-synthetic-rotated-{n}"),
                        "refresh_token_expiration": (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
                    }))
                }
            }),
        )
        .route(
            PROVISION_START_PATH,
            post({
                let (state, challenge) = (Arc::clone(&state), Arc::clone(&challenge));
                move |Json(body): Json<Value>| async move {
                    *state.starts.lock().unwrap() += 1;
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
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Stub { origin, state }
}

struct Fixture {
    shared: DaemonShared,
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
        let shared = DaemonShared::load(store).unwrap();
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
