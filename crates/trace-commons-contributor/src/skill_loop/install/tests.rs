//! INTEGRATION: verifies atomic Codex skill installation, source lineage, crash recovery,
//! and rollback isolation against filesystem mutations.

use super::*;
use crate::skill_loop::{SkillDraft, review_candidate};
use std::collections::BTreeSet;
use std::sync::OnceLock;

const TEST_OWNER_SCOPE_SHA256: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

fn test_identity() -> &'static DeviceIdentity {
    static IDENTITY: OnceLock<DeviceIdentity> = OnceLock::new();
    IDENTITY.get_or_init(|| DeviceIdentity::generate_for_test().expect("test identity"))
}

fn plan_codex_install(
    review: &SkillReview,
    evaluation_id: Uuid,
    root: &Path,
) -> Result<CodexInstallPlan, SkillInstallError> {
    super::plan_codex_install(
        review,
        evaluation_id,
        root,
        test_identity(),
        TEST_OWNER_SCOPE_SHA256,
    )
}

fn commit_codex_install(
    plan: &CodexInstallPlan,
    review: &SkillReview,
    as_previewed_sha256: &str,
    as_previewed_marker_sha256: &str,
) -> Result<CodexInstalledSkill, SkillInstallError> {
    super::commit_codex_install(
        plan,
        review,
        test_identity(),
        TEST_OWNER_SCOPE_SHA256,
        as_previewed_sha256,
        as_previewed_marker_sha256,
    )
}

fn rollback_codex_install(
    installed: &CodexInstalledSkill,
) -> Result<SkillRollbackResult, SkillInstallError> {
    super::rollback_codex_install(installed, test_identity(), TEST_OWNER_SCOPE_SHA256)
}

fn codex_install_status_for_submission(
    source_submission_id: Uuid,
    root: &Path,
) -> Result<Option<CodexInstalledSkill>, SkillInstallError> {
    super::codex_install_status_for_submission(
        source_submission_id,
        root,
        test_identity(),
        TEST_OWNER_SCOPE_SHA256,
    )
}

fn review() -> SkillReview {
    review_named("repair-generated-sources", Uuid::new_v4())
}

fn review_named(name: &str, source_submission_id: Uuid) -> SkillReview {
    let candidate = crate::skill_loop::SkillCandidate {
        candidate_id: Uuid::new_v4(),
        family: crate::skill_loop::GENERATED_SOURCE_FAMILY,
        source_submission_id,
        source_correction: "correction".to_string(),
        source_evidence: Vec::new(),
        source_task_fingerprint: crate::skill_loop::source_task_fingerprint(
            "Repair a generated GraphQL client from its source schema.",
        )
        .expect("source fingerprint"),
        draft: SkillDraft {
            name: name.to_string(),
            description: "Use when a generated file must be repaired through its source."
                .to_string(),
            procedure:
                "# Procedure\n\nInspect the generator, edit its source, regenerate, and verify drift."
                    .to_string(),
        },
        replaces_review_id: None,
        manual_control_instruction: crate::skill_loop::MANUAL_CONTROL_INSTRUCTION,
        evaluation_contract: crate::skill_loop::evaluation_contract(),
    };
    review_candidate(&candidate, candidate.draft.clone()).expect("review")
}

#[test]
fn install_requires_the_exact_preview_and_refuses_occupied_targets() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    assert!(plan.can_install);
    assert_eq!(plan.skill, plan.target.join(SKILL_FILE_NAME));
    assert_eq!(plan.marker, plan.target.join(MARKER_NAME));
    assert_eq!(sha256(plan.skill_md.as_bytes()), plan.skill_sha256);
    assert_eq!(sha256(plan.marker_json.as_bytes()), plan.marker_file_sha256);
    let previewed_marker: InstallMarker =
        serde_json::from_str(&plan.marker_json).expect("previewed marker JSON");
    assert!(marker_has_valid_authentication(
        &previewed_marker,
        test_identity(),
        TEST_OWNER_SCOPE_SHA256,
    ));
    assert!(marker_matches_review(
        &previewed_marker,
        &review,
        plan.evaluation_id
    ));
    assert_eq!(
        commit_codex_install(&plan, &review, "wrong", &plan.marker_file_sha256),
        Err(SkillInstallError::PlanChanged)
    );
    assert_eq!(
        commit_codex_install(&plan, &review, &plan.skill_sha256, "wrong"),
        Err(SkillInstallError::PlanChanged)
    );
    let mut changed_marker = plan.clone();
    changed_marker.marker_json.push(' ');
    changed_marker.marker_file_sha256 = sha256(changed_marker.marker_json.as_bytes());
    assert_eq!(
        commit_codex_install(
            &changed_marker,
            &review,
            &changed_marker.skill_sha256,
            &changed_marker.marker_file_sha256,
        ),
        Err(SkillInstallError::PlanChanged)
    );
    assert!(!plan.target.exists());
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("commit reviewed install");
    assert_eq!(installed.source_submission_id, review.source_submission_id);
    assert_eq!(
        fs::read_to_string(&plan.skill).expect("installed skill"),
        plan.skill_md
    );
    assert_eq!(
        fs::read_to_string(&plan.marker).expect("installed ownership marker"),
        plan.marker_json
    );
    let occupied = plan_codex_install(&review, Uuid::new_v4(), &root).expect("occupied plan");
    assert!(occupied.occupied);
    assert!(!occupied.can_install);
    let names = directory_names(&installed.local_target_path).expect("installed names");
    assert_eq!(
        names,
        BTreeSet::from([MARKER_NAME.to_string(), SKILL_FILE_NAME.to_string()])
    );
}

#[test]
fn exclusive_publish_refuses_concurrent_target_without_overwrite() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    fs::create_dir(&root).expect("skills root");
    let root_identity = ownership::directory_identity(&root, SkillInstallError::InvalidRoot)
        .expect("root identity");
    let target = root.join("concurrent-skill");
    let staged = publication::stage_install(
        &root,
        Uuid::new_v4(),
        br#"{"schema_version":2}"#,
        b"# Concurrent skill\n",
    )
    .expect("stage complete package");

    fs::create_dir(&target).expect("concurrent target");
    fs::write(target.join("sentinel"), b"existing owner").expect("sentinel");

    assert_eq!(
        publication::publish_staged_install(staged, &target, root_identity),
        Err(SkillInstallError::Occupied)
    );
    assert_eq!(
        fs::read(target.join("sentinel")).expect("unchanged sentinel"),
        b"existing owner"
    );
    let entries = fs::read_dir(&root)
        .expect("read root")
        .map(|entry| entry.expect("root entry").file_name())
        .collect::<Vec<_>>();
    assert_eq!(entries, [target.file_name().expect("target name")]);
}

#[test]
fn install_serialization_exposes_only_symbolic_locations() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("private-home/.codex/skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let serialized_plan = serde_json::to_value(plan.preview()).expect("serialize plan");
    let serialized_plan_text = serialized_plan.to_string();
    let private_root = directory.path().to_string_lossy();

    assert_eq!(
        serialized_plan["target_location"],
        format!("$CODEX_HOME/skills/{}", review.draft.name)
    );
    assert_eq!(
        serialized_plan["skill_location"],
        format!("$CODEX_HOME/skills/{}/SKILL.md", review.draft.name)
    );
    assert!(serialized_plan.get("target_path").is_none());
    assert!(serialized_plan.get("skill_path").is_none());
    assert!(serialized_plan.get("marker_path").is_none());
    assert!(!serialized_plan_text.contains(private_root.as_ref()));
    assert!(!format!("{plan:?}").contains(private_root.as_ref()));

    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("install");
    let serialized_install = serde_json::to_value(installed.receipt()).expect("serialize install");
    assert_eq!(serialized_install["target_location"], plan.target_location);
    assert!(serialized_install.get("target_path").is_none());
    assert!(
        !serialized_install
            .to_string()
            .contains(private_root.as_ref())
    );
    assert!(!format!("{installed:?}").contains(private_root.as_ref()));
}

#[test]
fn final_sync_failure_returns_the_exact_published_install() {
    fn fail_final_sync(_: &Path, _: &Path) -> Result<(), SkillInstallError> {
        Err(SkillInstallError::WriteFailed)
    }

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed = super::commit_codex_install_with_final_sync(
        &plan,
        &review,
        test_identity(),
        TEST_OWNER_SCOPE_SHA256,
        &plan.skill_sha256,
        &plan.marker_file_sha256,
        fail_final_sync,
    )
    .expect("published install reconciles after sync failure");

    assert_eq!(
        codex_install_status_for_submission(review.source_submission_id, &root)
            .expect("status after sync failure"),
        Some(installed)
    );
}

#[test]
fn rollback_hides_the_install_and_retains_an_unloadable_backup() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("commit reviewed install");
    let result = rollback_codex_install(&installed).expect("rollback");
    assert!(result.removed);
    assert!(result.retained_directory);
    assert!(!installed.local_target_path.exists());
    let quarantined = fs::read_dir(&root)
        .expect("skills root")
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(QUARANTINE_PREFIX)
        })
        .expect("retained quarantine")
        .path();
    assert_eq!(
        directory_names(&quarantined).expect("retained names"),
        BTreeSet::from([
            RETAINED_MARKER_NAME.to_string(),
            RETAINED_SKILL_NAME.to_string(),
        ])
    );
    assert_eq!(
        fs::read_to_string(quarantined.join(RETAINED_SKILL_NAME)).expect("retained skill"),
        review.skill_md
    );
    assert_eq!(
        fs::read_to_string(quarantined.join(RETAINED_MARKER_NAME)).expect("retained marker"),
        plan.marker_json
    );
}

#[test]
fn rollback_refuses_modified_or_extended_installs() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("commit reviewed install");
    fs::write(
        installed.local_target_path.join(SKILL_FILE_NAME),
        "owner edit",
    )
    .expect("modify skill");
    assert_eq!(
        rollback_codex_install(&installed),
        Err(SkillInstallError::SkillChanged)
    );
    assert!(installed.local_target_path.exists());

    fs::write(
        installed.local_target_path.join(SKILL_FILE_NAME),
        &review.skill_md,
    )
    .expect("restore skill");
    fs::write(installed.local_target_path.join("notes.md"), "owner note").expect("add file");
    assert_eq!(
        rollback_codex_install(&installed),
        Err(SkillInstallError::UnexpectedContents)
    );
    assert_eq!(
        fs::read_to_string(installed.local_target_path.join("notes.md"))
            .expect("retained owner note"),
        "owner note"
    );
}

#[test]
fn status_and_rollback_refuse_a_modified_ownership_marker() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("commit reviewed install");
    let target = installed.local_target_path.as_path();
    let marker_path = target.join(MARKER_NAME);
    let mut marker: InstallMarker =
        serde_json::from_slice(&fs::read(&marker_path).expect("installed ownership marker"))
            .expect("marker JSON");
    marker.source_evidence_ids.push(Uuid::new_v4());
    fs::write(
        &marker_path,
        serde_json::to_vec_pretty(&marker).expect("modified marker JSON"),
    )
    .expect("modify marker");

    assert_eq!(
        codex_install_status_for_submission(review.source_submission_id, &root),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert_eq!(
        rollback_codex_install(&installed),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert!(target.exists());
    assert_eq!(
        fs::read(marker_path).expect("modified marker retained"),
        serde_json::to_vec_pretty(&marker).expect("expected marker bytes")
    );
}

#[test]
fn restart_status_rejects_a_tampered_signed_marker_with_a_recomputed_digest() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let identity_store = crate::config::ConfigStore::open(directory.path().join("identity"))
        .expect("identity store");
    let initial_identity =
        DeviceIdentity::load_or_generate(&identity_store).expect("initial identity");
    let review = review();
    let plan = super::plan_codex_install(
        &review,
        Uuid::new_v4(),
        &root,
        &initial_identity,
        TEST_OWNER_SCOPE_SHA256,
    )
    .expect("plan");
    let installed = super::commit_codex_install(
        &plan,
        &review,
        &initial_identity,
        TEST_OWNER_SCOPE_SHA256,
        &plan.skill_sha256,
        &plan.marker_file_sha256,
    )
    .expect("commit reviewed install");
    drop(initial_identity);
    let restarted_identity = DeviceIdentity::load(&identity_store)
        .expect("reload identity")
        .expect("persisted identity");
    let target = installed.local_target_path.as_path();
    let marker_path = target.join(MARKER_NAME);
    let mut marker: InstallMarker =
        serde_json::from_slice(&fs::read(&marker_path).expect("installed ownership marker"))
            .expect("marker JSON");
    marker.source_evidence_ids.push(Uuid::new_v4());
    marker.marker_sha256 = install_marker_sha256(&marker).expect("replacement marker digest");
    fs::write(
        &marker_path,
        serde_json::to_vec_pretty(&marker).expect("replacement marker JSON"),
    )
    .expect("replace marker");

    assert_eq!(
        super::codex_install_status_for_submission(
            review.source_submission_id,
            &root,
            &restarted_identity,
            TEST_OWNER_SCOPE_SHA256,
        ),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert_eq!(
        super::rollback_codex_install(&installed, &restarted_identity, TEST_OWNER_SCOPE_SHA256,),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert!(target.exists());
}

#[test]
fn restart_status_requires_the_original_device_and_owner_scope() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("install");
    let other_identity = DeviceIdentity::generate_for_test().expect("other identity");
    let other_owner = "sha256:1111111111111111111111111111111111111111111111111111111111111111";

    assert_eq!(
        super::codex_install_status_for_submission(
            review.source_submission_id,
            &root,
            &other_identity,
            TEST_OWNER_SCOPE_SHA256,
        ),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert_eq!(
        super::codex_install_status_for_submission(
            review.source_submission_id,
            &root,
            test_identity(),
            other_owner,
        ),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert!(installed.local_target_path.exists());
}

#[test]
fn status_and_rollback_reject_unknown_marker_fields_without_touching_the_install() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("commit reviewed install");
    let target = installed.local_target_path.as_path();
    let marker_path = target.join(MARKER_NAME);
    let mut marker: serde_json::Value =
        serde_json::from_slice(&fs::read(&marker_path).expect("installed ownership marker"))
            .expect("marker JSON");
    marker.as_object_mut().expect("marker object").insert(
        "unexpected_owner_claim".to_string(),
        serde_json::json!(true),
    );
    let changed = serde_json::to_vec_pretty(&marker).expect("changed marker JSON");
    fs::write(&marker_path, &changed).expect("write changed marker");

    assert_eq!(
        codex_install_status_for_submission(review.source_submission_id, &root)
            .expect("unclaimed status"),
        None
    );
    assert_eq!(
        rollback_codex_install(&installed),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert!(target.exists());
    assert_eq!(fs::read(marker_path).expect("marker retained"), changed);
    assert_eq!(
        fs::read_to_string(target.join(SKILL_FILE_NAME)).expect("skill retained"),
        review.skill_md
    );
}

#[test]
fn status_rejects_an_oversized_final_marker_before_reading_it() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("install");
    let marker_path = installed.local_target_path.join(MARKER_NAME);
    fs::File::create(&marker_path)
        .expect("truncate marker")
        .set_len(MAX_MARKER_FILE_BYTES + 1)
        .expect("extend marker");

    assert_eq!(
        codex_install_status_for_submission(review.source_submission_id, &root),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert_eq!(
        rollback_codex_install(&installed),
        Err(SkillInstallError::InstallNotOwned)
    );
    assert_eq!(
        fs::metadata(marker_path)
            .expect("oversized marker retained")
            .len(),
        MAX_MARKER_FILE_BYTES + 1
    );
}

#[test]
fn status_and_rollback_reject_an_oversized_skill_before_reading_it() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("install");
    let skill_path = installed.local_target_path.join(SKILL_FILE_NAME);
    fs::File::create(&skill_path)
        .expect("truncate skill")
        .set_len(MAX_SKILL_FILE_BYTES + 1)
        .expect("extend skill");

    assert_eq!(
        codex_install_status_for_submission(review.source_submission_id, &root),
        Err(SkillInstallError::SkillChanged)
    );
    assert_eq!(
        rollback_codex_install(&installed),
        Err(SkillInstallError::SkillChanged)
    );
    assert_eq!(
        fs::metadata(skill_path)
            .expect("oversized skill retained")
            .len(),
        MAX_SKILL_FILE_BYTES + 1
    );
}

#[test]
fn status_recovers_a_custom_name_by_source_and_ignores_wrong_sessions() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review_named("custom-generated-source-repair", Uuid::new_v4());
    assert_eq!(
        codex_install_status_for_submission(review.source_submission_id, &root)
            .expect("empty status"),
        None
    );
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("commit reviewed install");
    assert_eq!(
        codex_install_status_for_submission(Uuid::new_v4(), &root).expect("other session status"),
        None
    );
    assert_eq!(
        codex_install_status_for_submission(review.source_submission_id, &root)
            .expect("installed status"),
        Some(installed)
    );
}

#[test]
fn status_skips_an_unrelated_occupied_default_directory() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    fs::create_dir_all(root.join("repair-generated-sources")).expect("occupied target");
    fs::write(root.join("repair-generated-sources/owner.txt"), "keep").expect("owner file");
    let source_submission_id = Uuid::new_v4();
    let default_review = review_named("repair-generated-sources", source_submission_id);
    let occupied = plan_codex_install(&default_review, Uuid::new_v4(), &root).expect("plan");
    assert!(occupied.occupied);

    let custom_review = review_named("custom-repair", source_submission_id);
    let plan = plan_codex_install(&custom_review, Uuid::new_v4(), &root).expect("custom plan");
    let installed = commit_codex_install(
        &plan,
        &custom_review,
        &plan.skill_sha256,
        &plan.marker_file_sha256,
    )
    .expect("custom install");
    assert_eq!(
        codex_install_status_for_submission(source_submission_id, &root).expect("status"),
        Some(installed)
    );
    assert_eq!(
        fs::read_to_string(root.join("repair-generated-sources/owner.txt"))
            .expect("owner file retained"),
        "keep"
    );
}

#[test]
fn malformed_partial_install_is_never_claimed_or_removed() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let target = root.join(&review.draft.name);
    fs::create_dir_all(&target).expect("partial target");
    let partial_name = "partial-marker";
    fs::write(target.join(partial_name), b"{\"schema_version\":").expect("partial marker");

    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    assert!(plan.occupied);
    assert_eq!(
        codex_install_status_for_submission(review.source_submission_id, &root).expect("status"),
        None
    );
    assert_eq!(
        fs::read(target.join(partial_name)).expect("partial retained"),
        b"{\"schema_version\":"
    );
}

#[test]
fn status_fails_closed_on_multiple_owned_matches() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let source_submission_id = Uuid::new_v4();
    for name in ["first-source-repair", "second-source-repair"] {
        let review = review_named(name, source_submission_id);
        let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("install");
    }
    assert_eq!(
        codex_install_status_for_submission(source_submission_id, &root),
        Err(SkillInstallError::MultipleMatches)
    );
}

#[test]
fn status_scan_is_bounded_by_all_root_entries() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    fs::create_dir(&root).expect("root");
    for index in 0..=MAX_SKILL_ROOT_ENTRIES {
        fs::write(root.join(format!("unrelated-{index}")), "unrelated").expect("root entry");
    }

    assert_eq!(
        codex_install_status_for_submission(Uuid::new_v4(), &root),
        Err(SkillInstallError::ScanLimit)
    );
}

#[cfg(unix)]
#[test]
fn installed_files_are_private_and_rollback_refuses_symlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};

    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &root).expect("plan");
    let installed =
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256)
            .expect("install");
    let target = installed.local_target_path.as_path();
    for name in [MARKER_NAME, SKILL_FILE_NAME] {
        let mode = fs::metadata(target.join(name))
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
    let external_skill = directory.path().join("external-skill.md");
    fs::write(&external_skill, &review.skill_md).expect("external skill");
    fs::remove_file(target.join(SKILL_FILE_NAME)).expect("remove installed file");
    symlink(&external_skill, target.join(SKILL_FILE_NAME)).expect("skill symlink");

    assert_eq!(
        rollback_codex_install(&installed),
        Err(SkillInstallError::SkillChanged)
    );
    assert!(target.join(SKILL_FILE_NAME).is_symlink());
    assert_eq!(
        fs::read_to_string(external_skill).expect("external content"),
        review.skill_md
    );
}

#[cfg(unix)]
#[test]
fn checked_read_rejects_a_regular_file_replaced_by_a_symlink_before_open() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let checked = directory.path().join("checked.json");
    let replacement = directory.path().join("replacement.json");
    fs::write(&checked, b"original").expect("checked file");
    fs::write(&replacement, b"replacement").expect("replacement file");

    let result = ownership::read_checked_regular_file_if_present_after(
        &checked,
        64,
        SkillInstallError::SkillChanged,
        || {
            fs::remove_file(&checked).expect("remove checked file");
            symlink(&replacement, &checked).expect("replace with symlink");
        },
    );

    assert_eq!(result, Err(SkillInstallError::SkillChanged));
    assert_eq!(
        fs::read(replacement).expect("replacement retained"),
        b"replacement"
    );
}

#[cfg(unix)]
#[test]
fn checked_read_rejects_a_regular_file_replaced_by_a_fifo_before_open() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let checked = directory.path().join("checked.json");
    fs::write(&checked, b"original").expect("checked file");

    let result = ownership::read_checked_regular_file_if_present_after(
        &checked,
        64,
        SkillInstallError::SkillChanged,
        || {
            fs::remove_file(&checked).expect("remove checked file");
            let status = std::process::Command::new("mkfifo")
                .arg(&checked)
                .status()
                .expect("run mkfifo");
            assert!(status.success(), "mkfifo failed");
        },
    );

    assert_eq!(result, Err(SkillInstallError::SkillChanged));
}

#[cfg(unix)]
#[test]
fn quarantine_restore_does_not_overwrite_a_concurrent_replacement() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let root = directory.path().join("skills");
    fs::create_dir_all(&root).expect("root");
    let target = root.join("repair-generated-sources");
    fs::create_dir(&target).expect("target");
    fs::write(target.join("old"), "old").expect("old file");
    let quarantined = quarantine_target(&target).expect("quarantine");
    fs::create_dir(&target).expect("concurrent replacement");
    fs::write(target.join("new"), "new").expect("new file");

    assert_eq!(
        restore_quarantine(&quarantined),
        Err(SkillInstallError::WriteFailed)
    );
    assert_eq!(
        fs::read_to_string(target.join("new")).expect("replacement retained"),
        "new"
    );
    assert_eq!(
        fs::read_to_string(quarantined.path().join("old")).expect("quarantine retained"),
        "old"
    );
}

#[cfg(unix)]
#[test]
fn install_refuses_a_symlinked_skills_root() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().expect("temporary directory");
    let real_root = directory.path().join("real-skills");
    fs::create_dir(&real_root).expect("real skills root");
    let linked_root = directory.path().join("linked-skills");
    symlink(&real_root, &linked_root).expect("skills root symlink");
    let review = review();
    let plan = plan_codex_install(&review, Uuid::new_v4(), &linked_root).expect("plan");

    assert_eq!(
        commit_codex_install(&plan, &review, &plan.skill_sha256, &plan.marker_file_sha256,),
        Err(SkillInstallError::InvalidRoot)
    );
    assert!(fs::read_dir(real_root).expect("real root").next().is_none());
}
