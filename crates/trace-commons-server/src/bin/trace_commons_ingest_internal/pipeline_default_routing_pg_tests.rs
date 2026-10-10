// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pipeline default routing (spec 2026-10-10): the operator-armed mode that
//! routes every tenant with no routing row to the versioned pipeline, through
//! the same qualification and activation the admin routes use.
//!
//! Every test runs over a real PostgreSQL as the suite's runtime login
//! (`runtime_backend`: `NOSUPERUSER`, `NOBYPASSRLS`, a member of
//! `trace_ingest_runtime` only), so the row-level security and the grants of
//! the V124 enumeration are the ones the pilot's ingest runs under.
//!
//! Nested inside `tests` beside `pipeline_activation_pg_tests`, whose
//! `pub(super)` route fixture it reuses.

use super::*;


use super::pipeline_http_pg_tests::{
    account_owner_backend, runtime_backend, tenant_tx, write_routing_as_operator,
};
use trace_commons_server::versioned_pipeline_activation::{PipelineActivationStore, RoutingState};

/// Inserts `tenant_id` into `trace_tenants` as the database owner: a tenant
/// that signed up and has not uploaded yet.
async fn seed_tenant(owner: &PgBackend, tenant_id: &str) {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    tx.execute(
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[&tenant_id],
    )
    .await
    .expect("seed the tenant");
    tx.commit().await.unwrap();
}

/// Every unrouted tenant whose id starts with `prefix`, read in pages of
/// `page` from the cursor `prefix` (every id with the prefix sorts after it).
async fn unrouted_with_prefix(
    store: &PipelineActivationStore,
    prefix: &str,
    page: i64,
) -> Vec<String> {
    let mut found = Vec::new();
    let mut after = prefix.to_string();
    loop {
        let rows = store
            .unrouted_tenants(Some(&after), page)
            .await
            .expect("the unrouted page reads as the runtime role");
        let Some(last) = rows.last().cloned() else {
            return found;
        };
        for tenant_id in rows {
            if !tenant_id.starts_with(prefix) {
                return found;
            }
            found.push(tenant_id);
        }
        after = last;
    }
}

/// Spec test 8: both V124 functions answer the non-superuser runtime role,
/// across tenants, with tenant ids (and the routing state) only, in byte
/// order from a cursor; and the runtime role still cannot read
/// `trace_tenants` across tenants itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_enumeration_works_as_the_runtime_role_and_returns_ids_only() {
    let Some(runtime) = runtime_backend(2).await else {
        return;
    };
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let prefix = format!("tenant-defroute-enum-{}-", Uuid::new_v4().simple());
    let unrouted_a = format!("{prefix}a");
    let unrouted_b = format!("{prefix}b");
    let contained = format!("{prefix}c");
    let legacy = format!("{prefix}l");
    let pipeline = format!("{prefix}p");
    seed_tenant(&owner, &unrouted_a).await;
    seed_tenant(&owner, &unrouted_b).await;
    write_routing_as_operator(&contained, "contained").await;
    write_routing_as_operator(&legacy, "legacy").await;
    write_routing_as_operator(&pipeline, "pipeline").await;

    let store = PipelineActivationStore::new(runtime.clone());
    assert!(
        store
            .default_routing_enumeration_ready()
            .await
            .expect("the privilege check reads"),
        "the runtime role may execute both V124 functions"
    );
    // Pages of one prove the cursor; a page of 1000 the bound.
    for page in [1, 1000] {
        assert_eq!(
            unrouted_with_prefix(&store, &prefix, page).await,
            vec![unrouted_a.clone(), unrouted_b.clone()],
            "only the tenants with no routing row, in byte order (page {page})"
        );
    }
    let mut routed = Vec::new();
    let mut after = prefix.clone();
    loop {
        let rows = store
            .routed_tenants(Some(&after), 1)
            .await
            .expect("the routed page reads as the runtime role");
        let Some((last, _)) = rows.last().cloned() else {
            break;
        };
        if !last.starts_with(&prefix) {
            break;
        }
        routed.extend(rows);
        after = last;
    }
    assert_eq!(
        routed,
        vec![
            (contained.clone(), RoutingState::Contained),
            (pipeline.clone(), RoutingState::Pipeline),
        ],
        "the pipeline and contained rows, never a legacy one"
    );
    // A limit outside the bound is clamped, never an error.
    assert!(store.unrouted_tenants(None, -5).await.unwrap().is_empty());

    // The functions return the declared columns and nothing else.
    let client = runtime.trace_pool_for_test().get().await.unwrap();
    let columns = |sql: &'static str| {
        let client = &client;
        async move {
            client
                .prepare(sql)
                .await
                .expect("prepare as the runtime role")
                .columns()
                .iter()
                .map(|column| column.name().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        columns("SELECT * FROM trace_pipeline_unrouted_tenants(NULL, 1)").await,
        vec!["tenant_id"]
    );
    assert_eq!(
        columns("SELECT * FROM trace_pipeline_routed_tenants(NULL, 1)").await,
        vec!["tenant_id", "routing_state"]
    );

    // Without a tenant context the runtime role reads no tenant directly:
    // the tenant policy either refuses or matches nothing.
    let direct = client
        .query_one(
            "SELECT COUNT(*) FROM trace_tenants WHERE tenant_id LIKE $1 || '%'",
            &[&prefix],
        )
        .await;
    if let Ok(row) = direct {
        assert_eq!(row.get::<_, i64>(0), 0, "no cross-tenant read");
    }
    let direct_routing = client
        .query_one(
            "SELECT COUNT(*) FROM pipeline_tenant_routing WHERE tenant_id LIKE $1 || '%'",
            &[&prefix],
        )
        .await;
    if let Ok(row) = direct_routing {
        assert_eq!(row.get::<_, i64>(0), 0, "no cross-tenant read");
    }

    // The owner of the functions cannot log in or bypass row-level
    // security, and the runtime role is not a member of it.
    let owner_client = owner.trace_pool_for_test().get().await.unwrap();
    let guard = owner_client
        .query_one(
            "SELECT rolcanlogin, rolbypassrls, rolsuper,
                    pg_has_role($1, 'trace_pipeline_routing_enumeration_guard', 'USAGE')
               FROM pg_roles WHERE rolname = 'trace_pipeline_routing_enumeration_guard'",
            &[&super::pipeline_http_pg_tests::PIPELINE_HTTP_RUNTIME_ROLE],
        )
        .await
        .expect("the guard role exists");
    assert!(!guard.get::<_, bool>(0), "the guard cannot log in");
    assert!(!guard.get::<_, bool>(1), "the guard cannot bypass RLS");
    assert!(!guard.get::<_, bool>(2), "the guard is no superuser");
    assert!(
        !guard.get::<_, bool>(3),
        "the runtime role does not hold the guard's privileges"
    );
}
