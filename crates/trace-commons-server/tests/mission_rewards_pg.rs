// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

#[path = "mission_rewards_pg/boundaries.rs"]
mod boundaries;
#[path = "mission_rewards_pg/deadlines.rs"]
mod deadlines;
#[path = "mission_rewards_pg/fixture.rs"]
mod fixture;
#[path = "mission_rewards_pg/operator_cli.rs"]
mod operator_cli;

use serde_json::{Value, json};
use trace_commons_server::mission_rewards::{RewardActivityKind, RewardError};
use uuid::Uuid;

use crate::fixture::{
    RewardPgFixture, assert_refusal, award_submission, create_program, digest, history_entry,
    race_for_slot, reserve_and_submit, terms,
};

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn reward_authority_and_replay_are_fail_closed() {
    let fixture = RewardPgFixture::new().await;
    let tenant = fixture.tenant.as_str();
    let issuer = fixture.issuer.as_str();
    let suffix = fixture.suffix.as_str();
    let issuer_db = &fixture.issuer_db;
    let reviewer_db = &fixture.reviewer_db;
    let no_grant_db = &fixture.no_grant_db;
    let admin_db = &fixture.admin_db;
    let admin = &fixture.admin;
    let reviewer_actor = fixture.reviewer_actor.clone();
    let runtime = fixture.runtime().await;
    let row = runtime.query_one("SELECT session_user::text, rolsuper OR rolbypassrls FROM pg_roles WHERE rolname = session_user", &[]).await.expect("inspect runtime login");
    assert_eq!(row.get::<_, String>(0), issuer);
    assert!(
        !row.get::<_, bool>(1),
        "runtime must never be superuser or BYPASSRLS"
    );
    assert!(
        admin
            .query_one(
                "SELECT relforcerowsecurity FROM pg_class WHERE relname = 'trace_reward_programs'",
                &[]
            )
            .await
            .expect("inspect RLS")
            .get::<_, bool>(0)
    );

    let mission = Uuid::new_v4();
    let mission_terms = terms(RewardActivityKind::MissionCompletion, 10, 30, 20);
    assert_refusal(
        reviewer_db
            .reward_program_create(tenant, mission, &mission_terms)
            .await,
        RewardError::Unauthorized,
    );
    assert_refusal(
        no_grant_db.reward_program_show(tenant, mission).await,
        RewardError::Unauthorized,
    );
    assert_refusal(
        admin_db.reward_program_show(tenant, mission).await,
        RewardError::Unauthorized,
    );
    let created = issuer_db
        .reward_program_create(tenant, mission, &mission_terms)
        .await
        .expect("issuer creates mission");
    assert_eq!(created["identity_mode"], "operator_asserted");
    assert_eq!(created["redemption"], "none");
    assert_eq!(
        issuer_db
            .reward_program_create(tenant, mission, &mission_terms)
            .await
            .expect("exact create replay"),
        created
    );
    let mut altered = mission_terms.clone();
    altered.capacity_units = 40;
    assert_refusal(
        issuer_db
            .reward_program_create(tenant, mission, &altered)
            .await,
        RewardError::PayloadConflict,
    );
    assert_refusal(
        issuer_db.reward_program_show("wrong-tenant", mission).await,
        RewardError::Unauthorized,
    );
    let participant = digest("participant-a");
    let reservation = Uuid::new_v4();
    let work = digest("mission-work-a");
    let consent = digest("consent-a");
    let held = issuer_db
        .reward_reserve(tenant, mission, reservation, &participant, &work, &consent)
        .await
        .expect("reserve mission");
    assert_eq!(
        issuer_db
            .reward_reserve(tenant, mission, reservation, &participant, &work, &consent)
            .await
            .expect("exact reserve replay"),
        held
    );
    assert_refusal(
        issuer_db
            .reward_reserve(
                tenant,
                mission,
                Uuid::new_v4(),
                &participant,
                &work,
                &consent,
            )
            .await,
        RewardError::WorkDuplicate,
    );
    let evidence = digest("mission-evidence-a");
    let held_history = issuer_db
        .reward_history(tenant, &participant, 1)
        .await
        .unwrap();
    assert_eq!(
        held_history["programs"][0]["program_id"],
        mission.to_string()
    );
    assert_eq!(held_history["programs"][0]["awarded_units"], json!(0));
    issuer_db
        .reward_claim_submit(tenant, reservation, &evidence, &digest("evaluation-a"))
        .await
        .expect("submit mission evidence");
    let decision = Uuid::new_v4();
    let (first_review, duplicate_review) = tokio::join!(
        reviewer_db.reward_review(tenant, reservation, decision, true, "completion_verified"),
        reviewer_db.reward_review(tenant, reservation, decision, true, "completion_verified")
    );
    let award = first_review.expect("accept mission");
    assert_eq!(
        duplicate_review.expect("concurrent exact review replay"),
        award
    );
    assert_refusal(
        reviewer_db
            .reward_review(
                tenant,
                reservation,
                Uuid::new_v4(),
                true,
                "completion_verified",
            )
            .await,
        RewardError::StateConflict,
    );
    assert!(
        runtime
            .execute("INSERT INTO trace_reward_programs DEFAULT VALUES", &[])
            .await
            .is_err(),
        "runtime has no direct DML"
    );
    assert!(
        runtime
            .execute("SET ROLE trace_reward_guard", &[])
            .await
            .is_err()
    );
    assert!(
        runtime
            .query_one("SELECT public.trace_reward_authorize($1, NULL)", &[&tenant])
            .await
            .is_err()
    );
    assert!(
        runtime
            .query_one("SELECT * FROM trace_reward_operators", &[])
            .await
            .is_err()
    );

    let insight = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::InsightContribution,
        5,
        10,
        10,
    )
    .await;
    let (insight_reservation, _) = reserve_and_submit(
        issuer_db,
        tenant,
        insight,
        &digest("participant-b"),
        "insight",
    )
    .await;
    reviewer_db
        .reward_review(
            tenant,
            insight_reservation,
            Uuid::new_v4(),
            true,
            "qualified_negative",
        )
        .await
        .expect("qualified negative is eligible");
    let (inconclusive, _) = reserve_and_submit(
        issuer_db,
        tenant,
        insight,
        &digest("participant-c"),
        "inconclusive",
    )
    .await;
    reviewer_db
        .reward_review(
            tenant,
            inconclusive,
            Uuid::new_v4(),
            true,
            "qualified_inconclusive",
        )
        .await
        .expect("qualified inconclusive is eligible");

    let duplicate_actor_role = format!("reward_alias_{suffix}");
    admin
        .batch_execute(&format!("CREATE ROLE {duplicate_actor_role} LOGIN NOSUPERUSER NOBYPASSRLS; GRANT trace_reward_runtime TO {duplicate_actor_role};"))
        .await
        .expect("create duplicate-actor login");
    assert!(admin
        .execute("INSERT INTO trace_reward_operators (tenant_id, login_role, actor_hash, operator_role) VALUES ($1, $2::name, $3, 'reviewer')", &[&tenant, &duplicate_actor_role, &reviewer_actor])
        .await
        .is_err(), "one actor cannot acquire a second tenant login grant");
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn reward_reviews_require_an_independent_actor() {
    let fixture = RewardPgFixture::new().await;
    let tenant = fixture.tenant.as_str();
    let issuer = fixture.issuer.as_str();
    let reviewer = fixture.reviewer.as_str();
    let issuer_db = &fixture.issuer_db;
    let reviewer_db = &fixture.reviewer_db;
    let admin = &fixture.admin;
    let reviewer_actor = fixture.reviewer_actor.clone();
    let self_review_program = Uuid::new_v4();
    let mut sponsor_terms = terms(RewardActivityKind::MissionCompletion, 1, 20, 20);
    sponsor_terms.sponsor_hash = reviewer_actor.clone();
    issuer_db
        .reward_program_create(tenant, self_review_program, &sponsor_terms)
        .await
        .expect("create sponsor self-review fixture");
    let (sponsor_self, _) = reserve_and_submit(
        issuer_db,
        tenant,
        self_review_program,
        &digest("sponsor-participant"),
        "sponsor",
    )
    .await;
    assert_refusal(
        reviewer_db
            .reward_review(
                tenant,
                sponsor_self,
                Uuid::new_v4(),
                true,
                "completion_verified",
            )
            .await,
        RewardError::SelfReview,
    );

    let creator_program = Uuid::new_v4();
    issuer_db
        .reward_program_create(
            tenant,
            creator_program,
            &terms(RewardActivityKind::MissionCompletion, 1, 10, 10),
        )
        .await
        .expect("create ordinary self-review fixture");
    let (participant_self, _) = reserve_and_submit(
        issuer_db,
        tenant,
        creator_program,
        &reviewer_actor,
        "participant-self",
    )
    .await;
    assert_refusal(
        reviewer_db
            .reward_review(
                tenant,
                participant_self,
                Uuid::new_v4(),
                true,
                "completion_verified",
            )
            .await,
        RewardError::SelfReview,
    );

    let (creator_self, _) = reserve_and_submit(
        issuer_db,
        tenant,
        creator_program,
        &digest("creator-participant"),
        "creator",
    )
    .await;
    admin.execute("UPDATE trace_reward_operators SET operator_role = 'reviewer' WHERE tenant_id = $1 AND login_role = $2::name", &[&tenant, &issuer]).await.expect("temporarily provision creator as reviewer");
    assert_refusal(
        issuer_db
            .reward_review(
                tenant,
                creator_self,
                Uuid::new_v4(),
                true,
                "completion_verified",
            )
            .await,
        RewardError::SelfReview,
    );
    admin.execute("UPDATE trace_reward_operators SET operator_role = 'issuer' WHERE tenant_id = $1 AND login_role = $2::name", &[&tenant, &issuer]).await.expect("restore issuer grant");

    let submitter_self = Uuid::new_v4();
    issuer_db
        .reward_reserve(
            tenant,
            creator_program,
            submitter_self,
            &digest("submitter-participant"),
            &digest("submitter-work"),
            &digest("submitter-consent"),
        )
        .await
        .expect("reserve submitter fixture");
    admin.execute("UPDATE trace_reward_operators SET operator_role = 'issuer' WHERE tenant_id = $1 AND login_role = $2::name", &[&tenant, &reviewer]).await.expect("temporarily provision reviewer as issuer");
    reviewer_db
        .reward_claim_submit(
            tenant,
            submitter_self,
            &digest("submitter-evidence"),
            &digest("submitter-evaluation"),
        )
        .await
        .expect("submit under reviewer actor");
    admin.execute("UPDATE trace_reward_operators SET operator_role = 'reviewer' WHERE tenant_id = $1 AND login_role = $2::name", &[&tenant, &reviewer]).await.expect("restore reviewer grant");
    assert_refusal(
        reviewer_db
            .reward_review(
                tenant,
                submitter_self,
                Uuid::new_v4(),
                true,
                "completion_verified",
            )
            .await,
        RewardError::SelfReview,
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn reward_history_duplicates_and_invalidation_are_consistent() {
    let fixture = RewardPgFixture::new().await;
    let tenant = fixture.tenant.as_str();
    let issuer_db = &fixture.issuer_db;
    let reviewer_db = &fixture.reviewer_db;
    let admin = &fixture.admin;

    let mission = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        10,
        30,
        20,
    )
    .await;
    let participant = digest("duplicate-participant");
    let reservation = Uuid::new_v4();
    let work = digest("duplicate-mission-work");
    let evidence = digest("duplicate-mission-evidence");
    issuer_db
        .reward_reserve(
            tenant,
            mission,
            reservation,
            &participant,
            &work,
            &digest("duplicate-mission-consent"),
        )
        .await
        .expect("reserve awarded duplicate fixture");
    issuer_db
        .reward_claim_submit(
            tenant,
            reservation,
            &evidence,
            &digest("duplicate-mission-evaluation"),
        )
        .await
        .expect("submit awarded duplicate fixture");
    reviewer_db
        .reward_review(
            tenant,
            reservation,
            Uuid::new_v4(),
            true,
            "completion_verified",
        )
        .await
        .expect("award duplicate fixture");
    let duplicate_program = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        1,
        10,
        10,
    )
    .await;
    assert_refusal(
        issuer_db
            .reward_reserve(
                tenant,
                duplicate_program,
                Uuid::new_v4(),
                &digest("participant-z"),
                &work,
                &digest("consent-z"),
            )
            .await,
        RewardError::WorkDuplicate,
    );
    let duplicate_reservation = Uuid::new_v4();
    issuer_db
        .reward_reserve(
            tenant,
            duplicate_program,
            duplicate_reservation,
            &digest("participant-z"),
            &digest("new-work"),
            &digest("consent-z"),
        )
        .await
        .expect("new work reserves");
    assert_refusal(
        issuer_db
            .reward_claim_submit(
                tenant,
                duplicate_reservation,
                &evidence,
                &digest("new-evaluation"),
            )
            .await,
        RewardError::EvidenceDuplicate,
    );

    let tombstone = digest("tombstone-evidence");
    issuer_db
        .reward_invalidate(tenant, &tombstone, Uuid::new_v4())
        .await
        .expect("pre-award tombstone");
    let blocked = Uuid::new_v4();
    issuer_db
        .reward_reserve(
            tenant,
            duplicate_program,
            blocked,
            &digest("participant-y"),
            &digest("tombstone-work"),
            &digest("consent-y"),
        )
        .await
        .expect("reserve tombstoned evidence work");
    assert_refusal(
        issuer_db
            .reward_claim_submit(tenant, blocked, &tombstone, &digest("tombstone-eval"))
            .await,
        RewardError::EvidenceInvalidated,
    );
    issuer_db
        .reward_invalidate(tenant, &evidence, Uuid::new_v4())
        .await
        .expect("post-award tombstone");
    let invalidated_history = issuer_db
        .reward_history(tenant, &participant, 10)
        .await
        .expect("post-award history");
    let invalidated_entry = history_entry(&invalidated_history, reservation);
    assert_eq!(invalidated_entry["state"], "awarded_invalidated");
    assert_eq!(invalidated_entry["awarded_units"], json!(10));
    assert_eq!(invalidated_entry["invalidated"], true);
    assert_eq!(
        issuer_db
            .reward_program_show(tenant, mission)
            .await
            .expect("show after invalidation")["capacity_used_units"],
        json!(10)
    );

    let pending = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        1,
        1,
        1,
    )
    .await;
    let (_, pending_evidence) = reserve_and_submit(
        issuer_db,
        tenant,
        pending,
        &digest("pending-participant"),
        "pending-invalidation",
    )
    .await;
    let invalidation = Uuid::new_v4();
    let invalidated = issuer_db
        .reward_invalidate(tenant, &pending_evidence, invalidation)
        .await
        .expect("invalidate submitted claim");
    assert_eq!(
        issuer_db
            .reward_invalidate(tenant, &pending_evidence, invalidation)
            .await
            .expect("exact invalidation replay"),
        invalidated
    );
    assert_eq!(
        issuer_db
            .reward_program_show(tenant, pending)
            .await
            .expect("pending invalidation releases capacity")["capacity_used_units"],
        json!(0)
    );
    issuer_db
        .reward_reserve(
            tenant,
            pending,
            Uuid::new_v4(),
            &digest("post-invalidation-participant"),
            &digest("post-invalidation-work"),
            &digest("post-invalidation-consent"),
        )
        .await
        .expect("released invalidated slot can reserve");

    let review_invalidation_race = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        1,
        1,
        1,
    )
    .await;
    let race_participant = digest("review-invalidation-race-participant");
    let (race_reservation, race_evidence) = reserve_and_submit(
        issuer_db,
        tenant,
        review_invalidation_race,
        &race_participant,
        "review-invalidation-race",
    )
    .await;
    let (review_race, invalidate_race) = tokio::join!(
        reviewer_db.reward_review(
            tenant,
            race_reservation,
            Uuid::new_v4(),
            true,
            "completion_verified"
        ),
        issuer_db.reward_invalidate(tenant, &race_evidence, Uuid::new_v4())
    );
    assert!(
        invalidate_race.is_ok(),
        "invalidation must serialize successfully"
    );
    let race_history = issuer_db
        .reward_history(tenant, &race_participant, 1)
        .await
        .expect("race history");
    let race_entry = history_entry(&race_history, race_reservation);
    match review_race {
        Ok(review) => {
            assert_eq!(review["state"], "awarded");
            assert_eq!(race_entry["state"], "awarded_invalidated");
            assert_eq!(race_entry["awarded_units"], json!(1));
        }
        Err(RewardError::EvidenceInvalidated | RewardError::StateConflict) => {
            assert_eq!(race_entry["state"], "invalidated");
            assert_eq!(race_entry["awarded_units"], json!(0));
        }
        Err(other) => panic!("review race must either win or observe invalidation: {other}"),
    }

    let history_participant = digest("limited-history-participant");
    let first_program = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        2,
        8,
        8,
    )
    .await;
    let other_program = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        3,
        6,
        6,
    )
    .await;
    award_submission(
        issuer_db,
        reviewer_db,
        tenant,
        first_program,
        &history_participant,
        "history-first",
    )
    .await;
    award_submission(
        issuer_db,
        reviewer_db,
        tenant,
        other_program,
        &history_participant,
        "history-other",
    )
    .await;
    let latest = award_submission(
        issuer_db,
        reviewer_db,
        tenant,
        first_program,
        &history_participant,
        "history-latest",
    )
    .await;
    admin
        .execute(
            "UPDATE trace_reward_reservations SET created_at = clock_timestamp() + interval '1 second' WHERE reservation_id = $1",
            &[&latest],
        )
        .await
        .expect("make the repeated-program award deterministically newest");
    let limited = issuer_db
        .reward_history(tenant, &history_participant, 1)
        .await
        .expect("read limited history");
    let entries = limited["entries"]
        .as_array()
        .expect("limited entries array");
    let programs = limited["programs"]
        .as_array()
        .expect("limited programs array");
    assert_eq!(entries.len(), 1);
    assert_eq!(programs.len(), 1);
    assert_eq!(limited["truncated"], true);
    assert_eq!(limited["programs_scope"], "visible_entries");
    assert_eq!(entries[0]["program_id"], first_program.to_string());
    assert_eq!(programs[0]["program_id"], first_program.to_string());
    assert_eq!(programs[0]["awarded_units"], json!(4));
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn reward_capacity_races_and_expiry_are_consistent() {
    let fixture = RewardPgFixture::new().await;
    let tenant = fixture.tenant.as_str();
    let issuer_db = &fixture.issuer_db;
    let reviewer_db = &fixture.reviewer_db;
    let admin = &fixture.admin;
    let runtime = fixture.runtime().await;
    let terminal = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        1,
        1,
        1,
    )
    .await;
    let (submitted, submitted_evidence) = reserve_and_submit(
        issuer_db,
        tenant,
        terminal,
        &digest("submitted-hold-participant"),
        "submitted-hold",
    )
    .await;
    admin.execute("UPDATE trace_reward_reservations SET expires_at = clock_timestamp() - interval '1 second' WHERE reservation_id = $1", &[&submitted]).await.expect("age submitted hold");
    assert_refusal(
        issuer_db
            .reward_reserve(
                tenant,
                terminal,
                Uuid::new_v4(),
                &digest("held-cap-participant"),
                &digest("held-cap-work"),
                &digest("held-cap-consent"),
            )
            .await,
        RewardError::CapacityExhausted,
    );
    let rejected_decision = Uuid::new_v4();
    reviewer_db
        .reward_review(
            tenant,
            submitted,
            rejected_decision,
            false,
            "rubric_not_met",
        )
        .await
        .expect("reject submitted claim");
    assert_refusal(
        reviewer_db
            .reward_review(
                tenant,
                submitted,
                rejected_decision,
                false,
                "evidence_incomplete",
            )
            .await,
        RewardError::PayloadConflict,
    );
    let after_rejection = Uuid::new_v4();
    issuer_db
        .reward_reserve(
            tenant,
            terminal,
            after_rejection,
            &digest("after-reject-participant"),
            &digest("after-reject-work"),
            &digest("after-reject-consent"),
        )
        .await
        .expect("rejection releases capacity");
    assert_refusal(
        issuer_db
            .reward_claim_submit(
                tenant,
                after_rejection,
                &submitted_evidence,
                &digest("after-reject-evaluation"),
            )
            .await,
        RewardError::EvidenceDuplicate,
    );
    assert_refusal(
        issuer_db
            .reward_reserve(
                tenant,
                terminal,
                Uuid::new_v4(),
                &digest("work-replay-participant"),
                &digest("submitted-hold-work"),
                &digest("work-replay-consent"),
            )
            .await,
        RewardError::WorkDuplicate,
    );

    issuer_db
        .reward_invalidate(tenant, &submitted_evidence, Uuid::new_v4())
        .await
        .expect("invalidate rejected evidence without rewriting the outcome");
    let rejected_history = issuer_db
        .reward_history(tenant, &digest("submitted-hold-participant"), 1)
        .await
        .unwrap();
    let rejected_entry = history_entry(&rejected_history, submitted);
    assert_eq!(rejected_entry["state"], "rejected");
    assert_eq!(rejected_entry["invalidated"], true);

    let one_slot = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        1,
        1,
        1,
    )
    .await;
    let (left, right) = tokio::join!(
        race_for_slot(issuer_db, tenant, one_slot, "race-a"),
        race_for_slot(issuer_db, tenant, one_slot, "race-b")
    );
    assert_eq!(
        [left.is_ok(), right.is_ok()]
            .into_iter()
            .filter(|ok| *ok)
            .count(),
        1,
        "last slot allocates once"
    );
    let participant_cap = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        3,
        9,
        3,
    )
    .await;
    let cap_participant = digest("cap-participant");
    issuer_db
        .reward_reserve(
            tenant,
            participant_cap,
            Uuid::new_v4(),
            &cap_participant,
            &digest("cap-work-a"),
            &digest("cap-consent-a"),
        )
        .await
        .expect("first participant allocation");
    assert_refusal(
        issuer_db
            .reward_reserve(
                tenant,
                participant_cap,
                Uuid::new_v4(),
                &cap_participant,
                &digest("cap-work-b"),
                &digest("cap-consent-b"),
            )
            .await,
        RewardError::ParticipantCap,
    );
    let capped = create_program(
        issuer_db,
        tenant,
        RewardActivityKind::MissionCompletion,
        i64::MAX,
        i64::MAX,
        i64::MAX,
    )
    .await;
    issuer_db
        .reward_reserve(
            tenant,
            capped,
            Uuid::new_v4(),
            &digest("max-participant"),
            &digest("max-work"),
            &digest("max-consent"),
        )
        .await
        .expect("i64 max reserve avoids overflow");

    let expiring = Uuid::new_v4();
    let mut short = terms(RewardActivityKind::MissionCompletion, 1, 2, 2);
    short.reservation_ttl_seconds = 1;
    issuer_db
        .reward_program_create(tenant, expiring, &short)
        .await
        .expect("create expiry program");
    let expired = Uuid::new_v4();
    issuer_db
        .reward_reserve(
            tenant,
            expiring,
            expired,
            &digest("expired-p"),
            &digest("expired-work"),
            &digest("expired-consent"),
        )
        .await
        .expect("reserve expiring hold");
    admin.execute("UPDATE trace_reward_reservations SET expires_at = clock_timestamp() - interval '1 second' WHERE reservation_id = $1", &[&expired]).await.expect("age held reservation");
    assert_refusal(
        issuer_db
            .reward_claim_submit(tenant, expired, &digest("expired-e"), &digest("expired-v"))
            .await,
        RewardError::ReservationExpired,
    );
    issuer_db
        .reward_cancel(tenant, expired, Uuid::new_v4())
        .await
        .expect("cancel expired unsubmitted reservation");

    let invalid_terms = json!({"schema_version": 1, "activity_kind": null});
    let invalid: Result<Value, tokio_postgres::Error> = runtime
        .query_one(
            "SELECT public.trace_reward_program_create($1, $2, $3)",
            &[&tenant, &Uuid::new_v4(), &invalid_terms],
        )
        .await
        .map(|r| r.get(0));
    assert!(
        invalid.is_err(),
        "direct SQL refuses null/unknown term types"
    );
}
