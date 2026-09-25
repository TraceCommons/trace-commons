// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use secrecy::SecretString;
use std::sync::Arc;
use trace_commons_protocol::inference_connection::{
    ConnectionWitnessConfig, DISCLOSURE_VERSION, SelectInferenceConnection,
};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::Database;
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::db::postgres_inference_connection::{
    InferenceDisconnectOutcome as Disconnect, InferenceSelectionOutcome as Selection,
};
use trace_commons_server::inference_connection::OperatorInferenceConnection;
use uuid::Uuid;

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

fn catalog(name: &str) -> OperatorInferenceConnection {
    OperatorInferenceConnection::new(
        name.into(),
        "near-ai".into(),
        DISCLOSURE_VERSION,
        ConnectionWitnessConfig {
            url: format!("https://private-witness.example/{name}"),
            signing_address: format!("0x{}", "ab".repeat(20)),
            expected_measurements: vec![format!("mrtd={}", "ab".repeat(48))],
        },
        Some("https://private-receipt.example/v1".into()),
    )
    .unwrap()
}

fn request(
    catalog: &OperatorInferenceConnection,
    expected: Option<i64>,
    key: Uuid,
) -> SelectInferenceConnection {
    let offer = catalog.offer();
    SelectInferenceConnection {
        offer_id: offer.offer_id,
        revision: offer.revision,
        config_digest: offer.config_digest,
        disclosure_version: offer.disclosure_version,
        idempotency_key: key,
        expected_current_version: expected,
    }
}

async fn seed(admin: &deadpool_postgres::Object, tenant: &str, account: Uuid, anchor: bool) {
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await
        .unwrap();
    admin
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    if anchor {
        let hash = format!("sha256:{}", Uuid::new_v4().simple().to_string().repeat(2));
        admin.execute("INSERT INTO trace_near_account_anchors
            (tenant_id, account_id, anchor_hash, sealed_account_name, index_pepper_ref, account_name_key_ref)
            VALUES ($1, $2, $3, $4, 'fixture-pepper', 'fixture-key')",
            &[&tenant, &account, &hash, &serde_json::json!({"fixture":true})]).await.unwrap();
    }
}

#[tokio::test]
async fn selection_is_account_scoped_versioned_idempotent_and_rls_forced() {
    let Ok(url) = std::env::var("TRACE_COMMONS_INFERENCE_CONNECTION_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIPPED: isolated inference connection PostgreSQL fixture required");
        return;
    };
    assert!(url.contains("127.0.0.1/trace_inference_connection_z8_test"));
    let admin_db = PgBackend::new(&config(url.clone())).await.unwrap();
    admin_db.run_migrations().await.unwrap();
    let admin = admin_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    admin.batch_execute("DO $$ BEGIN IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'tc_inference_connection_z8_runtime') THEN CREATE ROLE tc_inference_connection_z8_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$; GRANT trace_inference_connection_runtime TO tc_inference_connection_z8_runtime;").await.unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url
        .set_username("tc_inference_connection_z8_runtime")
        .unwrap();
    let runtime = Arc::new(PgBackend::new(&config(runtime_url.into())).await.unwrap());
    let tenant = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let other_tenant = format!("near-{}", Uuid::new_v4().simple().to_string().repeat(2));
    let account = Uuid::new_v4();
    let other_account = Uuid::new_v4();
    let unanchored = Uuid::new_v4();
    let closed = Uuid::new_v4();
    seed(&admin, &tenant, account, true).await;
    seed(&admin, &tenant, other_account, true).await;
    seed(&admin, &tenant, unanchored, false).await;
    seed(&admin, &tenant, closed, true).await;
    seed(&admin, &other_tenant, Uuid::new_v4(), true).await;
    admin
        .execute(
            "UPDATE trace_accounts SET closed_at = now() WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &closed],
        )
        .await
        .unwrap();

    let offer = catalog("first");
    let initial = request(&offer, None, Uuid::new_v4());
    assert_eq!(
        runtime
            .current_inference_connection(&tenant, account, std::slice::from_ref(&offer))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        runtime
            .select_inference_connection(&tenant, account, &initial, &catalog("different"))
            .await
            .unwrap(),
        Selection::InvalidSelection
    );
    assert_eq!(
        runtime
            .select_inference_connection(&tenant, unanchored, &initial, &offer)
            .await
            .unwrap(),
        Selection::AccountIneligible
    );
    assert_eq!(
        runtime
            .select_inference_connection(&tenant, closed, &initial, &offer)
            .await
            .unwrap(),
        Selection::AccountIneligible
    );
    assert_eq!(
        runtime
            .select_inference_connection(&other_tenant, account, &initial, &offer)
            .await
            .unwrap(),
        Selection::AccountIneligible
    );
    let (left, right) = tokio::join!(
        runtime.select_inference_connection(&tenant, account, &initial, &offer),
        runtime.select_inference_connection(&tenant, account, &initial, &offer),
    );
    let Selection::Selected(first) = left.unwrap() else {
        panic!("first selection failed")
    };
    assert_eq!(right.unwrap(), Selection::Selected(first.clone()));
    assert_eq!(first.state_version, 1);
    assert_eq!(
        runtime
            .current_inference_connection(&tenant, other_account, std::slice::from_ref(&offer))
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        runtime
            .disconnect_inference_connection(&tenant, other_account, first.connection_id)
            .await
            .unwrap(),
        Disconnect::NotFound
    );
    let mut changed = initial.clone();
    changed.expected_current_version = Some(1);
    assert_eq!(
        runtime
            .select_inference_connection(&tenant, account, &changed, &offer)
            .await
            .unwrap(),
        Selection::IdempotencyConflict
    );
    let mut stale = request(&offer, Some(1), Uuid::new_v4());
    stale.revision = format!("sha256:{}", "0".repeat(64));
    assert_eq!(
        runtime
            .select_inference_connection(&tenant, account, &stale, &offer)
            .await
            .unwrap(),
        Selection::ReselectionRequired
    );
    assert_eq!(
        runtime
            .select_inference_connection(
                &tenant,
                account,
                &request(&offer, None, Uuid::new_v4()),
                &offer
            )
            .await
            .unwrap(),
        Selection::VersionConflict
    );
    let retired = runtime
        .current_inference_connection(&tenant, account, &[])
        .await
        .unwrap()
        .unwrap();
    assert!(retired.reselection_required);
    let rotated = OperatorInferenceConnection::new(
        "first".into(),
        "near-ai".into(),
        DISCLOSURE_VERSION,
        ConnectionWitnessConfig {
            url: "https://private-witness.example/rotated".into(),
            signing_address: format!("0x{}", "ab".repeat(20)),
            expected_measurements: vec![format!("mrtd={}", "ab".repeat(48))],
        },
        Some("https://private-receipt.example/v1".into()),
    )
    .unwrap();
    assert_eq!(
        runtime
            .select_inference_connection(&tenant, account, &initial, &rotated)
            .await
            .unwrap(),
        Selection::ReselectionRequired
    );
    assert!(
        !runtime
            .current_inference_connection(&tenant, account, std::slice::from_ref(&offer))
            .await
            .unwrap()
            .unwrap()
            .reselection_required
    );

    let second = catalog("second");
    let third = catalog("third");
    let a = request(&second, Some(1), Uuid::new_v4());
    let b = request(&third, Some(1), Uuid::new_v4());
    let (left, right) = tokio::join!(
        runtime.select_inference_connection(&tenant, account, &a, &second),
        runtime.select_inference_connection(&tenant, account, &b, &third),
    );
    let results = [left.unwrap(), right.unwrap()];
    assert_eq!(
        results
            .iter()
            .filter(|outcome| matches!(outcome, Selection::Selected(_)))
            .count(),
        1
    );
    assert!(results.contains(&Selection::VersionConflict));
    let Selection::Selected(current) = results
        .iter()
        .find(|outcome| matches!(outcome, Selection::Selected(_)))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(current.state_version, 2);
    assert_eq!(
        runtime
            .disconnect_inference_connection(&tenant, account, current.connection_id)
            .await
            .unwrap(),
        Disconnect::Revoked { state_version: 3 }
    );
    assert_eq!(
        runtime
            .disconnect_inference_connection(&tenant, account, current.connection_id)
            .await
            .unwrap(),
        Disconnect::Revoked { state_version: 3 }
    );
    assert_eq!(
        runtime
            .current_inference_connection(&tenant, account, &[second, third])
            .await
            .unwrap(),
        None
    );
    let resumed = catalog("resumed");
    let Selection::Selected(resumed_selection) = runtime
        .select_inference_connection(
            &tenant,
            account,
            &request(&resumed, None, Uuid::new_v4()),
            &resumed,
        )
        .await
        .unwrap()
    else {
        panic!("selection after disconnect failed")
    };
    assert_eq!(resumed_selection.state_version, 4);
    drop(runtime);
    let restarted = PgBackend::new(&config(url.clone())).await.unwrap();
    let restored = restarted
        .current_inference_connection(&tenant, account, &[resumed])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(restored.connection_id, resumed_selection.connection_id);
    assert!(!restored.reselection_required);

    let flags = admin.query_one("SELECT bool_and(relrowsecurity AND relforcerowsecurity)
        FROM pg_class WHERE relname IN ('trace_account_inference_connections',
        'trace_account_inference_connection_requests', 'trace_account_inference_connection_events')", &[]).await.unwrap();
    assert!(flags.get::<_, bool>(0));
    let mut restricted_url = reqwest::Url::parse(&url).unwrap();
    restricted_url
        .set_username("tc_inference_connection_z8_runtime")
        .unwrap();
    let restricted = PgBackend::new(&config(restricted_url.into()))
        .await
        .unwrap();
    let client = restricted
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    assert!(
        client
            .query("SELECT * FROM trace_account_inference_connections", &[])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        client
            .execute(
                "UPDATE trace_account_inference_connections SET revision = $1 WHERE connection_id = $2",
                &[&restored.revision, &restored.connection_id],
            )
            .await
            .is_err()
    );
    assert!(
        client
            .query("SELECT * FROM onboarding_invite_grants", &[])
            .await
            .is_err()
    );
}
