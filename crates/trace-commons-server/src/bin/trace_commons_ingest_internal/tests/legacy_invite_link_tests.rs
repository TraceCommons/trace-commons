// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! HTTP layer of the legacy invite link routes (V81). The database rules
//! themselves are covered by `tests/legacy_invite_link_pg.rs`.

use super::*;
use crate::legacy_invite_link_routes::{NOT_ENABLED, challenge_handler, link_handler};
use axum::body::Body;
use ring::signature::KeyPair as _;
use tower::ServiceExt;
use trace_commons_protocol::legacy_invite_link::LegacyInviteLinkRequest;
use trace_commons_protocol::legacy_invite_link::{
    LegacyInviteLinkStatement, legacy_invite_link_statement_bytes,
};
use trace_commons_server::legacy_invite_link::LegacyInviteLinkSigner;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn session_ctx(tenant: &str, account: Uuid, method: AccountAuthMethod) -> AccountCtx {
    AccountCtx {
        account_id: trace_commons_server::account_session::AccountId::from_uuid(account),
        principal_set:
            trace_commons_server::account_session::AccountPrincipalSet::from_iter_for_test_only(
                Vec::<String>::new(),
            ),
        auth_method: method,
        tenant_id: tenant.to_string(),
        actor_ref: format!("account-actor:{account}"),
        auth_credential_id: None,
        client_kind: "near".into(),
    }
}

fn signer() -> Arc<LegacyInviteLinkSigner> {
    let keys =
        trace_commons_server::trace_upload_claim_issuer::generate_upload_claim_keypair().unwrap();
    Arc::new(
        LegacyInviteLinkSigner::from_pem(&keys.private_key_pem, &keys.public_key_pem, "k1")
            .unwrap(),
    )
}

#[tokio::test]
async fn legacy_link_routes_refuse_device_bearers_and_stay_closed_unless_enabled() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    for uri in [
        "/v1/account/invites/legacy-link/challenge",
        "/v1/account/invites/legacy-link",
    ] {
        let response = app(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(AUTHORIZATION, "Bearer token-a")
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{uri}");
    }
    let near = format!("near-{}", "a".repeat(64));
    let ctx = session_ctx(&near, Uuid::new_v4(), AccountAuthMethod::NativeToken);
    let err = challenge_handler(
        State(state.clone()),
        Extension(ctx.clone()),
        HeaderMap::new(),
    )
    .await
    .expect_err("closed unless enabled");
    assert_eq!(err.0, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(err.1.0.error, NOT_ENABLED);
    let mut device = ctx;
    device.auth_method = AccountAuthMethod::DeviceBearer;
    let err = challenge_handler(State(state), Extension(device), HeaderMap::new())
        .await
        .expect_err("device context refused");
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert_eq!(err.1.0.error, "account session required");
}

#[tokio::test]
async fn legacy_link_cookie_sessions_must_be_same_origin() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut state = test_state(temp.path().to_path_buf());
    Arc::make_mut(&mut state).legacy_invite_link = Some(signer());
    let near = format!("near-{}", "a".repeat(64));
    let ctx = session_ctx(&near, Uuid::new_v4(), AccountAuthMethod::SessionCookie);
    let mut headers = HeaderMap::new();
    headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
    let err = challenge_handler(State(state), Extension(ctx), headers)
        .await
        .expect_err("cross-site cookie refused");
    assert_eq!(err.0, StatusCode::FORBIDDEN);
    assert_eq!(err.1.0.error, "cross-origin account mutation");
}

/// The handler pair end to end against PostgreSQL: challenge, sign, link,
/// and a second account refused with the conflict label.
#[tokio::test]
async fn legacy_link_handlers_link_and_refuse_a_second_claim() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let db: Arc<dyn Database> = backend.clone();
    let mut state = test_state_with_options(
        temp.path().to_path_buf(),
        Some(db),
        None,
        false,
        false,
        false,
        false,
    );
    Arc::make_mut(&mut state).legacy_invite_link = Some(signer());

    let legacy = format!("tenant-http-link-{}", Uuid::new_v4().simple());
    let invite = format!("sha256:{}", "c".repeat(64));
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let device = ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let device_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
        device.public_key().as_ref(),
    );
    let mut accounts = Vec::new();
    for prefix in ["near", "nearai"] {
        let tenant = format!("{prefix}-{}", Uuid::new_v4().simple().to_string().repeat(2));
        let account = Uuid::new_v4();
        admin
            .execute(
                "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
                &[&tenant],
            )
            .await
            .unwrap();
        admin
            .execute(
                "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
                &[
                    &tenant as &(dyn tokio_postgres::types::ToSql + Sync),
                    &account,
                ],
            )
            .await
            .unwrap();
        admin
            .execute(
                "INSERT INTO trace_near_account_anchors
                    (tenant_id, account_id, anchor_hash, sealed_account_name,
                     index_pepper_ref, account_name_key_ref)
                 VALUES ($1, $2, $3, $4, 'p', 'k')",
                &[
                    &tenant as &(dyn tokio_postgres::types::ToSql + Sync),
                    &account,
                    &format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2)),
                    &serde_json::json!({"test_fixture": true}),
                ],
            )
            .await
            .unwrap();
        accounts.push((tenant, account));
    }
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&legacy],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO onboarding_invites (tenant_id, invite_subject_hash, max_uses, consumed_uses)
             VALUES ($1, $2, 3, 1)",
            &[&legacy, &invite],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO device_keys (device_key_id, tenant_id, public_key, invite_subject_hash)
             VALUES ($1, $2, $3, $4)",
            &[
                &device_id,
                &legacy,
                &B64.encode(device.public_key().as_ref()),
                &invite,
            ],
        )
        .await
        .unwrap();

    let mut outcomes = Vec::new();
    for (tenant, account) in &accounts {
        let ctx = session_ctx(tenant, *account, AccountAuthMethod::NativeToken);
        let (_, Json(challenge)) = challenge_handler(
            State(state.clone()),
            Extension(ctx.clone()),
            HeaderMap::new(),
        )
        .await
        .expect("challenge");
        let statement = LegacyInviteLinkStatement {
            legacy_tenant_id: legacy.clone(),
            device_key_id: device_id.clone(),
            invite_subject_hash: invite.clone(),
            account_tenant_id: tenant.clone(),
            account_id: *account,
            nonce: challenge.nonce.clone(),
            issued_at: challenge.issued_at,
        };
        let signature = device.sign(&legacy_invite_link_statement_bytes(&statement));
        let request = LegacyInviteLinkRequest {
            legacy_tenant_id: legacy.clone(),
            device_key_id: device_id.clone(),
            device_public_key: B64.encode(device.public_key().as_ref()),
            invite_subject_hash: invite.clone(),
            nonce: challenge.nonce,
            issued_at: challenge.issued_at,
            signature: B64.encode(signature.as_ref()),
        };
        outcomes.push(
            link_handler(
                State(state.clone()),
                Extension(ctx),
                HeaderMap::new(),
                Json(request),
            )
            .await,
        );
    }
    let (headers, Json(linked)) = outcomes.remove(0).expect("first account links");
    assert_eq!(headers.get("cache-control").unwrap(), "no-store");
    assert_eq!(linked.record.statement.account_tenant_id, accounts[0].0);
    assert_eq!(linked.authority, "invited");
    let err = outcomes.remove(0).expect_err("second account refused");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert_eq!(err.1.0.error, "legacy_link_tenant_claimed");
}
