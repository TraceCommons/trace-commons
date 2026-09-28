// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `POST /v1/device/invite-subject` against a real registry.
//!
//! A device onboarded through the real `/v1/onboard` path reads back exactly
//! the invite subject hash the issuer stored for it, signed with its own key;
//! and the named refusals hold against real rows: another device's key, a
//! revoked device, and a device with no invite behind it.
//!
//! Skipped without `TRACE_COMMONS_DEVICE_INVITE_SUBJECT_PG_TEST_DATABASE_URL`,
//! which must name a database beginning `admission_test_device_invite_subject`
//! on the literal host 127.0.0.1.

use axum::body::{Body, to_bytes};
use axum::http::{Method, Request, StatusCode};
use base64::Engine as _;
use ring::signature::{Ed25519KeyPair, KeyPair};
use secrecy::SecretString;
use std::sync::Arc;
use tower::ServiceExt;
use trace_commons_protocol::device_invite_subject::{
    DEVICE_INVITE_SUBJECT_PATH, DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION,
};
use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::trace_upload_claim_allowlist::{AllowlistSourceSpec, hash_invite_code};
use trace_commons_server::trace_upload_claim_issuer::{
    TraceUploadClaimIssuerConfig, generate_upload_claim_keypair, trace_upload_claim_issuer_router,
};
use uuid::Uuid;

const ENV: &str = "TRACE_COMMONS_DEVICE_INVITE_SUBJECT_PG_TEST_DATABASE_URL";
const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

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

struct Device {
    key: Ed25519KeyPair,
    id: String,
}

fn new_device() -> Device {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let key = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let id = device_key_id_from_public_key_bytes(key.public_key().as_ref());
    Device { key, id }
}

async fn call(
    router: axum::Router,
    uri: &str,
    headers: &[(&str, String)],
    body: String,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = router
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// Ask for `device`'s invite subject, signed by `signer`.
async fn read_subject(
    router: axum::Router,
    tenant: &str,
    device: &Device,
    signer: &Device,
) -> (StatusCode, serde_json::Value) {
    let body = serde_json::json!({
        "schema_version": DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION,
        "tenant_id": tenant,
        "device_key_id": device.id,
        "issued_at": chrono::Utc::now().timestamp(),
    })
    .to_string();
    let signature = B64.encode(signer.key.sign(body.as_bytes()).as_ref());
    call(
        router,
        DEVICE_INVITE_SUBJECT_PATH,
        &[
            ("x-trace-device-key-id", device.id.clone()),
            ("x-trace-device-signature", signature),
        ],
        body,
    )
    .await
}

#[tokio::test]
async fn a_device_reads_back_the_invite_it_was_onboarded_with() {
    let Ok(url) = std::env::var(ENV) else {
        eprintln!("SKIPPED: {ENV} not set");
        return;
    };
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(
        parsed
            .path()
            .starts_with("/admission_test_device_invite_subject")
    );
    let backend = Arc::new(PgBackend::new(&config(url)).await.unwrap());
    backend.run_migrations().await.unwrap();
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();

    let suffix: String = Uuid::new_v4()
        .simple()
        .to_string()
        .to_uppercase()
        .chars()
        .take(8)
        .collect();
    let code = format!("INVITEEE{suffix}");
    let tenant = format!("tenant-invitee-{}", Uuid::new_v4().simple());
    let dir = tempfile::tempdir().unwrap();
    let allowlist = dir.path().join("allowlist.json");
    std::fs::write(
        &allowlist,
        serde_json::json!({
            "version": 1,
            "generated_at": "2026-09-27T00:00:00Z",
            "policy_label": "pilot",
            "entries": [{
                "kind": "invite",
                "subject_hash": hash_invite_code(&code),
                "tenant_id": tenant,
                "max_uses": 3,
                "note_label": "Invitee",
            }],
        })
        .to_string(),
    )
    .unwrap();
    let issuer_keys = generate_upload_claim_keypair().unwrap();
    let workload_keys = generate_upload_claim_keypair().unwrap();
    let db: Arc<dyn Database> = backend.clone();
    let router = || {
        trace_upload_claim_issuer_router(TraceUploadClaimIssuerConfig {
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
            tenant_access_grant_db: None,
            require_tenant_access_grants: false,
            shutdown_grace_seconds: 30,
            request_timeout_seconds: 10,
            max_request_bytes: 64 * 1024,
            allowlist_source: Some(AllowlistSourceSpec::File(allowlist.clone())),
            allowlist_refresh_interval_seconds: 60,
            allowlist_max_stale_seconds: 3600,
            onboarding_device_key_db: Some(db.clone()),
            onboarding_ingest_url: Some("https://ingest.example".into()),
            onboarding_community_url: None,
            onboarding_profile_url: None,
            onboarding_leaderboard_url: None,
            admin_bind: None,
            invite_admin_backend: None,
            invite_admin_registry: None,
            invite_registry_authoritative: false,
        })
        .expect("router")
    };

    // Onboard two devices with the invite, through the real route.
    let first = new_device();
    let second = new_device();
    for device in [&first, &second] {
        let body = serde_json::json!({
            "schema_version": trace_commons_protocol::onboarding::TRACE_ONBOARD_REQUEST_SCHEMA_VERSION,
            "invite_code": code,
            "device_public_key": B64.encode(device.key.public_key().as_ref()),
            "client_info": {"agent": "trace-commons-contributor", "version": "0.x.y"},
        });
        let (status, onboarded) = call(router(), "/v1/onboard", &[], body.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{onboarded}");
        assert_eq!(onboarded["tenant_id"], tenant);
    }

    // The device's own key reads back exactly the stored invite hash.
    let (status, answer) = read_subject(router(), &tenant, &first, &first).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(
        answer,
        serde_json::json!({ "invite_subject_hash": hash_invite_code(&code) })
    );

    // A sibling device's key cannot read it for this device.
    let (status, answer) = read_subject(router(), &tenant, &first, &second).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    assert_eq!(answer["error"], "invalid device key signature");

    // Named under another tenant, the device is not registered there.
    let (status, answer) = read_subject(router(), "tenant-someone-else", &first, &first).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    assert_eq!(answer["error"], "device_key_not_registered");

    // A revoked device is refused by name.
    admin
        .execute(
            "UPDATE device_keys SET revoked_at = now() WHERE device_key_id = $1",
            &[&second.id],
        )
        .await
        .unwrap();
    let (status, answer) = read_subject(router(), &tenant, &second, &second).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    assert_eq!(answer["error"], "device_key_revoked");

    // A device with no invite behind it (a NEAR-onboarded one) is refused.
    let near = new_device();
    let near_tenant = format!("near-{}", "ab".repeat(32));
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&near_tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO device_keys
                (device_key_id, tenant_id, public_key, invite_subject_hash, onboarding_origin)
             VALUES ($1, $2, $3, NULL, 'near')",
            &[
                &near.id,
                &near_tenant,
                &B64.encode(near.key.public_key().as_ref()),
            ],
        )
        .await
        .unwrap();
    let (status, answer) = read_subject(router(), &near_tenant, &near, &near).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    assert_eq!(answer["error"], "device_not_invite_onboarded");
}
