//! INTEGRATION: verifies install planning, commit, rollback, and async transaction behavior.

use std::sync::Arc;
use std::sync::mpsc;

use super::*;

#[test]
fn held_install_plans_are_single_use() {
    let mut state = SkillLoopState::default();
    let (_, review, report) = advance_to_evaluated(&mut state, Uuid::new_v4());
    let plan = install_plan(&review, report.evaluation_id);
    assert_eq!(hold_plan(&mut state, plan.clone(), Instant::now()), plan);
    assert!(state.take_plan(plan.plan_id).is_some());
    assert!(state.take_plan(plan.plan_id).is_none());
}

#[test]
fn install_plan_retries_return_the_same_plan_and_expired_plans_are_replaced() {
    let mut state = SkillLoopState::default();
    let (_, review, report) = advance_to_evaluated(&mut state, Uuid::new_v4());
    let plan = install_plan(&review, report.evaluation_id);
    hold_plan(&mut state, plan.clone(), Instant::now());
    match state.begin_install_plan(report.evaluation_id) {
        InstallPlanStart::Existing(existing) => assert_eq!(*existing, plan.preview().clone()),
        _ => panic!("an unexpired retry must return the held plan"),
    }

    let position = state
        .workflows
        .iter_mut()
        .position(|workflow| matches!(&workflow.phase, SkillWorkflowPhase::Planned { .. }))
        .expect("planned workflow");
    if let SkillWorkflowPhase::Planned { held, .. } = &mut state.workflows[position].phase {
        held.minted = Instant::now() - INSTALL_PLAN_TTL - Duration::from_secs(1);
    }
    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Ready(_)
    ));
    assert!(state.take_plan(plan.plan_id).is_none());
}

#[test]
fn failed_install_and_rollback_restore_the_passing_evaluation() {
    let source = Uuid::new_v4();
    let mut state = SkillLoopState::default();
    let (_, review, report) = advance_to_evaluated(&mut state, source);
    let first_plan = install_plan(&review, report.evaluation_id);
    hold_plan(&mut state, first_plan.clone(), Instant::now());
    state
        .take_plan(first_plan.plan_id)
        .expect("take first plan");
    state.finish_failed_install(first_plan.plan_id);

    let second_plan = install_plan(&review, report.evaluation_id);
    hold_plan(&mut state, second_plan.clone(), Instant::now());
    state
        .take_plan(second_plan.plan_id)
        .expect("take second plan");
    let first_receipt = installed(&review, report.evaluation_id);
    state.insert_install(first_receipt.clone());
    assert_eq!(state.install_receipt(source), Some(first_receipt));
    state.restore_after_rollback(source);
    assert_eq!(state.install_receipt(source), None);
    let planning_id = start_planning(&mut state, report.evaluation_id);
    state.finish_planning(planning_id);

    let second_receipt = installed(&review, report.evaluation_id);
    state.insert_install(second_receipt.clone());
    assert_eq!(state.install_receipt(source), Some(second_receipt));
    state.clear_missing_install(source);
    assert_eq!(state.install_receipt(source), None);
    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Ready(_)
    ));
}

#[test]
fn status_reconciliation_refuses_a_recomputed_marker_without_replacing_the_receipt() {
    let source = Uuid::new_v4();
    let mut state = SkillLoopState::default();
    let (_, review, report) = advance_to_evaluated(&mut state, source);
    let receipt = installed(&review, report.evaluation_id);
    state.insert_install(receipt.clone());

    let mut recomputed_marker_status = receipt.clone();
    recomputed_marker_status.marker_sha256 = "different-self-consistent-marker".to_string();
    assert_eq!(
        state.reconcile_install_status(source, Some(recomputed_marker_status)),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert_eq!(state.install_receipt(source), Some(receipt));
}

#[test]
fn restart_recovery_holds_the_first_disk_receipt_until_status_is_missing() {
    let source = Uuid::new_v4();
    let review = reviewed(&candidate(source));
    let receipt = installed(&review, Uuid::new_v4());
    let mut state = SkillLoopState::default();

    assert_eq!(
        state.reconcile_install_status(source, Some(receipt.clone())),
        Ok(Some(receipt.clone()))
    );
    assert_eq!(state.install_receipt(source), Some(receipt.clone()));

    let mut later_status = receipt.clone();
    later_status.marker_sha256 = "later-marker-digest".to_string();
    assert_eq!(
        state.reconcile_install_status(source, Some(later_status)),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert_eq!(state.install_receipt(source), Some(receipt));

    assert_eq!(state.reconcile_install_status(source, None), Ok(None));
    assert_eq!(state.install_receipt(source), None);
}

#[test]
fn restart_recovery_receipts_survive_workflow_capacity() {
    let mut state = SkillLoopState::default();
    let mut sources = Vec::new();
    for _ in 0..=MAX_HELD_ITEMS {
        let source = Uuid::new_v4();
        let review = reviewed(&candidate(source));
        let receipt = installed(&review, Uuid::new_v4());
        state
            .reconcile_install_status(source, Some(receipt))
            .expect("recover disk receipt");
        sources.push(source);
    }

    assert_eq!(state.install_receipts.len(), MAX_HELD_ITEMS + 1);
    assert!(state.install_receipt(sources[0]).is_some());
    assert!(
        state
            .install_receipt(*sources.last().expect("last source"))
            .is_some()
    );
}

#[test]
fn authenticated_owner_switch_invalidates_workflows_and_install_receipts() {
    let source = Uuid::new_v4();
    let mut state = SkillLoopState::default();
    state.bind_owner("sha256:owner-a");
    let (_, review, report) = advance_to_evaluated(&mut state, source);
    state.insert_install(installed(&review, report.evaluation_id));
    assert!(state.install_receipt(source).is_some());
    assert!(!state.workflows.is_empty());

    state.bind_owner("sha256:owner-a");
    assert!(state.install_receipt(source).is_some());

    state.bind_owner("sha256:owner-b");
    assert!(state.workflows.is_empty());
    assert!(state.install_receipts.is_empty());
    assert!(state.active_evaluation.is_none());
}

#[test]
fn stale_install_cleanup_cannot_reset_a_different_plan_attempt() {
    let mut state = SkillLoopState::default();
    let (_, review, report) = advance_to_evaluated(&mut state, Uuid::new_v4());
    let plan = install_plan(&review, report.evaluation_id);
    hold_plan(&mut state, plan.clone(), Instant::now());
    state.take_plan(plan.plan_id).expect("take plan");

    state.finish_failed_install(Uuid::new_v4());
    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Unknown
    ));
    state.finish_failed_install(plan.plan_id);
    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Ready(_)
    ));
}

#[test]
fn planning_admission_protects_the_evaluation_from_capacity_eviction() {
    let mut state = SkillLoopState::default();
    let (target_candidate, target_review, target_report) =
        advance_to_evaluated(&mut state, Uuid::new_v4());
    for _ in 1..MAX_HELD_ITEMS {
        advance_to_evaluated(&mut state, Uuid::new_v4());
    }

    let planning_id = start_planning(&mut state, target_report.evaluation_id);
    state
        .hold_candidate(candidate(Uuid::new_v4()))
        .expect("candidate with eviction");
    assert!(state.candidate(target_candidate.candidate_id).is_some());

    let plan = install_plan(&target_review, target_report.evaluation_id);
    assert_eq!(
        state
            .hold_install_plan(
                target_report.evaluation_id,
                planning_id,
                HeldInstallPlan {
                    plan: plan.clone(),
                    minted: Instant::now(),
                },
            )
            .expect("planning workflow survived"),
        plan.preview().clone()
    );
}

#[test]
fn planning_cleanup_is_token_scoped_and_restores_the_evaluation() {
    let mut state = SkillLoopState::default();
    let (_, _, report) = advance_to_evaluated(&mut state, Uuid::new_v4());
    let first_planning_id = start_planning(&mut state, report.evaluation_id);

    state.finish_planning(Uuid::new_v4());
    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Unknown
    ));
    state.finish_planning(first_planning_id);

    let second_planning_id = start_planning(&mut state, report.evaluation_id);
    state.finish_planning(first_planning_id);
    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Unknown
    ));
    state.finish_planning(second_planning_id);
    assert!(matches!(
        state.begin_install_plan(report.evaluation_id),
        InstallPlanStart::Ready(_)
    ));
}

#[test]
fn status_absent_then_commit_keeps_cache_equal_to_disk() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let (_identity_directory, identity_store) = crate::config::tests_support::temp_store();
    let identity = Arc::new(
        crate::identity::DeviceIdentity::load_or_generate(&identity_store)
            .expect("test device identity"),
    );
    let owner_scope = trace_commons_protocol::onboarding::user_subject_hash("install-state-owner");
    let source = Uuid::new_v4();
    let mut workflow = SkillLoopState::default();
    let (_, review, report) = advance_to_evaluated(&mut workflow, source);
    let plan = crate::skill_loop::plan_codex_install(
        &review,
        report.evaluation_id,
        &root,
        identity.as_ref(),
        &owner_scope,
    )
    .expect("install plan");
    hold_plan(&mut workflow, plan.clone(), Instant::now());
    workflow.take_plan(plan.plan_id).expect("take plan");
    let state = Arc::new(std::sync::Mutex::new(workflow));
    let (read_tx, read_rx) = mpsc::channel();
    let (attempt_tx, attempt_rx) = mpsc::channel();
    let status_state = Arc::clone(&state);
    let status_root = root.clone();
    let status_identity = Arc::clone(&identity);
    let status_owner_scope = owner_scope.clone();
    let status = std::thread::spawn(move || {
        let _transaction = install_transaction();
        let found = crate::skill_loop::codex_install_status_for_submission(
            source,
            &status_root,
            status_identity.as_ref(),
            &status_owner_scope,
        )
        .expect("empty status");
        assert!(found.is_none());
        read_tx.send(()).expect("status read signal");
        attempt_rx.recv().expect("commit attempt signal");
        status_state
            .lock()
            .expect("workflow lock")
            .clear_missing_install(source);
    });
    read_rx.recv().expect("status read");
    let commit_state = Arc::clone(&state);
    let commit_identity = Arc::clone(&identity);
    let commit_owner_scope = owner_scope.clone();
    let commit = std::thread::spawn(move || {
        attempt_tx.send(()).expect("commit attempt");
        let _transaction = install_transaction();
        let installed = crate::skill_loop::commit_codex_install(
            &plan,
            &review,
            commit_identity.as_ref(),
            &commit_owner_scope,
            &plan.skill_sha256,
            &plan.marker_file_sha256,
        )
        .expect("commit install");
        commit_state
            .lock()
            .expect("workflow lock")
            .insert_install(installed);
    });
    status.join().expect("status thread");
    commit.join().expect("commit thread");

    assert!(
        crate::skill_loop::codex_install_status_for_submission(
            source,
            &root,
            identity.as_ref(),
            &owner_scope,
        )
        .expect("final status")
        .is_some()
    );
    let state = state.lock().expect("workflow lock");
    assert!(state.install_receipt(source).is_some());
    assert!(state.workflows.iter().any(|workflow| {
        workflow.candidate.source_submission_id == source
            && matches!(workflow.phase, SkillWorkflowPhase::Evaluated(_))
    }));
}

#[test]
fn status_present_then_rollback_keeps_cache_equal_to_disk() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let (_identity_directory, identity_store) = crate::config::tests_support::temp_store();
    let identity = Arc::new(
        crate::identity::DeviceIdentity::load_or_generate(&identity_store)
            .expect("test device identity"),
    );
    let owner_scope = trace_commons_protocol::onboarding::user_subject_hash("install-state-owner");
    let source = Uuid::new_v4();
    let mut workflow = SkillLoopState::default();
    let (_, review, report) = advance_to_evaluated(&mut workflow, source);
    let plan = crate::skill_loop::plan_codex_install(
        &review,
        report.evaluation_id,
        &root,
        identity.as_ref(),
        &owner_scope,
    )
    .expect("install plan");
    hold_plan(&mut workflow, plan.clone(), Instant::now());
    workflow.take_plan(plan.plan_id).expect("take plan");
    let installed = crate::skill_loop::commit_codex_install(
        &plan,
        &review,
        identity.as_ref(),
        &owner_scope,
        &plan.skill_sha256,
        &plan.marker_file_sha256,
    )
    .expect("install");
    workflow.insert_install(installed.clone());
    let state = Arc::new(std::sync::Mutex::new(workflow));
    let (read_tx, read_rx) = mpsc::channel();
    let (attempt_tx, attempt_rx) = mpsc::channel();
    let status_state = Arc::clone(&state);
    let status_root = root.clone();
    let status_identity = Arc::clone(&identity);
    let status_owner_scope = owner_scope.clone();
    let status = std::thread::spawn(move || {
        let _transaction = install_transaction();
        let found = crate::skill_loop::codex_install_status_for_submission(
            source,
            &status_root,
            status_identity.as_ref(),
            &status_owner_scope,
        )
        .expect("installed status")
        .expect("installed skill");
        read_tx.send(()).expect("status read signal");
        attempt_rx.recv().expect("rollback attempt signal");
        status_state
            .lock()
            .expect("workflow lock")
            .insert_install(found);
    });
    read_rx.recv().expect("status read");
    let rollback_state = Arc::clone(&state);
    let rollback_identity = Arc::clone(&identity);
    let rollback_owner_scope = owner_scope.clone();
    let rollback = std::thread::spawn(move || {
        attempt_tx.send(()).expect("rollback attempt");
        let _transaction = install_transaction();
        crate::skill_loop::rollback_codex_install(
            &installed,
            rollback_identity.as_ref(),
            &rollback_owner_scope,
        )
        .expect("rollback install");
        rollback_state
            .lock()
            .expect("workflow lock")
            .restore_after_rollback(source);
    });
    status.join().expect("status thread");
    rollback.join().expect("rollback thread");

    assert_eq!(
        crate::skill_loop::codex_install_status_for_submission(
            source,
            &root,
            identity.as_ref(),
            &owner_scope,
        )
        .expect("final status"),
        None
    );
    assert!(matches!(
        state
            .lock()
            .expect("workflow lock")
            .begin_install_plan(report.evaluation_id),
        InstallPlanStart::Ready(_)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn occupied_install_transaction_does_not_starve_async_runtime_progress() {
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let _transaction = install_transaction();
        held_tx.send(()).expect("held transaction signal");
        let _ = release_rx.recv();
    });
    held_rx.recv().expect("install transaction held");

    let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
    let dispatch = tokio::spawn(async move {
        entered_tx.send(()).expect("dispatch entry signal");
        crate::daemon::run_blocking(|| {
            let _transaction = install_transaction();
        });
    });
    entered_rx.await.expect("dispatcher started");

    let progress = tokio::spawn(async {
        tokio::task::yield_now().await;
    });
    let progressed = tokio::time::timeout(Duration::from_millis(250), progress)
        .await
        .is_ok();

    drop(release_tx);
    holder.join().expect("transaction holder");
    tokio::time::timeout(Duration::from_secs(1), dispatch)
        .await
        .expect("install dispatch completion")
        .expect("install dispatch task");

    assert!(
        progressed,
        "waiting for the install transaction parked the runtime worker"
    );
}
