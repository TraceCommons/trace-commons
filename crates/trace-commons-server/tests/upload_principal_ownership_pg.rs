// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use secrecy::SecretString;
use sha2::{Digest, Sha256};
use trace_commons_gate_api::RailId;
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

fn signed_principal(tenant: &str, actor: &str) -> String {
    let material = format!("signed:{tenant}:{actor}");
    format!(
        "principal_sha256:{}",
        hex::encode(Sha256::digest(
            format!("12:signed_claim:{}:{material}", material.len()).as_bytes()
        ))
    )
}

#[tokio::test]
async fn upload_ownership_follows_live_device_membership_without_authentication_authority() {
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
    admin.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='tc_upload_owner_test') THEN CREATE ROLE tc_upload_owner_test LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; GRANT trace_ingest_runtime TO tc_upload_owner_test; GRANT USAGE ON SCHEMA public TO tc_upload_owner_test;").await.unwrap();
    // The deployed runtime received grants on these pre-V62 tables before
    // migrations began carrying grants. Keep new-table grants migration-only.
    admin.batch_execute("GRANT SELECT,INSERT ON trace_tenants,trace_accounts,trace_account_principals TO tc_upload_owner_test; GRANT UPDATE ON trace_tenants,trace_account_principals TO tc_upload_owner_test; GRANT SELECT,UPDATE ON device_keys,trace_near_provisioned_devices TO tc_upload_owner_test; GRANT SELECT ON trace_submissions,trace_credit_holds TO tc_upload_owner_test;").await.unwrap();
    // Ownership linkage is private server state, including from the narrow
    // cross-tenant login lookup role. Exercise actual access, not only ACL text.
    let mut privacy_client = owner
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    for relation in [
        "trace_device_upload_principals",
        "trace_account_owned_principals",
    ] {
        assert!(
            !admin
                .query_one(
                    "SELECT has_table_privilege('trace_login_resolver',$1,'SELECT')",
                    &[&relation]
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
        let privacy_tx = privacy_client.transaction().await.unwrap();
        privacy_tx
            .batch_execute("SET LOCAL ROLE trace_login_resolver")
            .await
            .unwrap();
        let denied = privacy_tx
            .query(&format!("SELECT * FROM {relation} LIMIT 1"), &[])
            .await
            .unwrap_err();
        assert_eq!(denied.code().map(|code| code.code()), Some("42501"));
        privacy_tx.rollback().await.unwrap();
    }
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url.set_username("tc_upload_owner_test").unwrap();
    let runtime = PgBackend::new(&config(runtime_url.into())).await.unwrap();
    let tenant = format!("upload-owner-{}", Uuid::new_v4());
    let other_tenant = format!("upload-other-{}", Uuid::new_v4());
    let device = format!("sha256:{}", "a".repeat(64));
    // Unique device keys are global across all tenant fixtures.
    let device = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(format!("{tenant}:{device}")))
    );
    let parent = device_principal_ref(&tenant, &device);
    let actor = device_claim_actor(&tenant, &device, Some("sha256:subject"));
    let alias = signed_principal(&tenant, &actor);
    let account = Uuid::new_v4();
    let survivor = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1),($2)",
            &[&tenant, &other_tenant],
        )
        .await
        .unwrap();
    admin.execute("INSERT INTO device_keys(tenant_id,device_key_id,public_key,invite_subject_hash) VALUES($1,$2,'test',$2)", &[&tenant,&device]).await.unwrap();
    // A registered device can upload before an account exists.
    runtime
        .bind_device_upload_principal(&tenant, &device, &actor)
        .await
        .unwrap();
    assert_eq!(
        admin
            .query_one(
                "SELECT count(*) FROM trace_device_upload_principals WHERE tenant_id=$1",
                &[&tenant]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    assert!(
        runtime
            .resolve_principals_to_accounts(&tenant, std::slice::from_ref(&alias))
            .await
            .unwrap()
            .is_empty()
    );
    admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2),($1,$3)",
            &[&tenant, &account, &survivor],
        )
        .await
        .unwrap();
    admin.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)", &[&tenant,&account,&parent]).await.unwrap();
    admin.execute("INSERT INTO trace_near_account_anchors(tenant_id,account_id,anchor_hash,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,'{}',$3,$3)", &[&tenant,&account,&device]).await.unwrap();
    admin.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,account_id,principal_ref,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$4)", &[&tenant,&account,&parent,&device]).await.unwrap();
    runtime
        .bind_device_upload_principal(&tenant, &device, &actor)
        .await
        .unwrap();
    assert!(
        runtime
            .expand_account_principals(&tenant, account)
            .await
            .unwrap()
            .contains(&alias)
    );
    assert_eq!(
        runtime
            .resolve_principals_to_accounts(&tenant, std::slice::from_ref(&alias))
            .await
            .unwrap()[&alias],
        account
    );
    assert_eq!(
        runtime
            .payout_principal_accounts(&tenant, std::slice::from_ref(&alias))
            .await
            .unwrap()[&alias],
        account
    );
    assert_eq!(
        runtime
            .resolve_account_for_principal(&tenant, &alias)
            .await
            .unwrap(),
        None
    );
    assert!(
        runtime
            .create_or_reuse_account(&tenant, &alias)
            .await
            .is_err()
    );
    // Historical native uploads use the hashed device subject. Repair must
    // match that exact derivation, not every submission in this tenant.
    let subject = trace_commons_protocol::onboarding::user_subject_hash(&device);
    let native_actor = device_claim_actor(&tenant, &device, Some(&subject));
    let native_alias = signed_principal(&tenant, &native_actor);
    for principal in [&native_alias, &alias] {
        admin.execute("INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,redaction_pipeline_version,redaction_hash) VALUES($1,$2,$2,$3,'1','test','test','accepted','low','test','test')", &[&tenant,&Uuid::new_v4(),&principal]).await.unwrap();
    }
    assert_eq!(
        runtime
            .repair_native_device_upload_principal(&tenant, &device, true)
            .await
            .unwrap(),
        1
    );
    assert!(
        !runtime
            .expand_account_principals(&tenant, account)
            .await
            .unwrap()
            .contains(&native_alias)
    );
    assert_eq!(
        runtime
            .repair_native_device_upload_principal(&tenant, &device, false)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        runtime
            .repair_native_device_upload_principal(&tenant, &device, false)
            .await
            .unwrap(),
        1
    );
    assert!(
        runtime
            .expand_account_principals(&tenant, account)
            .await
            .unwrap()
            .contains(&native_alias)
    );
    let batch = Uuid::new_v4();
    admin.execute("INSERT INTO trace_payout_rate_epochs(rail_id,version,numerator,denominator) VALUES('near_ai','test',250,1) ON CONFLICT DO NOTHING", &[]).await.unwrap();
    let credit_hash = format!(
        "sha256:{}",
        hex::encode(Sha256::digest(native_alias.as_bytes()))
    );
    admin.execute("INSERT INTO trace_credit_settlement_batches(tenant_id,settlement_batch_id,policy_version,status,reason_hash,source_list_hash,settled_credit_points,settled_credit_micros,actor_principal_ref) VALUES($1,$2,'test','finalized',$3,$3,'25',25000000,'operator')", &[&tenant,&batch,&credit_hash]).await.unwrap();
    admin.execute("INSERT INTO trace_payout_accruals(tenant_id,accrual_id,rail_id,kind,settlement_batch_id,credit_account_hash,award_units,rail_units,rate_version,instrument_id) VALUES($1,$2,'near_ai','credit',$3,$4,25000000,6250000000,'test','trace_credit')", &[&tenant,&Uuid::new_v4(),&batch,&credit_hash]).await.unwrap();
    let rail = RailId::near_ai();
    let resolved = runtime
        .payout_principal_accounts(&tenant, std::slice::from_ref(&native_alias))
        .await
        .unwrap()[&native_alias];
    assert_eq!(resolved, account);
    assert_eq!(
        runtime
            .claim_payout_accruals(&tenant, &rail, &credit_hash, resolved, &credit_hash)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        runtime
            .claim_payout_accruals(&tenant, &rail, &credit_hash, resolved, &credit_hash)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        runtime
            .payout_owed(&tenant, account, &rail)
            .await
            .unwrap()
            .net(),
        Some(6_250_000_000)
    );
    // A hold under either provenance alias must reach the account before pay.
    for reference in [
        format!("account:{}", account.to_string().to_uppercase()),
        format!("account:{}", account.simple()),
        format!("account:{}", account.braced()),
        format!("account:{}", account.urn()),
        format!("ACCOUNT:{account}"),
    ] {
        let hold = Uuid::new_v4();
        let hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(reference.as_bytes()))
        );
        admin.execute("INSERT INTO trace_credit_holds(tenant_id,hold_id,credit_account_ref,credit_account_hash,reason,reason_hash,actor_principal_ref) VALUES($1,$2,$3,$4,'test',$4,'operator')", &[&tenant,&hold,&reference,&hash]).await.unwrap();
        assert!(
            runtime
                .payout_account_standing(&tenant, account)
                .await
                .unwrap()
                .credit_held
        );
        admin
            .execute(
                "UPDATE trace_credit_holds SET released_at=now() WHERE tenant_id=$1 AND hold_id=$2",
                &[&tenant, &hold],
            )
            .await
            .unwrap();
        assert!(
            !runtime
                .payout_account_standing(&tenant, account)
                .await
                .unwrap()
                .credit_held
        );
    }
    admin.execute("INSERT INTO trace_credit_holds(tenant_id,hold_id,credit_account_ref,credit_account_hash,reason,reason_hash,actor_principal_ref) VALUES($1,$2,$3,$4,'test',$4,'operator')", &[&tenant,&Uuid::new_v4(),&native_alias,&credit_hash]).await.unwrap();
    assert!(
        runtime
            .payout_account_standing(&tenant, account)
            .await
            .unwrap()
            .credit_held
    );
    assert!(
        runtime
            .bind_device_upload_principal(&other_tenant, &device, &actor)
            .await
            .is_err()
    );
    assert!(
        runtime
            .expand_account_principals(&other_tenant, account)
            .await
            .unwrap()
            .is_empty()
    );
    let mut restricted = runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let scoped = restricted.transaction().await.unwrap();
    scoped
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
            &[&other_tenant],
        )
        .await
        .unwrap();
    assert_eq!(
        scoped
            .query_one("SELECT count(*) FROM trace_account_owned_principals", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    scoped.commit().await.unwrap();
    // The reverse namespace race is refused too: a pre-existing independent
    // login principal cannot be silently claimed as this device's provenance.
    let conflicting_actor = device_claim_actor(&tenant, &device, Some("conflict"));
    let conflicting_alias = signed_principal(&tenant, &conflicting_actor);
    runtime
        .create_or_reuse_account(&tenant, &conflicting_alias)
        .await
        .unwrap();
    assert!(
        runtime
            .bind_device_upload_principal(&tenant, &device, &conflicting_actor)
            .await
            .is_err()
    );
    // An uncommitted login insertion must serialize against ownership binding.
    // Without the shared namespace lock, the binder could observe no login
    // membership and commit an alias while this transaction commits a login.
    let race_actor = device_claim_actor(&tenant, &device, Some("race"));
    let race_alias = signed_principal(&tenant, &race_actor);
    let race_tx = restricted.transaction().await.unwrap();
    race_tx
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id',$1,true)",
            &[&tenant],
        )
        .await
        .unwrap();
    race_tx.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)", &[&tenant,&survivor,&race_alias]).await.unwrap();
    let binding = runtime.bind_device_upload_principal(&tenant, &device, &race_actor);
    tokio::pin!(binding);
    tokio::select! {
        result = &mut binding => panic!("binding bypassed uncommitted namespace lock: {result:?}"),
        () = tokio::time::sleep(std::time::Duration::from_millis(100)) => {}
    }
    race_tx.commit().await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), &mut binding)
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(
        runtime
            .resolve_account_for_principal(&tenant, &race_alias)
            .await
            .unwrap(),
        Some(survivor)
    );
    assert_eq!(admin.query_one("SELECT count(*) FROM trace_device_upload_principals WHERE tenant_id=$1 AND upload_principal_ref=$2", &[&tenant,&race_alias]).await.unwrap().get::<_,i64>(0), 0);
    // Exercise the real merge transaction, including its guarded NEAR anchor
    // move and close of the absorbed account. The proposal is test evidence;
    // production obtains it by consuming the second account's login proof.
    let proposal = Uuid::new_v4();
    admin.execute("INSERT INTO trace_account_merge_proposals(tenant_id,proposal_id,surviving_account_id,absorbed_account_id,absorbed_principal_count,expires_at) VALUES($1,$2,$3,$4,1,now()+interval '1 hour')", &[&tenant,&proposal,&survivor,&account]).await.unwrap();
    assert!(
        owner
            .execute_merge(&tenant, survivor, proposal)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        runtime
            .payout_account_standing(&tenant, account)
            .await
            .unwrap()
            .credit_held
    );
    // Existing accrual ownership never follows later membership changes.
    assert_eq!(
        runtime
            .claim_payout_accruals(&tenant, &rail, &credit_hash, survivor, &credit_hash)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        runtime
            .payout_owed(&tenant, survivor, &rail)
            .await
            .unwrap()
            .net(),
        Some(0)
    );
    assert_eq!(
        runtime
            .payout_owed(&tenant, account, &rail)
            .await
            .unwrap()
            .net(),
        Some(6_250_000_000)
    );
    assert!(
        runtime
            .expand_account_principals(&tenant, account)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        runtime
            .expand_account_principals(&tenant, survivor)
            .await
            .unwrap()
            .contains(&alias)
    );
    // Unlinking a provisioned device does not reclassify it as legacy login.
    admin.execute("UPDATE trace_account_principals SET unlinked_at=now() WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&parent]).await.unwrap();
    let unlinked_actor = device_claim_actor(&tenant, &device, Some("after-unlink"));
    let unlinked_alias = signed_principal(&tenant, &unlinked_actor);
    runtime
        .bind_device_upload_principal(&tenant, &device, &unlinked_actor)
        .await
        .unwrap();
    assert!(
        runtime
            .create_or_reuse_account(&tenant, &unlinked_alias)
            .await
            .is_err()
    );
    assert!(
        runtime
            .payout_principal_accounts(&tenant, std::slice::from_ref(&unlinked_alias))
            .await
            .unwrap()
            .is_empty()
    );
    admin.execute("UPDATE trace_account_principals SET unlinked_at=NULL WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&parent]).await.unwrap();
    admin
        .execute(
            "UPDATE device_keys SET revoked_at=now() WHERE device_key_id=$1",
            &[&device],
        )
        .await
        .unwrap();
    assert!(
        runtime
            .bind_device_upload_principal(&tenant, &device, &actor)
            .await
            .is_err()
    );
    // Revocation prevents new credentials; historical ownership survives, like the parent.
    assert!(
        runtime
            .expand_account_principals(&tenant, survivor)
            .await
            .unwrap()
            .contains(&alias)
    );
    admin.execute("UPDATE trace_account_principals SET unlinked_at=now() WHERE tenant_id=$1 AND principal_ref=$2", &[&tenant,&parent]).await.unwrap();
    assert!(
        runtime
            .payout_account_standing(&tenant, account)
            .await
            .unwrap()
            .credit_held
    );
    let remaining = runtime
        .expand_account_principals(&tenant, survivor)
        .await
        .unwrap();
    assert!(!remaining.contains(&parent));
    assert!(!remaining.contains(&alias));
    assert!(!remaining.contains(&native_alias));
    assert!(remaining.contains(&race_alias));
    assert!(
        runtime
            .payout_principal_accounts(&tenant, std::slice::from_ref(&alias))
            .await
            .unwrap()
            .is_empty()
    );
}
