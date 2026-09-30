//! INTEGRATION: exercises tested-skill daemon state transitions and forced interleavings.

use std::path::PathBuf;

use super::*;

fn candidate(source_submission_id: Uuid) -> SkillCandidate {
    SkillCandidate {
        candidate_id: Uuid::new_v4(),
        family: crate::skill_loop::GENERATED_SOURCE_FAMILY,
        source_submission_id,
        source_correction: "Change the source schema and regenerate the generated file."
            .to_string(),
        source_evidence: Vec::new(),
        source_task_fingerprint: crate::skill_loop::source_task_fingerprint(
            "Repair a generated GraphQL client from its source schema.",
        )
        .expect("source fingerprint"),
        draft: SkillDraft {
            name: "repair-generated-sources".to_string(),
            description: "Use when generated files have an authoritative source.".to_string(),
            procedure: "Read the generator, edit the source, regenerate, and verify.".to_string(),
        },
        replaces_review_id: None,
        manual_control_instruction: crate::skill_loop::MANUAL_CONTROL_INSTRUCTION,
        evaluation_contract: crate::skill_loop::evaluation_contract(),
    }
}

fn reviewed(candidate: &SkillCandidate) -> SkillReview {
    crate::skill_loop::review_candidate(candidate, candidate.draft.clone()).expect("review fixture")
}

fn report(review: &SkillReview) -> SkillEvaluationReport {
    SkillEvaluationReport {
        evaluation_id: Uuid::new_v4(),
        review_id: review.review_id,
        skill_sha256: review.skill_sha256.clone(),
        contract: crate::skill_loop::evaluation_contract(),
        execution: crate::skill_loop::SkillEvaluationExecution {
            requested_model: "nearai/test".to_string(),
            served_model: "nearai/test".to_string(),
            model_owned_by: "nearai",
            source_task_fingerprint_sha256: review.source_task_fingerprint.sha256.clone(),
            selected_plan_task_ids: Vec::new(),
            excluded_source_overlap_task_ids: Vec::new(),
            reserve_plan_task_ids: Vec::new(),
            ambient_context: Vec::new(),
        },
        summaries: Vec::new(),
        plan_summaries: Vec::new(),
        applicability_summaries: Vec::new(),
        trials: Vec::new(),
        regressions: Vec::new(),
        install_allowed: true,
        gate_reason: "skill-improved-over-both-controls",
    }
}

fn custom_review(candidate: &SkillCandidate) -> SkillReview {
    let mut draft = candidate.draft.clone();
    draft
        .description
        .push_str(" Verify every generated output.");
    crate::skill_loop::review_candidate(candidate, draft).expect("custom review fixture")
}

fn installed(review: &SkillReview, evaluation_id: Uuid) -> CodexInstalledSkill {
    CodexInstalledSkill::from_test_receipt(
        InstalledSkill {
            install_id: Uuid::new_v4(),
            evaluation_id,
            source_submission_id: review.source_submission_id,
            tool: "Codex".to_string(),
            name: review.draft.name.clone(),
            target_location: "$CODEX_HOME/skills/repair-generated-sources".to_string(),
            skill_sha256: review.skill_sha256.clone(),
            marker_sha256: "marker-digest".to_string(),
            installed_at: "2026-09-11T00:00:00Z".to_string(),
        },
        PathBuf::from("/tmp/skill"),
    )
}

fn phase_name(phase: &SkillWorkflowPhase) -> &'static str {
    match phase {
        SkillWorkflowPhase::Candidate => "candidate",
        SkillWorkflowPhase::Reviewed(_) => "reviewed",
        SkillWorkflowPhase::Evaluating(_) => "evaluating",
        SkillWorkflowPhase::Evaluated(_) => "evaluated",
        SkillWorkflowPhase::Planning { .. } => "planning",
        SkillWorkflowPhase::Planned { .. } => "planned",
        SkillWorkflowPhase::Installing { .. } => "installing",
    }
}

fn advance_to_evaluated(
    state: &mut SkillLoopState,
    source_submission_id: Uuid,
) -> (SkillCandidate, SkillReview, SkillEvaluationReport) {
    let candidate = candidate(source_submission_id);
    let review = reviewed(&candidate);
    state
        .hold_candidate(candidate.clone())
        .expect("hold candidate");
    state
        .hold_review(review.clone(), None)
        .expect("hold review");
    assert!(matches!(
        state.begin_evaluation(review.review_id),
        EvaluationStart::Started
    ));
    let report = report(&review);
    state.insert_evaluation(report.clone());
    (candidate, review, report)
}

fn install_plan(review: &SkillReview, evaluation_id: Uuid) -> CodexInstallPlan {
    let preview = SkillInstallPlan {
        plan_id: Uuid::new_v4(),
        evaluation_id,
        tool: "Codex",
        target_location: "$CODEX_HOME/skills/repair-generated-sources".to_string(),
        skill_location: "$CODEX_HOME/skills/repair-generated-sources/SKILL.md".to_string(),
        marker_location: "$CODEX_HOME/skills/repair-generated-sources/.trace-commons-install.json"
            .to_string(),
        occupied: false,
        can_install: true,
        skill_md: review.skill_md.clone(),
        skill_sha256: review.skill_sha256.clone(),
        marker_json: "{}".to_string(),
        marker_file_sha256: "marker-file-digest".to_string(),
    };
    CodexInstallPlan::from_test_preview(preview, PathBuf::from("/tmp/skill"))
}

fn start_planning(state: &mut SkillLoopState, evaluation_id: Uuid) -> Uuid {
    match state.begin_install_plan(evaluation_id) {
        InstallPlanStart::Ready(start) => start.2,
        _ => panic!("evaluation is not ready for planning"),
    }
}

fn hold_plan(
    state: &mut SkillLoopState,
    plan: CodexInstallPlan,
    minted: Instant,
) -> CodexInstallPlan {
    let planning_id = start_planning(state, plan.evaluation_id);
    let returned = plan.clone();
    state
        .hold_install_plan(
            plan.evaluation_id,
            planning_id,
            HeldInstallPlan { plan, minted },
        )
        .expect("hold install plan");
    returned
}

#[path = "tests/credentials.rs"]
mod credentials;
#[path = "tests/evaluation_admission.rs"]
mod evaluation_admission;
#[path = "tests/eviction.rs"]
mod eviction;
#[path = "tests/installation.rs"]
mod installation;
#[path = "tests/ipc_workflow.rs"]
mod ipc_workflow;
#[path = "tests/review_recovery.rs"]
mod review_recovery;
