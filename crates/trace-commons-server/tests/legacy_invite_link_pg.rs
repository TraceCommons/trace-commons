// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Linking a legacy `tenant-…` invite identity to a NEAR account (V81), and
//! account-admission readiness under coexistence.
//!
//! One sequential test against one fresh database, because the readiness
//! function is fleet-wide: every assertion about it depends on everything
//! seeded before it. The link path runs through the real library entry point
//! (signature verification, countersigning) and the real SECURITY DEFINER
//! function, as a login that holds only `trace_account_invite_runtime`.
//!
//! Skipped without `TRACE_COMMONS_LEGACY_INVITE_LINK_PG_TEST_DATABASE_URL`,
//! which must name a database beginning `admission_test_legacy_invite_link`
//! on the literal host 127.0.0.1.

use base64::Engine as _;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use secrecy::SecretString;
use std::sync::Arc;
use trace_commons_protocol::legacy_invite_link::{
    LegacyInviteLinkRecord, LegacyInviteLinkRecordKind, LegacyInviteLinkRequest,
    LegacyInviteLinkStatement, legacy_invite_link_record_bytes, legacy_invite_link_statement_bytes,
};
use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::legacy_invite_link::{
    LegacyInviteLinkSigner, LinkRefusal, issue_challenge, link_legacy_invite,
};
use trace_commons_server::trace_upload_claim_issuer::generate_upload_claim_keypair;
use uuid::Uuid;

const ENV: &str = "TRACE_COMMONS_LEGACY_INVITE_LINK_PG_TEST_DATABASE_URL";
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

fn hash(label: &str) -> String {
    use sha2::{Digest, Sha256};
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

async fn exec(
    admin: &deadpool_postgres::Object,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) {
    admin
        .execute(sql, params)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn seed_tenant(admin: &deadpool_postgres::Object, tenant: &str) {
    exec(
        admin,
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[&tenant],
    )
    .await;
}

async fn seed_near_account(admin: &deadpool_postgres::Object, tenant: &str, anchor: bool) -> Uuid {
    seed_tenant(admin, tenant).await;
    let account = Uuid::new_v4();
    exec(
        admin,
        "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
        &[&tenant, &account],
    )
    .await;
    if anchor {
        let anchor_hash = hash(&format!("anchor:{account}"));
        exec(
            admin,
            "INSERT INTO trace_near_account_anchors
                (tenant_id, account_id, anchor_hash, sealed_account_name,
                 index_pepper_ref, account_name_key_ref)
             VALUES ($1, $2, $3, $4, 'fixture-pepper', 'fixture-key')",
            &[
                &tenant,
                &account,
                &anchor_hash,
                &serde_json::json!({"test_fixture": true}),
            ],
        )
        .await;
    }
    account
}

/// A legacy invite tenant exactly as `/v1/onboard` leaves it: the V29
/// per-tenant invite row, one device registered under it, and the account the
/// device's first authenticated call mints.
async fn seed_legacy_tenant(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    invite_hash: &str,
    max_uses: i32,
    device: &Device,
) {
    seed_tenant(admin, tenant).await;
    exec(
        admin,
        "INSERT INTO onboarding_invites (tenant_id, invite_subject_hash, max_uses, consumed_uses)
         VALUES ($1, $2, $3, 1)
         ON CONFLICT (tenant_id, invite_subject_hash) DO UPDATE
            SET consumed_uses = onboarding_invites.consumed_uses + 1",
        &[&tenant, &invite_hash, &max_uses],
    )
    .await;
    seed_device(admin, tenant, invite_hash, device).await;
    exec(
        admin,
        "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
        &[&tenant, &Uuid::new_v4()],
    )
    .await;
}

async fn seed_device(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    invite_hash: &str,
    device: &Device,
) {
    let public_key = B64.encode(device.key.public_key().as_ref());
    exec(
        admin,
        "INSERT INTO device_keys (device_key_id, tenant_id, public_key, invite_subject_hash)
         VALUES ($1, $2, $3, $4)",
        &[&device.id, &tenant, &public_key, &invite_hash],
    )
    .await;
}

async fn ready(admin: &deadpool_postgres::Object) -> bool {
    admin
        .query_one("SELECT trace_account_admission_linkage_ready()", &[])
        .await
        .unwrap()
        .get(0)
}

struct Linker {
    db: Arc<PgBackend>,
    signer: LegacyInviteLinkSigner,
}

impl Linker {
    /// Sign a statement for `account` with `device` over a fresh challenge.
    async fn request(
        &self,
        account_tenant: &str,
        account: Uuid,
        legacy_tenant: &str,
        invite_hash: &str,
        device: &Device,
    ) -> LegacyInviteLinkRequest {
        let challenge = issue_challenge(self.db.as_ref(), account_tenant, account)
            .await
            .expect("challenge issues");
        let statement = LegacyInviteLinkStatement {
            legacy_tenant_id: legacy_tenant.into(),
            device_key_id: device.id.clone(),
            invite_subject_hash: invite_hash.into(),
            account_tenant_id: account_tenant.into(),
            account_id: account,
            nonce: challenge.nonce.clone(),
            issued_at: challenge.issued_at,
        };
        let signature = device
            .key
            .sign(&legacy_invite_link_statement_bytes(&statement));
        LegacyInviteLinkRequest {
            legacy_tenant_id: legacy_tenant.into(),
            device_key_id: device.id.clone(),
            device_public_key: B64.encode(device.key.public_key().as_ref()),
            invite_subject_hash: invite_hash.into(),
            nonce: challenge.nonce,
            issued_at: challenge.issued_at,
            signature: B64.encode(signature.as_ref()),
        }
    }

    async fn link(
        &self,
        account_tenant: &str,
        account: Uuid,
        legacy_tenant: &str,
        invite_hash: &str,
        device: &Device,
    ) -> Result<trace_commons_protocol::legacy_invite_link::LegacyInviteLinkResponse, LinkRefusal>
    {
        let request = self
            .request(account_tenant, account, legacy_tenant, invite_hash, device)
            .await;
        link_legacy_invite(
            self.db.as_ref(),
            &self.signer,
            account_tenant,
            account,
            &request,
        )
        .await
    }
}

async fn count(
    admin: &deadpool_postgres::Object,
    sql: &str,
    params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
) -> i64 {
    admin.query_one(sql, params).await.unwrap().get(0)
}

#[tokio::test]
async fn legacy_invite_link_and_coexistence_readiness() {
    let Ok(url) = std::env::var(ENV) else {
        eprintln!("SKIPPED: {ENV} not set");
        return;
    };
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(
        parsed
            .path()
            .starts_with("/admission_test_legacy_invite_link")
    );
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin
        .batch_execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tc_legacy_link_runtime')
             THEN CREATE ROLE tc_legacy_link_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;
             GRANT trace_account_invite_runtime TO tc_legacy_link_runtime;",
        )
        .await
        .unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url.set_username("tc_legacy_link_runtime").unwrap();
    let runtime = Arc::new(PgBackend::new(&config(runtime_url.into())).await.unwrap());
    let keys = generate_upload_claim_keypair().unwrap();
    let signer = LegacyInviteLinkSigner::from_pem(
        &keys.private_key_pem,
        &keys.public_key_pem,
        "ingest-test-1",
    )
    .expect("signer builds");
    let linker = Linker {
        db: runtime.clone(),
        signer,
    };

    // --- Fleet: individual and pooled legacy invite tenants, NEAR accounts.
    let individual = format!("tenant-individual-{}", Uuid::new_v4().simple());
    let individual_invite = hash(&format!("invite:{individual}"));
    let individual_device = new_device();
    seed_legacy_tenant(
        &admin,
        &individual,
        &individual_invite,
        3,
        &individual_device,
    )
    .await;
    let second_device = new_device();
    seed_device(&admin, &individual, &individual_invite, &second_device).await;

    let pooled = format!("tenant-pooled-{}", Uuid::new_v4().simple());
    let pooled_invite = hash(&format!("invite:{pooled}"));
    let pooled_device = new_device();
    seed_legacy_tenant(&admin, &pooled, &pooled_invite, 2000, &pooled_device).await;
    let pooled_other = new_device();
    seed_device(&admin, &pooled, &pooled_invite, &pooled_other).await;
    exec(
        &admin,
        "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
         VALUES ($1, 'shared event invite')",
        &[&pooled],
    )
    .await;

    let near_a = anchored("near");
    let account_a = seed_near_account(&admin, &near_a, true).await;
    let near_b = anchored("nearai");
    let account_b = seed_near_account(&admin, &near_b, true).await;
    let unanchored = anchored("near");
    let account_unanchored = seed_near_account(&admin, &unanchored, false).await;

    // Decision C: invite-onboarded legacy devices and their tenants' accounts,
    // linked or not, and pooled tenants coexist. V77 refused this fleet.
    assert!(
        ready(&admin).await,
        "legacy invite tenants coexist; they must not block account admission"
    );

    // --- The link itself.
    let linked = linker
        .link(
            &near_a,
            account_a,
            &individual,
            &individual_invite,
            &individual_device,
        )
        .await
        .expect("a live invite device links its tenant");
    assert_eq!(linked.authority, "invited");
    assert_eq!(linked.trust_version, 1);
    assert_eq!(linked.record.statement.legacy_tenant_id, individual);
    assert_eq!(linked.record.statement.account_tenant_id, near_a);
    assert_eq!(linked.record.statement.account_id, account_a);
    assert_eq!(linked.record.server_kid, "ingest-test-1");
    assert_eq!(linked.record.kind, LegacyInviteLinkRecordKind::Link);
    // The client can check both signatures from the returned record alone.
    UnparsedPublicKey::new(&ED25519, individual_device.key.public_key().as_ref())
        .verify(
            &legacy_invite_link_statement_bytes(&linked.record.statement),
            &B64.decode(&linked.record.device_signature).unwrap(),
        )
        .expect("device signature verifies");
    UnparsedPublicKey::new(&ED25519, linker.signer.public_key_bytes())
        .verify(
            &legacy_invite_link_record_bytes(&linked.record),
            &B64.decode(&linked.server_signature).unwrap(),
        )
        .expect("server countersignature verifies");
    // A grant from the tenant's invite, and no use spent anywhere.
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_account_invite_grants
              WHERE tenant_id=$1 AND account_id=$2 AND invite_subject_hash=$3 AND revoked_at IS NULL",
            &[&near_a, &account_a, &individual_invite],
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &admin,
            "SELECT consumed_uses::BIGINT FROM onboarding_invites WHERE tenant_id=$1 AND invite_subject_hash=$2",
            &[&individual, &individual_invite],
        )
        .await,
        1,
        "linking spends no invite use"
    );
    let authority: String = admin
        .query_one(
            "SELECT authority FROM trace_account_trust WHERE tenant_id=$1 AND account_id=$2",
            &[&near_a, &account_a],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(authority, "invited");
    // Hash-only audit: neither tenant id appears in the row.
    let audit: serde_json::Value = admin
        .query_one(
            "SELECT safe_metadata FROM trace_account_audit
              WHERE tenant_id=$1 AND action='legacy_invite_linked'",
            &[&near_a],
        )
        .await
        .unwrap()
        .get(0);
    let audit_text = audit.to_string();
    assert!(!audit_text.contains(&individual), "{audit_text}");
    assert_eq!(audit["legacy_tenant_hash"], hash(&individual));

    // Idempotent for the same account from the same device: the original
    // record comes back, byte for byte.
    let again = linker
        .link(
            &near_a,
            account_a,
            &individual,
            &individual_invite,
            &individual_device,
        )
        .await
        .expect("re-link by the same account and device");
    assert_eq!(again.record, linked.record);
    assert_eq!(again.server_signature, linked.server_signature);
    assert_eq!(again.trust_version, 1);

    // (V91) Another device of the same tenant, signed into the SAME account,
    // gets an attestation of its own under the existing link: its statement,
    // its signature, countersigned. It can verify it against its own key,
    // which the original record -- signed by the first device -- never let
    // it do.
    let attested = linker
        .link(
            &near_a,
            account_a,
            &individual,
            &individual_invite,
            &second_device,
        )
        .await
        .expect("a second device of the linked tenant attests");
    assert_eq!(attested.record.statement.device_key_id, second_device.id);
    assert_eq!(attested.record.statement.legacy_tenant_id, individual);
    assert_eq!(attested.record.statement.account_tenant_id, near_a);
    assert_eq!(attested.record.statement.account_id, account_a);
    assert_ne!(attested.record.link_id, linked.record.link_id);
    assert_eq!(
        attested.record.kind,
        LegacyInviteLinkRecordKind::DeviceAttestation,
        "an attestation says it is one"
    );
    assert_eq!(
        attested.trust_version, 1,
        "no new trust: the link carried it"
    );
    UnparsedPublicKey::new(&ED25519, second_device.key.public_key().as_ref())
        .verify(
            &legacy_invite_link_statement_bytes(&attested.record.statement),
            &B64.decode(&attested.record.device_signature).unwrap(),
        )
        .expect("the attestation carries the second device's own signature");
    UnparsedPublicKey::new(&ED25519, linker.signer.public_key_bytes())
        .verify(
            &legacy_invite_link_record_bytes(&attested.record),
            &B64.decode(&attested.server_signature).unwrap(),
        )
        .expect("countersigned");
    // ...under the attestation domain only: presented as a link, it fails.
    UnparsedPublicKey::new(&ED25519, linker.signer.public_key_bytes())
        .verify(
            &legacy_invite_link_record_bytes(&LegacyInviteLinkRecord {
                kind: LegacyInviteLinkRecordKind::Link,
                ..attested.record.clone()
            }),
            &B64.decode(&attested.server_signature).unwrap(),
        )
        .expect_err("an attestation is not countersigned as a link");
    // Idempotent: the same device again gets its first attestation back.
    let attested_again = linker
        .link(
            &near_a,
            account_a,
            &individual,
            &individual_invite,
            &second_device,
        )
        .await
        .expect("a repeated attestation");
    assert_eq!(attested_again.record, attested.record);
    assert_eq!(attested_again.server_signature, attested.server_signature);
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_legacy_invite_link_devices WHERE device_key_id=$1",
            &[&second_device.id],
        )
        .await,
        1
    );
    let attest_audit: serde_json::Value = admin
        .query_one(
            "SELECT safe_metadata FROM trace_account_audit
              WHERE tenant_id=$1 AND action='legacy_invite_device_attested'",
            &[&near_a],
        )
        .await
        .expect("one attestation audit row")
        .get(0);
    assert!(!attest_audit.to_string().contains(&individual));
    assert_eq!(attest_audit["legacy_tenant_hash"], hash(&individual));
    // A device outside the tenant's invite devices attests nothing.
    let stranger = new_device();
    assert_eq!(
        linker
            .link(
                &near_a,
                account_a,
                &individual,
                &individual_invite,
                &stranger
            )
            .await
            .expect_err("an unregistered device"),
        LinkRefusal::DeviceNotEligible
    );
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_account_audit
              WHERE tenant_id=$1 AND action='legacy_invite_linked'",
            &[&near_a]
        )
        .await,
        1
    );
    // Legacy devices keep working: nothing about them changed.
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM device_keys WHERE tenant_id=$1 AND revoked_at IS NULL",
            &[&individual],
        )
        .await,
        2
    );
    assert!(ready(&admin).await, "a linked legacy tenant still coexists");

    // --- A second account claiming the same tenant: refused and flagged.
    let claimed = linker
        .link(
            &near_b,
            account_b,
            &individual,
            &individual_invite,
            &second_device,
        )
        .await
        .expect_err("a tenant links to one account");
    assert_eq!(claimed, LinkRefusal::TenantClaimed);
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_account_invite_grants WHERE tenant_id=$1",
            &[&near_b],
        )
        .await,
        0,
        "the refused claimant gains nothing"
    );
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_legacy_invite_link_conflicts
              WHERE legacy_tenant_id=$1 AND tenant_id=$2 AND account_id=$3 AND resolved_at IS NULL",
            &[&individual, &near_b, &account_b],
        )
        .await,
        1
    );
    assert!(
        !ready(&admin).await,
        "a non-pooled tenant claimed by two accounts is ambiguous and blocks admission"
    );
    exec(
        &admin,
        "UPDATE trace_legacy_invite_link_conflicts SET resolved_at = now() WHERE legacy_tenant_id=$1",
        &[&individual],
    )
    .await;
    assert!(ready(&admin).await, "a resolved conflict no longer blocks");

    // --- Pooled tenants are never linkable, from any of their devices.
    for device in [&pooled_device, &pooled_other] {
        assert_eq!(
            linker
                .link(&near_b, account_b, &pooled, &pooled_invite, device)
                .await
                .expect_err("pooled tenants are not linkable"),
            LinkRefusal::TenantPooled
        );
    }
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_legacy_invite_links WHERE legacy_tenant_id=$1",
            &[&pooled]
        )
        .await,
        0
    );

    // --- Revocation (decision F).
    let revoked_invite_tenant = format!("tenant-revoked-{}", Uuid::new_v4().simple());
    let revoked_invite = hash(&format!("invite:{revoked_invite_tenant}"));
    let revoked_invite_device = new_device();
    seed_legacy_tenant(
        &admin,
        &revoked_invite_tenant,
        &revoked_invite,
        3,
        &revoked_invite_device,
    )
    .await;
    exec(
        &admin,
        "UPDATE onboarding_invites SET revoked_at = now() WHERE tenant_id=$1",
        &[&revoked_invite_tenant],
    )
    .await;
    assert_eq!(
        linker
            .link(
                &near_b,
                account_b,
                &revoked_invite_tenant,
                &revoked_invite,
                &revoked_invite_device
            )
            .await
            .expect_err("a revoked invite does not carry over"),
        LinkRefusal::InviteRevoked
    );

    let registry_revoked_tenant = format!("tenant-regrevoked-{}", Uuid::new_v4().simple());
    let registry_revoked_invite = hash(&format!("invite:{registry_revoked_tenant}"));
    let registry_revoked_device = new_device();
    seed_legacy_tenant(
        &admin,
        &registry_revoked_tenant,
        &registry_revoked_invite,
        3,
        &registry_revoked_device,
    )
    .await;
    exec(
        &admin,
        "INSERT INTO onboarding_invite_grants
            (invite_subject_hash, policy_label, tenant_mode, fixed_tenant_id,
             policy_version, max_uses, consumed_uses, issuance_source, revoked_at)
         VALUES ($1, 'pilot', 'fixed', $2, 'v1', 3, 1, 'test', now())",
        &[&registry_revoked_invite, &registry_revoked_tenant],
    )
    .await;
    assert_eq!(
        linker
            .link(
                &near_b,
                account_b,
                &registry_revoked_tenant,
                &registry_revoked_invite,
                &registry_revoked_device
            )
            .await
            .expect_err("a registry-revoked invite does not carry over"),
        LinkRefusal::InviteRevoked
    );

    // Expired and exhausted, but redeemed: carries over.
    let expired_tenant = format!("tenant-expired-{}", Uuid::new_v4().simple());
    let expired_invite = hash(&format!("invite:{expired_tenant}"));
    let expired_device = new_device();
    seed_legacy_tenant(&admin, &expired_tenant, &expired_invite, 1, &expired_device).await;
    exec(
        &admin,
        "INSERT INTO onboarding_invite_grants
            (invite_subject_hash, policy_label, tenant_mode, fixed_tenant_id,
             policy_version, max_uses, consumed_uses, issuance_source, expires_at)
         VALUES ($1, 'pilot', 'fixed', $2, 'v1', 1, 1, 'test', now() - interval '1 day')",
        &[&expired_invite, &expired_tenant],
    )
    .await;
    let carried = linker
        .link(
            &near_b,
            account_b,
            &expired_tenant,
            &expired_invite,
            &expired_device,
        )
        .await
        .expect("an expired, already-redeemed invite carries over");
    assert_eq!(carried.authority, "invited");
    assert_eq!(
        count(
            &admin,
            "SELECT consumed_uses::BIGINT FROM onboarding_invite_grants WHERE invite_subject_hash=$1",
            &[&expired_invite],
        )
        .await,
        1,
        "the exhausted registry invite is not spent again"
    );

    // --- Device eligibility: revoked, wrong key, wrong invite, wrong tenant.
    let revoked_device_tenant = format!("tenant-revdev-{}", Uuid::new_v4().simple());
    let revoked_device_invite = hash(&format!("invite:{revoked_device_tenant}"));
    let revoked_device = new_device();
    seed_legacy_tenant(
        &admin,
        &revoked_device_tenant,
        &revoked_device_invite,
        3,
        &revoked_device,
    )
    .await;
    exec(
        &admin,
        "UPDATE device_keys SET revoked_at = now() WHERE device_key_id=$1",
        &[&revoked_device.id],
    )
    .await;
    assert_eq!(
        linker
            .link(
                &near_b,
                account_b,
                &revoked_device_tenant,
                &revoked_device_invite,
                &revoked_device
            )
            .await
            .expect_err("a revoked device cannot link"),
        LinkRefusal::DeviceNotEligible
    );
    let stranger = new_device();
    assert_eq!(
        linker
            .link(
                &near_b,
                account_b,
                &revoked_device_tenant,
                &revoked_device_invite,
                &stranger
            )
            .await
            .expect_err("an unregistered key cannot link"),
        LinkRefusal::DeviceNotEligible
    );
    // A pooled device cannot claim an individual tenant by naming it.
    assert_eq!(
        linker
            .link(
                &near_b,
                account_b,
                &revoked_device_tenant,
                &pooled_invite,
                &pooled_device
            )
            .await
            .expect_err("another tenant's device cannot link"),
        LinkRefusal::DeviceNotEligible
    );

    // An instance enrollment is `invite`-origin in device_keys but redeemed no
    // invite; it is not an invite identity.
    let instance_tenant = format!("tenant-{}", "e".repeat(64));
    let instance_hash = hash("instance-subject");
    let instance_device = new_device();
    seed_tenant(&admin, &instance_tenant).await;
    seed_device(&admin, &instance_tenant, &instance_hash, &instance_device).await;
    assert_eq!(
        linker
            .link(
                &near_b,
                account_b,
                &instance_tenant,
                &instance_hash,
                &instance_device
            )
            .await
            .expect_err("instance enrollments are not invite identities"),
        LinkRefusal::DeviceNotEligible
    );

    // --- Account eligibility and the challenge.
    let fresh_tenant = format!("tenant-fresh-{}", Uuid::new_v4().simple());
    let fresh_invite = hash(&format!("invite:{fresh_tenant}"));
    let fresh_device = new_device();
    seed_legacy_tenant(&admin, &fresh_tenant, &fresh_invite, 3, &fresh_device).await;
    assert_eq!(
        linker
            .link(
                &unanchored,
                account_unanchored,
                &fresh_tenant,
                &fresh_invite,
                &fresh_device
            )
            .await
            .expect_err("an account without a NEAR anchor cannot link"),
        LinkRefusal::AccountIneligible
    );
    // A signature over another account's statement does not verify.
    let mut forged = linker
        .request(
            &near_a,
            account_a,
            &fresh_tenant,
            &fresh_invite,
            &fresh_device,
        )
        .await;
    assert_eq!(
        link_legacy_invite(
            runtime.as_ref(),
            &linker.signer,
            &near_b,
            account_b,
            &forged
        )
        .await
        .expect_err("statement is bound to the session account"),
        LinkRefusal::SignatureInvalid
    );
    // A challenge is single-use, and belongs to the account it was issued to.
    let request = linker
        .request(
            &near_b,
            account_b,
            &fresh_tenant,
            &fresh_invite,
            &fresh_device,
        )
        .await;
    let first = link_legacy_invite(
        runtime.as_ref(),
        &linker.signer,
        &near_b,
        account_b,
        &request,
    )
    .await;
    assert!(first.is_ok(), "{first:?}");
    assert_eq!(
        link_legacy_invite(
            runtime.as_ref(),
            &linker.signer,
            &near_b,
            account_b,
            &request
        )
        .await
        .expect_err("a spent challenge is refused"),
        LinkRefusal::ChallengeInvalid
    );
    forged.signature = B64.encode([0u8; 64]);
    assert_eq!(
        link_legacy_invite(
            runtime.as_ref(),
            &linker.signer,
            &near_a,
            account_a,
            &forged
        )
        .await
        .expect_err("a bad signature is refused before the database"),
        LinkRefusal::SignatureInvalid
    );
    let expired_challenge = linker
        .request(
            &near_a,
            account_a,
            &fresh_tenant,
            &fresh_invite,
            &fresh_device,
        )
        .await;
    exec(
        &admin,
        "UPDATE trace_legacy_invite_link_challenges SET expires_at = issued_at + 1
          WHERE tenant_id=$1 AND consumed_at IS NULL",
        &[&near_a],
    )
    .await;
    // Expiry is compared in whole seconds; step past it.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert_eq!(
        link_legacy_invite(
            runtime.as_ref(),
            &linker.signer,
            &near_a,
            account_a,
            &expired_challenge
        )
        .await
        .expect_err("an expired challenge is refused"),
        LinkRefusal::ChallengeInvalid
    );

    // --- The runtime login reads nothing across tenants.
    let runtime_client = runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    for denied in [
        "SELECT count(*) FROM trace_legacy_invite_links",
        "SELECT count(*) FROM trace_legacy_invite_pooled_tenants",
        "SELECT count(*) FROM trace_legacy_invite_link_conflicts",
        "SELECT trace_account_admission_linkage_ready()",
    ] {
        assert!(
            runtime_client.query_one(denied, &[]).await.is_err(),
            "the runtime login must not run `{denied}`"
        );
    }
    let visible_devices: i64 = runtime_client
        .query_one("SELECT count(*) FROM device_keys", &[])
        .await
        .map(|row| row.get(0))
        .unwrap_or(0);
    assert_eq!(visible_devices, 0, "no tenant context, no devices");
    let guard: (bool, bool, bool) = {
        let row = admin
            .query_one(
                "SELECT rolcanlogin, rolbypassrls,
                        pg_has_role('tc_legacy_link_runtime', 'trace_legacy_invite_link_guard', 'MEMBER')
                   FROM pg_roles WHERE rolname = 'trace_legacy_invite_link_guard'",
                &[],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1), row.get(2))
    };
    assert_eq!(guard, (false, false, false));

    // --- The remaining readiness rules.
    // An open legacy account with no invite behind it blocks...
    let orphan = format!("tenant-orphan-{}", Uuid::new_v4().simple());
    seed_tenant(&admin, &orphan).await;
    exec(
        &admin,
        "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
        &[&orphan, &Uuid::new_v4()],
    )
    .await;
    assert!(
        !ready(&admin).await,
        "an account with no invite behind it is not coexistence"
    );
    // ...unless the operator has classified the tenant as pooled.
    exec(
        &admin,
        "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label) VALUES ($1, 'shared')",
        &[&orphan],
    )
    .await;
    assert!(ready(&admin).await);
    // An active NEAR-namespace device without live account linkage still blocks.
    let loose = new_device();
    let loose_tenant = anchored("near");
    seed_tenant(&admin, &loose_tenant).await;
    exec(
        &admin,
        "INSERT INTO device_keys (device_key_id, tenant_id, public_key, invite_subject_hash, onboarding_origin)
         VALUES ($1, $2, $3, NULL, 'near')",
        &[&loose.id, &loose_tenant, &B64.encode(loose.key.public_key().as_ref())],
    )
    .await;
    assert!(
        !ready(&admin).await,
        "NEAR devices still need live account linkage"
    );
}

/// One statement's parameters, as ingest passes them to both functions.
struct Call<'a> {
    account_tenant: &'a str,
    account: Uuid,
    legacy_tenant: &'a str,
    invite_hash: &'a str,
    device: &'a Device,
    nonce: String,
    issued_at: i64,
}

impl<'a> Call<'a> {
    /// A statement over a fresh challenge issued to the account.
    async fn new(
        runtime: &PgBackend,
        account_tenant: &'a str,
        account: Uuid,
        legacy_tenant: &'a str,
        invite_hash: &'a str,
        device: &'a Device,
    ) -> Self {
        let challenge = issue_challenge(runtime, account_tenant, account)
            .await
            .expect("challenge issues");
        Self {
            account_tenant,
            account,
            legacy_tenant,
            invite_hash,
            device,
            nonce: challenge.nonce,
            issued_at: challenge.issued_at,
        }
    }

    /// The same statement, naming another device.
    fn by(&self, device: &'a Device) -> Self {
        Self {
            device,
            nonce: self.nonce.clone(),
            ..*self
        }
    }
}

#[derive(Clone, Copy)]
enum Step {
    Link,
    Attest,
}

/// Calls `trace_link_legacy_invite` and `trace_attest_legacy_invite_device`
/// directly as the runtime login, in order, in ONE transaction in
/// `account_tenant`'s context -- no signature checks and no ingest in
/// between. `pause` sleeps before the last call. Commits, as ingest does on
/// a refusal, and returns each call's outcome.
async fn in_one_tx(
    runtime: &PgBackend,
    account_tenant: &str,
    steps: &[(Step, &Call<'_>)],
    pause: Option<std::time::Duration>,
) -> Vec<String> {
    let mut client = runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&account_tenant],
    )
    .await
    .unwrap();
    let mut outcomes = Vec::new();
    for (index, (step, call)) in steps.iter().enumerate() {
        if index + 1 == steps.len() {
            if let Some(pause) = pause {
                tokio::time::sleep(pause).await;
            }
        }
        let function = match step {
            Step::Link => "trace_link_legacy_invite",
            Step::Attest => "trace_attest_legacy_invite_device",
        };
        let outcome: String = tx
            .query_one(
                &format!(
                    "SELECT outcome FROM {function}($1,$2,$3,$4,$5,$6,$7,$8,$9,1,
                                                    'c2ln','ingest-test-1','c2ln')"
                ),
                &[
                    &call.account_tenant,
                    &call.account,
                    &call.legacy_tenant,
                    &call.device.id,
                    &call.device.key.public_key().as_ref().to_vec(),
                    &call.invite_hash,
                    &call.nonce,
                    &call.issued_at,
                    &Uuid::new_v4(),
                ],
            )
            .await
            .unwrap_or_else(|e| panic!("{function}: {e}"))
            .get(0);
        outcomes.push(outcome);
    }
    tx.commit().await.unwrap();
    outcomes
}

/// The link function for `link`, then the attestation function for `attest`,
/// in one transaction.
async fn direct(
    runtime: &PgBackend,
    link: &Call<'_>,
    attest: &Call<'_>,
    pause: Option<std::time::Duration>,
) -> Vec<String> {
    in_one_tx(
        runtime,
        attest.account_tenant,
        &[(Step::Link, link), (Step::Attest, attest)],
        pause,
    )
    .await
}

/// V92: `trace_attest_legacy_invite_device` holds every refusal on its own,
/// called directly as the runtime login, rather than relying on the link
/// function having run first. Each case is set up so that the link function
/// alone would let the attestation through, or would lead it to a different
/// label: deleting the matching V92 guard turns that case red.
///
/// Runs alongside `legacy_invite_link_and_coexistence_readiness` against the
/// same database, so it creates nothing that affects readiness (no conflict
/// rows, no orphan accounts) and counts only its own tenants' rows.
#[tokio::test]
async fn the_attestation_function_holds_every_refusal_on_its_own() {
    let Ok(url) = std::env::var(ENV) else {
        eprintln!("SKIPPED: {ENV} not set");
        return;
    };
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(
        parsed
            .path()
            .starts_with("/admission_test_legacy_invite_link")
    );
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin
        .batch_execute(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tc_legacy_attest_runtime')
             THEN CREATE ROLE tc_legacy_attest_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;
             GRANT trace_account_invite_runtime TO tc_legacy_attest_runtime;",
        )
        .await
        .unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url
        .set_username("tc_legacy_attest_runtime")
        .unwrap();
    let runtime = Arc::new(PgBackend::new(&config(runtime_url.into())).await.unwrap());
    let keys = generate_upload_claim_keypair().unwrap();
    let linker = Linker {
        db: runtime.clone(),
        signer: LegacyInviteLinkSigner::from_pem(
            &keys.private_key_pem,
            &keys.public_key_pem,
            "ingest-test-1",
        )
        .unwrap(),
    };
    let rt = runtime.as_ref();
    let outcomes = |link: &str, attest: &str| vec![link.to_string(), attest.to_string()];

    let near_a = anchored("near");
    let account_a = seed_near_account(&admin, &near_a, true).await;
    let near_b = anchored("nearai");
    let account_b = seed_near_account(&admin, &near_b, true).await;

    // A tenant linked to A by its first device under invite X. `devices`
    // joined under X too; `other_invite_device` joined the same tenant under
    // invite Y; `unredeemed` is registered under an invite Z the tenant has
    // no `onboarding_invites` row for.
    let tenant = format!("tenant-v92-{}", Uuid::new_v4().simple());
    let invite_x = hash(&format!("invite-x:{tenant}"));
    let invite_y = hash(&format!("invite-y:{tenant}"));
    let invite_z = hash(&format!("invite-z:{tenant}"));
    let first = new_device();
    seed_legacy_tenant(&admin, &tenant, &invite_x, 3, &first).await;
    let devices: Vec<Device> = (0..5).map(|_| new_device()).collect();
    for device in &devices {
        seed_device(&admin, &tenant, &invite_x, device).await;
    }
    let other_invite_device = new_device();
    exec(
        &admin,
        "INSERT INTO onboarding_invites (tenant_id, invite_subject_hash, max_uses, consumed_uses)
         VALUES ($1, $2, 3, 1)",
        &[&tenant, &invite_y],
    )
    .await;
    seed_device(&admin, &tenant, &invite_y, &other_invite_device).await;
    let unredeemed = new_device();
    seed_device(&admin, &tenant, &invite_z, &unredeemed).await;
    let stranger = new_device();
    linker
        .link(&near_a, account_a, &tenant, &invite_x, &first)
        .await
        .expect("the first device links the tenant to A");

    // The honest path, directly: the link function answers already_linked
    // and spends the challenge in this transaction; the attestation function
    // attests and records the spend against this device.
    let honest = Call::new(rt, &near_a, account_a, &tenant, &invite_x, &devices[0]).await;
    assert_eq!(
        direct(rt, &honest, &honest, None).await,
        outcomes("already_linked", "attested")
    );
    let claimed_by: Option<String> = admin
        .query_one(
            "SELECT attested_device_key_id FROM trace_legacy_invite_link_challenges
              WHERE tenant_id=$1 AND nonce_hash=$2",
            &[&near_a, &hash(&honest.nonce)],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(claimed_by.as_deref(), Some(devices[0].id.as_str()));

    // --- challenge_invalid.
    // Never spent.
    let unspent = Call::new(rt, &near_a, account_a, &tenant, &invite_x, &devices[1]).await;
    assert_eq!(
        in_one_tx(rt, &near_a, &[(Step::Attest, &unspent)], None).await,
        ["challenge_invalid"]
    );
    // Spent in an EARLIER transaction, then replayed straight into the
    // attestation function: the review's long-spent nonce.
    let replayed = Call::new(rt, &near_a, account_a, &tenant, &invite_x, &devices[1]).await;
    assert_eq!(
        in_one_tx(rt, &near_a, &[(Step::Link, &replayed)], None).await,
        ["already_linked"]
    );
    assert_eq!(
        in_one_tx(rt, &near_a, &[(Step::Attest, &replayed)], None).await,
        ["challenge_invalid"],
        "a challenge spent in another transaction backs no attestation"
    );
    // One spend backs one attestation: a second device cannot ride on it.
    let shared = Call::new(rt, &near_a, account_a, &tenant, &invite_x, &devices[2]).await;
    assert_eq!(
        in_one_tx(
            rt,
            &near_a,
            &[
                (Step::Link, &shared),
                (Step::Attest, &shared),
                (Step::Attest, &shared.by(&devices[3])),
            ],
            None,
        )
        .await,
        ["already_linked", "attested", "challenge_invalid"]
    );
    // Expired between the spend and the attestation.
    let expiring = Call::new(rt, &near_a, account_a, &tenant, &invite_x, &devices[3]).await;
    exec(
        &admin,
        "UPDATE trace_legacy_invite_link_challenges SET expires_at = issued_at + 2
          WHERE tenant_id=$1 AND nonce_hash=$2",
        &[&near_a, &hash(&expiring.nonce)],
    )
    .await;
    assert_eq!(
        direct(
            rt,
            &expiring,
            &expiring,
            Some(std::time::Duration::from_millis(3600))
        )
        .await,
        outcomes("already_linked", "challenge_invalid"),
        "a challenge that expired before the attestation backs none"
    );

    // --- device_not_eligible. The link function spends the challenge and
    // refuses; the attestation function must refuse on its own.
    let stranger_call = Call::new(rt, &near_a, account_a, &tenant, &invite_x, &stranger).await;
    assert_eq!(
        direct(rt, &stranger_call, &stranger_call, None).await,
        outcomes("device_not_eligible", "device_not_eligible")
    );
    let unredeemed_call = Call::new(rt, &near_a, account_a, &tenant, &invite_z, &unredeemed).await;
    assert_eq!(
        direct(rt, &unredeemed_call, &unredeemed_call, None).await,
        outcomes("device_not_eligible", "device_not_eligible"),
        "a device under an invite its tenant never redeemed"
    );

    // --- tenant_claimed. B's challenge is spent by a refused link call (an
    // unregistered device, so no conflict row is written); an attestation for
    // a real device of A's tenant must still refuse B.
    let as_b = Call::new(rt, &near_b, account_b, &tenant, &invite_x, &stranger).await;
    assert_eq!(
        direct(rt, &as_b, &as_b.by(&devices[0]), None).await,
        outcomes("device_not_eligible", "tenant_claimed"),
        "another account never gets an attestation, not even A's device's"
    );

    // --- invite_not_linked (the Medium): this account's link, another
    // invite of the same tenant.
    let other = Call::new(
        rt,
        &near_a,
        account_a,
        &tenant,
        &invite_y,
        &other_invite_device,
    )
    .await;
    assert_eq!(
        direct(rt, &other, &other, None).await,
        outcomes("already_linked", "invite_not_linked")
    );
    // Through ingest's entry point: its own refusal, not tenant_claimed.
    let request = linker
        .request(&near_a, account_a, &tenant, &invite_y, &other_invite_device)
        .await;
    assert_eq!(
        link_legacy_invite(rt, &linker.signer, &near_a, account_a, &request)
            .await
            .expect_err("another invite of the linked tenant is refused"),
        LinkRefusal::InviteNotLinked
    );
    // The challenge stays spent, and the refusal is recorded for the
    // operator, hash-only, naming both invites by hash.
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_legacy_invite_link_challenges
              WHERE tenant_id=$1 AND nonce_hash=$2 AND consumed_at IS NOT NULL",
            &[&near_a, &hash(&request.nonce)],
        )
        .await,
        1
    );
    let refused = admin
        .query(
            "SELECT outcome, safe_metadata FROM trace_account_audit
              WHERE tenant_id=$1 AND action='legacy_invite_device_attest_refused'",
            &[&near_a],
        )
        .await
        .unwrap();
    assert_eq!(refused.len(), 2, "one row per refusal");
    for row in &refused {
        assert_eq!(row.get::<_, String>(0), "invite_not_linked");
        let metadata: serde_json::Value = row.get(1);
        assert!(!metadata.to_string().contains(&tenant), "{metadata}");
        assert_eq!(metadata["legacy_tenant_hash"], hash(&tenant));
        assert_eq!(metadata["device_key_id"], other_invite_device.id);
        assert_eq!(metadata["invite_subject_hash"], invite_y);
        assert_eq!(metadata["linked_invite_subject_hash"], invite_x);
    }
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_legacy_invite_link_devices WHERE device_key_id=$1",
            &[&other_invite_device.id],
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_legacy_invite_link_conflicts WHERE legacy_tenant_id=$1",
            &[&tenant],
        )
        .await,
        0,
        "one account under two invites is not a two-account conflict"
    );

    // --- A row V91 recorded under the link domain is superseded in place by
    // the device's next attestation, which then verifies as an attestation.
    let v91_device = &devices[4];
    exec(
        &admin,
        "INSERT INTO trace_legacy_invite_link_devices
            (attestation_id, link_id, tenant_id, account_id, device_key_id, invite_subject_hash,
             nonce, issued_at, device_signature, attested_at, server_kid, server_signature,
             countersign_domain)
         SELECT $1, l.link_id, l.tenant_id, l.account_id, $2, l.invite_subject_hash,
                repeat('f', 64), 1, 'c2ln', 1, 'ingest-test-1', 'c2ln',
                'trace-commons.legacy-invite-link-record.v1'
           FROM trace_legacy_invite_links l
          WHERE l.legacy_tenant_id = $3 AND l.revoked_at IS NULL",
        &[&Uuid::new_v4(), &v91_device.id, &tenant],
    )
    .await;
    let superseding = linker
        .link(&near_a, account_a, &tenant, &invite_x, v91_device)
        .await
        .expect("a device with a V91 row attests again");
    UnparsedPublicKey::new(&ED25519, linker.signer.public_key_bytes())
        .verify(
            &legacy_invite_link_record_bytes(&superseding.record),
            &B64.decode(&superseding.server_signature).unwrap(),
        )
        .expect("the superseding attestation verifies as one");
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_legacy_invite_link_devices
              WHERE device_key_id=$1
                AND countersign_domain='trace-commons.legacy-invite-device-attestation-record.v1'",
            &[&v91_device.id],
        )
        .await,
        1,
        "superseded in place: still one row for the device"
    );

    // --- tenant_pooled and invite_revoked, each on a tenant of its own that
    // A linked before the operator acted.
    // Each of the three revocations the link function honours is checked.
    for (case, expected) in [
        ("pooled", "tenant_pooled"),
        ("invite_revoked", "invite_revoked"),
        ("registry_revoked", "invite_revoked"),
        ("grant_revoked", "invite_revoked"),
    ] {
        let t = format!("tenant-v92-{case}-{}", Uuid::new_v4().simple());
        let invite = hash(&format!("invite:{t}"));
        let d1 = new_device();
        let d2 = new_device();
        seed_legacy_tenant(&admin, &t, &invite, 3, &d1).await;
        seed_device(&admin, &t, &invite, &d2).await;
        linker
            .link(&near_a, account_a, &t, &invite, &d1)
            .await
            .expect("links");
        match case {
            "pooled" => {
                exec(
                    &admin,
                    "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
                     VALUES ($1, 'shared')",
                    &[&t],
                )
                .await
            }
            "invite_revoked" => {
                exec(
                    &admin,
                    "UPDATE onboarding_invites SET revoked_at = now() WHERE tenant_id=$1",
                    &[&t],
                )
                .await
            }
            "registry_revoked" => {
                exec(
                    &admin,
                    "INSERT INTO onboarding_invite_grants
                        (invite_subject_hash, policy_label, tenant_mode, fixed_tenant_id,
                         policy_version, max_uses, consumed_uses, issuance_source, revoked_at)
                     VALUES ($1, 'pilot', 'fixed', $2, 'v1', 3, 1, 'test', now())",
                    &[&invite, &t],
                )
                .await
            }
            _ => {
                exec(
                    &admin,
                    "UPDATE trace_account_invite_grants SET revoked_at = now()
                      WHERE tenant_id=$1 AND account_id=$2 AND invite_subject_hash=$3",
                    &[&near_a, &account_a, &invite],
                )
                .await
            }
        }
        let call = Call::new(rt, &near_a, account_a, &t, &invite, &d2).await;
        assert_eq!(
            direct(rt, &call, &call, None).await,
            outcomes(expected, expected),
            "{case}"
        );
    }

    // --- tenant_claimed with no live link: an operator revoked it after the
    // link function read it.
    let t = format!("tenant-v92-unlinked-{}", Uuid::new_v4().simple());
    let invite = hash(&format!("invite:{t}"));
    let d1 = new_device();
    let d2 = new_device();
    seed_legacy_tenant(&admin, &t, &invite, 3, &d1).await;
    seed_device(&admin, &t, &invite, &d2).await;
    linker
        .link(&near_a, account_a, &t, &invite, &d1)
        .await
        .expect("links");
    exec(
        &admin,
        "UPDATE trace_legacy_invite_links SET revoked_at = now() WHERE legacy_tenant_id=$1",
        &[&t],
    )
    .await;
    let spend = Call::new(rt, &near_a, account_a, &t, &invite, &stranger).await;
    assert_eq!(
        direct(rt, &spend, &spend.by(&d2), None).await,
        outcomes("device_not_eligible", "tenant_claimed")
    );

    // --- account_ineligible, by shape: a legacy tenant id in the account
    // namespace.
    let shaped = Call {
        legacy_tenant: &near_b,
        ..Call::new(rt, &near_a, account_a, &tenant, &invite_x, &devices[1]).await
    };
    assert_eq!(
        in_one_tx(rt, &near_a, &[(Step::Attest, &shaped)], None).await,
        ["account_ineligible"]
    );
    // A closed account attests nothing, even under its own live link. Last,
    // because it closes A.
    exec(
        &admin,
        "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id=$1 AND account_id=$2",
        &[&near_a, &account_a],
    )
    .await;
    let closed = Call::new(rt, &near_a, account_a, &tenant, &invite_x, &devices[1]).await;
    assert_eq!(
        direct(rt, &closed, &closed, None).await,
        outcomes("account_ineligible", "account_ineligible")
    );

    // Only the honest call, the one-spend case's first device and the
    // superseded row attested anything under this tenant's link.
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM trace_legacy_invite_link_devices d
               JOIN trace_legacy_invite_links l ON l.link_id = d.link_id
              WHERE l.legacy_tenant_id = $1",
            &[&tenant],
        )
        .await,
        3
    );
}

/// Decision D: a shared, multi-use invite on the file-allowlist path (the
/// shape of the Devfolio event codes) still onboards a new device exactly as
/// before, including after its tenant is marked pooled. Nothing in V81 reads
/// or gates the device onboarding path.
#[tokio::test]
async fn shared_file_invite_still_onboards_new_devices_after_pooling() {
    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request, StatusCode};
    use tower::ServiceExt;
    use trace_commons_server::trace_upload_claim_allowlist::{
        AllowlistSourceSpec, hash_invite_code,
    };
    use trace_commons_server::trace_upload_claim_issuer::{
        TraceUploadClaimIssuerConfig, trace_upload_claim_issuer_router,
    };

    let Ok(url) = std::env::var(ENV) else {
        eprintln!("SKIPPED: {ENV} not set");
        return;
    };
    let parsed = reqwest::Url::parse(&url).expect("test database URL");
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(
        parsed
            .path()
            .starts_with("/admission_test_legacy_invite_link")
    );
    let backend = Arc::new(PgBackend::new(&config(url)).await.unwrap());
    backend.run_migrations().await.unwrap();
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();

    // One code, one fixed tenant, a large max_uses: every redeemer pools.
    let suffix: String = Uuid::new_v4()
        .simple()
        .to_string()
        .to_uppercase()
        .chars()
        .take(8)
        .collect();
    let code = format!("DEVFOLIO{suffix}");
    assert_eq!(code.len(), 16);
    let tenant = format!("tenant-event-{}", Uuid::new_v4().simple());
    let dir = tempfile::tempdir().unwrap();
    let allowlist = dir.path().join("allowlist.json");
    std::fs::write(
        &allowlist,
        serde_json::json!({
            "version": 1,
            "generated_at": "2026-08-17T00:00:00Z",
            "policy_label": "pilot",
            "entries": [{
                "kind": "invite",
                "subject_hash": hash_invite_code(&code),
                "tenant_id": tenant,
                "max_uses": 2000,
                "note_label": "Event participant",
            }],
        })
        .to_string(),
    )
    .unwrap();
    let issuer_keys = generate_upload_claim_keypair().unwrap();
    let workload_keys = generate_upload_claim_keypair().unwrap();
    let db: Arc<dyn Database> = backend.clone();
    let issuer_config = || TraceUploadClaimIssuerConfig {
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
    };
    let onboard = |device: &Device| {
        let router = trace_upload_claim_issuer_router(issuer_config()).expect("router");
        let body = serde_json::json!({
            "schema_version": trace_commons_protocol::onboarding::TRACE_ONBOARD_REQUEST_SCHEMA_VERSION,
            "invite_code": code,
            "device_public_key": B64.encode(device.key.public_key().as_ref()),
            "client_info": {"agent": "ironclaw", "version": "0.x.y"},
        });
        async move {
            let response = router
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/v1/onboard")
                        .header("content-type", "application/json")
                        .body(Body::from(body.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 1 << 20).await.unwrap();
            (
                status,
                serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            )
        }
    };

    let first_device = new_device();
    let (status, before) = onboard(&first_device).await;
    assert_eq!(status, StatusCode::OK, "{before}");
    assert_eq!(before["tenant_id"], tenant);
    assert_eq!(before["device_key_id"], first_device.id);

    exec(
        &admin,
        "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
         VALUES ($1, 'shared event invite')",
        &[&tenant],
    )
    .await;

    for _ in 0..2 {
        let device = new_device();
        let (status, after) = onboard(&device).await;
        assert_eq!(status, StatusCode::OK, "{after}");
        assert_eq!(after["tenant_id"], tenant, "every redeemer pools");
        assert_eq!(after["device_key_id"], device.id);
        let mut expected = before.clone();
        expected["device_key_id"] = after["device_key_id"].clone();
        assert_eq!(after, expected, "onboarding is unchanged by the marker");
    }
    assert_eq!(
        count(
            &admin,
            "SELECT consumed_uses::BIGINT FROM onboarding_invites
              WHERE tenant_id=$1 AND invite_subject_hash=$2",
            &[&tenant, &hash_invite_code(&code)],
        )
        .await,
        3,
        "max_uses counts devices on the file path, as before"
    );
    assert_eq!(
        count(
            &admin,
            "SELECT count(*) FROM device_keys
              WHERE tenant_id=$1 AND onboarding_origin='invite' AND revoked_at IS NULL",
            &[&tenant],
        )
        .await,
        3
    );
}
