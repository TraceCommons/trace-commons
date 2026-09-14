// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: run with an isolated TRACE_COMMONS_REWARDS_PG_TEST_URL and --ignored.

#[allow(dead_code)]
#[path = "mission_rewards_pg/fixture.rs"]
mod fixture;

use chrono::{Duration, Utc};
use serde_json::json;
use trace_commons_server::reward_participant::RewardReservationState;
use trace_commons_server::{
    config::{DatabaseConfig, SslMode},
    db::{Database, postgres::PgBackend},
    error::DatabaseError,
    mission_rewards::{RewardActivityKind, RewardError, RewardProgramTerms},
    reward_participant::{
        RewardHistoryQuery, RewardOffer, RewardOfferManifest, RewardReservationRequest,
    },
};
use uuid::Uuid;

use crate::fixture::{RewardPgFixture, cli, digest, role_url};

struct ParticipantFixture {
    ledger: RewardPgFixture,
    runtime: PgBackend,
    login: String,
    accounts: [Uuid; 3],
}

impl ParticipantFixture {
    async fn new() -> Self {
        let ledger = RewardPgFixture::new().await;
        let login = format!("reward_participant_{}", ledger.suffix);
        ledger
            .admin
            .batch_execute(&format!(
                "CREATE ROLE {login} LOGIN NOSUPERUSER NOBYPASSRLS;
             GRANT trace_reward_participant_runtime TO {login};"
            ))
            .await
            .expect("create participant runtime");
        ledger.admin.execute(
            "INSERT INTO trace_reward_participant_logins(tenant_id,login_role) VALUES($1,$2::name)",
            &[&ledger.tenant, &login],
        ).await.expect("provision participant tenant");
        let accounts = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        for account in accounts {
            ledger
                .admin
                .execute(
                    "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
                    &[&ledger.tenant, &account],
                )
                .await
                .expect("create account");
        }
        let runtime = PgBackend::new(&DatabaseConfig {
            url: role_url(&ledger.url, &login).into(),
            pool_size: 4,
            ssl_mode: SslMode::Prefer,
            login_resolver_url: None,
            gate_driver_url: None,
            pii_backstop_driver_url: None,
            invite_registry_url: None,
        })
        .await
        .expect("participant runtime connects");
        Self {
            ledger,
            runtime,
            login,
            accounts,
        }
    }

    async fn offer(&self, definition: &str, capacity: i64, cap: i64) -> RewardOffer {
        let (terms, manifest) = offer_terms(definition, capacity, cap);
        self.ledger
            .issuer_db
            .reward_offer_publish(&self.ledger.tenant, Uuid::new_v4(), &terms, &manifest)
            .await
            .expect("publish intelligible offer")
    }

    async fn reserve(
        &self,
        account: usize,
        offer: &RewardOffer,
    ) -> trace_commons_server::reward_participant::RewardReservation {
        self.runtime
            .reserve_reward_offer(
                &self.ledger.tenant,
                self.accounts[account],
                offer.program_id,
                &request(offer),
            )
            .await
            .expect("reserve own offer")
    }

    async fn history(
        &self,
        account: usize,
    ) -> trace_commons_server::reward_participant::RewardParticipantHistory {
        self.runtime
            .get_reward_history(
                &self.ledger.tenant,
                self.accounts[account],
                &RewardHistoryQuery {
                    limit: 100,
                    before: None,
                },
            )
            .await
            .expect("own history")
    }

    async fn grant_account_merge_privileges(&self) {
        self.ledger
            .admin
            .batch_execute(&format!(
                "GRANT SELECT,INSERT,UPDATE ON trace_tenants TO {0};
                 GRANT SELECT,UPDATE ON trace_accounts,trace_account_merge_proposals,trace_account_principals,
                     trace_webauthn_credentials,trace_near_identities,trace_public_runs,trace_sessions TO {0};
                 GRANT INSERT ON trace_account_audit TO {0};
                 GRANT USAGE ON SEQUENCE trace_account_audit_audit_sequence_seq TO {0};",
                self.login
            ))
            .await
            .expect("grant existing account merge privileges");
    }

    /// How many distinct reward principals the given accounts resolve to. Two
    /// means a merge left the alias groups apart.
    async fn reward_principals(&self, accounts: [usize; 2]) -> i64 {
        self.ledger
            .admin
            .query_one(
                "SELECT count(DISTINCT reward_principal_id) FROM trace_reward_principal_accounts \
                 WHERE tenant_id=$1 AND account_id IN ($2,$3)",
                &[
                    &self.ledger.tenant,
                    &self.accounts[accounts[0]],
                    &self.accounts[accounts[1]],
                ],
            )
            .await
            .expect("count reward principals")
            .get(0)
    }

    async fn proposal_consumed_at(&self, proposal: Uuid) -> Option<chrono::DateTime<Utc>> {
        self.ledger
            .admin
            .query_one(
                "SELECT consumed_at FROM trace_account_merge_proposals \
                 WHERE tenant_id=$1 AND proposal_id=$2",
                &[&self.ledger.tenant, &proposal],
            )
            .await
            .expect("read merge proposal")
            .get(0)
    }

    async fn merge_proposal(&self, surviving: usize, absorbed: usize) -> Uuid {
        let proposal = Uuid::new_v4();
        self.ledger.admin.execute("INSERT INTO trace_account_merge_proposals(tenant_id,proposal_id,surviving_account_id,absorbed_account_id,expires_at) VALUES($1,$2,$3,$4,now()+interval '10 minutes')", &[&self.ledger.tenant,&proposal,&self.accounts[surviving],&self.accounts[absorbed]]).await.expect("insert merge proposal");
        proposal
    }
}

fn offer_terms(
    definition: &str,
    capacity: i64,
    cap: i64,
) -> (RewardProgramTerms, RewardOfferManifest) {
    let manifest = RewardOfferManifest {
        schema_version: 1,
        definition: definition.into(),
        rubric: "All required tests pass.".into(),
        required_evidence: "Provide the complete reviewed attempt set.".into(),
        rights: "Reward review only; corpus contribution remains optional.".into(),
        challenge_policy: "Pilot rejection releases its allocation.".into(),
        evaluator_policy: "An independent reviewer evaluates the recorded evidence.".into(),
        reservation_terms: "Reserve capacity for this version; no evidence is submitted.".into(),
    };
    let terms = RewardProgramTerms {
        schema_version: 1,
        activity_kind: RewardActivityKind::MissionCompletion,
        definition_hash: digest(&manifest.definition),
        rubric_hash: digest(&manifest.rubric),
        evaluator_policy_hash: digest(&manifest.evaluator_policy),
        required_evidence_hash: digest(&manifest.required_evidence),
        rights_hash: digest(&manifest.rights),
        challenge_policy_hash: digest(&manifest.challenge_policy),
        sponsor_hash: digest("synthetic sponsor"),
        award_units: 7,
        capacity_units: capacity,
        participant_cap_units: cap,
        closes_at: Utc::now() + Duration::hours(1),
        reservation_ttl_seconds: 600,
    };
    (terms, manifest)
}

fn request(offer: &RewardOffer) -> RewardReservationRequest {
    RewardReservationRequest {
        reservation_id: Uuid::new_v4(),
        offer_version_hash: offer.offer_version_hash.clone(),
    }
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_offer_cli_publishes_and_suspends_with_issuer_authority() {
    let f = ParticipantFixture::new().await;
    let files = tempfile::tempdir().unwrap();
    let terms_file = files.path().join("terms.json");
    let manifest_file = files.path().join("manifest.json");
    let (terms, manifest) = offer_terms("A published CLI offer.", 70, 14);
    std::fs::write(&terms_file, serde_json::to_vec(&terms).unwrap()).unwrap();
    std::fs::write(&manifest_file, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let program = Uuid::new_v4();
    let args = vec![
        "offer-publish".into(),
        "--program".into(),
        program.to_string(),
        "--terms".into(),
        terms_file.display().to_string(),
        "--manifest".into(),
        manifest_file.display().to_string(),
    ];
    let url = role_url(&f.ledger.url, &f.ledger.issuer);
    let published = cli(&url, &f.ledger.tenant, &args, true).await;
    assert_eq!(published["award_units"], "7");
    assert_eq!(published["manifest"]["definition"], manifest.definition);
    assert_eq!(published, cli(&url, &f.ledger.tenant, &args, true).await);
    cli(
        &role_url(&f.ledger.url, &f.ledger.reviewer),
        &f.ledger.tenant,
        &args,
        false,
    )
    .await;
    let paused = cli(
        &url,
        &f.ledger.tenant,
        &[
            "offer-suspend".into(),
            "--program".into(),
            program.to_string(),
            "--suspended".into(),
            "true".into(),
        ],
        true,
    )
    .await;
    assert_eq!(paused["suspended"], true);
    assert_eq!(
        paused["offer_version_hash"],
        published["offer_version_hash"]
    );
    assert!(f.runtime.get_reward_offer(program).await.unwrap().suspended);
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_history_tracks_review_invalidation_and_expiry_without_delivery() {
    let f = ParticipantFixture::new().await;
    let tenant = &f.ledger.tenant;
    let offer = f.offer("Review the synthetic evidence.", 70, 14).await;
    let reservation = f.reserve(0, &offer).await;
    let evidence = digest("participant evidence");
    assert_eq!(reservation.state, RewardReservationState::Reserved);
    assert_eq!(reservation.awarded_units, 0);
    assert!(
        f.runtime
            .reward_review(
                tenant,
                reservation.reservation_id,
                Uuid::new_v4(),
                true,
                "completion_verified"
            )
            .await
            .is_err()
    );
    f.ledger
        .issuer_db
        .reward_claim_submit(
            tenant,
            reservation.reservation_id,
            &evidence,
            &digest("evaluation"),
        )
        .await
        .unwrap();
    assert_eq!(
        f.history(0).await.entries[0].state,
        RewardReservationState::Submitted
    );
    f.ledger
        .reviewer_db
        .reward_review(
            tenant,
            reservation.reservation_id,
            Uuid::new_v4(),
            true,
            "completion_verified",
        )
        .await
        .unwrap();
    let awarded = f.history(0).await;
    assert_eq!(awarded.awarded_units, "7");
    assert_eq!(awarded.entries[0].state, RewardReservationState::Awarded);
    assert_eq!(awarded.entries[0].decisions.len(), 1);
    let payload = serde_json::to_value(&awarded).unwrap();
    assert!(payload.get("available_units").is_none());
    assert!(payload.get("balance").is_none());
    f.ledger
        .issuer_db
        .reward_invalidate(tenant, &evidence, Uuid::new_v4())
        .await
        .unwrap();
    let invalidated = f.history(0).await;
    assert_eq!(invalidated.awarded_units, "7");
    assert_eq!(
        invalidated.entries[0].state,
        RewardReservationState::AwardedInvalidated
    );
    assert!(invalidated.entries[0].invalidated);
    assert_eq!(invalidated.entries[0].decisions.len(), 2);
    assert_eq!(
        f.runtime
            .get_reward_offer(offer.program_id)
            .await
            .unwrap()
            .capacity_used_units,
        7
    );

    let expiring = f.offer("A reservation that expires.", 7, 7).await;
    let held = f.reserve(0, &expiring).await;
    f.ledger.admin.execute("UPDATE trace_reward_reservations SET expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND reservation_id=$2", &[tenant, &held.reservation_id]).await.unwrap();
    assert_eq!(
        f.runtime
            .get_reward_reservation(tenant, f.accounts[0], held.reservation_id)
            .await
            .unwrap()
            .state,
        RewardReservationState::Expired
    );
    assert_eq!(
        f.runtime
            .get_reward_offer(expiring.program_id)
            .await
            .unwrap()
            .capacity_used_units,
        0
    );
    f.reserve(1, &expiring).await;
    let replay = RewardReservationRequest {
        reservation_id: held.reservation_id,
        offer_version_hash: expiring.offer_version_hash.clone(),
    };
    assert_eq!(
        f.runtime
            .reserve_reward_offer(tenant, f.accounts[0], expiring.program_id, &replay)
            .await
            .unwrap()
            .state,
        RewardReservationState::Expired
    );
    assert_eq!(
        f.runtime
            .get_reward_offer(expiring.program_id)
            .await
            .unwrap()
            .capacity_used_units,
        7
    );
    let credits: i64 = f
        .ledger
        .admin
        .query_one(
            "SELECT count(*) FROM trace_credit_ledger WHERE tenant_id=$1",
            &[tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(credits, 0);
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_publication_is_immutable_and_runtime_has_no_operator_authority() {
    let f = ParticipantFixture::new().await;
    let tenant = &f.ledger.tenant;
    let program = Uuid::new_v4();
    let (terms, manifest) = offer_terms("Inspect the synthetic implementation.", 70, 14);
    assert_eq!(
        f.runtime
            .reward_offer_publish(tenant, program, &terms, &manifest)
            .await
            .unwrap_err(),
        RewardError::StoreUnavailable
    );
    assert!(
        f.ledger
            .reviewer_db
            .reward_offer_publish(tenant, program, &terms, &manifest)
            .await
            .is_err()
    );
    let mut mismatched = manifest.clone();
    mismatched.rubric.push_str(" Changed.");
    assert_eq!(
        f.ledger
            .issuer_db
            .reward_offer_publish(tenant, program, &terms, &mismatched)
            .await
            .unwrap_err(),
        RewardError::PayloadConflict
    );
    let created = f
        .ledger
        .issuer_db
        .reward_offer_publish(tenant, program, &terms, &manifest)
        .await
        .unwrap();
    let replay = f
        .ledger
        .issuer_db
        .reward_offer_publish(tenant, program, &terms, &manifest)
        .await
        .unwrap();
    assert_eq!(created.offer_version_hash, replay.offer_version_hash);
    let operator_view = f
        .ledger
        .issuer_db
        .reward_program_show(tenant, program)
        .await
        .unwrap();
    assert_eq!(operator_view["identity_mode"], "account_bound");
    let operator_replay = f
        .ledger
        .issuer_db
        .reward_program_create(tenant, program, &terms)
        .await
        .unwrap();
    assert_eq!(operator_replay["identity_mode"], "account_bound");
    let public = f.runtime.get_reward_offer(program).await.unwrap();
    assert_eq!(public.manifest, manifest);
    assert_eq!(public.capacity_used_units, 0);
    assert_eq!(
        f.runtime
            .get_reward_offer(Uuid::new_v4())
            .await
            .unwrap_err(),
        RewardError::NotFound
    );
    assert!(
        f.ledger
            .no_grant_db
            .get_reward_offer(program)
            .await
            .is_err()
    );

    let mut altered = manifest.clone();
    altered.reservation_terms.push_str(" Altered.");
    assert_eq!(
        f.ledger
            .issuer_db
            .reward_offer_publish(tenant, program, &terms, &altered)
            .await
            .unwrap_err(),
        RewardError::PayloadConflict
    );
    let pilot = Uuid::new_v4();
    f.ledger
        .issuer_db
        .reward_program_create(tenant, pilot, &terms)
        .await
        .unwrap();
    assert!(
        f.ledger
            .issuer_db
            .reward_offer_publish(tenant, pilot, &terms, &manifest)
            .await
            .is_err()
    );
    assert_eq!(
        f.runtime.get_reward_offer(pilot).await.unwrap_err(),
        RewardError::NotFound
    );

    // Direct SQL privileges establish the boundary independently of HTTP checks.
    let runtime = f
        .runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    for function in [
        "trace_reward_program_create(text,uuid,jsonb)",
        "trace_reward_reserve(text,uuid,uuid,text,text,text)",
        "trace_reward_claim_submit(text,uuid,text,text)",
        "trace_reward_review(text,uuid,uuid,boolean,text)",
        "trace_reward_cancel(text,uuid,uuid)",
        "trace_reward_invalidate(text,text,uuid)",
    ] {
        assert!(
            !runtime
                .query_one(
                    "SELECT has_function_privilege(current_user,$1,'EXECUTE')",
                    &[&function]
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
    }
    assert!(
        !runtime
            .query_one(
                "SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname=current_user",
                &[]
            )
            .await
            .unwrap()
            .get::<_, bool>(0)
    );
    for table in [
        "trace_reward_offers",
        "trace_reward_offer_controls",
        "trace_reward_participant_logins",
        "trace_reward_principals",
        "trace_reward_principal_accounts",
        "trace_reward_participant_reservations",
    ] {
        assert!(
            !runtime
                .query_one(
                    "SELECT has_table_privilege(current_user,$1,'SELECT,INSERT,UPDATE,DELETE')",
                    &[&table]
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
        assert!(f.ledger.admin.query_one("SELECT relrowsecurity AND relforcerowsecurity FROM pg_class WHERE oid=$1::text::regclass", &[&table]).await.unwrap().get::<_,bool>(0));
    }
    for table in [
        "trace_reward_reservations",
        "trace_reward_awards",
        "trace_reward_decisions",
    ] {
        assert!(
            !runtime
                .query_one(
                    "SELECT has_table_privilege(current_user,$1,'SELECT')",
                    &[&table]
                )
                .await
                .unwrap()
                .get::<_, bool>(0)
        );
    }
    assert!(f.history(0).await.entries.is_empty());
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_capacity_replay_and_ownership_survive_races() {
    let f = ParticipantFixture::new().await;
    let offer = f
        .offer("Complete the single-slot synthetic task.", 7, 7)
        .await;
    let a = request(&offer);
    let b = request(&offer);
    let (left, right) = tokio::join!(
        f.runtime
            .reserve_reward_offer(&f.ledger.tenant, f.accounts[0], offer.program_id, &a),
        f.runtime
            .reserve_reward_offer(&f.ledger.tenant, f.accounts[1], offer.program_id, &b),
    );
    assert_ne!(
        left.is_ok(),
        right.is_ok(),
        "exactly one participant consumes the slot"
    );
    let (owner, loser, original, reserved) = match (left, right) {
        (Ok(reserved), Err(RewardError::CapacityExhausted)) => (0, 1, a, reserved),
        (Err(RewardError::CapacityExhausted), Ok(reserved)) => (1, 0, b, reserved),
        other => panic!("unexpected capacity race: {other:?}"),
    };
    f.ledger
        .issuer_db
        .reward_offer_suspend(&f.ledger.tenant, offer.program_id, true)
        .await
        .unwrap();
    let replay = f
        .runtime
        .reserve_reward_offer(
            &f.ledger.tenant,
            f.accounts[owner],
            offer.program_id,
            &original,
        )
        .await
        .unwrap();
    assert_eq!(replay.reservation_id, reserved.reservation_id);
    let mut changed = original.clone();
    changed.offer_version_hash = digest("changed version");
    assert_eq!(
        f.runtime
            .reserve_reward_offer(
                &f.ledger.tenant,
                f.accounts[owner],
                offer.program_id,
                &changed
            )
            .await
            .unwrap_err(),
        RewardError::PayloadConflict
    );
    assert_eq!(
        f.runtime
            .get_reward_reservation(&f.ledger.tenant, f.accounts[loser], reserved.reservation_id)
            .await
            .unwrap_err(),
        RewardError::NotFound
    );
    assert_eq!(
        f.runtime
            .get_reward_history(
                &f.ledger.tenant,
                f.accounts[loser],
                &RewardHistoryQuery {
                    limit: 1,
                    before: Some(reserved.reservation_id)
                }
            )
            .await
            .unwrap_err(),
        RewardError::NotFound
    );
    assert_eq!(
        f.runtime
            .reserve_reward_offer(
                &f.ledger.tenant,
                f.accounts[loser],
                offer.program_id,
                &request(&offer)
            )
            .await
            .unwrap_err(),
        RewardError::OfferSuspended
    );
    assert!(
        f.runtime
            .get_reward_history(
                "another-tenant",
                f.accounts[owner],
                &RewardHistoryQuery {
                    limit: 1,
                    before: None
                }
            )
            .await
            .is_err()
    );
    assert_eq!(f.history(owner).await.entries.len(), 1);
    assert_eq!(f.history(owner).await.awarded_units, "0");
    assert_eq!(
        f.runtime
            .get_reward_offer(offer.program_id)
            .await
            .unwrap()
            .capacity_used_units,
        7
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_foreign_reservation_replays_are_not_found() {
    let f = ParticipantFixture::new().await;
    let offer = f.offer("Foreign replay ownership fixture.", 70, 14).await;
    let reserved = f.reserve(0, &offer).await;
    let replay = RewardReservationRequest {
        reservation_id: reserved.reservation_id,
        offer_version_hash: offer.offer_version_hash.clone(),
    };
    // Cover both branches: a caller without enrollment and an unrelated
    // enrolled principal must receive the same ownership refusal.
    for enrolled in [false, true] {
        if enrolled {
            let other = f.offer("Other account enrollment fixture.", 70, 14).await;
            f.reserve(1, &other).await;
        }
        for version in [
            offer.offer_version_hash.clone(),
            digest("foreign changed payload"),
        ] {
            let foreign = RewardReservationRequest {
                offer_version_hash: version,
                ..replay.clone()
            };
            assert_eq!(
                f.runtime
                    .reserve_reward_offer(
                        &f.ledger.tenant,
                        f.accounts[1],
                        offer.program_id,
                        &foreign
                    )
                    .await
                    .unwrap_err(),
                RewardError::NotFound
            );
        }
    }
    assert_eq!(f.history(0).await.entries.len(), 1);
    assert_eq!(f.history(1).await.entries.len(), 1);
    assert_eq!(
        f.runtime
            .get_reward_offer(offer.program_id)
            .await
            .unwrap()
            .capacity_used_units,
        7
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_duplicate_work_and_read_only_history_are_enforced() {
    let f = ParticipantFixture::new().await;
    assert!(f.history(0).await.entries.is_empty());
    assert_eq!(
        f.ledger
            .admin
            .query_one(
                "SELECT count(*) FROM trace_reward_principals WHERE tenant_id=$1",
                &[&f.ledger.tenant]
            )
            .await
            .unwrap()
            .get::<_, i64>(0),
        0
    );
    let one = f
        .offer("Repeatable definition shared across offers.", 70, 14)
        .await;
    let two = f
        .offer("Repeatable definition shared across offers.", 70, 14)
        .await;
    let first = f.reserve(0, &one).await;
    assert_eq!(
        f.runtime
            .reserve_reward_offer(
                &f.ledger.tenant,
                f.accounts[0],
                two.program_id,
                &request(&two)
            )
            .await
            .unwrap_err(),
        RewardError::WorkDuplicate
    );
    // The issuer cannot bypass account ownership by calling the R0 reserve API.
    assert!(
        f.ledger
            .issuer_db
            .reward_reserve(
                &f.ledger.tenant,
                one.program_id,
                Uuid::new_v4(),
                &digest("invented participant"),
                &digest("invented work"),
                &digest("consent")
            )
            .await
            .is_err()
    );
    for i in 0..3 {
        let next = f.offer(&format!("Distinct task {i}."), 70, 14).await;
        f.reserve(0, &next).await;
    }
    let mut seen = Vec::new();
    let mut before = None;
    loop {
        let page = f
            .runtime
            .get_reward_history(
                &f.ledger.tenant,
                f.accounts[0],
                &RewardHistoryQuery { limit: 1, before },
            )
            .await
            .unwrap();
        seen.extend(page.entries.iter().map(|entry| entry.reservation_id));
        if !page.truncated {
            assert!(page.next_cursor.is_none());
            break;
        }
        before = page.next_cursor;
        assert!(before.is_some());
        assert!(seen.len() <= 4, "cursor must advance");
    }
    assert_eq!(seen.len(), 4);
    assert!(seen.contains(&first.reservation_id));
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), 4);
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_merge_preserves_alias_history_and_duplicate_checks() {
    let f = ParticipantFixture::new().await;
    // Distinct work namespaces and one reservation each, so the union violates
    // neither the participant cap nor the lifetime duplicate-work check.
    let shared = f
        .offer("One task for independently controlled accounts.", 21, 7)
        .await;
    let other = f
        .offer("A second, unrelated task for the absorbed account.", 21, 7)
        .await;
    let first = f.reserve(0, &shared).await;
    let second = f.reserve(1, &other).await;
    f.grant_account_merge_privileges().await;
    let proposal = f.merge_proposal(0, 1).await;
    let runtime = f
        .runtime
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    runtime
        .execute(
            "SELECT set_config('trace_commons.trace_tenant_id',$1,false)",
            &[&f.ledger.tenant],
        )
        .await
        .unwrap();
    assert!(
        runtime
            .execute(
                "SELECT trace_reward_accounts_merge($1,$2,$3,$4)",
                &[&f.ledger.tenant, &f.accounts[0], &f.accounts[1], &proposal]
            )
            .await
            .is_err(),
        "unconsumed proposal cannot remap rewards"
    );
    drop(runtime);
    assert!(
        f.runtime
            .execute_merge(&f.ledger.tenant, f.accounts[0], proposal)
            .await
            .unwrap()
            .is_some()
    );
    let history = f.history(0).await;
    assert_eq!(history.entries.len(), 2);
    assert!(
        history
            .entries
            .iter()
            .any(|entry| entry.reservation_id == first.reservation_id)
    );
    assert!(
        history
            .entries
            .iter()
            .any(|entry| entry.reservation_id == second.reservation_id)
    );
    assert!(
        f.runtime
            .get_reward_history(
                &f.ledger.tenant,
                f.accounts[1],
                &RewardHistoryQuery {
                    limit: 20,
                    before: None
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        f.runtime
            .get_reward_offer(shared.program_id)
            .await
            .unwrap()
            .capacity_used_units,
        7
    );
    assert_eq!(
        f.runtime
            .get_reward_offer(other.program_id)
            .await
            .unwrap()
            .capacity_used_units,
        7
    );
    let later = f
        .offer("One task for independently controlled accounts.", 21, 7)
        .await;
    assert_eq!(
        f.runtime
            .reserve_reward_offer(
                &f.ledger.tenant,
                f.accounts[0],
                later.program_id,
                &request(&later)
            )
            .await
            .unwrap_err(),
        RewardError::WorkDuplicate
    );
    let after = f
        .runtime
        .get_reward_reservation(&f.ledger.tenant, f.accounts[0], second.reservation_id)
        .await
        .unwrap();
    assert_eq!(after.reservation_id, second.reservation_id);
    assert!(
        serde_json::to_value(after)
            .unwrap()
            .get("participant_hash")
            .is_none()
    );
    assert_eq!(history.awarded_units, "0");
    assert_eq!(json!(history.entries[0].pinned_units), json!(7));
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn revoked_participant_login_rolls_back_entire_account_merge() {
    let f = ParticipantFixture::new().await;
    let shared = f.offer("Atomic merge rollback fixture.", 21, 7).await;
    let other = f
        .offer("Atomic merge rollback fixture, absorbed.", 21, 7)
        .await;
    let first = f.reserve(0, &shared).await;
    let second = f.reserve(1, &other).await;
    f.grant_account_merge_privileges().await;
    let proposal = f.merge_proposal(0, 1).await;
    f.ledger
        .admin
        .execute(
            "DELETE FROM trace_reward_participant_logins WHERE tenant_id=$1 AND login_role=$2::name",
            &[&f.ledger.tenant, &f.login],
        )
        .await
        .expect("revoke participant tenant login mapping");
    assert!(
        f.runtime
            .execute_merge(&f.ledger.tenant, f.accounts[0], proposal)
            .await
            .is_err(),
        "missing reward authorization must fail the account merge"
    );
    let consumed_at: Option<chrono::DateTime<Utc>> = f
        .ledger
        .admin
        .query_one(
            "SELECT consumed_at FROM trace_account_merge_proposals WHERE tenant_id=$1 AND proposal_id=$2",
            &[&f.ledger.tenant, &proposal],
        )
        .await
        .unwrap()
        .get(0);
    assert!(consumed_at.is_none(), "failed merge consumed proposal");
    let open_accounts: i64 = f
        .ledger
        .admin
        .query_one(
            "SELECT count(*) FROM trace_accounts WHERE tenant_id=$1 AND account_id IN ($2,$3) AND closed_at IS NULL",
            &[&f.ledger.tenant, &f.accounts[0], &f.accounts[1]],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        open_accounts, 2,
        "failed merge must leave both accounts open"
    );
    let reward_principals: i64 = f
        .ledger
        .admin
        .query_one(
            "SELECT count(DISTINCT reward_principal_id) FROM trace_reward_principal_accounts WHERE tenant_id=$1 AND account_id IN ($2,$3)",
            &[&f.ledger.tenant, &f.accounts[0], &f.accounts[1]],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        reward_principals, 2,
        "failed merge must preserve separate reward aliases"
    );
    for program in [shared.program_id, other.program_id] {
        assert_eq!(
            f.runtime
                .get_reward_offer(program)
                .await
                .unwrap()
                .capacity_used_units,
            7,
            "failed merge must not release or duplicate capacity"
        );
    }
    f.ledger
        .admin
        .execute(
            "INSERT INTO trace_reward_participant_logins(tenant_id,login_role) VALUES($1,$2::name)",
            &[&f.ledger.tenant, &f.login],
        )
        .await
        .expect("restore participant tenant login mapping");
    let survivor_history = f.history(0).await;
    let absorbed_history = f.history(1).await;
    assert_eq!(survivor_history.entries.len(), 1);
    assert_eq!(
        survivor_history.entries[0].reservation_id,
        first.reservation_id
    );
    assert_eq!(absorbed_history.entries.len(), 1);
    assert_eq!(
        absorbed_history.entries[0].reservation_id,
        second.reservation_id
    );
    assert!(
        f.runtime
            .execute_merge(&f.ledger.tenant, f.accounts[0], proposal)
            .await
            .unwrap()
            .is_some()
    );
}

/// The merge hook admits a proposal only when the consuming UPDATE ran in the
/// current transaction. `xmin` is a 32-bit `xid`; `pg_current_xact_id()` is a
/// 64-bit `xid8` carrying the wraparound epoch in its high word. Rendering both
/// as text makes them equal only while the epoch is zero, so the shipped
/// predicate would refuse every merge after the first wraparound. Casting the
/// `xid8` down to `xid` discards exactly the epoch, which is what `xmin` already
/// lacks, so the comparison holds in every epoch. The epoch cannot be advanced
/// from a test, so this asserts the property on a synthesised epoch-1 value.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn merge_freshness_check_is_transaction_id_epoch_independent() {
    let f = RewardPgFixture::new().await;
    // 4294967591 = (1 << 32) + 295: epoch 1, counter 295. A row written by that
    // transaction stores xmin = 295.
    let row = f
        .admin
        .query_one(
            "SELECT '4294967591'::xid8::xid = '295'::xid AS matches_as_xid,
                    '4294967591'::xid8::TEXT = '295'::xid::TEXT AS matches_as_text",
            &[],
        )
        .await
        .expect("evaluate both predicate shapes");
    assert!(
        row.get::<_, bool>("matches_as_xid"),
        "the xid8 to xid cast must keep the freshness check epoch-independent"
    );
    assert!(
        !row.get::<_, bool>("matches_as_text"),
        "the text comparison stops matching once the epoch is non-zero"
    );
    let migration = include_str!("../../../migrations/V71__reward_participant_access.sql");
    assert!(
        migration.contains("proposal.xmin = pg_catalog.pg_current_xact_id()::xid"),
        "the merge hook must compare transaction ids, not their renderings"
    );
    assert!(
        !migration.contains("pg_current_xact_id()::TEXT"),
        "no text rendering of a transaction id may gate the merge hook"
    );
}

/// Joining two alias groups must not create state `trace_reward_participant_
/// reserve` refuses. Two accounts each holding the per-participant maximum on
/// one program would, merged, leave a single payout identity holding twice the
/// advertised cap, so the merge is refused and nothing moves.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_merge_refuses_when_union_exceeds_participant_cap() {
    let f = ParticipantFixture::new().await;
    let shared = f.offer("One capped task, two accounts.", 21, 7).await;
    f.reserve(0, &shared).await;
    f.reserve(1, &shared).await;
    f.grant_account_merge_privileges().await;
    let proposal = f.merge_proposal(0, 1).await;
    let refusal = f
        .runtime
        .execute_merge(&f.ledger.tenant, f.accounts[0], proposal)
        .await
        .expect_err("a merge over the participant cap must be refused");
    assert!(
        matches!(&refusal, DatabaseError::Constraint(label)
            if label == "reward_merge_participant_cap"),
        "expected the named cap refusal, got {refusal}"
    );
    assert_eq!(
        f.reward_principals([0, 1]).await,
        2,
        "aliases must not join"
    );
    assert!(f.proposal_consumed_at(proposal).await.is_none());
    assert_eq!(
        f.runtime
            .get_reward_offer(shared.program_id)
            .await
            .unwrap()
            .capacity_used_units,
        14,
        "a refused merge must neither release nor duplicate capacity"
    );
}

/// The same rule for the lifetime work-namespace check: two programs published
/// from the same definition share a work namespace, so merging accounts that
/// each hold one would leave one identity claiming that work twice. Neither
/// account is over its cap, so this is the case the cap check alone misses.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn participant_merge_refuses_duplicate_work_namespace() {
    let f = ParticipantFixture::new().await;
    let definition = "One definition, published twice.";
    let first = f.offer(definition, 21, 7).await;
    let second = f.offer(definition, 21, 7).await;
    assert_ne!(first.program_id, second.program_id);
    f.reserve(0, &first).await;
    f.reserve(1, &second).await;
    f.grant_account_merge_privileges().await;
    let proposal = f.merge_proposal(0, 1).await;
    let refusal = f
        .runtime
        .execute_merge(&f.ledger.tenant, f.accounts[0], proposal)
        .await
        .expect_err("a merge duplicating a work namespace must be refused");
    assert!(
        matches!(&refusal, DatabaseError::Constraint(label)
            if label == "reward_merge_work_duplicate"),
        "expected the named duplicate-work refusal, got {refusal}"
    );
    assert_eq!(
        f.reward_principals([0, 1]).await,
        2,
        "aliases must not join"
    );
    assert!(f.proposal_consumed_at(proposal).await.is_none());
}
