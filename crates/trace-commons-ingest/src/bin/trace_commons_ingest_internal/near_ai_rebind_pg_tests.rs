// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! F10, changing the NEAR AI login an account uses, against the real router
//! and a fresh PostgreSQL. Spec:
//! `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`.
//!
//! Accounts are made by the real sign-up handlers (NEAR AI login and NEAR
//! wallet), the handlers run on a NOSUPERUSER NOBYPASSRLS runtime role, and
//! only NEAR AI's `/users/me` and the NEAR RPC are local stand-ins. The
//! `/users/me` stand-in answers the bearer token itself as the subject, so a
//! test names an identity by the token it signs in with.

use super::*;
use axum::body::Body;
use ring::signature::KeyPair as _;
use tower::ServiceExt;
use trace_commons_protocol::account_login_change as wire;
use trace_commons_server::db::PayoutStore as _;

const WALLET_RECIPIENT: &str = "app.tracecommons.test";
const ORIGIN: &str = "https://commons.example";

struct RebindDb {
    admin: Arc<PgBackend>,
    runtime: Arc<PgBackend>,
}

async fn fresh_db() -> RebindDb {
    let pg = super::migrated_pg_fixture::migrated_pg(
        "TRACE_COMMONS_REBIND_PG_TEST_URL",
        Some("admission_test_rebind"),
    )
    .await;
    let runtime = pg.narrow_runtime("tc_rebind_runtime").await;
    // Production's ingest login holds the definer functions through this
    // group (V90), as the pilot's does; without it a function V150 grants
    // only to the group would look callable here and fail in service.
    pg.admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .batch_execute("GRANT trace_ingest_runtime TO tc_rebind_runtime")
        .await
        .unwrap();
    RebindDb {
        admin: pg.admin,
        runtime,
    }
}

fn serial() -> &'static tokio::sync::Mutex<()> {
    static SERIAL: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    SERIAL.get_or_init(Default::default)
}

/// A subject nobody has used, per call, so a re-run never meets its own rows.
fn subject(label: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "rebind-{label}-{}-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// A NEAR account name nobody has signed up with, per run.
fn wallet_account() -> String {
    format!(
        "rebind-{}-{}.testnet",
        std::process::id(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
    )
}

fn identity() -> trace_commons_server::near_account_identity::NearAccountIdentity {
    let crypto =
        trace_commons_server::secrets::SecretsCrypto::new(SecretString::from("a".repeat(32)))
            .unwrap();
    let kek: Arc<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper> = Arc::new(
        trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(
            crypto,
            "rebind-fixture",
        ),
    );
    trace_commons_server::near_account_identity::NearAccountIdentity::from_parts(
        Some(&base64::engine::general_purpose::STANDARD.encode([9u8; 32])),
        Some(kek),
    )
    .unwrap()
}

fn keypair() -> ring::signature::Ed25519KeyPair {
    let pkcs8 =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn device_public(device: &ring::signature::Ed25519KeyPair) -> [u8; 32] {
    device.public_key().as_ref().try_into().unwrap()
}

fn wallet_key(wallet: &ring::signature::Ed25519KeyPair) -> String {
    format!(
        "ed25519:{}",
        bs58::encode(wallet.public_key().as_ref()).into_string()
    )
}

fn challenge_pair() -> (String, String) {
    let verifier = "rebind-fixture-verifier".repeat(3);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The NEP-413 preimage a wallet signs, formed independently of the server.
fn nep413(
    message: &str,
    nonce_b64: &str,
    recipient: &str,
    callback: &str,
    wallet: &ring::signature::Ed25519KeyPair,
) -> String {
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(nonce_b64)
        .unwrap();
    let mut bytes = 2_147_484_061_u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(message.len() as u32).to_le_bytes());
    bytes.extend_from_slice(message.as_bytes());
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
    bytes.extend_from_slice(recipient.as_bytes());
    bytes.push(1);
    bytes.extend_from_slice(&(callback.len() as u32).to_le_bytes());
    bytes.extend_from_slice(callback.as_bytes());
    b64(wallet.sign(&Sha256::digest(bytes)).as_ref())
}

/// `/users/me`, answering the bearer token as the subject.
async fn stub_near_ai() -> String {
    let router = axum::Router::new().route(
        "/users/me",
        axum::routing::get(|headers: HeaderMap| async move {
            let token = headers
                .get(AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.strip_prefix("Bearer "))
                .unwrap_or_default()
                .to_string();
            axum::Json(serde_json::json!({ "id": token, "auth_provider": "github" }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

/// The NEAR RPC: `wallet` is a FullAccess key of every account but one.
async fn stub_rpc(wallet: String) -> String {
    let router = axum::Router::new().route(
        "/",
        axum::routing::post(move |Json(request): Json<serde_json::Value>| {
            let wallet = wallet.clone();
            async move {
                let keys = if request["params"]["account_id"] != "someone-else.testnet" {
                    serde_json::json!([{"public_key": wallet,
                        "access_key": {"permission": "FullAccess"}}])
                } else {
                    serde_json::json!([])
                };
                Json(serde_json::json!({"jsonrpc":"2.0","id":"tc","result":{"keys":keys}}))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    url
}

async fn state(db: Arc<PgBackend>, wallet: String) -> (Arc<AppState>, tempfile::TempDir) {
    // SAFETY: this ignored suite is serialised and runs in its own CI step.
    unsafe {
        std::env::remove_var("TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON");
        std::env::set_var(
            "TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL",
            "https://issuer.example",
        );
        std::env::set_var("TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE", "upload");
    }
    let near_ai = stub_near_ai().await;
    let rpc = stub_rpc(wallet).await;
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state(temp.path().to_path_buf());
    let s = Arc::make_mut(&mut state);
    s.near_provisioning_enabled = true;
    s.near_provisioning_admission_ready = true;
    s.near_provisioning_public_origin = Some(ORIGIN.into());
    s.account_near_config = Some(Arc::new(trace_commons_server::config::NearConfig {
        rpc_url: rpc,
        network: "testnet".into(),
        recipient: WALLET_RECIPIENT.into(),
    }));
    s.near_account_identity = Some(Arc::new(identity()));
    s.db_mirror = Some(db as Arc<dyn Database>);
    s.near_ai_introspection_base_url = Some(near_ai);
    (state, temp)
}

async fn request(
    state: &Arc<AppState>,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
    bearer: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = axum::http::Request::builder().method(method).uri(path);
    let bytes = if let Some(body) = body {
        builder = builder.header(CONTENT_TYPE, "application/json");
        serde_json::to_vec(&body).unwrap()
    } else {
        Vec::new()
    };
    if let Some(bearer) = bearer {
        builder = builder.header(AUTHORIZATION, format!("Bearer {bearer}"));
    }
    let response = app(state.clone())
        .oneshot(builder.body(Body::from(bytes)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

/// A signed-in account: what the client keeps after sign-up.
struct SignedIn {
    tenant: String,
    account: uuid::Uuid,
    bearer: String,
    device: ring::signature::Ed25519KeyPair,
}

/// Sign up (or in) with a NEAR AI login through the real handlers.
async fn near_ai_sign_in(
    state: &Arc<AppState>,
    device: &ring::signature::Ed25519KeyPair,
    token: &str,
) -> (StatusCode, serde_json::Value) {
    let (verifier, challenge) = challenge_pair();
    let (status, started) = request(
        state,
        "POST",
        "/v1/account/near-ai/provision/start/v2",
        Some(serde_json::json!({
            "device_public_key": b64(&device_public(device)),
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(started["nonce"].as_str().unwrap())
        .unwrap()
        .try_into()
        .unwrap();
    let ceremony = started["ceremony_id"].as_str().unwrap();
    let bytes = trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
        &nonce,
        ceremony,
        &device_public(device),
        &challenge,
        started["expires_at"].as_i64().unwrap(),
    );
    request(
        state,
        "POST",
        "/v1/account/near-ai/provision/finish/v2",
        Some(serde_json::json!({
            "ceremony_id": ceremony,
            "code_verifier": verifier,
            "device_public_key": b64(&device_public(device)),
            "device_signature": b64(device.sign(&bytes).as_ref()),
            "access_token": token,
        })),
        None,
    )
    .await
}

fn signed_in(device: ring::signature::Ed25519KeyPair, answer: &serde_json::Value) -> SignedIn {
    SignedIn {
        tenant: answer["tenant_id"].as_str().unwrap().to_string(),
        account: uuid::Uuid::parse_str(answer["account_id"].as_str().unwrap()).unwrap(),
        bearer: answer["access_token"].as_str().unwrap().to_string(),
        device,
    }
}

async fn near_ai_account(state: &Arc<AppState>, token: &str) -> SignedIn {
    let device = keypair();
    let (status, answer) = near_ai_sign_in(state, &device, token).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    signed_in(device, &answer)
}

/// Sign up with a NEAR wallet through the real handlers.
async fn wallet_sign_in(
    state: &Arc<AppState>,
    account: &str,
    device: &ring::signature::Ed25519KeyPair,
    wallet: &ring::signature::Ed25519KeyPair,
) -> (StatusCode, serde_json::Value) {
    let (verifier, challenge) = challenge_pair();
    let (status, started) = request(
        state,
        "POST",
        "/v1/account/near/provision/start/v2",
        Some(serde_json::json!({
            "account_id": account,
            "device_public_key": b64(&device_public(device)),
            "code_challenge": challenge, "code_challenge_method": "S256",
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let signing = base64::engine::general_purpose::STANDARD
        .decode(started["device_signing_bytes"].as_str().unwrap())
        .unwrap();
    request(
        state,
        "POST",
        "/v1/account/near/provision/finish/v2",
        Some(serde_json::json!({
            "ceremony_id": started["ceremony_id"], "account_id": account,
            "device_public_key": b64(&device_public(device)),
            "wallet_public_key": wallet_key(wallet),
            "wallet_signature": nep413(
                started["message"].as_str().unwrap(),
                started["nonce"].as_str().unwrap(),
                started["recipient"].as_str().unwrap(),
                started["wallet_url"].as_str().unwrap(),
                wallet,
            ),
            "device_signature": b64(device.sign(&signing).as_ref()),
            "code_verifier": verifier,
        })),
        None,
    )
    .await
}

/// The change, start to finish, as the client runs it.
enum Current<'a> {
    Login(&'a str),
    Wallet {
        account: &'a str,
        wallet: &'a ring::signature::Ed25519KeyPair,
    },
}

async fn change(
    state: &Arc<AppState>,
    who: &SignedIn,
    signer: &ring::signature::Ed25519KeyPair,
    current: Current<'_>,
    new_token: &str,
) -> (StatusCode, serde_json::Value) {
    let (verifier, challenge) = challenge_pair();
    let wallet_account = match current {
        Current::Wallet { account, .. } => Some(account.to_string()),
        Current::Login(_) => None,
    };
    let (status, started) = request(
        state,
        "POST",
        wire::REBIND_START_PATH,
        Some(
            serde_json::to_value(wire::RebindStartRequest {
                device_public_key: b64(&device_public(signer)),
                code_challenge: challenge.clone(),
                code_challenge_method: "S256".into(),
                wallet_account_id: wallet_account,
            })
            .unwrap(),
        ),
        Some(&who.bearer),
    )
    .await;
    if status != StatusCode::OK {
        return (status, started);
    }
    let started: wire::RebindStartResponse = serde_json::from_value(started).unwrap();
    let proof = match current {
        Current::Login(token) => wire::CurrentProof::NearAiLogin {
            access_token: token.into(),
        },
        Current::Wallet { account, wallet } => {
            let challenge = started.wallet.as_ref().expect("a wallet challenge");
            wire::CurrentProof::Wallet {
                account_id: account.into(),
                wallet_public_key: wallet_key(wallet),
                wallet_signature: nep413(
                    &challenge.message,
                    &started.nonce,
                    &challenge.recipient,
                    &challenge.wallet_url,
                    wallet,
                ),
            }
        }
    };
    let nonce: [u8; 32] = base64::engine::general_purpose::STANDARD
        .decode(&started.nonce)
        .unwrap()
        .try_into()
        .unwrap();
    let bytes = wire::rebind_device_bytes(
        &nonce,
        &started.ceremony_id,
        &device_public(signer),
        &challenge,
        started.expires_at,
        &proof.digest(),
        &wire::new_login_digest(new_token),
    );
    request(
        state,
        "POST",
        wire::REBIND_FINISH_PATH,
        Some(
            serde_json::to_value(wire::RebindFinishRequest {
                ceremony_id: started.ceremony_id,
                code_verifier: verifier,
                device_public_key: b64(&device_public(signer)),
                device_signature: b64(signer.sign(&bytes).as_ref()),
                current_proof: proof,
                new_access_token: new_token.into(),
            })
            .unwrap(),
        ),
        Some(&who.bearer),
    )
    .await
}

/// `(live, retired)` anchors of one account, read as the migration owner in
/// the tenant's context.
async fn anchors(db: &PgBackend, tenant: &str, account: uuid::Uuid) -> (Vec<String>, Vec<String>) {
    let mut client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let rows = tx
        .query(
            "SELECT anchor_hash, retired_at IS NOT NULL FROM trace_near_account_anchors
              WHERE tenant_id=$1 AND account_id=$2 ORDER BY anchor_hash",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    let mut live = Vec::new();
    let mut retired = Vec::new();
    for row in rows {
        if row.get::<_, bool>(1) {
            retired.push(row.get(0));
        } else {
            live.push(row.get(0));
        }
    }
    (live, retired)
}

async fn in_tenant<T>(
    db: &PgBackend,
    tenant: &str,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    read: impl Fn(&tokio_postgres::Row) -> T,
) -> Vec<T> {
    let mut client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let rows = tx.query(sql, params).await.unwrap();
    let out = rows.iter().map(read).collect();
    tx.commit().await.unwrap();
    out
}

async fn exec_in_tenant(db: &PgBackend, tenant: &str, sql: &str) {
    let mut client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.batch_execute(sql).await.unwrap();
    tx.commit().await.unwrap();
}

async fn count_accounts(db: &PgBackend) -> i64 {
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    client
        .query_one("SELECT count(*) FROM trace_accounts", &[])
        .await
        .unwrap()
        .get(0)
}

fn uniform_deny(status: StatusCode, body: &serde_json::Value) {
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(body["error"], "native sign-in could not be completed");
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn a_near_ai_account_moves_to_a_new_login_and_the_old_one_is_retired() {
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let wallet = keypair();
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&wallet)).await;
    let (old, new) = (subject("old"), subject("new"));
    let who = near_ai_account(&state, &old).await;
    let (before_live, _) = anchors(&db.admin, &who.tenant, who.account).await;
    assert_eq!(before_live.len(), 1);

    let (status, body) = change(&state, &who, &who.device, Current::Login(&old), &new).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer: wire::RebindFinishResponse = serde_json::from_value(body).unwrap();
    assert_eq!(answer.tenant_id, who.tenant);
    assert_eq!(answer.account_id, who.account.to_string());
    assert_eq!(answer.identity, wire::IdentityKind::NearAiLogin);
    assert!(!answer.switched_back);

    let (live, retired) = anchors(&db.admin, &who.tenant, who.account).await;
    assert_eq!(live, vec![answer.anchor_hash.clone()]);
    assert_eq!(
        retired, before_live,
        "the old anchor is retired, not deleted"
    );

    // Devices follow the account's one live anchor.
    let device_anchors = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT DISTINCT anchor_hash FROM trace_near_provisioned_devices
          WHERE tenant_id=$1 AND account_id=$2",
        &[&who.tenant, &who.account],
        |row| row.get::<_, String>(0),
    )
    .await;
    assert_eq!(device_anchors, live);

    // Audit: label-only, no anchors or providers.
    let audit = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT safe_metadata FROM trace_account_audit
          WHERE tenant_id=$1 AND action='near_ai_login_rebound'",
        &[&who.tenant],
        |row| row.get::<_, serde_json::Value>(0),
    )
    .await;
    assert_eq!(
        audit,
        vec![
            serde_json::json!({"identity":"near_ai_login","from":"near_ai_login","switched_back":false})
        ]
    );
    let changes = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT from_source, switched_back FROM trace_account_login_changes
          WHERE tenant_id=$1 AND account_id=$2",
        &[&who.tenant, &who.account],
        |row| (row.get::<_, String>(0), row.get::<_, bool>(1)),
    )
    .await;
    assert_eq!(changes, vec![("near_ai_login".to_string(), false)]);

    // The old login no longer signs in, and mints nothing.
    let accounts = count_accounts(&db.admin).await;
    let (status, body) = near_ai_sign_in(&state, &keypair(), &old).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], wire::NEAR_AI_LOGIN_RETIRED);
    assert_eq!(count_accounts(&db.admin).await, accounts);

    // The new login signs in to the same account, and says how.
    let (status, body) = near_ai_sign_in(&state, &keypair(), &new).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tenant_id"], who.tenant.as_str());
    assert_eq!(body["account_id"], who.account.to_string());
    assert_eq!(body["identity"], "near_ai_login");
    assert_eq!(body["auth_provider"], "github");

    // A new payout link proves against the live login only.
    let login_anchors = db
        .runtime
        .payout_account_login_anchors(&who.tenant, who.account)
        .await
        .unwrap();
    assert_eq!(login_anchors, live);

    // Readiness reads devices, which the change re-keyed.
    let ready: bool = db
        .admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .query_one("SELECT trace_account_admission_linkage_ready()", &[])
        .await
        .unwrap()
        .get(0);
    assert!(ready);
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn a_wallet_account_records_a_near_ai_login_with_a_wallet_proof() {
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let wallet = keypair();
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&wallet)).await;
    let device = keypair();
    let name = wallet_account();
    let (status, answer) = wallet_sign_in(&state, &name, &device, &wallet).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let who = signed_in(device, &answer);
    assert!(who.tenant.starts_with("near-"));
    assert!(
        db.runtime
            .payout_account_login_anchors(&who.tenant, who.account)
            .await
            .unwrap()
            .is_empty(),
        "a wallet account holds no NEAR AI login before the change"
    );

    // The wrong account name is the uniform deny, and changes nothing.
    let other = keypair();
    let (status, body) = change(
        &state,
        &who,
        &who.device,
        Current::Wallet {
            account: "someone-else.testnet",
            wallet: &other,
        },
        &subject("never"),
    )
    .await;
    uniform_deny(status, &body);

    let new = subject("wallet-new");
    let (status, body) = change(
        &state,
        &who,
        &who.device,
        Current::Wallet {
            account: &name,
            wallet: &wallet,
        },
        &new,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer: wire::RebindFinishResponse = serde_json::from_value(body).unwrap();
    assert_eq!(
        answer.tenant_id, who.tenant,
        "the account keeps its near- tenant"
    );
    assert_eq!(
        db.runtime
            .payout_account_login_anchors(&who.tenant, who.account)
            .await
            .unwrap(),
        vec![answer.anchor_hash.clone()]
    );
    let audit = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT safe_metadata FROM trace_account_audit
          WHERE tenant_id=$1 AND action='near_ai_login_rebound'",
        &[&who.tenant],
        |row| row.get::<_, serde_json::Value>(0),
    )
    .await;
    assert_eq!(audit[0]["from"], "wallet");

    // The wallet no longer signs in, and mints no new account.
    let accounts = count_accounts(&db.admin).await;
    let (status, body) = wallet_sign_in(&state, &name, &keypair(), &wallet).await;
    uniform_deny(status, &body);
    assert_eq!(count_accounts(&db.admin).await, accounts);
    let identities = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT count(*) FROM trace_near_identities
          WHERE tenant_id=$1 AND account_id=$2 AND revoked_at IS NULL",
        &[&who.tenant, &who.account],
        |row| row.get::<_, i64>(0),
    )
    .await;
    assert_eq!(identities, vec![0], "the wallet's sign-in keys are revoked");

    // The NEAR AI login signs in to the near- tenant.
    let (status, body) = near_ai_sign_in(&state, &keypair(), &new).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["tenant_id"], who.tenant.as_str());
    assert_eq!(body["account_id"], who.account.to_string());
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn a_login_that_already_has_an_account_is_refused_by_name() {
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&keypair())).await;
    let (a, b) = (subject("a"), subject("b"));
    let first = near_ai_account(&state, &a).await;
    let _second = near_ai_account(&state, &b).await;
    let before = anchors(&db.admin, &first.tenant, first.account).await;
    let (status, body) = change(&state, &first, &first.device, Current::Login(&a), &b).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], wire::REBIND_IN_USE);
    assert_eq!(
        anchors(&db.admin, &first.tenant, first.account).await,
        before
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn the_wrong_current_login_or_a_reused_new_one_changes_nothing() {
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&keypair())).await;
    let current = subject("current");
    let who = near_ai_account(&state, &current).await;
    let before = anchors(&db.admin, &who.tenant, who.account).await;
    // A current proof that is not this account's login.
    let (status, body) = change(
        &state,
        &who,
        &who.device,
        Current::Login(&subject("stranger")),
        &subject("x"),
    )
    .await;
    uniform_deny(status, &body);
    // The new login the same as the current one.
    let (status, body) = change(
        &state,
        &who,
        &who.device,
        Current::Login(&current),
        &current,
    )
    .await;
    uniform_deny(status, &body);
    // Another device's key under this account's session.
    let (status, body) = change(
        &state,
        &who,
        &keypair(),
        Current::Login(&current),
        &subject("y"),
    )
    .await;
    uniform_deny(status, &body);
    assert_eq!(anchors(&db.admin, &who.tenant, who.account).await, before);
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn the_cooldown_holds_and_switching_back_revives_the_old_login() {
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&keypair())).await;
    let (a, b) = (subject("a"), subject("b"));
    let who = near_ai_account(&state, &a).await;
    let (original, _) = anchors(&db.admin, &who.tenant, who.account).await;
    let (status, body) = change(&state, &who, &who.device, Current::Login(&a), &b).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = change(&state, &who, &who.device, Current::Login(&b), &a).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], wire::REBIND_TOO_SOON);

    exec_in_tenant(
        &db.admin,
        &who.tenant,
        &format!(
            "UPDATE trace_account_login_changes SET changed_at = now() - interval '8 days'
              WHERE account_id = '{}'",
            who.account
        ),
    )
    .await;
    let (status, body) = change(&state, &who, &who.device, Current::Login(&b), &a).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let answer: wire::RebindFinishResponse = serde_json::from_value(body).unwrap();
    assert!(answer.switched_back);
    let (live, retired) = anchors(&db.admin, &who.tenant, who.account).await;
    assert_eq!(live, original);
    assert_eq!(retired.len(), 1);
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn revoked_and_code_joined_devices_cannot_start_a_change() {
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&keypair())).await;
    let login = subject("devices");
    let who = near_ai_account(&state, &login).await;
    let device_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
        &device_public(&who.device),
    );

    // Joined by code: a redeemed code names this device. Both hashes are
    // UNIQUE across the table, so they are unique per run.
    let code_hash = format!(
        "{:x}",
        Sha256::digest(format!("code-{device_id}").as_bytes())
    );
    let nonce_hash = format!(
        "{:x}",
        Sha256::digest(format!("nonce-{device_id}").as_bytes())
    );
    exec_in_tenant(
        &db.admin,
        &who.tenant,
        &format!(
            "INSERT INTO trace_account_device_codes
                 (tenant_id, code_id, account_id, code_hash, minted_by_device_key_id,
                  mint_nonce_hash, expires_at, redeemed_at, redeemed_device_key_id)
             VALUES ('{t}', gen_random_uuid(), '{a}', 'sha256:{h}', '{d}', 'sha256:{n}',
                     now() + interval '1 hour', now(), '{d}')",
            t = who.tenant,
            a = who.account,
            h = code_hash,
            n = nonce_hash,
            d = device_id
        ),
    )
    .await;
    let (status, body) = change(
        &state,
        &who,
        &who.device,
        Current::Login(&login),
        &subject("z"),
    )
    .await;
    uniform_deny(status, &body);

    // Revoked: a fresh account, its device key revoked under its session.
    let other = subject("revoked");
    let second = near_ai_account(&state, &other).await;
    let second_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
        &device_public(&second.device),
    );
    exec_in_tenant(
        &db.admin,
        &second.tenant,
        &format!("UPDATE device_keys SET revoked_at = now() WHERE device_key_id = '{second_id}'"),
    )
    .await;
    let (status, body) = change(
        &state,
        &second,
        &second.device,
        Current::Login(&other),
        &subject("w"),
    )
    .await;
    assert!(
        status == StatusCode::BAD_REQUEST || status == StatusCode::UNAUTHORIZED,
        "{status} {body}"
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn a_change_refuses_under_a_live_admission_lease() {
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&keypair())).await;
    let login = subject("lease");
    let who = near_ai_account(&state, &login).await;
    let (live, _) = anchors(&db.admin, &who.tenant, who.account).await;
    let hex = live[0].strip_prefix("sha256:").unwrap();
    exec_in_tenant(
        &db.admin,
        &who.tenant,
        &format!(
            "INSERT INTO trace_admission_submissions
                 (tenant_id, submission_id, anchor_hash, body_hash, kind, status, lease_id,
                  lease_expires_at, last_cost_bound)
             VALUES ('{t}', gen_random_uuid(), '{hex}', '{h}', 'window', 'reserved',
                     gen_random_uuid(), now() + interval '5 minutes', 1)",
            t = who.tenant,
            h = "a".repeat(64)
        ),
    )
    .await;
    let (status, body) = change(
        &state,
        &who,
        &who.device,
        Current::Login(&login),
        &subject("lease-new"),
    )
    .await;
    uniform_deny(status, &body);
    assert_eq!(anchors(&db.admin, &who.tenant, who.account).await.0, live);
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn pending_link_requests_are_dropped_and_a_live_link_is_kept() {
    use trace_commons_server::db::{
        PayoutLinkInsert, PayoutLinkState, PayoutLinkWrite, PayoutStore, SealedRecipient,
    };
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&keypair())).await;
    let login = subject("payout");
    let who = near_ai_account(&state, &login).await;
    let digest = |label: &str| format!("sha256:{:x}", Sha256::digest(label.as_bytes()));
    let link_id = uuid::Uuid::new_v4();
    assert_eq!(
        db.admin
            .insert_payout_link(
                &who.tenant,
                &PayoutLinkWrite {
                    account_id: who.account,
                    rail_id: trace_commons_gate_api::RailId::new("near_ai").unwrap(),
                    link_id,
                    recipient_kind: trace_commons_gate_api::RecipientKind::new("near_ai_org")
                        .unwrap(),
                    recipient: SealedRecipient {
                        sealed: serde_json::json!({"v": 1}),
                        recipient_index: digest("index"),
                        recipient_key_ref: "key-test-1".into(),
                        index_pepper_ref: "pepper-test-1".into(),
                        subject_index: digest("subject"),
                        recipient_ref: None,
                    },
                    program: "program-test".into(),
                    state: PayoutLinkState::Active,
                    actor_ref_hash: digest("actor"),
                },
            )
            .await
            .unwrap(),
        PayoutLinkInsert::Inserted
    );
    exec_in_tenant(
        &db.admin,
        &who.tenant,
        &format!(
            "INSERT INTO trace_payout_link_requests
                 (tenant_id, account_id, rail_id, idempotency_key, handle_hash,
                  request_digest, action, expires_at)
             VALUES ('{t}', '{a}', 'near_ai', gen_random_uuid(), '{h}', '{h}', 'unlink',
                     now() + interval '5 minutes')",
            t = who.tenant,
            a = who.account,
            h = digest(&format!("handle-{}", who.account))
        ),
    )
    .await;
    let (status, body) = change(
        &state,
        &who,
        &who.device,
        Current::Login(&login),
        &subject("payout-new"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let pending = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT count(*) FROM trace_payout_link_requests WHERE tenant_id=$1 AND account_id=$2",
        &[&who.tenant, &who.account],
        |row| row.get::<_, i64>(0),
    )
    .await;
    assert_eq!(pending, vec![0]);
    let links = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT state, unlink_requested_at IS NULL FROM trace_payout_links
          WHERE tenant_id=$1 AND link_id=$2",
        &[&who.tenant, &link_id],
        |row| (row.get::<_, String>(0), row.get::<_, bool>(1)),
    )
    .await;
    assert_eq!(links, vec![("active".to_string(), true)]);
}

async fn seed_link(
    db: &PgBackend,
    who: &SignedIn,
    state: trace_commons_server::db::PayoutLinkState,
) -> uuid::Uuid {
    use trace_commons_server::db::{PayoutLinkInsert, PayoutLinkWrite, SealedRecipient};
    let digest = |label: &str| format!("sha256:{:x}", Sha256::digest(label.as_bytes()));
    let link_id = uuid::Uuid::new_v4();
    assert_eq!(
        db.insert_payout_link(
            &who.tenant,
            &PayoutLinkWrite {
                account_id: who.account,
                rail_id: trace_commons_gate_api::RailId::new("near_ai").unwrap(),
                link_id,
                recipient_kind: trace_commons_gate_api::RecipientKind::new("near_ai_org").unwrap(),
                recipient: SealedRecipient {
                    sealed: serde_json::json!({"v": 1}),
                    recipient_index: digest("index"),
                    recipient_key_ref: "key-test-1".into(),
                    index_pepper_ref: "pepper-test-1".into(),
                    subject_index: digest("subject"),
                    recipient_ref: None,
                },
                program: "program-test".into(),
                state,
                actor_ref_hash: digest("actor"),
            },
        )
        .await
        .unwrap(),
        PayoutLinkInsert::Inserted
    );
    link_id
}

/// A wallet-origin account holds no proved payout link after #1862 and V147.
/// If one is there anyway, an active link is unlinked as V147 would have,
/// with one hash-only audit row, and a waiting one holds the change for an
/// operator.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn a_wallet_accounts_stray_payout_link_is_unlinked_or_holds_the_change() {
    use trace_commons_server::db::PayoutLinkState;
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let wallet = keypair();
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&wallet)).await;

    // Waiting: held.
    let name = wallet_account();
    let device = keypair();
    let (status, answer) = wallet_sign_in(&state, &name, &device, &wallet).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let held = signed_in(device, &answer);
    seed_link(&db.admin, &held, PayoutLinkState::Waiting).await;
    let before = anchors(&db.admin, &held.tenant, held.account).await;
    let (status, body) = change(
        &state,
        &held,
        &held.device,
        Current::Wallet {
            account: &name,
            wallet: &wallet,
        },
        &subject("held"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], wire::REBIND_PAYOUT_REVIEW_REQUIRED);
    assert_eq!(anchors(&db.admin, &held.tenant, held.account).await, before);

    // Active: unlinked, audited. A second wallet: one wallet key belongs
    // to one account.
    let wallet = keypair();
    let (state, _temp2) = self::state(db.runtime.clone(), wallet_key(&wallet)).await;
    let name = wallet_account();
    let device = keypair();
    let (status, answer) = wallet_sign_in(&state, &name, &device, &wallet).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    let who = signed_in(device, &answer);
    let link = seed_link(&db.admin, &who, PayoutLinkState::Active).await;
    let (status, body) = change(
        &state,
        &who,
        &who.device,
        Current::Wallet {
            account: &name,
            wallet: &wallet,
        },
        &subject("unlinked"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let unlinked = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT unlink_requested_at IS NOT NULL FROM trace_payout_links WHERE link_id=$1",
        &[&link],
        |row| row.get::<_, bool>(0),
    )
    .await;
    assert_eq!(unlinked, vec![true]);
    let audit = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT action, label, link_id_hash FROM trace_payout_audit
          WHERE tenant_id=$1 AND label='near_ai_login_required'",
        &[&who.tenant],
        |row| {
            (
                row.get::<_, String>(0),
                row.get::<_, String>(1),
                row.get::<_, String>(2),
            )
        },
    )
    .await;
    assert_eq!(
        audit,
        vec![(
            "unlink_requested".to_string(),
            "near_ai_login_required".to_string(),
            trace_commons_server::db::payout_link_id_hash(link)
        )]
    );
}

/// Two changes of one account at once: one wins, the other changes nothing.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn two_concurrent_changes_of_one_account_leave_exactly_one() {
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&keypair())).await;
    let login = subject("race");
    let who = near_ai_account(&state, &login).await;
    let (b, c) = (subject("race-b"), subject("race-c"));
    let (first, second) = tokio::join!(
        change(&state, &who, &who.device, Current::Login(&login), &b),
        change(&state, &who, &who.device, Current::Login(&login), &c),
    );
    let won = [&first, &second]
        .iter()
        .filter(|(status, _)| *status == StatusCode::OK)
        .count();
    assert_eq!(won, 1, "{first:?} {second:?}");
    let (live, retired) = anchors(&db.admin, &who.tenant, who.account).await;
    assert_eq!(live.len(), 1);
    assert_eq!(retired.len(), 1);
    let changes = in_tenant(
        &db.admin,
        &who.tenant,
        "SELECT count(*) FROM trace_account_login_changes WHERE tenant_id=$1 AND account_id=$2",
        &[&who.tenant, &who.account],
        |row| row.get::<_, i64>(0),
    )
    .await;
    assert_eq!(changes, vec![1]);
}

/// Forced RLS: a session of one tenant cannot reach another tenant's account,
/// even naming it directly to the store.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REBIND_PG_TEST_URL"]
async fn a_session_of_one_tenant_cannot_change_another_tenants_account() {
    use trace_commons_server::account_login_change::{RebindPrecheck, RebindSession};
    let _serial = serial().lock().await;
    let db = fresh_db().await;
    let (state, _temp) = state(db.runtime.clone(), wallet_key(&keypair())).await;
    let a = near_ai_account(&state, &subject("tenant-a")).await;
    let b = near_ai_account(&state, &subject("tenant-b")).await;
    let device_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
        &device_public(&a.device),
    );
    let (_, session_hash) =
        trace_commons_server::account_native_auth::native_token_parts(&a.bearer).unwrap();
    // Tenant A's session and device, naming tenant B's account.
    let crossed = RebindSession {
        tenant_id: a.tenant.clone(),
        account_id: b.account,
        session_token_hash: session_hash,
        device_key_id: device_id,
    };
    assert!(matches!(
        db.runtime.near_ai_rebind_precheck(&crossed).await.unwrap(),
        RebindPrecheck::Refused(_)
    ));
}
