//! The change of login (F10), with its browser legs' outcomes recorded
//! directly and a loopback stub as the commons. Every token, id and tenant
//! here is synthetic.

use std::sync::{Arc, Mutex};

use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use base64::Engine;
use chrono::Utc;
use serde_json::{Value, json};

use super::*;
use crate::daemon::ipc::{Request, Response};
use trace_commons_protocol::account_login_change::IdentityKind;

const ACCOUNT: &str = "6f1d2c3b-4a59-4e6f-8a7b-9c0d1e2f3a4b";
const SESSION: &str = "tcn1_bmVhcmFp.live-session";
const NONCE: [u8; 32] = [7; 32];
const OLD_ACCESS: &str = "near-access-old-login";
const NEW_ACCESS: &str = "near-access-new-login";

fn near_ai_tenant() -> String {
    format!("nearai-{}", "ab".repeat(32))
}

fn wallet_tenant() -> String {
    format!("near-{}", "cd".repeat(32))
}

/// What the stub commons answers, and what it saw.
#[derive(Default)]
struct Commons {
    starts: Mutex<Vec<Value>>,
    finishes: Mutex<Vec<Value>>,
    bearers: Mutex<Vec<String>>,
    /// `(status, label)` the finish refuses with, when set.
    refuse: Mutex<Option<(u16, &'static str)>>,
    /// The start answers this kind.
    kind: Mutex<Option<IdentityKind>>,
    tenant: Mutex<String>,
}

struct Fixture {
    shared: Arc<DaemonShared>,
    _dir: tempfile::TempDir,
    commons: Arc<Commons>,
    origin: String,
    identity: DeviceIdentity,
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned()
}

async fn spawn(commons: Arc<Commons>) -> String {
    use axum::response::IntoResponse as _;
    let app = Router::new()
        .route(
            wire::REBIND_START_PATH,
            post({
                let commons = Arc::clone(&commons);
                move |headers: HeaderMap, Json(body): Json<Value>| async move {
                    commons.bearers.lock().unwrap().push(bearer(&headers));
                    commons.starts.lock().unwrap().push(body);
                    let kind = commons
                        .kind
                        .lock()
                        .unwrap()
                        .unwrap_or(IdentityKind::NearAiLogin);
                    Json(json!({
                        "ceremony_id": format!("ceremony-{}", commons.starts.lock().unwrap().len()),
                        "nonce": base64::engine::general_purpose::STANDARD.encode(NONCE),
                        "expires_at": Utc::now().timestamp() + 900,
                        "current_kind": kind,
                    }))
                    .into_response()
                }
            }),
        )
        .route(
            wire::REBIND_FINISH_PATH,
            post({
                let commons = Arc::clone(&commons);
                move |headers: HeaderMap, Json(body): Json<Value>| async move {
                    commons.bearers.lock().unwrap().push(bearer(&headers));
                    commons.finishes.lock().unwrap().push(body);
                    if let Some((status, label)) = *commons.refuse.lock().unwrap() {
                        return (
                            StatusCode::from_u16(status).unwrap(),
                            Json(json!({ "error": label })),
                        )
                            .into_response();
                    }
                    Json(json!({
                        "tenant_id": commons.tenant.lock().unwrap().clone(),
                        "account_id": ACCOUNT,
                        "anchor_hash": format!("sha256:{}", "ef".repeat(32)),
                        "auth_provider": "github",
                        "identity": "near_ai_login",
                        "switched_back": false,
                    }))
                    .into_response()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    origin
}

impl Fixture {
    async fn new(tenant: &str, kind: Option<IdentityKind>) -> Self {
        let (dir, store) = crate::config::tests_support::temp_store();
        let shared = Arc::new(DaemonShared::load(store).unwrap());
        let identity = DeviceIdentity::load_or_generate(&shared.store).unwrap();
        let commons = Arc::new(Commons::default());
        *commons.tenant.lock().unwrap() = tenant.to_string();
        *commons.kind.lock().unwrap() = kind.or(Some(IdentityKind::NearAiLogin));
        let origin = spawn(Arc::clone(&commons)).await;
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = origin.clone();
        cfg.tenant_id = tenant.to_string();
        cfg.device_key_id = identity.device_key_id.clone();
        cfg.user_subject = identity.device_key_id.clone();
        cfg.account_identity = kind;
        shared.store.save_config(&cfg).unwrap();
        let record = json!({
            "access_token": SESSION,
            "expires_at": Utc::now() + chrono::Duration::hours(6),
            "account_id": ACCOUNT,
            "ingest_origin": origin,
        });
        shared
            .store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&record).unwrap(),
            )
            .unwrap();
        Self {
            shared,
            _dir: dir,
            commons,
            origin,
            identity,
        }
    }

    async fn call(&self, method: &str, params: Value) -> Response {
        let req = Request {
            id: 7,
            method: method.into(),
            params,
        };
        crate::daemon::ipc::handle_request_async(&self.shared, &req).await
    }

    async fn step(&self, step: &str, extra: Value) -> Response {
        let mut params = json!({ "step": step });
        if let (Some(params), Value::Object(extra)) = (params.as_object_mut(), extra) {
            params.extend(extra);
        }
        self.call("account_login_change", params).await
    }

    async fn confirm(&self, change_id: &str) -> Value {
        let req = Request {
            id: 8,
            method: "account_login_change".into(),
            params: json!({ "step": "confirm", "change_id": change_id, "confirmed": true }),
        };
        let api = CloudApi::for_test(&self.origin).unwrap();
        let response = confirm(&self.shared, &req, Some(change_id), &api).await;
        assert!(
            response.error.is_none(),
            "{:?}",
            response.error.map(|e| e.message)
        );
        response.result.unwrap()
    }

    fn dir(&self) -> &Path {
        self.shared.store.dir()
    }

    fn status(&self) -> Value {
        let cfg = self.shared.store.load_config().unwrap();
        status_fields(&self.shared.store, cfg.as_ref())
    }
}

fn minted(key_id: &str) -> MintedKey {
    MintedKey {
        key: format!("sk-synthetic-{key_id}"),
        key_id: key_id.into(),
        key_prefix: "sk-syn".into(),
        organization_id: "org-new".into(),
        workspace_id: "ws-new".into(),
    }
}

fn held(provider: &'static str, identity: Option<&str>) -> HeldLogin {
    HeldLogin {
        tokens: SessionTokens {
            access_token: NEW_ACCESS.into(),
            refresh_token: "rt-synthetic-new".into(),
        },
        expires_at: None,
        user_agent: "Mozilla/5.0 Test".into(),
        provider: Some(provider),
        minted: minted("key-new"),
        identity: identity.map(str::to_owned),
        unstored: UnstoredKey::disarmed_for_test(),
    }
}

fn result(response: &Response) -> &Value {
    assert!(
        response.error.is_none(),
        "{:?}",
        response.error.as_ref().map(|e| e.message.as_str())
    );
    response.result.as_ref().unwrap()
}

fn refusal(response: &Response) -> Option<&str> {
    response.error.as_ref().map(|error| error.message.as_str())
}

#[tokio::test]
async fn status_offers_the_change_in_both_account_namespaces_only() {
    let near_ai = Fixture::new(&near_ai_tenant(), None).await;
    let status = near_ai.status();
    assert_eq!(status["login_change_available"], true);
    assert_eq!(status["payouts_need_login"], false);
    assert_eq!(status["login_change_held"], false);

    let wallet = Fixture::new(&wallet_tenant(), None).await;
    let status = wallet.status();
    assert_eq!(status["login_change_available"], true);
    assert_eq!(
        status["payouts_need_login"], true,
        "a wallet-origin account's payouts need a NEAR AI login"
    );

    // After a change, the payouts line clears.
    let changed = Fixture::new(&wallet_tenant(), Some(IdentityKind::NearAiLogin)).await;
    assert_eq!(changed.status()["payouts_need_login"], false);

    let invite = Fixture::new("tenant-invite", None).await;
    let status = invite.status();
    assert_eq!(status["login_change_available"], false);
    assert_eq!(status["payouts_need_login"], false);
    assert_eq!(
        refusal(&invite.step("start", json!({})).await),
        Some(ERR_NOT_AVAILABLE)
    );
}

#[tokio::test]
async fn a_device_that_joined_by_code_cannot_start_a_change() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    fx.shared
        .store
        .write_daemon_file(crate::config::JOINED_BY_CODE_FILE, b"1\n")
        .unwrap();
    assert_eq!(fx.status()["login_change_available"], false);
    assert_eq!(
        refusal(&fx.step("start", json!({})).await),
        Some(ERR_NOT_AVAILABLE)
    );
}

#[tokio::test]
async fn a_refused_login_is_held_aside_and_skips_signing_in_again() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    assert!(hold_aside(fx.dir(), held("github", Some("sha256:new"))).is_none());
    assert_eq!(fx.status()["login_change_held"], true);

    let started = fx.step("start", json!({})).await;
    let view = result(&started);
    assert_eq!(
        view["step"], "current",
        "the current identity is proved first"
    );
    assert_eq!(view["new_provider"], "github");
    assert_eq!(view["current_kind"], "near_ai_login");
    assert_eq!(
        fx.status()["login_change_held"],
        false,
        "moved into the ceremony"
    );

    let id = view["change_id"].as_str().unwrap().to_owned();
    record_current_near_ai(fx.dir(), &id, Some((OLD_ACCESS.into(), None)));
    // Still at current: the step was not claimed. Claim it the way the IPC
    // call does, then record.
    assert_eq!(
        result(&fx.step("status", json!({ "change_id": id })).await)["step"],
        "current"
    );
}

#[tokio::test]
async fn hold_aside_answers_the_login_back_where_the_change_is_not_offered() {
    let fx = Fixture::new("tenant-invite", None).await;
    assert!(hold_aside(fx.dir(), held("github", None)).is_some());
    assert_eq!(fx.status()["login_change_held"], false);
}

/// Drive a step's outcome as its browser leg would: claim it, then record.
fn prove_near_ai(fx: &Fixture, id: &str, identity: Option<&str>) {
    claim(fx.dir(), Some(id), Step::Current).unwrap();
    record_current_near_ai(
        fx.dir(),
        id,
        Some((OLD_ACCESS.into(), identity.map(str::to_owned))),
    );
}

fn sign_in_new_login(fx: &Fixture, id: &str, login: HeldLogin) {
    claim(fx.dir(), Some(id), Step::New).unwrap();
    record_new_login(fx.dir(), id, Some(Ok(login)));
}

async fn started(fx: &Fixture) -> String {
    let started = fx.step("start", json!({})).await;
    result(&started)["change_id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn a_current_sign_in_with_another_login_is_refused_current_login() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    super::super::nearai_credential::spend_org::record_identity(
        &fx.shared.store,
        "key-old",
        "sha256:old",
    );
    let id = started(&fx).await;

    prove_near_ai(&fx, &id, Some("sha256:someone-else"));

    let view = result(&fx.step("status", json!({ "change_id": id })).await).clone();
    assert_eq!(view["step"], "current");
    assert_eq!(view["error"], STEP_ERROR_CURRENT_LOGIN);

    prove_near_ai(&fx, &id, Some("sha256:old"));
    let view = result(&fx.step("status", json!({ "change_id": id })).await).clone();
    assert_eq!(view["step"], "new");
    assert!(view["error"].is_null());
}

#[tokio::test]
async fn the_new_login_must_not_be_the_current_one() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    super::super::nearai_credential::spend_org::record_identity(
        &fx.shared.store,
        "key-old",
        "sha256:old",
    );
    let id = started(&fx).await;
    prove_near_ai(&fx, &id, Some("sha256:old"));

    sign_in_new_login(&fx, &id, held("google", Some("sha256:old")));

    let view = result(&fx.step("status", json!({ "change_id": id })).await).clone();
    assert_eq!(view["step"], "new");
    assert_eq!(view["error"], STEP_ERROR_IN_USE);
}

#[tokio::test]
async fn steps_out_of_order_are_refused() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    let id = started(&fx).await;
    assert_eq!(
        refusal(&fx.step("sign_in_new", json!({ "change_id": id })).await),
        Some(ERR_OUT_OF_ORDER)
    );
    assert_eq!(
        refusal(
            &fx.step("confirm", json!({ "change_id": id, "confirmed": true }))
                .await
        ),
        Some(ERR_OUT_OF_ORDER)
    );
    assert_eq!(
        refusal(&fx.step("status", json!({ "change_id": "another" })).await),
        Some(ERR_NO_CEREMONY)
    );
    assert_eq!(refusal(&fx.step("fly", json!({})).await), Some("step"));
}

#[tokio::test]
async fn a_near_ai_change_finishes_with_both_proofs_and_rebinds_this_device() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    let id = started(&fx).await;
    prove_near_ai(&fx, &id, None);
    sign_in_new_login(&fx, &id, held("github", Some("sha256:new")));
    assert_eq!(
        result(&fx.step("status", json!({ "change_id": id })).await)["step"],
        "confirm"
    );

    let view = fx.confirm(&id).await;

    assert_eq!(view["step"], "done", "{view}");
    assert!(view["error"].is_null());
    // The commons' start and finish ran back to back, on the account
    // session, with the device's signature over both proofs.
    let starts = fx.commons.starts.lock().unwrap().clone();
    let finishes = fx.commons.finishes.lock().unwrap().clone();
    assert_eq!((starts.len(), finishes.len()), (1, 1));
    assert!(starts[0].get("wallet_account_id").is_none());
    assert!(
        fx.commons
            .bearers
            .lock()
            .unwrap()
            .iter()
            .all(|b| b == SESSION)
    );
    let finish = &finishes[0];
    assert_eq!(finish["current_proof"]["kind"], "near_ai_login");
    assert_eq!(finish["current_proof"]["access_token"], OLD_ACCESS);
    assert_eq!(finish["new_access_token"], NEW_ACCESS);
    let device: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&fx.identity.public_key_b64)
        .unwrap()
        .try_into()
        .unwrap();
    let challenge = starts[0]["code_challenge"].as_str().unwrap();
    let verifier = finish["code_verifier"].as_str().unwrap();
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier)),
        challenge,
        "the finish carries the start's PKCE verifier"
    );
    let expires_at = Utc::now().timestamp() + 900;
    let signed = |expires: i64| {
        wire::rebind_device_bytes(
            &NONCE,
            finish["ceremony_id"].as_str().unwrap(),
            &device,
            challenge,
            expires,
            &CurrentProof::NearAiLogin {
                access_token: OLD_ACCESS.into(),
            }
            .digest(),
            &wire::new_login_digest(NEW_ACCESS),
        )
    };
    let signature = base64::engine::general_purpose::STANDARD
        .decode(finish["device_signature"].as_str().unwrap())
        .unwrap();
    // The stub's expiry is "now + 900" at its answer: within a few seconds.
    assert!(
        (expires_at - 5..=expires_at).any(|expires| {
            ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, device)
                .verify(&signed(expires), &signature)
                .is_ok()
        }),
        "the device signs the rebind bytes over both proofs"
    );
    // This device signs in with the new login. A `nearai-` account already
    // did by its namespace, so its config is not rewritten for it.
    let cfg = fx.shared.store.load_config().unwrap().unwrap();
    assert!(cfg.signs_in_with_near_ai());
    assert_eq!(cfg.account_identity, None);
    assert_eq!(
        super::super::nearai_credential::login_record::commons_login(&fx.shared.store),
        Some("github")
    );
    let stored =
        crate::daemon::settings::DaemonSettings::load_with_cloud_credentials(&fx.shared.store)
            .unwrap();
    assert_eq!(
        stored
            .near_ai_inference
            .as_ref()
            .map(|key| key.key_id.as_str()),
        Some("key-new")
    );
    // The account session is the one it had: no new session is minted.
    let loaded = crate::account_auth::try_load_session_with_snapshot(&fx.shared.store)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.session.access_token, SESSION);
}

#[tokio::test]
async fn a_wallet_origin_change_records_that_the_account_now_signs_in_with_near_ai() {
    let fx = Fixture::new(&wallet_tenant(), None).await;
    let id = started(&fx).await;
    assert_eq!(
        result(&fx.step("status", json!({ "change_id": id })).await)["current_kind"],
        "wallet"
    );
    // A wallet account's proof needs the name typed in the sheet.
    assert_eq!(
        refusal(&fx.step("prove_current", json!({ "change_id": id })).await),
        Some(ERR_WALLET_ACCOUNT_REQUIRED)
    );
    hold_wallet_proof_for_test(
        fx.dir(),
        &id,
        "alice.testnet",
        "ceremony-wallet",
        NONCE,
        Utc::now().timestamp() + 600,
        &"v".repeat(43),
    );
    sign_in_new_login(&fx, &id, held("google", Some("sha256:new")));

    let view = fx.confirm(&id).await;

    assert_eq!(view["step"], "done", "{view}");
    let finishes = fx.commons.finishes.lock().unwrap().clone();
    assert_eq!(
        fx.commons.starts.lock().unwrap().len(),
        0,
        "started with the wallet leg"
    );
    assert_eq!(finishes[0]["current_proof"]["kind"], "wallet");
    assert_eq!(finishes[0]["current_proof"]["account_id"], "alice.testnet");
    assert_eq!(finishes[0]["ceremony_id"], "ceremony-wallet");
    let cfg = fx.shared.store.load_config().unwrap().unwrap();
    assert_eq!(cfg.tenant_id, wallet_tenant(), "the tenant never changes");
    assert_eq!(cfg.account_identity, Some(IdentityKind::NearAiLogin));
    assert!(cfg.signs_in_with_near_ai());
    assert_eq!(fx.status()["payouts_need_login"], false);
}

#[tokio::test]
async fn a_wallet_proof_whose_commons_ceremony_ran_out_is_asked_again() {
    let fx = Fixture::new(&wallet_tenant(), None).await;
    let id = started(&fx).await;
    hold_wallet_proof_for_test(
        fx.dir(),
        &id,
        "alice.testnet",
        "ceremony-wallet",
        NONCE,
        Utc::now().timestamp() + 5,
        &"v".repeat(43),
    );
    sign_in_new_login(&fx, &id, held("google", Some("sha256:new")));

    let view = fx.confirm(&id).await;

    assert_eq!(view["step"], "current", "the wallet signs again");
    assert_eq!(view["error"], STEP_ERROR_WALLET);
    assert_eq!(view["new_provider"], "google", "the new login is kept");
    assert!(fx.commons.finishes.lock().unwrap().is_empty());
}

async fn refused_with(status: u16, label: &'static str, tenant: &str) -> (Fixture, Value) {
    let fx = Fixture::new(tenant, None).await;
    *fx.commons.refuse.lock().unwrap() = Some((status, label));
    let id = started(&fx).await;
    if tenant == wallet_tenant() {
        hold_wallet_proof_for_test(
            fx.dir(),
            &id,
            "alice.testnet",
            "ceremony-wallet",
            NONCE,
            Utc::now().timestamp() + 600,
            &"v".repeat(43),
        );
    } else {
        prove_near_ai(&fx, &id, None);
    }
    sign_in_new_login(&fx, &id, held("github", Some("sha256:new")));
    let view = fx.confirm(&id).await;
    (fx, view)
}

fn nothing_changed(fx: &Fixture) {
    let cfg = fx.shared.store.load_config().unwrap().unwrap();
    assert_eq!(cfg.account_identity, None, "the config is untouched");
    let stored =
        crate::daemon::settings::DaemonSettings::load_with_cloud_credentials(&fx.shared.store)
            .unwrap();
    assert!(stored.near_ai_inference.is_none(), "no login was stored");
}

#[tokio::test]
async fn a_new_login_already_in_use_is_asked_again_and_nothing_changes() {
    let (fx, view) = refused_with(409, wire::REBIND_IN_USE, &near_ai_tenant()).await;
    assert_eq!(view["error"], STEP_ERROR_IN_USE);
    assert_eq!(view["step"], "new", "the current proof is kept");
    assert!(
        view["new_provider"].is_null(),
        "the refused login is dropped"
    );
    nothing_changed(&fx);
}

#[tokio::test]
async fn a_change_within_the_cooldown_is_too_soon_and_nothing_changes() {
    let (fx, view) = refused_with(409, wire::REBIND_TOO_SOON, &near_ai_tenant()).await;
    assert_eq!(view["error"], STEP_ERROR_TOO_SOON);
    nothing_changed(&fx);
}

#[tokio::test]
async fn a_wallet_proof_the_commons_denies_is_the_wallet_error() {
    let (fx, view) = refused_with(400, "invalid_request", &wallet_tenant()).await;
    assert_eq!(view["error"], STEP_ERROR_WALLET);
    assert_eq!(
        view["step"], "current",
        "the spent wallet proof is asked again"
    );
    nothing_changed(&fx);
}

#[tokio::test]
async fn a_held_payout_link_review_is_unavailable_in_the_sheet() {
    let (fx, view) = refused_with(409, wire::REBIND_PAYOUT_REVIEW_REQUIRED, &wallet_tenant()).await;
    assert_eq!(view["error"], STEP_ERROR_UNAVAILABLE);
    nothing_changed(&fx);
}

#[tokio::test]
async fn an_answer_for_another_tenant_changes_nothing_here() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    *fx.commons.tenant.lock().unwrap() = format!("nearai-{}", "99".repeat(32));
    let id = started(&fx).await;
    prove_near_ai(&fx, &id, None);
    sign_in_new_login(&fx, &id, held("github", Some("sha256:new")));
    let view = fx.confirm(&id).await;
    assert_eq!(view["error"], STEP_ERROR_UNAVAILABLE);
    nothing_changed(&fx);
}

#[tokio::test]
async fn cancel_drops_the_ceremony_and_the_held_login() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    assert!(hold_aside(fx.dir(), held("github", None)).is_none());
    let id = started(&fx).await;
    let cancelled = fx
        .call("account_login_change_cancel", json!({ "change_id": id }))
        .await;
    assert_eq!(result(&cancelled)["cancelled"], true);
    assert_eq!(
        refusal(&fx.step("status", json!({ "change_id": id })).await),
        Some(ERR_NO_CEREMONY)
    );
    assert_eq!(fx.status()["login_change_held"], false);
}

#[tokio::test]
async fn confirm_needs_the_persons_explicit_confirmation() {
    let fx = Fixture::new(&near_ai_tenant(), None).await;
    let id = started(&fx).await;
    prove_near_ai(&fx, &id, None);
    sign_in_new_login(&fx, &id, held("github", None));
    let response = fx.step("confirm", json!({ "change_id": id })).await;
    assert_eq!(refusal(&response), Some("confirmed"));
    assert!(fx.commons.starts.lock().unwrap().is_empty());
    assert_eq!(
        result(&fx.step("status", json!({ "change_id": id })).await)["step"],
        "confirm"
    );
}

#[tokio::test]
async fn the_daemon_status_carries_the_change_fields() {
    let fx = Fixture::new(&wallet_tenant(), None).await;
    let status = fx.call("status", json!({})).await;
    let status = result(&status);
    assert_eq!(status["login_change_available"], true);
    assert_eq!(status["payouts_need_login"], true);
    assert_eq!(status["login_change_held"], false);
}
