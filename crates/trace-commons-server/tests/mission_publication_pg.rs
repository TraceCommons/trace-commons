// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: run with an isolated TRACE_COMMONS_REWARDS_PG_TEST_URL and --ignored.

#[allow(dead_code)]
#[path = "mission_rewards_pg/fixture.rs"]
mod fixture;

use std::collections::{BTreeSet, HashSet};
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use serde_json::Value;
use sha2::{Digest, Sha256};
use trace_commons_protocol::{
    mission_catalog::{MissionCatalogQuery, MissionPublication},
    mission_evaluation::{
        MISSION_EVALUATION_MAX_CONCURRENCY, MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
        MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS, MISSION_EVALUATION_REQUIRED_MODEL_OWNER,
        MISSION_EVALUATION_SCHEMA_VERSION, MISSION_EVALUATION_TOTAL_REQUESTS, MISSION_EVALUATOR_ID,
        MissionEvaluationPackage, MissionEvaluationPolicy, SkillDraft, render_skill,
    },
};
use trace_commons_server::{
    config::{DatabaseConfig, SslMode},
    db::{Database, postgres::PgBackend},
    mission_rewards::{RewardActivityKind, RewardError, RewardProgramTerms},
    reward_participant::{RewardOffer, RewardOfferManifest},
};
use uuid::Uuid;

use crate::fixture::{RewardPgFixture, cli, digest, role_url};

struct MissionFixture {
    rewards: RewardPgFixture,
    reader: PgBackend,
    reader_login: String,
}

impl MissionFixture {
    async fn new() -> Self {
        let rewards = RewardPgFixture::new().await;
        let reader_login = format!("mission_reader_{}", rewards.suffix);
        rewards
            .admin
            .batch_execute(&format!(
                "CREATE ROLE {reader_login} LOGIN NOSUPERUSER NOBYPASSRLS INHERIT;
                 GRANT trace_reward_participant_runtime TO {reader_login};"
            ))
            .await
            .expect("create public mission reader");
        let reader = PgBackend::new(&DatabaseConfig {
            url: role_url(&rewards.url, &reader_login).into(),
            pool_size: 4,
            ssl_mode: SslMode::Prefer,
            login_resolver_url: None,
            gate_driver_url: None,
            pii_backstop_driver_url: None,
            invite_registry_url: None,
        })
        .await
        .expect("public mission reader connects");
        Self {
            rewards,
            reader,
            reader_login,
        }
    }

    async fn offer(&self, closes_at: chrono::DateTime<Utc>, definition: &str) -> RewardOffer {
        let manifest = RewardOfferManifest {
            schema_version: 1,
            definition: definition.into(),
            rubric: "All required synthetic checks pass.".into(),
            required_evidence: "Provide the complete synthetic attempt set.".into(),
            rights: "Reward review only; corpus contribution remains optional.".into(),
            challenge_policy: "A rejected synthetic attempt releases its allocation.".into(),
            evaluator_policy: "An independent reviewer evaluates synthetic evidence.".into(),
            reservation_terms: "Reserve capacity for this immutable offer version.".into(),
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
            sponsor_hash: digest("synthetic mission sponsor"),
            award_units: 7,
            capacity_units: 70,
            participant_cap_units: 14,
            closes_at,
            reservation_ttl_seconds: 600,
        };
        self.rewards
            .issuer_db
            .reward_offer_publish(&self.rewards.tenant, Uuid::new_v4(), &terms, &manifest)
            .await
            .expect("publish mission offer")
    }

    async fn active_offer(&self, definition: &str) -> RewardOffer {
        self.offer(Utc::now() + Duration::hours(1), definition)
            .await
    }

    async fn publish(
        &self,
        offer: &RewardOffer,
        package: &MissionEvaluationPackage,
    ) -> Result<MissionPublication, RewardError> {
        self.rewards
            .issuer_db
            .reward_mission_publish(&self.rewards.tenant, offer.program_id, package)
            .await
    }
}

fn bare_digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn mission_package(offer: &RewardOffer, mission: Uuid, task: &str) -> MissionEvaluationPackage {
    let skill = SkillDraft {
        name: "repair-generated-sources".into(),
        description: "Repair generated files at their authoritative source.".into(),
        procedure: "# Repair\n\nUpdate the source, then regenerate outputs.".into(),
    };
    MissionEvaluationPackage {
        schema_version: MISSION_EVALUATION_SCHEMA_VERSION,
        mission_id: mission,
        program_id: offer.program_id,
        offer_version_hash: offer.offer_version_hash.clone(),
        task: task.into(),
        skill_sha256: bare_digest(render_skill(&skill).as_bytes()),
        skill,
        evaluator_id: MISSION_EVALUATOR_ID.into(),
        // Synthetic structural fixture only; this test invokes no model and
        // makes no claim about provider qualification.
        evaluation_contract_hash: bare_digest(b"synthetic mission evaluation contract"),
        execution: MissionEvaluationPolicy {
            required_model_owner: MISSION_EVALUATION_REQUIRED_MODEL_OWNER.into(),
            total_requests: MISSION_EVALUATION_TOTAL_REQUESTS,
            output_token_limit: MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
            request_timeout_seconds: MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
            max_concurrency: MISSION_EVALUATION_MAX_CONCURRENCY,
        },
    }
}

fn assert_reward_error<T>(result: Result<T, RewardError>, expected: RewardError) {
    assert_eq!(result.err(), Some(expected));
}

fn object_keys(value: &Value) -> BTreeSet<&str> {
    value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect()
}

async fn visible_mission_ids(db: &PgBackend) -> Vec<Uuid> {
    let mut before = None;
    let mut ids = Vec::new();
    for _ in 0..10_000 {
        let query = MissionCatalogQuery { limit: 50, before };
        let page = db
            .list_mission_catalog(&query)
            .await
            .expect("read public mission catalog");
        ids.extend(page.entries.iter().map(|entry| entry.mission_id));
        match page.next_cursor {
            Some(cursor) => before = Some(cursor),
            None => return ids,
        }
    }
    panic!("public mission catalog cursor did not terminate");
}

fn assert_state_conflict(error: tokio_postgres::Error) {
    let db = error.as_db_error().expect("database refusal");
    assert_eq!(db.code().code(), "P0001");
    assert_eq!(db.message(), "reward_state_conflict");
}

async fn sql_package_valid(fixture: &MissionFixture, package_json: &str) -> bool {
    fixture
        .rewards
        .admin
        .query_one(
            "SELECT public.trace_reward_mission_package_valid($1)",
            &[&package_json],
        )
        .await
        .expect("evaluate SQL package boundary")
        .get(0)
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn issuer_publication_is_canonical_replay_safe_and_tenant_authorized() {
    let f = MissionFixture::new().await;
    let offer = f
        .active_offer("Canonical mission publication fixture.")
        .await;
    let mission = Uuid::new_v4();
    let package = mission_package(&offer, mission, "Repair the synthetic generated manifest.");
    let canonical = serde_json::to_string(&package).expect("canonical package JSON");

    let (left, right) = tokio::join!(f.publish(&offer, &package), f.publish(&offer, &package));
    let published = left.expect("first concurrent publication");
    assert_eq!(right.expect("second concurrent publication"), published);
    assert_eq!(published.package_json.as_bytes(), canonical.as_bytes());
    assert_eq!(published.package_sha256, bare_digest(canonical.as_bytes()));
    assert_eq!(published.package_sha256.len(), 64);
    assert!(!published.package_sha256.starts_with("sha256:"));
    assert_eq!(
        published
            .package()
            .expect("publication package")
            .skill_sha256,
        bare_digest(render_skill(&package.skill).as_bytes())
    );
    assert_eq!(
        f.publish(&offer, &package)
            .await
            .expect("exact publication retry"),
        published
    );

    let mut changed = package.clone();
    changed.task.push_str(" Changed.");
    assert_reward_error(
        f.publish(&offer, &changed).await,
        RewardError::PayloadConflict,
    );

    let mut stale_hash = package.clone();
    stale_hash.skill_sha256 = "0".repeat(64);
    assert_reward_error(
        f.rewards
            .no_grant_db
            .reward_mission_publish(&f.rewards.tenant, offer.program_id, &stale_hash)
            .await,
        RewardError::RequestInvalid,
    );
    let mut expanded_policy = package.clone();
    expanded_policy.execution.max_concurrency += 1;
    assert_reward_error(
        f.rewards
            .no_grant_db
            .reward_mission_publish(&f.rewards.tenant, offer.program_id, &expanded_policy)
            .await,
        RewardError::RequestInvalid,
    );
    assert_reward_error(
        f.rewards
            .no_grant_db
            .reward_mission_publish(&f.rewards.tenant, Uuid::new_v4(), &package)
            .await,
        RewardError::RequestInvalid,
    );

    let admin_is_superuser: bool = f
        .rewards
        .admin
        .query_one(
            "SELECT rolsuper FROM pg_roles WHERE rolname=current_user",
            &[],
        )
        .await
        .expect("inspect administrative fixture")
        .get(0);
    assert!(admin_is_superuser);
    for denied in [
        &f.rewards.reviewer_db,
        &f.rewards.no_grant_db,
        &f.rewards.admin_db,
    ] {
        assert_reward_error(
            denied
                .reward_mission_publish(&f.rewards.tenant, offer.program_id, &package)
                .await,
            RewardError::Unauthorized,
        );
    }
    assert_reward_error(
        f.rewards
            .issuer_db
            .reward_mission_publish("another-tenant", offer.program_id, &package)
            .await,
        RewardError::Unauthorized,
    );

    let race_offer = f.active_offer("Concurrent conflict fixture.").await;
    let first = mission_package(&race_offer, Uuid::new_v4(), "First concurrent package.");
    let mut second = first.clone();
    second.task = "Second concurrent package.".into();
    let (left, right) = tokio::join!(
        f.publish(&race_offer, &first),
        f.publish(&race_offer, &second)
    );
    assert!(matches!(
        (&left, &right),
        (Ok(_), Err(RewardError::PayloadConflict)) | (Err(RewardError::PayloadConflict), Ok(_))
    ));
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn sql_projection_boundary_rejects_rust_structural_corpus_and_task_controls() {
    let f = MissionFixture::new().await;
    let offer = f.active_offer("SQL projection boundary fixture.").await;
    let package = mission_package(
        &offer,
        Uuid::new_v4(),
        "Repair the synthetic generated manifest.",
    );
    let valid_json = serde_json::to_string(&package).expect("serialize valid package");
    assert!(sql_package_valid(&f, &valid_json).await);

    let mut rejected = Vec::new();
    let mut value = serde_json::to_value(&package).expect("package value");
    value["unexpected"] = true.into();
    rejected.push(("top-level-shape", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["skill"]
        .as_object_mut()
        .expect("skill object")
        .remove("name");
    rejected.push(("skill-shape", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["execution"]["total_requests"] = (MISSION_EVALUATION_TOTAL_REQUESTS + 1).into();
    rejected.push(("total-requests-policy-bound", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["execution"]["output_token_limit"] = (MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT + 1).into();
    rejected.push(("output-token-policy-bound", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["execution"]["request_timeout_seconds"] =
        (MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS + 1).into();
    rejected.push(("timeout-policy-bound", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["execution"]["max_concurrency"] = (MISSION_EVALUATION_MAX_CONCURRENCY + 1).into();
    rejected.push(("concurrency-policy-bound", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["execution"]["required_model_owner"] = "other".into();
    rejected.push(("model-owner-policy-bound", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["evaluator_id"] = "skill-evaluation-v2".into();
    rejected.push(("evaluator-policy-bound", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["mission_id"] = Uuid::nil().to_string().into();
    rejected.push(("nil-mission-id", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["program_id"] = "not-a-uuid".into();
    rejected.push(("program-id-format", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["offer_version_hash"] = format!("sha256:{}", "A".repeat(64)).into();
    rejected.push(("offer-hash-format", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["skill_sha256"] = "A".repeat(64).into();
    rejected.push(("skill-hash-format", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["evaluation_contract_hash"] = "short".into();
    rejected.push(("contract-hash-format", value));
    let mut value = serde_json::to_value(&package).expect("package value");
    value["skill"]["procedure"] = "x".repeat(70_000).into();
    rejected.push(("package-byte-bound", value));

    for (label, value) in rejected {
        let bytes = serde_json::to_vec(&value).expect("serialize rejection case");
        assert!(
            MissionEvaluationPackage::parse(&bytes).is_err(),
            "Rust unexpectedly accepted {label}"
        );
        let json = String::from_utf8(bytes).expect("JSON is UTF-8");
        assert!(
            !sql_package_valid(&f, &json).await,
            "SQL unexpectedly accepted {label}"
        );
    }

    for codepoint in (0_u32..=31).chain(127..=159) {
        let control = char::from_u32(codepoint).expect("valid control scalar");
        if matches!(control, '\n' | '\t') {
            continue;
        }
        let mut candidate = package.clone();
        candidate.task = format!("Repair{control}the manifest.");
        assert!(
            candidate.validate().is_err(),
            "Rust unexpectedly accepted U+{codepoint:04X}"
        );
        let json = serde_json::to_string(&candidate).expect("serialize control case");
        assert!(
            !sql_package_valid(&f, &json).await,
            "SQL unexpectedly accepted U+{codepoint:04X}"
        );
    }

    let mut allowed_controls = package.clone();
    allowed_controls.task = "Repair\tthis manifest.\nThen verify it.".into();
    assert_eq!(allowed_controls.validate(), Ok(()));
    assert!(
        sql_package_valid(
            &f,
            &serde_json::to_string(&allowed_controls).expect("serialize allowed controls")
        )
        .await
    );

    // SQL protects storage and anonymous projections. Rust additionally binds
    // the renderer/hash and applies the evolving outbound privacy classifier.
    let mut stale_renderer = package.clone();
    stale_renderer
        .skill
        .procedure
        .push_str("\nVerify the result.");
    assert!(stale_renderer.validate().is_err());
    assert!(
        sql_package_valid(
            &f,
            &serde_json::to_string(&stale_renderer).expect("serialize stale renderer")
        )
        .await
    );
    let mut privacy_rejected = package;
    privacy_rejected.task = "Send results to private.person@example.com".into();
    assert!(privacy_rejected.validate().is_err());
    assert!(
        sql_package_valid(
            &f,
            &serde_json::to_string(&privacy_rejected).expect("serialize privacy case")
        )
        .await
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn direct_issuer_cannot_publish_task_controls_into_public_projection() {
    let f = MissionFixture::new().await;
    let runtime = f.rewards.runtime().await;
    for (index, control) in ['\r', '\u{0085}'].into_iter().enumerate() {
        let offer = f
            .active_offer(&format!("Direct issuer control fixture {index}."))
            .await;
        let mission = Uuid::new_v4();
        let package = mission_package(
            &offer,
            mission,
            &format!("Repair{control}the synthetic manifest."),
        );
        let package_json = serde_json::to_string(&package).expect("serialize control package");
        let error = runtime
            .query_one(
                "SELECT public.trace_reward_mission_publish($1,$2,$3)",
                &[&f.rewards.tenant, &offer.program_id, &package_json],
            )
            .await
            .expect_err("SQL projection boundary refuses task control");
        let db = error.as_db_error().expect("database refusal");
        assert_eq!(db.code().code(), "P0001");
        assert_eq!(db.message(), "reward_request_invalid");
        assert!(!visible_mission_ids(&f.reader).await.contains(&mission));
    }
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn public_reader_uses_only_bounded_functions_and_rows_are_immutable() {
    let f = MissionFixture::new().await;
    let offer = f.active_offer("Public reader privilege fixture.").await;
    let mission = Uuid::new_v4();
    let package = mission_package(&offer, mission, "Read one synthetic published mission.");
    let publication = f
        .publish(&offer, &package)
        .await
        .expect("publish reader fixture");

    assert_eq!(
        f.reader
            .get_mission_publication(mission)
            .await
            .expect("reader gets mission"),
        publication
    );
    let page = f
        .reader
        .list_mission_catalog(&MissionCatalogQuery {
            limit: 50,
            before: None,
        })
        .await
        .expect("reader lists missions");
    assert!(!page.entries.is_empty());
    assert_reward_error(
        f.reader
            .reward_mission_publish(&f.rewards.tenant, offer.program_id, &package)
            .await,
        RewardError::StoreUnavailable,
    );

    let reader = f
        .reader
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("reader SQL connection");
    let identity = reader
        .query_one(
            "SELECT session_user::text, roles.rolsuper OR roles.rolbypassrls,
                    NULLIF(current_setting('trace_commons.trace_tenant_id', true), '')
               FROM pg_catalog.pg_roles roles
              WHERE roles.rolname=session_user",
            &[],
        )
        .await
        .expect("inspect public reader identity");
    assert_eq!(identity.get::<_, String>(0), f.reader_login);
    assert!(!identity.get::<_, bool>(1));
    assert!(identity.get::<_, Option<String>>(2).is_none());
    let login_rows: i64 = f
        .rewards
        .admin
        .query_one(
            "SELECT count(*) FROM trace_reward_participant_logins WHERE login_role=$1::name",
            &[&f.reader_login],
        )
        .await
        .expect("inspect participant provisioning")
        .get(0);
    assert_eq!(login_rows, 0, "public catalog needs no participant account");

    let grants = reader
        .query_one(
            "SELECT NOT has_table_privilege(current_user,'trace_reward_mission_packages','SELECT,INSERT,UPDATE,DELETE,TRUNCATE'),
                    has_function_privilege(current_user,'trace_reward_mission_get(uuid)','EXECUTE'),
                    has_function_privilege(current_user,'trace_reward_mission_list(uuid,integer)','EXECUTE'),
                    NOT has_function_privilege(current_user,'trace_reward_mission_publish(text,uuid,text)','EXECUTE')",
            &[],
        )
        .await
        .expect("inspect reader grants");
    for column in 0..4 {
        assert!(grants.get::<_, bool>(column));
    }
    assert!(
        reader
            .query_one("SELECT * FROM trace_reward_mission_packages", &[])
            .await
            .is_err()
    );

    let get_value: Value = reader
        .query_one("SELECT public.trace_reward_mission_get($1)", &[&mission])
        .await
        .expect("invoke public get function")
        .get(0);
    assert_eq!(
        object_keys(&get_value),
        BTreeSet::from([
            "package_json",
            "package_sha256",
            "published_at",
            "schema_version",
        ])
    );
    let no_before: Option<Uuid> = None;
    let list_value: Value = reader
        .query_one(
            "SELECT public.trace_reward_mission_list($1,$2)",
            &[&no_before, &50_i32],
        )
        .await
        .expect("invoke public list function")
        .get(0);
    assert_eq!(
        object_keys(&list_value),
        BTreeSet::from(["entries", "next_cursor", "schema_version"])
    );
    let first_entry = &list_value["entries"]
        .as_array()
        .expect("catalog entry array")[0];
    assert_eq!(
        object_keys(first_entry),
        BTreeSet::from([
            "mission_id",
            "offer_version_hash",
            "package_sha256",
            "program_id",
            "published_at",
            "task_preview",
        ])
    );

    let admin = &f.rewards.admin;
    let production_grants = admin
        .query_one(
            "SELECT has_function_privilege('trace_reward_runtime','trace_reward_mission_publish(text,uuid,text)','EXECUTE'),
                    NOT has_table_privilege('trace_reward_guard','trace_reward_mission_packages','UPDATE,DELETE,TRUNCATE'),
                    has_column_privilege('trace_reward_offer_reader','trace_reward_mission_packages','package_json','SELECT'),
                    NOT has_column_privilege('trace_reward_offer_reader','trace_reward_mission_packages','publisher_hash','SELECT'),
                    has_column_privilege('trace_reward_offer_reader','trace_reward_mission_packages','offer_version_hash','SELECT'),
                    has_column_privilege('trace_reward_offer_reader','trace_reward_mission_packages','task_preview','SELECT'),
                    (
                        SELECT count(*) = 2 AND bool_and(attribute.attgenerated = 's')
                          FROM pg_catalog.pg_attribute attribute
                         WHERE attribute.attrelid = 'trace_reward_mission_packages'::regclass
                           AND attribute.attname IN ('offer_version_hash', 'task_preview')
                    )",
            &[],
        )
        .await
        .expect("inspect production grants");
    for column in 0..7 {
        assert!(production_grants.get::<_, bool>(column));
    }
    let projections = admin
        .query_one(
            "SELECT offer_version_hash, task_preview
               FROM trace_reward_mission_packages
              WHERE tenant_id=$1 AND program_id=$2",
            &[&f.rewards.tenant, &offer.program_id],
        )
        .await
        .expect("read stored mission projections");
    assert_eq!(projections.get::<_, String>(0), package.offer_version_hash);
    assert_eq!(
        projections.get::<_, String>(1),
        "Read one synthetic published mission."
    );

    for operation in ["update", "delete", "truncate"] {
        admin
            .batch_execute("BEGIN")
            .await
            .expect("begin guard check");
        let result = match operation {
            "update" => admin
                .execute(
                    "UPDATE trace_reward_mission_packages SET package_json=package_json WHERE tenant_id=$1 AND program_id=$2",
                    &[&f.rewards.tenant, &offer.program_id],
                )
                .await
                .map(|_| ()),
            "delete" => admin
                .execute(
                    "DELETE FROM trace_reward_mission_packages WHERE tenant_id=$1 AND program_id=$2",
                    &[&f.rewards.tenant, &offer.program_id],
                )
                .await
                .map(|_| ()),
            "truncate" => admin
                .batch_execute("TRUNCATE trace_reward_mission_packages")
                .await,
            _ => unreachable!(),
        };
        assert_state_conflict(result.expect_err("immutable trigger refuses mutation"));
        admin
            .batch_execute("ROLLBACK")
            .await
            .expect("rollback failed mutation");
    }
    assert_eq!(
        f.reader
            .get_mission_publication(mission)
            .await
            .expect("immutable row remains"),
        publication
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn public_catalog_cursor_is_exclusive_complete_and_locally_filterable() {
    let f = MissionFixture::new().await;
    let mut expected = HashSet::new();
    for index in 0..5 {
        let offer = f
            .active_offer(&format!("Catalog pagination fixture {index}."))
            .await;
        let mission = Uuid::new_v4();
        expected.insert(mission);
        f.publish(
            &offer,
            &mission_package(&offer, mission, &format!("Synthetic catalog task {index}.")),
        )
        .await
        .expect("publish catalog fixture");
    }

    let mut before = None;
    let mut seen = Vec::new();
    let mut final_page_seen = false;
    for _ in 0..10_000 {
        let query = MissionCatalogQuery { limit: 2, before };
        let page = f
            .reader
            .list_mission_catalog(&query)
            .await
            .expect("page public catalog");
        assert!(page.entries.len() <= 2);
        for pair in page.entries.windows(2) {
            assert!(pair[0].mission_id > pair[1].mission_id);
        }
        if let Some(bound) = before {
            assert!(page.entries.iter().all(|entry| entry.mission_id < bound));
        }
        seen.extend(page.entries.iter().map(|entry| entry.mission_id));
        match page.next_cursor {
            Some(cursor) => {
                assert_eq!(page.entries.len(), 2);
                assert_eq!(
                    page.entries.last().map(|entry| entry.mission_id),
                    Some(cursor)
                );
                before = Some(cursor);
            }
            None => {
                final_page_seen = true;
                break;
            }
        }
    }
    assert!(final_page_seen);
    let unique: HashSet<_> = seen.iter().copied().collect();
    assert_eq!(
        unique.len(),
        seen.len(),
        "catalog cursor never repeats rows"
    );
    let scoped: HashSet<_> = unique.intersection(&expected).copied().collect();
    assert_eq!(scoped, expected, "catalog contains every synthetic mission");

    for limit in [0, 51] {
        assert_reward_error(
            f.rewards
                .no_grant_db
                .list_mission_catalog(&MissionCatalogQuery {
                    limit,
                    before: None,
                })
                .await,
            RewardError::RequestInvalid,
        );
    }
    assert_reward_error(
        f.rewards
            .no_grant_db
            .list_mission_catalog(&MissionCatalogQuery {
                limit: 1,
                before: Some(Uuid::nil()),
            })
            .await,
        RewardError::RequestInvalid,
    );
    assert_reward_error(
        f.rewards
            .no_grant_db
            .get_mission_publication(Uuid::nil())
            .await,
        RewardError::RequestInvalid,
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn suspension_and_database_clock_deadline_gate_reads_but_retain_replays() {
    let f = MissionFixture::new().await;
    let offer = f
        .active_offer("Suspended mission publication fixture.")
        .await;
    let mission = Uuid::new_v4();
    let package = mission_package(&offer, mission, "Synthetic mission suspension task.");
    let publication = f
        .publish(&offer, &package)
        .await
        .expect("publish suspension fixture");
    f.rewards
        .issuer_db
        .reward_offer_suspend(&f.rewards.tenant, offer.program_id, true)
        .await
        .expect("suspend mission offer");
    assert_reward_error(
        f.reader.get_mission_publication(mission).await,
        RewardError::OfferSuspended,
    );
    assert!(!visible_mission_ids(&f.reader).await.contains(&mission));
    assert_eq!(
        f.publish(&offer, &package)
            .await
            .expect("suspended exact retry"),
        publication
    );
    f.rewards
        .issuer_db
        .reward_offer_suspend(&f.rewards.tenant, offer.program_id, false)
        .await
        .expect("restore mission offer");
    assert_eq!(
        f.reader
            .get_mission_publication(mission)
            .await
            .expect("unsuspended mission is readable"),
        publication
    );

    let expiring = f
        .offer(
            Utc::now() + Duration::seconds(3),
            "Database-clock deadline fixture.",
        )
        .await;
    let expiring_mission = Uuid::new_v4();
    let expiring_package = mission_package(
        &expiring,
        expiring_mission,
        "Synthetic mission closes on the database clock.",
    );
    let expiring_publication = f
        .publish(&expiring, &expiring_package)
        .await
        .expect("publish short-lived mission");
    tokio::time::timeout(StdDuration::from_secs(5), async {
        loop {
            match f.reader.get_mission_publication(expiring_mission).await {
                Err(RewardError::ProgramClosed) => break,
                Ok(_) => tokio::time::sleep(StdDuration::from_millis(50)).await,
                Err(_) => panic!("deadline returned an unexpected safe refusal"),
            }
        }
    })
    .await
    .expect("database deadline closes within bounded wait");
    assert!(
        !visible_mission_ids(&f.reader)
            .await
            .contains(&expiring_mission)
    );
    assert_eq!(
        f.publish(&expiring, &expiring_package)
            .await
            .expect("closed exact retry"),
        expiring_publication
    );
    assert_reward_error(
        f.reader.get_mission_publication(Uuid::new_v4()).await,
        RewardError::NotFound,
    );
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL"]
async fn operator_cli_publishes_exact_package_and_wrong_role_is_refused() {
    let f = MissionFixture::new().await;
    let offer = f.active_offer("Mission publication CLI fixture.").await;
    let package = mission_package(
        &offer,
        Uuid::new_v4(),
        "Publish this synthetic mission through the operator CLI.",
    );
    let files = tempfile::tempdir().expect("temporary package directory");
    let package_file = files.path().join("mission-package.json");
    let package_bytes = serde_json::to_vec(&package).expect("serialize CLI package");
    std::fs::write(&package_file, package_bytes).expect("write CLI package");
    let arguments = vec![
        "mission-publish".into(),
        "--program".into(),
        offer.program_id.to_string(),
        "--package".into(),
        package_file.display().to_string(),
    ];
    let issuer_url = role_url(&f.rewards.url, &f.rewards.issuer);
    let first = cli(&issuer_url, &f.rewards.tenant, &arguments, true).await;
    let retry = cli(&issuer_url, &f.rewards.tenant, &arguments, true).await;
    assert_eq!(first, retry);
    assert_eq!(
        first["package_json"],
        serde_json::to_string(&package).unwrap()
    );
    let reviewer_url = role_url(&f.rewards.url, &f.rewards.reviewer);
    cli(&reviewer_url, &f.rewards.tenant, &arguments, false).await;
}
