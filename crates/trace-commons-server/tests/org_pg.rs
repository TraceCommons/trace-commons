// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Organizations (V159) on a real PostgreSQL, as the least-privilege ingest
//! runtime role: creating an org with its holder, admitting seats within
//! their limits, linking devices, the policy write with its tenant-policy
//! mirror and audit row, seat removal, tenant isolation, and the settlement
//! target the store's record yields.

#[path = "support/role_ddl_lock.rs"]
mod role_ddl_lock;

use chrono::{Duration, Utc};
use role_ddl_lock::with_role_ddl_lock;
use trace_commons_protocol::org_policy::{
    OrgMemberAutomatic, OrgPolicy, OrgRole, OrgSeatKind, OrgSharing, OrgStatus,
};
use trace_commons_server::{
    config::DatabaseConfig,
    db::{
        Database, NewOrg, OrgPolicyUpdateOutcome, OrgSeatAdmission, OrgSeatAdmitOutcome,
        OrgSeatRemoveOutcome, postgres::PgBackend,
    },
    org::{OrgSettlementTarget, org_settlement_target, org_settlement_target_for},
    trace_corpus_storage::TraceCorpusStore,
};
use uuid::Uuid;

/// The migrator's backend (fixtures) and the ingest runtime's (every method
/// under test, under V159's grants and forced RLS).
async fn backends() -> Option<(PgBackend, PgBackend)> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return None;
    };
    let parsed = reqwest::Url::parse(&url).unwrap();
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 4))
        .await
        .unwrap();
    admin.run_migrations().await.unwrap();
    // V90: the runtime's privileges on tables older than V62 are the
    // operator's table-wide grant, not a migration's. Model that grant on
    // only the older tables the org store touches; V159's own grants cover
    // the org tables, so a missing one still fails here.
    admin
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .batch_execute(&with_role_ddl_lock(
            "GRANT SELECT, INSERT, UPDATE, DELETE ON trace_tenants, trace_tenant_policies,
                    trace_tenant_access_grants, trace_accounts, trace_account_principals,
                    trace_account_audit, trace_sessions, device_keys
               TO trace_ingest_runtime;
             GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO trace_ingest_runtime",
        ))
        .await
        .unwrap();
    let mut runtime_url = parsed.clone();
    runtime_url.set_query(Some("options=-c%20role%3Dtrace_ingest_runtime"));
    let runtime = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 2))
        .await
        .unwrap();
    let row = runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap()
        .query_one(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .unwrap();
    assert!(!row.get::<_, bool>(0) && !row.get::<_, bool>(1));
    Some((admin, runtime))
}

fn hash() -> String {
    use sha2::Digest as _;
    format!(
        "sha256:{}",
        hex::encode(sha2::Sha256::digest(Uuid::new_v4().as_bytes()))
    )
}

async fn new_org(runtime: &PgBackend, seat_limit: i32, cap: Option<i64>) -> String {
    let tenant_id = trace_commons_server::org::random_org_tenant_id();
    runtime
        .create_org(NewOrg {
            tenant_id: tenant_id.clone(),
            policy: OrgPolicy::conservative_default(),
            seat_limit,
            server_limit: 1,
            settlement_cap_micros: cap,
            actor_ref: "operator:test".into(),
        })
        .await
        .unwrap();
    tenant_id
}

fn member(tenant_id: &str, role: OrgRole) -> OrgSeatAdmission {
    OrgSeatAdmission {
        tenant_id: tenant_id.to_string(),
        kind: OrgSeatKind::Member,
        org_role: Some(role),
        identity_index: hash(),
        authority_id: None,
        membership_proved_at: Some(Utc::now()),
        actor_ref: "operator:test".into(),
    }
}

async fn admitted(runtime: &PgBackend, admission: OrgSeatAdmission) -> Uuid {
    match runtime.admit_org_seat(admission).await.unwrap() {
        OrgSeatAdmitOutcome::Admitted { account_id, .. } => account_id,
        other => panic!("seat not admitted: {other:?}"),
    }
}

/// Register a device key in the tenant, as onboarding does, from the
/// migrator's connection.
async fn device_key(admin: &PgBackend, tenant_id: &str) -> String {
    let device_key_id = hash();
    let client = admin.trace_pool_for_test().get().await.unwrap();
    client
        .execute(
            "INSERT INTO device_keys (device_key_id, tenant_id, public_key, invite_subject_hash)
             VALUES ($1, $2, 'pk', $3)",
            &[&device_key_id, &tenant_id, &hash()],
        )
        .await
        .unwrap();
    device_key_id
}

#[tokio::test]
async fn create_org_makes_a_holder_and_mirrors_policy() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant_id = new_org(&runtime, 5, Some(5_000_000)).await;
    let org = runtime.get_org(&tenant_id).await.unwrap().expect("org");
    assert_eq!(org.status, OrgStatus::Active);
    assert_eq!(org.policy, OrgPolicy::conservative_default());
    assert_eq!(
        org.policy_digest,
        OrgPolicy::conservative_default().digest()
    );
    assert_eq!(org.policy_version, 1);
    assert!(org.holder_account_open);
    assert!(!org.near_ai_org_bound);
    let holder = org.holder_account_id.expect("holder account");
    let seat = runtime
        .get_org_seat(&tenant_id, holder)
        .await
        .unwrap()
        .expect("holder seat");
    assert_eq!(seat.kind, OrgSeatKind::Holder);

    // The tenant policy mirror carries the org's ceilings.
    let mirror = runtime
        .get_trace_tenant_policy(&tenant_id)
        .await
        .unwrap()
        .expect("mirror");
    assert_eq!(mirror.policy_version, "org-policy-v1");
    assert_eq!(
        mirror.allowed_uses,
        vec!["debugging", "evaluation", "aggregate_analytics"]
    );

    // Credit settles to the holder, under the org cap.
    assert_eq!(
        org_settlement_target_for(&runtime, &tenant_id)
            .await
            .unwrap(),
        OrgSettlementTarget::Holder {
            account_id: holder,
            cap_micros: 5_000_000
        }
    );

    // A second holder seat is refused by the one-holder index.
    let client = admin.trace_pool_for_test().get().await.unwrap();
    let extra = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant_id, &extra],
        )
        .await
        .unwrap();
    assert!(
        client
            .execute(
                "INSERT INTO trace_org_seats (tenant_id, account_id, kind) VALUES ($1, $2, 'holder')",
                &[&tenant_id, &extra],
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn create_org_refuses_a_tenant_outside_the_namespace() {
    let Some((_, runtime)) = backends().await else {
        return;
    };
    for tenant_id in ["tenant-x".to_string(), "org-short".to_string()] {
        assert!(
            runtime
                .create_org(NewOrg {
                    tenant_id,
                    policy: OrgPolicy::conservative_default(),
                    seat_limit: 1,
                    server_limit: 1,
                    settlement_cap_micros: None,
                    actor_ref: "operator:test".into(),
                })
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn an_org_with_no_cap_blocks_settlement() {
    let Some((_, runtime)) = backends().await else {
        return;
    };
    let tenant_id = new_org(&runtime, 5, None).await;
    assert_eq!(
        org_settlement_target_for(&runtime, &tenant_id)
            .await
            .unwrap(),
        OrgSettlementTarget::Blocked("org_settlement_cap_missing")
    );
    // An org-namespace tenant with no record at all is blocked too, and the
    // read leaves no tenant behind.
    let ghost = trace_commons_server::org::random_org_tenant_id();
    assert!(runtime.get_org(&ghost).await.unwrap().is_none());
    assert_eq!(
        org_settlement_target(&ghost, None),
        OrgSettlementTarget::Blocked("org_record_missing")
    );
}

#[tokio::test]
async fn seats_are_found_again_and_bounded_by_the_limit() {
    let Some((_, runtime)) = backends().await else {
        return;
    };
    let tenant_id = new_org(&runtime, 2, Some(1)).await;
    let admission = member(&tenant_id, OrgRole::OrgAdmin);
    let first = admitted(&runtime, admission.clone()).await;
    // The same identity finds the same seat, and the proof moves forward.
    let mut again = admission.clone();
    again.membership_proved_at = Some(Utc::now() + Duration::minutes(1));
    match runtime.admit_org_seat(again).await.unwrap() {
        OrgSeatAdmitOutcome::Admitted {
            account_id,
            created,
        } => {
            assert_eq!(account_id, first);
            assert!(!created);
        }
        other => panic!("{other:?}"),
    }
    let seat = runtime
        .get_org_seat(&tenant_id, first)
        .await
        .unwrap()
        .unwrap();
    assert!(seat.is_org_admin());
    assert!(seat.requires_membership_proof);
    assert!(seat.membership_proved_at.unwrap() > Utc::now());

    admitted(&runtime, member(&tenant_id, OrgRole::Member)).await;
    assert_eq!(
        runtime
            .admit_org_seat(member(&tenant_id, OrgRole::Member))
            .await
            .unwrap(),
        OrgSeatAdmitOutcome::SeatLimitReached
    );
    // The same identity as another kind is refused.
    let mut as_server = admission;
    as_server.kind = OrgSeatKind::Server;
    as_server.org_role = None;
    as_server.membership_proved_at = None;
    assert_eq!(
        runtime.admit_org_seat(as_server).await.unwrap(),
        OrgSeatAdmitOutcome::SeatKindMismatch
    );
}

#[tokio::test]
async fn device_and_principal_find_the_seat() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant_id = new_org(&runtime, 5, Some(1)).await;
    let account = admitted(&runtime, member(&tenant_id, OrgRole::Member)).await;
    let device = device_key(&admin, &tenant_id).await;
    assert!(
        runtime
            .register_org_seat_device(&tenant_id, account, &device, None)
            .await
            .unwrap()
    );
    let by_device = runtime
        .get_org_seat_for_device(&tenant_id, &device)
        .await
        .unwrap()
        .expect("seat by device");
    assert_eq!(by_device.account_id, account);
    let principal = trace_commons_server::account_trust::device_principal_ref(&tenant_id, &device);
    let by_principal = runtime
        .get_org_seat_for_principal(&tenant_id, &principal)
        .await
        .unwrap()
        .expect("seat by principal");
    assert_eq!(by_principal.account_id, account);

    // A key from another tenant is never linked.
    let other_tenant = new_org(&runtime, 5, Some(1)).await;
    let foreign = device_key(&admin, &other_tenant).await;
    assert!(
        !runtime
            .register_org_seat_device(&tenant_id, account, &foreign, None)
            .await
            .unwrap()
    );
    // An expired short-lived key no longer finds its seat.
    let short = device_key(&admin, &tenant_id).await;
    assert!(
        runtime
            .register_org_seat_device(
                &tenant_id,
                account,
                &short,
                Some(Utc::now() - Duration::seconds(1))
            )
            .await
            .unwrap()
    );
    assert!(
        runtime
            .get_org_seat_for_device(&tenant_id, &short)
            .await
            .unwrap()
            .is_none()
    );

    let listing = runtime.list_org_seats(&tenant_id).await.unwrap();
    assert_eq!(listing.len(), 1, "the holder is not listed");
    assert_eq!(listing[0].active_devices, 1);
}

#[tokio::test]
async fn policy_write_versions_mirrors_and_audits() {
    let Some((_, runtime)) = backends().await else {
        return;
    };
    let tenant_id = new_org(&runtime, 5, Some(1)).await;
    let mut wider = OrgPolicy::conservative_default();
    wider.member_automatic = OrgMemberAutomatic::Allowed;
    let outcome = runtime
        .update_org_policy(&tenant_id, 1, &wider, "account-actor:admin")
        .await
        .unwrap();
    let OrgPolicyUpdateOutcome::Updated {
        policy_version,
        policy_digest,
        widened,
    } = outcome
    else {
        panic!("{outcome:?}");
    };
    assert_eq!(policy_version, 2);
    assert_eq!(policy_digest, wider.digest());
    assert!(widened);

    // A stale version is refused and changes nothing.
    let mut off = wider.clone();
    off.sharing = OrgSharing::Off;
    assert_eq!(
        runtime
            .update_org_policy(&tenant_id, 1, &off, "account-actor:admin")
            .await
            .unwrap(),
        OrgPolicyUpdateOutcome::VersionConflict { current_version: 2 }
    );
    match runtime
        .update_org_policy(&tenant_id, 2, &off, "account-actor:admin")
        .await
        .unwrap()
    {
        OrgPolicyUpdateOutcome::Updated { widened, .. } => assert!(!widened, "off narrows"),
        other => panic!("{other:?}"),
    }
    let org = runtime.get_org(&tenant_id).await.unwrap().unwrap();
    assert_eq!(org.policy.sharing, OrgSharing::Off);
    assert_eq!(org.policy_version, 3);
    let mirror = runtime
        .get_trace_tenant_policy(&tenant_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mirror.policy_version, "org-policy-v3");
    assert!(
        !mirror.allowed_uses.is_empty(),
        "sharing off keeps the lists: an empty list would mean no restriction"
    );

    let audit = runtime.list_org_audit(&tenant_id, 10).await.unwrap();
    let actions: Vec<&str> = audit.iter().map(|entry| entry.action.as_str()).collect();
    assert_eq!(
        actions,
        vec!["org_policy_changed", "org_policy_changed", "org_created"]
    );
    assert_eq!(audit[1].safe_metadata["widened"], true);
    // Hash-only metadata: no raw tenant id anywhere in it.
    for entry in &audit {
        assert!(!entry.safe_metadata.to_string().contains(&tenant_id));
    }
}

#[tokio::test]
async fn removing_a_seat_revokes_its_devices_and_keeps_the_last_admin() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let tenant_id = new_org(&runtime, 5, Some(1)).await;
    let admin_seat = admitted(&runtime, member(&tenant_id, OrgRole::OrgAdmin)).await;
    let member_seat = admitted(&runtime, member(&tenant_id, OrgRole::Member)).await;
    let device = device_key(&admin, &tenant_id).await;
    assert!(
        runtime
            .register_org_seat_device(&tenant_id, member_seat, &device, None)
            .await
            .unwrap()
    );

    assert_eq!(
        runtime
            .remove_org_seat(&tenant_id, admin_seat, "account-actor:admin")
            .await
            .unwrap(),
        OrgSeatRemoveOutcome::LastAdmin
    );
    let holder = runtime
        .get_org(&tenant_id)
        .await
        .unwrap()
        .unwrap()
        .holder_account_id
        .unwrap();
    assert_eq!(
        runtime
            .remove_org_seat(&tenant_id, holder, "account-actor:admin")
            .await
            .unwrap(),
        OrgSeatRemoveOutcome::HolderSeat
    );

    match runtime
        .remove_org_seat(&tenant_id, member_seat, "account-actor:admin")
        .await
        .unwrap()
    {
        OrgSeatRemoveOutcome::Removed {
            devices_revoked, ..
        } => assert_eq!(devices_revoked, 1),
        other => panic!("{other:?}"),
    }
    let key = runtime
        .get_device_key(&tenant_id, &device)
        .await
        .unwrap()
        .unwrap();
    assert!(key.revoked_at.is_some());
    let seat = runtime
        .get_org_seat(&tenant_id, member_seat)
        .await
        .unwrap()
        .unwrap();
    assert!(!seat.is_active());
    // Removed once; a second removal is an unknown seat, and the identity
    // cannot be admitted back.
    assert_eq!(
        runtime
            .remove_org_seat(&tenant_id, member_seat, "account-actor:admin")
            .await
            .unwrap(),
        OrgSeatRemoveOutcome::NotFound
    );
}

#[tokio::test]
async fn org_rows_are_tenant_isolated() {
    let Some((_, runtime)) = backends().await else {
        return;
    };
    let a = new_org(&runtime, 5, Some(1)).await;
    let b = new_org(&runtime, 5, Some(1)).await;
    let seat_a = admitted(&runtime, member(&a, OrgRole::Member)).await;
    assert!(runtime.get_org_seat(&b, seat_a).await.unwrap().is_none());
    assert!(
        runtime
            .remove_org_seat(&b, seat_a, "account-actor:b")
            .await
            .map(|outcome| outcome == OrgSeatRemoveOutcome::NotFound)
            .unwrap()
    );
    assert!(runtime.list_org_seats(&b).await.unwrap().is_empty());
}
