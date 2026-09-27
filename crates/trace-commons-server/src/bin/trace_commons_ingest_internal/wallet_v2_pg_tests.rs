// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Wallet v2 completion against the router and a fresh PostgreSQL database.
//! Only the external NEAR RPC response is local. The production verifier checks
//! the signed NEP-413 payload, device proof, PKCE binding and FullAccess result.

use super::*;
use axum::body::Body;
use ring::signature::KeyPair as _;
use tower::ServiceExt;

const WALLET_ACCOUNT: &str = "wallet-v2-fixture.testnet";
const WALLET_RECIPIENT: &str = "app.tracecommons.test";

fn database_config(url: &str, resolver_url: Option<&str>) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url.to_owned()),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url: resolver_url.map(|value| SecretString::from(value.to_owned())),
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

async fn fresh_db() -> Arc<PgBackend> {
    let url = std::env::var("TRACE_COMMONS_WALLET_V2_PG_TEST_URL")
        .expect("wallet v2 needs an explicit fresh PostgreSQL URL");
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert_eq!(parsed.path(), "/admission_test_walletv2");
    let migrator = PgBackend::new(&database_config(&url, None)).await.unwrap();
    migrator.run_migrations().await.unwrap();

    // The anchor lookup must use the resolver role and its narrow RLS policy.
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    let connection_task = tokio::spawn(connection);
    client
        .batch_execute(
            "ALTER ROLE trace_login_resolver LOGIN; \
             GRANT USAGE ON SCHEMA public TO trace_login_resolver; \
             GRANT SELECT (tenant_id, anchor_hash) \
               ON trace_near_account_anchors TO trace_login_resolver;",
        )
        .await
        .unwrap();
    drop(client);
    connection_task.abort();
    let mut resolver = parsed;
    resolver.set_username("trace_login_resolver").unwrap();
    Arc::new(
        PgBackend::new(&database_config(&url, Some(resolver.as_str())))
            .await
            .unwrap(),
    )
}

fn identity() -> trace_commons_server::near_account_identity::NearAccountIdentity {
    let crypto =
        trace_commons_server::secrets::SecretsCrypto::new(SecretString::from("a".repeat(32)))
            .unwrap();
    let kek: Arc<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper> = Arc::new(
        trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(
            crypto,
            "wallet-v2-fixture",
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

fn wallet_key(wallet: &ring::signature::Ed25519KeyPair) -> String {
    format!(
        "ed25519:{}",
        bs58::encode(wallet.public_key().as_ref()).into_string()
    )
}

fn challenge_pair() -> (String, String) {
    let verifier = "wallet-v2-verifier".repeat(4);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// Independently form the NEP-413 Borsh preimage, including the Some(callback)
/// option emitted by the actual wallet v2 start route.
fn sign_wallet(started: &serde_json::Value, wallet: &ring::signature::Ed25519KeyPair) -> String {
    let message = started["message"].as_str().unwrap();
    let recipient = started["recipient"].as_str().unwrap();
    let callback = started["wallet_url"].as_str().unwrap();
    let nonce = base64::engine::general_purpose::STANDARD
        .decode(started["nonce"].as_str().unwrap())
        .unwrap();
    assert_eq!(nonce.len(), 32);
    let mut bytes = 2_147_484_061_u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(message.len() as u32).to_le_bytes());
    bytes.extend_from_slice(message.as_bytes());
    bytes.extend_from_slice(&nonce);
    bytes.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
    bytes.extend_from_slice(recipient.as_bytes());
    bytes.push(1); // Borsh Option::Some(callback_url)
    bytes.extend_from_slice(&(callback.len() as u32).to_le_bytes());
    bytes.extend_from_slice(callback.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(wallet.sign(&Sha256::digest(bytes)).as_ref())
}

async fn rpc_fixture(public_key: String) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let requests = calls.clone();
    let router = axum::Router::new().route(
        "/",
        axum::routing::post(move |Json(request): Json<serde_json::Value>| {
            let public_key = public_key.clone();
            let requests = requests.clone();
            async move {
                assert_eq!(request["jsonrpc"], "2.0");
                assert_eq!(request["method"], "query");
                assert_eq!(request["params"]["request_type"], "view_access_key_list");
                assert_eq!(request["params"]["finality"], "final");
                assert_eq!(request["params"]["account_id"], WALLET_ACCOUNT);
                requests.fetch_add(1, Ordering::SeqCst);
                Json(serde_json::json!({
                    "jsonrpc":"2.0","id":"tc",
                    "result":{"keys":[{"public_key":public_key,
                        "access_key":{"permission":"FullAccess"}}]}
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (url, calls)
}

fn state(db: Arc<PgBackend>, rpc_url: String) -> Arc<AppState> {
    // No legacy witness is published; v2 must still complete. This ignored test
    // runs alone in CI because readiness reads these process-global variables.
    unsafe {
        std::env::remove_var("TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON");
        std::env::set_var(
            "TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL",
            "https://issuer.example",
        );
        std::env::set_var("TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE", "upload");
    }
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state(temp.path().to_path_buf());
    std::mem::forget(temp);
    let settings = Arc::make_mut(&mut state);
    settings.near_provisioning_enabled = true;
    settings.near_provisioning_admission_ready = true;
    settings.near_provisioning_public_origin = Some("https://commons.example".into());
    settings.account_near_config = Some(Arc::new(trace_commons_server::config::NearConfig {
        rpc_url,
        network: "testnet".into(),
        recipient: WALLET_RECIPIENT.into(),
    }));
    settings.near_account_identity = Some(Arc::new(identity()));
    settings.db_mirror = Some(db as Arc<dyn Database>);
    state
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
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn start(
    state: &Arc<AppState>,
    account: &str,
    device_public_key: &str,
    challenge: &str,
) -> serde_json::Value {
    let (status, value) = request(
        state,
        "POST",
        "/v1/account/near/provision/start/v2",
        Some(serde_json::json!({
            "account_id":account,"device_public_key":device_public_key,
            "code_challenge":challenge,"code_challenge_method":"S256"
        })),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["recipient"], WALLET_RECIPIENT);
    assert_eq!(value["network"], "testnet");
    assert_eq!(
        value["wallet_url"],
        "https://commons.example/account/near/provision/wallet"
    );
    assert!(value["ceremony_id"].as_str().is_some());
    assert!(value["expires_at"].as_i64().is_some());
    for absent in [
        "witness",
        "issuer_url",
        "audience",
        "inference_receipt_endpoint",
    ] {
        assert!(value.get(absent).is_none(), "start leaked {absent}");
    }
    value
}

fn finish_body(
    started: &serde_json::Value,
    account: &str,
    device: &ring::signature::Ed25519KeyPair,
    wallet: &ring::signature::Ed25519KeyPair,
    verifier: &str,
) -> serde_json::Value {
    let device_public_key =
        base64::engine::general_purpose::STANDARD.encode(device.public_key().as_ref());
    let signing_bytes = base64::engine::general_purpose::STANDARD
        .decode(started["device_signing_bytes"].as_str().unwrap())
        .unwrap();
    serde_json::json!({
        "ceremony_id":started["ceremony_id"], "account_id":account,
        "device_public_key":device_public_key,"wallet_public_key":wallet_key(wallet),
        "wallet_signature":sign_wallet(started,wallet),
        "device_signature":base64::engine::general_purpose::STANDARD
            .encode(device.sign(&signing_bytes).as_ref()),
        "code_verifier":verifier,
    })
}

async fn finish(state: &Arc<AppState>, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
    request(
        state,
        "POST",
        "/v1/account/near/provision/finish/v2",
        Some(body),
        None,
    )
    .await
}

async fn row_counts(db: &PgBackend, tenant: &str) -> [i64; 6] {
    let mut client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let row = tx
        .query_one(
            "SELECT \
        (SELECT count(*) FROM trace_near_account_anchors WHERE tenant_id=$1), \
        (SELECT count(*) FROM trace_accounts WHERE tenant_id=$1), \
        (SELECT count(*) FROM trace_near_identities WHERE tenant_id=$1), \
        (SELECT count(*) FROM device_keys WHERE tenant_id=$1), \
        (SELECT count(*) FROM trace_near_provisioned_devices WHERE tenant_id=$1), \
        (SELECT count(*) FROM trace_account_inference_connections WHERE tenant_id=$1)",
            &[&tenant],
        )
        .await
        .unwrap();
    [0, 1, 2, 3, 4, 5].map(|index| row.get(index))
}

async fn global_provisioned_counts(db: &PgBackend) -> [i64; 8] {
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let row = client
        .query_one(
            "SELECT \
             (SELECT count(*) FROM trace_near_account_anchors), \
             (SELECT count(*) FROM trace_accounts), \
             (SELECT count(*) FROM trace_near_identities), \
             (SELECT count(*) FROM device_keys), \
             (SELECT count(*) FROM trace_account_principals), \
             (SELECT count(*) FROM trace_near_provisioned_devices), \
             (SELECT count(*) FROM trace_sessions), \
             (SELECT count(*) FROM trace_account_inference_connections)",
            &[],
        )
        .await
        .unwrap();
    [0, 1, 2, 3, 4, 5, 6, 7].map(|index| row.get(index))
}

async fn persisted_wallet_device_join(
    db: &PgBackend,
    finished: &serde_json::Value,
    device_public_key: &str,
    wallet_public_key: &str,
) -> i64 {
    let tenant = finished["tenant_id"].as_str().unwrap();
    let account = uuid::Uuid::parse_str(finished["account_id"].as_str().unwrap()).unwrap();
    let device_key_id = finished["device_key_id"].as_str().unwrap();
    let anchor = finished["anchor_hash"].as_str().unwrap();
    let mut client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
        &[&tenant],
    )
    .await
    .unwrap();
    tx.query_one(
        "SELECT count(*) FROM trace_near_provisioned_devices n \
         JOIN trace_accounts a USING (tenant_id,account_id) \
         JOIN device_keys d USING (tenant_id,device_key_id) \
         JOIN trace_account_principals p \
           ON p.tenant_id=n.tenant_id AND p.account_id=n.account_id \
              AND p.principal_ref=n.principal_ref \
         JOIN trace_near_account_anchors h \
           ON h.tenant_id=n.tenant_id AND h.account_id=n.account_id \
         JOIN trace_near_identities i \
           ON i.tenant_id=n.tenant_id AND i.account_id=n.account_id \
         WHERE n.tenant_id=$1 AND n.account_id=$2 AND n.device_key_id=$3 \
           AND n.anchor_hash=$4 AND h.anchor_hash=$4 \
           AND d.public_key=$5 AND d.onboarding_origin='near' \
           AND i.public_key=$6 AND i.near_account_id=$7 \
           AND p.unlinked_at IS NULL AND a.closed_at IS NULL",
        &[
            &tenant,
            &account,
            &device_key_id,
            &anchor,
            &device_public_key,
            &wallet_public_key,
            &WALLET_ACCOUNT,
        ],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_WALLET_V2_PG_TEST_URL"]
async fn wallet_v2_signed_completion_and_refusal_boundaries() {
    use std::sync::atomic::Ordering;
    let db = fresh_db().await;
    let wallet = keypair();
    let device = keypair();
    let wrong_device = keypair();
    let public_key = base64::engine::general_purpose::STANDARD.encode(device.public_key().as_ref());
    let (rpc_url, rpc_calls) = rpc_fixture(wallet_key(&wallet)).await;
    let state = state(db.clone(), rpc_url);
    let (verifier, challenge) = challenge_pair();
    let before = global_provisioned_counts(&db).await;

    let (status, legacy) = request(
        &state,
        "GET",
        "/v1/account/near/provision/capabilities",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(legacy["ready"], false, "v1 still requires the witness");
    assert!(legacy.get("witness").is_none());
    let (status, capabilities) = request(
        &state,
        "GET",
        "/v1/account/near/provision/capabilities/v2",
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(capabilities["ready"], true);
    assert_eq!(
        capabilities["wallet_start_path"],
        "/v1/account/near/provision/start/v2"
    );
    assert_eq!(
        capabilities["wallet_finish_path"],
        "/v1/account/near/provision/finish/v2"
    );
    assert_eq!(
        capabilities["inference_connection_selection_required"],
        true
    );
    for absent in [
        "witness",
        "issuer_url",
        "audience",
        "inference_receipt_endpoint",
    ] {
        assert!(
            capabilities.get(absent).is_none(),
            "capabilities leaked {absent}"
        );
    }

    let (status, _) = request(
        &state,
        "POST",
        "/v1/account/near/provision/start",
        Some(serde_json::json!({
            "account_id":WALLET_ACCOUNT,"device_public_key":public_key,
            "code_challenge":challenge,"code_challenge_method":"S256"
        })),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "v1 start still needs a witness"
    );

    // Each failed finish consumes its own ceremony. Invalid proofs must never
    // reach RPC or produce an account, session, device or connection selection.
    for variant in [
        "wallet_signature",
        "account",
        "pkce",
        "device_key",
        "device_signature",
    ] {
        let started = start(&state, WALLET_ACCOUNT, &public_key, &challenge).await;
        let mut body = finish_body(&started, WALLET_ACCOUNT, &device, &wallet, &verifier);
        match variant {
            "wallet_signature" => body["wallet_signature"] = serde_json::json!("AAAA"),
            "account" => body["account_id"] = serde_json::json!("other.testnet"),
            "pkce" => body["code_verifier"] = serde_json::json!("b".repeat(64)),
            "device_key" => {
                body["device_public_key"] = serde_json::json!(
                    base64::engine::general_purpose::STANDARD
                        .encode(wrong_device.public_key().as_ref())
                )
            }
            "device_signature" => body["device_signature"] = serde_json::json!("AAAA"),
            _ => unreachable!(),
        }
        let (status, refused) = finish(&state, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{variant}: {refused}");
        assert_eq!(rpc_calls.load(Ordering::SeqCst), 0, "{variant} reached RPC");
        assert_eq!(
            global_provisioned_counts(&db).await,
            before,
            "{variant} persisted account, identity, device, session or connection state"
        );
    }

    let started = start(&state, WALLET_ACCOUNT, &public_key, &challenge).await;
    let valid_body = finish_body(&started, WALLET_ACCOUNT, &device, &wallet, &verifier);
    let (status, finished) = finish(&state, valid_body.clone()).await;
    assert_eq!(status, StatusCode::OK, "{finished}");
    assert_eq!(rpc_calls.load(Ordering::SeqCst), 1);
    assert_eq!(finished["token_type"], "Bearer");
    assert!(
        finished["access_token"]
            .as_str()
            .unwrap()
            .starts_with("tcn1_")
    );
    for absent in [
        "witness",
        "issuer_url",
        "audience",
        "inference_receipt_endpoint",
        "connection_id",
    ] {
        assert!(finished.get(absent).is_none(), "finish leaked {absent}");
    }
    let tenant = finished["tenant_id"].as_str().unwrap();
    assert_eq!(row_counts(&db, tenant).await, [1, 1, 1, 1, 1, 0]);
    assert_eq!(
        persisted_wallet_device_join(&db, &finished, &public_key, &wallet_key(&wallet)).await,
        1,
        "the authenticated account must join the proved wallet and device"
    );
    let (status, identities) = request(
        &state,
        "GET",
        "/v1/account/near-identities",
        None,
        Some(finished["access_token"].as_str().unwrap()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{identities}");
    assert_eq!(
        identities["near_identities"][0]["public_key"],
        wallet_key(&wallet)
    );
    assert_eq!(
        identities["near_identities"][0]["near_account_id"],
        WALLET_ACCOUNT
    );

    let (status, _) = finish(&state, valid_body).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "consumed ceremony cannot replay"
    );
    assert_eq!(rpc_calls.load(Ordering::SeqCst), 1);

    let started = start(&state, WALLET_ACCOUNT, &public_key, &challenge).await;
    let (status, returned) = finish(
        &state,
        finish_body(&started, WALLET_ACCOUNT, &device, &wallet, &verifier),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{returned}");
    assert_eq!(rpc_calls.load(Ordering::SeqCst), 2);
    assert_eq!(returned["tenant_id"], finished["tenant_id"]);
    assert_eq!(returned["account_id"], finished["account_id"]);
    assert_eq!(returned["device_key_id"], finished["device_key_id"]);
    assert_eq!(returned["anchor_hash"], finished["anchor_hash"]);
    assert_eq!(row_counts(&db, tenant).await, [1, 1, 1, 1, 1, 0]);
}
