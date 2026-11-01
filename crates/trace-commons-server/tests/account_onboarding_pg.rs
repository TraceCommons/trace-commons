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
use trace_commons_server::account_trust::resolve_contribution_account;
use trace_commons_server::config::{DatabaseConfig, NearConfig, SslMode};
use trace_commons_server::db::{Database, NewSession, postgres::PgBackend};
use trace_commons_server::near_account_identity::NearAccountIdentity;
use trace_commons_server::trace_artifact_kek::{KmsKeyWrapper, LocalMasterKeyWrapper};

/// Provisioning now needs a pepper and an account-name key. The identity is a
/// required argument with no default, so there is nothing to configure here
/// except real controls -- an absent one refuses rather than falling back.
fn identity() -> NearAccountIdentity {
    let crypto =
        trace_commons_server::secrets::SecretsCrypto::new(SecretString::new("a".repeat(32).into()))
            .expect("fixture SecretsCrypto");
    let kek: std::sync::Arc<dyn KmsKeyWrapper> = std::sync::Arc::new(LocalMasterKeyWrapper::new(
        crypto,
        "account-onboarding-pg-fixture",
    ));
    NearAccountIdentity::from_parts(
        Some(&base64::engine::general_purpose::STANDARD.encode([9u8; 32])),
        Some(kek),
    )
    .expect("fixture identity")
}

fn config(url: String) -> DatabaseConfig {
    config_with_resolver(url.clone(), url)
}

/// The runtime pool and the resolver pool, as two roles rather than one.
///
/// They must not be the same connection, and the difference is not cosmetic.
/// V61 gives the resolver a permissive `USING (true)` SELECT policy scoped
/// `TO trace_login_resolver`, which is what lets a session holding no tenant
/// context resolve a blind index at all. A pool connecting as any other
/// non-superuser role is left with `trace_corpus_tenant_isolation` alone,
/// whose predicate compares `tenant_id` against a `trace_current_tenant_id()`
/// that is NULL here -- so it matches nothing, and `near_anchor_tenant`
/// returns `None` for an anchor that is present and committed.
///
/// This fixture used to pass one URL for both. Every returning contributor
/// then looked new, which is not a visible failure so much as a second tenant
/// minted quietly for someone who already had one. `config.rs` states the
/// requirement (the resolver user MUST be `trace_login_resolver`) and
/// `postgres.rs` states that it is never aliased to the runtime pool; nothing
/// enforces either, so the fixture has to model it correctly on purpose.
fn config_with_resolver(url: String, resolver_url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 8,
        ssl_mode: SslMode::Prefer,
        // The anchor is a blind index and the tenant is random, so a returning
        // contributor can only be found by resolving the index with no tenant
        // context -- the same narrow resolver role V30 introduced for redeem.
        login_resolver_url: Some(SecretString::from(resolver_url)),
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

/// Serialises this file's tests against each other. The cross-tenant tests
/// assert that a refusal changes no row anywhere, with table-wide counts --
/// a refused attempt has no tenant or account to scope them by -- so a sibling
/// provisioning concurrently would read as a refusal that wrote rows.
fn serial() -> &'static tokio::sync::Mutex<()> {
    static SERIAL: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    SERIAL.get_or_init(Default::default)
}

#[tokio::test]
async fn durable_provisioning_is_atomic_replay_safe_and_tenant_scoped() {
    let _serial = serial().lock().await;
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
    // V30 creates `trace_login_resolver` NOLOGIN, so a test that wants to
    // connect as it has to grant LOGIN. Granting only SELECT on the two
    // columns V61 exposes keeps the fixture honest about what this role may
    // read: it must never see `sealed_account_name`, which is the only stored
    // copy of the contributor's account name.
    admin_client.batch_execute("ALTER ROLE trace_login_resolver LOGIN; GRANT USAGE ON SCHEMA public TO trace_login_resolver; GRANT SELECT (tenant_id, anchor_hash) ON trace_near_account_anchors TO trace_login_resolver;").await.unwrap();
    let mut parsed = reqwest::Url::parse(&url).unwrap();
    parsed.set_username("tc_near_runtime").unwrap();
    let mut resolver = reqwest::Url::parse(&url).unwrap();
    resolver.set_username("trace_login_resolver").unwrap();
    let db = PgBackend::new(&config_with_resolver(parsed.into(), resolver.into()))
        .await
        .unwrap();
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
    // The second backend stands in for a second machine racing the first, so
    // it needs the same two-role split. Handing it one URL for both pools is
    // what made the concurrent case unwinnable: its resolver saw no anchors at
    // all, so the racer it models could never recognise a contributor the
    // other had just provisioned.
    let db2 = PgBackend::new(&config_with_resolver(
        {
            let mut u = reqwest::Url::parse(&url).unwrap();
            u.set_username("tc_near_runtime").unwrap();
            u.into()
        },
        {
            let mut u = reqwest::Url::parse(&url).unwrap();
            u.set_username("trace_login_resolver").unwrap();
            u.into()
        },
    ))
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
            &identity(),
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
    let trust_account = resolve_contribution_account(&db, &result.tenant_id, &principal)
        .await
        .expect("live authenticated principal resolves");
    assert_eq!(trust_account.account_id(), result.account_id);
    assert_eq!(trust_account.tenant_id(), result.tenant_id);
    assert_eq!(
        db.get_near_provisioned_account(&result.tenant_id, &principal)
            .await
            .unwrap(),
        Some(result.account_id)
    );
    assert!(
        resolve_contribution_account(&db, "other-tenant", &principal)
            .await
            .is_err()
    );
    assert!(
        resolve_contribution_account(&db, &result.tenant_id, &hash("spoofed-principal"))
            .await
            .is_err()
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
    let (racing_one, racing_two) = (identity(), identity());
    let (one, two) = tokio::join!(
        db.provision_verified_near_account(
            p1,
            NewSession {
                token_hash: &h1,
                client_kind: "native",
                expires_at: Utc::now() + Duration::hours(12)
            },
            &racing_one
        ),
        db2.provision_verified_near_account(
            p2,
            NewSession {
                token_hash: &h2,
                client_kind: "native",
                expires_at: Utc::now() + Duration::hours(12)
            },
            &racing_two
        )
    );
    assert_eq!(one.unwrap().account_id, result.account_id);
    assert_eq!(two.unwrap().account_id, result.account_id);
    let second_device = Ed25519KeyPair::from_pkcs8(
        Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap();
    let second_bytes: [u8; 32] = second_device.public_key().as_ref().try_into().unwrap();
    let second_pending = PendingNearProvisioning::issue(
        &cfg,
        &account,
        second_bytes,
        [8; 32],
        Utc::now().timestamp(),
    )
    .unwrap();
    let second_wallet_sig = signature(&second_pending, &wallet);
    let second_device_sig = base64::engine::general_purpose::STANDARD.encode(
        second_device
            .sign(&second_pending.device_signing_bytes())
            .as_ref(),
    );
    let second_proof = second_pending
        .verify(
            &cfg,
            ProvisioningAssertion {
                wallet_public_key: &key,
                wallet_signature: &second_wallet_sig,
                device_signature: &second_device_sig,
            },
            &[8; 32],
            Utc::now().timestamp(),
        )
        .await
        .unwrap();
    let second = db
        .provision_verified_near_account(
            second_proof,
            NewSession {
                token_hash: &hash(&uuid::Uuid::new_v4().to_string()),
                client_kind: "native",
                expires_at: Utc::now() + Duration::hours(12),
            },
            &identity(),
        )
        .await
        .unwrap();
    let second_principal = format!(
        "principal_sha256:{}",
        hex::encode(Sha256::digest(format!(
            "device:{}:{}",
            second.tenant_id, second.device_key_id
        )))
    );
    assert_eq!(second.account_id, trust_account.account_id());
    assert_ne!(
        identity().index_label("testnet", &account),
        identity().login_index_label(&account),
        "equal wallet and login identifiers must occupy distinct domains"
    );
    assert_eq!(
        resolve_contribution_account(&db, &second.tenant_id, &second_principal)
            .await
            .unwrap()
            .account_id(),
        trust_account.account_id()
    );
    assert_eq!(
        resolve_contribution_account(&db2, &result.tenant_id, &principal)
            .await
            .unwrap()
            .account_id(),
        trust_account.account_id()
    );
    admin_client
        .execute(
            "UPDATE trace_accounts SET closed_at=now() WHERE tenant_id=$1 AND account_id=$2",
            &[&result.tenant_id, &result.account_id],
        )
        .await
        .unwrap();
    assert!(
        resolve_contribution_account(&db, &result.tenant_id, &principal)
            .await
            .is_err()
    );
    admin_client
        .execute(
            "UPDATE trace_accounts SET closed_at=NULL WHERE tenant_id=$1 AND account_id=$2",
            &[&result.tenant_id, &result.account_id],
        )
        .await
        .unwrap();
    admin_client
        .execute(
            "UPDATE trace_account_principals SET unlinked_at=now() WHERE tenant_id=$1 AND principal_ref=$2",
            &[&result.tenant_id, &principal],
        )
        .await
        .unwrap();
    assert!(
        resolve_contribution_account(&db, &result.tenant_id, &principal)
            .await
            .is_err()
    );
    admin_client
        .execute(
            "UPDATE trace_account_principals SET unlinked_at=NULL WHERE tenant_id=$1 AND principal_ref=$2",
            &[&result.tenant_id, &principal],
        )
        .await
        .unwrap();
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
            &identity(),
        )
        .await;
    assert!(failed.is_err());
    assert!(
        db.get_near_provisioned_anchor(&result.tenant_id, &principal)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        resolve_contribution_account(&db, &result.tenant_id, &principal)
            .await
            .is_err()
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

/// A migrated database, a non-superuser runtime pool, and a superuser handle
/// for seeding rows the runtime role could never write for another tenant.
async fn provisioning_fixture(url: &str) -> (PgBackend, deadpool_postgres::Object) {
    let admin = PgBackend::new(&config(url.to_owned())).await.unwrap();
    admin.run_migrations().await.unwrap();
    let admin_client = admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin_client.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='tc_near_runtime') THEN CREATE ROLE tc_near_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; GRANT USAGE ON SCHEMA public TO tc_near_runtime; GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA public TO tc_near_runtime; GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO tc_near_runtime;").await.unwrap();
    admin_client.batch_execute("ALTER ROLE trace_login_resolver LOGIN; GRANT USAGE ON SCHEMA public TO trace_login_resolver; GRANT SELECT (tenant_id, anchor_hash) ON trace_near_account_anchors TO trace_login_resolver;").await.unwrap();
    let mut runtime = reqwest::Url::parse(url).unwrap();
    runtime.set_username("tc_near_runtime").unwrap();
    let mut resolver = reqwest::Url::parse(url).unwrap();
    resolver.set_username("trace_login_resolver").unwrap();
    let db = PgBackend::new(&config_with_resolver(runtime.into(), resolver.into()))
        .await
        .unwrap();
    (db, admin_client)
}

fn fresh_keypair() -> Ed25519KeyPair {
    Ed25519KeyPair::from_pkcs8(
        Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap()
            .as_ref(),
    )
    .unwrap()
}

/// Register `device` the way a legacy invite redemption did: under a
/// `tenant-...` tenant, origin `invite`. Returns that tenant.
async fn seed_legacy_device(admin: &deadpool_postgres::Object, device: &Ed25519KeyPair) -> String {
    let tenant = format!("tenant-legacy-{}", uuid::Uuid::new_v4().simple());
    let device_key_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
        device.public_key().as_ref(),
    );
    admin
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,$4,'invite')",
            &[
                &device_key_id,
                &tenant,
                &base64::engine::general_purpose::STANDARD.encode(device.public_key().as_ref()),
                &hash(&format!("legacy-invite-{tenant}")),
            ],
        )
        .await
        .unwrap();
    tenant
}

/// Every row provisioning writes, table-wide, plus the tenant(s) holding the
/// device key.
async fn provisioning_snapshot(
    admin: &deadpool_postgres::Object,
    device: &Ed25519KeyPair,
) -> (Vec<i64>, Vec<String>) {
    let device_key_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
        device.public_key().as_ref(),
    );
    let row = admin
        .query_one(
            "SELECT (SELECT count(*) FROM trace_near_account_anchors), \
                    (SELECT count(*) FROM trace_accounts), \
                    (SELECT count(*) FROM trace_near_identities), \
                    (SELECT count(*) FROM device_keys), \
                    (SELECT count(*) FROM trace_account_principals), \
                    (SELECT count(*) FROM trace_near_provisioned_devices), \
                    (SELECT count(*) FROM trace_sessions), \
                    (SELECT count(*) FROM trace_account_audit), \
                    (SELECT count(*) FROM trace_tenants)",
            &[],
        )
        .await
        .unwrap();
    let counts = (0..9).map(|i| row.get::<_, i64>(i)).collect();
    let tenants = admin
        .query(
            "SELECT tenant_id FROM device_keys WHERE device_key_id=$1",
            &[&device_key_id],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get(0))
        .collect();
    (counts, tenants)
}

fn refusal_label<T: std::fmt::Debug>(
    result: Result<T, trace_commons_server::error::DatabaseError>,
) -> String {
    match result {
        Err(trace_commons_server::error::DatabaseError::Pool(label)) => label,
        other => panic!("expected a named Pool refusal, got {other:?}"),
    }
}

fn native_session(token_hash: &str) -> NewSession<'_> {
    NewSession {
        token_hash,
        client_kind: "native",
        expires_at: Utc::now() + Duration::hours(12),
    }
}

/// **A device key already registered to another tenant must not be quietly
/// kept there by NEAR wallet provisioning.**
///
/// `device_keys.device_key_id` is a global primary key. Provisioning inserts
/// the device `ON CONFLICT DO NOTHING`, so a key that a legacy invite already
/// registered under `tenant-...` is never inserted under the NEAR tenant. Had
/// provisioning then reported success, the contributor would hold a NEAR
/// session while every upload their device signs kept authenticating into the
/// legacy tenant. The refusal must be named -- so "this device belongs to
/// another tenant" is distinguishable from every other refusal -- carry no
/// identifier, and leave every row as it was.
#[tokio::test]
async fn wallet_provisioning_refuses_a_device_key_registered_to_another_tenant() {
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let wallet = fresh_keypair();
    let device = fresh_keypair();
    let legacy_tenant = seed_legacy_device(&admin, &device).await;

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
    let key = wallet_key(&wallet);
    let proof_for = async |device: &Ed25519KeyPair| {
        let device_bytes: [u8; 32] = device.public_key().as_ref().try_into().unwrap();
        let pending = PendingNearProvisioning::issue(
            &cfg,
            &account,
            device_bytes,
            [8; 32],
            Utc::now().timestamp(),
        )
        .unwrap();
        let wallet_signature = signature(&pending, &wallet);
        let device_signature = base64::engine::general_purpose::STANDARD
            .encode(device.sign(&pending.device_signing_bytes()).as_ref());
        pending
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
            .unwrap()
    };
    let token = |label: &str| hash(&format!("{label}-{}", uuid::Uuid::new_v4()));

    // First contact: the account has no tenant yet, so one is minted.
    let before = provisioning_snapshot(&admin, &device).await;
    assert_eq!(before.1, vec![legacy_tenant.clone()]);
    let t = token("wallet-cross-tenant");
    let refused = db
        .provision_verified_near_account(proof_for(&device).await, native_session(&t), &identity())
        .await;
    let after = provisioning_snapshot(&admin, &device).await;
    assert_eq!(
        after.1,
        vec![legacy_tenant.clone()],
        "the device key must still belong to exactly the legacy tenant"
    );
    assert_eq!(
        refusal_label(refused),
        "near_provisioning_device_key_registered_elsewhere",
        "a device registered to another tenant must be refused by name"
    );
    assert_eq!(after.0, before.0, "a refused provisioning changes no rows");

    // The same account provisions normally from a device nobody else holds,
    // and a retry with that device is the idempotent success it always was.
    let fresh_device = fresh_keypair();
    let t = token("wallet-fresh");
    let first = db
        .provision_verified_near_account(
            proof_for(&fresh_device).await,
            native_session(&t),
            &identity(),
        )
        .await
        .expect("a device registered nowhere provisions");
    let t = token("wallet-retry");
    let retry = db
        .provision_verified_near_account(
            proof_for(&fresh_device).await,
            native_session(&t),
            &identity(),
        )
        .await
        .expect("a retry from the same device and account is idempotent");
    assert_eq!(retry.tenant_id, first.tenant_id);
    assert_eq!(retry.account_id, first.account_id);
    assert_eq!(retry.device_key_id, first.device_key_id);

    // Same tenant, different account: the device's principal is re-pointed at
    // another account in the contributor's own tenant. A retry must neither
    // keep that link silently nor move it back.
    let principal = format!(
        "principal_sha256:{}",
        hex::encode(Sha256::digest(format!(
            "device:{}:{}",
            first.tenant_id, first.device_key_id
        )))
    );
    let other_account = uuid::Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&first.tenant_id, &other_account],
        )
        .await
        .unwrap();
    admin
        .execute(
            "UPDATE trace_account_principals SET account_id=$3 WHERE tenant_id=$1 AND principal_ref=$2",
            &[&first.tenant_id, &principal, &other_account],
        )
        .await
        .unwrap();
    let before = provisioning_snapshot(&admin, &fresh_device).await;
    let t = token("wallet-principal-elsewhere");
    let refused = db
        .provision_verified_near_account(
            proof_for(&fresh_device).await,
            native_session(&t),
            &identity(),
        )
        .await;
    assert_eq!(
        refusal_label(refused),
        "near_provisioning_device_bound_to_other_account"
    );
    assert_eq!(provisioning_snapshot(&admin, &fresh_device).await, before);
    let linked: uuid::Uuid = admin
        .query_one(
            "SELECT account_id FROM trace_account_principals WHERE tenant_id=$1 AND principal_ref=$2",
            &[&first.tenant_id, &principal],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(linked, other_account, "the refused retry moved nothing");
    admin
        .execute(
            "UPDATE trace_account_principals SET account_id=$3 WHERE tenant_id=$1 AND principal_ref=$2",
            &[&first.tenant_id, &principal, &first.account_id],
        )
        .await
        .unwrap();

    // The same wallet key proving a second NEAR account: the key is bound to
    // the first account's tenant, so the second is refused by name.
    let second_account = format!("q{}.testnet", uuid::Uuid::new_v4().simple());
    let second_device = fresh_keypair();
    let second_bytes: [u8; 32] = second_device.public_key().as_ref().try_into().unwrap();
    let pending = PendingNearProvisioning::issue(
        &cfg,
        &second_account,
        second_bytes,
        [8; 32],
        Utc::now().timestamp(),
    )
    .unwrap();
    let wallet_signature = signature(&pending, &wallet);
    let device_signature = base64::engine::general_purpose::STANDARD
        .encode(second_device.sign(&pending.device_signing_bytes()).as_ref());
    let second_proof = pending
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
    let before = provisioning_snapshot(&admin, &second_device).await;
    let t = token("wallet-key-elsewhere");
    let refused = db
        .provision_verified_near_account(second_proof, native_session(&t), &identity())
        .await;
    assert_eq!(
        refusal_label(refused),
        "near_provisioning_wallet_key_registered_elsewhere"
    );
    assert_eq!(provisioning_snapshot(&admin, &second_device).await, before);

    // Returning contributor: the account now has a tenant, and the legacy
    // device is still refused through the same insert.
    let before = provisioning_snapshot(&admin, &device).await;
    let t = token("wallet-cross-tenant-returning");
    let refused = db
        .provision_verified_near_account(proof_for(&device).await, native_session(&t), &identity())
        .await;
    assert_eq!(
        refusal_label(refused),
        "near_provisioning_device_key_registered_elsewhere"
    );
    assert_eq!(provisioning_snapshot(&admin, &device).await, before);
    task.abort();
}

/// The NEAR AI login sibling of the wallet test above: same insert, same
/// global key, same named refusal.
#[tokio::test]
async fn near_ai_login_provisioning_refuses_a_device_key_registered_to_another_tenant() {
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let device = fresh_keypair();
    let legacy_tenant = seed_legacy_device(&admin, &device).await;

    let subject = format!("auth0|cross-tenant-{}", uuid::Uuid::new_v4().simple());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = axum::Router::new().route(
        "/users/me",
        axum::routing::get(move || {
            let subject = subject.clone();
            async move { axum::Json(serde_json::json!({"id": subject, "auth_provider": "github"})) }
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let login = trace_commons_server::near_ai_login::introspect_login(
        &base,
        &SecretString::from("stub-token".to_string()),
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("stub introspection");
    let token = |label: &str| hash(&format!("{label}-{}", uuid::Uuid::new_v4()));
    let bytes =
        |device: &Ed25519KeyPair| -> [u8; 32] { device.public_key().as_ref().try_into().unwrap() };

    let before = provisioning_snapshot(&admin, &device).await;
    let t = token("login-cross-tenant");
    let refused = db
        .provision_near_ai_login(
            &login,
            &bytes(&device),
            native_session(&t),
            &identity(),
            None,
        )
        .await;
    let after = provisioning_snapshot(&admin, &device).await;
    assert_eq!(
        after.1,
        vec![legacy_tenant.clone()],
        "the device key must still belong to exactly the legacy tenant"
    );
    assert_eq!(
        refusal_label(refused),
        "near_provisioning_device_key_registered_elsewhere"
    );
    assert_eq!(after.0, before.0, "a refused provisioning changes no rows");

    let fresh_device = fresh_keypair();
    let t = token("login-fresh");
    let first = db
        .provision_near_ai_login(
            &login,
            &bytes(&fresh_device),
            native_session(&t),
            &identity(),
            None,
        )
        .await
        .expect("a device registered nowhere provisions");
    let t = token("login-retry");
    let retry = db
        .provision_near_ai_login(
            &login,
            &bytes(&fresh_device),
            native_session(&t),
            &identity(),
            None,
        )
        .await
        .expect("a retry from the same device and account is idempotent");
    assert_eq!(retry.tenant_id, first.tenant_id);
    assert_eq!(retry.account_id, first.account_id);

    let before = provisioning_snapshot(&admin, &device).await;
    let t = token("login-cross-tenant-returning");
    let refused = db
        .provision_near_ai_login(
            &login,
            &bytes(&device),
            native_session(&t),
            &identity(),
            None,
        )
        .await;
    assert_eq!(
        refusal_label(refused),
        "near_provisioning_device_key_registered_elsewhere"
    );
    assert_eq!(provisioning_snapshot(&admin, &device).await, before);
    task.abort();
}

// --- NEAR AI sign-up writes the contributor grant (owner decision 2026-10-24) --
//
// Invite enrollment writes a contributor tenant access grant for the device
// principal from the instance's policy template, and the issuer reads that
// grant as the device's claim ceiling. NEAR AI sign-up wrote none, so every
// NEAR AI account's claims got the floor (no `model_training`) whatever the
// contributor consented to. These pin the grant the sign-up now writes, its
// idempotency, the issuer-side backfill for accounts provisioned before it,
// and the claim the issuer then mints.

const SIGNUP_TEMPLATE: &str = r#"{"policy_version":"signup-test-v1",
    "allowed_consent_scopes":["debugging_evaluation","ranking_training","model_training","public_attribution"],
    "allowed_uses":["debugging","evaluation","ranking_model_training","model_training","aggregate_analytics"]}"#;

/// Narrower than [`SIGNUP_TEMPLATE`]: a re-provisioning under it must not
/// narrow (or, the other way round, widen) a grant already written.
const NARROW_TEMPLATE: &str = r#"{"policy_version":"signup-test-narrow",
    "allowed_consent_scopes":["debugging_evaluation"],
    "allowed_uses":["debugging"]}"#;

fn signup_template(
    value: &str,
) -> trace_commons_server::near_ai_signup_policy::NearAiSignupPolicyTemplate {
    trace_commons_server::near_ai_signup_policy::NearAiSignupPolicyTemplate::parse(value)
        .expect("fixture template parses")
}

/// A verified NEAR AI login for a fresh subject, from a stub `/users/me`.
async fn stub_near_ai_login(
    label: &str,
) -> (
    trace_commons_server::near_ai_login::VerifiedNearAiLogin,
    tokio::task::JoinHandle<()>,
) {
    let subject = format!("auth0|{label}-{}", uuid::Uuid::new_v4().simple());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = axum::Router::new().route(
        "/users/me",
        axum::routing::get(move || {
            let subject = subject.clone();
            async move { axum::Json(serde_json::json!({"id": subject, "auth_provider": "google"})) }
        }),
    );
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let login = trace_commons_server::near_ai_login::introspect_login(
        &base,
        &SecretString::from("stub-token".to_string()),
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("stub introspection");
    (login, task)
}

fn device_bytes(device: &Ed25519KeyPair) -> [u8; 32] {
    device.public_key().as_ref().try_into().unwrap()
}

fn session_hash(label: &str) -> String {
    hash(&format!("{label}-{}", uuid::Uuid::new_v4()))
}

/// The grant rows for one device principal, read past RLS:
/// `(status, allowed_consent_scopes, allowed_uses, metadata source)`.
async fn signup_grants(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    device_key_id: &str,
) -> Vec<(String, serde_json::Value, serde_json::Value, Option<String>)> {
    let principal =
        trace_commons_server::account_trust::device_principal_ref(tenant, device_key_id);
    admin
        .query(
            "SELECT status, allowed_consent_scopes, allowed_uses, metadata_json->>'source'
               FROM trace_tenant_access_grants
              WHERE tenant_id = $1 AND principal_ref = $2 AND role = 'contributor'",
            &[&tenant, &principal],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2), row.get(3)))
        .collect()
}

fn template_grant_row(
    value: &str,
) -> (String, serde_json::Value, serde_json::Value, Option<String>) {
    let template = signup_template(value);
    (
        "active".to_string(),
        serde_json::json!(template.allowed_consent_scopes()),
        serde_json::json!(template.allowed_uses()),
        Some("near_ai_signup".to_string()),
    )
}

#[tokio::test]
async fn near_ai_sign_up_writes_the_contributor_grant_from_the_template_once() {
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let (login, task) = stub_near_ai_login("signup-grant").await;
    let template = signup_template(SIGNUP_TEMPLATE);
    let device = fresh_keypair();

    let t = session_hash("signup");
    let first = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&device),
            native_session(&t),
            &identity(),
            Some(&template),
        )
        .await
        .expect("sign-up provisions");
    assert_eq!(
        signup_grants(&admin, &first.tenant_id, &first.device_key_id).await,
        vec![template_grant_row(SIGNUP_TEMPLATE)],
        "sign-up writes one active contributor grant carrying the template"
    );

    // Signing in again, under the same template or a different one, neither
    // duplicates the grant nor rewrites it.
    for value in [SIGNUP_TEMPLATE, NARROW_TEMPLATE] {
        let t = session_hash("signin");
        let again = db
            .provision_near_ai_login(
                &login,
                &device_bytes(&device),
                native_session(&t),
                &identity(),
                Some(&signup_template(value)),
            )
            .await
            .expect("a returning sign-in provisions");
        assert_eq!(again.tenant_id, first.tenant_id);
        assert_eq!(
            signup_grants(&admin, &first.tenant_id, &first.device_key_id).await,
            vec![template_grant_row(SIGNUP_TEMPLATE)],
            "re-provisioning leaves the first grant exactly as written"
        );
    }

    // A revoked grant stays revoked: provisioning never revives it.
    let principal = trace_commons_server::account_trust::device_principal_ref(
        &first.tenant_id,
        &first.device_key_id,
    );
    admin
        .execute(
            "UPDATE trace_tenant_access_grants SET status = 'revoked', revoked_at = now()
              WHERE tenant_id = $1 AND principal_ref = $2",
            &[&first.tenant_id, &principal],
        )
        .await
        .unwrap();
    let t = session_hash("after-revoke");
    db.provision_near_ai_login(
        &login,
        &device_bytes(&device),
        native_session(&t),
        &identity(),
        Some(&template),
    )
    .await
    .expect("a sign-in after revocation still provisions");
    let rows = signup_grants(&admin, &first.tenant_id, &first.device_key_id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "revoked", "a revoked grant is never revived");

    // No template: no grant, so the issuer keeps the floor.
    let (other_login, other_task) = stub_near_ai_login("signup-no-template").await;
    let other_device = fresh_keypair();
    let t = session_hash("no-template");
    let other = db
        .provision_near_ai_login(
            &other_login,
            &device_bytes(&other_device),
            native_session(&t),
            &identity(),
            None,
        )
        .await
        .expect("sign-up without a template provisions");
    assert!(
        signup_grants(&admin, &other.tenant_id, &other.device_key_id)
            .await
            .is_empty(),
        "without a template sign-up writes no grant"
    );
    task.abort();
    other_task.abort();
}

#[tokio::test]
async fn a_near_ai_account_from_before_the_grant_is_backfilled_once_and_nothing_else_is() {
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let (login, task) = stub_near_ai_login("signup-backfill").await;
    let template = signup_template(SIGNUP_TEMPLATE);
    let device = fresh_keypair();
    let t = session_hash("pre-grant");
    // Provisioned the way every NEAR AI account was before this change.
    let account = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&device),
            native_session(&t),
            &identity(),
            None,
        )
        .await
        .unwrap();
    assert!(
        signup_grants(&admin, &account.tenant_id, &account.device_key_id)
            .await
            .is_empty()
    );

    for round in 0..2 {
        assert!(
            db.backfill_near_ai_signup_device_grant(
                &account.tenant_id,
                &account.device_key_id,
                &template,
                Utc::now()
            )
            .await
            .unwrap(),
            "a live NEAR AI device is eligible (round {round})"
        );
        assert_eq!(
            signup_grants(&admin, &account.tenant_id, &account.device_key_id).await,
            vec![template_grant_row(SIGNUP_TEMPLATE)],
            "the backfill writes the sign-up grant once (round {round})"
        );
    }
    // A later, different template does not rewrite it.
    db.backfill_near_ai_signup_device_grant(
        &account.tenant_id,
        &account.device_key_id,
        &signup_template(NARROW_TEMPLATE),
        Utc::now(),
    )
    .await
    .unwrap();
    assert_eq!(
        signup_grants(&admin, &account.tenant_id, &account.device_key_id).await,
        vec![template_grant_row(SIGNUP_TEMPLATE)]
    );

    // An invite-enrolled device is not a NEAR AI sign-up: no grant.
    let invite_device = fresh_keypair();
    let invite_tenant = seed_legacy_device(&admin, &invite_device).await;
    let invite_device_key_id =
        trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            invite_device.public_key().as_ref(),
        );
    assert!(
        !db.backfill_near_ai_signup_device_grant(
            &invite_tenant,
            &invite_device_key_id,
            &template,
            Utc::now()
        )
        .await
        .unwrap()
    );
    assert!(
        signup_grants(&admin, &invite_tenant, &invite_device_key_id)
            .await
            .is_empty()
    );

    // A revoked NEAR AI device is not eligible either.
    let (revoked_login, revoked_task) = stub_near_ai_login("signup-backfill-revoked").await;
    let revoked_device = fresh_keypair();
    let t = session_hash("revoked-device");
    let revoked = db
        .provision_near_ai_login(
            &revoked_login,
            &device_bytes(&revoked_device),
            native_session(&t),
            &identity(),
            None,
        )
        .await
        .unwrap();
    admin
        .execute(
            "UPDATE device_keys SET revoked_at = now() WHERE device_key_id = $1",
            &[&revoked.device_key_id],
        )
        .await
        .unwrap();
    assert!(
        !db.backfill_near_ai_signup_device_grant(
            &revoked.tenant_id,
            &revoked.device_key_id,
            &template,
            Utc::now()
        )
        .await
        .unwrap()
    );
    assert!(
        signup_grants(&admin, &revoked.tenant_id, &revoked.device_key_id)
            .await
            .is_empty()
    );

    // A device id under the wrong tenant finds nothing (RLS-scoped lookup).
    assert!(
        !db.backfill_near_ai_signup_device_grant(
            &invite_tenant,
            &account.device_key_id,
            &template,
            Utc::now()
        )
        .await
        .unwrap()
    );
    task.abort();
    revoked_task.abort();
}

/// A NEAR AI-provisioned device's claim, through the real issuer router over
/// the real database: the consented `model_training` is granted when the
/// template allows it, a request past the template is clipped, and with no
/// template the floor stays.
#[tokio::test]
async fn a_near_ai_device_claim_honours_consent_up_to_the_template() {
    use axum::body::{Body, to_bytes};
    use std::sync::Arc;
    use tower::ServiceExt as _;
    use trace_commons_server::trace_upload_claim_issuer::{
        TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION, TraceUploadClaimIssuerConfig,
        generate_upload_claim_keypair, trace_upload_claim_issuer_router,
    };

    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let db = Arc::new(db);
    let issuer_keys = generate_upload_claim_keypair().unwrap();
    let workload_keys = generate_upload_claim_keypair().unwrap();
    let config = |template: Option<&str>| TraceUploadClaimIssuerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        signing_private_key_pem: issuer_keys.private_key_pem.clone(),
        signing_public_key_pem: issuer_keys.public_key_pem.clone(),
        signing_kid: "issuer-test".into(),
        issuer: "trace-commons-upload-issuer".into(),
        audience: "trace-commons-upload".into(),
        max_ttl_seconds: 300,
        workload_public_key_pem: workload_keys.public_key_pem.clone(),
        workload_issuer: None,
        workload_audience: None,
        tenant_access_grant_db: Some(db.clone() as Arc<dyn Database>),
        require_tenant_access_grants: false,
        shutdown_grace_seconds: 30,
        request_timeout_seconds: 10,
        max_request_bytes: 64 * 1024,
        allowlist_source: None,
        allowlist_refresh_interval_seconds: 60,
        allowlist_max_stale_seconds: 3600,
        onboarding_device_key_db: Some(db.clone() as Arc<dyn Database>),
        onboarding_ingest_url: Some("https://ingest.example".into()),
        onboarding_community_url: None,
        onboarding_profile_url: None,
        onboarding_leaderboard_url: None,
        admin_bind: None,
        invite_admin_backend: None,
        invite_admin_registry: None,
        invite_registry_authoritative: false,
        near_ai_signup_policy_template: template.map(signup_template),
    };
    let claim = |config: TraceUploadClaimIssuerConfig,
                 tenant: String,
                 device_key_id: String,
                 device: &Ed25519KeyPair,
                 scopes: Vec<&str>| {
        // What the contributor daemon sends: its consented scopes and the uses
        // they imply (`consent::scopes_to_allowed_uses`).
        let mut uses: Vec<&str> = Vec::new();
        for scope in &scopes {
            uses.extend(match *scope {
                "debugging_evaluation" => &["debugging", "evaluation"][..],
                "benchmark_only" => &["benchmark_generation"][..],
                "ranking_training" => &["ranking_model_training"][..],
                "model_training" => &["model_training"][..],
                _ => &[][..],
            });
        }
        uses.push("aggregate_analytics");
        let body = serde_json::json!({
            "schema_version": TRACE_UPLOAD_CLAIM_REQUEST_SCHEMA_VERSION,
            "tenant_id": tenant,
            "audience": "trace-commons-upload",
            "consent_scopes": scopes,
            "allowed_uses": uses,
            "requested_at": Utc::now(),
        })
        .to_string();
        let signature =
            base64::engine::general_purpose::STANDARD.encode(device.sign(body.as_bytes()).as_ref());
        let router = trace_upload_claim_issuer_router(config).expect("router");
        async move {
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .method(axum::http::Method::POST)
                        .uri("/v1/trace-upload-claim")
                        .header("content-type", "application/json")
                        .header("x-trace-device-key-id", device_key_id)
                        .header("x-trace-device-signature", signature)
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(status, axum::http::StatusCode::OK, "{body}");
            let set = |key: &str| {
                body[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap().to_string())
                    .collect::<std::collections::BTreeSet<_>>()
            };
            (set("consent_scopes"), set("allowed_uses"))
        }
    };
    let names = |values: &[&str]| {
        values
            .iter()
            .map(|v| v.to_string())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let consented = vec![
        "debugging_evaluation",
        "benchmark_only",
        "ranking_training",
        "model_training",
        "public_attribution",
    ];

    // Signed up with the template in force.
    let (login, task) = stub_near_ai_login("claim-signup").await;
    let device = fresh_keypair();
    let t = session_hash("claim-signup");
    let signed_up = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&device),
            native_session(&t),
            &identity(),
            Some(&signup_template(SIGNUP_TEMPLATE)),
        )
        .await
        .unwrap();
    let (scopes, uses) = claim(
        config(Some(SIGNUP_TEMPLATE)),
        signed_up.tenant_id.clone(),
        signed_up.device_key_id.clone(),
        &device,
        consented.clone(),
    )
    .await;
    assert_eq!(
        scopes,
        names(&[
            "debugging_evaluation",
            "ranking_training",
            "model_training",
            "public_attribution"
        ]),
        "consent is granted up to the template; benchmark_only is clipped"
    );
    assert_eq!(
        uses,
        names(&[
            "debugging",
            "evaluation",
            "ranking_model_training",
            "model_training",
            "aggregate_analytics"
        ]),
        "model_training reaches the claim; benchmark_generation is clipped"
    );

    // Provisioned before the grant existed, issuer without a template: the
    // floor, exactly as before.
    let (old_login, old_task) = stub_near_ai_login("claim-pre-grant").await;
    let old_device = fresh_keypair();
    let t = session_hash("claim-pre-grant");
    let old = db
        .provision_near_ai_login(
            &old_login,
            &device_bytes(&old_device),
            native_session(&t),
            &identity(),
            None,
        )
        .await
        .unwrap();
    let floor = claim(
        config(None),
        old.tenant_id.clone(),
        old.device_key_id.clone(),
        &old_device,
        consented.clone(),
    )
    .await;
    assert_eq!(
        floor,
        (
            names(&["debugging_evaluation", "public_attribution"]),
            names(&["debugging", "evaluation", "aggregate_analytics"])
        ),
        "no grant and no template: the floor"
    );
    assert!(
        signup_grants(&admin, &old.tenant_id, &old.device_key_id)
            .await
            .is_empty()
    );

    // The same account's next claim once the issuer has the template: the
    // issuer writes the missing grant and the claim carries model_training.
    let (_, uses) = claim(
        config(Some(SIGNUP_TEMPLATE)),
        old.tenant_id.clone(),
        old.device_key_id.clone(),
        &old_device,
        consented.clone(),
    )
    .await;
    assert!(uses.contains("model_training"), "{uses:?}");
    assert_eq!(
        signup_grants(&admin, &old.tenant_id, &old.device_key_id).await,
        vec![template_grant_row(SIGNUP_TEMPLATE)],
        "the claim backfilled the sign-up grant"
    );

    // An invite-enrolled device without a grant is not backfilled: floor.
    let invite_device = fresh_keypair();
    let invite_tenant = seed_legacy_device(&admin, &invite_device).await;
    let invite_device_key_id =
        trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            invite_device.public_key().as_ref(),
        );
    let (_, uses) = claim(
        config(Some(SIGNUP_TEMPLATE)),
        invite_tenant.clone(),
        invite_device_key_id.clone(),
        &invite_device,
        consented.clone(),
    )
    .await;
    assert_eq!(
        uses,
        names(&["debugging", "evaluation", "aggregate_analytics"])
    );
    assert!(
        signup_grants(&admin, &invite_tenant, &invite_device_key_id)
            .await
            .is_empty()
    );

    // A revoked sign-up grant is not rewritten by the backfill: floor.
    let principal = trace_commons_server::account_trust::device_principal_ref(
        &signed_up.tenant_id,
        &signed_up.device_key_id,
    );
    admin
        .execute(
            "UPDATE trace_tenant_access_grants SET status = 'revoked', revoked_at = now()
              WHERE tenant_id = $1 AND principal_ref = $2",
            &[&signed_up.tenant_id, &principal],
        )
        .await
        .unwrap();
    let (_, uses) = claim(
        config(Some(SIGNUP_TEMPLATE)),
        signed_up.tenant_id.clone(),
        signed_up.device_key_id.clone(),
        &device,
        consented,
    )
    .await;
    assert_eq!(
        uses,
        names(&["debugging", "evaluation", "aggregate_analytics"]),
        "a revoked grant leaves the floor"
    );
    task.abort();
    old_task.abort();
}

/// Two laptops signing in with one NEAR AI login land on one account, and
/// that account can list, rename and revoke them; another account can do
/// none of it.
#[tokio::test]
async fn a_second_laptop_joins_the_account_and_each_device_can_be_listed_renamed_and_revoked() {
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let (login, task) = stub_near_ai_login("devices").await;
    let template = signup_template(SIGNUP_TEMPLATE);

    let laptop = fresh_keypair();
    let laptop_session = session_hash("devices-laptop");
    let first = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&laptop),
            native_session(&laptop_session),
            &identity(),
            Some(&template),
        )
        .await
        .expect("the first laptop provisions");
    let second_laptop = fresh_keypair();
    let second_session = session_hash("devices-second");
    let second = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&second_laptop),
            native_session(&second_session),
            &identity(),
            Some(&template),
        )
        .await
        .expect("the second laptop provisions");
    assert_eq!(second.tenant_id, first.tenant_id);
    assert_eq!(
        second.account_id, first.account_id,
        "one NEAR AI login is one account, whichever laptop signs in"
    );
    assert_ne!(second.device_key_id, first.device_key_id);
    let (tenant, account) = (first.tenant_id.clone(), first.account_id);

    let devices = db
        .list_account_devices(&tenant, account, Some(&laptop_session))
        .await
        .unwrap();
    let ids: Vec<&str> = devices.iter().map(|d| d.device_key_id.as_str()).collect();
    assert_eq!(
        ids,
        vec![first.device_key_id.as_str(), second.device_key_id.as_str()]
    );
    assert!(devices.iter().all(|d| d.onboarding_origin == "near_ai"));
    assert!(devices.iter().all(|d| d.last_seen_at.is_some()));
    assert_eq!(
        devices.iter().map(|d| d.this_device).collect::<Vec<_>>(),
        vec![true, false],
        "only the device whose session asked is this device"
    );
    assert!(
        db.list_account_devices(&tenant, account, None)
            .await
            .unwrap()
            .iter()
            .all(|d| !d.this_device)
    );

    // Rename, and clear.
    assert!(
        db.rename_account_device(&tenant, account, &second.device_key_id, Some("zaki-server"))
            .await
            .unwrap()
    );
    let labels: Vec<Option<String>> = db
        .list_account_devices(&tenant, account, None)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.label)
        .collect();
    assert_eq!(labels, vec![None, Some("zaki-server".to_string())]);

    // Another account sees none of these devices and changes nothing.
    let (other_login, other_task) = stub_near_ai_login("devices-other").await;
    let other_device = fresh_keypair();
    let other = db
        .provision_near_ai_login(
            &other_login,
            &device_bytes(&other_device),
            native_session(&session_hash("devices-other")),
            &identity(),
            Some(&template),
        )
        .await
        .expect("another account provisions");
    let other_ids: Vec<String> = db
        .list_account_devices(&other.tenant_id, other.account_id, None)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.device_key_id)
        .collect();
    assert_eq!(other_ids, vec![other.device_key_id.clone()]);
    for tenant_id in [&tenant, &other.tenant_id] {
        assert!(
            !db.rename_account_device(tenant_id, other.account_id, &second.device_key_id, None)
                .await
                .unwrap()
        );
        assert_eq!(
            db.revoke_account_device(
                tenant_id,
                other.account_id,
                &second.device_key_id,
                "account-actor:other",
                None
            )
            .await
            .unwrap(),
            trace_commons_server::db::RevokeAccountDeviceResult::default(),
            "a foreign device is not found and nothing changes"
        );
    }

    // Revoke the second laptop: its key, grant and session end; the first
    // laptop is untouched.
    let result = db
        .revoke_account_device(
            &tenant,
            account,
            &second.device_key_id,
            "account-actor:me",
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        result,
        trace_commons_server::db::RevokeAccountDeviceResult {
            removed: true,
            sessions_revoked: 1,
            grants_revoked: 1,
            unattributed_sessions_revoked: 0,
        }
    );
    let remaining: Vec<String> = db
        .list_account_devices(&tenant, account, None)
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.device_key_id)
        .collect();
    assert_eq!(remaining, vec![first.device_key_id.clone()]);
    let key_revoked: bool = admin
        .query_one(
            "SELECT revoked_at IS NOT NULL FROM device_keys WHERE device_key_id = $1",
            &[&second.device_key_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(key_revoked);
    let grants = signup_grants(&admin, &tenant, &second.device_key_id).await;
    assert_eq!(grants.len(), 1);
    assert_eq!(grants[0].0, "revoked");
    assert_eq!(
        signup_grants(&admin, &tenant, &first.device_key_id).await,
        vec![template_grant_row(SIGNUP_TEMPLATE)]
    );
    assert!(
        db.validate_session(&tenant, &second_session)
            .await
            .unwrap()
            .is_none(),
        "the revoked device's session no longer validates"
    );
    assert!(
        db.validate_session(&tenant, &laptop_session)
            .await
            .unwrap()
            .is_some(),
        "the other laptop stays signed in"
    );
    assert!(
        !db.revoke_account_device(
            &tenant,
            account,
            &second.device_key_id,
            "account-actor:me",
            None
        )
        .await
        .unwrap()
        .removed,
        "revoking twice finds nothing the second time"
    );
    assert!(
        !db.rename_account_device(&tenant, account, &second.device_key_id, Some("again"))
            .await
            .unwrap(),
        "a revoked device cannot be renamed"
    );
    task.abort();
    other_task.abort();
}

/// A signed-in laptop mints a code, a headless server redeems it and joins
/// the same account, and only that server can renew its session by key.
#[tokio::test]
async fn a_code_minted_on_a_laptop_links_a_server_that_alone_renews_by_key() {
    use trace_commons_server::db::{
        DeviceSignInOutcome, MintAccountDeviceCodeOutcome, NewAccountDeviceCode,
    };
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let (login, task) = stub_near_ai_login("device-code").await;
    let template = signup_template(SIGNUP_TEMPLATE);
    let laptop = fresh_keypair();
    let first = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&laptop),
            native_session(&session_hash("code-laptop")),
            &identity(),
            Some(&template),
        )
        .await
        .expect("the laptop provisions");
    let (tenant, account) = (first.tenant_id.clone(), first.account_id);
    let mint = |code_hash: String, nonce: String, minter: String, cap: i64| {
        let tenant = tenant.clone();
        let db = &db;
        async move {
            db.mint_account_device_code(NewAccountDeviceCode {
                tenant_id: &tenant,
                account_id: account,
                minter_device_key_id: &minter,
                mint_nonce_hash: &nonce,
                code_hash: &code_hash,
                label: Some("zaki-server"),
                expires_at: Utc::now() + Duration::minutes(10),
                outstanding_cap: cap,
            })
            .await
            .unwrap()
        }
    };

    // Only an active device of the account can mint.
    let stranger = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
        &device_bytes(&fresh_keypair()),
    );
    assert_eq!(
        mint(hash("c0"), hash("n0"), stranger, 3).await,
        MintAccountDeviceCodeOutcome::MinterNotADevice
    );
    let code_hash = hash("code-1");
    let MintAccountDeviceCodeOutcome::Minted { code_id } = mint(
        code_hash.clone(),
        hash("n1"),
        first.device_key_id.clone(),
        3,
    )
    .await
    else {
        panic!("the laptop mints");
    };
    assert_eq!(
        mint(hash("code-2"), hash("n1"), first.device_key_id.clone(), 3).await,
        MintAccountDeviceCodeOutcome::NonceReused,
        "a mint proof's nonce is accepted once"
    );
    assert_eq!(
        mint(hash("code-3"), hash("n3"), first.device_key_id.clone(), 1).await,
        MintAccountDeviceCodeOutcome::TooManyOutstanding
    );
    let status = db
        .account_device_code_status(&tenant, account, code_id)
        .await
        .unwrap()
        .unwrap();
    assert!(!status.expired);
    assert_eq!(status.redeemed_device_key_id, None);

    // A wrong code finds nothing; the right one links the server.
    let server = fresh_keypair();
    let server_session = session_hash("code-server");
    assert_eq!(
        db.redeem_account_device_code(
            &tenant,
            &hash("not-a-code"),
            &device_bytes(&server),
            native_session(&server_session),
            Some(&template),
        )
        .await
        .unwrap(),
        DeviceSignInOutcome::NotFound
    );
    let DeviceSignInOutcome::SignedIn(linked) = db
        .redeem_account_device_code(
            &tenant,
            &code_hash,
            &device_bytes(&server),
            native_session(&server_session),
            Some(&template),
        )
        .await
        .unwrap()
    else {
        panic!("the server redeems");
    };
    assert_eq!(linked.account_id, account);
    assert_eq!(linked.anchor_hash, first.anchor_hash);
    assert!(
        db.validate_session(&tenant, &server_session)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        signup_grants(&admin, &tenant, &linked.device_key_id).await,
        vec![template_grant_row(SIGNUP_TEMPLATE)]
    );
    let devices = db
        .list_account_devices(&tenant, account, Some(&server_session))
        .await
        .unwrap();
    assert_eq!(devices.len(), 2);
    assert_eq!(devices[1].device_key_id, linked.device_key_id);
    assert_eq!(devices[1].label.as_deref(), Some("zaki-server"));
    assert!(devices[1].this_device);
    assert_eq!(
        db.account_device_code_status(&tenant, account, code_id)
            .await
            .unwrap()
            .unwrap()
            .redeemed_device_key_id,
        Some(linked.device_key_id.clone())
    );
    assert_eq!(
        db.redeem_account_device_code(
            &tenant,
            &code_hash,
            &device_bytes(&fresh_keypair()),
            native_session(&session_hash("code-again")),
            Some(&template),
        )
        .await
        .unwrap(),
        DeviceSignInOutcome::NotFound,
        "a code is spent by its first redeem"
    );

    // The server joined by code, so it cannot mint one in turn.
    assert_eq!(
        mint(
            hash("code-relay"),
            hash("n-relay"),
            linked.device_key_id.clone(),
            3
        )
        .await,
        MintAccountDeviceCodeOutcome::MinterNotADevice,
        "a code-joined device never mints"
    );

    // Unauthenticated paths never create a tenant: a redeem or renewal naming
    // an unknown tenant finds nothing and writes no tenant row.
    let unknown_tenant = format!("nearai-unknown-{}", uuid::Uuid::new_v4().simple());
    assert_eq!(
        db.redeem_account_device_code(
            &unknown_tenant,
            &code_hash,
            &device_bytes(&fresh_keypair()),
            native_session(&session_hash("code-unknown-tenant")),
            Some(&template),
        )
        .await
        .unwrap(),
        DeviceSignInOutcome::NotFound
    );
    assert_eq!(
        db.renew_device_session(
            &unknown_tenant,
            &device_bytes(&server),
            &hash("r-unknown"),
            native_session(&session_hash("renew-unknown-tenant")),
            Duration::zero(),
        )
        .await
        .unwrap(),
        DeviceSignInOutcome::NotFound
    );
    let tenant_rows: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_tenants WHERE tenant_id = $1",
            &[&unknown_tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(tenant_rows, 0, "no tenant row for an unknown tenant");

    // Renewal: the laptop joined by NEAR AI, not by code, and is refused.
    let renew = |key: [u8; 32], nonce: String, interval: Duration| {
        let tenant = tenant.clone();
        let db = &db;
        async move {
            let token = session_hash("renewed");
            let outcome = db
                .renew_device_session(&tenant, &key, &nonce, native_session(&token), interval)
                .await
                .unwrap();
            (outcome, token)
        }
    };
    assert_eq!(
        renew(device_bytes(&laptop), hash("r0"), Duration::zero())
            .await
            .0,
        DeviceSignInOutcome::NotFound
    );
    let (outcome, token) = renew(device_bytes(&server), hash("r1"), Duration::zero()).await;
    assert!(matches!(outcome, DeviceSignInOutcome::SignedIn(ref p) if p.account_id == account));
    assert!(
        db.validate_session(&tenant, &token)
            .await
            .unwrap()
            .is_some()
    );
    assert_eq!(
        renew(device_bytes(&server), hash("r1"), Duration::zero())
            .await
            .0,
        DeviceSignInOutcome::NonceReused
    );
    assert_eq!(
        renew(device_bytes(&server), hash("r2"), Duration::minutes(10))
            .await
            .0,
        DeviceSignInOutcome::TooSoon
    );

    // Revoking the server ends its renewal; revoking the laptop voids the
    // codes it minted.
    assert!(
        db.revoke_account_device(
            &tenant,
            account,
            &linked.device_key_id,
            "account-actor:me",
            None
        )
        .await
        .unwrap()
        .removed
    );
    assert_eq!(
        renew(device_bytes(&server), hash("r3"), Duration::zero())
            .await
            .0,
        DeviceSignInOutcome::NotFound
    );
    let late_hash = hash("code-late");
    assert!(matches!(
        mint(
            late_hash.clone(),
            hash("n4"),
            first.device_key_id.clone(),
            3
        )
        .await,
        MintAccountDeviceCodeOutcome::Minted { .. }
    ));
    assert!(
        db.revoke_account_device(
            &tenant,
            account,
            &first.device_key_id,
            "account-actor:me",
            None
        )
        .await
        .unwrap()
        .removed
    );
    assert_eq!(
        db.redeem_account_device_code(
            &tenant,
            &late_hash,
            &device_bytes(&fresh_keypair()),
            native_session(&session_hash("code-late")),
            Some(&template),
        )
        .await
        .unwrap(),
        DeviceSignInOutcome::NotFound
    );
    task.abort();
}

/// A revoked device's key is refused by name when it signs in again, a new
/// key on the same machine joins as a new device, and nothing the revoke
/// could not see keeps the old key signed in: an unkeyed native session
/// from before V139 ends with it, the caller's own and browser sessions do
/// not, and an expired grant is left as it was.
#[tokio::test]
async fn a_revoked_device_is_refused_by_name_and_signs_in_again_with_a_new_key() {
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let (login, task) = stub_near_ai_login("revoked-again").await;
    let template = signup_template(SIGNUP_TEMPLATE);

    let laptop = fresh_keypair();
    let laptop_session = session_hash("revoked-again-laptop");
    let first = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&laptop),
            native_session(&laptop_session),
            &identity(),
            Some(&template),
        )
        .await
        .expect("the laptop provisions");
    let lost = fresh_keypair();
    let lost_session = session_hash("revoked-again-lost");
    let second = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&lost),
            native_session(&lost_session),
            &identity(),
            Some(&template),
        )
        .await
        .expect("the second device provisions");
    let (tenant, account) = (first.tenant_id.clone(), first.account_id);

    // Sessions that name no device key: a native one from before V139 (the
    // lost device's, as far as anyone can tell), the caller's own native one,
    // and a browser one.
    let unkeyed = |kind: &'static str, label: &'static str| {
        let admin = &admin;
        let tenant = tenant.clone();
        async move {
            let token = session_hash(label);
            admin
                .execute(
                    "INSERT INTO trace_sessions(tenant_id,session_id,account_id,token_hash,client_kind,expires_at)
                     VALUES($1,$2,$3,$4,$5,now() + interval '1 hour')",
                    &[&tenant, &uuid::Uuid::new_v4(), &account, &token, &kind],
                )
                .await
                .unwrap();
            token
        }
    };
    let legacy_session = unkeyed("native", "revoked-again-legacy").await;
    let caller_session = unkeyed("native", "revoked-again-caller").await;
    let browser_session = unkeyed("web", "revoked-again-browser").await;
    // A native session that names a credential (only a passkey sign-in, before
    // V144, minted one) whose credential row is gone, so V144's join missed it.
    // Nothing tells it apart from the device's own, so it ends with the device.
    let passkey_native_session = session_hash("revoked-again-passkey-native");
    admin
        .execute(
            "INSERT INTO trace_sessions(tenant_id,session_id,account_id,token_hash,client_kind,auth_credential_id,expires_at)
             VALUES($1,$2,$3,$4,'native','passkey-credential-1',now() + interval '1 hour')",
            &[&tenant, &uuid::Uuid::new_v4(), &account, &passkey_native_session],
        )
        .await
        .unwrap();
    // A second grant on the device, already expired: not the revoke's to
    // rewrite or count.
    let principal: String = admin
        .query_one(
            "SELECT principal_ref FROM trace_near_provisioned_devices WHERE device_key_id = $1",
            &[&second.device_key_id],
        )
        .await
        .unwrap()
        .get(0);
    let expired_grant = uuid::Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_tenant_access_grants(tenant_id,grant_id,principal_ref,role,status,issued_at,updated_at)
             VALUES($1,$2,$3,'contributor','expired',now() - interval '2 days',now() - interval '1 day')",
            &[&tenant, &expired_grant, &principal],
        )
        .await
        .unwrap();

    let result = db
        .revoke_account_device(
            &tenant,
            account,
            &second.device_key_id,
            "account-actor:me",
            Some(&caller_session),
        )
        .await
        .unwrap();
    assert_eq!(
        result,
        trace_commons_server::db::RevokeAccountDeviceResult {
            removed: true,
            sessions_revoked: 1,
            grants_revoked: 1,
            unattributed_sessions_revoked: 2,
        },
        "the unkeyed native sessions end with the device; the caller's own and \
         the browser session do not, and the expired grant is not counted"
    );
    for (token, live, what) in [
        (&legacy_session, false, "the pre-V139 native session ends"),
        (&caller_session, true, "the caller's own session is spared"),
        (
            &browser_session,
            true,
            "a browser session is not a device session",
        ),
        (
            &passkey_native_session,
            false,
            "a native session left over from a passkey sign-in ends too",
        ),
    ] {
        assert_eq!(
            db.validate_session(&tenant, token).await.unwrap().is_some(),
            live,
            "{what}"
        );
    }
    let (status, revoked_at_set, updated_recently): (String, bool, bool) = {
        let row = admin
            .query_one(
                "SELECT status, revoked_at IS NOT NULL, updated_at > now() - interval '1 hour'
                   FROM trace_tenant_access_grants WHERE grant_id = $1",
                &[&expired_grant],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1), row.get(2))
    };
    assert_eq!(
        (status.as_str(), revoked_at_set, updated_recently),
        ("expired", false, false),
        "an expired grant is left as it was"
    );
    let revoked_grant_updated: bool = admin
        .query_one(
            "SELECT bool_and(updated_at > now() - interval '1 hour')
               FROM trace_tenant_access_grants
              WHERE principal_ref = $1 AND status = 'revoked'",
            &[&principal],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        revoked_grant_updated,
        "the revoked grant's updated_at moves"
    );

    assert!(
        db.validate_session(&tenant, &laptop_session)
            .await
            .unwrap()
            .is_some(),
        "the other device stays signed in"
    );

    // Signing in again with the revoked key is refused by name and writes
    // nothing.
    let retry_session = session_hash("revoked-again-retry");
    let error = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&lost),
            native_session(&retry_session),
            &identity(),
            Some(&template),
        )
        .await
        .expect_err("a revoked key never signs in again");
    assert!(
        trace_commons_server::account_onboarding::is_device_key_revoked(&error),
        "refused by name, got {error}"
    );
    let retry_sessions: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_sessions WHERE token_hash = $1",
            &[&retry_session],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(retry_sessions, 0);

    // A new key on the same machine joins the account as a new device.
    let rekeyed = fresh_keypair();
    let rekeyed_session = session_hash("revoked-again-rekeyed");
    let third = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&rekeyed),
            native_session(&rekeyed_session),
            &identity(),
            Some(&template),
        )
        .await
        .expect("a new key signs in");
    assert_eq!(
        (third.tenant_id.as_str(), third.account_id),
        (tenant.as_str(), account)
    );
    let ids: Vec<String> = db
        .list_account_devices(&tenant, account, Some(&rekeyed_session))
        .await
        .unwrap()
        .into_iter()
        .filter(|d| d.this_device)
        .map(|d| d.device_key_id)
        .collect();
    assert_eq!(ids, vec![third.device_key_id.clone()]);
    task.abort();
}

/// A revoke that is committing while a renewal by key, or a redeem of a code
/// the revoked laptop minted, reads the key holds that read back until it
/// commits, and the read then sees the key revoked: no keyed session on a
/// revoked key, and no device joined under a revoked laptop.
#[tokio::test]
async fn a_revoke_in_flight_holds_back_a_renewal_and_a_redeem_until_it_commits() {
    use trace_commons_server::db::{
        DeviceSignInOutcome, MintAccountDeviceCodeOutcome, NewAccountDeviceCode,
    };
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let (login, task) = stub_near_ai_login("device-code-race").await;
    let template = signup_template(SIGNUP_TEMPLATE);
    let laptop = fresh_keypair();
    let first = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&laptop),
            native_session(&session_hash("race-laptop")),
            &identity(),
            Some(&template),
        )
        .await
        .expect("the laptop provisions");
    let (tenant, account) = (first.tenant_id.clone(), first.account_id);
    let salt = uuid::Uuid::new_v4().simple().to_string();
    let mint = |label: &str| {
        let code_hash = hash(&format!("race-code-{label}-{salt}"));
        let nonce = hash(&format!("race-nonce-{label}-{salt}"));
        let tenant = tenant.clone();
        let minter = first.device_key_id.clone();
        let db = &db;
        async move {
            let outcome = db
                .mint_account_device_code(NewAccountDeviceCode {
                    tenant_id: &tenant,
                    account_id: account,
                    minter_device_key_id: &minter,
                    mint_nonce_hash: &nonce,
                    code_hash: &code_hash,
                    label: None,
                    expires_at: Utc::now() + Duration::minutes(10),
                    outstanding_cap: 5,
                })
                .await
                .unwrap();
            assert!(matches!(
                outcome,
                MintAccountDeviceCodeOutcome::Minted { .. }
            ));
            code_hash
        }
    };
    let server_code = mint("server").await;
    let late_code = mint("late").await;
    let server = fresh_keypair();
    let DeviceSignInOutcome::SignedIn(linked) = db
        .redeem_account_device_code(
            &tenant,
            &server_code,
            &device_bytes(&server),
            native_session(&session_hash("race-server")),
            Some(&template),
        )
        .await
        .unwrap()
    else {
        panic!("the server redeems");
    };

    // The blocker plays the revoke: it updates the key row and holds the
    // lock until it commits.
    let blocker_backend = PgBackend::new(&config(url.clone())).await.unwrap();
    let mut blocker = blocker_backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let held = std::time::Duration::from_millis(400);

    // Renewal by key while the server's revoke is in flight.
    let tx = blocker.transaction().await.unwrap();
    tx.execute(
        "UPDATE device_keys SET revoked_at = now() WHERE device_key_id = $1",
        &[&linked.device_key_id],
    )
    .await
    .unwrap();
    let renewed_token = session_hash("race-renewed");
    let server_key = device_bytes(&server);
    let renew_nonce = hash(&format!("race-renew-{salt}"));
    let renewal = db.renew_device_session(
        &tenant,
        &server_key,
        &renew_nonce,
        native_session(&renewed_token),
        Duration::zero(),
    );
    tokio::pin!(renewal);
    assert!(
        tokio::time::timeout(held, &mut renewal).await.is_err(),
        "the renewal waits on the revoke's row lock"
    );
    tx.commit().await.unwrap();
    assert_eq!(renewal.await.unwrap(), DeviceSignInOutcome::NotFound);
    let renewed: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_sessions WHERE token_hash = $1",
            &[&renewed_token],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(renewed, 0, "no keyed session on the revoked key");

    // A redeem of the laptop's code while the laptop's revoke is in flight.
    let tx = blocker.transaction().await.unwrap();
    tx.execute(
        "UPDATE device_keys SET revoked_at = now() WHERE device_key_id = $1",
        &[&first.device_key_id],
    )
    .await
    .unwrap();
    let joiner = fresh_keypair();
    let joiner_key = device_bytes(&joiner);
    let joiner_session = session_hash("race-joiner");
    let redeem = db.redeem_account_device_code(
        &tenant,
        &late_code,
        &joiner_key,
        native_session(&joiner_session),
        Some(&template),
    );
    tokio::pin!(redeem);
    assert!(
        tokio::time::timeout(held, &mut redeem).await.is_err(),
        "the redeem waits on the minter's row lock"
    );
    tx.commit().await.unwrap();
    assert_eq!(redeem.await.unwrap(), DeviceSignInOutcome::NotFound);
    let joined: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_account_device_codes
              WHERE code_hash = $1 AND redeemed_at IS NOT NULL",
            &[&late_code],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(joined, 0, "no device joined under the revoked laptop");
    task.abort();
}

/// Inserts a submission by `principal` with `status`, received `received_at`.
async fn seed_submission(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    principal: &str,
    status: &str,
    received_at: chrono::DateTime<Utc>,
) -> uuid::Uuid {
    let submission = uuid::Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,
                schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,
                redaction_pipeline_version,redaction_hash,received_at)
             VALUES($1,$2,$3,$4,'v1','v1','test',$5,'low','test',$6,$7)",
            &[
                &tenant,
                &submission,
                &uuid::Uuid::new_v4(),
                &principal,
                &status,
                &"a".repeat(64),
                &received_at,
            ],
        )
        .await
        .unwrap();
    submission
}

/// Each device in the list carries how many of its traces still stand in
/// the commons, and when it last sent one, so the app's map can show the
/// account's other computers contributing. A device's traces are the
/// submissions made under its own principal or an upload principal bound to
/// it (V135); "still stands" is received, accepted or quarantined, and not
/// withdrawn, revoked or purged -- the server side of History's accepted,
/// submitted and quarantined rows.
#[tokio::test]
async fn each_listed_device_says_what_it_has_contributed() {
    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let (login, task) = stub_near_ai_login("devices-contributed").await;
    let template = signup_template(SIGNUP_TEMPLATE);
    let tag = format!("{:?}", std::time::SystemTime::now());
    let laptop_token = session_hash(&format!("contrib-laptop-{tag}"));
    let laptop = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&fresh_keypair()),
            native_session(&laptop_token),
            &identity(),
            Some(&template),
        )
        .await
        .expect("the laptop provisions");
    let server_token = session_hash(&format!("contrib-server-{tag}"));
    let server = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&fresh_keypair()),
            native_session(&server_token),
            &identity(),
            Some(&template),
        )
        .await
        .expect("the server provisions");
    let (tenant, account) = (laptop.tenant_id.clone(), laptop.account_id);
    let principal_of = |device: String| {
        let admin = &admin;
        async move {
            admin
                .query_one(
                    "SELECT principal_ref FROM trace_near_provisioned_devices WHERE device_key_id = $1",
                    &[&device],
                )
                .await
                .unwrap()
                .get::<_, String>(0)
        }
    };
    let laptop_principal = principal_of(laptop.device_key_id.clone()).await;
    let server_principal = principal_of(server.device_key_id.clone()).await;

    // Nothing sent yet: zero, said as zero, and no last time.
    let fresh = db
        .list_account_devices(&tenant, account, None)
        .await
        .unwrap();
    assert!(fresh.iter().all(|d| d.contributed_traces == 0));
    assert!(fresh.iter().all(|d| d.last_contributed_at.is_none()));

    let day = |n: i64| Utc::now() - Duration::days(n);
    // The laptop, under its own principal: two that stand, the newer one
    // quarantined; a rejected one, a withdrawn one, a revoked one and a
    // purged one that do not.
    seed_submission(&admin, &tenant, &laptop_principal, "accepted", day(5)).await;
    seed_submission(&admin, &tenant, &laptop_principal, "quarantined", day(2)).await;
    seed_submission(&admin, &tenant, &laptop_principal, "rejected", day(1)).await;
    let withdrawn = seed_submission(&admin, &tenant, &laptop_principal, "accepted", day(1)).await;
    admin
        .execute(
            "UPDATE trace_submissions SET withdrawn_at = now() WHERE submission_id = $1",
            &[&withdrawn],
        )
        .await
        .unwrap();
    seed_submission(&admin, &tenant, &laptop_principal, "revoked", day(1)).await;
    let purged = seed_submission(&admin, &tenant, &laptop_principal, "accepted", day(1)).await;
    admin
        .execute(
            "UPDATE trace_submissions SET purged_at = now() WHERE submission_id = $1",
            &[&purged],
        )
        .await
        .unwrap();
    // The server, through an upload principal bound to it (V135), received
    // and not yet given a verdict.
    let upload = format!(
        "principal_sha256:{}",
        hash(&format!("upload-{tag}")).trim_start_matches("sha256:")
    );
    admin
        .execute(
            "INSERT INTO trace_device_upload_principals(tenant_id,upload_principal_ref,device_principal_ref,device_key_id)
             VALUES($1,$2,'principal_sha256:' || encode(sha256(convert_to('device:' || $1 || ':' || $3, 'UTF8')), 'hex'),$3)",
            &[&tenant, &upload, &server.device_key_id],
        )
        .await
        .unwrap();
    let bound: String = admin
        .query_one(
            "SELECT device_principal_ref FROM trace_device_upload_principals WHERE upload_principal_ref = $1",
            &[&upload],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        bound, server_principal,
        "an upload principal names the device's own principal"
    );
    seed_submission(&admin, &tenant, &upload, "received", day(3)).await;
    // Under the account but under no device (another principal of it):
    // counted for no device.
    let unattached = format!(
        "principal_sha256:{}",
        hash(&format!("unattached-{tag}")).trim_start_matches("sha256:")
    );
    seed_submission(&admin, &tenant, &unattached, "accepted", day(1)).await;

    let devices = db
        .list_account_devices(&tenant, account, None)
        .await
        .unwrap();
    let by_id = |id: &str| {
        devices
            .iter()
            .find(|d| d.device_key_id == id)
            .unwrap()
            .clone()
    };
    let laptop_row = by_id(&laptop.device_key_id);
    let server_row = by_id(&server.device_key_id);
    assert_eq!(laptop_row.contributed_traces, 2);
    assert_eq!(server_row.contributed_traces, 1);
    let close = |a: Option<chrono::DateTime<Utc>>, b: chrono::DateTime<Utc>| {
        (a.expect("a last contribution") - b).num_seconds().abs() < 5
    };
    assert!(
        close(laptop_row.last_contributed_at, day(2)),
        "the newest that stands, not the rejected one"
    );
    assert!(close(server_row.last_contributed_at, day(3)));

    // Another account's devices count none of these.
    let (other_login, other_task) = stub_near_ai_login("devices-contributed-other").await;
    let other = db
        .provision_near_ai_login(
            &other_login,
            &device_bytes(&fresh_keypair()),
            native_session(&session_hash(&format!("contrib-other-{tag}"))),
            &identity(),
            Some(&template),
        )
        .await
        .expect("another account provisions");
    let theirs = db
        .list_account_devices(&other.tenant_id, other.account_id, None)
        .await
        .unwrap();
    assert!(theirs.iter().all(|d| d.contributed_traces == 0));
    task.abort();
    other_task.abort();
}

/// A NEAR AI device provisioned before sign-up wrote grants gets its grant
/// backfilled by a judgement claim too, as it would by an upload claim, so
/// the two routes admit the same devices.
#[tokio::test]
async fn a_judgement_claim_backfills_the_near_ai_signup_grant_like_an_upload_claim() {
    use axum::body::{Body, to_bytes};
    use std::sync::Arc;
    use tower::ServiceExt as _;
    use trace_commons_protocol::insights_judgement::{
        JUDGEMENT_CLAIM_PATH, JUDGEMENT_CLAIM_REQUEST_SCHEMA_VERSION,
    };
    use trace_commons_server::trace_upload_claim_issuer::{
        TraceUploadClaimIssuerConfig, generate_upload_claim_keypair,
        trace_upload_claim_issuer_router,
    };

    let _serial = serial().lock().await;
    let Ok(url) = std::env::var("TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let (db, admin) = provisioning_fixture(&url).await;
    let db = Arc::new(db);
    let issuer_keys = generate_upload_claim_keypair().unwrap();
    let workload_keys = generate_upload_claim_keypair().unwrap();
    let config = |template: Option<&str>| TraceUploadClaimIssuerConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        signing_private_key_pem: issuer_keys.private_key_pem.clone(),
        signing_public_key_pem: issuer_keys.public_key_pem.clone(),
        signing_kid: "issuer-test".into(),
        issuer: "trace-commons-upload-issuer".into(),
        audience: "trace-commons-upload".into(),
        max_ttl_seconds: 300,
        workload_public_key_pem: workload_keys.public_key_pem.clone(),
        workload_issuer: None,
        workload_audience: None,
        tenant_access_grant_db: Some(db.clone() as Arc<dyn Database>),
        require_tenant_access_grants: true,
        shutdown_grace_seconds: 30,
        request_timeout_seconds: 10,
        max_request_bytes: 64 * 1024,
        allowlist_source: None,
        allowlist_refresh_interval_seconds: 60,
        allowlist_max_stale_seconds: 3600,
        onboarding_device_key_db: Some(db.clone() as Arc<dyn Database>),
        onboarding_ingest_url: Some("https://ingest.example".into()),
        onboarding_community_url: None,
        onboarding_profile_url: None,
        onboarding_leaderboard_url: None,
        admin_bind: None,
        invite_admin_backend: None,
        invite_admin_registry: None,
        invite_registry_authoritative: false,
        near_ai_signup_policy_template: template.map(signup_template),
    };
    let judgement_claim = |config: TraceUploadClaimIssuerConfig,
                           tenant: String,
                           device_key_id: String,
                           device: &Ed25519KeyPair| {
        let body = serde_json::json!({
            "schema_version": JUDGEMENT_CLAIM_REQUEST_SCHEMA_VERSION,
            "tenant_id": tenant,
            "device_key_id": device_key_id,
            "issued_at": Utc::now().timestamp(),
        })
        .to_string();
        let signature =
            base64::engine::general_purpose::STANDARD.encode(device.sign(body.as_bytes()).as_ref());
        let router = trace_upload_claim_issuer_router(config).expect("router");
        async move {
            let response = router
                .oneshot(
                    axum::http::Request::builder()
                        .method(axum::http::Method::POST)
                        .uri(JUDGEMENT_CLAIM_PATH)
                        .header("content-type", "application/json")
                        .header("x-trace-device-key-id", device_key_id)
                        .header("x-trace-device-signature", signature)
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            (status, body)
        }
    };

    // Provisioned the way every NEAR AI account was before sign-up wrote
    // grants: no grant row.
    let (login, task) = stub_near_ai_login("judgement-claim-backfill").await;
    let device = fresh_keypair();
    let t = session_hash("judgement-claim-backfill");
    let account = db
        .provision_near_ai_login(
            &login,
            &device_bytes(&device),
            native_session(&t),
            &identity(),
            None,
        )
        .await
        .unwrap();
    assert!(
        signup_grants(&admin, &account.tenant_id, &account.device_key_id)
            .await
            .is_empty()
    );

    // Without the sign-up template there is nothing to backfill, so the
    // grant rule refuses.
    let (status, body) = judgement_claim(
        config(None),
        account.tenant_id.clone(),
        account.device_key_id.clone(),
        &device,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "judgement_claim_grant_required", "{body}");

    // With it, the judgement claim backfills the grant and is issued.
    let (status, body) = judgement_claim(
        config(Some(SIGNUP_TEMPLATE)),
        account.tenant_id.clone(),
        account.device_key_id.clone(),
        &device,
    )
    .await;
    assert_eq!(status, axum::http::StatusCode::OK, "{body}");
    assert!(body["access_token"].is_string(), "{body}");
    assert_eq!(
        signup_grants(&admin, &account.tenant_id, &account.device_key_id).await,
        vec![template_grant_row(SIGNUP_TEMPLATE)]
    );
    task.abort();
}
