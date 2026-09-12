// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: boundary regressions for V69 tenant isolation and ledger serialization.

use std::time::Duration;

use serde_json::json;
use trace_commons_server::mission_rewards::{RewardActivityKind, RewardError};
use uuid::Uuid;

use crate::fixture::{
    RewardPgFixture, assert_refusal, award_submission, create_program, digest, history_entry,
    reserve_and_submit,
};

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn authorized_operator_cannot_cross_tenant_reward_boundaries() {
    let fixture_a = RewardPgFixture::new().await;
    let fixture_b = RewardPgFixture::new().await;
    let participant = digest("cross-tenant-participant");
    let program = create_program(
        &fixture_a.issuer_db,
        &fixture_a.tenant,
        RewardActivityKind::MissionCompletion,
        7,
        7,
        7,
    )
    .await;
    award_submission(
        &fixture_a.issuer_db,
        &fixture_a.reviewer_db,
        &fixture_a.tenant,
        program,
        &participant,
        "cross-tenant-award",
    )
    .await;

    assert_refusal(
        fixture_b
            .issuer_db
            .reward_program_show(&fixture_b.tenant, program)
            .await,
        RewardError::NotFound,
    );
    let history = fixture_b
        .issuer_db
        .reward_history(&fixture_b.tenant, &participant, 10)
        .await
        .expect("authorized empty tenant history");
    assert_eq!(history["entries"], json!([]));
    assert!(
        history["programs"]
            .as_array()
            .expect("program list")
            .is_empty()
    );
    assert_refusal(
        fixture_b
            .issuer_db
            .reward_program_show(&fixture_a.tenant, program)
            .await,
        RewardError::Unauthorized,
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn claim_and_cancel_replays_preserve_terminal_ledger_rules() {
    let fixture = RewardPgFixture::new().await;
    let program = create_program(
        &fixture.issuer_db,
        &fixture.tenant,
        RewardActivityKind::MissionCompletion,
        2,
        10,
        10,
    )
    .await;
    let participant = digest("claim-replay-participant");
    let (submitted, evidence) = reserve_and_submit(
        &fixture.issuer_db,
        &fixture.tenant,
        program,
        &participant,
        "claim-replay",
    )
    .await;
    let exact = fixture
        .issuer_db
        .reward_claim_submit(
            &fixture.tenant,
            submitted,
            &evidence,
            &digest("claim-replay-evaluation"),
        )
        .await
        .expect("exact claim replay");
    assert_eq!(exact["state"], "submitted");
    assert_refusal(
        fixture
            .issuer_db
            .reward_claim_submit(
                &fixture.tenant,
                submitted,
                &evidence,
                &digest("changed-evaluation"),
            )
            .await,
        RewardError::PayloadConflict,
    );
    assert_refusal(
        fixture
            .issuer_db
            .reward_cancel(&fixture.tenant, submitted, Uuid::new_v4())
            .await,
        RewardError::StateConflict,
    );
    fixture
        .reviewer_db
        .reward_review(
            &fixture.tenant,
            submitted,
            Uuid::new_v4(),
            true,
            "completion_verified",
        )
        .await
        .expect("award submitted claim");
    assert_refusal(
        fixture
            .issuer_db
            .reward_cancel(&fixture.tenant, submitted, Uuid::new_v4())
            .await,
        RewardError::StateConflict,
    );

    let cancellable = Uuid::new_v4();
    fixture
        .issuer_db
        .reward_reserve(
            &fixture.tenant,
            program,
            cancellable,
            &digest("cancel-participant"),
            &digest("cancel-work"),
            &digest("cancel-consent"),
        )
        .await
        .expect("reserve cancellable claim");
    let decision = Uuid::new_v4();
    let cancelled = fixture
        .issuer_db
        .reward_cancel(&fixture.tenant, cancellable, decision)
        .await
        .expect("cancel reserved claim");
    assert_eq!(
        fixture
            .issuer_db
            .reward_cancel(&fixture.tenant, cancellable, decision)
            .await
            .expect("exact cancellation replay"),
        cancelled
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn history_waits_for_pending_review_before_reporting_award() {
    let fixture = RewardPgFixture::new().await;
    let participant = digest("history-lock-participant");
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
        "history-lock",
    )
    .await;
    let mut client = fixture
        .reviewer_db
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("reviewer pool connection");
    let transaction = client.transaction().await.expect("review transaction");
    transaction
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&fixture.tenant],
        )
        .await
        .expect("bind review transaction tenant");
    transaction
        .query_one(
            "SELECT public.trace_reward_review($1, $2, $3, true, 'completion_verified')",
            &[&fixture.tenant, &reservation, &Uuid::new_v4()],
        )
        .await
        .expect("execute uncommitted review");

    let history = fixture
        .issuer_db
        .reward_history(&fixture.tenant, &participant, 10);
    tokio::pin!(history);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut history)
            .await
            .is_err(),
        "history must wait on the same tenant lock as review"
    );
    transaction
        .commit()
        .await
        .expect("commit review transaction");
    let history = history.await.expect("history after review commit");
    let entry = history_entry(&history, reservation);
    assert_eq!(entry["state"], "awarded");
    assert_eq!(entry["awarded_units"], json!(7));
    assert_eq!(history["programs"][0]["program_id"], program.to_string());
    assert_eq!(history["programs"][0]["awarded_units"], json!(7));
}
