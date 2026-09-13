//! INTEGRATION: verifies bounded workflow eviction preserves recoverable work.

use std::path::PathBuf;

use super::*;

#[test]
fn bounded_eviction_preserves_the_active_workflow() {
    let mut state = SkillLoopState::default();
    let active_candidate = state
        .hold_candidate(candidate(Uuid::new_v4()))
        .expect("active candidate");
    let active_review = reviewed(&active_candidate);
    state
        .hold_review(active_review.clone(), None)
        .expect("active review");
    assert!(matches!(
        state.begin_evaluation(active_review.review_id),
        EvaluationStart::Started
    ));
    for _ in 1..MAX_HELD_ITEMS {
        state
            .hold_candidate(candidate(Uuid::new_v4()))
            .expect("candidate below capacity");
    }
    state
        .hold_candidate(candidate(Uuid::new_v4()))
        .expect("candidate with eviction");

    assert_eq!(state.workflows.len(), MAX_HELD_ITEMS);
    assert!(state.candidate(active_candidate.candidate_id).is_some());
    assert_eq!(state.active_evaluation, Some(active_review.review_id));
}

#[test]
fn bounded_eviction_preserves_an_active_evaluation_and_unexpired_plan() {
    let mut state = SkillLoopState::default();
    let (_, planned_review, planned_report) = advance_to_evaluated(&mut state, Uuid::new_v4());
    let planned = install_plan(&planned_review, planned_report.evaluation_id);
    hold_plan(&mut state, planned.clone(), Instant::now());

    let active_candidate = state
        .hold_candidate(candidate(Uuid::new_v4()))
        .expect("active candidate");
    let active_review = reviewed(&active_candidate);
    state
        .hold_review(active_review.clone(), None)
        .expect("active review");
    assert!(matches!(
        state.begin_evaluation(active_review.review_id),
        EvaluationStart::Started
    ));
    for _ in state.workflows.len()..MAX_HELD_ITEMS {
        state
            .hold_candidate(candidate(Uuid::new_v4()))
            .expect("candidate below capacity");
    }
    state
        .hold_candidate(candidate(Uuid::new_v4()))
        .expect("candidate with eviction");

    assert_eq!(state.workflows.len(), MAX_HELD_ITEMS);
    assert!(state.candidate(active_candidate.candidate_id).is_some());
    match state.begin_install_plan(planned_report.evaluation_id) {
        InstallPlanStart::Existing(existing) => assert_eq!(*existing, planned.preview().clone()),
        _ => panic!("unexpired plan was evicted"),
    }
}

#[test]
fn bounded_eviction_keeps_install_receipt_orthogonal_to_workflow_history() {
    let mut state = SkillLoopState::default();
    let (evaluated_candidate, _, _) = advance_to_evaluated(&mut state, Uuid::new_v4());
    let installed_source = Uuid::new_v4();
    let installed_candidate = state
        .hold_candidate(candidate(installed_source))
        .expect("installed candidate");
    state.insert_install(CodexInstalledSkill::from_test_receipt(
        InstalledSkill {
            install_id: Uuid::new_v4(),
            evaluation_id: Uuid::new_v4(),
            source_submission_id: installed_source,
            tool: "Codex".to_string(),
            name: "repair-generated-sources".to_string(),
            target_location: "$CODEX_HOME/skills/repair-generated-sources".to_string(),
            skill_sha256: "digest".to_string(),
            marker_sha256: "marker-digest".to_string(),
            installed_at: "2026-09-11T00:00:00Z".to_string(),
        },
        PathBuf::from("/tmp/skill"),
    ));
    for _ in state.workflows.len()..MAX_HELD_ITEMS {
        state
            .hold_candidate(candidate(Uuid::new_v4()))
            .expect("candidate below capacity");
    }
    state
        .hold_candidate(candidate(Uuid::new_v4()))
        .expect("candidate with eviction");

    assert!(state.candidate(evaluated_candidate.candidate_id).is_some());
    assert!(state.candidate(installed_candidate.candidate_id).is_none());
    assert!(state.install_receipt(installed_source).is_some());
}

#[test]
fn bounded_eviction_retains_the_evaluation_inside_an_expired_plan() {
    let mut state = SkillLoopState::default();
    let (evaluated_candidate, review, report) = advance_to_evaluated(&mut state, Uuid::new_v4());
    let plan = install_plan(&review, report.evaluation_id);
    hold_plan(
        &mut state,
        plan,
        Instant::now() - INSTALL_PLAN_TTL - Duration::from_secs(1),
    );
    for _ in state.workflows.len()..MAX_HELD_ITEMS {
        state
            .hold_candidate(candidate(Uuid::new_v4()))
            .expect("candidate below capacity");
    }
    state
        .hold_candidate(candidate(Uuid::new_v4()))
        .expect("candidate with eviction");

    assert!(state.candidate(evaluated_candidate.candidate_id).is_some());
    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Ready(_)
    ));
}
