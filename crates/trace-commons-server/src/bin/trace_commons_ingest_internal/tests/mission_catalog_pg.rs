// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: included as a child of `reward_participant_tests`.

use std::collections::{BTreeSet, HashSet};

use serde_json::Value;
use sha2::{Digest, Sha256};
use trace_commons_protocol::{
    mission_catalog::{MissionCatalogPage, MissionCatalogQuery, MissionPublication},
    mission_evaluation::{
        MISSION_EVALUATION_MAX_CONCURRENCY, MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
        MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS, MISSION_EVALUATION_REQUIRED_MODEL_OWNER,
        MISSION_EVALUATION_SCHEMA_VERSION, MISSION_EVALUATION_TOTAL_REQUESTS, MISSION_EVALUATOR_ID,
        MissionEvaluationPackage, MissionEvaluationPolicy, SkillDraft, render_skill,
    },
};
use uuid::Uuid;

use crate::tests::reward_participant_tests::*;

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn package(
    offer: &trace_commons_server::reward_participant::RewardOffer,
    mission_id: Uuid,
    task: &str,
) -> MissionEvaluationPackage {
    let skill = SkillDraft {
        name: "repair-generated-sources".into(),
        description: "Repair generated files at their authoritative source.".into(),
        procedure: "# Repair\n\nUpdate the source, then regenerate outputs.".into(),
    };
    MissionEvaluationPackage {
        schema_version: MISSION_EVALUATION_SCHEMA_VERSION,
        mission_id,
        program_id: offer.program_id,
        offer_version_hash: offer.offer_version_hash.clone(),
        task: task.into(),
        skill_sha256: digest(render_skill(&skill).as_bytes()),
        skill,
        evaluator_id: MISSION_EVALUATOR_ID.into(),
        // Synthetic structural fixture only; this makes no claim about
        // provider qualification.
        evaluation_contract_hash: digest(b"synthetic mission evaluation contract"),
        execution: MissionEvaluationPolicy {
            required_model_owner: MISSION_EVALUATION_REQUIRED_MODEL_OWNER.into(),
            total_requests: MISSION_EVALUATION_TOTAL_REQUESTS,
            output_token_limit: MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
            request_timeout_seconds: MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
            max_concurrency: MISSION_EVALUATION_MAX_CONCURRENCY,
        },
    }
}

fn object_keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect()
}

async fn get_page(fixture: &RewardHttpFixture, query: &MissionCatalogQuery) -> MissionCatalogPage {
    let uri = match query.before {
        Some(before) => format!(
            "{}/v1/missions?limit={}&before={before}",
            fixture.base, query.limit
        ),
        None => format!("{}/v1/missions?limit={}", fixture.base, query.limit),
    };
    let response = fixture
        .client
        .get(uri)
        .send()
        .await
        .expect("catalog request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_safe_reqwest_headers(response.headers());
    let bytes = response.bytes().await.expect("catalog response bytes");
    MissionCatalogPage::parse(&bytes, query).expect("bounded catalog response")
}

async fn assert_no_catalog_side_effects(
    fixture: &RewardHttpFixture,
    first_program: Uuid,
    second_program: Uuid,
) {
    let client = fixture
        .backends
        .admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("administrator connection");
    let rows = client
        .query_one(
            "SELECT
                (SELECT count(*) FROM trace_reward_reservations
                  WHERE tenant_id = $1 AND program_id IN ($2, $3)),
                (SELECT count(*) FROM trace_reward_awards
                  WHERE tenant_id = $1 AND program_id IN ($2, $3)),
                (SELECT count(*) FROM trace_reward_decisions d
                   JOIN trace_reward_reservations r
                     ON r.tenant_id = d.tenant_id AND r.reservation_id = d.reservation_id
                  WHERE r.tenant_id = $1 AND r.program_id IN ($2, $3))",
            &[&TENANT, &first_program, &second_program],
        )
        .await
        .expect("catalog side-effect counts");
    for column in 0..3 {
        assert_eq!(
            rows.get::<_, i64>(column),
            0,
            "catalog reads create no reservation, award, or evidence decision"
        );
    }
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL loopback reward_test_* database"]
async fn public_mission_catalog_http_is_bounded_anonymous_and_side_effect_free() {
    let fixture = RewardHttpFixture::start().await;
    let first_mission = Uuid::new_v4();
    let first_package = package(
        &fixture.offer,
        first_mission,
        "Repair the first synthetic generated manifest.",
    );
    let first_publication = fixture
        .backends
        .issuer
        .reward_mission_publish(TENANT, fixture.offer.program_id, &first_package)
        .await
        .expect("issuer publishes first mission");

    let second_offer = publish_offer(fixture.backends.issuer.as_ref(), Uuid::new_v4()).await;
    let second_mission = Uuid::new_v4();
    let second_package = package(
        &second_offer,
        second_mission,
        "Repair the second synthetic generated manifest.",
    );
    fixture
        .backends
        .issuer
        .reward_mission_publish(TENANT, second_offer.program_id, &second_package)
        .await
        .expect("issuer publishes second mission");

    let detail = fixture
        .client
        .get(format!("{}/v1/missions/{first_mission}", fixture.base))
        .send()
        .await
        .expect("anonymous mission detail request");
    assert_eq!(detail.status(), reqwest::StatusCode::OK);
    assert_safe_reqwest_headers(detail.headers());
    let detail_bytes = detail.bytes().await.expect("mission detail bytes");
    let detail_value: Value = serde_json::from_slice(&detail_bytes).expect("mission detail JSON");
    assert_eq!(
        object_keys(&detail_value),
        BTreeSet::from([
            "package_json",
            "package_sha256",
            "published_at",
            "schema_version",
        ])
    );
    let publication = MissionPublication::parse(&detail_bytes).expect("bounded publication");
    assert_eq!(publication, first_publication);
    assert_eq!(
        publication.package_json,
        serde_json::to_string(&first_package).unwrap()
    );
    assert_eq!(
        publication.package_sha256,
        digest(publication.package_json.as_bytes())
    );
    let decoded = publication.package().expect("exact mission package");
    assert_eq!(decoded.mission_id, first_mission);
    assert_eq!(decoded.program_id, fixture.offer.program_id);

    let first_query = MissionCatalogQuery {
        limit: 1,
        before: None,
    };
    let first_page = get_page(&fixture, &first_query).await;
    assert_eq!(first_page.entries.len(), 1);
    let cursor = first_page.next_cursor.expect("full first page has cursor");
    let second_query = MissionCatalogQuery {
        limit: 1,
        before: Some(cursor),
    };
    let second_page = get_page(&fixture, &second_query).await;
    assert_eq!(second_page.entries.len(), 1);
    assert!(second_page.next_cursor.is_none());
    let mission_ids: Vec<_> = first_page
        .entries
        .iter()
        .chain(&second_page.entries)
        .map(|entry| entry.mission_id)
        .collect();
    assert_eq!(
        mission_ids.iter().copied().collect::<HashSet<_>>().len(),
        mission_ids.len(),
        "exclusive cursor must not repeat missions"
    );
    assert_eq!(
        mission_ids.iter().copied().collect::<HashSet<_>>(),
        HashSet::from([first_mission, second_mission])
    );
    for entry in first_page.entries.iter().chain(&second_page.entries) {
        let value = serde_json::to_value(entry).expect("catalog entry serializes");
        assert_eq!(
            object_keys(&value),
            BTreeSet::from([
                "mission_id",
                "offer_version_hash",
                "package_sha256",
                "program_id",
                "published_at",
                "task_preview",
            ])
        );
    }

    for (uri, status) in [
        (
            format!("{}/v1/missions?limit=0", fixture.base),
            reqwest::StatusCode::BAD_REQUEST,
        ),
        (
            format!("{}/v1/missions?limit=51", fixture.base),
            reqwest::StatusCode::BAD_REQUEST,
        ),
        (
            format!("{}/v1/missions?before={}", fixture.base, Uuid::nil()),
            reqwest::StatusCode::BAD_REQUEST,
        ),
        (
            format!("{}/v1/missions/not-a-uuid", fixture.base),
            reqwest::StatusCode::BAD_REQUEST,
        ),
        (
            format!("{}/v1/missions/{}", fixture.base, Uuid::nil()),
            reqwest::StatusCode::BAD_REQUEST,
        ),
        (
            format!("{}/v1/missions/{}", fixture.base, Uuid::new_v4()),
            reqwest::StatusCode::NOT_FOUND,
        ),
    ] {
        let response = fixture
            .client
            .get(uri)
            .send()
            .await
            .expect("safe refusal request");
        assert_eq!(response.status(), status);
        assert_safe_reqwest_headers(response.headers());
    }

    fixture
        .backends
        .issuer
        .reward_offer_suspend(TENANT, fixture.offer.program_id, true)
        .await
        .expect("issuer suspends first offer");
    let suspended = get_page(&fixture, &first_query).await;
    assert!(
        !suspended
            .entries
            .iter()
            .any(|entry| entry.mission_id == first_mission)
    );
    let hidden = fixture
        .client
        .get(format!("{}/v1/missions/{first_mission}", fixture.base))
        .send()
        .await
        .expect("hidden detail request");
    assert_eq!(hidden.status(), reqwest::StatusCode::NOT_FOUND);
    assert_safe_reqwest_headers(hidden.headers());
    assert_eq!(
        fixture
            .backends
            .issuer
            .reward_mission_publish(TENANT, fixture.offer.program_id, &first_package)
            .await
            .expect("exact issuer publication replay"),
        first_publication
    );

    assert_no_catalog_side_effects(&fixture, fixture.offer.program_id, second_offer.program_id)
        .await;
}
