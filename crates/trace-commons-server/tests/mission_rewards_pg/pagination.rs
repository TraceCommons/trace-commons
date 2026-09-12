// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: exercises V69 history keyset pagination through the public PostgreSQL boundary.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use trace_commons_server::mission_rewards::{RewardActivityKind, RewardError};
use uuid::Uuid;

use crate::fixture::{RewardPgFixture, assert_refusal, cli, create_program, digest, role_url};

fn fixed_uuid(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn entry_ids(history: &Value) -> Vec<Uuid> {
    history["entries"]
        .as_array()
        .expect("history entries array")
        .iter()
        .map(|entry| {
            Uuid::parse_str(
                entry["reservation_id"]
                    .as_str()
                    .expect("reservation id string"),
            )
            .expect("reservation id UUID")
        })
        .collect()
}

fn program_balance(history: &Value, program: Uuid) -> &Value {
    history["programs"]
        .as_array()
        .expect("history programs array")
        .iter()
        .find(|entry| entry["program_id"] == program.to_string())
        .expect("page includes visible program balance")
}

async fn reserve(
    fixture: &RewardPgFixture,
    program: Uuid,
    reservation: Uuid,
    participant: &str,
    seed: &str,
) -> String {
    fixture
        .issuer_db
        .reward_reserve(
            &fixture.tenant,
            program,
            reservation,
            participant,
            &digest(&format!("{seed}-work")),
            &digest(&format!("{seed}-consent")),
        )
        .await
        .expect("reserve deterministic fixture claim");
    digest(&format!("{seed}-evidence"))
}

async fn submit_and_award(
    fixture: &RewardPgFixture,
    reservation: Uuid,
    evidence: &str,
    seed: &str,
) {
    fixture
        .issuer_db
        .reward_claim_submit(
            &fixture.tenant,
            reservation,
            evidence,
            &digest(&format!("{seed}-evaluation")),
        )
        .await
        .expect("submit fixture claim");
    fixture
        .reviewer_db
        .reward_review(
            &fixture.tenant,
            reservation,
            Uuid::new_v4(),
            true,
            "completion_verified",
        )
        .await
        .expect("award fixture claim");
}

async fn tie_created_at(fixture: &RewardPgFixture, reservations: &[Uuid], at: DateTime<Utc>) {
    for reservation in reservations {
        fixture
            .admin
            .execute(
                "UPDATE public.trace_reward_reservations SET created_at = $1 \
                 WHERE tenant_id = $2 AND reservation_id = $3",
                &[&at, &fixture.tenant, reservation],
            )
            .await
            .expect("set fixture tie only");
    }
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn history_keyset_pages_are_complete_stable_and_tenant_scoped() {
    let fixture = RewardPgFixture::new().await;
    let participant = digest("pagination-participant");
    let awarded_program = create_program(
        &fixture.issuer_db,
        &fixture.tenant,
        RewardActivityKind::MissionCompletion,
        7,
        28,
        28,
    )
    .await;
    let zero_balance_program = create_program(
        &fixture.issuer_db,
        &fixture.tenant,
        RewardActivityKind::InsightContribution,
        5,
        10,
        10,
    )
    .await;
    let invalidated = fixed_uuid(1);
    let awarded = fixed_uuid(2);
    let zero_balance = fixed_uuid(3);

    let invalidated_evidence = reserve(
        &fixture,
        awarded_program,
        invalidated,
        &participant,
        "pagination-invalidated",
    )
    .await;
    submit_and_award(
        &fixture,
        invalidated,
        &invalidated_evidence,
        "pagination-invalidated",
    )
    .await;
    fixture
        .issuer_db
        .reward_invalidate(&fixture.tenant, &invalidated_evidence, Uuid::new_v4())
        .await
        .expect("invalidate awarded fixture evidence");
    let awarded_evidence = reserve(
        &fixture,
        awarded_program,
        awarded,
        &participant,
        "pagination-awarded",
    )
    .await;
    submit_and_award(&fixture, awarded, &awarded_evidence, "pagination-awarded").await;
    reserve(
        &fixture,
        zero_balance_program,
        zero_balance,
        &participant,
        "pagination-zero-balance",
    )
    .await;

    let tied_at = fixture
        .admin
        .query_one("SELECT pg_catalog.clock_timestamp()", &[])
        .await
        .expect("read fixture tie timestamp")
        .get::<_, DateTime<Utc>>(0);
    tie_created_at(&fixture, &[invalidated, awarded, zero_balance], tied_at).await;

    let first = fixture
        .issuer_db
        .reward_history_page(&fixture.tenant, &participant, 2, None)
        .await
        .expect("read first keyset page");
    assert_eq!(entry_ids(&first), vec![zero_balance, awarded]);
    assert_eq!(first["truncated"], true);
    assert_eq!(first["next_cursor"], awarded.to_string());
    assert_eq!(
        program_balance(&first, awarded_program)["awarded_units"],
        json!(14),
        "visible program carries the participant's full award total"
    );
    assert_eq!(
        program_balance(&first, zero_balance_program)["awarded_units"],
        json!(0),
        "a visible unawarded program remains visible"
    );
    assert_eq!(
        program_balance(&first, awarded_program)["identity_mode"],
        "operator_asserted"
    );
    assert_eq!(
        fixture
            .admin
            .query_one(
                "SELECT identity_mode FROM public.trace_reward_programs \
                 WHERE tenant_id = $1 AND program_id = $2",
                &[&fixture.tenant, &awarded_program],
            )
            .await
            .expect("read persisted program identity mode")
            .get::<_, String>(0),
        "operator_asserted"
    );

    let inserted_between_pages = fixed_uuid(4);
    reserve(
        &fixture,
        zero_balance_program,
        inserted_between_pages,
        &participant,
        "pagination-inserted-between-pages",
    )
    .await;
    tie_created_at(&fixture, &[inserted_between_pages], tied_at).await;
    let second = fixture
        .issuer_db
        .reward_history_page(&fixture.tenant, &participant, 2, Some(awarded))
        .await
        .expect("read second keyset page");
    assert_eq!(entry_ids(&second), vec![invalidated]);
    assert_eq!(second["truncated"], false);
    assert!(second["next_cursor"].is_null(), "final page has no cursor");
    assert_eq!(
        program_balance(&second, awarded_program)["awarded_units"],
        json!(14),
        "the final page retains the full total including invalidated awards"
    );
    assert_eq!(second["entries"][0]["state"], "awarded_invalidated");
    assert!(
        !entry_ids(&first)
            .iter()
            .any(|id| entry_ids(&second).contains(id)),
        "keyset pages do not overlap"
    );
    assert!(
        !entry_ids(&second).contains(&inserted_between_pages),
        "newer insert after the cursor cannot duplicate or displace old records"
    );

    let after_last = fixture
        .issuer_db
        .reward_history_page(&fixture.tenant, &participant, 2, Some(invalidated))
        .await
        .expect("read page after last cursor");
    assert_eq!(entry_ids(&after_last), Vec::<Uuid>::new());
    assert!(after_last["next_cursor"].is_null());
    assert_eq!(after_last["truncated"], false);

    let other_participant_cursor = fixed_uuid(5);
    reserve(
        &fixture,
        awarded_program,
        other_participant_cursor,
        &digest("pagination-other-participant"),
        "pagination-other-participant",
    )
    .await;
    assert_refusal(
        fixture
            .issuer_db
            .reward_history_page(
                &fixture.tenant,
                &participant,
                2,
                Some(other_participant_cursor),
            )
            .await,
        RewardError::NotFound,
    );
    assert_refusal(
        fixture
            .issuer_db
            .reward_history_page(&fixture.tenant, &participant, 2, Some(fixed_uuid(99)))
            .await,
        RewardError::NotFound,
    );
    assert_refusal(
        fixture
            .issuer_db
            .reward_history_page("wrong-tenant", &participant, 2, None)
            .await,
        RewardError::Unauthorized,
    );

    let other_tenant = RewardPgFixture::new().await;
    let other_program = create_program(
        &other_tenant.issuer_db,
        &other_tenant.tenant,
        RewardActivityKind::MissionCompletion,
        1,
        10,
        10,
    )
    .await;
    let other_old = fixed_uuid(0);
    reserve(
        &other_tenant,
        other_program,
        other_old,
        &participant,
        "pagination-other-tenant-old",
    )
    .await;
    reserve(
        &other_tenant,
        other_program,
        awarded,
        &participant,
        "pagination-other-tenant-same-cursor",
    )
    .await;
    let other_tied_at = other_tenant
        .admin
        .query_one("SELECT pg_catalog.clock_timestamp()", &[])
        .await
        .expect("read other tenant tie timestamp")
        .get::<_, DateTime<Utc>>(0);
    tie_created_at(&other_tenant, &[other_old, awarded], other_tied_at).await;
    let other_page = other_tenant
        .issuer_db
        .reward_history_page(&other_tenant.tenant, &participant, 2, Some(awarded))
        .await
        .expect("tenant-local same UUID cursor");
    assert_eq!(entry_ids(&other_page), vec![other_old]);
    assert_refusal(
        other_tenant
            .issuer_db
            .reward_history_page(&other_tenant.tenant, &participant, 2, Some(invalidated))
            .await,
        RewardError::NotFound,
    );
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn cli_history_pages_with_before_return_distinct_records_and_full_balances() {
    let fixture = RewardPgFixture::new().await;
    let participant = digest("pagination-cli-participant");
    let program = create_program(
        &fixture.issuer_db,
        &fixture.tenant,
        RewardActivityKind::MissionCompletion,
        3,
        6,
        6,
    )
    .await;
    for seed in ["pagination-cli-first", "pagination-cli-second"] {
        let reservation = Uuid::new_v4();
        let evidence = reserve(&fixture, program, reservation, &participant, seed).await;
        submit_and_award(&fixture, reservation, &evidence, seed).await;
    }

    let issuer_url = role_url(&fixture.url, &fixture.issuer);
    let first = cli(
        &issuer_url,
        &fixture.tenant,
        &[
            "history".into(),
            "--participant-hash".into(),
            participant.clone(),
            "--limit".into(),
            "1".into(),
        ],
        true,
    )
    .await;
    let cursor = Uuid::parse_str(first["next_cursor"].as_str().expect("CLI next cursor"))
        .expect("CLI next cursor UUID");
    assert_eq!(first["truncated"], true);
    assert_eq!(program_balance(&first, program)["awarded_units"], json!(6));

    let second = cli(
        &issuer_url,
        &fixture.tenant,
        &[
            "history".into(),
            "--participant-hash".into(),
            participant,
            "--limit".into(),
            "1".into(),
            "--before".into(),
            cursor.to_string(),
        ],
        true,
    )
    .await;
    assert_eq!(second["truncated"], false);
    assert!(second["next_cursor"].is_null());
    assert_ne!(entry_ids(&first), entry_ids(&second));
    assert_eq!(program_balance(&second, program)["awarded_units"], json!(6));
}
