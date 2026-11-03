// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Legacy account payout migration
//! (docs/superpowers/specs/2026-11-03-legacy-account-payout-migration-design.md).
//!
//! Every test runs the real library entry points as LOGIN roles holding only
//! the runtime roles the deployed services hold, never this suite's owner.
//! Skipped without `TRACE_COMMONS_PG_TEST_DATABASE_URL` on 127.0.0.1. Run
//! with `--test-threads=1`.

#[path = "support/role_ddl_lock.rs"]
mod role_ddl_lock;

use base64::Engine as _;
use ring::signature::{Ed25519KeyPair, KeyPair};
use role_ddl_lock::with_role_ddl_lock;
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use trace_commons_protocol::legacy_invite_link::{
    LegacyInviteLinkRequest, LegacyInviteLinkStatement, legacy_invite_link_statement_bytes,
};
use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;
use trace_commons_server::account_trust::{device_claim_actor, device_upload_principal_refs};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, PayoutStore, postgres::PgBackend};
use trace_commons_server::legacy_invite_link::{
    LegacyInviteLinkSigner, issue_challenge, link_legacy_invite,
};
use trace_commons_server::trace_upload_claim_issuer::generate_upload_claim_keypair;
use uuid::Uuid;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    }
}

fn hash(label: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(label.as_bytes())))
}

fn anchored(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple().to_string().repeat(2))
}

struct Device {
    key: Ed25519KeyPair,
    id: String,
}

fn new_device() -> Device {
    let rng = ring::rand::SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let key = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
    let id = device_key_id_from_public_key_bytes(key.public_key().as_ref());
    Device { key, id }
}

struct Fixture {
    owner: Arc<PgBackend>,
    admin: deadpool_postgres::Object,
    /// A login holding only `trace_account_invite_runtime` (the link path).
    link_runtime: Arc<PgBackend>,
    signer: LegacyInviteLinkSigner,
}

async fn fixture() -> Option<Fixture> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_PG_TEST_DATABASE_URL required");
        return None;
    };
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    let owner = Arc::new(PgBackend::new(&config(url.clone())).await.unwrap());
    owner.run_migrations().await.unwrap();
    let admin = owner
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin
        .batch_execute(&with_role_ddl_lock(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tc_legacy_carry_link')
             THEN CREATE ROLE tc_legacy_carry_link LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;
             GRANT trace_account_invite_runtime TO tc_legacy_carry_link;",
        ))
        .await
        .unwrap();
    let mut link_url = parsed.clone();
    link_url.set_username("tc_legacy_carry_link").unwrap();
    let link_runtime = Arc::new(PgBackend::new(&config(link_url.into())).await.unwrap());
    let keys = generate_upload_claim_keypair().unwrap();
    let signer =
        LegacyInviteLinkSigner::from_pem(&keys.private_key_pem, &keys.public_key_pem, "carry-1")
            .unwrap();
    Some(Fixture {
        owner,
        admin,
        link_runtime,
        signer,
    })
}

impl Fixture {
    async fn exec(&self, sql: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) {
        self.admin
            .execute(sql, params)
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
    }

    /// A NEAR AI account with a recorded `near_ai_login` anchor.
    async fn near_ai_account(&self) -> (String, Uuid) {
        let tenant = anchored("nearai");
        let account = Uuid::new_v4();
        self.exec(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await;
        self.exec(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await;
        self.exec(
            "INSERT INTO trace_near_account_anchors
                (tenant_id, account_id, anchor_hash, sealed_account_name,
                 index_pepper_ref, account_name_key_ref, identity_source, auth_provider)
             VALUES ($1, $2, $3, $4, 'fixture-pepper', 'fixture-key', 'near_ai_login', 'github')",
            &[
                &tenant,
                &account,
                &hash(&format!("anchor:{account}")),
                &serde_json::json!({"test_fixture": true}),
            ],
        )
        .await;
        (tenant, account)
    }

    /// A legacy invite tenant as `/v1/onboard` leaves it: the invite row and
    /// one device registered under it. No account: the device never minted
    /// a login link, so its credit is keyed to its bare upload principal.
    async fn legacy_invite_tenant(&self, device: &Device) -> (String, String) {
        let tenant = format!("tenant-carry-{}", Uuid::new_v4().simple());
        let invite = hash(&format!("invite:{tenant}"));
        self.exec(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await;
        self.exec(
            "INSERT INTO onboarding_invites (tenant_id, invite_subject_hash, max_uses, consumed_uses)
             VALUES ($1, $2, 3, 1)",
            &[&tenant, &invite],
        )
        .await;
        self.exec(
            "INSERT INTO device_keys (device_key_id, tenant_id, public_key, invite_subject_hash)
             VALUES ($1, $2, $3, $4)",
            &[
                &device.id,
                &tenant,
                &B64.encode(device.key.public_key().as_ref()),
                &invite,
            ],
        )
        .await;
        (tenant, invite)
    }

    /// The V81 tenant link, through the library entry point, as the link
    /// runtime: a fresh challenge and a statement signed by `device`.
    async fn link(
        &self,
        account_tenant: &str,
        account: Uuid,
        legacy_tenant: &str,
        invite: &str,
        device: &Device,
    ) {
        let db = self.link_runtime.as_ref();
        let challenge = issue_challenge(db, account_tenant, account)
            .await
            .expect("challenge issues");
        let statement = LegacyInviteLinkStatement {
            legacy_tenant_id: legacy_tenant.into(),
            device_key_id: device.id.clone(),
            invite_subject_hash: invite.into(),
            account_tenant_id: account_tenant.into(),
            account_id: account,
            nonce: challenge.nonce.clone(),
            issued_at: challenge.issued_at,
        };
        let signature = device
            .key
            .sign(&legacy_invite_link_statement_bytes(&statement));
        let request = LegacyInviteLinkRequest {
            legacy_tenant_id: legacy_tenant.into(),
            device_key_id: device.id.clone(),
            device_public_key: B64.encode(device.key.public_key().as_ref()),
            invite_subject_hash: invite.into(),
            nonce: challenge.nonce,
            issued_at: challenge.issued_at,
            signature: B64.encode(signature.as_ref()),
        };
        link_legacy_invite(db, &self.signer, account_tenant, account, &request)
            .await
            .expect("tenant links");
    }
}

/// Owner decision 5, option (A), 2026-11-03. Linking the tenant claims the
/// signing device's past upload credit: both forms of the signed-upload
/// principal of its bare device-key actor resolve to one account in the
/// legacy tenant, for the settlement key and for the accrual claim. A
/// per-user-subject actor cannot be derived from the key and stays unowned
/// (the spec's known limit).
#[tokio::test]
async fn linking_a_tenant_claims_the_signing_devices_upload_credit() {
    let Some(fx) = fixture().await else { return };
    let device = new_device();
    let (legacy, invite) = fx.legacy_invite_tenant(&device).await;
    let (near, account) = fx.near_ai_account().await;

    let bare = device_upload_principal_refs(&legacy, &device.id, &device.id)
        .expect("bare actor parses")
        .to_vec();
    let subject_actor = device_claim_actor(&legacy, &device.id, Some("user-1"));
    let subject = device_upload_principal_refs(&legacy, &device.id, &subject_actor)
        .expect("subject actor parses")
        .to_vec();

    fx.link(&near, account, &legacy, &invite, &device).await;

    let settled = fx
        .owner
        .resolve_principals_to_accounts(&legacy, &bare)
        .await
        .unwrap();
    let claimable = fx
        .owner
        .payout_principal_accounts(&legacy, &bare)
        .await
        .unwrap();
    let owner_of = |principal: &String| settled.get(principal).copied();
    let claimed = owner_of(&bare[0]).expect(
        "the signed-upload principal of the device's bare actor resolves to a legacy account",
    );
    assert_eq!(owner_of(&bare[1]), Some(claimed), "both forms, one account");
    for principal in &bare {
        assert_eq!(claimable.get(principal), Some(&claimed));
    }
    // The account is in the legacy tenant, never the NEAR AI one.
    let in_legacy: bool = fx
        .admin
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM trace_accounts WHERE tenant_id = $1 AND account_id = $2)",
            &[&legacy, &claimed],
        )
        .await
        .unwrap()
        .get(0);
    assert!(in_legacy);
    // Known limit: per-user-subject credit stays unowned.
    assert!(
        fx.owner
            .resolve_principals_to_accounts(&legacy, &subject)
            .await
            .unwrap()
            .is_empty()
    );
}
