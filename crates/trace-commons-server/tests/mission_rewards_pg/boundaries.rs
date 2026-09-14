// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: boundary regressions for V69 tenant isolation and ledger serialization.

use serde_json::{Value, json};
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::mission_rewards::{RewardActivityKind, RewardError};
use uuid::Uuid;

use crate::fixture::{
    RewardPgFixture, assert_refusal, award_submission, create_program, digest, reserve_and_submit,
    terms,
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

// A work digest is held by trace_reward_reservations_live_work_idx and by the
// matching state predicate in trace_reward_reserve. Before they carried a state
// predicate, one cancelled or rejected reservation spent its work digest for the
// life of the tenant, with no function able to release it.
async fn reserve_work(
    db: &PgBackend,
    tenant: &str,
    program: Uuid,
    participant: &str,
    work: &str,
    consent_seed: &str,
) -> (Uuid, Result<Value, RewardError>) {
    let reservation = Uuid::new_v4();
    let result = db
        .reward_reserve(
            tenant,
            program,
            reservation,
            participant,
            work,
            &digest(consent_seed),
        )
        .await;
    (reservation, result)
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn only_live_and_awarded_reservations_hold_their_work_digest() {
    let fixture = RewardPgFixture::new().await;
    let tenant = fixture.tenant.as_str();
    let issuer_db = &fixture.issuer_db;
    let reviewer_db = &fixture.reviewer_db;
    let program = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        1,
        20,
        20,
    )
    .await;
    let participant = digest("work-digest-participant");

    // Cancellation releases the digest.
    let cancelled_work = digest("work-digest-cancelled");
    let (cancelled, held) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &cancelled_work,
        "consent-cancelled",
    )
    .await;
    held.expect("reserve cancellable work");
    let (_, blocked) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &cancelled_work,
        "consent-cancelled-retry",
    )
    .await;
    assert_refusal(blocked, RewardError::WorkDuplicate);
    issuer_db
        .reward_cancel(tenant, cancelled, Uuid::new_v4())
        .await
        .expect("cancel reserved claim");
    let (_, after_cancel) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &cancelled_work,
        "consent-cancelled-again",
    )
    .await;
    after_cancel.expect("cancellation releases the work digest");

    // An award holds the digest for good.
    let awarded_work = digest("work-digest-awarded");
    let (awarded, reserved) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &awarded_work,
        "consent-awarded",
    )
    .await;
    reserved.expect("reserve awardable work");
    issuer_db
        .reward_claim_submit(
            tenant,
            awarded,
            &digest("work-digest-awarded-evidence"),
            &digest("work-digest-awarded-evaluation"),
        )
        .await
        .expect("submit awardable work");
    reviewer_db
        .reward_review(tenant, awarded, Uuid::new_v4(), true, "completion_verified")
        .await
        .expect("award submitted claim");
    let (_, after_award) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &awarded_work,
        "consent-awarded-again",
    )
    .await;
    assert_refusal(after_award, RewardError::WorkDuplicate);

    // Rejection releases the digest.
    let rejected_work = digest("work-digest-rejected");
    let (rejected, reserved) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &rejected_work,
        "consent-rejected",
    )
    .await;
    reserved.expect("reserve rejectable work");
    issuer_db
        .reward_claim_submit(
            tenant,
            rejected,
            &digest("work-digest-rejected-evidence"),
            &digest("work-digest-rejected-evaluation"),
        )
        .await
        .expect("submit rejectable work");
    reviewer_db
        .reward_review(tenant, rejected, Uuid::new_v4(), false, "rubric_not_met")
        .await
        .expect("reject submitted claim");
    let (_, after_rejection) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &rejected_work,
        "consent-rejected-again",
    )
    .await;
    after_rejection.expect("rejection releases the work digest");

    // Invalidating a pending submission releases the digest.
    let invalidated_work = digest("work-digest-invalidated");
    let invalidated_evidence = digest("work-digest-invalidated-evidence");
    let (invalidated, reserved) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &invalidated_work,
        "consent-invalidated",
    )
    .await;
    reserved.expect("reserve invalidatable work");
    issuer_db
        .reward_claim_submit(
            tenant,
            invalidated,
            &invalidated_evidence,
            &digest("work-digest-invalidated-evaluation"),
        )
        .await
        .expect("submit invalidatable work");
    issuer_db
        .reward_invalidate(tenant, &invalidated_evidence, Uuid::new_v4())
        .await
        .expect("invalidate pending submission");
    let (_, after_invalidation) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &invalidated_work,
        "consent-invalidated-again",
    )
    .await;
    after_invalidation.expect("invalidation releases the work digest");

    // Expiry is stored as a reserved row with a past deadline, so the digest
    // stays held until the reservation is cancelled. The runbook says so.
    let expired_work = digest("work-digest-expired");
    let (expired, reserved) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &expired_work,
        "consent-expired",
    )
    .await;
    reserved.expect("reserve expiring work");
    fixture
        .admin
        .execute(
            "UPDATE trace_reward_reservations \
             SET expires_at = pg_catalog.clock_timestamp() - interval '1 second' \
             WHERE tenant_id = $1 AND reservation_id = $2",
            &[&fixture.tenant, &expired],
        )
        .await
        .expect("age the reservation past its deadline");
    let (_, while_expired) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &expired_work,
        "consent-expired-retry",
    )
    .await;
    assert_refusal(while_expired, RewardError::WorkDuplicate);
    issuer_db
        .reward_cancel(tenant, expired, Uuid::new_v4())
        .await
        .expect("cancel the expired reservation");
    let (_, after_expiry_cancel) = reserve_work(
        issuer_db,
        tenant,
        program,
        &participant,
        &expired_work,
        "consent-expired-again",
    )
    .await;
    after_expiry_cancel.expect("cancelling an expired reservation releases the work digest");
    assert_eq!(
        issuer_db
            .reward_history(tenant, &participant, 100)
            .await
            .expect("read participant history")["entries"]
            .as_array()
            .expect("history entries")
            .len(),
        9,
        "every reservation that succeeded is still recorded"
    );
}

fn refusal_label(error: &tokio_postgres::Error) -> String {
    error
        .as_db_error()
        .expect("a refusal is a database error")
        .message()
        .to_owned()
}

// jsonb keeps the written form of a number, so `1` and `1.0` are two terms
// objects with two terms_hash values, and replaying one against the other
// conflicts. The validator admits only the integer spelling, which is what
// makes a given set of terms have exactly one digest.
#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn reward_terms_admit_one_spelling_of_each_integer() {
    let fixture = RewardPgFixture::new().await;
    let runtime = fixture.runtime().await;
    let integer_terms = serde_json::to_value(terms(RewardActivityKind::MissionCompletion, 1, 4, 4))
        .expect("serialize integer terms");
    let mut decimal_terms = integer_terms.clone();
    decimal_terms["award_units"] = json!(1.0);
    assert_eq!(
        decimal_terms["award_units"].to_string(),
        "1.0",
        "the decimal spelling must survive serialization"
    );

    let program = Uuid::new_v4();
    let refusal = runtime
        .query_one(
            "SELECT public.trace_reward_program_create($1, $2, $3)",
            &[&fixture.tenant, &program, &decimal_terms],
        )
        .await
        .expect_err("a decimal spelling must be refused");
    assert_eq!(
        refusal_label(&refusal),
        "reward_request_invalid",
        "refusal must carry the safe label"
    );

    let created = runtime
        .query_one(
            "SELECT public.trace_reward_program_create($1, $2, $3)",
            &[&fixture.tenant, &program, &integer_terms],
        )
        .await
        .expect("the integer spelling creates the program")
        .get::<_, Value>(0);
    let replayed = runtime
        .query_one(
            "SELECT public.trace_reward_program_create($1, $2, $3)",
            &[&fixture.tenant, &program, &integer_terms],
        )
        .await
        .expect("exact replay")
        .get::<_, Value>(0);
    assert_eq!(
        created["terms_hash"], replayed["terms_hash"],
        "one spelling digests to one terms_hash"
    );
    let conflict = runtime
        .query_one(
            "SELECT public.trace_reward_program_create($1, $2, $3)",
            &[&fixture.tenant, &program, &decimal_terms],
        )
        .await
        .expect_err("the decimal spelling cannot reach the replay comparison");
    assert_eq!(
        refusal_label(&conflict),
        "reward_request_invalid",
        "a second spelling is refused before it can conflict"
    );
}
