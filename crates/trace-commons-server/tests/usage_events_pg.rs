// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Anonymous weekly usage counts (V152) on a real PostgreSQL, as the
//! least-privilege ingest runtime: it may store a batch through the
//! guard-owned function and nothing else.

use trace_commons_protocol::usage_analytics::{UsageEventBatch, UsageEventRow, new_install_id};
use trace_commons_server::{
    config::DatabaseConfig,
    db::{Database, postgres::PgBackend},
};

/// The migrator's backend (to read what was stored) and the runtime's.
async fn backends() -> Option<(PgBackend, PgBackend)> {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        eprintln!("SKIP: TRACE_COMMONS_PG_TEST_DATABASE_URL not configured");
        return None;
    };
    let admin = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .unwrap();
    admin.run_migrations().await.unwrap();
    let mut runtime_url = reqwest::Url::parse(&url).unwrap();
    runtime_url.set_query(Some("options=-c%20role%3Dtrace_ingest_runtime"));
    let runtime = PgBackend::new(&DatabaseConfig::from_postgres_url(runtime_url.as_str(), 2))
        .await
        .unwrap();
    Some((admin, runtime))
}

fn row(event: &str, count: u64, week: &str) -> UsageEventRow {
    UsageEventRow {
        event: event.into(),
        count,
        week: week.into(),
    }
}

/// What the guard sees for one install: (week, event, count).
async fn stored(admin: &PgBackend, install_id: uuid::Uuid) -> Vec<(String, String, i64)> {
    let client = admin.trace_pool_for_test().get().await.unwrap();
    // V152 hands the guard back, so the migrator reads as an operator
    // would: take the guard, read, give it back.
    client
        .batch_execute(
            "GRANT trace_usage_events_guard TO CURRENT_USER;
             SET ROLE trace_usage_events_guard",
        )
        .await
        .unwrap();
    let rows = client
        .query(
            "SELECT week, event, event_count FROM trace_usage_event_weeks
              WHERE install_id = $1 ORDER BY week, event",
            &[&install_id],
        )
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();
    client
        .batch_execute("RESET ROLE; REVOKE trace_usage_events_guard FROM CURRENT_USER")
        .await
        .unwrap();
    rows
}

#[tokio::test]
async fn the_runtime_stores_a_batch_and_a_resend_replaces_it() {
    let Some((admin, runtime)) = backends().await else {
        return;
    };
    let install_id = new_install_id();
    let batch = UsageEventBatch {
        install_id,
        rows: vec![
            row("tab_insights_opened", 3, "2026-W45"),
            row("setup_finished", 1, "2026-W45"),
        ],
    };
    assert_eq!(runtime.record_usage_events(&batch).await.unwrap(), 2);
    // The same week again (a lost response, resent): replaced, not doubled.
    assert_eq!(runtime.record_usage_events(&batch).await.unwrap(), 2);
    let next = UsageEventBatch {
        install_id,
        rows: vec![row("tab_insights_opened", 5, "2026-W46")],
    };
    runtime.record_usage_events(&next).await.unwrap();
    assert_eq!(
        stored(&admin, install_id).await,
        vec![
            ("2026-W45".into(), "setup_finished".into(), 1),
            ("2026-W45".into(), "tab_insights_opened".into(), 3),
            ("2026-W46".into(), "tab_insights_opened".into(), 5),
        ]
    );
}

#[tokio::test]
async fn the_runtime_cannot_read_or_write_the_table_directly() {
    let Some((_admin, runtime)) = backends().await else {
        return;
    };
    let client = runtime.trace_pool_for_test().get().await.unwrap();
    for sql in [
        "SELECT count(*) FROM trace_usage_event_weeks",
        "INSERT INTO trace_usage_event_weeks (install_id, week, event, event_count)
              VALUES (gen_random_uuid(), '2026-W45', 'setup_finished', 1)",
        "DELETE FROM trace_usage_event_weeks",
    ] {
        let err = client.batch_execute(sql).await.unwrap_err();
        assert_eq!(
            err.code(),
            Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
            "{sql}"
        );
    }
}

#[tokio::test]
async fn the_table_refuses_what_the_protocol_refuses() {
    let Some((_admin, runtime)) = backends().await else {
        return;
    };
    // Bypassing the service's validation, the table's checks still hold.
    for rows in [
        vec![row("Not An Event", 1, "2026-W45")],
        vec![row("setup_finished", 1, "2026-W99")],
        vec![row("setup_finished", 100_001, "2026-W45")],
    ] {
        let batch = UsageEventBatch {
            install_id: new_install_id(),
            rows,
        };
        let err = runtime.record_usage_events(&batch).await.unwrap_err();
        // A check constraint (23514), not some other failure.
        assert!(format!("{err:?}").contains("E23514"), "{err:?}");
    }
}
