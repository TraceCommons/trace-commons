use super::*;

fn draft() -> SkillDraft {
    SkillDraft {
        name: "repair-generated-sources".to_string(),
        description: "Repair generated files at their authoritative source.".to_string(),
        procedure: "# Repair\n\nUpdate the source, then regenerate outputs.".to_string(),
    }
}

fn valid_package() -> MissionEvaluationPackage {
    let skill = draft();
    MissionEvaluationPackage {
        schema_version: MISSION_EVALUATION_SCHEMA_VERSION,
        mission_id: Uuid::from_u128(1),
        program_id: Uuid::from_u128(2),
        offer_version_hash: format!("sha256:{}", "a".repeat(64)),
        task: "Repair the generated manifest from its source schema.".to_string(),
        skill_sha256: sha256(render_skill(&skill).as_bytes()),
        skill,
        evaluator_id: MISSION_EVALUATOR_ID.to_string(),
        evaluation_contract_hash: "b".repeat(64),
        execution: MissionEvaluationPolicy {
            required_model_owner: MISSION_EVALUATION_REQUIRED_MODEL_OWNER.to_string(),
            total_requests: MISSION_EVALUATION_TOTAL_REQUESTS,
            output_token_limit: MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
            request_timeout_seconds: MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
            max_concurrency: MISSION_EVALUATION_MAX_CONCURRENCY,
        },
    }
}

#[test]
fn rendered_skill_bytes_are_frozen() {
    assert_eq!(
        render_skill(&draft()),
        "---\nname: repair-generated-sources\ndescription: \"Repair generated files at their authoritative source.\"\ncompatibility: Designed for coding agents with repository read, edit, and command tools.\nmetadata:\n  author: Trace Commons contributor\n  family: generated-source-repair\n---\n\n# Repair\n\nUpdate the source, then regenerate outputs.\n"
    );
}

#[test]
fn valid_package_round_trips_and_ignores_json_key_order() {
    let original = valid_package();
    let bytes = serde_json::to_vec(&original).unwrap();
    let parsed = MissionEvaluationPackage::parse(&bytes).unwrap();
    assert_eq!(parsed, original);
    let reordered = serde_json::to_vec(&serde_json::to_value(&original).expect("package value"))
        .expect("reordered JSON");
    let reordered = MissionEvaluationPackage::parse(&reordered).unwrap();
    assert_eq!(reordered, original);
    assert_eq!(
        reordered.package_sha256().unwrap(),
        original.package_sha256().unwrap()
    );
}

#[test]
fn package_digest_changes_for_task_offer_and_artifact_mutations() {
    let original = valid_package();
    let digest = original.package_sha256().unwrap();
    let mut changed = original.clone();
    changed.task.push('!');
    assert_ne!(changed.package_sha256().unwrap(), digest);
    changed = original.clone();
    changed.offer_version_hash.replace_range(7..8, "c");
    assert_ne!(changed.package_sha256().unwrap(), digest);
    changed = original.clone();
    changed.mission_id = Uuid::from_u128(3);
    assert_ne!(changed.package_sha256().unwrap(), digest);
    changed = original.clone();
    changed.program_id = Uuid::from_u128(4);
    assert_ne!(changed.package_sha256().unwrap(), digest);
    changed = original.clone();
    changed.evaluation_contract_hash.replace_range(..1, "c");
    assert_ne!(changed.package_sha256().unwrap(), digest);
    changed = original;
    changed.skill.procedure.push_str("\nDo more.");
    assert_eq!(
        changed.validate(),
        Err(MissionEvaluationPackageError::StaleSkillSha256)
    );
    changed.skill_sha256 = sha256(render_skill(&changed.skill).as_bytes());
    assert_ne!(changed.package_sha256().unwrap(), digest);
}

#[test]
fn parse_refuses_malformed_oversized_and_nested_unknown_input() {
    assert_eq!(
        MissionEvaluationPackage::parse(b"{"),
        Err(MissionEvaluationPackageError::MalformedJson)
    );
    assert_eq!(
        MissionEvaluationPackage::parse(&vec![b' '; MISSION_EVALUATION_PACKAGE_MAX_BYTES + 1]),
        Err(MissionEvaluationPackageError::InputTooLarge)
    );
    let mut value = serde_json::to_value(valid_package()).unwrap();
    value["unexpected"] = serde_json::json!(true);
    assert_eq!(
        MissionEvaluationPackage::parse(&serde_json::to_vec(&value).unwrap()),
        Err(MissionEvaluationPackageError::MalformedJson)
    );
    let mut value = serde_json::to_value(valid_package()).unwrap();
    value["skill"]["unexpected"] = serde_json::json!(true);
    assert!(serde_json::from_value::<MissionEvaluationPackage>(value.clone()).is_err());
    assert_eq!(
        MissionEvaluationPackage::parse(&serde_json::to_vec(&value).unwrap()),
        Err(MissionEvaluationPackageError::MalformedJson)
    );
    let mut value = serde_json::to_value(valid_package()).unwrap();
    value["execution"]["unexpected"] = serde_json::json!(true);
    assert_eq!(
        MissionEvaluationPackage::parse(&serde_json::to_vec(&value).unwrap()),
        Err(MissionEvaluationPackageError::MalformedJson)
    );
}

#[test]
fn task_boundaries_controls_and_privacy_are_rejected() {
    let mut candidate = valid_package();
    candidate.task.clear();
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::TaskRequired)
    );
    candidate = valid_package();
    candidate.task = "x".repeat(MISSION_EVALUATION_TASK_MAX_BYTES + 1);
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::TaskTooLong)
    );
    candidate = valid_package();
    candidate.task = "Repair\0the schema".to_string();
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::TaskControlCharacter)
    );
    candidate = valid_package();
    candidate.task = "Send results to private.person@example.com".to_string();
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::SensitiveTask)
    );
}

#[test]
fn ids_and_digests_must_be_non_nil_and_lowercase() {
    let mut candidate = valid_package();
    candidate.mission_id = Uuid::nil();
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::NilMissionId)
    );
    candidate = valid_package();
    candidate.program_id = Uuid::nil();
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::NilProgramId)
    );
    candidate = valid_package();
    candidate.offer_version_hash = format!("sha256:{}", "A".repeat(64));
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::InvalidOfferVersionHash)
    );
    candidate = valid_package();
    candidate.offer_version_hash = "a".repeat(64);
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::InvalidOfferVersionHash)
    );
    candidate = valid_package();
    candidate.skill_sha256 = "A".repeat(64);
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::InvalidSkillSha256)
    );
    candidate = valid_package();
    candidate.evaluation_contract_hash = "A".repeat(64);
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::InvalidEvaluationContractHash)
    );
}

#[test]
fn schema_and_draft_validation_boundaries_are_enforced() {
    let mut candidate = valid_package();
    candidate.schema_version = 2;
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::UnsupportedSchemaVersion)
    );

    let boundary = SkillDraft {
        name: "a".repeat(SKILL_NAME_MAX_CHARS),
        description: "d".repeat(SKILL_DESCRIPTION_MAX_CHARS),
        procedure: "p".repeat(SKILL_PROCEDURE_MAX_CHARS),
    };
    assert_eq!(validate_draft(&boundary), Ok(()));
    let mut invalid = boundary.clone();
    invalid.description.push('d');
    assert_eq!(
        validate_draft(&invalid),
        Err(SkillDraftError::DescriptionTooLong)
    );
    invalid = boundary.clone();
    invalid.procedure.push('p');
    assert_eq!(
        validate_draft(&invalid),
        Err(SkillDraftError::ProcedureTooLong)
    );
    invalid = boundary;
    invalid.description = " description".to_string();
    assert_eq!(
        validate_draft(&invalid),
        Err(SkillDraftError::DescriptionRequired)
    );
    invalid.description = "description".to_string();
    invalid.procedure = "procedure\u{0001}".to_string();
    assert_eq!(
        validate_draft(&invalid),
        Err(SkillDraftError::ControlCharacter)
    );
    invalid.procedure = "Use token AKIAIOSFODNN7EXAMPLE to authenticate.".to_string();
    assert_eq!(
        validate_draft(&invalid),
        Err(SkillDraftError::SensitiveText)
    );
}

#[test]
fn standalone_skill_draft_remains_forward_compatible() {
    let draft: SkillDraft = serde_json::from_str(
        r#"{"name":"repair-generated-sources","description":"Repair source.","procedure":"Repair it.","future_field":true}"#,
    )
    .expect("legacy SkillDraft permits unknown fields");
    assert_eq!(draft.name, "repair-generated-sources");
}

#[test]
fn policy_rejects_each_mismatch_and_runtime_or_owner_change() {
    let mut candidate = valid_package();
    candidate.evaluator_id = "skill-evaluation-v2".to_string();
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::UnsupportedEvaluator)
    );
    candidate = valid_package();
    candidate.execution.required_model_owner = "other".to_string();
    assert_eq!(
        candidate.validate(),
        Err(MissionEvaluationPackageError::InvalidRequiredModelOwner)
    );
    let cases: [(
        fn(&mut MissionEvaluationPolicy, u32),
        u32,
        MissionEvaluationPackageError,
    ); 4] = [
        (
            |policy, value| policy.total_requests = value,
            MISSION_EVALUATION_TOTAL_REQUESTS,
            MissionEvaluationPackageError::TotalRequestsMismatch,
        ),
        (
            |policy, value| policy.output_token_limit = value,
            MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
            MissionEvaluationPackageError::OutputTokenLimitMismatch,
        ),
        (
            |policy, value| policy.request_timeout_seconds = value,
            MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
            MissionEvaluationPackageError::RequestTimeoutMismatch,
        ),
        (
            |policy, value| policy.max_concurrency = value,
            MISSION_EVALUATION_MAX_CONCURRENCY,
            MissionEvaluationPackageError::MaxConcurrencyMismatch,
        ),
    ];
    for (set, exact, expected) in cases {
        for value in [exact - 1, exact, exact + 1] {
            let mut changed = valid_package();
            set(&mut changed.execution, value);
            let result = changed.validate();
            if value == exact {
                assert_eq!(result, Ok(()));
            } else {
                assert_eq!(result, Err(expected.clone()));
            }
        }
    }
}
