// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use serde_json::json;
use trace_commons_server::mission_rewards::RewardActivityKind;
use uuid::Uuid;

use crate::fixture::{RewardPgFixture, cli, digest, history_entry, role_url, terms};

#[tokio::test]
#[ignore = "requires PostgreSQL 16+ at isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn reward_operator_cli_uses_real_login_roles() {
    let fixture = RewardPgFixture::new().await;
    let tenant = fixture.tenant.as_str();
    let url = fixture.url.as_str();
    let issuer = fixture.issuer.as_str();
    let reviewer = fixture.reviewer.as_str();
    let cli_program = Uuid::new_v4();
    let cli_terms = std::env::temp_dir().join(format!("reward-terms-{}.json", Uuid::new_v4()));
    std::fs::write(
        &cli_terms,
        serde_json::to_vec(&terms(RewardActivityKind::MissionCompletion, 2, 4, 4))
            .expect("serialize terms"),
    )
    .expect("write temporary terms");
    let cli_issuer_url = role_url(url, issuer);
    let cli_reviewer_url = role_url(url, reviewer);
    let create = vec![
        "program-create".into(),
        "--program".into(),
        cli_program.to_string(),
        "--terms".into(),
        cli_terms.display().to_string(),
    ];
    let cli_created = cli(&cli_issuer_url, tenant, &create, true).await;
    assert_eq!(cli_created["program_id"], cli_program.to_string());
    assert_eq!(cli_created["redemption"], "none");
    let cli_shown = cli(
        &cli_issuer_url,
        tenant,
        &[
            "program-show".into(),
            "--program".into(),
            cli_program.to_string(),
        ],
        true,
    )
    .await;
    assert_eq!(cli_shown["capacity_units"], json!(4));
    assert_eq!(cli_shown["capacity_used_units"], json!(0));
    let cli_reservation = Uuid::new_v4();
    let cli_reserved = cli(
        &cli_issuer_url,
        tenant,
        &[
            "reserve".into(),
            "--program".into(),
            cli_program.to_string(),
            "--reservation".into(),
            cli_reservation.to_string(),
            "--participant-hash".into(),
            digest("cli-p"),
            "--work-hash".into(),
            digest("cli-work"),
            "--consent-hash".into(),
            digest("cli-consent"),
        ],
        true,
    )
    .await;
    assert_eq!(cli_reserved["state"], "reserved");
    assert_eq!(cli_reserved["award_units"], json!(2));
    let cli_submitted = cli(
        &cli_issuer_url,
        tenant,
        &[
            "claim-submit".into(),
            "--reservation".into(),
            cli_reservation.to_string(),
            "--evidence-hash".into(),
            digest("cli-evidence"),
            "--evaluation-hash".into(),
            digest("cli-evaluation"),
        ],
        true,
    )
    .await;
    assert_eq!(cli_submitted["state"], "submitted");
    let cli_reviewed = cli(
        &cli_reviewer_url,
        tenant,
        &[
            "review".into(),
            "--reservation".into(),
            cli_reservation.to_string(),
            "--decision".into(),
            Uuid::new_v4().to_string(),
            "--accept".into(),
            "true".into(),
            "--reason".into(),
            "completion_verified".into(),
        ],
        true,
    )
    .await;
    assert_eq!(cli_reviewed["state"], "awarded");
    assert_eq!(cli_reviewed["award_units"], json!(2));
    let cli_history = cli(
        &cli_reviewer_url,
        tenant,
        &[
            "history".into(),
            "--participant-hash".into(),
            digest("cli-p"),
            "--limit".into(),
            "1".into(),
        ],
        true,
    )
    .await;
    assert_eq!(
        history_entry(&cli_history, cli_reservation)["awarded_units"],
        json!(2)
    );
    let cancellable = Uuid::new_v4();
    fixture
        .issuer_db
        .reward_reserve(
            tenant,
            cli_program,
            cancellable,
            &digest("cli-p"),
            &digest("cli-cancel-work"),
            &digest("cli-cancel-consent"),
        )
        .await
        .expect("reserve claim for CLI cancellation");
    let cancelled = cli(
        &cli_issuer_url,
        tenant,
        &[
            "cancel".into(),
            "--reservation".into(),
            cancellable.to_string(),
            "--decision".into(),
            Uuid::new_v4().to_string(),
        ],
        true,
    )
    .await;
    assert_eq!(cancelled["state"], "cancelled");
    let invalidated = cli(
        &cli_issuer_url,
        tenant,
        &[
            "invalidate".into(),
            "--evidence-hash".into(),
            digest("cli-evidence"),
            "--decision".into(),
            Uuid::new_v4().to_string(),
        ],
        true,
    )
    .await;
    assert_eq!(invalidated["evidence_invalidated"], true);
    let terminal_history = cli(
        &cli_issuer_url,
        tenant,
        &[
            "history".into(),
            "--participant-hash".into(),
            digest("cli-p"),
        ],
        true,
    )
    .await;
    assert_eq!(
        history_entry(&terminal_history, cli_reservation)["state"],
        "awarded_invalidated"
    );
    assert_eq!(terminal_history["programs"][0]["awarded_units"], json!(2));
    assert_eq!(
        history_entry(&terminal_history, cancellable)["state"],
        "cancelled"
    );
    cli(&cli_reviewer_url, tenant, &create, false).await;
    std::fs::remove_file(cli_terms).expect("remove temporary terms");
}
