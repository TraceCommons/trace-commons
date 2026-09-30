use super::*;
use crate::public_run::SessionEvidenceCandidate;

fn detail(correction: Option<&str>) -> SessionDetail {
    SessionDetail {
        content_unavailable: false,
        task: Some("Repair the generated package manifest from its source schema.".to_string()),
        contribution_status: Some(
            trace_commons_protocol::public_run::PublicRunContributionStatus::Accepted,
        ),
        permitted_uses: Vec::new(),
        task_success: Some(trace_commons_protocol::trace_contribution::TaskSuccess::Partial),
        user_feedback: Some(UserFeedback::Correction),
        human_correction: correction.map(str::to_string),
        evidence: vec![SessionEvidenceCandidate {
            event_id: Uuid::nil(),
            kind: TraceContributionEventType::AssistantMessage,
            excerpt: "Changed the checked-in output directly.".to_string(),
        }],
        contributed_version: "2.0".to_string(),
        consent_policy_version: "2.0".to_string(),
        redaction_pipeline_version: "2.0".to_string(),
        publication: None,
        publication_version: 0,
        retained_source_slug: None,
    }
}

#[test]
fn trigger_requires_generated_source_and_corrective_action_signals() {
    assert!(correction_matches_generated_source_family(
        "Do not edit the generated file. Change the source schema and regenerate it."
    ));
    assert!(!correction_matches_generated_source_family(
        "The generated file has the wrong value."
    ));
    assert!(!correction_matches_generated_source_family(
        "Change the source schema before the release."
    ));
    assert!(!correction_matches_generated_source_family(
        "The generator documentation is useful."
    ));
}

#[test]
fn candidate_keeps_source_evidence_out_of_the_package() {
    let correction = "Do not edit the generated file. Change the source schema and regenerate it.";
    let candidate =
        propose_candidate(Uuid::nil(), &detail(Some(correction))).expect("supported correction");
    assert_eq!(candidate.replaces_review_id, None);
    assert_eq!(
        candidate.source_evidence[0].kind,
        TraceContributionEventType::AssistantMessage
    );
    let review = review_candidate(&candidate, candidate.draft.clone()).expect("valid review");
    assert!(!review.skill_md.contains(correction));
    assert!(
        !review
            .skill_md
            .contains("Changed the checked-in output directly.")
    );
    assert_eq!(review.source_evidence_ids, vec![Uuid::nil()]);
    assert_eq!(review.skill_sha256, sha256(review.skill_md.as_bytes()));
}

#[test]
fn unsupported_corrections_are_not_made_generic() {
    let result = propose_candidate(
        Uuid::nil(),
        &detail(Some(
            "Retry the network request after renewing the session.",
        )),
    );
    assert_eq!(result, Err("skill-family-not-supported"));
}

#[test]
fn non_accepted_sessions_cannot_produce_a_skill() {
    let mut source = detail(Some(
        "Do not edit the generated file. Change the source schema and regenerate it.",
    ));
    source.contribution_status =
        Some(trace_commons_protocol::public_run::PublicRunContributionStatus::Quarantined);
    assert_eq!(
        propose_candidate(Uuid::nil(), &source),
        Err("skill-session-not-accepted")
    );
}

#[test]
fn review_refuses_credentials_and_wallet_recovery_cues() {
    let mut draft = SkillDraft {
        name: DEFAULT_SKILL_NAME.to_string(),
        description: DEFAULT_DESCRIPTION.to_string(),
        procedure: DEFAULT_PROCEDURE.to_string(),
    };
    draft.procedure.push_str("\nUse recovery phrase here.");
    assert_eq!(validate_draft(&draft), Err(SkillDraftError::SensitiveText));
    draft.procedure = format!(
        "{}\napi_key = {}{}",
        DEFAULT_PROCEDURE,
        "sk-proj-",
        "x".repeat(16)
    );
    assert_eq!(validate_draft(&draft), Err(SkillDraftError::SensitiveText));

    let mnemonic = std::iter::repeat_n("abandon", 11)
        .chain(["about"])
        .collect::<Vec<_>>()
        .join(" ");
    draft.procedure = format!("{DEFAULT_PROCEDURE}\n{mnemonic}");
    assert_eq!(validate_draft(&draft), Err(SkillDraftError::SensitiveText));

    for prefix in ["ED25519:", "Ed25519:"] {
        draft.procedure = format!("{DEFAULT_PROCEDURE}\n{prefix}{}", "1".repeat(88));
        assert_eq!(
            validate_draft(&draft),
            Err(SkillDraftError::SensitiveText),
            "NEAR private-key prefixes must be rejected case-insensitively"
        );
    }
}

#[test]
fn names_are_bounded_lowercase_kebab_case() {
    assert!(valid_skill_name("repair-generated-sources"));
    assert!(valid_skill_name(&"a".repeat(SKILL_NAME_MAX_CHARS)));
    assert!(!valid_skill_name(&"a".repeat(SKILL_NAME_MAX_CHARS + 1)));
    for value in [
        "",
        "Repair-files",
        "repair_files",
        "-repair",
        "repair-",
        "a--b",
    ] {
        assert!(!valid_skill_name(value), "{value}");
    }
}

#[test]
fn draft_text_limits_accept_the_boundary_and_reject_one_character_more() {
    let boundary = SkillDraft {
        name: "boundary-skill".to_string(),
        description: "d".repeat(SKILL_DESCRIPTION_MAX_CHARS),
        procedure: "p".repeat(SKILL_PROCEDURE_MAX_CHARS),
    };
    assert_eq!(validate_draft(&boundary), Ok(()));

    let mut too_long = boundary.clone();
    too_long.description.push('d');
    assert_eq!(
        validate_draft(&too_long),
        Err(SkillDraftError::DescriptionTooLong)
    );
    too_long = boundary;
    too_long.procedure.push('p');
    assert_eq!(
        validate_draft(&too_long),
        Err(SkillDraftError::ProcedureTooLong)
    );
}

#[test]
fn source_task_fingerprint_matches_only_exact_or_lexically_near_tasks() {
    let source = source_task_fingerprint(
        "Change the Windows package tile background and regenerate every display scale.",
    )
    .expect("source fingerprint");
    let near = source_task_fingerprint(
        "Change the Windows package tile background, then regenerate every display scale.",
    )
    .expect("near fingerprint");
    let unrelated = source_task_fingerprint(
        "Correct punctuation in a handwritten README without changing generated files.",
    )
    .expect("unrelated fingerprint");
    assert!(source.is_exact_or_near_duplicate(&source));
    assert_eq!(
        similarity_distance(&source.similarity_hash, &near.similarity_hash),
        Some(32)
    );
    assert!(source.is_exact_or_near_duplicate(&near));
    assert!(!source.is_exact_or_near_duplicate(&unrelated));

    let embedded = source_task_fingerprint(
        "Change the Windows package tile background and regenerate every display scale while also replacing the network protocol, rewriting account storage, migrating databases, and updating unrelated documentation.",
    )
    .expect("embedded fingerprint");
    assert!(!source.is_exact_or_near_duplicate(&embedded));
}
