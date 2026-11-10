// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! NEAR AI login provisioning over the real router: the upload claim a
//! provisioned device mints, open sign-up, and the named refusal for a
//! revoked device key.
//!
//! Every test here is PostgreSQL-backed and self-skips without
//! `TRACE_COMMONS_PG_TEST_DATABASE_URL`; the anchor resolves through the
//! login-resolver role, so `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` is
//! needed as well, exactly as for the rest of the account suite.
//!
//! What is real: the provisioning handlers over the real router, the
//! ceremony rows, the device signatures (a real Ed25519 key signing the bytes
//! the server hands back), the shared preimage functions, and the database
//! writes. What is stubbed: NEAR AI's `GET /users/me`, through the
//! `cfg(test)` introspection seam.
//!
//! The readiness check reads process-global environment variables, which
//! [`ProvisioningEnv`] sets and restores; like the rest of the PostgreSQL
//! suite these tests assume `--test-threads=1`.

use trace_commons_ingest::test_support::near_ai_provisioning_fixture::*;
use trace_commons_ingest::test_support::*;
use trace_commons_server::account_session::hash_secret;
use secrecy::SecretString;
use chrono::Utc;
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::config::DatabaseConfig;
use axum::http::header::CONTENT_TYPE;
use axum::http::header::AUTHORIZATION;
use uuid::Uuid;
use axum::http::StatusCode;
use trace_commons_server::db::Database;
use std::time::Duration;
use std::collections::BTreeSet;
use std::collections::BTreeMap;
use std::sync::Arc;
use axum::body::Body;
use tower::ServiceExt;

const PROVISION_START: &str = "/v1/account/near-ai/provision/start/v2";
const PROVISION_FINISH: &str = "/v1/account/near-ai/provision/finish/v2";

// --- Fixtures ----------------------------------------------------------------

/// A started ceremony as the client sees it.
struct Started {
    ceremony_id: String,
    signing_bytes: Vec<u8>,
    verifier: String,
}

async fn start(state: &Arc<AppState>, uri: &str, bearer: Option<&str>, device: &Device) -> Started {
    use base64::Engine as _;
    let (verifier, challenge) = pkce();
    let reply = send(
        state,
        "POST",
        uri,
        Some(&serde_json::json!({
            "device_public_key": device.public_b64(),
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        })),
        bearer,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{uri}: {:?}", reply.json());
    let body = reply.json();
    let decode = |field: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(body[field].as_str().expect(field))
            .expect(field)
    };
    Started {
        ceremony_id: body["ceremony_id"].as_str().expect("id").to_string(),
        signing_bytes: decode("device_signing_bytes"),
        verifier,
    }
}

async fn finish(
    state: &Arc<AppState>,
    uri: &str,
    bearer: Option<&str>,
    started: &Started,
    device: &Device,
    signature: &str,
) -> Reply {
    send(
        state,
        "POST",
        uri,
        Some(&serde_json::json!({
            "ceremony_id": started.ceremony_id,
            "code_verifier": started.verifier,
            "device_public_key": device.public_b64(),
            "device_signature": signature,
            "access_token": "near-ai-access-token-fixture",
        })),
        bearer,
    )
    .await
}

/// The unauthenticated provisioning an existing NEAR AI contributor ran:
/// account X.
async fn provision(h: &Harness, device: &Device) -> serde_json::Value {
    let started = start(&h.state, PROVISION_START, None, device).await;
    let signature = device.sign(&started.signing_bytes);
    let reply = finish(
        &h.state,
        PROVISION_FINISH,
        None,
        &started,
        device,
        &signature,
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "provision: {:?}",
        reply.json()
    );
    reply.json()
}

async fn count(admin: &deadpool_postgres::Object, sql: &str, params: &[&str]) -> i64 {
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
        .iter()
        .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    admin
        .query_one(sql, &params)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .get(0)
}

async fn audit_metadata(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    action: &str,
) -> Vec<serde_json::Value> {
    admin
        .query(
            "SELECT safe_metadata FROM trace_account_audit WHERE tenant_id = $1 AND action = $2",
            &[&tenant, &action],
        )
        .await
        .expect("audit")
        .into_iter()
        .map(|row| row.get(0))
        .collect()
}

// --- Upload claim to account admission, over the real token path -------------

/// The real upload-claim issuer, reading device keys from the test database,
/// and an ingest state that verifies what it signs. Nothing between the two
/// is hand-built: the claim is whatever the issuer mints for a
/// device-signed request, and the `TenantCtx` is whatever ingest's
/// `authenticate_ctx` makes of it.
struct ClaimPath {
    issuer: axum::Router,
    ingest: Arc<AppState>,
}

const CLAIM_ISSUER: &str = "trace-commons-test-upload-issuer";
const CLAIM_AUDIENCE: &str = "trace-commons-test-upload";

impl ClaimPath {
    fn new(h: &Harness) -> Self {
        use trace_commons_server::trace_upload_claim_issuer as issuer;
        let keys = issuer::generate_upload_claim_keypair().expect("issuer keypair");
        let config = issuer::TraceUploadClaimIssuerConfig {
            bind: "127.0.0.1:0".parse().expect("bind"),
            signing_private_key_pem: keys.private_key_pem,
            signing_public_key_pem: keys.public_key_pem.clone(),
            signing_kid: keys.suggested_kid.clone(),
            issuer: CLAIM_ISSUER.to_string(),
            audience: CLAIM_AUDIENCE.to_string(),
            max_ttl_seconds: 300,
            // No workload token is minted here; any valid key will do.
            workload_public_key_pem: keys.public_key_pem.clone(),
            workload_issuer: None,
            workload_audience: None,
            tenant_access_grant_db: None,
            require_tenant_access_grants: false,
            shutdown_grace_seconds: 1,
            request_timeout_seconds: 30,
            max_request_bytes: 64 * 1024,
            allowlist_source: None,
            allowlist_refresh_interval_seconds: 60,
            allowlist_max_stale_seconds: 600,
            onboarding_device_key_db: Some(h.backend.clone() as Arc<dyn Database>),
            // Required alongside the device registry; nothing here reads them.
            onboarding_ingest_url: Some("https://ingest.example".to_string()),
            onboarding_community_url: Some("https://community.example".to_string()),
            onboarding_profile_url: Some("https://community.example/profile".to_string()),
            onboarding_leaderboard_url: Some("https://community.example/leaderboard".to_string()),
            admin_bind: None,
            invite_admin_backend: None,
            invite_admin_registry: None,
            invite_registry_authoritative: false,
            near_ai_signup_policy_template: None,
        };
        let issuer = issuer::trace_upload_claim_issuer_router(config).expect("issuer router");
        let mut ingest = h.state.clone();
        let settings = Arc::make_mut(&mut ingest);
        settings.signed_token_verifier = Some(shared_signed_token_verifier(
            TraceCommonsSignedTokenVerifier {
                default_secret: None,
                keyed_secrets: BTreeMap::new(),
                // The issuer stamps its kid on every claim.
                default_eddsa_public_key: None,
                keyed_eddsa_public_keys: BTreeMap::from([(
                    keys.suggested_kid,
                    TraceCommonsSignedEddsaPublicKey {
                        pem: keys.public_key_pem,
                        not_before: None,
                        not_after: None,
                    },
                )]),
                managed_eddsa_key_ids: BTreeSet::new(),
                managed_eddsa_keyset_last_refreshed_at: None,
                managed_eddsa_keyset_last_refresh_failed_at: None,
                issuer: Some(CLAIM_ISSUER.to_string()),
                audience: Some(CLAIM_AUDIENCE.to_string()),
                revoked_jtis: BTreeSet::new(),
                max_ttl_seconds: Some(900),
                require_jti: true,
            },
        ));
        settings.require_eddsa_signed_tokens = true;
        Self { issuer, ingest }
    }

    /// `POST /v1/trace-upload-claim` exactly as the contributor sends it
    /// (`identity::build_signed_claim_request_with_scopes`): the body carries
    /// the tenant and a hashed per-user subject, and the device signs it.
    async fn mint(&self, device: &Device, tenant: &str) -> Result<String, StatusCode> {
        let device_key_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            &device.public_bytes(),
        );
        let body = serde_json::json!({
            "schema_version": "ironclaw.trace_upload_claim_request.v1",
            "tenant_id": tenant,
            "audience": CLAIM_AUDIENCE,
            "consent_scopes": [],
            "allowed_uses": [],
            "subject": trace_commons_protocol::onboarding::user_subject_hash("near-ai-user"),
            "requested_at": Utc::now().to_rfc3339(),
        })
        .to_string();
        let response = self
            .issuer
            .clone()
            .oneshot(
                axum::http::Request::post("/v1/trace-upload-claim")
                    .header("content-type", "application/json")
                    .header("x-trace-device-key-id", &device_key_id)
                    .header("x-trace-device-signature", device.sign(body.as_bytes()))
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("issuer responds");
        let status = response.status();
        if status != StatusCode::OK {
            return Err(status);
        }
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("claim body");
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("claim json");
        Ok(json["access_token"]
            .as_str()
            .expect("access_token")
            .to_string())
    }

    /// The `TenantCtx` ingest's upload route builds from that bearer.
    fn ctx(&self, token: &str) -> TenantCtx {
        authenticate_ctx(&self.ingest, &auth_headers(token)).expect("ingest accepts the claim")
    }

    async fn resolve(
        &self,
        ctx: &TenantCtx,
    ) -> Result<
        trace_commons_server::account_trust::TrustAccount,
        trace_commons_server::account_trust::TrustRefusal,
    > {
        let db = self.ingest.db_mirror.as_ref().expect("mirror");
        trace_commons_server::account_trust::resolve_contribution_account(
            db.as_ref(),
            ctx.tenant_id(),
            &ctx.account_principal_ref(),
        )
        .await
    }
}

async fn provisioned_account(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    device: &str,
) -> Uuid {
    admin
        .query_one(
            "SELECT account_id FROM trace_near_provisioned_devices
              WHERE tenant_id = $1 AND device_key_id = $2",
            &[&tenant, &device],
        )
        .await
        .expect("provisioned device")
        .get(0)
}

/// The pilot failure of 2026-10-12: a NEAR AI device's upload claim, minted
/// by the issuer and verified by ingest, must reach the account its device
/// was provisioned into. Until this was fixed, ingest looked the account up
/// by the claim's signed-claim principal, which no provisioning path writes,
/// and every `nearai-` upload was refused with `account_trust_refused`.
#[tokio::test]
async fn pg_a_near_ai_device_upload_claim_resolves_its_provisioned_account() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let device = Device::new();
    let provisioned = provision(&h, &device).await;
    let tenant = provisioned["tenant_id"]
        .as_str()
        .expect("tenant")
        .to_string();
    let key = provisioned["device_key_id"]
        .as_str()
        .expect("device")
        .to_string();
    assert!(tenant.starts_with("nearai-"), "{tenant}");

    let path = ClaimPath::new(&h);
    let token = path.mint(&device, &tenant).await.expect("issuer mints");
    let ctx = path.ctx(&token);
    assert_eq!(ctx.tenant_id(), tenant);
    assert_eq!(
        ctx.account_principal_ref(),
        trace_commons_server::account_trust::device_principal_ref(&tenant, &key),
        "admission looks the device up by the principal it was provisioned under"
    );

    let account = path
        .resolve(&ctx)
        .await
        .expect("the claim resolves its account");
    assert_eq!(
        account.account_id(),
        provisioned_account(&admin, &tenant, &key).await
    );
    let anchor = admission::anchor(&path.ingest, &ctx)
        .await
        .expect("anchor lookup")
        .expect("an anchored tenant has an anchor");
    assert!(
        trace_commons_protocol::admission::is_hash(&anchor),
        "{anchor}"
    );

    // The ledger rechecks the live device under the principal the
    // reservation carries (`trace_account_admission_live_device`), so the
    // derived principal has to satisfy that join too, not only the lookup.
    let policy = trace_commons_server::account_trust::parse_bounded_policy(
        r#"{"version":"claim-path-fixture","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
        &["claim-path-fixture"],
    )
    .expect("fixture policy");
    let reserved = h
        .backend
        .reserve_account_admission(
            &trace_commons_server::admission_ledger::AccountAdmissionReservation {
                account,
                principal_ref: ctx.account_principal_ref(),
                expected_trust_version: None,
                submission_id: Uuid::new_v4(),
                body_hash: "0".repeat(64),
                lease_id: Uuid::new_v4(),
                policy,
                lease_seconds: 60,
                attested: false,
            },
        )
        .await
        .expect("reserve");
    assert!(
        matches!(
            reserved.decision,
            trace_commons_server::admission_ledger::AdmissionDecision::Reserved
        ),
        "the device's claim reserves against its account: {:?}",
        reserved.decision
    );

    // The reservation's rows hold `trace_accounts`, which `drop_tenants` removes.
    for table in [
        "trace_account_admission_submissions",
        "trace_account_admission_budget",
        "trace_account_trust",
    ] {
        admin
            .execute(
                &format!("DELETE FROM {table} WHERE tenant_id = $1"),
                &[&tenant],
            )
            .await
            .unwrap_or_else(|error| panic!("cleanup {table}: {error}"));
    }
    reset_account_rate_limiter_for_db_test().await;
    drop_tenants(&admin, &[&tenant]).await;
}

/// One device's claim never reaches another device's account. Each token
/// yields only the principal of the key it was minted for: a device in
/// another tenant (another account) is unreachable, the issuer will not
/// mint a claim for a key under a tenant it is not registered in, and in
/// one tenant a revoked second device stops resolving while the first
/// carries on, so resolution is per device and not per tenant.
#[tokio::test]
async fn pg_one_devices_upload_claim_cannot_resolve_another_devices_account() {
    let Some(h) = harness().await else {
        return;
    };
    let Some(other) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let device_a = Device::new();
    let a = provision(&h, &device_a).await;
    let tenant_a = a["tenant_id"].as_str().expect("tenant").to_string();
    let key_a = a["device_key_id"].as_str().expect("device").to_string();
    let device_b = Device::new();
    let b = provision(&other, &device_b).await;
    let tenant_b = b["tenant_id"].as_str().expect("tenant").to_string();
    let key_b = b["device_key_id"].as_str().expect("device").to_string();
    assert_ne!(tenant_a, tenant_b);

    let path = ClaimPath::new(&h);
    let ctx_a = path.ctx(&path.mint(&device_a, &tenant_a).await.expect("mint a"));
    let ctx_b = path.ctx(&path.mint(&device_b, &tenant_b).await.expect("mint b"));
    let account_a = path.resolve(&ctx_a).await.expect("a resolves");
    let account_b = path.resolve(&ctx_b).await.expect("b resolves");
    assert_eq!(
        account_a.account_id(),
        provisioned_account(&admin, &tenant_a, &key_a).await
    );
    assert_eq!(
        account_b.account_id(),
        provisioned_account(&admin, &tenant_b, &key_b).await
    );
    assert_ne!(account_a.account_id(), account_b.account_id());

    // Device A asking for a claim in B's tenant: its key is not registered
    // there, so the issuer refuses before any claim exists.
    assert_eq!(
        path.mint(&device_a, &tenant_b).await,
        Err(StatusCode::FORBIDDEN)
    );

    // A second device provisioned by the same NEAR AI login lands in A's
    // tenant. Each claim names its own key's principal, and revoking the
    // second key refuses only the second.
    let device_a2 = Device::new();
    let a2 = provision(&h, &device_a2).await;
    let key_a2 = a2["device_key_id"].as_str().expect("device").to_string();
    assert_eq!(a2["tenant_id"].as_str(), Some(tenant_a.as_str()));
    let ctx_a2 = path.ctx(&path.mint(&device_a2, &tenant_a).await.expect("mint a2"));
    assert_eq!(
        ctx_a2.account_principal_ref(),
        trace_commons_server::account_trust::device_principal_ref(&tenant_a, &key_a2)
    );
    assert_ne!(
        ctx_a2.account_principal_ref(),
        ctx_a.account_principal_ref()
    );
    admin
        .execute(
            "UPDATE device_keys SET revoked_at = NOW() WHERE tenant_id = $1 AND device_key_id = $2",
            &[&tenant_a, &key_a2],
        )
        .await
        .expect("revoke a2");
    assert_eq!(
        path.resolve(&ctx_a2).await,
        Err(trace_commons_server::account_trust::TrustRefusal::Refused),
        "a revoked device's still-valid claim resolves nothing"
    );
    assert_eq!(
        path.resolve(&ctx_a)
            .await
            .map(|account| account.account_id()),
        Ok(account_a.account_id()),
        "revoking one device leaves the other resolving"
    );

    reset_account_rate_limiter_for_db_test().await;
    drop_tenants(&admin, &[&tenant_a, &tenant_b]).await;
}

// --- Open sign-up (staging S12) ----------------------------------------------

/// Anyone with a NEAR AI account signs up (staging spec S12, section 3.12).
///
/// A staging build admits a new NEAR AI login exactly as a default build does:
/// no invite and no list. This test has no `cfg`, so it runs in both builds and holds them
/// to the same answer. The subjects are minted here, not by the fixtures, so
/// nothing a fixture does on their behalf can be what lets them in.
mod open_sign_up {
    use super::*;

    #[tokio::test]
    async fn a_new_near_ai_login_gets_an_account_with_no_invite() {
        let Some(h) = harness().await else { return };
        let admin = h.admin().await;

        // Sign-in: a login nobody has seen gets a tenant, account and anchor.
        let subject = unseen_subject();
        h.sign_in_to_near_ai_as(&subject);
        let device = Device::new();
        let started = start(&h.state, PROVISION_START, None, &device).await;
        let signature = device.sign(&started.signing_bytes);
        let reply = finish(
            &h.state,
            PROVISION_FINISH,
            None,
            &started,
            &device,
            &signature,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
        let tenant = reply.json()["tenant_id"]
            .as_str()
            .expect("tenant")
            .to_string();
        assert!(tenant.starts_with("nearai-"), "a new tenant of its own");
        assert_eq!(
            anchors_for(&admin, &identity().login_index_label(&subject))
                .await
                .len(),
            1,
            "one anchor for the new login"
        );
        assert_eq!(
            audit_metadata(&admin, &tenant, "near_ai_login_provisioned").await,
            vec![serde_json::json!({"identity": "near_ai_login", "admission": "not_granted"})]
        );
        assert_eq!(
            count(
                &admin,
                "SELECT count(*) FROM trace_account_invite_grants WHERE tenant_id = $1",
                &[tenant.as_str()],
            )
            .await,
            0,
            "no invite was used"
        );

        drop_tenants(&admin, &[&tenant]).await;
    }

    /// The contributor grant rows for a device principal:
    /// `(allowed_uses, metadata source)`.
    async fn contributor_grants(
        admin: &deadpool_postgres::Object,
        tenant: &str,
        device_key_id: &str,
    ) -> Vec<(serde_json::Value, Option<String>)> {
        let principal =
            trace_commons_server::account_trust::device_principal_ref(tenant, device_key_id);
        admin
            .query(
                "SELECT allowed_uses, metadata_json->>'source' FROM trace_tenant_access_grants
                  WHERE tenant_id = $1 AND principal_ref = $2 AND role = 'contributor'
                    AND status = 'active'",
                &[&tenant, &principal],
            )
            .await
            .unwrap()
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect()
    }

    /// Owner decision 2026-10-24 ("Honor app consent"): with the deployment's
    /// sign-up policy template configured, a NEAR AI sign-up writes the
    /// device's contributor grant from it -- the grant the issuer
    /// reads as the claim ceiling, so `model_training` the contributor
    /// consented to reaches their traces. Without a template, none is
    /// written and the floor stays.
    #[tokio::test]
    async fn sign_up_writes_the_contributor_grant_from_the_template() {
        let Some(mut h) = harness().await else { return };
        let admin = h.admin().await;
        let template =
            trace_commons_server::near_ai_signup_policy::NearAiSignupPolicyTemplate::parse(
                r#"{"policy_version":"ingest-test-v1",
                    "allowed_consent_scopes":["debugging_evaluation","model_training","public_attribution"],
                    "allowed_uses":["debugging","evaluation","model_training","aggregate_analytics"]}"#,
            )
            .unwrap();
        let expected = vec![(
            serde_json::json!([
                "debugging",
                "evaluation",
                "model_training",
                "aggregate_analytics"
            ]),
            Some("near_ai_signup".to_string()),
        )];

        // Without a template: an account, no grant.
        let subject = unseen_subject();
        h.sign_in_to_near_ai_as(&subject);
        let device = Device::new();
        let started = start(&h.state, PROVISION_START, None, &device).await;
        let signature = device.sign(&started.signing_bytes);
        let reply = finish(
            &h.state,
            PROVISION_FINISH,
            None,
            &started,
            &device,
            &signature,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
        let untemplated_tenant = reply.json()["tenant_id"].as_str().unwrap().to_string();
        let untemplated_device = reply.json()["device_key_id"].as_str().unwrap().to_string();
        assert!(
            contributor_grants(&admin, &untemplated_tenant, &untemplated_device)
                .await
                .is_empty(),
            "no template, no grant"
        );

        Arc::get_mut(&mut h.state)
            .expect("the harness owns its state")
            .near_ai_signup_policy_template = Some(template);

        // Sign-up with the template.
        h.sign_in_to_near_ai_as(&unseen_subject());
        let device = Device::new();
        let started = start(&h.state, PROVISION_START, None, &device).await;
        let signature = device.sign(&started.signing_bytes);
        let reply = finish(
            &h.state,
            PROVISION_FINISH,
            None,
            &started,
            &device,
            &signature,
        )
        .await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
        let tenant = reply.json()["tenant_id"].as_str().unwrap().to_string();
        let device_key_id = reply.json()["device_key_id"].as_str().unwrap().to_string();
        assert_eq!(
            contributor_grants(&admin, &tenant, &device_key_id).await,
            expected
        );
        drop_tenants(&admin, &[&untemplated_tenant, &tenant]).await;
    }

    /// The sign-in with no admission ledger and no witness: what a staging
    /// server runs (staging spec S12).
    #[cfg(feature = "near-ai-staging")]
    async fn sign_in_without_the_ledger(h: &Harness, subject: &str) -> Reply {
        assert!(h.state.admission.is_none(), "no admission ledger");
        assert!(h.state.witness_bypass.is_none(), "no witness");
        h.sign_in_to_near_ai_as(subject);
        let device = Device::new();
        let started = start(&h.state, PROVISION_START, None, &device).await;
        let signature = device.sign(&started.signing_bytes);
        finish(
            &h.state,
            PROVISION_FINISH,
            None,
            &started,
            &device,
            &signature,
        )
        .await
    }

    /// A staging build creates accounts without the admission ledger or the
    /// witness. Production is unchanged; see the default-build twin below.
    #[cfg(feature = "near-ai-staging")]
    #[tokio::test]
    async fn a_staging_build_signs_up_without_the_admission_ledger() {
        let Some(h) = harness_with_admission(false).await else {
            return;
        };
        let admin = h.admin().await;
        assert_eq!(
            near_provisioning::near_ai_login_v2_readiness_refusal(&h.state),
            None
        );
        let subject = unseen_subject();
        let reply = sign_in_without_the_ledger(&h, &subject).await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
        let tenant = reply.json()["tenant_id"]
            .as_str()
            .expect("tenant")
            .to_string();
        assert!(tenant.starts_with("nearai-"), "a new tenant of its own");
        assert_eq!(
            anchors_for(&admin, &identity().login_index_label(&subject))
                .await
                .len(),
            1
        );
        drop_tenants(&admin, &[&tenant]).await;
    }

    /// A default build still requires the admission ledger before it creates
    /// an account for a NEAR AI login, and refuses by that control's name.
    #[cfg(not(feature = "near-ai-staging"))]
    #[tokio::test]
    async fn a_default_build_refuses_sign_up_without_the_admission_ledger() {
        let Some(h) = harness_with_admission(false).await else {
            return;
        };
        let admin = h.admin().await;
        assert_eq!(
            near_provisioning::near_ai_login_v2_readiness_refusal(&h.state),
            Some("near_provisioning_admission_ready")
        );
        assert!(h.state.admission.is_none(), "no admission ledger");
        let subject = unseen_subject();
        h.sign_in_to_near_ai_as(&subject);
        let (_, challenge) = pkce();
        let reply = send(
            &h.state,
            "POST",
            PROVISION_START,
            Some(&serde_json::json!({
                "device_public_key": Device::new().public_b64(),
                "code_challenge": challenge,
                "code_challenge_method": "S256",
            })),
            None,
        )
        .await;
        assert_ne!(reply.status, StatusCode::OK, "{:?}", reply.json());
        assert!(
            anchors_for(&admin, &identity().login_index_label(&subject))
                .await
                .is_empty(),
            "nothing written"
        );
    }

    /// A v2 NEAR AI sign-up, then that account's first upload as the app
    /// makes it: an issuer-signed upload claim naming the device the sign-up
    /// bound, and an envelope with no admission evidence. Returns the
    /// sign-up's tenant and the upload's reply.
    async fn sign_up_then_upload(h: &Harness) -> (String, Reply) {
        let subject = unseen_subject();
        h.sign_in_to_near_ai_as(&subject);
        let device = Device::new();
        let started = start(&h.state, PROVISION_START, None, &device).await;
        let signature = device.sign(&started.signing_bytes);
        let signed_up = finish(
            &h.state,
            PROVISION_FINISH,
            None,
            &started,
            &device,
            &signature,
        )
        .await;
        assert_eq!(signed_up.status, StatusCode::OK, "{:?}", signed_up.json());
        let tenant = signed_up.json()["tenant_id"]
            .as_str()
            .expect("tenant")
            .to_string();
        assert!(tenant.starts_with("nearai-"), "a new tenant of its own");

        // The upload claim the issuer mints for this device.
        let device_key_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            &device.public_bytes(),
        );
        let actor = trace_commons_server::account_trust::device_claim_actor(
            &tenant,
            &device_key_id,
            Some(&trace_commons_protocol::onboarding::user_subject_hash(
                &subject,
            )),
        );
        let token = eddsa_signed_tenant_token(serde_json::json!({
            "tenant_id": tenant,
            "role": "contributor",
            "sub": actor,
            "principal_ref": actor,
            "allowed_consent_scopes": ["debugging_evaluation"],
            "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"],
            "exp": (Utc::now() + Duration::minutes(5)).timestamp(),
        }));
        let mut state = h.state.clone();
        Arc::make_mut(&mut state).signed_token_verifier = Some(shared_signed_token_verifier(
            test_eddsa_signed_token_verifier(),
        ));
        let envelope = serde_json::to_value(low_risk_envelope().await).expect("envelope");
        let uploaded = send(&state, "POST", "/v1/traces", Some(&envelope), Some(&token)).await;
        (tenant, uploaded)
    }

    /// A staging build admits the first upload of an account its own v2
    /// NEAR AI sign-up created, with no admission ledger and no witness
    /// (staging spec S12). Before this, every staging account was refused
    /// at upload with `legacy_admission_unconfigured`.
    #[cfg(feature = "near-ai-staging")]
    #[tokio::test]
    async fn a_staging_build_admits_the_first_upload_without_the_admission_ledger() {
        let Some(h) = harness_with_admission(false).await else {
            return;
        };
        assert!(h.state.admission.is_none(), "no admission ledger");
        assert!(h.state.account_admission.is_none(), "no account admission");
        assert!(h.state.witness_bypass.is_none(), "no witness");
        let (_tenant, uploaded) = sign_up_then_upload(&h).await;
        assert_eq!(uploaded.status, StatusCode::OK, "{:?}", uploaded.json());
        assert_eq!(
            uploaded.json()["status"],
            "accepted",
            "{:?}",
            uploaded.json()
        );
    }

    /// The default-build twin: the same sign-up and upload with no admission
    /// ledger is still refused, by the same label it always was. A
    /// production build never takes the staging admission.
    #[cfg(not(feature = "near-ai-staging"))]
    #[tokio::test]
    async fn a_default_build_refuses_the_first_upload_without_the_admission_ledger() {
        // Sign-up readiness on; the ledger itself is still absent.
        let Some(h) = harness_with_admission(true).await else {
            return;
        };
        assert!(h.state.admission.is_none(), "no admission ledger");
        assert!(h.state.account_admission.is_none(), "no account admission");
        let (_tenant, uploaded) = sign_up_then_upload(&h).await;
        assert_eq!(
            uploaded.status,
            StatusCode::FORBIDDEN,
            "{:?}",
            uploaded.json()
        );
        assert_eq!(uploaded.json()["error"], "admission_refused");
    }

    fn published_witness() -> serde_json::Value {
        serde_json::json!({
            "url": "https://witness.example",
            "signing_address": format!("0x{}", "ab".repeat(20)),
            "expected_measurements": [format!("mrtd={}", "cd".repeat(48))],
        })
    }

    /// The old capabilities route answers byte for byte as it did before v2
    /// published the issuer: with a witness configured, the document every
    /// deployed client reads.
    #[tokio::test]
    async fn the_old_capabilities_route_is_unchanged() {
        let Some(h) = harness_with_admission(!cfg!(feature = "near-ai-staging")).await else {
            return;
        };
        let _witness = EnvVar::set(
            "TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON",
            Some(&published_witness().to_string()),
        );
        let _receipt = EnvVar::set(
            "TRACE_COMMONS_NEAR_PROVISIONING_RECEIPT_ENDPOINT",
            Some("https://receipts.example"),
        );
        let legacy = send(&h.state, "GET", LEGACY_CAPABILITIES, None, None).await;
        assert_eq!(legacy.status, StatusCode::OK);
        assert_eq!(
            legacy.bytes,
            serde_json::to_vec(&serde_json::json!({
                "ready": false,
                "near_ai_login_ready": true,
                "network": null,
                "witness": published_witness(),
                "issuer_url": "https://issuer.example",
                "audience": "upload",
                "inference_receipt_endpoint": "https://receipts.example",
                "funding_available": false,
            }))
            .unwrap(),
            "{:?}",
            legacy.json()
        );
    }
}

// --- #1668 review: a revoked device key is refused by name -------------------

/// The wire label every sign-in finish answers with when the device's key
/// was revoked by its account.
const DEVICE_KEY_REVOKED_WIRE: &str = "device_key_revoked";

/// A device whose key its account revoked signs in again through NEAR AI
/// provisioning: refused by name with 409, not the uniform deny, so the
/// client can tell the person to sign out and sign in with a new key. A new
/// key from the same login still signs in.
#[tokio::test]
async fn pg_a_revoked_device_key_is_refused_by_name_at_provisioning() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let device = Device::new();
    let first = provision(&h, &device).await;
    let tenant = first["tenant_id"].as_str().expect("tenant").to_string();
    let revoked = admin
        .execute(
            "UPDATE device_keys SET revoked_at = now() WHERE device_key_id = $1",
            &[&first["device_key_id"].as_str().expect("device key id")],
        )
        .await
        .expect("revoke the key");
    assert_eq!(revoked, 1);

    let started = start(&h.state, PROVISION_START, None, &device).await;
    let signature = device.sign(&started.signing_bytes);
    let reply = finish(
        &h.state,
        PROVISION_FINISH,
        None,
        &started,
        &device,
        &signature,
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT, "{:?}", reply.json());
    assert_eq!(
        reply.json(),
        serde_json::json!({ "error": DEVICE_KEY_REVOKED_WIRE }),
        "the refusal is named, and names nothing but itself"
    );

    let again = provision(&h, &Device::new()).await;
    assert_eq!(
        again["tenant_id"],
        tenant.as_str(),
        "a new key joins the same account"
    );
    reset_account_rate_limiter_for_db_test().await;
    drop_tenants(&admin, &[&tenant]).await;
}

/// How a test request authenticates to the account surface.
enum AccountAuth<'a> {
    /// A browser session cookie (`{b64url(tenant)}.{secret}`).
    Cookie(&'a str),
    /// A native `tcn1_` bearer token.
    Bearer(&'a str),
}

async fn send_account(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    body: Option<&serde_json::Value>,
    auth: &AccountAuth<'_>,
) -> Reply {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    builder = match auth {
        AccountAuth::Cookie(value) => builder
            .header(
                axum::http::header::COOKIE,
                format!("{ACCOUNT_SESSION_COOKIE}={value}"),
            )
            .header("sec-fetch-site", "same-origin"),
        AccountAuth::Bearer(token) => builder.header(AUTHORIZATION, format!("Bearer {token}")),
    };
    let body = match body {
        Some(body) => {
            builder = builder.header(CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(body).expect("json"))
        }
        None => Body::empty(),
    };
    let response = app(state.clone())
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("router is infallible");
    Reply::read(response).await
}

/// A session on `account` that no NEAR AI sign-in minted, inserted as a
/// device-link sign-in or the loopback native sign-in would leave it.
/// Returns the raw secret.
async fn insert_plain_session(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    account: Uuid,
    client_kind: &str,
) -> String {
    let secret = trace_commons_server::account_session::generate_session_secret();
    let token_hash = trace_commons_server::account_session::hash_secret(&secret);
    admin
        .execute(
            "INSERT INTO trace_sessions(tenant_id,session_id,account_id,token_hash,client_kind,expires_at)
             VALUES($1,$2,$3,$4,$5,now() + interval '1 hour')",
            &[&tenant, &Uuid::new_v4(), &account, &token_hash, &client_kind],
        )
        .await
        .expect("insert session");
    secret
}

/// Every route behind the strong-authenticator gate, each with a well-formed
/// body, so a `403` can only have come from the gate.
fn gated_requests() -> Vec<(&'static str, &'static str, Option<serde_json::Value>)> {
    vec![
        ("POST", "/v1/account/near/enroll/start", None),
        (
            "POST",
            "/v1/account/near/enroll/finish",
            Some(serde_json::json!({
                "accountId": "alice.near",
                "publicKey": "ed25519:unknown-key",
                "signature": "AAAA",
            })),
        ),
        (
            "PATCH",
            "/v1/account/near-identities/ed25519:unknown-key/payout",
            Some(serde_json::json!({"payout": true})),
        ),
        (
            "DELETE",
            "/v1/account/near-identities/ed25519:unknown-key",
            None,
        ),
        (
            "POST",
            "/v1/account/merge/confirm",
            Some(serde_json::json!({"proposal_id": Uuid::new_v4()})),
        ),
    ]
}

/// A NEAR AI login is a strong authenticator: an account that signs in with
/// NEAR AI and holds no NEAR wallet key is not left with zero, so a weak
/// session (a browser device-link session, or a native token no NEAR AI
/// sign-in minted) is refused every authenticator change, the payout
/// target and an account merge.
#[tokio::test]
async fn pg_a_weak_session_cannot_change_authenticators_on_a_near_ai_login_account() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let device = Device::new();
    let provisioned = provision(&h, &device).await;
    let tenant = provisioned["tenant_id"]
        .as_str()
        .expect("tenant")
        .to_string();
    let account: Uuid = provisioned["account_id"]
        .as_str()
        .expect("account")
        .parse()
        .expect("uuid");

    let web = insert_plain_session(&admin, &tenant, account, "web").await;
    let web_cookie = format!(
        "{}.{}",
        base64::Engine::encode(
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
            tenant.as_bytes()
        ),
        web
    );
    let native = insert_plain_session(&admin, &tenant, account, "native").await;
    let native_token =
        trace_commons_server::account_native_auth::native_token_value(&tenant, &native);

    for auth in [
        AccountAuth::Cookie(&web_cookie),
        AccountAuth::Bearer(&native_token),
    ] {
        for (method, uri, body) in gated_requests() {
            reset_account_rate_limiter_for_db_test().await;
            let reply = send_account(&h.state, method, uri, body.as_ref(), &auth).await;
            assert_eq!(
                reply.status,
                StatusCode::FORBIDDEN,
                "{method} {uri}: {:?}",
                reply.json()
            );
        }
    }

    drop_tenants(&admin, &[&tenant]).await;
}

/// No NEAR AI session passes the gate, however recent: a NEAR AI sign-in
/// and the daemon's unattended renewal mint the same session, so neither is
/// proof that a person just signed in. On an account with a strong
/// authenticator, only a NEAR wallet session may make these changes.
#[tokio::test]
async fn pg_a_near_ai_sign_in_session_cannot_change_authenticators() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let device = Device::new();
    let provisioned = provision(&h, &device).await;
    let tenant = provisioned["tenant_id"]
        .as_str()
        .expect("tenant")
        .to_string();
    let token = provisioned["access_token"]
        .as_str()
        .expect("access token")
        .to_string();
    let auth = AccountAuth::Bearer(&token);

    for (method, uri, body) in gated_requests() {
        reset_account_rate_limiter_for_db_test().await;
        let reply = send_account(&h.state, method, uri, body.as_ref(), &auth).await;
        assert_eq!(
            reply.status,
            StatusCode::FORBIDDEN,
            "{method} {uri}: {:?}",
            reply.json()
        );
    }

    drop_tenants(&admin, &[&tenant]).await;
}

/// The gate counts a NEAR AI login as a strong authenticator, once however
/// many anchors a merge left on the account; since V150 a merge leaves the
/// absorbed one retired, and a retired login counts for nothing. Removing a NEAR key still
/// reports how many NEAR keys remain, so a NEAR AI account that removes its
/// last one reads `0`, as it did before.
#[tokio::test]
async fn pg_a_near_ai_account_counts_its_login_once_and_reports_remaining_near_keys() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let device = Device::new();
    let provisioned = provision(&h, &device).await;
    let tenant = provisioned["tenant_id"]
        .as_str()
        .expect("tenant")
        .to_string();
    let account: Uuid = provisioned["account_id"]
        .as_str()
        .expect("account")
        .parse()
        .expect("uuid");
    let anchor = provisioned["anchor_hash"]
        .as_str()
        .expect("anchor")
        .to_string();
    // A second login anchor, as a merge that re-keys the absorbed account's
    // anchors onto the survivor leaves: retired, since V150 keeps one live
    // anchor per account.
    admin
        .execute(
            "INSERT INTO trace_near_account_anchors
                    (tenant_id, anchor_hash, account_id, sealed_account_name,
                     index_pepper_ref, account_name_key_ref, identity_source, auth_provider,
                     retired_at)
             SELECT tenant_id, 'sha256:' || repeat('e', 64), account_id, sealed_account_name,
                    index_pepper_ref, account_name_key_ref, identity_source, auth_provider, now()
               FROM trace_near_account_anchors WHERE anchor_hash = $1",
            &[&anchor],
        )
        .await
        .expect("second anchor");
    let backend = h.backend.clone();
    assert_eq!(
        backend
            .count_active_strong_authenticators(&tenant, account)
            .await
            .expect("count"),
        1
    );

    admin
        .execute(
            "INSERT INTO trace_near_identities(tenant_id, public_key, near_account_id, account_id)
             VALUES ($1, 'ed25519:near-ai-account-key', 'alice.near', $2)",
            &[&tenant, &account],
        )
        .await
        .expect("near key");
    assert_eq!(
        backend
            .count_active_strong_authenticators(&tenant, account)
            .await
            .expect("count"),
        2
    );
    let revoked = backend
        .revoke_account_near_identity(&tenant, account, "ed25519:near-ai-account-key")
        .await
        .expect("revoke");
    assert!(revoked.removed);
    assert_eq!(revoked.remaining_strong, 0, "no NEAR key remains");

    admin
        .execute(
            "DELETE FROM trace_near_account_anchors WHERE anchor_hash = $1",
            &[&format!("sha256:{}", "e".repeat(64))],
        )
        .await
        .expect("cleanup");
    drop_tenants(&admin, &[&tenant]).await;
}

// --- Busy is not refused --------------------------------------------------------
//
// The contributor's NEAR AI sign-in check (#1840) runs this ceremony to ask
// whether a new near.ai login belongs to the enrolled account, and reads the
// uniform `400` as "it belongs to another one". A commons that is rate
// limiting, or whose database or NEAR AI did not answer, must say so with
// `429` or `503` instead, and every refusal must stay the one `400`.

/// A NEAR AI stand-in that answers every `GET /users/me` with `status`.
async fn stub_near_ai_status(status: StatusCode) -> String {
    let router = axum::Router::new().route(
        "/users/me",
        axum::routing::get(move || async move { status }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move { axum::serve(listener, router).await.expect("stub") });
    base
}

/// A base URL nothing listens on: the port was bound and released.
async fn closed_base_url() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    drop(listener);
    base
}

/// `state` with NEAR AI introspection pointed at `base`.
fn with_near_ai(state: &Arc<AppState>, base: String) -> Arc<AppState> {
    let mut state = state.clone();
    Arc::make_mut(&mut state).near_ai_introspection_base_url = Some(base);
    state
}

/// A finish body that passes every shape check, for a ceremony nobody
/// started.
fn well_formed_unknown_finish(device: &Device) -> serde_json::Value {
    let (verifier, _) = pkce();
    serde_json::json!({
        "ceremony_id": format!("unknown-{}", Uuid::new_v4().simple()),
        "code_verifier": verifier,
        "device_public_key": device.public_b64(),
        "device_signature": device.sign(b"not the ceremony"),
        "access_token": "near-ai-access-token-fixture",
    })
}

/// A `POST` of `body` to `uri`, unauthenticated.
async fn post(state: &Arc<AppState>, uri: &str, body: &serde_json::Value) -> Reply {
    send(state, "POST", uri, Some(body), None).await
}

/// The uniform deny, byte for byte, as the router answers it.
fn uniform_deny_bytes() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "error": "native sign-in could not be completed"
    }))
    .expect("json")
}

fn assert_uniform_deny(answer: &Reply, cause: &str) {
    assert_eq!(answer.status, StatusCode::BAD_REQUEST, "{cause}");
    assert_eq!(answer.bytes, uniform_deny_bytes(), "{cause}");
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"), "{cause}");
    assert_eq!(answer.retry_after, None, "{cause}");
}

fn assert_unavailable(answer: &Reply, cause: &str) {
    assert_eq!(answer.status, StatusCode::SERVICE_UNAVAILABLE, "{cause}");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&answer.bytes).expect("json"),
        serde_json::json!({ "error": "near_ai_sign_in_unavailable" }),
        "{cause}: one label, naming no dependency"
    );
    assert_eq!(answer.cache_control.as_deref(), Some("no-store"), "{cause}");
}

/// Over the per-address limit, a NEAR AI start and finish answer `429` with
/// `Retry-After`, not the uniform deny the client reads as a refusal. Under
/// it, a refused request is still that deny.
#[tokio::test]
async fn pg_a_rate_limited_near_ai_sign_in_answers_429_not_the_refusal() {
    let Some(h) = harness().await else {
        return;
    };
    // The v1 routes run the same functions; this harness publishes no
    // enrollment witness, so they are not ready here and answer the deny
    // before the limit is counted.
    for uri in [PROVISION_START, PROVISION_FINISH] {
        reset_account_rate_limiter_for_db_test().await;
        // An empty body is refused after the limit is counted, so each one
        // spends a slot and answers the uniform deny until the limit is hit.
        for attempt in 0..30 {
            let answer = post(&h.state, uri, &serde_json::json!({})).await;
            assert_uniform_deny(&answer, &format!("{uri} attempt {attempt}"));
        }
        let limited = post(&h.state, uri, &serde_json::json!({})).await;
        assert_eq!(limited.status, StatusCode::TOO_MANY_REQUESTS, "{uri}");
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&limited.bytes).expect("json"),
            serde_json::json!({
                "error": "near_ai_sign_in_rate_limited",
                "retry_after_seconds": 60,
            }),
            "{uri}"
        );
        assert_eq!(limited.retry_after.as_deref(), Some("60"), "{uri}");
        assert_eq!(limited.cache_control.as_deref(), Some("no-store"), "{uri}");
    }
    reset_account_rate_limiter_for_db_test().await;
}

/// A database that does not answer is `503` at start and at finish, not the
/// uniform deny. The account registry here points at a port nothing listens
/// on, so every connection the pool tries is refused.
#[tokio::test]
async fn pg_a_near_ai_sign_in_whose_database_does_not_answer_answers_503() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let closed = closed_base_url().await;
    let port = closed.rsplit(':').next().expect("port");
    let unreachable = Arc::new(
        PgBackend::new(&DatabaseConfig {
            url: SecretString::from(format!("postgres://nobody@127.0.0.1:{port}/nothing")),
            pool_size: 1,
            ssl_mode: trace_commons_server::config::SslMode::Disable,
            login_resolver_url: None,
            gate_driver_url: None,
            pii_backstop_driver_url: None,
            invite_registry_url: None,
        })
        .await
        .expect("the pool is built lazily"),
    );
    let mut state = h.state.clone();
    Arc::make_mut(&mut state).db_mirror = Some(unreachable as Arc<dyn Database>);
    let device = Device::new();
    let (_, challenge) = pkce();

    let started = post(
        &state,
        PROVISION_START,
        &serde_json::json!({
            "device_public_key": device.public_b64(),
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        }),
    )
    .await;
    assert_unavailable(&started, "start, database down");

    let finished = post(
        &state,
        PROVISION_FINISH,
        &well_formed_unknown_finish(&device),
    )
    .await;
    assert_unavailable(&finished, "finish, database down");
    reset_account_rate_limiter_for_db_test().await;
}

/// NEAR AI not answering -- nothing listening, or a `503` or `429` of its own
/// -- is `503` at finish, and writes nothing. The ceremony is consumed all the
/// same (the wire contract: a `503` at finish means start again), so the same
/// finish again is refused even once NEAR AI answers: a finish body is a
/// complete credential and must not complete a sign-in after the owner gave
/// up on it.
#[tokio::test]
async fn pg_a_near_ai_finish_whose_near_ai_does_not_answer_answers_503() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    for (cause, base) in [
        ("near ai unreachable", closed_base_url().await),
        (
            "near ai 503",
            stub_near_ai_status(StatusCode::SERVICE_UNAVAILABLE).await,
        ),
        (
            "near ai 429",
            stub_near_ai_status(StatusCode::TOO_MANY_REQUESTS).await,
        ),
    ] {
        let state = with_near_ai(&h.state, base);
        let device = Device::new();
        let devices_before = count(
            &admin,
            "SELECT count(*)::bigint FROM device_keys WHERE public_key = $1",
            &[&device.public_b64()],
        )
        .await;
        let started = start(&state, PROVISION_START, None, &device).await;
        let signature = device.sign(&started.signing_bytes);
        let answer = finish(
            &state,
            PROVISION_FINISH,
            None,
            &started,
            &device,
            &signature,
        )
        .await;
        assert_unavailable(&answer, cause);
        assert_eq!(
            count(
                &admin,
                "SELECT count(*)::bigint FROM device_keys WHERE public_key = $1",
                &[&device.public_b64()],
            )
            .await,
            devices_before,
            "{cause}: an unanswered finish registered the device"
        );
        // Consumed: the same finish, even with NEAR AI answering, is refused.
        let resent = finish(
            &h.state,
            PROVISION_FINISH,
            None,
            &started,
            &device,
            &signature,
        )
        .await;
        assert_uniform_deny(&resent, &format!("{cause}, resent after the 503"));
        signs_in_after_starting_again(&h, &admin, &device, cause).await;
    }
    reset_account_rate_limiter_for_db_test().await;
}

/// Every way a finish is refused answers the same `400`, byte for byte, with
/// the same headers -- including the one the sign-in check exists to find,
/// a device whose key is held by the account of a different near.ai login.
/// Telling busy from refused added no way to tell one refusal from another.
#[tokio::test]
async fn pg_every_near_ai_finish_refusal_is_the_identical_uniform_deny() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let mut answers: Vec<(&str, Reply)> = Vec::new();

    // Not a finish body at all.
    answers.push((
        "malformed body",
        post(&h.state, PROVISION_FINISH, &serde_json::json!({})).await,
    ));

    // A ceremony nobody started.
    let stranger = Device::new();
    answers.push((
        "unknown ceremony",
        post(
            &h.state,
            PROVISION_FINISH,
            &well_formed_unknown_finish(&stranger),
        )
        .await,
    ));

    // A signature over something other than this ceremony's bytes.
    let device = Device::new();
    let started = start(&h.state, PROVISION_START, None, &device).await;
    let wrong = device.sign(b"some other ceremony");
    answers.push((
        "wrong device signature",
        finish(&h.state, PROVISION_FINISH, None, &started, &device, &wrong).await,
    ));

    // NEAR AI answered, and named no one: its 401.
    let refusing = with_near_ai(
        &h.state,
        stub_near_ai_status(StatusCode::UNAUTHORIZED).await,
    );
    let device = Device::new();
    let started = start(&refusing, PROVISION_START, None, &device).await;
    let signature = device.sign(&started.signing_bytes);
    answers.push((
        "near ai 401",
        finish(
            &refusing,
            PROVISION_FINISH,
            None,
            &started,
            &device,
            &signature,
        )
        .await,
    ));

    // A replayed finish: the first one succeeded and consumed the ceremony.
    let enrolled = Device::new();
    let started = start(&h.state, PROVISION_START, None, &enrolled).await;
    let signature = enrolled.sign(&started.signing_bytes);
    let first = finish(
        &h.state,
        PROVISION_FINISH,
        None,
        &started,
        &enrolled,
        &signature,
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{:?}", first.json());
    let enrolled_tenant = first.json()["tenant_id"]
        .as_str()
        .expect("tenant")
        .to_string();
    answers.push((
        "replayed ceremony",
        finish(
            &h.state,
            PROVISION_FINISH,
            None,
            &started,
            &enrolled,
            &signature,
        )
        .await,
    ));

    // The enrolled device, now signed in to near.ai as someone else: its key
    // is held by the first login's account. This is the answer the sign-in
    // check reads as "a different NEAR AI account".
    h.sign_in_to_near_ai_as(&fresh_subject());
    let started = start(&h.state, PROVISION_START, None, &enrolled).await;
    let signature = enrolled.sign(&started.signing_bytes);
    answers.push((
        "device key held by another login's account",
        finish(
            &h.state,
            PROVISION_FINISH,
            None,
            &started,
            &enrolled,
            &signature,
        )
        .await,
    ));

    for (cause, answer) in &answers {
        assert_uniform_deny(answer, cause);
    }
    assert_eq!(answers.len(), 6);
    reset_account_rate_limiter_for_db_test().await;
    drop_tenants(&admin, &[&enrolled_tenant]).await;
}

/// An account registry on the test server, with or without the login
/// resolver pool, or pointed at a database that does not exist.
async fn registry(database: Option<&str>, with_resolver: bool) -> Arc<PgBackend> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .expect("the harness found a database");
    let url = match database {
        Some(name) => {
            let mut parsed = reqwest::Url::parse(&url).expect("url");
            parsed.set_path(&format!("/{name}"));
            parsed.to_string()
        }
        None => url,
    };
    Arc::new(
        PgBackend::new(&DatabaseConfig {
            url: SecretString::from(url),
            pool_size: 2,
            ssl_mode: trace_commons_server::config::SslMode::Prefer,
            login_resolver_url: if with_resolver {
                trace_commons_server::config::DatabaseConfig::login_resolver_url_from_env()
            } else {
                None
            },
            gate_driver_url: None,
            pii_backstop_driver_url: None,
            invite_registry_url: None,
        })
        .await
        .expect("the pool is built lazily"),
    )
}

/// A commons that offers the ceremony but is not ready to run it -- no account
/// registry, no identity material -- answers `503` at start and at finish,
/// not the uniform deny a signed-out device reads as "another account".
/// Readiness is the same for every request, so this says nothing about who
/// the request names.
#[tokio::test]
async fn pg_a_near_ai_commons_that_is_not_ready_answers_503() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let device = Device::new();
    let (_, challenge) = pkce();
    let start_body = serde_json::json!({
        "device_public_key": device.public_b64(),
        "code_challenge": challenge,
        "code_challenge_method": "S256",
    });
    let unready: [(&str, fn(&mut AppState)); 2] = [
        ("no account registry", |s| s.db_mirror = None),
        ("no identity material", |s| s.near_account_identity = None),
    ];
    for (cause, make_unready) in unready {
        let mut state = h.state.clone();
        make_unready(Arc::make_mut(&mut state));
        let started = post(&state, PROVISION_START, &start_body).await;
        assert_unavailable(&started, &format!("start, {cause}"));
        let finished = post(
            &state,
            PROVISION_FINISH,
            &well_formed_unknown_finish(&device),
        )
        .await;
        assert_unavailable(&finished, &format!("finish, {cause}"));
    }
    // A path the operator switched off is not "unavailable" at start: it will
    // not start working on retry, so the start answers the uniform deny, like
    // a route that is not there. A finish only follows a start that answered,
    // so a finish meeting the same state is the deployment in flux (a rollout,
    // a replica that disagrees) and answers `503`, never the deny the #1840
    // check reads as "another account".
    let mut off = h.state.clone();
    Arc::make_mut(&mut off).near_provisioning_enabled = false;
    assert_uniform_deny(
        &post(&off, PROVISION_START, &start_body).await,
        "start, off",
    );
    assert_unavailable(
        &post(&off, PROVISION_FINISH, &well_formed_unknown_finish(&device)).await,
        "finish, off",
    );
    // The v1 routes need an enrollment witness this harness does not publish:
    // the same rule, on the route a production client calls.
    assert_uniform_deny(
        &post(&h.state, "/v1/account/near-ai/provision/start", &start_body).await,
        "v1 start, no witness",
    );
    assert_unavailable(
        &post(
            &h.state,
            "/v1/account/near-ai/provision/finish",
            &well_formed_unknown_finish(&device),
        )
        .await,
        "v1 finish, no witness",
    );
    reset_account_rate_limiter_for_db_test().await;
}

/// A misconfigured account registry -- a database that does not exist --
/// answers `503` at start and at finish: PostgreSQL refuses every
/// connection the same way, whoever the request names.
#[tokio::test]
async fn pg_a_near_ai_sign_in_whose_database_is_misconfigured_answers_503() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let missing = registry(
        Some(&format!("no_such_db_{}", Uuid::new_v4().simple())),
        true,
    )
    .await;
    let mut state = h.state.clone();
    Arc::make_mut(&mut state).db_mirror = Some(missing as Arc<dyn Database>);
    let device = Device::new();
    let (_, challenge) = pkce();
    let started = post(
        &state,
        PROVISION_START,
        &serde_json::json!({
            "device_public_key": device.public_b64(),
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        }),
    )
    .await;
    assert_unavailable(&started, "start, database does not exist");
    let finished = post(
        &state,
        PROVISION_FINISH,
        &well_formed_unknown_finish(&device),
    )
    .await;
    assert_unavailable(&finished, "finish, database does not exist");
    reset_account_rate_limiter_for_db_test().await;
}

/// The one deployment failure that surfaces inside provisioning rather than
/// before it: no login-resolver pool. Every provisioning resolves the anchor
/// through it first, so it fails the same way for every login; the finish
/// answers `503` and registers nothing.
#[tokio::test]
async fn pg_a_near_ai_finish_without_the_resolver_pool_answers_503() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let mut state = h.state.clone();
    Arc::make_mut(&mut state).db_mirror = Some(registry(None, false).await as Arc<dyn Database>);
    let device = Device::new();
    let started = start(&state, PROVISION_START, None, &device).await;
    let signature = device.sign(&started.signing_bytes);
    let finished = finish(
        &state,
        PROVISION_FINISH,
        None,
        &started,
        &device,
        &signature,
    )
    .await;
    assert_unavailable(&finished, "finish, no resolver pool");
    assert_eq!(
        count(
            &admin,
            "SELECT count(*)::bigint FROM device_keys WHERE public_key = $1",
            &[&device.public_b64()],
        )
        .await,
        0,
        "an unanswered finish registered the device"
    );
    // The ceremony was consumed: once the registry is whole, the same finish
    // is still refused; the client starts again.
    let resent = finish(
        &h.state,
        PROVISION_FINISH,
        None,
        &started,
        &device,
        &signature,
    )
    .await;
    assert_uniform_deny(&resent, "resent after the 503");
    // The 503 came after introspection, at provisioning's first step (the
    // resolver pool), so the attempt wrote nothing; starting again signs in.
    signs_in_after_starting_again(&h, &admin, &device, "no resolver pool").await;
    reset_account_rate_limiter_for_db_test().await;
}

/// What the client does after a `503` at finish: a new start and finish for
/// the same device, against a commons that answers again. It must succeed and
/// leave exactly one device row, whatever the failed finish did first.
async fn signs_in_after_starting_again(
    h: &Harness,
    admin: &deadpool_postgres::Object,
    device: &Device,
    cause: &str,
) {
    let again = start(&h.state, PROVISION_START, None, device).await;
    let signature = device.sign(&again.signing_bytes);
    let answered = finish(&h.state, PROVISION_FINISH, None, &again, device, &signature).await;
    assert_eq!(
        answered.status,
        StatusCode::OK,
        "{cause}, started again: {:?}",
        answered.json()
    );
    assert_eq!(
        count(
            admin,
            "SELECT count(*)::bigint FROM device_keys WHERE public_key = $1",
            &[&device.public_b64()],
        )
        .await,
        1,
        "{cause}: one device row after starting again"
    );
    let tenant = answered.json()["tenant_id"]
        .as_str()
        .expect("tenant")
        .to_string();
    drop_tenants(admin, &[&tenant]).await;
}

/// A stored ceremony the finishing process cannot read -- a start on one
/// build and a finish on another during a rollout, or a row of another
/// ceremony kind -- is deployment state, not a refusal: `503`, never the
/// uniform deny the #1840 check reads as "another account". The take still
/// consumed the row, so the client starts again.
#[tokio::test]
async fn pg_an_unreadable_stored_ceremony_answers_503() {
    let Some(h) = harness().await else {
        return;
    };
    reset_account_rate_limiter_for_db_test().await;
    let admin = h.admin().await;
    let device = Device::new();
    let body = well_formed_unknown_finish(&device);
    let ceremony_id = body["ceremony_id"].as_str().expect("id").to_string();
    // A payload no ceremony type reads, as a build that stored another shape
    // would leave it.
    admin
        .execute(
            "INSERT INTO trace_near_provisioning_ceremonies(ceremony_hash, payload, expires_at)
             VALUES ($1, $2, now() + interval '5 minutes')",
            &[
                &hash_secret(&ceremony_id),
                &serde_json::json!({"shape": "from another build"}),
            ],
        )
        .await
        .expect("seed an unreadable ceremony");
    assert_unavailable(
        &post(&h.state, PROVISION_FINISH, &body).await,
        "unreadable ceremony",
    );
    // Consumed by the take: the same finish is now an unknown ceremony.
    assert_uniform_deny(
        &post(&h.state, PROVISION_FINISH, &body).await,
        "resent after the unreadable ceremony",
    );
    reset_account_rate_limiter_for_db_test().await;
}
