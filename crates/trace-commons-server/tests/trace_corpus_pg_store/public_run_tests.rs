// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn public_runs_preserve_reviewed_versions_provenance_and_withdrawal() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("pg-public-run-{unique}");
    let source_slug = format!("run-source-{}", &unique[..12]);
    let variation_slug = format!("run-variation-{}", &unique[..12]);
    let principal_ref = "principal:public-run-test";
    let source_submission_id = Uuid::new_v4();
    let variation_submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, source_submission_id))
        .await
        .expect("insert source submission");
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, variation_submission_id))
        .await
        .expect("insert variation submission");
    let account_id = backend
        .create_or_reuse_account(&tenant_id, principal_ref)
        .await
        .expect("create source account");

    let source_publication_id = Uuid::new_v4();
    let source = backend
        .upsert_public_run(PublicRunWrite {
            tenant_id: tenant_id.clone(),
            publication_id: source_publication_id,
            account_id,
            submission_id: source_submission_id,
            slug: source_slug.clone(),
            title: "Source workflow".to_string(),
            outcome_summary: "The source completed.".to_string(),
            correction_excerpt: None,
            workflow: "Run the source steps.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::new_v4(),
                excerpt: "The source completed successfully.".to_string(),
            }],
            task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            approval_sha256: format!("sha256:{}", "a".repeat(64)),
            source_publication_id: None,
            expected_publication_version: 0,
        })
        .await
        .expect("publish source");
    assert_eq!(source.row.version, 1);

    let updated = backend
        .upsert_public_run(PublicRunWrite {
            tenant_id: tenant_id.clone(),
            publication_id: Uuid::new_v4(),
            account_id,
            submission_id: source_submission_id,
            slug: "unused-replacement-slug".to_string(),
            title: "Source workflow revised".to_string(),
            outcome_summary: "The source completed with a bounded retry.".to_string(),
            correction_excerpt: Some("Use one bounded retry.".to_string()),
            workflow: "Run the source steps, then retry once.".to_string(),
            reuse_permission: PublicRunReusePermission::Cc0_10,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::new_v4(),
                excerpt: "The bounded retry completed.".to_string(),
            }],
            task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Partial,
            contributed_version: "trace.contribution.v1".to_string(),
            approval_sha256: format!("sha256:{}", "b".repeat(64)),
            source_publication_id: None,
            expected_publication_version: 1,
        })
        .await
        .expect("update source");
    assert_eq!(updated.row.publication_id, source_publication_id);
    assert_eq!(updated.row.slug, source_slug);
    assert_eq!(updated.row.version, 2);
    assert_eq!(
        updated.row.reuse_permission,
        PublicRunReusePermission::Cc0_10
    );

    let variation_publication_id = Uuid::new_v4();
    backend
        .upsert_public_run(PublicRunWrite {
            tenant_id: tenant_id.clone(),
            publication_id: variation_publication_id,
            account_id,
            submission_id: variation_submission_id,
            slug: variation_slug.clone(),
            title: "Source variation".to_string(),
            outcome_summary: "The variation completed.".to_string(),
            correction_excerpt: None,
            workflow: "Reuse the source with the varied input.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::new_v4(),
                excerpt: "The variation completed successfully.".to_string(),
            }],
            task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            approval_sha256: format!("sha256:{}", "c".repeat(64)),
            source_publication_id: Some(source_publication_id),
            expected_publication_version: 0,
        })
        .await
        .expect("publish variation");

    let cycle = backend
        .upsert_public_run(PublicRunWrite {
            tenant_id: tenant_id.clone(),
            publication_id: Uuid::new_v4(),
            account_id,
            submission_id: source_submission_id,
            slug: "unused-cycle-slug".to_string(),
            title: "Source workflow revised again".to_string(),
            outcome_summary: "The source completed with its variation.".to_string(),
            correction_excerpt: None,
            workflow: "Run the source steps with the variation.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::new_v4(),
                excerpt: "The source variation was observed.".to_string(),
            }],
            task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            approval_sha256: format!("sha256:{}", "d".repeat(64)),
            source_publication_id: Some(variation_publication_id),
            expected_publication_version: 2,
        })
        .await
        .expect_err("a source-to-variation update must reject the provenance cycle");
    assert!(matches!(
        cycle,
        DatabaseError::Constraint(ref message) if message == PUBLIC_RUN_PROVENANCE_CYCLE
    ));

    let public_source = backend
        .get_public_run_page_by_slug(&source_slug, 12)
        .await
        .expect("read public source")
        .expect("public source exists");
    assert_eq!(public_source.slug, source_slug);
    assert_eq!(public_source.variations.len(), 1);
    assert_eq!(public_source.variations[0].slug, variation_slug);
    let resolved_source = backend
        .resolve_public_run_source(&source_slug)
        .await
        .expect("resolve public source")
        .expect("resolved source exists");
    assert_eq!(resolved_source.publication_id, source_publication_id);

    let role_client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("get role verification connection");
    let role = role_client
        .query_one(
            "SELECT rolcanlogin, rolbypassrls,
                    EXISTS (
                        SELECT 1 FROM pg_auth_members AS membership
                        JOIN pg_roles AS granted ON granted.oid = membership.roleid
                        JOIN pg_roles AS grantee ON grantee.oid = membership.member
                        WHERE granted.rolname = 'trace_public_run_reader'
                          AND grantee.rolname = current_user
                    ) AS member,
                    has_table_privilege('trace_public_run_reader', 'trace_public_runs', 'INSERT') AS can_insert,
                    has_column_privilege('trace_public_run_reader', 'trace_public_runs', 'account_id', 'SELECT') AS can_read_account,
                    has_column_privilege('trace_public_run_reader', 'trace_public_runs', 'submission_id', 'SELECT') AS can_read_submission
             FROM pg_roles WHERE rolname = 'trace_public_run_reader'",
            &[],
        )
        .await
        .expect("read public-run role controls");
    assert!(!role.get::<_, bool>("rolcanlogin"));
    assert!(!role.get::<_, bool>("rolbypassrls"));
    assert!(!role.get::<_, bool>("member"));
    assert!(!role.get::<_, bool>("can_insert"));
    assert!(!role.get::<_, bool>("can_read_account"));
    assert!(!role.get::<_, bool>("can_read_submission"));

    let graph_role = role_client
        .query_one(
            "SELECT rolcanlogin, rolbypassrls,
                    EXISTS (
                        SELECT 1 FROM pg_auth_members AS membership
                        JOIN pg_roles AS granted ON granted.oid = membership.roleid
                        JOIN pg_roles AS grantee ON grantee.oid = membership.member
                        WHERE granted.rolname = 'trace_public_run_graph_guard'
                          AND grantee.rolname = current_user
                    ) AS member,
                    has_table_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'INSERT') AS can_insert,
                    has_table_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'UPDATE') AS can_update,
                    has_table_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'DELETE') AS can_delete,
                    has_column_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'publication_id', 'SELECT') AS can_read_publication,
                    has_column_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'source_publication_id', 'SELECT') AS can_read_source,
                    has_column_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'account_id', 'SELECT') AS can_read_account,
                    has_column_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'submission_id', 'SELECT') AS can_read_submission,
                    has_column_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'tenant_id', 'SELECT') AS can_read_tenant,
                    has_column_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'slug', 'SELECT') AS can_read_slug,
                    has_column_privilege('trace_public_run_graph_guard', 'trace_public_runs', 'title', 'SELECT') AS can_read_title,
                    has_function_privilege(
                        'trace_public_run_graph_guard',
                        'trace_public_run_would_cycle(uuid,uuid)',
                        'EXECUTE'
                    ) AS can_execute_guard,
                    has_function_privilege(
                        current_user,
                        'trace_public_run_retained_source(text,uuid,uuid)',
                        'EXECUTE'
                    ) AS runtime_can_execute_retained_source
             FROM pg_roles WHERE rolname = 'trace_public_run_graph_guard'",
            &[],
        )
        .await
        .expect("read public-run graph role controls");
    assert!(!graph_role.get::<_, bool>("rolcanlogin"));
    assert!(!graph_role.get::<_, bool>("rolbypassrls"));
    assert!(!graph_role.get::<_, bool>("member"));
    assert!(!graph_role.get::<_, bool>("can_insert"));
    assert!(!graph_role.get::<_, bool>("can_update"));
    assert!(!graph_role.get::<_, bool>("can_delete"));
    assert!(graph_role.get::<_, bool>("can_read_publication"));
    assert!(graph_role.get::<_, bool>("can_read_source"));
    assert!(graph_role.get::<_, bool>("can_read_account"));
    assert!(graph_role.get::<_, bool>("can_read_submission"));
    assert!(graph_role.get::<_, bool>("can_read_tenant"));
    assert!(graph_role.get::<_, bool>("can_read_slug"));
    assert!(!graph_role.get::<_, bool>("can_read_title"));
    assert!(graph_role.get::<_, bool>("can_execute_guard"));
    assert!(graph_role.get::<_, bool>("runtime_can_execute_retained_source"));

    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("get audit verification connection");
    let tx = client
        .transaction()
        .await
        .expect("start audit verification");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await
    .expect("set audit tenant context");
    let audit_rows = tx
        .query(
            "SELECT actor_ref, safe_metadata::text AS safe_metadata
             FROM trace_account_audit
             WHERE tenant_id = $1 AND action = 'public_run_publish'
             ORDER BY created_at",
            &[&tenant_id],
        )
        .await
        .expect("read publication audit rows");
    assert_eq!(audit_rows.len(), 3);
    for row in audit_rows {
        let actor_ref: String = row.get("actor_ref");
        let safe_metadata: String = row.get("safe_metadata");
        assert!(actor_ref.starts_with("account-actor:"));
        assert!(safe_metadata.contains("sha256:"));
        assert!(!safe_metadata.contains("workflow"));
        assert!(!safe_metadata.contains("completed"));
    }
    tx.commit().await.expect("commit audit verification");

    backend
        .record_trace_withdrawal(
            &tenant_id,
            source_submission_id,
            Utc::now(),
            "accepted",
            "commons_not_distributed",
        )
        .await
        .expect("withdraw source trace and public page atomically");
    assert!(
        backend
            .get_public_run_page_by_slug(&source_slug, 12)
            .await
            .expect("read withdrawn source")
            .is_none()
    );
    assert!(
        backend
            .resolve_public_run_source(&source_slug)
            .await
            .expect("resolve withdrawn source")
            .is_none()
    );
    let withdrawn_source = backend
        .get_owned_public_run_state(&tenant_id, account_id, source_submission_id)
        .await
        .expect("read withdrawn source owner state")
        .row
        .expect("withdrawn source owner state exists");
    assert_eq!(withdrawn_source.version, 3);
    assert!(withdrawn_source.unpublished_at.is_some());
    let surviving_variation = backend
        .get_public_run_page_by_slug(&variation_slug, 12)
        .await
        .expect("read surviving variation")
        .expect("surviving variation remains public");
    assert!(surviving_variation.source.is_none());
    assert!(surviving_variation.source_unavailable);

    cleanup_tenant(&backend, &tenant_id).await;
}

#[tokio::test]
async fn public_run_publish_and_withdraw_race_cannot_reopen_a_revoked_trace() {
    let Some(publisher) = postgres_backend().await else {
        return;
    };
    let Some(withdrawer) = postgres_backend().await else {
        return;
    };
    publisher.run_migrations().await.expect("run migrations");

    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("pg-public-run-race-{unique}");
    let submission_id = Uuid::new_v4();
    publisher
        .upsert_trace_submission(sample_submission(&tenant_id, submission_id))
        .await
        .expect("insert race source");
    let account_id = publisher
        .create_or_reuse_account(&tenant_id, "principal:public-run-race")
        .await
        .expect("create race account");
    let slug = format!("run-{}", &unique[..16]);
    let write = PublicRunWrite {
        tenant_id: tenant_id.clone(),
        publication_id: Uuid::new_v4(),
        account_id,
        submission_id,
        slug: slug.clone(),
        title: "Concurrent workflow".to_string(),
        outcome_summary: "The concurrent operation completed.".to_string(),
        correction_excerpt: None,
        workflow: "Publish while the source is being withdrawn.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: Uuid::new_v4(),
            excerpt: "The source was accepted before the race.".to_string(),
        }],
        task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
        contributed_version: "trace.contribution.v1".to_string(),
        approval_sha256: format!("sha256:{}", "d".repeat(64)),
        source_publication_id: None,
        expected_publication_version: 0,
    };
    let withdrawn_at = Utc::now();
    let (publish, withdrawal) = tokio::join!(
        publisher.upsert_public_run(write),
        withdrawer.record_trace_withdrawal(
            &tenant_id,
            submission_id,
            withdrawn_at,
            "accepted",
            "commons_not_distributed",
        )
    );
    withdrawal.expect("withdrawal wins or follows publication");
    if let Err(error) = publish {
        assert!(
            matches!(
                error,
                DatabaseError::Constraint(ref message)
                    if message == trace_commons_server::db::PUBLIC_RUN_SOURCE_NOT_ACCEPTED
            ),
            "the only permitted publish refusal is the revoked-source constraint: {error}"
        );
    }
    assert!(
        publisher
            .get_public_run_page_by_slug(&slug, 12)
            .await
            .expect("read race result")
            .is_none(),
        "no interleaving may leave the withdrawn source public"
    );

    cleanup_tenant(&publisher, &tenant_id).await;
}

/// These tests deliberately contend on the production-wide provenance lock.
/// Keep them out of each other's measurement window so the explicit lock holder
/// in one test cannot delay the operation exercised by the other.
fn provenance_graph_lock_test_guard() -> &'static tokio::sync::Mutex<()> {
    static GUARD: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    GUARD.get_or_init(Default::default)
}

#[tokio::test]
async fn root_publications_do_not_wait_for_the_provenance_graph_lock() {
    let _guard = provenance_graph_lock_test_guard().lock().await;
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("pg-public-run-root-lock-{unique}");
    let submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, submission_id))
        .await
        .expect("insert root publication source");
    let account_id = backend
        .create_or_reuse_account(&tenant_id, "principal:public-run-root-lock")
        .await
        .expect("create root publication account");

    let mut lock_client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("get provenance lock connection");
    let lock_tx = lock_client
        .transaction()
        .await
        .expect("start provenance lock transaction");
    lock_tx
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await
        .expect("set provenance lock tenant");
    let lock_class: i32 = 0x7075_6272;
    let lock_object: i32 = 0x756e_7072;
    lock_tx
        .execute(
            "SELECT pg_advisory_xact_lock($1, $2)",
            &[&lock_class, &lock_object],
        )
        .await
        .expect("hold provenance graph lock");

    let root = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        backend.upsert_public_run(PublicRunWrite {
            tenant_id: tenant_id.clone(),
            publication_id: Uuid::new_v4(),
            account_id,
            submission_id,
            slug: format!("run-root-{}", &unique[..12]),
            title: "Independent root workflow".to_string(),
            outcome_summary: "The independent root completed.".to_string(),
            correction_excerpt: None,
            workflow: "Run the independent root steps.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::new_v4(),
                excerpt: "The independent root completed successfully.".to_string(),
            }],
            task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            approval_sha256: format!("sha256:{}", "e".repeat(64)),
            source_publication_id: None,
            expected_publication_version: 0,
        }),
    )
    .await
    .expect("a root publication must not wait for the graph lock")
    .expect("publish independent root");
    assert_eq!(root.row.version, 1);

    lock_tx
        .rollback()
        .await
        .expect("release provenance graph lock");
    cleanup_tenant(&backend, &tenant_id).await;
}

#[tokio::test]
async fn sourced_publications_wait_for_the_provenance_graph_lock() {
    let _guard = provenance_graph_lock_test_guard().lock().await;
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("pg-public-run-source-lock-{unique}");
    let source_submission_id = Uuid::new_v4();
    let variation_submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, source_submission_id))
        .await
        .expect("insert source submission");
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, variation_submission_id))
        .await
        .expect("insert variation submission");
    let account_id = backend
        .create_or_reuse_account(&tenant_id, "principal:public-run-source-lock")
        .await
        .expect("create source lock account");
    let source_publication_id = Uuid::new_v4();
    backend
        .upsert_public_run(PublicRunWrite {
            tenant_id: tenant_id.clone(),
            publication_id: source_publication_id,
            account_id,
            submission_id: source_submission_id,
            slug: format!("run-lock-source-{}", &unique[..12]),
            title: "Source lock workflow".to_string(),
            outcome_summary: "The source lock workflow completed.".to_string(),
            correction_excerpt: None,
            workflow: "Run the source lock workflow.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::new_v4(),
                excerpt: "The source lock result was observed.".to_string(),
            }],
            task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            approval_sha256: format!("sha256:{}", "1".repeat(64)),
            source_publication_id: None,
            expected_publication_version: 0,
        })
        .await
        .expect("publish source lock root");

    let mut lock_client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("get provenance lock connection");
    let lock_tx = lock_client
        .transaction()
        .await
        .expect("start provenance lock transaction");
    let lock_class: i32 = 0x7075_6272;
    let lock_object: i32 = 0x756e_7072;
    lock_tx
        .execute(
            "SELECT pg_advisory_xact_lock($1, $2)",
            &[&lock_class, &lock_object],
        )
        .await
        .expect("hold provenance graph lock");

    let mut variation = Box::pin(backend.upsert_public_run(PublicRunWrite {
        tenant_id: tenant_id.clone(),
        publication_id: Uuid::new_v4(),
        account_id,
        submission_id: variation_submission_id,
        slug: format!("run-lock-variation-{}", &unique[..12]),
        title: "Locked source variation".to_string(),
        outcome_summary: "The source variation completed after serialization.".to_string(),
        correction_excerpt: None,
        workflow: "Wait for the provenance guard, then publish the variation.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: Uuid::new_v4(),
            excerpt: "The variation waited for the provenance guard.".to_string(),
        }],
        task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
        contributed_version: "trace.contribution.v1".to_string(),
        approval_sha256: format!("sha256:{}", "2".repeat(64)),
        source_publication_id: Some(source_publication_id),
        expected_publication_version: 0,
    }));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut variation)
            .await
            .is_err(),
        "a sourced publication must wait while the graph lock is held"
    );
    lock_tx
        .rollback()
        .await
        .expect("release provenance graph lock");
    let published = variation.await.expect("publish sourced variation");
    assert_eq!(
        published.row.source_publication_id,
        Some(source_publication_id)
    );

    let delayed_submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, delayed_submission_id))
        .await
        .expect("insert row-lock-delayed submission");
    let row_lock_tx = lock_client
        .transaction()
        .await
        .expect("start submission row-lock transaction");
    row_lock_tx
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await
        .expect("set row-lock tenant");
    row_lock_tx
        .query_one(
            "SELECT status FROM trace_submissions
             WHERE tenant_id = $1 AND submission_id = $2
             FOR UPDATE",
            &[&tenant_id, &delayed_submission_id],
        )
        .await
        .expect("hold delayed submission row lock");

    let mut delayed_variation = Box::pin(backend.upsert_public_run(PublicRunWrite {
        tenant_id: tenant_id.clone(),
        publication_id: Uuid::new_v4(),
        account_id,
        submission_id: delayed_submission_id,
        slug: format!("run-row-lock-variation-{}", &unique[..12]),
        title: "Row-lock-delayed variation".to_string(),
        outcome_summary: "The graph lock stayed available during a local row wait.".to_string(),
        correction_excerpt: None,
        workflow: "Wait for the submission row, then validate provenance.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: Uuid::new_v4(),
            excerpt: "The unrelated provenance graph stayed available.".to_string(),
        }],
        task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
        contributed_version: "trace.contribution.v1".to_string(),
        approval_sha256: format!("sha256:{}", "5".repeat(64)),
        source_publication_id: Some(source_publication_id),
        expected_publication_version: 0,
    }));
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            &mut delayed_variation,
        )
        .await
        .is_err(),
        "the sourced publication must wait for its submission row"
    );

    let row_lock_backend_pid: i32 = row_lock_tx
        .query_one("SELECT pg_backend_pid() AS pid", &[])
        .await
        .expect("read submission row-lock backend pid")
        .get("pid");
    let probe_client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("get blocked publication probe connection");
    let blocked_publications = probe_client
        .query(
            "SELECT pid
             FROM pg_stat_activity
             WHERE datname = current_database()
               AND $1 = ANY(pg_blocking_pids(pid))
               AND wait_event_type = 'Lock'",
            &[&row_lock_backend_pid],
        )
        .await
        .expect("identify publication blocked by submission row lock");
    assert_eq!(
        blocked_publications.len(),
        1,
        "the submission row lock must block exactly the publication under test"
    );
    let blocked_publication_pid: i32 = blocked_publications[0].get("pid");
    let lock_class = i64::from(lock_class);
    let lock_object = i64::from(lock_object);
    let blocked_publication_holds_graph_lock: bool = probe_client
        .query_one(
            "SELECT EXISTS (
                 SELECT 1
                 FROM pg_locks
                 WHERE pid = $1
                   AND locktype = 'advisory'
                   AND classid::bigint = $2
                   AND objid::bigint = $3
                   AND granted
             ) AS held",
            &[&blocked_publication_pid, &lock_class, &lock_object],
        )
        .await
        .expect("inspect blocked publication provenance lock ownership")
        .get("held");
    assert!(
        !blocked_publication_holds_graph_lock,
        "a local submission row wait must not retain the global provenance graph lock"
    );
    row_lock_tx
        .rollback()
        .await
        .expect("release delayed submission row lock");
    let delayed = delayed_variation
        .await
        .expect("publish row-lock-delayed variation");
    assert_eq!(
        delayed.row.source_publication_id,
        Some(source_publication_id)
    );

    cleanup_tenant(&backend, &tenant_id).await;
}

#[tokio::test]
async fn unpublish_advances_the_approval_version_and_blocks_stale_republish() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("pg-public-run-consent-{unique}");
    let submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, submission_id))
        .await
        .expect("insert consent-version submission");
    let account_id = backend
        .create_or_reuse_account(&tenant_id, "principal:public-run-consent")
        .await
        .expect("create consent-version account");
    let initial = PublicRunWrite {
        tenant_id: tenant_id.clone(),
        publication_id: Uuid::new_v4(),
        account_id,
        submission_id,
        slug: format!("run-consent-{}", &unique[..12]),
        title: "Consent-version workflow".to_string(),
        outcome_summary: "The initial approved workflow completed.".to_string(),
        correction_excerpt: None,
        workflow: "Publish the explicitly approved workflow.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: Uuid::new_v4(),
            excerpt: "The initial approved result was observed.".to_string(),
        }],
        task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
        contributed_version: "trace.contribution.v1".to_string(),
        approval_sha256: format!("sha256:{}", "6".repeat(64)),
        source_publication_id: None,
        expected_publication_version: 0,
    };
    let published = backend
        .upsert_public_run(initial.clone())
        .await
        .expect("publish consent-version workflow");
    assert_eq!(published.row.version, 1);

    let mut stale = initial.clone();
    stale.title = "Stale delayed workflow".to_string();
    stale.approval_sha256 = format!("sha256:{}", "7".repeat(64));
    stale.expected_publication_version = 1;
    let unpublish = backend
        .unpublish_public_run(&tenant_id, account_id, submission_id)
        .await
        .expect("unpublish consent-version workflow");
    assert!(unpublish.unpublished);
    assert_eq!(unpublish.expected_publication_version, 2);
    let tombstone = backend
        .get_owned_public_run_state(&tenant_id, account_id, submission_id)
        .await
        .expect("read unpublished owner state")
        .row
        .expect("unpublished owner state exists");
    assert_eq!(tombstone.version, 2);
    assert!(tombstone.unpublished_at.is_some());
    let inactive = backend
        .get_owned_public_run_state(&tenant_id, account_id, submission_id)
        .await
        .expect("read active owner state");
    assert!(inactive.page.is_none());

    let error = backend
        .upsert_public_run(stale)
        .await
        .expect_err("approval captured before unpublish must stay withdrawn");
    assert!(matches!(
        error,
        DatabaseError::Constraint(ref message) if message == PUBLIC_RUN_VERSION_CONFLICT
    ));
    assert!(
        backend
            .get_public_run_page_by_slug(&initial.slug, 12)
            .await
            .expect("read stale-republish result")
            .is_none()
    );

    let mut fresh = initial;
    fresh.title = "Freshly approved workflow".to_string();
    fresh.approval_sha256 = format!("sha256:{}", "8".repeat(64));
    fresh.expected_publication_version = 2;
    let republished = backend
        .upsert_public_run(fresh)
        .await
        .expect("republish after a fresh approval");
    assert_eq!(republished.row.version, 3);
    assert_eq!(republished.row.title, "Freshly approved workflow");

    cleanup_tenant(&backend, &tenant_id).await;
}

#[tokio::test]
async fn public_run_variation_limit_is_clamped_to_the_documented_bounds() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("pg-public-run-limit-{unique}");
    let principal_ref = "principal:public-run-limit";
    let source_submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, source_submission_id))
        .await
        .expect("insert variation limit source");
    let account_id = backend
        .create_or_reuse_account(&tenant_id, principal_ref)
        .await
        .expect("create variation limit account");
    let source_publication_id = Uuid::new_v4();
    let source_slug = format!("run-limit-source-{}", &unique[..12]);
    backend
        .upsert_public_run(PublicRunWrite {
            tenant_id: tenant_id.clone(),
            publication_id: source_publication_id,
            account_id,
            submission_id: source_submission_id,
            slug: source_slug.clone(),
            title: "Variation limit source".to_string(),
            outcome_summary: "The variation limit source completed.".to_string(),
            correction_excerpt: None,
            workflow: "Publish the variation limit source.".to_string(),
            reuse_permission: PublicRunReusePermission::CcBy40,
            evidence: vec![PublicRunEvidenceDraft {
                event_id: Uuid::new_v4(),
                excerpt: "The variation limit source was observed.".to_string(),
            }],
            task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
            contributed_version: "trace.contribution.v1".to_string(),
            approval_sha256: format!("sha256:{}", "3".repeat(64)),
            source_publication_id: None,
            expected_publication_version: 0,
        })
        .await
        .expect("publish variation limit source");

    for index in 0..21 {
        let submission_id = Uuid::new_v4();
        backend
            .upsert_trace_submission(sample_submission(&tenant_id, submission_id))
            .await
            .expect("insert bounded variation submission");
        backend
            .upsert_public_run(PublicRunWrite {
                tenant_id: tenant_id.clone(),
                publication_id: Uuid::new_v4(),
                account_id,
                submission_id,
                slug: format!("run-limit-{index}-{}", &unique[..12]),
                title: format!("Variation {index}"),
                outcome_summary: "The bounded variation completed.".to_string(),
                correction_excerpt: None,
                workflow: "Publish one bounded variation.".to_string(),
                reuse_permission: PublicRunReusePermission::CcBy40,
                evidence: vec![PublicRunEvidenceDraft {
                    event_id: Uuid::new_v4(),
                    excerpt: "The bounded variation was observed.".to_string(),
                }],
                task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
                contributed_version: "trace.contribution.v1".to_string(),
                approval_sha256: format!("sha256:{}", "4".repeat(64)),
                source_publication_id: Some(source_publication_id),
                expected_publication_version: 0,
            })
            .await
            .expect("publish bounded variation");
    }

    for (requested, expected) in [(-1, 1), (0, 1), (1, 1), (20, 20), (21, 20)] {
        let page = backend
            .get_public_run_page_by_slug(&source_slug, requested)
            .await
            .expect("read bounded variations")
            .expect("variation limit source remains public");
        assert_eq!(
            page.variations.len(),
            expected,
            "requested limit {requested}"
        );
    }

    cleanup_tenant(&backend, &tenant_id).await;
}

#[tokio::test]
async fn publication_waits_for_account_transition_and_rejects_a_closed_owner() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let unique = Uuid::new_v4().simple().to_string();
    let tenant_id = format!("pg-public-run-owner-race-{unique}");
    let submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, submission_id))
        .await
        .expect("insert account transition source");
    let account_id = backend
        .create_or_reuse_account(&tenant_id, "principal:public-run-owner-race")
        .await
        .expect("create account transition owner");
    let write = PublicRunWrite {
        tenant_id: tenant_id.clone(),
        publication_id: Uuid::new_v4(),
        account_id,
        submission_id,
        slug: format!("run-owner-{}", &unique[..12]),
        title: "Account transition workflow".to_string(),
        outcome_summary: "The account transition was serialized.".to_string(),
        correction_excerpt: None,
        workflow: "Wait for the account transition before publishing.".to_string(),
        reuse_permission: PublicRunReusePermission::CcBy40,
        evidence: vec![PublicRunEvidenceDraft {
            event_id: Uuid::new_v4(),
            excerpt: "The account was closed before the publication resumed.".to_string(),
        }],
        task_success: trace_commons_protocol::trace_contribution::TaskSuccess::Success,
        contributed_version: "trace.contribution.v1".to_string(),
        approval_sha256: format!("sha256:{}", "f".repeat(64)),
        source_publication_id: None,
        expected_publication_version: 0,
    };

    let mut transition_client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("get account transition connection");
    let transition = transition_client
        .transaction()
        .await
        .expect("start account transition");
    transition
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant_id],
        )
        .await
        .expect("set account transition tenant");
    transition
        .query_one(
            "SELECT account_id FROM trace_accounts
             WHERE tenant_id = $1 AND account_id = $2
             FOR UPDATE",
            &[&tenant_id, &account_id],
        )
        .await
        .expect("lock account transition owner");

    let mut publish = Box::pin(backend.upsert_public_run(write));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut publish)
            .await
            .is_err(),
        "publication must wait while the account owner is transitioning"
    );
    transition
        .execute(
            "UPDATE trace_accounts SET closed_at = now()
             WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant_id, &account_id],
        )
        .await
        .expect("close account owner");
    transition
        .commit()
        .await
        .expect("commit account transition");

    let error = publish
        .await
        .expect_err("publication must reject the closed account owner");
    assert!(matches!(
        error,
        DatabaseError::Constraint(ref message)
            if message == trace_commons_server::db::PUBLIC_RUN_ACCOUNT_CLOSED
    ));

    cleanup_tenant(&backend, &tenant_id).await;
}
