// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: deploys the complete migration chain, then verifies V69's runtime boundary.

use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use serde_json::json;
use trace_commons_server::{
    db::Database,
    mission_rewards::{RewardActivityKind, RewardError},
};
use uuid::Uuid;

use crate::fixture::{
    RewardPgFixture, assert_refusal, award_submission, create_program, digest, history_entry,
    reserve_and_submit, terms,
};

const PG_WAIT_TIMEOUT: StdDuration = StdDuration::from_secs(5);

async fn database_now(admin: &tokio_postgres::Client) -> DateTime<Utc> {
    admin
        .query_one("SELECT pg_catalog.clock_timestamp()", &[])
        .await
        .expect("read database clock")
        .get(0)
}

async fn wait_for_database_deadline(admin: &tokio_postgres::Client, deadline: DateTime<Utc>) {
    tokio::time::timeout(PG_WAIT_TIMEOUT, async {
        loop {
            let reached = admin
                .query_one("SELECT pg_catalog.clock_timestamp() >= $1", &[&deadline])
                .await
                .expect("compare database clock with deadline")
                .get::<_, bool>(0);
            if reached {
                return;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("database deadline reached within bounded wait");
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn full_migrations_preserve_rewards_and_expose_only_guarded_runtime_surface() {
    let fixture = RewardPgFixture::new().await;
    let program = create_program(
        &fixture.issuer_db,
        &fixture.tenant,
        RewardActivityKind::MissionCompletion,
        7,
        7,
        7,
    )
    .await;
    let participant = digest("deployment-awarded-participant");
    let reservation = award_submission(
        &fixture.issuer_db,
        &fixture.reviewer_db,
        &fixture.tenant,
        program,
        &participant,
        "deployment-awarded-claim",
    )
    .await;
    let before = fixture
        .issuer_db
        .reward_history(&fixture.tenant, &participant, 10)
        .await
        .expect("read award history before repeat migration");

    fixture
        .admin_db
        .run_migrations()
        .await
        .expect("repeat complete migration chain");

    let after = fixture
        .issuer_db
        .reward_history(&fixture.tenant, &participant, 10)
        .await
        .expect("read award history after repeat migration");
    assert_eq!(
        after, before,
        "repeat migration must preserve award history"
    );
    assert_eq!(history_entry(&after, reservation)["state"], "awarded");
    assert_eq!(
        fixture
            .admin
            .query_one(
                "SELECT pg_catalog.count(*) FROM _trace_commons_migrations \
                 WHERE version = 70 AND name = 'reward_history_pagination'",
                &[],
            )
            .await
            .expect("count V70 migration record")
            .get::<_, i64>(0),
        1,
        "V70 must have one durable migration record"
    );

    let forced_rls = fixture
        .admin
        .query(
            "SELECT relname FROM pg_catalog.pg_class \
             WHERE relnamespace = 'public'::pg_catalog.regnamespace \
               AND relname = ANY($1) \
               AND relrowsecurity AND relforcerowsecurity \
             ORDER BY relname",
            &[&&[
                "trace_reward_operators",
                "trace_reward_programs",
                "trace_reward_reservations",
                "trace_reward_decisions",
                "trace_reward_awards",
                "trace_reward_invalidations",
            ][..]],
        )
        .await
        .expect("inspect forced reward RLS");
    assert_eq!(forced_rls.len(), 6, "all reward tables force RLS");

    let executable_functions = fixture
        .admin
        .query(
            "SELECT p.proname, pg_catalog.oidvectortypes(p.proargtypes), \
                    owner.rolname, p.pronargdefaults \
             FROM pg_catalog.pg_proc p \
             JOIN pg_catalog.pg_namespace n ON n.oid = p.pronamespace \
             JOIN pg_catalog.pg_roles owner ON owner.oid = p.proowner \
             WHERE n.nspname = 'public' \
               AND p.proname LIKE 'trace_reward_%' \
               AND pg_catalog.has_function_privilege($1::name, p.oid, 'EXECUTE') \
             ORDER BY p.proname",
            &[&fixture.issuer],
        )
        .await
        .expect("inspect runtime executable functions");
    let signatures: Vec<(String, String)> = executable_functions
        .iter()
        .map(|function| (function.get(0), function.get(1)))
        .collect();
    let expected: Vec<(String, String)> = [
        ("trace_reward_cancel", "text, uuid, uuid"),
        ("trace_reward_claim_submit", "text, uuid, text, text"),
        ("trace_reward_history", "text, text, integer, uuid"),
        ("trace_reward_invalidate", "text, text, uuid"),
        ("trace_reward_program_create", "text, uuid, jsonb"),
        ("trace_reward_program_show", "text, uuid"),
        ("trace_reward_reserve", "text, uuid, uuid, text, text, text"),
        ("trace_reward_review", "text, uuid, uuid, boolean, text"),
    ]
    .into_iter()
    .map(|(name, arguments)| (name.to_owned(), arguments.to_owned()))
    .collect();
    assert_eq!(
        signatures, expected,
        "runtime exposes only the permitted API"
    );
    for function in &executable_functions {
        assert_eq!(function.get::<_, String>(2), "trace_reward_guard");
    }
    let history = executable_functions
        .iter()
        .find(|function| function.get::<_, String>(0) == "trace_reward_history")
        .expect("history function is executable by runtime");
    assert_eq!(history.get::<_, String>(1), "text, text, integer, uuid");
    assert_eq!(
        history.get::<_, i16>(3),
        1,
        "history cursor must default to NULL"
    );

    assert!(
        !fixture
            .admin
            .query_one(
                "SELECT pg_catalog.pg_has_role($1::name, 'trace_reward_guard', 'member')",
                &[&fixture.issuer],
            )
            .await
            .expect("inspect runtime guard membership")
            .get::<_, bool>(0),
        "runtime role must not hold guard membership"
    );
    let runtime = fixture.runtime().await;
    for statement in [
        "INSERT INTO public.trace_reward_programs DEFAULT VALUES",
        "UPDATE public.trace_reward_programs SET terms = terms",
        "DELETE FROM public.trace_reward_programs",
    ] {
        assert!(
            runtime.execute(statement, &[]).await.is_err(),
            "runtime direct DML must be denied: {statement}"
        );
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn database_deadlines_close_programs_expire_reservations_and_release_capacity() {
    let fixture = RewardPgFixture::new().await;
    let mut past_terms = terms(RewardActivityKind::MissionCompletion, 1, 1, 1);
    past_terms.closes_at = database_now(&fixture.admin).await - Duration::seconds(1);
    assert_refusal(
        fixture
            .issuer_db
            .reward_program_create(&fixture.tenant, Uuid::new_v4(), &past_terms)
            .await,
        RewardError::ProgramClosed,
    );

    let mut ttl_terms = terms(RewardActivityKind::MissionCompletion, 1, 1, 1);
    ttl_terms.closes_at = database_now(&fixture.admin).await + Duration::minutes(1);
    ttl_terms.reservation_ttl_seconds = 1;
    let ttl_program = Uuid::new_v4();
    fixture
        .issuer_db
        .reward_program_create(&fixture.tenant, ttl_program, &ttl_terms)
        .await
        .expect("create one-second TTL program");
    let expired_reservation = Uuid::new_v4();
    fixture
        .issuer_db
        .reward_reserve(
            &fixture.tenant,
            ttl_program,
            expired_reservation,
            &digest("expired-participant"),
            &digest("expired-work"),
            &digest("expired-consent"),
        )
        .await
        .expect("reserve expiring capacity");
    let expiration = fixture
        .admin
        .query_one(
            "SELECT expires_at FROM public.trace_reward_reservations \
             WHERE tenant_id = $1 AND reservation_id = $2",
            &[&fixture.tenant, &expired_reservation],
        )
        .await
        .expect("read persisted reservation deadline")
        .get::<_, DateTime<Utc>>(0);
    wait_for_database_deadline(&fixture.admin, expiration).await;
    assert_refusal(
        fixture
            .issuer_db
            .reward_claim_submit(
                &fixture.tenant,
                expired_reservation,
                &digest("expired-evidence"),
                &digest("expired-evaluation"),
            )
            .await,
        RewardError::ReservationExpired,
    );
    fixture
        .issuer_db
        .reward_reserve(
            &fixture.tenant,
            ttl_program,
            Uuid::new_v4(),
            &digest("reused-capacity-participant"),
            &digest("reused-capacity-work"),
            &digest("reused-capacity-consent"),
        )
        .await
        .expect("expired reservation releases capacity for distinct work");

    let mut close_terms = terms(RewardActivityKind::MissionCompletion, 1, 10, 10);
    close_terms.closes_at = database_now(&fixture.admin).await + Duration::seconds(2);
    let close_program = Uuid::new_v4();
    fixture
        .issuer_db
        .reward_program_create(&fixture.tenant, close_program, &close_terms)
        .await
        .expect("create shortly closing program");
    wait_for_database_deadline(&fixture.admin, close_terms.closes_at).await;
    assert_refusal(
        fixture
            .issuer_db
            .reward_reserve(
                &fixture.tenant,
                close_program,
                Uuid::new_v4(),
                &digest("closed-participant"),
                &digest("closed-work"),
                &digest("closed-consent"),
            )
            .await,
        RewardError::ProgramClosed,
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn history_serialization_waits_for_review_lock_then_returns_awarded_total() {
    let fixture = RewardPgFixture::new().await;
    let participant = digest("history-advisory-lock-participant");
    let program = create_program(
        &fixture.issuer_db,
        &fixture.tenant,
        RewardActivityKind::MissionCompletion,
        7,
        7,
        7,
    )
    .await;
    let (reservation, _) = reserve_and_submit(
        &fixture.issuer_db,
        &fixture.tenant,
        program,
        &participant,
        "history-advisory-lock",
    )
    .await;

    let mut reviewer = fixture
        .reviewer_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("acquire raw reviewer connection");
    let transaction = reviewer
        .transaction()
        .await
        .expect("start review transaction");
    transaction
        .execute(
            "SELECT pg_catalog.set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&fixture.tenant],
        )
        .await
        .expect("bind reviewer transaction tenant");
    transaction
        .query_one(
            "SELECT public.trace_reward_review($1, $2, $3, true, 'completion_verified')",
            &[&fixture.tenant, &reservation, &Uuid::new_v4()],
        )
        .await
        .expect("hold uncommitted exclusive review lock");

    let issuer = fixture.runtime().await;
    let issuer_pid = issuer
        .query_one("SELECT pg_catalog.pg_backend_pid()", &[])
        .await
        .expect("capture raw issuer backend pid")
        .get::<_, i32>(0);
    let tenant = fixture.tenant.clone();
    let participant_for_history = participant.clone();
    let history_task = tokio::spawn(async move {
        issuer
            .query_one(
                "SELECT public.trace_reward_history($1, $2, $3, $4)",
                &[
                    &tenant,
                    &participant_for_history,
                    &10_i32,
                    &Option::<Uuid>::None,
                ],
            )
            .await
            .expect("history query after review lock releases")
            .get::<_, serde_json::Value>(0)
    });

    tokio::time::timeout(PG_WAIT_TIMEOUT, async {
        loop {
            let waiting = fixture
                .admin
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM pg_catalog.pg_locks \
                     WHERE pid = $1 AND locktype = 'advisory' AND NOT granted)",
                    &[&issuer_pid],
                )
                .await
                .expect("inspect issuer advisory lock wait")
                .get::<_, bool>(0);
            if waiting {
                return;
            }
            tokio::time::sleep(StdDuration::from_millis(25)).await;
        }
    })
    .await
    .expect("history query visibly waits for review advisory lock");
    assert!(
        !history_task.is_finished(),
        "history result remains pending until review commits"
    );

    transaction.commit().await.expect("commit held review");
    let history = tokio::time::timeout(PG_WAIT_TIMEOUT, history_task)
        .await
        .expect("history completes after review commit")
        .expect("history task joins");
    let entry = history_entry(&history, reservation);
    assert_eq!(entry["state"], "awarded");
    assert_eq!(entry["awarded_units"], json!(7));
    assert_eq!(history["programs"][0]["program_id"], program.to_string());
    assert_eq!(history["programs"][0]["awarded_units"], json!(7));
}
