// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Exercise the installed trigger's account query under its actual definer
//! role and a generic prepared plan, with unrelated same-tenant data in the scan.

use serde_json::Value;
use trace_commons_server::account_trust::device_principal_ref;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use uuid::Uuid;

fn inspect_large_relation_access(plan: &Value, probe: &str, inspected: &mut usize) {
    let relation = plan.get("Relation Name").and_then(Value::as_str);
    if matches!(
        relation,
        Some("trace_account_trust_facts" | "trace_submissions")
    ) {
        *inspected += 1;
        let node = plan["Node Type"].as_str().unwrap();
        assert!(
            !node.contains("Seq Scan"),
            "{probe}: {relation:?} must use changed-key indexes"
        );
        let rows = plan["Actual Rows"].as_f64().unwrap_or(0.0);
        let removed = plan["Rows Removed by Filter"].as_f64().unwrap_or(0.0)
            + plan["Rows Removed by Index Recheck"]
                .as_f64()
                .unwrap_or(0.0);
        let loops = plan["Actual Loops"].as_f64().unwrap_or(0.0);
        assert!(
            (rows + removed) * loops <= 32.0,
            "{probe}: {relation:?} visited unrelated rows instead of bounded keys"
        );
    }
    if let Some(children) = plan.get("Plans").and_then(Value::as_array) {
        for child in children {
            inspect_large_relation_access(child, probe, inspected);
        }
    }
}

fn sql_literal(value: Option<&str>) -> String {
    value.map_or_else(
        || "NULL".into(),
        |value| format!("'{}'", value.replace('\'', "''")),
    )
}

#[tokio::test]
async fn installed_upload_frontier_query_uses_bounded_generic_index_probes() {
    let Ok(url) = std::env::var("TRACE_COMMONS_UPLOAD_TRUST_PLAN_PG_TEST_URL") else {
        eprintln!("SKIPPED: isolated TRACE_COMMONS_UPLOAD_TRUST_PLAN_PG_TEST_URL required");
        return;
    };
    let target = url.parse::<tokio_postgres::Config>().unwrap();
    assert!(
        matches!(target.get_hosts(),[tokio_postgres::config::Host::Tcp(host)] if host=="127.0.0.1")
    );
    assert!(
        target
            .get_dbname()
            .unwrap()
            .starts_with("admission_test_upload_trust_plan_")
    );
    let backend = PgBackend::new(&DatabaseConfig {
        url: url.into(),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    })
    .await
    .unwrap();
    backend.run_migrations().await.unwrap();
    let mut admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let tenant = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let noise = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let account = Uuid::new_v4();
    let noise_account = Uuid::new_v4();
    let device = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
    let parent = device_principal_ref(&tenant, &device);
    let alias = format!(
        "principal_sha256:{}",
        Uuid::new_v4().simple().to_string().repeat(2)
    );
    let alias_submission = Uuid::new_v4();
    let direct_submission = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1),($2)",
            &[&tenant, &noise],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2),($3,$4)",
            &[&tenant, &account, &noise, &noise_account],
        )
        .await
        .unwrap();
    admin.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)",&[&tenant,&account,&parent]).await.unwrap();
    admin.execute("INSERT INTO device_keys(tenant_id,device_key_id,public_key,onboarding_origin) VALUES($1,$2,'fixture','near_ai')",&[&tenant,&device]).await.unwrap();
    admin.execute("INSERT INTO trace_device_upload_principals(tenant_id,upload_principal_ref,device_principal_ref,device_key_id) VALUES($1,$2,$3,$4)",&[&tenant,&alias,&parent,&device]).await.unwrap();
    admin.execute("INSERT INTO trace_near_account_anchors(tenant_id,account_id,anchor_hash,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,'{}',$3,$3)",&[&tenant,&account,&device]).await.unwrap();
    admin.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,account_id,principal_ref,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$4)",&[&tenant,&account,&parent,&device]).await.unwrap();
    // Thousands of unrelated same-tenant facts make both a broad facts scan
    // and a tenant-only principal lookup observable, without timing thresholds
    // or disabling sequential scans in the planner.
    admin.execute("INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,redaction_pipeline_version,redaction_hash)
        SELECT $1,md5($1::text||':submission:'||g::text)::uuid,md5($1::text||':trace:'||g::text)::uuid,
        'principal_sha256:'||repeat(md5($1::text||g::text),2),'v1','v1','fixture','accepted','low','fixture',repeat('a',64)
        FROM generate_series(1,4096) g",&[&tenant]).await.unwrap();
    for (submission, principal) in [
        (alias_submission, alias.as_str()),
        (direct_submission, parent.as_str()),
    ] {
        admin.execute("INSERT INTO trace_submissions(tenant_id,submission_id,trace_id,auth_principal_ref,schema_version,consent_policy_version,retention_policy_id,status,privacy_risk,redaction_pipeline_version,redaction_hash) VALUES($1,$2,$3,$4,'v1','v1','fixture','accepted','low','fixture',$5)",&[&tenant,&submission,&Uuid::new_v4(),&principal,&"a".repeat(64)]).await.unwrap();
    }
    admin.execute("INSERT INTO trace_account_trust_facts(tenant_id,account_id,source_kind,source_id,submission_id,outcome,occurred_at)
        SELECT tenant_id,CASE WHEN tenant_id=$1 THEN $2::uuid ELSE $3::uuid END,'accepted_submission',submission_id,submission_id,'accepted',received_at
        FROM trace_submissions WHERE tenant_id IN ($1,$4)",&[&tenant,&account,&noise_account,&noise]).await.unwrap();
    admin.batch_execute("ANALYZE trace_account_trust_facts; ANALYZE trace_submissions; ANALYZE trace_device_upload_principals; ANALYZE trace_near_provisioned_devices; ANALYZE trace_account_principals").await.unwrap();
    let definition:String=admin.query_one("SELECT pg_get_functiondef('trace_account_trust_advance_upload_frontiers()'::regprocedure)",&[]).await.unwrap().get(0);
    let query = definition
        .split_once("FOR v_account IN")
        .unwrap()
        .1
        .split_once("\n    LOOP")
        .unwrap()
        .0;
    let mut query = query.to_string();
    for (name, parameter) in [
        ("v_old_tenant", "$1"),
        ("v_new_tenant", "$2"),
        ("v_old_submission", "$3"),
        ("v_new_submission", "$4"),
        ("v_old_principal", "$5"),
        ("v_new_principal", "$6"),
        ("v_old_device", "$7"),
        ("v_new_device", "$8"),
        ("v_old_parent", "$9"),
        ("v_new_parent", "$10"),
    ] {
        query = query.replace(name, parameter);
    }
    let tx = admin.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE trace_account_trust_input_guard; SET LOCAL plan_cache_mode=force_generic_plan").await.unwrap();
    tx.batch_execute(&format!("PREPARE upload_frontier_probe(TEXT,TEXT,UUID,UUID,TEXT,TEXT,TEXT,TEXT,TEXT,TEXT) AS {query}")).await.unwrap();
    let submission = alias_submission.to_string();
    for (probe, values) in [
        (
            "submission",
            [
                Some(tenant.as_str()),
                Some(tenant.as_str()),
                Some(submission.as_str()),
                Some(submission.as_str()),
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        ),
        (
            "alias",
            [
                Some(tenant.as_str()),
                Some(tenant.as_str()),
                None,
                None,
                Some(alias.as_str()),
                Some(alias.as_str()),
                None,
                None,
                None,
                None,
            ],
        ),
        (
            "device",
            [
                Some(tenant.as_str()),
                Some(tenant.as_str()),
                None,
                None,
                None,
                None,
                Some(device.as_str()),
                Some(device.as_str()),
                None,
                None,
            ],
        ),
        (
            "parent",
            [
                Some(tenant.as_str()),
                Some(tenant.as_str()),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(parent.as_str()),
                Some(parent.as_str()),
            ],
        ),
    ] {
        let arguments = values
            .into_iter()
            .map(sql_literal)
            .collect::<Vec<_>>()
            .join(",");
        let execute = format!("EXECUTE upload_frontier_probe({arguments})");
        let rows = tx.query(&execute, &[]).await.unwrap();
        assert_eq!(rows.len(), 1, "{probe} discovers only the affected account");
        assert_eq!(rows[0].get::<_, String>(0), tenant);
        assert_eq!(rows[0].get::<_, Uuid>(1), account);
        let plan: Value = tx
            .query_one(
                &format!("EXPLAIN (ANALYZE,BUFFERS,FORMAT JSON) {execute}"),
                &[],
            )
            .await
            .unwrap()
            .get(0);
        let mut inspected = 0;
        inspect_large_relation_access(&plan[0]["Plan"], probe, &mut inspected);
        assert!(
            inspected > 0,
            "{probe} explains the installed fact/submission probes"
        );
    }
    tx.batch_execute("DEALLOCATE upload_frontier_probe")
        .await
        .unwrap();
    tx.commit().await.unwrap();
    backend
        .enable_external_account_trust_growth()
        .await
        .unwrap();
    let before=admin.query("SELECT tenant_id,generation FROM trace_account_trust_frontiers WHERE tenant_id IN ($1,$2) ORDER BY tenant_id",&[&tenant,&noise]).await.unwrap();
    admin.execute("UPDATE trace_submissions SET status='quarantined' WHERE tenant_id=$1 AND submission_id=$2",&[&tenant,&alias_submission]).await.unwrap();
    let after=admin.query("SELECT tenant_id,generation FROM trace_account_trust_frontiers WHERE tenant_id IN ($1,$2) ORDER BY tenant_id",&[&tenant,&noise]).await.unwrap();
    for (before, after) in before.iter().zip(&after) {
        let affected = before.get::<_, String>(0) == tenant;
        assert_eq!(
            after.get::<_, i64>(1),
            before.get::<_, i64>(1) + i64::from(affected),
            "one distinct frontier update for the changed account; no unrelated tenant invalidation"
        );
    }
}
