//! INTEGRATION: verifies evaluation admission, deduplication, and reconciliation behavior.

use std::path::PathBuf;

use super::*;

#[test]
fn install_status_during_evaluation_does_not_discard_the_result() {
    let source = Uuid::new_v4();
    let mut state = SkillLoopState::default();
    let candidate = state.hold_candidate(candidate(source)).expect("candidate");
    let review = reviewed(&candidate);
    state.hold_review(review.clone(), None).expect("review");
    assert!(matches!(
        state.begin_evaluation(review.review_id),
        EvaluationStart::Started
    ));
    let receipt = CodexInstalledSkill::from_test_receipt(
        InstalledSkill {
            install_id: Uuid::new_v4(),
            evaluation_id: Uuid::new_v4(),
            source_submission_id: source,
            tool: "Codex".to_string(),
            name: review.draft.name.clone(),
            target_location: "$CODEX_HOME/skills/repair-generated-sources".to_string(),
            skill_sha256: review.skill_sha256.clone(),
            marker_sha256: "marker-digest".to_string(),
            installed_at: "2026-09-11T00:00:00Z".to_string(),
        },
        PathBuf::from("/tmp/skill"),
    );
    state.insert_install(receipt.clone());
    assert_eq!(state.install_receipt(source), Some(receipt));
    let report = report(&review);
    state.insert_evaluation(report.clone());

    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Ready(_)
    ));
}

#[test]
fn identical_reviews_and_concurrent_evaluations_are_deduplicated() {
    let mut state = SkillLoopState::default();
    let candidate = candidate(Uuid::new_v4());
    let review = reviewed(&candidate);
    state.hold_candidate(candidate).expect("hold candidate");
    let held = state
        .hold_review(review.clone(), None)
        .expect("hold review");
    let mut duplicate = review.clone();
    duplicate.review_id = Uuid::new_v4();
    assert_eq!(
        state
            .hold_review(duplicate, None)
            .expect("deduplicated")
            .review_id,
        held.review_id
    );

    assert!(matches!(
        state.begin_evaluation(held.review_id),
        EvaluationStart::Started
    ));
    assert!(matches!(
        state.begin_evaluation(held.review_id),
        EvaluationStart::InProgress
    ));
    state.finish_failed_evaluation(held.review_id);
    assert!(matches!(
        state.begin_evaluation(held.review_id),
        EvaluationStart::Started
    ));
}

#[test]
fn one_daemon_evaluation_admission_covers_distinct_reviews() {
    let mut state = SkillLoopState::default();
    let first_candidate = candidate(Uuid::new_v4());
    let second_candidate = candidate(Uuid::new_v4());
    let first = reviewed(&first_candidate);
    let second = reviewed(&second_candidate);
    state
        .hold_candidate(first_candidate)
        .expect("first candidate");
    state
        .hold_candidate(second_candidate)
        .expect("second candidate");
    state
        .hold_review(first.clone(), None)
        .expect("first review");
    state
        .hold_review(second.clone(), None)
        .expect("second review");

    assert!(matches!(
        state.begin_evaluation(first.review_id),
        EvaluationStart::Started
    ));
    assert!(matches!(
        state.begin_evaluation(second.review_id),
        EvaluationStart::Busy
    ));
    state.finish_failed_evaluation(first.review_id);
    assert!(matches!(
        state.begin_evaluation(second.review_id),
        EvaluationStart::Started
    ));
}

#[test]
fn repeated_candidate_request_cannot_replace_an_active_workflow() {
    let source = Uuid::new_v4();
    let mut state = SkillLoopState::default();
    let original = state
        .hold_candidate(candidate(source))
        .expect("original candidate");
    let review = reviewed(&original);
    state.hold_review(review.clone(), None).expect("review");
    assert!(matches!(
        state.begin_evaluation(review.review_id),
        EvaluationStart::Started
    ));

    let repeated = state
        .hold_candidate(candidate(source))
        .expect("repeated candidate");
    assert_eq!(repeated.candidate_id, original.candidate_id);
    assert!(matches!(
        state.begin_evaluation(review.review_id),
        EvaluationStart::InProgress
    ));
}
#[test]
fn rollback_reconciliation_does_not_clobber_an_active_evaluation() {
    let source = Uuid::new_v4();
    let mut state = SkillLoopState::default();
    let candidate = state.hold_candidate(candidate(source)).expect("candidate");
    let review = reviewed(&candidate);
    state.hold_review(review.clone(), None).expect("review");
    assert!(matches!(
        state.begin_evaluation(review.review_id),
        EvaluationStart::Started
    ));

    state.restore_after_rollback(source);
    assert_eq!(state.active_evaluation, Some(review.review_id));
    assert!(matches!(
        state.begin_evaluation(review.review_id),
        EvaluationStart::InProgress
    ));
}
