//! INTEGRATION: verifies review replacement, replay, and fresh-client recovery behavior.

use std::path::PathBuf;

use super::*;

#[test]
fn delayed_differing_review_cannot_replace_a_paid_workflow() {
    let source = Uuid::new_v4();
    let mut state = SkillLoopState::default();
    let candidate = candidate(source);
    let delayed = reviewed(&candidate);
    let mut current_draft = candidate.draft.clone();
    current_draft.description.push_str(" Verify every output.");
    let current =
        crate::skill_loop::review_candidate(&candidate, current_draft).expect("current review");
    state.hold_candidate(candidate).expect("candidate");
    state
        .hold_review(current.clone(), None)
        .expect("current review");
    assert!(matches!(
        state.begin_evaluation(current.review_id),
        EvaluationStart::Started
    ));
    assert!(
        state
            .hold_review(delayed.clone(), Some(current.review_id))
            .is_none()
    );
    let current_report = report(&current);
    state.insert_evaluation(current_report.clone());

    assert!(state.hold_review(delayed.clone(), None).is_none());
    let planning_id = start_planning(&mut state, current_report.evaluation_id);
    assert!(
        state
            .hold_review(delayed.clone(), Some(current.review_id))
            .is_none()
    );

    let first_plan = install_plan(&current, current_report.evaluation_id);
    state
        .hold_install_plan(
            current_report.evaluation_id,
            planning_id,
            HeldInstallPlan {
                plan: first_plan.clone(),
                minted: Instant::now(),
            },
        )
        .expect("planned current review");
    assert!(state.hold_review(delayed.clone(), None).is_none());

    state.take_plan(first_plan.plan_id).expect("installing");
    assert!(
        state
            .hold_review(delayed.clone(), Some(current.review_id))
            .is_none()
    );
    state.finish_failed_install(first_plan.plan_id);

    let second_plan = install_plan(&current, current_report.evaluation_id);
    hold_plan(&mut state, second_plan.clone(), Instant::now());
    state.take_plan(second_plan.plan_id).expect("installing");
    state.insert_install(CodexInstalledSkill::from_test_receipt(
        InstalledSkill {
            install_id: Uuid::new_v4(),
            evaluation_id: current_report.evaluation_id,
            source_submission_id: source,
            tool: "Codex".to_string(),
            name: current.draft.name.clone(),
            target_location: "$CODEX_HOME/skills/repair-generated-sources".to_string(),
            skill_sha256: current.skill_sha256.clone(),
            marker_sha256: "marker-digest".to_string(),
            installed_at: "2026-09-11T00:00:00Z".to_string(),
        },
        PathBuf::from("/tmp/skill"),
    ));
    assert!(
        state
            .hold_review(delayed, Some(current.review_id))
            .is_none()
    );

    state.restore_after_rollback(source);
    assert!(matches!(
        state.begin_install_plan(current_report.evaluation_id),
        InstallPlanStart::Ready(_)
    ));
}

#[test]
fn changed_review_requires_the_exact_inactive_review_id() {
    let mut state = SkillLoopState::default();
    let candidate = candidate(Uuid::new_v4());
    let initial = reviewed(&candidate);
    let mut reviewed_draft = candidate.draft.clone();
    reviewed_draft.description.push_str(" First revision.");
    let reviewed_replacement = crate::skill_loop::review_candidate(&candidate, reviewed_draft)
        .expect("reviewed replacement");
    state.hold_candidate(candidate.clone()).expect("candidate");
    state
        .hold_review(initial.clone(), None)
        .expect("initial review");

    assert!(
        state
            .hold_review(reviewed_replacement.clone(), None)
            .is_none()
    );
    assert!(
        state
            .hold_review(reviewed_replacement.clone(), Some(Uuid::new_v4()))
            .is_none()
    );
    assert_eq!(
        state
            .hold_review(reviewed_replacement.clone(), Some(initial.review_id))
            .expect("replace reviewed phase")
            .review_id,
        reviewed_replacement.review_id
    );

    assert!(matches!(
        state.begin_evaluation(reviewed_replacement.review_id),
        EvaluationStart::Started
    ));
    let reviewed_report = report(&reviewed_replacement);
    state.insert_evaluation(reviewed_report);
    let mut evaluated_draft = candidate.draft.clone();
    evaluated_draft.description.push_str(" Second revision.");
    let evaluated_replacement = crate::skill_loop::review_candidate(&candidate, evaluated_draft)
        .expect("evaluated replacement");
    state
        .hold_review(
            evaluated_replacement.clone(),
            Some(reviewed_replacement.review_id),
        )
        .expect("replace evaluated phase");

    assert!(matches!(
        state.begin_evaluation(evaluated_replacement.review_id),
        EvaluationStart::Started
    ));
    let evaluated_report = report(&evaluated_replacement);
    state.insert_evaluation(evaluated_report.clone());
    let plan = install_plan(&evaluated_replacement, evaluated_report.evaluation_id);
    hold_plan(&mut state, plan, Instant::now());
    let mut planned_draft = candidate.draft.clone();
    planned_draft.description.push_str(" Third revision.");
    let planned_replacement = crate::skill_loop::review_candidate(&candidate, planned_draft)
        .expect("planned replacement");
    assert_eq!(
        state
            .hold_review(
                planned_replacement.clone(),
                Some(evaluated_replacement.review_id),
            )
            .expect("replace planned phase")
            .review_id,
        planned_replacement.review_id
    );
}

#[test]
fn initial_review_payload_remains_backward_compatible() {
    let params: ReviewParams = serde_json::from_value(serde_json::json!({
        "candidate_id": Uuid::new_v4(),
        "draft": {
            "name": "repair-generated-sources",
            "description": "Use when generated files have an authoritative source.",
            "procedure": "Read the generator, edit the source, regenerate, and verify."
        }
    }))
    .expect("legacy review payload");

    assert!(params.replaces_review_id.is_none());
}

#[test]
fn fresh_client_recovers_the_approved_draft_and_review_token_from_every_review_phase() {
    let source = Uuid::new_v4();
    let stored_candidate = candidate(source);
    let original_draft = stored_candidate.draft.clone();
    let review = custom_review(&stored_candidate);
    let evaluation = report(&review);
    let evaluated = || {
        Box::new(EvaluatedSkill {
            review: review.clone(),
            report: evaluation.clone(),
        })
    };
    let plan = install_plan(&review, evaluation.evaluation_id);
    let phases = vec![
        ("reviewed", SkillWorkflowPhase::Reviewed(review.clone())),
        ("evaluating", SkillWorkflowPhase::Evaluating(review.clone())),
        ("evaluated", SkillWorkflowPhase::Evaluated(evaluated())),
        (
            "planning",
            SkillWorkflowPhase::Planning {
                evaluated: evaluated(),
                planning_id: Uuid::new_v4(),
            },
        ),
        (
            "planned",
            SkillWorkflowPhase::Planned {
                evaluated: evaluated(),
                held: HeldInstallPlan {
                    plan: plan.clone(),
                    minted: Instant::now(),
                },
            },
        ),
        (
            "installing",
            SkillWorkflowPhase::Installing {
                evaluated: evaluated(),
                plan_id: plan.plan_id,
            },
        ),
    ];

    for (expected_phase, phase) in phases {
        let active_evaluation =
            matches!(&phase, SkillWorkflowPhase::Evaluating(_)).then_some(review.review_id);
        let mut state = SkillLoopState {
            workflows: VecDeque::from([SkillWorkflow {
                candidate: stored_candidate.clone(),
                phase,
            }]),
            active_evaluation,
            ..SkillLoopState::default()
        };

        let recovered = state
            .hold_candidate(candidate(source))
            .expect("fresh client candidate replay");
        assert_eq!(recovered.candidate_id, stored_candidate.candidate_id);
        assert_eq!(recovered.draft, review.draft, "{expected_phase}");
        assert_eq!(
            recovered.replaces_review_id,
            Some(review.review_id),
            "{expected_phase}"
        );
        assert_eq!(phase_name(&state.workflows[0].phase), expected_phase);
        assert_eq!(state.workflows[0].candidate.draft, original_draft);
        assert_eq!(state.workflows[0].candidate.replaces_review_id, None);
    }
}

#[test]
fn identical_review_replay_is_read_only_during_active_phases() {
    let source = Uuid::new_v4();
    let stored_candidate = candidate(source);
    let review = custom_review(&stored_candidate);
    let evaluation = report(&review);
    let evaluated = || {
        Box::new(EvaluatedSkill {
            review: review.clone(),
            report: evaluation.clone(),
        })
    };
    let phases = vec![
        ("evaluating", SkillWorkflowPhase::Evaluating(review.clone())),
        (
            "planning",
            SkillWorkflowPhase::Planning {
                evaluated: evaluated(),
                planning_id: Uuid::new_v4(),
            },
        ),
        (
            "installing",
            SkillWorkflowPhase::Installing {
                evaluated: evaluated(),
                plan_id: Uuid::new_v4(),
            },
        ),
    ];

    for (expected_phase, phase) in phases {
        let active_evaluation =
            matches!(&phase, SkillWorkflowPhase::Evaluating(_)).then_some(review.review_id);
        let mut state = SkillLoopState {
            workflows: VecDeque::from([SkillWorkflow {
                candidate: stored_candidate.clone(),
                phase,
            }]),
            active_evaluation,
            ..SkillLoopState::default()
        };
        let recovered = state
            .hold_candidate(candidate(source))
            .expect("fresh client candidate replay");
        let duplicate = crate::skill_loop::review_candidate(&recovered, recovered.draft.clone())
            .expect("identical review replay");

        let held = state
            .hold_review(duplicate, recovered.replaces_review_id)
            .expect("idempotent review readback");
        assert_eq!(held.review_id, review.review_id, "{expected_phase}");
        assert_eq!(phase_name(&state.workflows[0].phase), expected_phase);
    }
}

#[test]
fn fresh_client_reuses_the_paid_evaluation_for_a_custom_review() {
    let source = Uuid::new_v4();
    let stored_candidate = candidate(source);
    let review = custom_review(&stored_candidate);
    let evaluation = report(&review);
    let mut state = SkillLoopState {
        workflows: VecDeque::from([SkillWorkflow {
            candidate: stored_candidate,
            phase: SkillWorkflowPhase::Evaluated(Box::new(EvaluatedSkill {
                review: review.clone(),
                report: evaluation.clone(),
            })),
        }]),
        active_evaluation: None,
        ..SkillLoopState::default()
    };

    let recovered = state
        .hold_candidate(candidate(source))
        .expect("fresh client candidate replay");
    let duplicate = crate::skill_loop::review_candidate(&recovered, recovered.draft.clone())
        .expect("identical review replay");
    let held = state
        .hold_review(duplicate, recovered.replaces_review_id)
        .expect("held custom review");
    assert_eq!(held.review_id, review.review_id);
    match state.begin_evaluation(held.review_id) {
        EvaluationStart::Existing(existing) => assert_eq!(*existing, evaluation),
        _ => panic!("paid evaluation must be reused"),
    }
    assert_eq!(phase_name(&state.workflows[0].phase), "evaluated");
}
