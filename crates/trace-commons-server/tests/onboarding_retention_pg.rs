// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
//! Opt-in isolated PostgreSQL validation of migration privilege and expiry scope.
use trace_commons_server::{
    config::{DatabaseConfig, SslMode},
    db::{Database, postgres::PgBackend},
};

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ONBOARDING_RETENTION_PG_TEST_URL"]
async fn restricted_migrator_loses_guard_membership_and_retention_preserves_replay() {
    let url = std::env::var("TRACE_COMMONS_ONBOARDING_RETENTION_PG_TEST_URL")
        .expect("isolated URL required");
    let mut parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test"));
    let cfg = |url: String| DatabaseConfig {
        url: url.into(),
        pool_size: 2,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    let admin = PgBackend::new(&cfg(url)).await.unwrap();
    admin.run_migrations().await.unwrap();
    let client = admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    client.batch_execute("DO $$ BEGIN
      IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='admission_retention_migrator') THEN CREATE ROLE admission_retention_migrator LOGIN NOSUPERUSER NOBYPASSRLS; END IF;
      IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='admission_retention_runtime') THEN CREATE ROLE admission_retention_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF;
    END $$;
    GRANT USAGE,CREATE ON SCHEMA public TO admission_retention_migrator;
    GRANT trace_admission_guard,trace_onboarding_retention_guard TO admission_retention_migrator WITH ADMIN OPTION;
    ALTER TABLE trace_near_provisioning_ceremonies OWNER TO admission_retention_migrator;
    ALTER TABLE trace_admission_challenges OWNER TO admission_retention_migrator;
    ALTER TABLE trace_admission_accounts OWNER TO admission_retention_migrator;
    ALTER TABLE trace_admission_submissions OWNER TO admission_retention_migrator;
    ALTER TABLE trace_admission_receipts OWNER TO admission_retention_migrator;
    ALTER TABLE trace_admission_global_budget OWNER TO admission_retention_migrator;").await.unwrap();
    parsed.set_username("admission_retention_migrator").unwrap();
    let migrator = PgBackend::new(&cfg(parsed.to_string())).await.unwrap();
    let migration = migrator
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    migration
        .batch_execute(include_str!(
            "../../../migrations/V59__trace_admission_ledger.sql"
        ))
        .await
        .unwrap();
    migration
        .batch_execute(include_str!(
            "../../../migrations/V60__onboarding_retention.sql"
        ))
        .await
        .unwrap();
    let membership: bool = migration.query_one("SELECT pg_has_role(current_user,'trace_admission_guard','MEMBER') OR pg_has_role(current_user,'trace_onboarding_retention_guard','MEMBER')",&[]).await.unwrap().get(0);
    assert!(!membership, "migration must not retain either definer role");
    // These grants must succeed AFTER membership revocation: explicit retained
    // EXECUTE WITH GRANT OPTION, not inherited function-owner authority.
    migration.batch_execute("GRANT USAGE ON SCHEMA public TO admission_retention_runtime;
    GRANT SELECT,INSERT,UPDATE ON trace_admission_challenges,trace_admission_accounts,trace_admission_submissions TO admission_retention_runtime;
    GRANT EXECUTE ON FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT),trace_transition_admission(TEXT,UUID,UUID,TEXT),trace_prune_onboarding_expiry(TEXT,INTEGER,BOOLEAN) TO admission_retention_runtime;").await.unwrap();
    client.batch_execute("TRUNCATE trace_near_provisioning_ceremonies,trace_admission_challenges,trace_admission_accounts,trace_admission_submissions,trace_admission_receipts,trace_admission_global_budget;
    INSERT INTO trace_near_provisioning_ceremonies(ceremony_hash,payload,expires_at) VALUES
      ('sha256:'||repeat('a',64),'{}',now()-interval '1 hour'),('sha256:'||repeat('b',64),'{}',now()+interval '1 hour');
    INSERT INTO trace_admission_challenges(tenant_id,anchor_hash,challenge_hash,expires_at,consumed_by) VALUES
      ('alpha',repeat('a',64),repeat('c',64),now()-interval '1 hour',NULL),
      ('alpha',repeat('a',64),repeat('d',64),now()-interval '1 hour','00000000-0000-0000-0000-000000000001'),
      ('beta',repeat('b',64),repeat('e',64),now()-interval '1 hour',NULL),
      ('alpha',repeat('a',64),repeat('f',64),now()+interval '1 hour',NULL);
    INSERT INTO trace_admission_receipts VALUES(repeat('f',64));
    INSERT INTO trace_admission_global_budget VALUES(TRUE,100,10);
    INSERT INTO trace_admission_accounts VALUES('alpha',repeat('a',64),1,100,1,10);
    INSERT INTO trace_admission_submissions VALUES('alpha','00000000-0000-0000-0000-000000000001',repeat('a',64),repeat('b',64),'attested',repeat('f',64),repeat('d',64),'completed','00000000-0000-0000-0000-000000000002',now()-interval '1 hour',10,FALSE,TRUE);").await.unwrap();
    parsed.set_username("admission_retention_runtime").unwrap();
    let runtime = PgBackend::new(&cfg(parsed.to_string())).await.unwrap();
    assert!(runtime.admission_runtime_ready().await.unwrap());
    assert_eq!(
        runtime
            .prune_onboarding_expiry("alpha", 1, true)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        runtime
            .prune_onboarding_expiry("alpha", 1, false)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        runtime
            .prune_onboarding_expiry("alpha", 1000, false)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        runtime
            .prune_onboarding_expiry("alpha", 1000, false)
            .await
            .unwrap(),
        0
    );
    assert!(
        runtime
            .prune_onboarding_expiry("alpha", 1001, false)
            .await
            .is_err()
    );
    let row=client.query_one("SELECT (SELECT count(*) FROM trace_near_provisioning_ceremonies), (SELECT count(*) FROM trace_admission_challenges),(SELECT count(*) FROM trace_admission_receipts),(SELECT cost_bound_used FROM trace_admission_accounts),(SELECT cost_bound_used FROM trace_admission_global_budget),(SELECT count(*) FROM trace_admission_submissions)",&[]).await.unwrap();
    assert_eq!(
        (0..6).map(|i| row.get::<_, i64>(i)).collect::<Vec<_>>(),
        vec![1, 2, 1, 10, 10, 1]
    );
    assert!(
        runtime
            .lookup_completed_submission_admission(
                "alpha",
                &"a".repeat(64),
                uuid::Uuid::from_u128(1),
                &"b".repeat(64)
            )
            .await
            .unwrap()
    );
}
