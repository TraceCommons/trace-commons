use crate::mission_attempt::*;
use crate::skill_loop::{EvaluationUsage, ProposedTaskPlan};
use std::thread;
use std::time::{Duration, Instant};

fn scope() -> MissionAttemptScope {
    account('a')
}

fn account(hex: char) -> MissionAttemptScope {
    MissionAttemptScope::Account {
        owner_sha256: hex.to_string().repeat(64),
    }
}

fn start() -> MissionAttemptStart {
    let mut expected_trials = Vec::new();
    for task in 0..8 {
        for arm in [
            EvaluationArm::Baseline,
            EvaluationArm::ManualInstruction,
            EvaluationArm::CandidateSkill,
        ] {
            expected_trials.push(MissionTrialKey {
                task_id: format!("fixture-{task}"),
                arm,
            });
        }
    }
    MissionAttemptStart {
        request_id: Uuid::new_v4(),
        approval_id: Uuid::new_v4(),
        mission_id: Uuid::new_v4(),
        program_id: Uuid::new_v4(),
        package_sha256: "b".repeat(64),
        offer_version_hash: format!("sha256:{}", "c".repeat(64)),
        skill_sha256: "d".repeat(64),
        evaluation_contract_hash: "e".repeat(64),
        requested_model: "nearai/model".into(),
        expected_trials,
    }
}

fn trial(key: &MissionTrialKey, passed: bool) -> SkillTrialResult {
    SkillTrialResult {
        task_id: key.task_id.clone(),
        cluster: "cluster".into(),
        task: "public task".into(),
        source_url: "https://example.invalid/public".into(),
        arm: key.arm,
        passed,
        failure_reasons: if passed {
            Vec::new()
        } else {
            vec!["negative-result".into()]
        },
        answer: Some(ProposedTaskPlan {
            diagnosis: "diagnosis".into(),
            edit_paths: vec!["source".into()],
            commands: vec!["generate".into()],
            verification: vec!["verify".into()],
        }),
        raw_output: "{}".into(),
        request_id: Some("provider-request".into()),
        served_model: "nearai/model".into(),
        finish_reason: "stop".into(),
        usage: EvaluationUsage {
            prompt_tokens: Some(1),
            completion_tokens: Some(1),
            reasoning_tokens: None,
            total_tokens: Some(2),
        },
    }
}

fn large_trial(key: &MissionTrialKey) -> SkillTrialResult {
    let mut result = trial(key, false);
    result.raw_output = "x".repeat(30_000);
    assert!(serde_json::to_vec(&result).unwrap().len() <= MAX_TRIAL_BYTES);
    result
}

fn new_store() -> (tempfile::TempDir, MissionAttemptStore) {
    let root = tempfile::tempdir().unwrap();
    let store = MissionAttemptStore::at(&root.path().join(MISSION_ATTEMPT_STORE_DIR));
    (root, store)
}

fn record_all(
    store: &MissionAttemptStore,
    scope: &MissionAttemptScope,
    attempt: &MissionAttempt,
    passed: bool,
) {
    for key in &attempt.start.expected_trials {
        store
            .record_trial(scope, attempt.attempt_id, trial(key, passed))
            .unwrap();
    }
}

#[test]
fn round_trip_reopen_and_absent_reads_do_not_create_state() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join(MISSION_ATTEMPT_STORE_DIR);
    let store = MissionAttemptStore::at(&dir);
    assert!(store.list(&scope()).unwrap().is_empty());
    assert!(!dir.exists());
    let begun = store.begin(&scope(), start()).unwrap();
    let reopened = MissionAttemptStore::at(&dir);
    assert_eq!(
        reopened.get(&scope(), begun.attempt.attempt_id).unwrap(),
        begun.attempt
    );
    assert_eq!(reopened.list(&scope()).unwrap().len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(dir.join(JOURNAL_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn offer_version_hash_requires_r1_prefixed_shape_while_other_digests_are_bare() {
    let (_root, store) = new_store();
    assert!(store.begin(&scope(), start()).is_ok());

    let mut bare_offer = start();
    bare_offer.offer_version_hash = "c".repeat(64);
    assert_eq!(
        store.begin(&scope(), bare_offer).unwrap_err(),
        MissionAttemptError::FieldInvalid
    );
    assert_eq!(
        MissionAttemptError::FieldInvalid.label(),
        "mission-attempt-field-invalid"
    );

    let mut prefixed_package = start();
    prefixed_package.package_sha256 = format!("sha256:{}", "b".repeat(64));
    assert_eq!(
        store
            .begin(&scope(), prefixed_package)
            .unwrap_err()
            .to_string(),
        "mission-attempt-field-invalid"
    );
}

#[test]
fn expected_keys_require_eight_unique_fixtures_with_all_three_arms() {
    let mut duplicate = start();
    duplicate.expected_trials[1] = duplicate.expected_trials[0].clone();
    assert_eq!(
        validate_start(&duplicate).unwrap_err().to_string(),
        "mission-attempt-field-invalid"
    );

    let mut missing_arm = start();
    missing_arm.expected_trials[2].arm = EvaluationArm::Baseline;
    assert_eq!(
        validate_start(&missing_arm).unwrap_err().to_string(),
        "mission-attempt-field-invalid"
    );

    let mut ninth_fixture = start();
    ninth_fixture.expected_trials[2].task_id = "fixture-extra".into();
    assert_eq!(
        validate_start(&ninth_fixture).unwrap_err().to_string(),
        "mission-attempt-field-invalid"
    );
}

#[test]
fn requested_model_must_be_bounded_trimmed_and_control_free() {
    let (_root, store) = new_store();
    for model in [
        String::new(),
        "x".repeat(MAX_MODEL_CHARS + 1),
        " nearai/model".to_string(),
        "nearai/\0model".to_string(),
    ] {
        let mut candidate = start();
        candidate.requested_model = model;
        assert_eq!(
            store.begin(&scope(), candidate).unwrap_err().to_string(),
            "mission-attempt-field-invalid"
        );
    }
    let mut boundary = start();
    boundary.requested_model = "x".repeat(MAX_MODEL_CHARS);
    assert!(store.begin(&scope(), boundary).is_ok());
}

#[test]
fn begin_is_idempotent_but_changed_binding_conflicts_and_scopes_isolate() {
    let (_root, store) = new_store();
    let start = start();
    let first = store.begin(&scope(), start.clone()).unwrap();
    let replay = store.begin(&scope(), start.clone()).unwrap();
    assert!(!replay.inserted);
    assert_eq!(first.attempt, replay.attempt);
    let mut changed = start.clone();
    changed.skill_sha256 = "f".repeat(64);
    assert_eq!(
        store.begin(&scope(), changed).unwrap_err().to_string(),
        "mission-attempt-conflict"
    );
    let practice = MissionAttemptScope::Practice;
    assert!(store.begin(&practice, start).unwrap().inserted);
    assert_eq!(
        store
            .get(&practice, first.attempt.attempt_id)
            .unwrap_err()
            .to_string(),
        "mission-attempt-not-found"
    );
    assert_eq!(store.list(&scope()).unwrap().len(), 1);
    assert_eq!(store.list(&practice).unwrap().len(), 1);
}

#[test]
fn account_scopes_are_mutually_invisible_even_for_the_same_request() {
    let (_root, store) = new_store();
    let start = start();
    let account_a = account('a');
    let account_b = account('b');
    let a = store.begin(&account_a, start.clone()).unwrap().attempt;
    let b = store.begin(&account_b, start).unwrap().attempt;

    assert_ne!(a.attempt_id, b.attempt_id);
    assert_eq!(
        store.get(&account_b, a.attempt_id).unwrap_err().to_string(),
        "mission-attempt-not-found"
    );
    assert_eq!(
        store.get(&account_a, b.attempt_id).unwrap_err().to_string(),
        "mission-attempt-not-found"
    );
    assert_eq!(store.list(&account_a).unwrap().len(), 1);
    assert_eq!(store.list(&account_b).unwrap().len(), 1);
}

#[test]
fn duplicate_trial_is_idempotent_conflict_refuses_and_partial_failure_survives() {
    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    let negative = trial(&begun.start.expected_trials[0], false);
    let once = store
        .record_trial(&scope(), begun.attempt_id, negative.clone())
        .unwrap();
    let twice = store
        .record_trial(&scope(), begun.attempt_id, negative.clone())
        .unwrap();
    assert_eq!(once, twice);
    assert_eq!(twice.trial_count, 1);

    let mut conflicting = negative;
    conflicting.raw_output = r#"{"different":true}"#.into();
    assert_eq!(
        store
            .record_trial(&scope(), begun.attempt_id, conflicting)
            .unwrap_err()
            .to_string(),
        "mission-attempt-conflict"
    );
    let replay = store.begin(&scope(), begun.start.clone()).unwrap();
    assert!(!replay.inserted);
    assert_eq!(replay.attempt.trial_count, 1);
    assert!(!replay.attempt.trials[0].passed);

    drop(store);
    let reopened = MissionAttemptStore::at(&root.path().join(MISSION_ATTEMPT_STORE_DIR));
    let partial = reopened.get(&scope(), begun.attempt_id).unwrap();
    assert_eq!(partial.trial_count, 1);
    assert!(!partial.trials[0].passed);
}

#[test]
fn completion_requires_all_trials_and_accepts_a_fully_negative_outcome() {
    let (_root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    store
        .record_trial(
            &scope(),
            begun.attempt_id,
            trial(&begun.start.expected_trials[0], false),
        )
        .unwrap();
    assert_eq!(
        store
            .finish(
                &scope(),
                begun.attempt_id,
                MissionAttemptStatus::Completed,
                COMPLETED_REASON,
            )
            .unwrap_err()
            .to_string(),
        "mission-attempt-invalid-state"
    );

    for key in begun.start.expected_trials.iter().skip(1) {
        store
            .record_trial(&scope(), begun.attempt_id, trial(key, false))
            .unwrap();
    }
    let completed = store
        .finish(
            &scope(),
            begun.attempt_id,
            MissionAttemptStatus::Completed,
            COMPLETED_REASON,
        )
        .unwrap();
    assert_eq!(completed.trial_count, EXPECTED_TRIALS);
    assert!(completed.trials.iter().all(|trial| !trial.passed));
}

#[test]
fn cancellation_requires_request_ack_drains_inflight_and_is_idempotent() {
    let (_root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    assert_eq!(
        store
            .finish(
                &scope(),
                begun.attempt_id,
                MissionAttemptStatus::Cancelled,
                CANCELLED_REASON,
            )
            .unwrap_err()
            .to_string(),
        "mission-attempt-invalid-state"
    );

    let requested = store.request_cancel(&scope(), begun.attempt_id).unwrap();
    assert_eq!(requested.status, MissionAttemptStatus::CancelRequested);
    assert_eq!(
        store.request_cancel(&scope(), begun.attempt_id).unwrap(),
        requested
    );
    let drained = store
        .record_trial(
            &scope(),
            begun.attempt_id,
            trial(&begun.start.expected_trials[0], true),
        )
        .unwrap();
    assert_eq!(drained.status, MissionAttemptStatus::CancelRequested);

    let cancelled = store
        .finish(
            &scope(),
            begun.attempt_id,
            MissionAttemptStatus::Cancelled,
            CANCELLED_REASON,
        )
        .unwrap();
    assert_eq!(cancelled.trial_count, 1);
    assert_eq!(
        store
            .finish(
                &scope(),
                begun.attempt_id,
                MissionAttemptStatus::Cancelled,
                CANCELLED_REASON,
            )
            .unwrap(),
        cancelled
    );
    assert_eq!(
        store.request_cancel(&scope(), begun.attempt_id).unwrap(),
        cancelled
    );
}

#[test]
fn finish_refuses_recovery_only_statuses_and_mismatched_terminal_reasons() {
    let (_root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    for (status, reason) in [
        (MissionAttemptStatus::Interrupted, INTERRUPTED_REASON),
        (MissionAttemptStatus::Running, COMPLETED_REASON),
        (MissionAttemptStatus::CancelRequested, CANCELLED_REASON),
        (MissionAttemptStatus::Completed, CANCELLED_REASON),
        (MissionAttemptStatus::Cancelled, COMPLETED_REASON),
    ] {
        assert_eq!(
            store
                .finish(&scope(), begun.attempt_id, status, reason)
                .unwrap_err()
                .to_string(),
            "mission-attempt-field-invalid"
        );
    }
}

#[test]
fn terminal_attempts_are_immutable_and_active_content_cannot_be_revoked() {
    let (_root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    assert_eq!(
        store
            .revoke_content(&scope(), begun.attempt_id)
            .unwrap_err()
            .to_string(),
        "mission-attempt-invalid-state"
    );
    let failed = store
        .finish(
            &scope(),
            begun.attempt_id,
            MissionAttemptStatus::Failed,
            "mission-attempt-provider-failed",
        )
        .unwrap();
    assert_eq!(
        store
            .record_trial(
                &scope(),
                begun.attempt_id,
                trial(&begun.start.expected_trials[0], true),
            )
            .unwrap_err()
            .to_string(),
        "mission-attempt-invalid-state"
    );
    assert_eq!(
        store
            .finish(
                &scope(),
                begun.attempt_id,
                MissionAttemptStatus::Cancelled,
                CANCELLED_REASON,
            )
            .unwrap_err()
            .to_string(),
        "mission-attempt-conflict"
    );
    assert_eq!(
        store.request_cancel(&scope(), begun.attempt_id).unwrap(),
        failed
    );
}

#[test]
fn revoked_completed_attempt_retains_counts_bindings_and_terminal_receipt() {
    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    record_all(&store, &scope(), &begun, false);
    let completed = store
        .finish(
            &scope(),
            begun.attempt_id,
            MissionAttemptStatus::Completed,
            COMPLETED_REASON,
        )
        .unwrap();
    let revoked = store.revoke_content(&scope(), begun.attempt_id).unwrap();
    assert!(revoked.content_revoked);
    assert!(revoked.trials.is_empty());
    assert_eq!(revoked.trial_count, EXPECTED_TRIALS);
    assert_eq!(revoked.start, completed.start);
    assert_eq!(revoked.status, MissionAttemptStatus::Completed);
    assert_eq!(revoked.terminal_reason.as_deref(), Some(COMPLETED_REASON));

    drop(store);
    let reopened = MissionAttemptStore::at(&root.path().join(MISSION_ATTEMPT_STORE_DIR));
    assert_eq!(reopened.get(&scope(), begun.attempt_id).unwrap(), revoked);
}

#[test]
fn restart_recovery_and_revocation_preserve_the_lifecycle_receipt() {
    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    store
        .record_trial(
            &scope(),
            begun.attempt_id,
            trial(&begun.start.expected_trials[0], false),
        )
        .unwrap();
    drop(store);
    let reopened = MissionAttemptStore::at(&root.path().join(MISSION_ATTEMPT_STORE_DIR));
    assert_eq!(reopened.recover_interrupted().unwrap(), 1);
    let recovered = reopened.get(&scope(), begun.attempt_id).unwrap();
    assert_eq!(recovered.status, MissionAttemptStatus::Interrupted);
    assert_eq!(recovered.trial_count, 1);
    assert!(!recovered.trials[0].passed);
    let revoked = reopened.revoke_content(&scope(), begun.attempt_id).unwrap();
    assert!(revoked.content_revoked);
    assert!(revoked.trials.is_empty());
    assert_eq!(revoked.trial_count, 1);
    assert_eq!(
        reopened.revoke_content(&scope(), begun.attempt_id).unwrap(),
        revoked
    );
    drop(reopened);
    let reopened = MissionAttemptStore::at(&root.path().join(MISSION_ATTEMPT_STORE_DIR));
    assert_eq!(reopened.get(&scope(), begun.attempt_id).unwrap(), revoked);
}

#[test]
fn recovery_also_terminates_cancel_requested_attempts() {
    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    store.request_cancel(&scope(), begun.attempt_id).unwrap();
    drop(store);

    let reopened = MissionAttemptStore::at(&root.path().join(MISSION_ATTEMPT_STORE_DIR));
    assert_eq!(reopened.recover_interrupted().unwrap(), 1);
    let recovered = reopened.get(&scope(), begun.attempt_id).unwrap();
    assert_eq!(recovered.status, MissionAttemptStatus::Interrupted);
    assert_eq!(
        recovered.terminal_reason.as_deref(),
        Some(INTERRUPTED_REASON)
    );
    assert!(
        reopened
            .revoke_content(&scope(), begun.attempt_id)
            .unwrap()
            .content_revoked
    );
}

#[test]
fn malformed_oversized_and_lock_contention_are_fixed_failures() {
    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    let dir = root.path().join(MISSION_ATTEMPT_STORE_DIR);
    fs::write(dir.join(JOURNAL_FILE), b"not-json").unwrap();
    assert_eq!(
        store.list(&scope()).unwrap_err().to_string(),
        "mission-attempt-store-invalid"
    );
    fs::write(
        dir.join(JOURNAL_FILE),
        vec![b' '; MAX_JOURNAL_BYTES as usize + 1],
    )
    .unwrap();
    assert_eq!(
        store.list(&scope()).unwrap_err().to_string(),
        "mission-attempt-store-invalid"
    );
    fs::remove_file(dir.join(JOURNAL_FILE)).unwrap();
    let fresh = MissionAttemptStore::at(&dir);
    fresh.begin(&scope(), start()).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(dir.join(LOCK_FILE))
        .unwrap();
    lock.try_lock().unwrap();
    assert_eq!(
        fresh.list(&scope()).unwrap_err().to_string(),
        "mission-attempt-store-busy"
    );
    lock.unlock().unwrap();
    assert_eq!(
        fresh
            .get(&scope(), begun.attempt_id)
            .unwrap_err()
            .to_string(),
        "mission-attempt-not-found"
    );
}

#[test]
fn oversized_trial_is_rejected_at_ingress_and_again_when_loaded_from_untrusted_bytes() {
    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    let mut oversized = trial(&begun.start.expected_trials[0], false);
    oversized.raw_output = "x".repeat(MAX_TRIAL_BYTES);
    assert!(serde_json::to_vec(&oversized).unwrap().len() > MAX_TRIAL_BYTES);
    let path = root
        .path()
        .join(MISSION_ATTEMPT_STORE_DIR)
        .join(JOURNAL_FILE);
    let before = fs::read(&path).unwrap();
    assert_eq!(
        store
            .record_trial(&scope(), begun.attempt_id, oversized.clone())
            .unwrap_err(),
        MissionAttemptError::TrialInvalid
    );
    assert_eq!(fs::read(&path).unwrap(), before);

    let (lock, mut journal) = store.locked_existing().unwrap();
    journal.attempts[0].attempt.trials.push(oversized);
    journal.attempts[0].attempt.trial_count = 1;
    fs::write(&path, serde_json::to_vec(&journal).unwrap()).unwrap();
    drop(lock);
    assert_eq!(
        store.list(&scope()).unwrap_err(),
        MissionAttemptError::StoreInvalid
    );
}

#[test]
fn record_trial_refuses_foreign_models_and_unexpected_trial_keys() {
    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    let path = root
        .path()
        .join(MISSION_ATTEMPT_STORE_DIR)
        .join(JOURNAL_FILE);

    let before_foreign_model = fs::read(&path).unwrap();
    let mut foreign_model = trial(&begun.start.expected_trials[0], false);
    foreign_model.served_model = "nearai/other-model".into();
    assert_eq!(
        store
            .record_trial(&scope(), begun.attempt_id, foreign_model)
            .unwrap_err(),
        MissionAttemptError::TrialInvalid
    );
    assert_eq!(fs::read(&path).unwrap(), before_foreign_model);

    let before_unexpected_key = fs::read(&path).unwrap();
    let mut unexpected_key = trial(&begun.start.expected_trials[0], false);
    unexpected_key.task_id = "fixture-absent".into();
    assert_eq!(
        store
            .record_trial(&scope(), begun.attempt_id, unexpected_key)
            .unwrap_err(),
        MissionAttemptError::TrialInvalid
    );
    assert_eq!(fs::read(&path).unwrap(), before_unexpected_key);
}

#[test]
fn journal_version_mismatch_is_refused_without_mutating_the_store() {
    let (root, store) = new_store();
    store.begin(&scope(), start()).unwrap();
    let path = root
        .path()
        .join(MISSION_ATTEMPT_STORE_DIR)
        .join(JOURNAL_FILE);

    let (lock, mut journal) = store.locked_existing().unwrap();
    journal.version = u32::MAX;
    let future_version_bytes = serde_json::to_vec(&journal).unwrap();
    drop(lock);
    fs::write(&path, &future_version_bytes).unwrap();

    assert_eq!(
        store.list(&scope()).unwrap_err(),
        MissionAttemptError::StoreVersionUnsupported
    );
    assert_eq!(fs::read(&path).unwrap(), future_version_bytes);
}

#[cfg(unix)]
#[test]
fn static_directory_journal_and_lock_symlinks_are_refused_without_following() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let root = tempfile::tempdir().unwrap();
    let target_dir = root.path().join("directory-target");
    fs::create_dir(&target_dir).unwrap();
    let linked_dir = root.path().join("linked-store");
    symlink(&target_dir, &linked_dir).unwrap();
    let directory_store = MissionAttemptStore::at(&linked_dir);
    assert_eq!(
        directory_store
            .begin(&scope(), start())
            .unwrap_err()
            .to_string(),
        "mission-attempt-store-unavailable"
    );
    assert!(fs::read_dir(&target_dir).unwrap().next().is_none());

    let journal_dir = root.path().join("journal-store");
    fs::create_dir(&journal_dir).unwrap();
    fs::set_permissions(&journal_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let journal_target = root.path().join("journal-target");
    fs::write(&journal_target, b"unchanged-journal").unwrap();
    symlink(&journal_target, journal_dir.join(JOURNAL_FILE)).unwrap();
    let journal_store = MissionAttemptStore::at(&journal_dir);
    assert_eq!(
        journal_store.list(&scope()).unwrap_err().to_string(),
        "mission-attempt-store-invalid"
    );
    assert_eq!(fs::read(&journal_target).unwrap(), b"unchanged-journal");

    let lock_dir = root.path().join("lock-store");
    fs::create_dir(&lock_dir).unwrap();
    fs::set_permissions(&lock_dir, fs::Permissions::from_mode(0o700)).unwrap();
    let lock_target = root.path().join("lock-target");
    fs::write(&lock_target, b"unchanged-lock").unwrap();
    symlink(&lock_target, lock_dir.join(LOCK_FILE)).unwrap();
    let lock_store = MissionAttemptStore::at(&lock_dir);
    assert_eq!(
        lock_store.begin(&scope(), start()).unwrap_err().to_string(),
        "mission-attempt-store-unavailable"
    );
    assert_eq!(fs::read(&lock_target).unwrap(), b"unchanged-lock");
}

#[test]
fn concurrent_record_writes_with_bounded_retry_lose_no_trials() {
    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    drop(store);

    thread::scope(|thread_scope| {
        let mut writers = Vec::new();
        for key in begun.start.expected_trials.iter().take(12) {
            let dir = root.path().join(MISSION_ATTEMPT_STORE_DIR);
            let attempt_id = begun.attempt_id;
            writers.push(thread_scope.spawn(move || {
                let store = MissionAttemptStore::at(&dir);
                for _ in 0..250 {
                    match store.record_trial(&scope(), attempt_id, trial(key, true)) {
                        Ok(_) => return,
                        Err(error) if error.to_string() == "mission-attempt-store-busy" => {
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(error) => panic!("{error}"),
                    }
                }
                panic!("bounded mission-attempt lock retry exhausted");
            }));
        }
        for writer in writers {
            writer.join().unwrap();
        }
    });

    let reopened = MissionAttemptStore::at(&root.path().join(MISSION_ATTEMPT_STORE_DIR));
    let recorded = reopened.get(&scope(), begun.attempt_id).unwrap();
    assert_eq!(recorded.trial_count, 12);
    assert_eq!(
        recorded
            .trials
            .iter()
            .map(|trial| (&trial.task_id, trial.arm))
            .collect::<BTreeSet<_>>()
            .len(),
        12
    );
}

#[test]
fn attempt_cap_refusal_preserves_previous_bytes() {
    let (root, store) = new_store();
    for _ in 0..MAX_ATTEMPTS {
        store.begin(&scope(), start()).unwrap();
    }
    let path = root
        .path()
        .join(MISSION_ATTEMPT_STORE_DIR)
        .join(JOURNAL_FILE);
    let before = fs::read(&path).unwrap();
    assert_eq!(
        store.begin(&scope(), start()).unwrap_err().to_string(),
        "mission-attempt-store-full"
    );
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn content_cap_refusal_preserves_trials_and_reserved_headroom_persists_failure() {
    let (root, store) = new_store();
    let target = store.begin(&scope(), start()).unwrap().attempt;
    let (lock, mut journal) = store.locked_existing().unwrap();

    for _ in 1..MAX_ATTEMPTS {
        let start = start();
        journal.attempts.push(ScopedAttempt {
            scope: scope(),
            attempt: MissionAttempt {
                attempt_id: Uuid::new_v4(),
                start,
                status: MissionAttemptStatus::Running,
                created_at_unix_seconds: target.created_at_unix_seconds,
                updated_at_unix_seconds: target.updated_at_unix_seconds,
                trials: Vec::new(),
                trial_count: 0,
                terminal_reason: None,
                content_revoked: false,
            },
        });
    }

    let first_key = journal.attempts[0].attempt.start.expected_trials[0].clone();
    journal.attempts[0]
        .attempt
        .trials
        .push(large_trial(&first_key));
    journal.attempts[0].attempt.trial_count = 1;

    let slots = (1..MAX_ATTEMPTS)
        .flat_map(|attempt_index| {
            (0..EXPECTED_TRIALS).map(move |key_index| (attempt_index, key_index))
        })
        .collect::<Vec<_>>();
    let base_bytes = serde_json::to_vec(&journal).unwrap().len() as u64;
    let sample_bytes = serde_json::to_vec(&large_trial(
        &journal.attempts[1].attempt.start.expected_trials[0],
    ))
    .unwrap()
    .len() as u64;
    let estimate = ((MAX_CONTENT_JOURNAL_BYTES - base_bytes) / (sample_bytes + 1)) as usize;
    let mut used = 0;
    for &(attempt_index, key_index) in slots.iter().take(estimate) {
        let key = journal.attempts[attempt_index]
            .attempt
            .start
            .expected_trials[key_index]
            .clone();
        journal.attempts[attempt_index]
            .attempt
            .trials
            .push(large_trial(&key));
        journal.attempts[attempt_index].attempt.trial_count += 1;
        used += 1;
    }
    while serde_json::to_vec(&journal).unwrap().len() as u64 > MAX_CONTENT_JOURNAL_BYTES {
        used -= 1;
        let attempt_index = slots[used].0;
        journal.attempts[attempt_index]
            .attempt
            .trials
            .pop()
            .unwrap();
        journal.attempts[attempt_index].attempt.trial_count -= 1;
    }

    let candidate_key = journal.attempts[0].attempt.start.expected_trials[1].clone();
    loop {
        journal.attempts[0]
            .attempt
            .trials
            .push(large_trial(&candidate_key));
        journal.attempts[0].attempt.trial_count += 1;
        let candidate_crosses =
            serde_json::to_vec(&journal).unwrap().len() as u64 > MAX_CONTENT_JOURNAL_BYTES;
        journal.attempts[0].attempt.trials.pop().unwrap();
        journal.attempts[0].attempt.trial_count -= 1;
        if candidate_crosses {
            break;
        }
        let (attempt_index, key_index) = slots[used];
        let key = journal.attempts[attempt_index]
            .attempt
            .start
            .expected_trials[key_index]
            .clone();
        journal.attempts[attempt_index]
            .attempt
            .trials
            .push(large_trial(&key));
        journal.attempts[attempt_index].attempt.trial_count += 1;
        used += 1;
    }

    store.save_content(&lock, &journal).unwrap();
    drop(lock);
    let path = root
        .path()
        .join(MISSION_ATTEMPT_STORE_DIR)
        .join(JOURNAL_FILE);
    let before_refusal = fs::read(&path).unwrap();
    let refusal_started = Instant::now();
    assert_eq!(
        store
            .record_trial(&scope(), target.attempt_id, large_trial(&candidate_key),)
            .unwrap_err()
            .to_string(),
        "mission-attempt-store-full"
    );
    let refusal_elapsed = refusal_started.elapsed();
    assert_eq!(fs::read(&path).unwrap(), before_refusal);

    let reason = format!("mission-attempt-{}", "f".repeat(80));
    let receipt_started = Instant::now();
    let failed = store
        .finish(
            &scope(),
            target.attempt_id,
            MissionAttemptStatus::Failed,
            &reason,
        )
        .unwrap();
    let receipt_elapsed = receipt_started.elapsed();
    eprintln!(
        "near-capacity journal: content_refusal={refusal_elapsed:?} receipt_write={receipt_elapsed:?}"
    );
    assert_eq!(failed.status, MissionAttemptStatus::Failed);
    assert_eq!(failed.trial_count, 1);
    assert_eq!(failed.trials[0].raw_output.len(), 30_000);

    let reopened = MissionAttemptStore::at(&root.path().join(MISSION_ATTEMPT_STORE_DIR));
    assert_eq!(reopened.get(&scope(), target.attempt_id).unwrap(), failed);
}

#[test]
fn a_full_journal_evicts_the_oldest_terminal_attempt_and_keeps_running_ones() {
    let (_root, store) = new_store();
    let mut ids = Vec::new();
    for _ in 0..MAX_ATTEMPTS {
        ids.push(store.begin(&scope(), start()).unwrap().attempt.attempt_id);
    }
    // Only the two oldest reach a terminal state; the rest are still paying
    // for inference and must survive.
    for id in ids.iter().take(2) {
        store
            .finish(
                &scope(),
                *id,
                MissionAttemptStatus::Failed,
                "provider-refused",
            )
            .unwrap();
    }

    let admitted = store.begin(&scope(), start()).unwrap();
    assert!(admitted.inserted, "the 129th attempt is admitted");
    assert_eq!(
        store.get(&scope(), ids[0]).unwrap_err().to_string(),
        "mission-attempt-not-found",
        "the oldest terminal attempt is the one evicted"
    );
    store
        .get(&scope(), ids[1])
        .expect("only the single slot that was needed is freed");
    store
        .get(&scope(), ids[2])
        .expect("an in-progress attempt is never evicted");
    assert_eq!(store.list(&scope()).unwrap().len(), MAX_ATTEMPTS);
}

#[test]
fn listing_attempts_during_a_run_never_refuses_the_recording_writer() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    let (root, store) = new_store();
    let begun = store.begin(&scope(), start()).unwrap().attempt;
    drop(store);
    let dir = root.path().join(MISSION_ATTEMPT_STORE_DIR);
    let stop = Arc::new(AtomicBool::new(false));
    let listings = Arc::new(AtomicUsize::new(0));

    thread::scope(|thread_scope| {
        let reader_dir = dir.clone();
        let reader_stop = Arc::clone(&stop);
        let listed = Arc::clone(&listings);
        thread_scope.spawn(move || {
            let reader = MissionAttemptStore::at(&reader_dir);
            while !reader_stop.load(Ordering::Relaxed) {
                // A UI polls; it does not spin. The reader's own admission is
                // allowed to lose to a saturated writer, so the count below is
                // what is asserted rather than every individual call. What must
                // never happen is the reverse, which is the writer's `expect`.
                if reader.list(&scope()).is_ok() {
                    listed.fetch_add(1, Ordering::Relaxed);
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        let writer = MissionAttemptStore::at(&dir);
        let mut refusals = Vec::new();
        for key in &begun.start.expected_trials {
            if let Err(error) = writer.record_trial(&scope(), begun.attempt_id, trial(key, true)) {
                refusals.push(error);
            }
        }
        // Stop the reader before asserting. Panicking first would leave it
        // spinning and the scope would never join.
        stop.store(true, Ordering::Relaxed);
        assert!(
            refusals.is_empty(),
            // A refusal here surfaces as PersistenceUnavailable and ends a paid
            // run, so a UI read must never be able to cause one.
            "a concurrent listing refused the recording writer: {refusals:?}"
        );
    });

    let reopened = MissionAttemptStore::at(&dir);
    assert_eq!(
        reopened
            .get(&scope(), begun.attempt_id)
            .unwrap()
            .trial_count,
        EXPECTED_TRIALS
    );
    assert!(
        listings.load(Ordering::Relaxed) > 0,
        "the reader was admitted at least once while the run recorded"
    );
}
