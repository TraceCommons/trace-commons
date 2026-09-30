// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: persisted deadline arithmetic for the V69 reward ledger.

use chrono::{DateTime, Duration, Utc};
use trace_commons_server::mission_rewards::RewardActivityKind;
use uuid::Uuid;

use crate::fixture::{RewardPgFixture, digest, terms};

fn json_timestamp(value: &serde_json::Value, field: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value[field].as_str().expect("timestamp result field"))
        .expect("RFC3339 result timestamp")
        .with_timezone(&Utc)
}

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn reward_deadlines_share_one_persisted_now_and_respect_program_close() {
    let fixture = RewardPgFixture::new().await;
    let mut ttl_terms = terms(RewardActivityKind::MissionCompletion, 1, 10, 10);
    ttl_terms.closes_at = Utc::now() + Duration::hours(2);
    ttl_terms.reservation_ttl_seconds = 60;
    let ttl_program = Uuid::new_v4();
    fixture
        .issuer_db
        .reward_program_create(&fixture.tenant, ttl_program, &ttl_terms)
        .await
        .expect("create TTL-bound program");
    let ttl_reservation = Uuid::new_v4();
    let ttl_result = fixture
        .issuer_db
        .reward_reserve(
            &fixture.tenant,
            ttl_program,
            ttl_reservation,
            &digest("ttl-participant"),
            &digest("ttl-work"),
            &digest("ttl-consent"),
        )
        .await
        .expect("reserve TTL-bound claim");
    let ttl_row = fixture
        .admin
        .query_one(
            "SELECT created_at, expires_at FROM trace_reward_reservations WHERE tenant_id = $1 AND reservation_id = $2",
            &[&fixture.tenant, &ttl_reservation],
        )
        .await
        .expect("read persisted TTL deadline");
    let created_at = ttl_row.get::<_, DateTime<Utc>>(0);
    let expires_at = ttl_row.get::<_, DateTime<Utc>>(1);
    assert_eq!(expires_at, created_at + Duration::seconds(60));
    assert_eq!(json_timestamp(&ttl_result, "expires_at"), expires_at);

    let submit_result = fixture
        .issuer_db
        .reward_claim_submit(
            &fixture.tenant,
            ttl_reservation,
            &digest("ttl-evidence"),
            &digest("ttl-evaluation"),
        )
        .await
        .expect("submit before TTL deadline");
    let submit_row = fixture
        .admin
        .query_one(
            "SELECT submitted_at, expires_at FROM trace_reward_reservations WHERE tenant_id = $1 AND reservation_id = $2",
            &[&fixture.tenant, &ttl_reservation],
        )
        .await
        .expect("read persisted submission");
    let submitted_at = submit_row.get::<_, DateTime<Utc>>(0);
    assert_eq!(json_timestamp(&submit_result, "submitted_at"), submitted_at);
    assert!(submitted_at < submit_row.get::<_, DateTime<Utc>>(1));

    let mut close_terms = terms(RewardActivityKind::MissionCompletion, 1, 10, 10);
    close_terms.closes_at = Utc::now() + Duration::minutes(5);
    close_terms.reservation_ttl_seconds = 600;
    let close_program = Uuid::new_v4();
    fixture
        .issuer_db
        .reward_program_create(&fixture.tenant, close_program, &close_terms)
        .await
        .expect("create close-bound program");
    let close_reservation = Uuid::new_v4();
    let close_result = fixture
        .issuer_db
        .reward_reserve(
            &fixture.tenant,
            close_program,
            close_reservation,
            &digest("close-participant"),
            &digest("close-work"),
            &digest("close-consent"),
        )
        .await
        .expect("reserve close-bound claim");
    let close_row = fixture
        .admin
        .query_one(
            "SELECT r.expires_at, p.closes_at FROM trace_reward_reservations r JOIN trace_reward_programs p ON p.tenant_id = r.tenant_id AND p.program_id = r.program_id WHERE r.tenant_id = $1 AND r.reservation_id = $2",
            &[&fixture.tenant, &close_reservation],
        )
        .await
        .expect("read persisted close deadline");
    let close_expiry = close_row.get::<_, DateTime<Utc>>(0);
    assert_eq!(close_expiry, close_row.get::<_, DateTime<Utc>>(1));
    assert_eq!(json_timestamp(&close_result, "expires_at"), close_expiry);
}
