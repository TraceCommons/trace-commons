// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use base64::Engine;
use chrono::{Duration, Utc};
use ring::signature::{Ed25519KeyPair, KeyPair};
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use trace_commons_server::account_onboarding::{
    NativeProvisioningPending, PendingNearProvisioning, ProvisioningAssertion,
};
use trace_commons_server::config::{DatabaseConfig, NearConfig, SslMode};
use trace_commons_server::db::{Database, NewSession, postgres::PgBackend};

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 8,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}
fn wallet_key(key: &Ed25519KeyPair) -> String {
    format!(
        "ed25519:{}",
        bs58::encode(key.public_key().as_ref()).into_string()
    )
}
fn signature(pending: &PendingNearProvisioning, key: &Ed25519KeyPair) -> String {
    let c = pending.challenge();
    let mut bytes = 2_147_484_061_u32.to_le_bytes().to_vec();
    bytes.extend_from_slice(&(c.message.len() as u32).to_le_bytes());
    bytes.extend_from_slice(c.message.as_bytes());
    bytes.extend_from_slice(c.nonce);
    bytes.extend_from_slice(&(c.recipient.len() as u32).to_le_bytes());
    bytes.extend_from_slice(c.recipient.as_bytes());
    bytes.push(u8::from(c.callback_url.is_some()));
    if let Some(url) = c.callback_url {
        bytes.extend_from_slice(&(url.len() as u32).to_le_bytes());
        bytes.extend_from_slice(url.as_bytes());
    }
    base64::engine::general_purpose::STANDARD.encode(key.sign(&Sha256::digest(bytes)).as_ref())
}
fn hash(text: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(text.as_bytes())))
}

#[tokio::test]
async fn durable_provisioning_is_atomic_replay_safe_and_tenant_scoped() {
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(
        url.contains("127.0.0.1"),
        "test requires explicitly isolated loopback DB"
    );
    let admin = PgBackend::new(&config(url.clone())).await.unwrap();
    admin.run_migrations().await.unwrap();
    let admin_client = admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin_client.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='tc_near_runtime') THEN CREATE ROLE tc_near_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; GRANT USAGE ON SCHEMA public TO tc_near_runtime; GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA public TO tc_near_runtime; GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO tc_near_runtime;").await.unwrap();
    let mut parsed = reqwest::Url::parse(&url).unwrap();
    parsed.set_username("tc_near_runtime").unwrap();
    let db = PgBackend::new(&config(parsed.into())).await.unwrap();
    let wallet = Ed25519KeyPair::from_pkcs8(
        Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let device = Ed25519KeyPair::from_pkcs8(
        Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let public = wallet_key(&wallet);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let rpc=axum::Router::new().route("/",axum::routing::post(move || {let public=public.clone();async move {axum::Json(serde_json::json!({"result":{"keys":[{"public_key":public,"access_key":{"permission":"FullAccess"}}]}}))}}));
    let task = tokio::spawn(async move { axum::serve(listener, rpc).await.unwrap() });
    let cfg = NearConfig {
        rpc_url: format!("http://{address}/"),
        network: "testnet".into(),
        recipient: "trace.test".into(),
    };
    let account = format!("p{}.testnet", uuid::Uuid::new_v4().simple());
    let device_bytes = device.public_key().as_ref().try_into().unwrap();
    let pending = PendingNearProvisioning::issue(
        &cfg,
        &account,
        device_bytes,
        [8; 32],
        Utc::now().timestamp(),
    )
    .unwrap()
    .with_wallet_callback("https://trace.test/account/near/provision/wallet")
    .unwrap();
    let wallet_signature = signature(&pending, &wallet);
    let device_signature = base64::engine::general_purpose::STANDARD
        .encode(device.sign(&pending.device_signing_bytes()).as_ref());
    let expiry = pending.challenge().expires_at;
    let ceremony_hash = hash(&uuid::Uuid::new_v4().to_string());
    db.store_near_provisioning_ceremony(
        &ceremony_hash,
        NativeProvisioningPending {
            ceremony: pending.into_stored(),
            code_challenge: "x".repeat(43),
        },
        expiry,
    )
    .await
    .unwrap();
    // A fresh backend proves this is not an in-process ceremony store.
    let db2 = PgBackend::new(&config({
        let mut u = reqwest::Url::parse(&url).unwrap();
        u.set_username("tc_near_runtime").unwrap();
        u.into()
    }))
    .await
    .unwrap();
    assert!(
        db2.take_near_provisioning_ceremony(&hash("wrong"))
            .await
            .unwrap()
            .is_none()
    );
    let (a, b) = tokio::join!(
        db.take_near_provisioning_ceremony(&ceremony_hash),
        db2.take_near_provisioning_ceremony(&ceremony_hash)
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_ne!(a.is_some(), b.is_some());
    let stored = a.or(b).unwrap();
    let pending = PendingNearProvisioning::restore(
        stored.ceremony,
        &cfg,
        &account,
        device_bytes,
        Some("https://trace.test/account/near/provision/wallet"),
    )
    .unwrap();
    let key = wallet_key(&wallet);
    let proof = pending
        .verify(
            &cfg,
            ProvisioningAssertion {
                wallet_public_key: &key,
                wallet_signature: &wallet_signature,
                device_signature: &device_signature,
            },
            &[8; 32],
            Utc::now().timestamp(),
        )
        .await
        .unwrap();
    let result = db
        .provision_verified_near_account(
            proof,
            NewSession {
                token_hash: &hash(&uuid::Uuid::new_v4().to_string()),
                client_kind: "native",
                expires_at: Utc::now() + Duration::hours(12),
            },
        )
        .await
        .unwrap();
    let principal = format!(
        "principal_sha256:{}",
        hex::encode(Sha256::digest(format!(
            "device:{}:{}",
            result.tenant_id, result.device_key_id
        )))
    );
    assert_eq!(
        db.get_near_provisioned_anchor(&result.tenant_id, &principal)
            .await
            .unwrap(),
        Some(result.anchor_hash.clone())
    );
    assert!(
        db.get_near_provisioned_anchor("other-tenant", &principal)
            .await
            .unwrap()
            .is_none()
    );
    let device_record = db
        .get_device_key(&result.tenant_id, &result.device_key_id)
        .await
        .unwrap()
        .unwrap();
    assert!(device_record.invite_subject_hash.is_none());
    let grants: i64 = admin_client
        .query_one(
            "SELECT count(*) FROM trace_tenant_access_grants WHERE tenant_id=$1",
            &[&result.tenant_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(grants, 0);
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let visible: i64 = client
        .query_one("SELECT count(*) FROM trace_near_account_anchors", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(visible, 0, "RLS hides anchors without tenant");
    // Two fresh ceremonies for the same account race safely into one account,
    // and replay cannot silently revive a revoked device.
    let make_proof = || async {
        let p = PendingNearProvisioning::issue(
            &cfg,
            &account,
            device_bytes,
            [8; 32],
            Utc::now().timestamp(),
        )
        .unwrap();
        let sig = signature(&p, &wallet);
        let ds = base64::engine::general_purpose::STANDARD
            .encode(device.sign(&p.device_signing_bytes()).as_ref());
        p.verify(
            &cfg,
            ProvisioningAssertion {
                wallet_public_key: &key,
                wallet_signature: &sig,
                device_signature: &ds,
            },
            &[8; 32],
            Utc::now().timestamp(),
        )
        .await
        .unwrap()
    };
    let p1 = make_proof().await;
    let p2 = make_proof().await;
    let h1 = hash(&uuid::Uuid::new_v4().to_string());
    let h2 = hash(&uuid::Uuid::new_v4().to_string());
    let (one, two) = tokio::join!(
        db.provision_verified_near_account(
            p1,
            NewSession {
                token_hash: &h1,
                client_kind: "native",
                expires_at: Utc::now() + Duration::hours(12)
            }
        ),
        db2.provision_verified_near_account(
            p2,
            NewSession {
                token_hash: &h2,
                client_kind: "native",
                expires_at: Utc::now() + Duration::hours(12)
            }
        )
    );
    assert_eq!(one.unwrap().account_id, result.account_id);
    assert_eq!(two.unwrap().account_id, result.account_id);
    admin_client
        .execute(
            "UPDATE device_keys SET revoked_at=now() WHERE device_key_id=$1",
            &[&result.device_key_id],
        )
        .await
        .unwrap();
    let failed = db
        .provision_verified_near_account(
            make_proof().await,
            NewSession {
                token_hash: &hash("revoked-session"),
                client_kind: "native",
                expires_at: Utc::now() + Duration::hours(12),
            },
        )
        .await;
    assert!(failed.is_err());
    assert!(
        db.get_near_provisioned_anchor(&result.tenant_id, &principal)
            .await
            .unwrap()
            .is_none()
    );
    let sessions: i64 = admin_client
        .query_one(
            "SELECT count(*) FROM trace_sessions WHERE token_hash=$1",
            &[&hash("revoked-session")],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(sessions, 0, "rollback leaves no session");
    task.abort();
}
