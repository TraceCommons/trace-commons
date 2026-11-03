// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Legacy account payout migration
//! (docs/superpowers/specs/2026-11-03-legacy-account-payout-migration-design.md).
//!
//! The first test is owner decision 5's precondition (spec section 3.5):
//! linking the signing device's principal to an account at tenant-link time
//! must make that device's past upload credit resolve to the account, both
//! for the settlement key and for the accrual claim. If it does not, the
//! decision goes back to the owner.

use secrecy::SecretString;
use sha2::{Digest, Sha256};
use trace_commons_server::account_trust::{device_claim_actor, device_principal_ref};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, PayoutStore, postgres::PgBackend};
use uuid::Uuid;

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

/// The principal ingest records for a signed upload claim by `actor`
/// (`account_trust::device_upload_principal_refs`, current form).
fn signed_principal(tenant: &str, actor: &str) -> String {
    let material = format!("signed:{tenant}:{actor}");
    format!(
        "principal_sha256:{}",
        hex::encode(Sha256::digest(
            format!("12:signed_claim:{}:{material}", material.len()).as_bytes()
        ))
    )
}

/// Decision 5's check. A legacy invite device (no NEAR provisioning) in a
/// `tenant-` tenant uploads; its credit is keyed by the upload principal.
/// The link step would link `device_principal_ref(tenant, device)`, the only
/// principal derivable from the device key alone. The device's upload
/// credit must then resolve to that account.
#[tokio::test]
async fn decision_5_linking_the_device_principal_claims_its_legacy_upload_credit() {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_PG_TEST_DATABASE_URL required");
        return;
    };
    assert!(url.contains("127.0.0.1"));
    let owner = PgBackend::new(&config(url.clone())).await.unwrap();
    owner.run_migrations().await.unwrap();
    let admin = owner
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='tc_legacy_carry_test') THEN CREATE ROLE tc_legacy_carry_test LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; GRANT trace_ingest_runtime TO tc_legacy_carry_test; GRANT USAGE ON SCHEMA public TO tc_legacy_carry_test;").await.unwrap();
    // Pre-V62 tables carry no migration grants; the deployed runtime has them.
    admin.batch_execute("GRANT SELECT,INSERT ON trace_tenants,trace_accounts,trace_account_principals TO tc_legacy_carry_test; GRANT UPDATE ON trace_tenants,trace_account_principals TO tc_legacy_carry_test; GRANT SELECT,UPDATE ON device_keys,trace_near_provisioned_devices TO tc_legacy_carry_test; GRANT SELECT ON trace_submissions,trace_credit_holds TO tc_legacy_carry_test;").await.unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url.set_username("tc_legacy_carry_test").unwrap();
    let runtime = PgBackend::new(&config(runtime_url.into())).await.unwrap();

    let tenant = format!("tenant-{}", Uuid::new_v4().simple());
    let device = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(format!("{tenant}:legacy-device")))
    );
    admin
        .execute("INSERT INTO trace_tenants(tenant_id) VALUES($1)", &[&tenant])
        .await
        .unwrap();
    // An invite-onboarded legacy device: `invite` origin, no NEAR provisioning.
    admin.execute("INSERT INTO device_keys(tenant_id,device_key_id,public_key,invite_subject_hash) VALUES($1,$2,'test',$2)", &[&tenant,&device]).await.unwrap();

    // Its uploads: a bare device-key actor (what the app sends), and a
    // per-user subject actor (what an instance-vouched enrollment may send).
    let bare_actor = device_claim_actor(&tenant, &device, None);
    let subject_actor = device_claim_actor(&tenant, &device, Some("user-1"));
    let upload_principals = [
        signed_principal(&tenant, &bare_actor),
        signed_principal(&tenant, &subject_actor),
    ];
    for actor in [&bare_actor, &subject_actor] {
        runtime
            .bind_device_upload_principal(&tenant, &device, actor)
            .await
            .unwrap();
    }

    // The link step, as section 3.5 designs it.
    let account = runtime
        .create_or_reuse_account(&tenant, &device_principal_ref(&tenant, &device))
        .await
        .unwrap();

    let settled = runtime
        .resolve_principals_to_accounts(&tenant, &upload_principals)
        .await
        .unwrap();
    let claimable = runtime
        .payout_principal_accounts(&tenant, &upload_principals)
        .await
        .unwrap();
    for principal in &upload_principals {
        assert_eq!(
            settled.get(principal),
            Some(&account),
            "settlement key of a legacy upload principal does not resolve to the linked account"
        );
        assert_eq!(
            claimable.get(principal),
            Some(&account),
            "accrual claim of a legacy upload principal does not resolve to the linked account"
        );
    }
}
