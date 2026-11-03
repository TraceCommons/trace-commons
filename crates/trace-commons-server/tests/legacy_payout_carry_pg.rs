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
use trace_commons_gate_api::{RailId, RecipientKind};
use trace_commons_protocol::legacy_invite_link::LegacyInviteLinkResponse;
use trace_commons_protocol::legacy_invite_link::{
    LegacyInviteLinkRequest, LegacyInviteLinkStatement, legacy_invite_link_statement_bytes,
};
use trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes;
use trace_commons_server::account_trust::{device_claim_actor, device_upload_principal_refs};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{
    CarryPrograms, Database, LegacyCarryReport, PayoutAuditAction, PayoutLinkAction,
    PayoutLinkConfirm, PayoutLinkConfirmWrite, PayoutLinkRequestInsert, PayoutLinkRequestWrite,
    PayoutLinkState, PayoutLinkTransition, PayoutStore, SealedRecipient, postgres::PgBackend,
};
use trace_commons_server::legacy_invite_link::{
    LegacyInviteLinkSigner, LinkRefusal, issue_challenge, link_legacy_invite,
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
    /// A login holding `trace_ingest_runtime` and the pilot's pre-V62
    /// table grants the payout path uses (the confirm and the carry).
    payout_runtime: Arc<PgBackend>,
    /// An operator login with only what revoking a tenant link needs.
    operator: deadpool_postgres::Object,
    signer: LegacyInviteLinkSigner,
}

const RAIL: &str = "near_ai";
const ACTOR: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

fn rail() -> RailId {
    RailId::new(RAIL).unwrap()
}

fn programs() -> CarryPrograms {
    CarryPrograms(Arc::new(|link_id: Uuid| {
        format!("tc1-{}", &hash(&link_id.to_string())[7..39])
    }))
}

fn recipient(seed: &str) -> SealedRecipient {
    SealedRecipient {
        sealed: serde_json::json!({"v": 1, "ciphertext": hash(&format!("ct:{seed}"))}),
        recipient_index: hash(&format!("index:{seed}")),
        recipient_key_ref: "key-test-1".into(),
        index_pepper_ref: "pepper-test-1".into(),
        subject_index: hash(&format!("subject:{seed}")),
        recipient_ref: Some(hash(&format!("ref:{seed}"))),
    }
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
    admin
        .batch_execute(&with_role_ddl_lock(
            "DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tc_legacy_carry_payout')
             THEN CREATE ROLE tc_legacy_carry_payout LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;
             GRANT trace_ingest_runtime TO tc_legacy_carry_payout;
             GRANT SELECT ON trace_accounts, trace_near_account_anchors, trace_credit_holds, trace_account_principals TO trace_ingest_runtime;
             GRANT INSERT ON trace_accounts, trace_account_principals TO trace_ingest_runtime;
             GRANT SELECT, INSERT, UPDATE ON trace_tenants TO trace_ingest_runtime;
             GRANT INSERT ON trace_account_audit TO trace_ingest_runtime;
             GRANT USAGE ON SEQUENCE trace_account_audit_audit_sequence_seq TO trace_ingest_runtime;
             DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tc_legacy_carry_operator')
             THEN CREATE ROLE tc_legacy_carry_operator LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;
             GRANT USAGE ON SCHEMA public TO tc_legacy_carry_operator;
             GRANT SELECT, UPDATE (revoked_at) ON trace_legacy_invite_links TO tc_legacy_carry_operator;",
        ))
        .await
        .unwrap();
    let mut payout_url = parsed.clone();
    payout_url.set_username("tc_legacy_carry_payout").unwrap();
    let payout_runtime = Arc::new(PgBackend::new(&config(payout_url.into())).await.unwrap());
    let mut operator_url = parsed.clone();
    operator_url
        .set_username("tc_legacy_carry_operator")
        .unwrap();
    let operator = PgBackend::new(&config(operator_url.into()))
        .await
        .unwrap()
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let keys = generate_upload_claim_keypair().unwrap();
    let signer =
        LegacyInviteLinkSigner::from_pem(&keys.private_key_pem, &keys.public_key_pem, "carry-1")
            .unwrap();
    Some(Fixture {
        owner,
        admin,
        link_runtime,
        payout_runtime,
        operator,
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
    ) -> Result<LegacyInviteLinkResponse, LinkRefusal> {
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
        link_legacy_invite(db, &self.signer, account_tenant, account, &request).await
    }
}

/// Owner decision 5, option (A), 2026-11-03. Linking the tenant claims the
/// signing device's past upload credit: both forms of the signed-upload
/// principal of each actor derivable from its key resolve to one account in
/// the legacy tenant, for the settlement key and for the accrual claim. The
/// app's uploads carry `subject = user_subject_hash(cfg.user_subject)`, and
/// an invite device's `user_subject` is its key id (review of #1865), so that
/// actor is the one that matters; the bare actor covers device-JWT uploads.
/// A subject that cannot be derived (a contributor label) stays unowned, the
/// spec's known limit.
#[tokio::test]
async fn linking_a_tenant_claims_the_signing_devices_upload_credit() {
    let Some(fx) = fixture().await else { return };
    let device = new_device();
    let (legacy, invite) = fx.legacy_invite_tenant(&device).await;
    let (near, account) = fx.near_ai_account().await;

    let app_actor = device_claim_actor(
        &legacy,
        &device.id,
        Some(&trace_commons_protocol::onboarding::user_subject_hash(
            &device.id,
        )),
    );
    let mut bare = device_upload_principal_refs(&legacy, &device.id, &device.id)
        .expect("bare actor parses")
        .to_vec();
    bare.extend(
        device_upload_principal_refs(&legacy, &device.id, &app_actor).expect("app actor parses"),
    );
    let labelled = trace_commons_protocol::onboarding::user_subject_hash("a-contributor-label");
    let subject_actor = device_claim_actor(&legacy, &device.id, Some(&labelled));
    let subject = device_upload_principal_refs(&legacy, &device.id, &subject_actor)
        .expect("subject actor parses")
        .to_vec();

    fx.link(&near, account, &legacy, &invite, &device)
        .await
        .expect("tenant links");

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
    for principal in &bare {
        assert_eq!(
            owner_of(principal),
            Some(claimed),
            "every form, one account"
        );
    }
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
    // Known limit: a subject not derivable from the key stays unowned.
    assert!(
        fx.owner
            .resolve_principals_to_accounts(&legacy, &subject)
            .await
            .unwrap()
            .is_empty()
    );
}

impl Fixture {
    /// An instance-enrolled legacy tenant: an `invite`-origin device with a
    /// stored subject hash but no `onboarding_invites` row.
    async fn legacy_instance_tenant(&self, device: &Device) -> (String, String) {
        let tenant = format!("tenant-instance-{}", Uuid::new_v4().simple());
        let subject = hash(&format!("instance:{tenant}"));
        self.exec(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await;
        self.exec(
            "INSERT INTO device_keys (device_key_id, tenant_id, public_key, invite_subject_hash)
             VALUES ($1, $2, $3, $4)",
            &[
                &device.id,
                &tenant,
                &B64.encode(device.key.public_key().as_ref()),
                &subject,
            ],
        )
        .await;
        (tenant, subject)
    }

    /// A NEAR AI account whose legacy tenant is linked, with the linked
    /// account the claim made there.
    async fn linked_pair(&self) -> (String, Uuid, String, Uuid) {
        let device = new_device();
        let (legacy, invite) = self.legacy_invite_tenant(&device).await;
        let (near, account) = self.near_ai_account().await;
        self.link(&near, account, &legacy, &invite, &device)
            .await
            .expect("tenant links");
        let legacy_account = self.accounts(&legacy).await[0];
        (near, account, legacy, legacy_account)
    }

    async fn accounts(&self, tenant: &str) -> Vec<Uuid> {
        self.admin
            .query(
                "SELECT account_id FROM trace_accounts WHERE tenant_id = $1 ORDER BY account_id",
                &[&tenant],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect()
    }

    async fn add_account(&self, tenant: &str, closed: bool) -> Uuid {
        let account = Uuid::new_v4();
        self.exec(
            "INSERT INTO trace_accounts (tenant_id, account_id, closed_at)
             VALUES ($1, $2, CASE WHEN $3 THEN now() END)",
            &[&tenant, &account, &closed],
        )
        .await;
        account
    }

    /// A link or unlink through the real request and confirm, as the payout
    /// runtime, carrying as ingest does.
    async fn confirm(
        &self,
        tenant: &str,
        account: Uuid,
        action: PayoutLinkAction,
        seed: &str,
    ) -> PayoutLinkConfirm {
        let db = self.payout_runtime.as_ref();
        let version = db
            .payout_account_link_view(tenant, account, &rail())
            .await
            .unwrap()
            .version;
        let handle_hash = hash(&format!("handle:{}", Uuid::new_v4()));
        let request = PayoutLinkRequestWrite {
            account_id: account,
            rail_id: rail(),
            idempotency_key: Uuid::new_v4(),
            handle_hash: handle_hash.clone(),
            request_digest: hash(seed),
            action,
            expected_current_version: (version > 0).then_some(version),
            recipient: matches!(action, PayoutLinkAction::Link)
                .then(|| (RecipientKind::new("near_ai_org").unwrap(), recipient(seed))),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
            actor_ref_hash: ACTOR.into(),
            proof_label: matches!(action, PayoutLinkAction::Link).then_some("near_ai_login"),
        };
        assert_eq!(
            db.insert_payout_link_request(tenant, &request)
                .await
                .unwrap(),
            PayoutLinkRequestInsert::Inserted
        );
        let link_id = Uuid::new_v4();
        db.confirm_payout_link_request(
            tenant,
            account,
            &rail(),
            &handle_hash,
            &PayoutLinkConfirmWrite {
                link_id,
                program: (programs().0)(link_id),
                actor_ref_hash: ACTOR.into(),
                carry_programs: Some(programs()),
            },
        )
        .await
        .unwrap()
    }

    /// The pass's first move: the account's waiting link goes active.
    async fn activate(&self, tenant: &str, account: Uuid) {
        let link = self.link_in(tenant, account, "waiting").await.unwrap();
        assert_eq!(
            self.payout_runtime
                .transition_payout_link(
                    tenant,
                    link,
                    PayoutLinkState::Waiting,
                    PayoutLinkState::Active,
                    PayoutAuditAction::LinkActivated,
                    ACTOR,
                    None,
                )
                .await
                .unwrap(),
            PayoutLinkTransition::Moved
        );
    }

    async fn link_in(&self, tenant: &str, account: Uuid, state: &str) -> Option<Uuid> {
        self.admin
            .query_opt(
                "SELECT link_id FROM trace_payout_links
                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3 AND state = $4",
                &[&tenant, &account, &RAIL, &state],
            )
            .await
            .unwrap()
            .map(|row| row.get(0))
    }

    async fn count(&self, sql: &str, params: &[&(dyn tokio_postgres::types::ToSql + Sync)]) -> i64 {
        self.admin.query_one(sql, params).await.unwrap().get(0)
    }

    async fn live_carries(&self, tenant: &str, account: Uuid) -> i64 {
        self.count(
            "SELECT count(*) FROM trace_legacy_payout_carries
              WHERE tenant_id = $1 AND account_id = $2 AND released_at IS NULL",
            &[&tenant, &account],
        )
        .await
    }

    async fn carry(&self, tenant: &str, account: Uuid) -> LegacyCarryReport {
        self.payout_runtime
            .carry_legacy_payout_links(tenant, account, &rail(), &programs(), ACTOR)
            .await
            .unwrap()
    }
}

/// Spec 3.3: a confirmed link is carried to every open account of the
/// linked legacy tenant, with the source's seal and indexes, audited by
/// hash; a repeat carries nothing; a closed account gets nothing.
#[tokio::test]
async fn a_confirmed_link_is_carried_to_every_open_legacy_account() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    let second = fx.add_account(&legacy, false).await;
    let closed = fx.add_account(&legacy, true).await;

    assert!(matches!(
        fx.confirm(&near, account, PayoutLinkAction::Link, "org-1")
            .await,
        PayoutLinkConfirm::Confirmed { .. }
    ));
    for legacy_account in [claimed, second] {
        let carried = fx
            .link_in(&legacy, legacy_account, "waiting")
            .await
            .expect("carried as a waiting link");
        let same: bool = fx
            .admin
            .query_one(
                "SELECT c.sealed_recipient = s.sealed_recipient
                    AND c.recipient_index = s.recipient_index
                    AND c.subject_index = s.subject_index
                    AND c.recipient_ref = s.recipient_ref
                    AND c.program <> s.program
                   FROM trace_payout_links c, trace_payout_links s
                  WHERE c.link_id = $1 AND s.tenant_id = $2 AND s.account_id = $3",
                &[&carried, &near, &account],
            )
            .await
            .unwrap()
            .get(0);
        assert!(same, "same recipient, own program");
        assert_eq!(fx.live_carries(&legacy, legacy_account).await, 1);
    }
    assert_eq!(fx.link_in(&legacy, closed, "waiting").await, None);
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_payout_audit
              WHERE tenant_id = $1 AND action = 'link_created' AND label = 'legacy_tenant_link'",
            &[&legacy],
        )
        .await,
        2
    );
    // Idempotent.
    assert_eq!(fx.carry(&near, account).await, LegacyCarryReport::default());
    // The pass activates a carried link like any other.
    fx.activate(&legacy, claimed).await;
    assert!(fx.link_in(&legacy, claimed, "active").await.is_some());
    // Nothing was written to the legacy tenant that names a NEAR AI login.
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_near_account_anchors WHERE tenant_id = $1",
            &[&legacy],
        )
        .await,
        0
    );
}

/// A recipient change carries the new generation and supersedes the old
/// carry without unlinking it (the pass closes it as a change); an unlink
/// of the source requests the carried link's unlink.
#[tokio::test]
async fn a_change_supersedes_and_an_unlink_releases_the_carry() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-a")
        .await;
    fx.activate(&near, account).await;
    fx.activate(&legacy, claimed).await;
    let first = fx.link_in(&legacy, claimed, "active").await.unwrap();

    assert!(matches!(
        fx.confirm(&near, account, PayoutLinkAction::Link, "org-b")
            .await,
        PayoutLinkConfirm::Confirmed { .. }
    ));
    assert!(fx.link_in(&legacy, claimed, "waiting").await.is_some());
    assert_eq!(fx.live_carries(&legacy, claimed).await, 1);
    let first_unlinked: bool = fx
        .admin
        .query_one(
            "SELECT unlink_requested_at IS NOT NULL FROM trace_payout_links WHERE link_id = $1",
            &[&first],
        )
        .await
        .unwrap()
        .get(0);
    // The superseded carry is unlinked outright (review of #1865), so it
    // stops even if the new carry never activates.
    assert!(first_unlinked, "a superseded active carry is unlinked");

    // A fresh pair for the unlink: the source is active and alone.
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-c")
        .await;
    fx.activate(&near, account).await;
    fx.activate(&legacy, claimed).await;
    let carried = fx.link_in(&legacy, claimed, "active").await.unwrap();
    assert!(matches!(
        fx.confirm(&near, account, PayoutLinkAction::Unlink, "unlink")
            .await,
        PayoutLinkConfirm::Confirmed { .. }
    ));
    let unlinked: bool = fx
        .admin
        .query_one(
            "SELECT unlink_requested_at IS NOT NULL FROM trace_payout_links WHERE link_id = $1",
            &[&carried],
        )
        .await
        .unwrap()
        .get(0);
    assert!(unlinked);
    assert_eq!(fx.live_carries(&legacy, claimed).await, 0);
}

/// Spec 3.3, decision 10: revoking the tenant link, as a non-superuser
/// operator login under the NEAR AI account's tenant context, releases its
/// carries in the same transaction. A waiting carry closes at zero; an
/// active one gets an unlink request.
#[tokio::test]
async fn revoking_the_tenant_link_releases_its_carries() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    let waiting_account = fx.add_account(&legacy, false).await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-r")
        .await;
    fx.activate(&legacy, claimed).await;
    let active = fx.link_in(&legacy, claimed, "active").await.unwrap();
    let waiting = fx
        .link_in(&legacy, waiting_account, "waiting")
        .await
        .unwrap();

    // Fixture tenant ids are `[a-z0-9-]` only, so inlining them is safe.
    fx.operator
        .batch_execute(&format!(
            "BEGIN;
             SELECT set_config('trace_commons.trace_tenant_id', '{near}', true);
             UPDATE trace_legacy_invite_links SET revoked_at = now()
              WHERE legacy_tenant_id = '{legacy}' AND revoked_at IS NULL;
             COMMIT;"
        ))
        .await
        .unwrap();
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_legacy_invite_links
              WHERE legacy_tenant_id = $1 AND revoked_at IS NOT NULL",
            &[&legacy],
        )
        .await,
        1
    );

    let row = fx
        .admin
        .query_one(
            "SELECT (SELECT unlink_requested_at IS NOT NULL FROM trace_payout_links WHERE link_id = $1),
                    (SELECT state || ':' || final_paid::TEXT FROM trace_payout_links WHERE link_id = $2)",
            &[&active, &waiting],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0));
    assert_eq!(row.get::<_, String>(1), "closed:0");
    assert_eq!(fx.live_carries(&legacy, claimed).await, 0);
    assert_eq!(fx.live_carries(&legacy, waiting_account).await, 0);
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_payout_audit
              WHERE tenant_id = $1 AND label = 'legacy_tenant_link_released'",
            &[&legacy],
        )
        .await,
        2
    );
}

/// Spec section 7 and 3.7: a tenant marked pooled after linking, and an
/// account closed after a carry, are released on the next carry; an account
/// with its own link is skipped, never replaced.
#[tokio::test]
async fn pooled_closed_and_own_link_accounts_are_not_carried() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    let own = fx.add_account(&legacy, false).await;
    fx.exec(
        "INSERT INTO trace_payout_links (tenant_id, account_id, rail_id, link_id, recipient_kind,
             state, sealed_recipient, recipient_index, recipient_key_ref, index_pepper_ref,
             subject_index, program)
         VALUES ($1, $2, $3, $4, 'near_ai_org', 'waiting', '{}', $5, 'k', 'p', $5, 'tc1-own')",
        &[&legacy, &own, &RAIL, &Uuid::new_v4(), &hash("own")],
    )
    .await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-p")
        .await;
    assert_eq!(fx.live_carries(&legacy, claimed).await, 1);
    assert_eq!(fx.live_carries(&legacy, own).await, 0);
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_payout_links WHERE tenant_id = $1 AND account_id = $2",
            &[&legacy, &own],
        )
        .await,
        1
    );

    fx.exec(
        "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
        &[&legacy, &claimed],
    )
    .await;
    // Since V148 the close itself released the carry (#1872); the next
    // carry has nothing left to release and still skips the own link.
    assert_eq!(fx.live_carries(&legacy, claimed).await, 0);
    let report = fx.carry(&near, account).await;
    assert_eq!(report.released, 0);
    assert_eq!(report.skipped, 1);
    assert_eq!(fx.live_carries(&legacy, claimed).await, 0);

    // Pooled after the link: every carry goes, and none comes back.
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-q")
        .await;
    assert_eq!(fx.live_carries(&legacy, claimed).await, 1);
    fx.exec(
        "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
         VALUES ($1, 'shared event invite')",
        &[&legacy],
    )
    .await;
    // Released by the mark itself (V148), and never carried again.
    assert_eq!(fx.live_carries(&legacy, claimed).await, 0);
    assert_eq!(fx.carry(&near, account).await, LegacyCarryReport::default());
    assert_eq!(fx.live_carries(&legacy, claimed).await, 0);
}

/// The carry needs the source account's own recorded NEAR AI login. A link
/// on an account without one (written before #1841) carries nothing.
#[tokio::test]
async fn an_account_without_a_login_anchor_carries_nothing() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-w")
        .await;
    assert_eq!(fx.live_carries(&legacy, claimed).await, 1);
    // Remove the anchor; the next carry releases and writes nothing new.
    fx.exec(
        "DELETE FROM trace_near_account_anchors WHERE tenant_id = $1",
        &[&near],
    )
    .await;
    let report = fx.carry(&near, account).await;
    assert_eq!(report.carried, 0);
    assert_eq!(report.released, 1);
}

/// "Payout link first, tenant link second": the post-link carry ingest
/// runs reaches the new tenant's accounts.
#[tokio::test]
async fn a_payout_link_made_before_the_tenant_link_is_carried_after_it() {
    let Some(fx) = fixture().await else { return };
    let device = new_device();
    let (legacy, invite) = fx.legacy_invite_tenant(&device).await;
    let (near, account) = fx.near_ai_account().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-f")
        .await;
    fx.link(&near, account, &legacy, &invite, &device)
        .await
        .expect("tenant links");
    let claimed = fx.accounts(&legacy).await[0];
    assert_eq!(fx.carry(&near, account).await.carried, 1);
    assert_eq!(fx.live_carries(&legacy, claimed).await, 1);
}

/// Decision 3: an instance-enrolled device links with no invite grant and
/// no trust change, its upload credit is claimed, and the other refusals
/// hold: a second account, and a pooled tenant.
#[tokio::test]
async fn an_instance_enrolled_device_links_without_invite_trust() {
    let Some(fx) = fixture().await else { return };
    let device = new_device();
    let (legacy, subject) = fx.legacy_instance_tenant(&device).await;
    let enrolled_subject = hash(&format!("enrolled-user:{legacy}"));
    fx.exec(
        "INSERT INTO trace_instance_enrollments (instance_subject_hash, user_subject_hash, tenant_id)
         VALUES ($1, $2, $3)",
        &[&hash(&format!("instance:{legacy}")), &enrolled_subject, &legacy],
    )
    .await;
    // A device the instance key enrolled into the same tenant later cannot
    // claim it (review of #1865): only the earliest live device links.
    let later = new_device();
    fx.exec(
        "INSERT INTO device_keys (device_key_id, tenant_id, public_key, invite_subject_hash, created_at)
         VALUES ($1, $2, $3, $4, now() + interval '1 minute')",
        &[
            &later.id,
            &legacy,
            &B64.encode(later.key.public_key().as_ref()),
            &subject,
        ],
    )
    .await;
    let (near, account) = fx.near_ai_account().await;
    assert_eq!(
        fx.link(&near, account, &legacy, &subject, &later)
            .await
            .unwrap_err(),
        LinkRefusal::DeviceNotEligible
    );
    let response = fx
        .link(&near, account, &legacy, &subject, &device)
        .await
        .expect("instance device links");
    assert_eq!(response.authority, "none");
    assert_eq!(response.trust_version, 0);
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_account_invite_grants WHERE tenant_id = $1",
            &[&near],
        )
        .await,
        0
    );
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_account_trust WHERE tenant_id = $1",
            &[&near],
        )
        .await,
        0
    );
    let mut claimed = device_upload_principal_refs(&legacy, &device.id, &device.id)
        .unwrap()
        .to_vec();
    claimed.extend(
        device_upload_principal_refs(
            &legacy,
            &device.id,
            &device_claim_actor(&legacy, &device.id, Some(&enrolled_subject)),
        )
        .unwrap(),
    );
    assert_eq!(
        fx.owner
            .resolve_principals_to_accounts(&legacy, &claimed)
            .await
            .unwrap()
            .len(),
        4,
        "the bare actor and the enrollment's subject actor, both forms"
    );
    // The same device again: the same record.
    assert_eq!(
        fx.link(&near, account, &legacy, &subject, &device)
            .await
            .unwrap()
            .record
            .link_id,
        response.record.link_id
    );
    // Another account cannot claim it.
    let (other, other_account) = fx.near_ai_account().await;
    assert_eq!(
        fx.link(&other, other_account, &legacy, &subject, &device)
            .await
            .unwrap_err(),
        LinkRefusal::TenantClaimed
    );
    // A pooled instance tenant never links.
    let pooled_device = new_device();
    let (pooled, pooled_subject) = fx.legacy_instance_tenant(&pooled_device).await;
    fx.exec(
        "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
         VALUES ($1, 'shared')",
        &[&pooled],
    )
    .await;
    assert_eq!(
        fx.link(&near, account, &pooled, &pooled_subject, &pooled_device)
            .await
            .unwrap_err(),
        LinkRefusal::TenantPooled
    );
    // The operator resolves the claim (the first link is right), so the
    // fleet-wide readiness other tests read is not left blocked.
    fx.exec(
        "UPDATE trace_legacy_invite_link_conflicts SET resolved_at = now()
          WHERE legacy_tenant_id = $1",
        &[&legacy],
    )
    .await;
    // A wrong key still reads as not eligible.
    let stranger = new_device();
    assert_eq!(
        fx.link(&near, account, &legacy, &subject, &stranger)
            .await
            .unwrap_err(),
        LinkRefusal::DeviceNotEligible
    );
}

/// C1 and C2: the whole migration writes no anchor and no NEAR AI device
/// in any legacy tenant, so readiness is untouched.
#[tokio::test]
async fn the_migration_leaves_readiness_and_anchors_alone() {
    let Some(fx) = fixture().await else { return };
    let (near, account, _legacy, _) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-z")
        .await;
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_near_account_anchors WHERE tenant_id !~ '^(near-|nearai-)'",
            &[],
        )
        .await,
        0
    );
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM device_keys
              WHERE tenant_id !~ '^(near-|nearai-)' AND onboarding_origin <> 'invite'",
            &[],
        )
        .await,
        0
    );
    let ready: bool = fx
        .admin
        .query_one("SELECT trace_account_admission_linkage_ready()", &[])
        .await
        .unwrap()
        .get(0);
    assert!(ready);
}

impl Fixture {
    async fn carry_invalid(&self, tenant: &str, account: Uuid) -> bool {
        self.payout_runtime
            .payout_account_standing(tenant, account)
            .await
            .unwrap()
            .carry_invalid
    }

    async fn revoke(&self, near: &str, legacy: &str) {
        // Fixture tenant ids are `[a-z0-9-]` only, so inlining them is safe.
        self.operator
            .batch_execute(&format!(
                "BEGIN;
                 SELECT set_config('trace_commons.trace_tenant_id', '{near}', true);
                 UPDATE trace_legacy_invite_links SET revoked_at = now()
                  WHERE legacy_tenant_id = '{legacy}' AND revoked_at IS NULL;
                 COMMIT;"
            ))
            .await
            .unwrap();
    }
}

/// Review of #1865, finding 1: after a recipient change and before the new
/// carry activates, a revocation stops both carries. The old one was
/// unlinked at the change; the new waiting one closes at zero.
#[tokio::test]
async fn a_revocation_after_a_change_and_before_activation_stops_both_carries() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-1a")
        .await;
    fx.activate(&near, account).await;
    fx.activate(&legacy, claimed).await;
    let old = fx.link_in(&legacy, claimed, "active").await.unwrap();
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-1b")
        .await;
    let new = fx.link_in(&legacy, claimed, "waiting").await.unwrap();
    fx.revoke(&near, &legacy).await;
    let row = fx
        .admin
        .query_one(
            "SELECT (SELECT unlink_requested_at IS NOT NULL FROM trace_payout_links WHERE link_id = $1),
                    (SELECT state FROM trace_payout_links WHERE link_id = $2)",
            &[&old, &new],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0), "the old carry is unlinked");
    assert_eq!(
        row.get::<_, String>(1),
        "closed",
        "the new carry closed at zero"
    );
    assert_eq!(fx.live_carries(&legacy, claimed).await, 0);
}

/// Review of #1865, finding 4, and #1872. A pooled mark, a deleted tenant
/// link and a closed source account release the carry in their own
/// transaction (V148), so nothing is left to hold. A credit hold on the
/// source account, which is temporary, and a source link unlinked outside
/// a confirm leave the carry live, and the account standing checked before
/// every send holds it.
#[tokio::test]
async fn the_send_time_standing_holds_a_carry_that_no_longer_stands() {
    let Some(fx) = fixture().await else { return };

    // Pooled after linking.
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-4a")
        .await;
    assert!(!fx.carry_invalid(&legacy, claimed).await);
    assert!(!fx.carry_invalid(&near, account).await);
    fx.exec(
        "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
         VALUES ($1, 'shared event invite')",
        &[&legacy],
    )
    .await;
    assert_eq!(
        fx.live_carries(&legacy, claimed).await,
        0,
        "released by the mark"
    );
    assert!(!fx.carry_invalid(&legacy, claimed).await);

    // The tenant link row deleted rather than revoked.
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-4b")
        .await;
    fx.exec(
        "DELETE FROM trace_legacy_invite_links WHERE legacy_tenant_id = $1",
        &[&legacy],
    )
    .await;
    assert_eq!(
        fx.live_carries(&legacy, claimed).await,
        0,
        "released by the delete"
    );

    // The NEAR AI account closed, then (another pair) put on a credit hold.
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-4c")
        .await;
    fx.exec(
        "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
        &[&near, &account],
    )
    .await;
    assert_eq!(
        fx.live_carries(&legacy, claimed).await,
        0,
        "released by the close"
    );
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-4d")
        .await;
    fx.exec(
        "INSERT INTO trace_credit_holds (tenant_id, hold_id, credit_account_ref,
             credit_account_hash, reason, reason_hash, actor_principal_ref)
         VALUES ($1, $2, $3, $4, 'test', $4, 'test-operator')",
        &[
            &near,
            &Uuid::new_v4(),
            &format!("account:{account}"),
            &hash(&format!("account:{account}")),
        ],
    )
    .await;
    assert!(fx.carry_invalid(&legacy, claimed).await);
    assert_eq!(
        fx.payout_runtime
            .payout_account_standing(&legacy, claimed)
            .await
            .unwrap()
            .hold_label(),
        Some(trace_commons_server::db::PAYOUT_LEGACY_CARRY_INVALID)
    );

    // The source link unlinked without a carry run.
    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-4e")
        .await;
    fx.activate(&near, account).await;
    fx.exec(
        "UPDATE trace_payout_links SET unlink_requested_at = now()
          WHERE tenant_id = $1 AND account_id = $2",
        &[&near, &account],
    )
    .await;
    assert!(fx.carry_invalid(&legacy, claimed).await);
}

/// Review of #1865, finding 7: only a carried waiting link may close at
/// zero; any other waiting link still moves only to active.
#[tokio::test]
async fn only_a_carried_waiting_link_closes_at_zero() {
    let Some(fx) = fixture().await else { return };
    let (near, account) = fx.near_ai_account().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-7")
        .await;
    let own = fx.link_in(&near, account, "waiting").await.unwrap();
    let refused = fx
        .admin
        .execute(
            "UPDATE trace_payout_links SET state = 'closed', final_paid = 0 WHERE link_id = $1",
            &[&own],
        )
        .await
        .unwrap_err();
    let message = refused
        .as_db_error()
        .map(|error| error.message().to_owned())
        .unwrap_or_default();
    assert!(message.contains("a link moves only"), "{refused:?}");
}

/// Review of #1865, finding 8: the legacy tenant's audit rows carry the
/// NEAR AI account's actor hash hashed again, so rows in the two tenants
/// cannot be joined on it.
#[tokio::test]
async fn carry_audit_actors_cannot_be_joined_across_tenants() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, _) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-8")
        .await;
    assert!(
        fx.count(
            "SELECT count(*) FROM trace_payout_audit WHERE tenant_id = $1",
            &[&legacy],
        )
        .await
            > 0
    );
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_payout_audit l JOIN trace_payout_audit n
                 ON n.actor_ref_hash = l.actor_ref_hash
              WHERE l.tenant_id = $1 AND n.tenant_id = $2",
            &[&legacy, &near],
        )
        .await,
        0
    );
}

/// Review of #1865 (second pass): the send-time check reads the source
/// account's standing as broadly as the account's own standing does. A
/// credit hold spelled with an uppercase, braced or compact UUID, and an
/// open payout hold on the rail, both hold the carry; and a call under
/// another tenant's context is refused, never answered "valid".
#[tokio::test]
async fn the_send_time_check_reads_every_hold_on_the_source_and_refuses_misuse() {
    let Some(fx) = fixture().await else { return };
    for spelling in [
        |id: Uuid| format!("ACCOUNT:{}", id.to_string().to_uppercase()),
        |id: Uuid| format!("account:{{{id}}}"),
        |id: Uuid| format!("account:{}", id.simple()),
        |id: Uuid| format!("account:urn:uuid:{id}"),
    ] {
        let (near, account, legacy, claimed) = fx.linked_pair().await;
        fx.confirm(&near, account, PayoutLinkAction::Link, "org-h")
            .await;
        assert!(!fx.carry_invalid(&legacy, claimed).await);
        let reference = spelling(account);
        fx.exec(
            "INSERT INTO trace_credit_holds (tenant_id, hold_id, credit_account_ref,
                 credit_account_hash, reason, reason_hash, actor_principal_ref)
             VALUES ($1, $2, $3, $4, 'test', $4, 'test-operator')",
            &[&near, &Uuid::new_v4(), &reference, &hash(&reference)],
        )
        .await;
        assert!(fx.carry_invalid(&legacy, claimed).await, "{reference}");
    }

    let (near, account, legacy, claimed) = fx.linked_pair().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-i")
        .await;
    fx.exec(
        "INSERT INTO trace_payout_holds (tenant_id, hold_id, account_id, rail_id, label)
         VALUES ($1, $2, $3, $4, 'test_hold')",
        &[&near, &Uuid::new_v4(), &account, &RAIL],
    )
    .await;
    assert!(fx.carry_invalid(&legacy, claimed).await);

    let refused = fx
        .admin
        .query_one(
            "SELECT trace_legacy_payout_carry_invalid($1, $2)",
            &[&legacy, &claimed],
        )
        .await
        .unwrap_err();
    assert_eq!(
        refused.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
}

/// #1874: the SQL hash helpers V146 matches on and the Rust functions the
/// store writes with agree, so a carry, a release and the send-time check
/// all name the same rows.
#[tokio::test]
async fn the_sql_hash_helpers_match_the_rust_hashes() {
    let Some(fx) = fixture().await else { return };
    for _ in 0..8 {
        let id = Uuid::new_v4();
        let row = fx
            .admin
            .query_one(
                "SELECT trace_payout_link_hash($1), trace_payout_account_hash($1),
                        trace_legacy_link_hash($1)",
                &[&id],
            )
            .await
            .unwrap();
        assert_eq!(
            row.get::<_, String>(0),
            trace_commons_server::db::payout_link_id_hash(id)
        );
        assert_eq!(
            row.get::<_, String>(1),
            trace_commons_server::db::payout_account_id_hash(id)
        );
        assert_eq!(
            row.get::<_, String>(2),
            trace_commons_server::db::payout_legacy_link_hash(id)
        );
    }
}

impl Fixture {
    /// A pair with the source and the carry both active, and a second legacy
    /// account whose carry is still waiting.
    async fn carried_pair(&self, seed: &str) -> (String, Uuid, String, Uuid, Uuid) {
        let (near, account, legacy, claimed) = self.linked_pair().await;
        let waiting = self.add_account(&legacy, false).await;
        self.confirm(&near, account, PayoutLinkAction::Link, seed)
            .await;
        self.activate(&near, account).await;
        self.activate(&legacy, claimed).await;
        (near, account, legacy, claimed, waiting)
    }

    /// Both carries released in the event's own transaction: the active one
    /// unlinked, the waiting one closed at zero, each audited with `label`.
    async fn assert_released(&self, legacy: &str, active: Uuid, waiting: Uuid, label: &str) {
        let row = self
            .admin
            .query_one(
                "SELECT (SELECT bool_and(l.unlink_requested_at IS NOT NULL)
                           FROM trace_payout_links l
                          WHERE l.tenant_id = $1 AND l.account_id = $2 AND l.state = 'active'),
                        (SELECT count(*) FROM trace_payout_links l
                          WHERE l.tenant_id = $1 AND l.account_id = $3 AND l.state = 'waiting'),
                        (SELECT count(*) FROM trace_legacy_payout_carries c
                          WHERE c.tenant_id = $1 AND c.released_at IS NULL),
                        (SELECT count(*) FROM trace_payout_audit a
                          WHERE a.tenant_id = $1 AND a.label = $4)",
                &[&legacy, &active, &waiting, &label],
            )
            .await
            .unwrap();
        assert_eq!(
            row.get::<_, Option<bool>>(0),
            Some(true),
            "active carry unlinked"
        );
        assert_eq!(row.get::<_, i64>(1), 0, "waiting carry closed");
        assert_eq!(row.get::<_, i64>(2), 0, "no live carry left");
        assert_eq!(row.get::<_, i64>(3), 2, "both releases audited as {label}");
    }
}

/// #1872: marking a linked tenant pooled releases its carries at once, so
/// nothing stays held until the next carry.
#[tokio::test]
async fn marking_a_tenant_pooled_releases_its_carries() {
    let Some(fx) = fixture().await else { return };
    let (_, _, legacy, active, waiting) = fx.carried_pair("org-p1").await;
    fx.exec(
        "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
         VALUES ($1, 'shared event invite')",
        &[&legacy],
    )
    .await;
    fx.assert_released(&legacy, active, waiting, "legacy_tenant_pooled")
        .await;
    assert!(
        !fx.carry_invalid(&legacy, active).await,
        "released, so not held"
    );
}

/// #1872: closing the NEAR AI account releases its carries in every
/// tenant it linked.
#[tokio::test]
async fn closing_the_source_account_releases_its_carries() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, active, waiting) = fx.carried_pair("org-c1").await;
    fx.exec(
        "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
        &[&near, &account],
    )
    .await;
    fx.assert_released(&legacy, active, waiting, "legacy_source_account_closed")
        .await;
}

/// #1872: closing a legacy account releases its own carry, and only its own.
#[tokio::test]
async fn closing_a_legacy_account_releases_its_own_carry() {
    let Some(fx) = fixture().await else { return };
    let (_, _, legacy, active, waiting) = fx.carried_pair("org-c2").await;
    fx.exec(
        "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
        &[&legacy, &active],
    )
    .await;
    assert_eq!(fx.live_carries(&legacy, active).await, 0);
    assert_eq!(
        fx.live_carries(&legacy, waiting).await,
        1,
        "the other account keeps its carry"
    );
}

/// #1872: deleting the tenant link row (rather than revoking it) releases
/// its carries, as a revocation does.
#[tokio::test]
async fn deleting_the_tenant_link_releases_its_carries() {
    let Some(fx) = fixture().await else { return };
    let (_, _, legacy, active, waiting) = fx.carried_pair("org-d1").await;
    fx.exec(
        "DELETE FROM trace_legacy_invite_links WHERE legacy_tenant_id = $1",
        &[&legacy],
    )
    .await;
    fx.assert_released(&legacy, active, waiting, "legacy_tenant_link_deleted")
        .await;
}

/// Review of #1879: deleting a linked legacy tenant (an erasure) still
/// succeeds while it has live carries. The tenant link's delete trigger
/// fires inside the cascade and must not write into a tenant being removed.
#[tokio::test]
async fn deleting_a_linked_legacy_tenant_with_live_carries_succeeds() {
    let Some(fx) = fixture().await else { return };
    let (_, _, legacy, active, _) = fx.carried_pair("org-e1").await;
    assert_eq!(fx.live_carries(&legacy, active).await, 1);
    fx.exec("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&legacy])
        .await;
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_payout_audit WHERE tenant_id = $1",
            &[&legacy],
        )
        .await,
        0
    );
}

/// Review of #1879: the send-time check is still the backstop for each
/// event V148 now releases from. With an event's trigger disabled, the
/// carry stays live, and the standing holds it.
#[tokio::test]
async fn the_send_time_check_backs_up_each_release_event() {
    let Some(fx) = fixture().await else { return };
    for (trigger, table) in [
        (
            "trace_legacy_pooled_tenant_releases_carries",
            "trace_legacy_invite_pooled_tenants",
        ),
        (
            "trace_legacy_account_close_releases_carries",
            "trace_accounts",
        ),
        (
            "trace_legacy_invite_link_delete_releases_carries",
            "trace_legacy_invite_links",
        ),
    ] {
        let (near, account, legacy, active, _) = fx.carried_pair("org-b1").await;
        fx.admin
            .batch_execute(&format!("ALTER TABLE {table} DISABLE TRIGGER {trigger}"))
            .await
            .unwrap();
        match table {
            "trace_legacy_invite_pooled_tenants" => {
                fx.exec(
                    "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
                     VALUES ($1, 'shared event invite')",
                    &[&legacy],
                )
                .await
            }
            "trace_accounts" => {
                fx.exec(
                    "UPDATE trace_accounts SET closed_at = now()
                      WHERE tenant_id = $1 AND account_id = $2",
                    &[&near, &account],
                )
                .await
            }
            _ => {
                fx.exec(
                    "DELETE FROM trace_legacy_invite_links WHERE legacy_tenant_id = $1",
                    &[&legacy],
                )
                .await
            }
        }
        fx.admin
            .batch_execute(&format!("ALTER TABLE {table} ENABLE TRIGGER {trigger}"))
            .await
            .unwrap();
        assert_eq!(
            fx.live_carries(&legacy, active).await,
            1,
            "{trigger} disabled"
        );
        assert!(
            fx.carry_invalid(&legacy, active).await,
            "{trigger}: the backstop holds"
        );
    }
}

/// Review of #1879: a carry that changes nothing writes no audit row, so the
/// pass carrying on every run does not grow the audit table with an
/// account that keeps its own link (decision 6) skipped each time.
#[tokio::test]
async fn a_carry_that_changes_nothing_writes_no_audit_row() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, _) = fx.linked_pair().await;
    let own = fx.add_account(&legacy, false).await;
    fx.exec(
        "INSERT INTO trace_payout_links (tenant_id, account_id, rail_id, link_id, recipient_kind,
             state, sealed_recipient, recipient_index, recipient_key_ref, index_pepper_ref,
             subject_index, program)
         VALUES ($1, $2, $3, $4, 'near_ai_org', 'waiting', '{}', $5, 'k', 'p', $5, 'tc1-own2')",
        &[&legacy, &own, &RAIL, &Uuid::new_v4(), &hash("own2")],
    )
    .await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-a1")
        .await;
    let audits = || async {
        fx.count(
            "SELECT count(*) FROM trace_account_audit
              WHERE tenant_id = $1 AND action = 'legacy_payout_carried'",
            &[&near],
        )
        .await
    };
    let before = audits().await;
    for _ in 0..3 {
        let report = fx.carry(&near, account).await;
        assert_eq!((report.carried, report.released, report.skipped), (0, 0, 1));
    }
    assert_eq!(audits().await, before);
}

const HASH_HELPERS: [&str; 3] = [
    "trace_payout_link_hash",
    "trace_payout_account_hash",
    "trace_legacy_link_hash",
];

/// Review of #1878: the helpers are STABLE (their body calls the STABLE
/// `convert_to`), carry no `SET` clause so the planner inlines them, and are
/// closed to PUBLIC with execute granted to the roles that run their callers.
#[tokio::test]
async fn the_hash_helpers_are_stable_inlined_and_closed_to_public() {
    let Some(fx) = fixture().await else { return };
    for helper in HASH_HELPERS {
        let signature = format!("public.{helper}(uuid)");
        let row = fx
            .admin
            .query_one(
                "SELECT p.provolatile::TEXT, p.proconfig IS NULL,
                        has_function_privilege('public', p.oid, 'EXECUTE'),
                        has_function_privilege('trace_ingest_runtime', p.oid, 'EXECUTE'),
                        has_function_privilege('trace_legacy_payout_carry_guard', p.oid, 'EXECUTE'),
                        has_function_privilege('trace_legacy_invite_link_guard', p.oid, 'EXECUTE')
                   FROM pg_proc p WHERE p.oid = to_regprocedure($1)",
                &[&signature],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>(0), "s", "{helper} is STABLE");
        assert!(row.get::<_, bool>(1), "{helper} has no SET clause");
        assert!(!row.get::<_, bool>(2), "{helper} is closed to PUBLIC");
        for (column, role) in [(3, "runtime"), (4, "carry guard"), (5, "link guard")] {
            assert!(
                row.get::<_, bool>(column),
                "{helper} is granted to the {role}"
            );
        }
        // Inlined: the plan shows the expression, not a call to the helper.
        let plan: Vec<String> = fx
            .admin
            .query(
                &format!("EXPLAIN (VERBOSE, COSTS OFF) SELECT {helper}(gen_random_uuid())"),
                &[],
            )
            .await
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect();
        let plan = plan.join("\n");
        assert!(!plan.contains(helper), "{helper} is inlined: {plan}");
        assert!(plan.contains("sha256"), "{plan}");
    }
}

/// Review of #1878: the three hash formulas are spelled only inside their
/// helpers. Any function in the database that writes one of the domain
/// strings itself (a later migration re-inlining a formula, or a `CREATE OR
/// REPLACE` of a V146 function that stops calling the helper) fails this.
#[tokio::test]
async fn no_function_but_the_helpers_spells_a_hash_formula() {
    let Some(fx) = fixture().await else { return };
    let spellers: Vec<String> = fx
        .admin
        .query(
            "SELECT p.proname::TEXT FROM pg_proc p
               JOIN pg_namespace n ON n.oid = p.pronamespace
              WHERE n.nspname = 'public'
                AND (p.prosrc LIKE '%''payout-link-v1:''%'
                     OR p.prosrc LIKE '%''payout-account-v1:''%'
                     OR p.prosrc LIKE '%''legacy-invite-link-v1:''%')
              ORDER BY 1",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| row.get(0))
        .collect();
    let mut helpers: Vec<String> = HASH_HELPERS.iter().map(|name| name.to_string()).collect();
    helpers.sort();
    assert_eq!(spellers, helpers);
    // And the V146 functions that match on them call the helpers.
    for (function, helper) in [
        (
            "trace_legacy_payout_release_carry",
            "trace_payout_link_hash",
        ),
        (
            "trace_legacy_payout_release_carry",
            "trace_payout_account_hash",
        ),
        (
            "trace_legacy_invite_links_release_carries",
            "trace_legacy_link_hash",
        ),
        (
            "trace_legacy_payout_carry_invalid",
            "trace_legacy_link_hash",
        ),
        (
            "trace_legacy_payout_carry_invalid",
            "trace_payout_link_hash",
        ),
        (
            "trace_legacy_link_claim_device_principals",
            "trace_payout_account_hash",
        ),
        (
            "trace_link_legacy_instance_device",
            "trace_payout_account_hash",
        ),
    ] {
        let calls: bool = fx
            .admin
            .query_one(
                "SELECT bool_or(prosrc LIKE '%' || $2 || '(%') FROM pg_proc WHERE proname = $1",
                &[&function, &helper],
            )
            .await
            .unwrap()
            .get(0);
        assert!(calls, "{function} calls {helper}");
    }
}

/// Review of #1878: V146 was edited in place, and the runner keeps no
/// checksum, so a database that ran an earlier V146 must be refused rather
/// than run on silently. The migration runner's post-run check does that:
/// a missing helper, or one with the earlier IMMUTABLE label, is an error
/// naming V146.
#[tokio::test]
async fn a_database_that_ran_an_earlier_v146_is_refused() {
    let Some(fx) = fixture().await else { return };
    let mut client = fx
        .owner
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    trace_commons_server::db::postgres::verify_in_place_migration_edits(&**client)
        .await
        .expect("a database migrated to head passes");
    for change in [
        "DROP FUNCTION public.trace_legacy_link_hash(uuid)",
        "ALTER FUNCTION public.trace_payout_link_hash(uuid) IMMUTABLE",
    ] {
        let tx = client.transaction().await.unwrap();
        tx.batch_execute(change).await.unwrap();
        let refused = trace_commons_server::db::postgres::verify_in_place_migration_edits(&*tx)
            .await
            .expect_err(change);
        assert!(refused.to_string().contains("V146"), "{refused}");
        tx.rollback().await.unwrap();
    }
}

impl Fixture {
    /// Opens (or reuses) the account for `principal` in `tenant` the way the
    /// login-link mint does since V148: carrying in the same transaction.
    async fn open_account(&self, tenant: &str, principal: &str) -> Uuid {
        self.payout_runtime
            .create_or_reuse_account_carrying(tenant, principal, &[(rail(), programs())], ACTOR)
            .await
            .unwrap()
    }
}

/// #1873, owner decision 2026-11-03: opening an account in a linked legacy
/// tenant (the login-link path) carries the NEAR AI account's link to it in
/// the same transaction, whether or not the tenant had a carry before; a
/// repeat reuses the account and carries nothing twice.
#[tokio::test]
async fn opening_an_account_in_a_linked_tenant_carries_to_it() {
    let Some(fx) = fixture().await else { return };
    // A tenant already carried.
    let (_, _, legacy, _, _) = fx.carried_pair("org-o1").await;
    let opened = fx
        .open_account(&legacy, "principal_sha256:opened-later")
        .await;
    assert_eq!(fx.live_carries(&legacy, opened).await, 1);
    assert_eq!(
        fx.open_account(&legacy, "principal_sha256:opened-later")
            .await,
        opened
    );
    assert_eq!(
        fx.live_carries(&legacy, opened).await,
        1,
        "nothing carried twice"
    );

    // A tenant linked before it had any account: no carry yet, so no live
    // link of its own. Its first account is carried at opening.
    let device = new_device();
    let (fresh, invite) = fx.legacy_invite_tenant(&device).await;
    let (near, account) = fx.near_ai_account().await;
    fx.confirm(&near, account, PayoutLinkAction::Link, "org-o2")
        .await;
    fx.link(&near, account, &fresh, &invite, &device)
        .await
        .unwrap();
    let first = fx
        .open_account(&fresh, "principal_sha256:first-account")
        .await;
    assert_eq!(fx.live_carries(&fresh, first).await, 1);
}

/// Review of #1879: a closed NEAR AI account carries nothing, at any event.
#[tokio::test]
async fn opening_an_account_carries_nothing_from_a_closed_source() {
    let Some(fx) = fixture().await else { return };
    let (near, account, legacy, _, _) = fx.carried_pair("org-o3").await;
    fx.exec(
        "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
        &[&near, &account],
    )
    .await;
    let opened = fx
        .open_account(&legacy, "principal_sha256:after-close")
        .await;
    assert_eq!(fx.live_carries(&legacy, opened).await, 0);
    assert!(!fx.carry_invalid(&legacy, opened).await);
}

/// An account opened in a legacy tenant with no tenant link is just an
/// account: nothing is carried, and nothing is written outside the tenant.
#[tokio::test]
async fn opening_an_account_in_an_unlinked_tenant_carries_nothing() {
    let Some(fx) = fixture().await else { return };
    let device = new_device();
    let (legacy, _) = fx.legacy_invite_tenant(&device).await;
    let opened = fx.open_account(&legacy, "principal_sha256:unlinked").await;
    assert_eq!(fx.live_carries(&legacy, opened).await, 0);
    assert_eq!(
        fx.count(
            "SELECT count(*) FROM trace_payout_links WHERE tenant_id = $1",
            &[&legacy],
        )
        .await,
        0
    );
}
