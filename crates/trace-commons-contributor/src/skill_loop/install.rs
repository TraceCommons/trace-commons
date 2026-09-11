//! INTEGRATION: supplies the review-before-write and digest-checked rollback
//! boundary for the first Codex Agent Skill installation target.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::DeviceIdentity;
use crate::skill_loop::{SkillReview, sha256, valid_skill_name};

mod marker;
mod ownership;
mod publication;

#[cfg(test)]
use marker::install_marker_sha256;
use marker::{
    canonical_marker_json, installed_from_marker, marker_has_valid_authentication,
    marker_matches_installed, sign_install_marker,
};
use ownership::{
    OwnedTargetState, directory_identity, inspect_owned_target, quarantine_target,
    retain_quarantined_package,
};
#[cfg(test)]
use ownership::{directory_names, restore_quarantine};
use publication::{publish_staged_install, stage_install};

const MARKER_NAME: &str = ".trace-commons-install.json";
const SKILL_FILE_NAME: &str = "SKILL.md";
const QUARANTINE_PREFIX: &str = ".trace-commons-quarantine-";
const RETAINED_SKILL_NAME: &str = ".trace-commons-retained-skill.md";
const RETAINED_MARKER_NAME: &str = ".trace-commons-retained-marker.json";
const MAX_SKILL_ROOT_ENTRIES: usize = 512;
const MAX_INSTALL_ENTRIES: usize = 8;
const MAX_MARKER_FILE_BYTES: u64 = 16 * 1_024;
const MAX_SKILL_FILE_BYTES: u64 = 64 * 1_024;

/// Serialises mutations inside this process. New packages are assembled in a
/// sibling directory and enter Codex's loadable namespace with one exclusive
/// directory rename. Destructive removal validates again after quarantine.
static INSTALL_FILESYSTEM: Mutex<()> = Mutex::new(());

/// Exact, owner-inspectable Codex installation preview created without writing files.
///
/// A commit must present both preview digests unchanged. Every path is symbolic;
/// the private capability required to commit this preview is a separate type.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillInstallPlan {
    /// Unique identifier for this one-time installation preview.
    pub plan_id: Uuid,
    /// Passing evaluation that authorized this preview.
    pub evaluation_id: Uuid,
    /// Coding tool targeted by the preview.
    pub tool: &'static str,
    /// Symbolic directory that will contain the installed package.
    pub target_location: String,
    /// Symbolic location of the `SKILL.md` write.
    pub skill_location: String,
    /// Symbolic location of the signed ownership-marker write.
    pub marker_location: String,
    /// Whether an existing filesystem entry prevents installation.
    pub occupied: bool,
    /// Whether the preview is currently eligible for commit.
    pub can_install: bool,
    /// Exact reviewed `SKILL.md` bytes proposed for installation.
    pub skill_md: String,
    /// SHA-256 digest of the exact `skill_md` bytes.
    pub skill_sha256: String,
    /// Exact UTF-8 bytes shown to the owner and later written to the ownership
    /// marker. The separate digest lets the commit request bind both reviewed
    /// persistent files without relying on JSON reserialization.
    pub marker_json: String,
    /// SHA-256 digest of the exact `marker_json` bytes.
    pub marker_file_sha256: String,
}

/// Non-serializable authority to commit one preview to its resolved local paths.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CodexInstallPlan {
    preview: SkillInstallPlan,
    target: PathBuf,
    skill: PathBuf,
    marker: PathBuf,
}

impl CodexInstallPlan {
    pub(crate) fn preview(&self) -> &SkillInstallPlan {
        &self.preview
    }

    #[cfg(test)]
    pub(crate) fn from_test_preview(preview: SkillInstallPlan, target: PathBuf) -> Self {
        Self {
            skill: target.join(SKILL_FILE_NAME),
            marker: target.join(MARKER_NAME),
            target,
            preview,
        }
    }
}

impl std::fmt::Debug for CodexInstallPlan {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("CodexInstallPlan")
            .field(&self.preview)
            .finish()
    }
}

#[cfg(test)]
impl std::ops::Deref for CodexInstallPlan {
    type Target = SkillInstallPlan;

    fn deref(&self) -> &Self::Target {
        self.preview()
    }
}

#[cfg(test)]
impl std::ops::DerefMut for CodexInstallPlan {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.preview
    }
}

/// Verified receipt for an installed, account-owned Codex Agent Skill.
///
/// The receipt binds the installation to its evaluation, source contribution,
/// reviewed skill digest, signed marker digest, device identity, and owner scope.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InstalledSkill {
    /// Installation identifier copied from the signed ownership marker.
    pub install_id: Uuid,
    /// Passing evaluation that authorized the installation.
    pub evaluation_id: Uuid,
    /// Account-owned contribution from which the skill was derived.
    pub source_submission_id: Uuid,
    /// Coding tool that loads this installation.
    pub tool: String,
    /// Validated Agent Skill name and directory name.
    pub name: String,
    /// Symbolic installation directory safe to expose over IPC.
    pub target_location: String,
    /// SHA-256 digest of the installed `SKILL.md` bytes.
    pub skill_sha256: String,
    /// Digest observed when the signed ownership marker was accepted.
    pub marker_sha256: String,
    /// RFC 3339 installation timestamp signed into the ownership marker.
    pub installed_at: String,
}

/// Non-serializable authority to inspect or roll back one verified installation.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct CodexInstalledSkill {
    receipt: InstalledSkill,
    local_target_path: PathBuf,
}

impl CodexInstalledSkill {
    pub(crate) fn receipt(&self) -> &InstalledSkill {
        &self.receipt
    }

    #[cfg(test)]
    pub(crate) fn from_test_receipt(receipt: InstalledSkill, local_target_path: PathBuf) -> Self {
        Self {
            receipt,
            local_target_path,
        }
    }
}

impl std::fmt::Debug for CodexInstalledSkill {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("CodexInstalledSkill")
            .field(&self.receipt)
            .finish()
    }
}

#[cfg(test)]
impl std::ops::Deref for CodexInstalledSkill {
    type Target = InstalledSkill;

    fn deref(&self) -> &Self::Target {
        self.receipt()
    }
}

#[cfg(test)]
impl std::ops::DerefMut for CodexInstalledSkill {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.receipt
    }
}

/// Result of removing an owned skill from its loadable Codex location.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRollbackResult {
    /// Whether the verified skill was removed from the loadable target path.
    pub removed: bool,
    /// Whether the package was retained in a non-loadable quarantine directory.
    pub retained_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct InstallMarker {
    schema_version: u8,
    install_id: Uuid,
    evaluation_id: Uuid,
    tool: String,
    name: String,
    skill_sha256: String,
    source_submission_id: Uuid,
    source_evidence_ids: Vec<Uuid>,
    owner_scope_sha256: String,
    device_key_id: String,
    installed_at: String,
    marker_sha256: String,
    device_signature_b64: String,
}

/// Stable refusal reasons for Codex Agent Skill installation and rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillInstallError {
    /// Neither `CODEX_HOME` nor the current user's home directory was available.
    HomeUnavailable,
    /// The supplied skills root or derived target was unsafe or non-absolute.
    InvalidRoot,
    /// The reviewed Agent Skill name was invalid.
    InvalidName,
    /// An existing target entry prevents creating the reviewed package.
    Occupied,
    /// Exact package, marker, path, lineage, or preview digests no longer match.
    PlanChanged,
    /// A filesystem write, publication, permission, or durability step failed.
    WriteFailed,
    /// The signed marker does not belong to the active device and account scope.
    InstallNotOwned,
    /// The installed `SKILL.md` changed after commit.
    SkillChanged,
    /// The owned target contains files outside the verified installation set.
    UnexpectedContents,
    /// More than one verified installation claims the same source contribution.
    MultipleMatches,
    /// The bounded Codex skills-root scan reached its entry limit.
    ScanLimit,
}

impl SkillInstallError {
    #[must_use]
    /// Returns the stable daemon refusal label for this install failure.
    pub fn label(self) -> &'static str {
        match self {
            Self::HomeUnavailable => "skill-home-unavailable",
            Self::InvalidRoot => "skill-install-root-invalid",
            Self::InvalidName => "skill-name-invalid",
            Self::Occupied => "skill-install-occupied",
            Self::PlanChanged => "skill-install-plan-changed",
            Self::WriteFailed => "skill-install-write-failed",
            Self::InstallNotOwned => "skill-install-not-owned",
            Self::SkillChanged => "skill-install-modified",
            Self::UnexpectedContents => "skill-install-extra-files",
            Self::MultipleMatches => "skill-install-multiple-matches",
            Self::ScanLimit => "skill-install-scan-limit",
        }
    }
}

/// Resolves the absolute Codex skills directory from `CODEX_HOME` or the user home.
///
/// Relative `CODEX_HOME` values are rejected.
pub fn codex_skills_root() -> Result<PathBuf, SkillInstallError> {
    let codex_home = std::env::var_os("CODEX_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .ok_or(SkillInstallError::HomeUnavailable)?;
    if !codex_home.is_absolute() {
        return Err(SkillInstallError::InvalidRoot);
    }
    Ok(codex_home.join("skills"))
}

/// Builds an exact, signed installation preview without writing package files.
///
/// The plan binds the reviewed `SKILL.md`, account-owned source lineage, evaluation,
/// active device identity, and owner-scope digest. It reports symbolic locations and
/// both file digests so the owner can inspect every persistent write before commit.
/// The caller must derive `owner_scope_sha256` from the authenticated account.
pub(crate) fn plan_codex_install(
    review: &SkillReview,
    evaluation_id: Uuid,
    root: &Path,
    identity: &DeviceIdentity,
    owner_scope_sha256: &str,
) -> Result<CodexInstallPlan, SkillInstallError> {
    if !root.is_absolute() {
        return Err(SkillInstallError::InvalidRoot);
    }
    if !valid_owner_scope_sha256(owner_scope_sha256) {
        return Err(SkillInstallError::InstallNotOwned);
    }
    if !valid_skill_name(&review.draft.name) {
        return Err(SkillInstallError::InvalidName);
    }
    if review.skill_md.len() > MAX_SKILL_FILE_BYTES as usize
        || sha256(review.skill_md.as_bytes()) != review.skill_sha256
    {
        return Err(SkillInstallError::PlanChanged);
    }
    let target = root.join(&review.draft.name);
    let skill_path = target.join(SKILL_FILE_NAME);
    let marker_path = target.join(MARKER_NAME);
    let occupied = match fs::symlink_metadata(&target) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => true,
    };
    let mut marker = InstallMarker {
        schema_version: 2,
        install_id: Uuid::new_v4(),
        evaluation_id,
        tool: "Codex".to_string(),
        name: review.draft.name.clone(),
        skill_sha256: review.skill_sha256.clone(),
        source_submission_id: review.source_submission_id,
        source_evidence_ids: review.source_evidence_ids.clone(),
        owner_scope_sha256: owner_scope_sha256.to_string(),
        device_key_id: identity.device_key_id.clone(),
        installed_at: Utc::now().to_rfc3339(),
        marker_sha256: String::new(),
        device_signature_b64: String::new(),
    };
    sign_install_marker(&mut marker, identity)?;
    let marker_json = canonical_marker_json(&marker)?;
    if marker_json.len() > MAX_MARKER_FILE_BYTES as usize {
        return Err(SkillInstallError::PlanChanged);
    }
    let marker_file_sha256 = sha256(marker_json.as_bytes());
    Ok(CodexInstallPlan {
        preview: SkillInstallPlan {
            plan_id: Uuid::new_v4(),
            evaluation_id,
            tool: "Codex",
            target_location: symbolic_target_location(&review.draft.name),
            skill_location: symbolic_skill_location(&review.draft.name),
            marker_location: symbolic_marker_location(&review.draft.name),
            occupied,
            can_install: !occupied,
            skill_md: review.skill_md.clone(),
            skill_sha256: review.skill_sha256.clone(),
            marker_json,
            marker_file_sha256,
        },
        target,
        skill: skill_path,
        marker: marker_path,
    })
}

/// Commits exactly the reviewed skill and marker bytes shown in an install preview.
///
/// Both preview digests, the signed device marker, owner scope, source lineage, and
/// symbolic-to-local path mapping are revalidated before atomic publication. The
/// operation refuses occupied targets and never replaces an unrelated installation.
/// The caller must derive `owner_scope_sha256` from the authenticated account.
pub(crate) fn commit_codex_install(
    plan: &CodexInstallPlan,
    review: &SkillReview,
    identity: &DeviceIdentity,
    owner_scope_sha256: &str,
    as_previewed_sha256: &str,
    as_previewed_marker_sha256: &str,
) -> Result<CodexInstalledSkill, SkillInstallError> {
    commit_codex_install_with_final_sync(
        plan,
        review,
        identity,
        owner_scope_sha256,
        as_previewed_sha256,
        as_previewed_marker_sha256,
        sync_published_install,
    )
}

fn commit_codex_install_with_final_sync(
    plan: &CodexInstallPlan,
    review: &SkillReview,
    identity: &DeviceIdentity,
    owner_scope_sha256: &str,
    as_previewed_sha256: &str,
    as_previewed_marker_sha256: &str,
    final_sync: fn(&Path, &Path) -> Result<(), SkillInstallError>,
) -> Result<CodexInstalledSkill, SkillInstallError> {
    let preview = &plan.preview;
    if !valid_owner_scope_sha256(owner_scope_sha256) {
        return Err(SkillInstallError::InstallNotOwned);
    }
    if !preview.can_install || preview.occupied {
        return Err(SkillInstallError::Occupied);
    }
    if preview.skill_sha256 != as_previewed_sha256
        || review.skill_sha256 != as_previewed_sha256
        || preview.skill_md.len() > MAX_SKILL_FILE_BYTES as usize
        || preview.marker_json.len() > MAX_MARKER_FILE_BYTES as usize
        || sha256(preview.skill_md.as_bytes()) != as_previewed_sha256
        || preview.skill_md != review.skill_md
        || preview.marker_file_sha256 != as_previewed_marker_sha256
        || sha256(preview.marker_json.as_bytes()) != as_previewed_marker_sha256
    {
        return Err(SkillInstallError::PlanChanged);
    }
    let target = plan.target.clone();
    let root = target.parent().ok_or(SkillInstallError::InvalidRoot)?;
    let name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(SkillInstallError::InvalidName)?;
    if !root.is_absolute() || !valid_skill_name(name) || name != review.draft.name {
        return Err(SkillInstallError::InvalidRoot);
    }
    let skill_path = target.join(SKILL_FILE_NAME);
    let marker_path = target.join(MARKER_NAME);
    if skill_path != plan.skill
        || marker_path != plan.marker
        || preview.target_location != symbolic_target_location(&review.draft.name)
        || preview.skill_location != symbolic_skill_location(&review.draft.name)
        || preview.marker_location != symbolic_marker_location(&review.draft.name)
    {
        return Err(SkillInstallError::PlanChanged);
    }
    let marker: InstallMarker =
        serde_json::from_str(&preview.marker_json).map_err(|_| SkillInstallError::PlanChanged)?;
    if canonical_marker_json(&marker)? != preview.marker_json
        || !marker_has_valid_authentication(&marker, identity, owner_scope_sha256)
        || !marker_matches_review(&marker, review, preview.evaluation_id)
        || marker.install_id.is_nil()
        || marker.installed_at.trim().is_empty()
    {
        return Err(SkillInstallError::PlanChanged);
    }
    let _guard = filesystem_guard();
    create_private_directories(root)?;
    let root_identity = directory_identity(root, SkillInstallError::InvalidRoot)?;
    if fs::symlink_metadata(&target).is_ok() {
        return Err(SkillInstallError::Occupied);
    }

    let staged = stage_install(
        root,
        marker.install_id,
        preview.marker_json.as_bytes(),
        preview.skill_md.as_bytes(),
    )?;
    publish_staged_install(staged, &target, root_identity)?;
    let _final_sync_failed = final_sync(&target, root).is_err();
    let Some(owned) = inspect_owned_target(
        &target,
        &review.draft.name,
        review.source_submission_id,
        identity,
        owner_scope_sha256,
    )?
    else {
        return Err(SkillInstallError::WriteFailed);
    };
    if owned.marker != marker {
        return Err(SkillInstallError::WriteFailed);
    }
    if owned.state != OwnedTargetState::Complete {
        return Err(SkillInstallError::WriteFailed);
    }
    // The complete directory is already visible after the exclusive rename.
    // A final parent-directory sync failure cannot be represented as absence;
    // status repeats the same authenticated package verification.
    installed_from_marker(marker, &target)
}

/// Removes a verified, unchanged installation from its loadable Codex path.
///
/// Rollback revalidates the signed marker against the active device, account scope,
/// source contribution, and receipt. The owned directory is atomically renamed to a
/// non-loadable quarantine location and retained for inspection or recovery. The
/// caller must derive `owner_scope_sha256` from the authenticated account.
pub(crate) fn rollback_codex_install(
    installed: &CodexInstalledSkill,
    identity: &DeviceIdentity,
    owner_scope_sha256: &str,
) -> Result<SkillRollbackResult, SkillInstallError> {
    if !valid_owner_scope_sha256(owner_scope_sha256) {
        return Err(SkillInstallError::InstallNotOwned);
    }
    let receipt = &installed.receipt;
    let target = installed.local_target_path.clone();
    let root = target.parent().ok_or(SkillInstallError::InvalidRoot)?;
    let target_name = target
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or(SkillInstallError::InvalidName)?;
    if !root.is_absolute() || !valid_skill_name(target_name) || target_name != receipt.name {
        return Err(SkillInstallError::InvalidRoot);
    }
    let _guard = filesystem_guard();
    let owned = inspect_owned_target(
        &target,
        &receipt.name,
        receipt.source_submission_id,
        identity,
        owner_scope_sha256,
    )
    .and_then(|owned| {
        let owned = owned.ok_or(SkillInstallError::InstallNotOwned)?;
        if owned.state != OwnedTargetState::Complete
            || !marker_matches_installed(&owned.marker, receipt)
        {
            return Err(match owned.state {
                OwnedTargetState::Changed(error) => error,
                OwnedTargetState::Complete => SkillInstallError::InstallNotOwned,
            });
        }
        Ok(owned)
    })?;

    // The directory rename is rollback's logical commit. Everything that can
    // reject the request runs before it, while the original target can remain
    // intact. After the rename, never delete a possibly raced package and
    // never report that the loadable target still exists.
    let quarantined = quarantine_target(&target)?;
    let _ = sync_directory(root);
    retain_quarantined_package(&quarantined, &owned.names);
    Ok(SkillRollbackResult {
        removed: true,
        retained_directory: true,
    })
}

/// Finds and verifies the installation derived from one account-owned contribution.
///
/// The bounded scan accepts only a uniquely matching signed marker for the active
/// device and owner scope. The caller must derive `owner_scope_sha256` from the
/// authenticated account.
pub(crate) fn codex_install_status_for_submission(
    source_submission_id: Uuid,
    root: &Path,
    identity: &DeviceIdentity,
    owner_scope_sha256: &str,
) -> Result<Option<CodexInstalledSkill>, SkillInstallError> {
    if !root.is_absolute() {
        return Err(SkillInstallError::InvalidRoot);
    }
    if !valid_owner_scope_sha256(owner_scope_sha256) {
        return Err(SkillInstallError::InstallNotOwned);
    }
    let root_metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(SkillInstallError::InvalidRoot),
    };
    if !root_metadata.file_type().is_dir() || root_metadata.file_type().is_symlink() {
        return Err(SkillInstallError::InvalidRoot);
    }
    let _guard = filesystem_guard();
    let mut children = Vec::new();
    for (scanned_entries, entry) in fs::read_dir(root)
        .map_err(|_| SkillInstallError::InvalidRoot)?
        .enumerate()
    {
        if scanned_entries >= MAX_SKILL_ROOT_ENTRIES {
            return Err(SkillInstallError::ScanLimit);
        }
        let entry = entry.map_err(|_| SkillInstallError::InvalidRoot)?;
        let file_type = entry
            .file_type()
            .map_err(|_| SkillInstallError::InvalidRoot)?;
        let name = match entry.file_name().into_string() {
            Ok(name) => name,
            Err(_) => continue,
        };
        if file_type.is_dir() && !file_type.is_symlink() && !name.starts_with(QUARANTINE_PREFIX) {
            children.push((entry.path(), name));
        }
    }
    let mut matching = Vec::new();
    for (target, name) in children {
        let Some(owned) = inspect_owned_target(
            &target,
            &name,
            source_submission_id,
            identity,
            owner_scope_sha256,
        )?
        else {
            continue;
        };
        if owned.marker.source_submission_id != source_submission_id {
            continue;
        }
        matching.push((target, name, owned));
    }
    match matching.len() {
        0 => Ok(None),
        1 => {
            let (target, _name, owned) =
                matching.pop().ok_or(SkillInstallError::InstallNotOwned)?;
            match owned.state {
                OwnedTargetState::Complete => {
                    installed_from_marker(owned.marker, &target).map(Some)
                }
                OwnedTargetState::Changed(error) => Err(error),
            }
        }
        _ => Err(SkillInstallError::MultipleMatches),
    }
}

fn create_private_directories(root: &Path) -> Result<(), SkillInstallError> {
    if root.exists() {
        let metadata = fs::symlink_metadata(root).map_err(|_| SkillInstallError::InvalidRoot)?;
        if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
            return Err(SkillInstallError::InvalidRoot);
        }
        return Ok(());
    }
    fs::create_dir_all(root).map_err(|_| SkillInstallError::WriteFailed)?;
    let metadata = fs::symlink_metadata(root).map_err(|_| SkillInstallError::InvalidRoot)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(SkillInstallError::InvalidRoot);
    }
    set_private_directory_permissions(root)
}

fn set_private_directory_permissions(path: &Path) -> Result<(), SkillInstallError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| SkillInstallError::WriteFailed)?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), SkillInstallError> {
    #[cfg(windows)]
    {
        let _ = path;
        return Ok(());
    }
    #[cfg(not(windows))]
    {
        let directory = OpenOptions::new()
            .read(true)
            .open(path)
            .map_err(|_| SkillInstallError::WriteFailed)?;
        directory
            .sync_all()
            .map_err(|_| SkillInstallError::WriteFailed)
    }
}

fn sync_published_install(target: &Path, root: &Path) -> Result<(), SkillInstallError> {
    sync_directory(target)?;
    sync_directory(root)
}

fn filesystem_guard() -> MutexGuard<'static, ()> {
    match INSTALL_FILESYSTEM.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn marker_matches_review(
    marker: &InstallMarker,
    review: &SkillReview,
    evaluation_id: Uuid,
) -> bool {
    marker.schema_version == 2
        && marker.evaluation_id == evaluation_id
        && marker.tool == "Codex"
        && marker.name == review.draft.name
        && marker.skill_sha256 == review.skill_sha256
        && marker.source_submission_id == review.source_submission_id
        && marker.source_evidence_ids == review.source_evidence_ids
}

fn valid_owner_scope_sha256(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn symbolic_target_location(name: &str) -> String {
    format!("$CODEX_HOME/skills/{name}")
}

fn symbolic_skill_location(name: &str) -> String {
    format!("{}/{}", symbolic_target_location(name), SKILL_FILE_NAME)
}

fn symbolic_marker_location(name: &str) -> String {
    format!("{}/{}", symbolic_target_location(name), MARKER_NAME)
}

#[cfg(test)]
mod tests;
