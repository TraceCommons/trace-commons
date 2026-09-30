//! INTEGRATION: classifies owned Codex skill directories without following symlinks,
//! bounding scans before install recovery or rollback can mutate the filesystem.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::identity::DeviceIdentity;

use crate::skill_loop::install::{
    InstallMarker, MARKER_NAME, MAX_INSTALL_ENTRIES, MAX_MARKER_FILE_BYTES, MAX_SKILL_FILE_BYTES,
    QUARANTINE_PREFIX, RETAINED_MARKER_NAME, RETAINED_SKILL_NAME, SKILL_FILE_NAME,
    SkillInstallError, canonical_marker_json, marker_has_valid_authentication, sha256,
    sync_directory, valid_skill_name,
};

#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DirectoryIdentity {
    device: u64,
    inode: u64,
}

#[cfg(not(unix))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DirectoryIdentity;

#[derive(Debug)]
pub(super) struct OwnedTarget {
    pub(super) marker: InstallMarker,
    pub(super) names: BTreeSet<String>,
    pub(super) state: OwnedTargetState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnedTargetState {
    Complete,
    Changed(SkillInstallError),
}

pub(super) fn inspect_owned_target(
    target: &Path,
    logical_name: &str,
    source_submission_id: Uuid,
    identity: &DeviceIdentity,
    owner_scope_sha256: &str,
) -> Result<Option<OwnedTarget>, SkillInstallError> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(SkillInstallError::InstallNotOwned),
    };
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Ok(None);
    }

    let marker = match read_checked_regular_file_if_present(
        &target.join(MARKER_NAME),
        MAX_MARKER_FILE_BYTES,
        SkillInstallError::InstallNotOwned,
    ) {
        Ok(Some(body)) => match parse_canonical_marker(&body) {
            Some(marker) => marker,
            None => return Ok(None),
        },
        Ok(None) => return Ok(None),
        Err(error) => return Err(error),
    };

    if marker.schema_version != 2
        || marker.tool != "Codex"
        || !valid_skill_name(&marker.name)
        || marker.name != logical_name
    {
        return Ok(None);
    }
    if marker.source_submission_id != source_submission_id {
        return Ok(Some(OwnedTarget {
            marker,
            names: BTreeSet::new(),
            state: OwnedTargetState::Changed(SkillInstallError::InstallNotOwned),
        }));
    }
    if !marker_has_valid_authentication(&marker, identity, owner_scope_sha256) {
        return Ok(Some(OwnedTarget {
            marker,
            names: BTreeSet::new(),
            state: OwnedTargetState::Changed(SkillInstallError::InstallNotOwned),
        }));
    }
    let names = match directory_names(target) {
        Ok(names) => names,
        Err(error) => {
            return Ok(Some(OwnedTarget {
                marker,
                names: BTreeSet::new(),
                state: OwnedTargetState::Changed(error),
            }));
        }
    };
    let expected = BTreeSet::from([MARKER_NAME.to_string(), SKILL_FILE_NAME.to_string()]);
    if names != expected {
        return Ok(Some(OwnedTarget {
            marker,
            names,
            state: OwnedTargetState::Changed(SkillInstallError::UnexpectedContents),
        }));
    }
    for name in &names {
        let entry_metadata = fs::symlink_metadata(target.join(name))
            .map_err(|_| SkillInstallError::InstallNotOwned)?;
        if !entry_metadata.file_type().is_file() || entry_metadata.file_type().is_symlink() {
            let error = if name == SKILL_FILE_NAME {
                SkillInstallError::SkillChanged
            } else {
                SkillInstallError::UnexpectedContents
            };
            return Ok(Some(OwnedTarget {
                marker,
                names,
                state: OwnedTargetState::Changed(error),
            }));
        }
    }
    let Some(body) = read_checked_regular_file_if_present(
        &target.join(SKILL_FILE_NAME),
        MAX_SKILL_FILE_BYTES,
        SkillInstallError::SkillChanged,
    )?
    else {
        return Ok(Some(OwnedTarget {
            marker,
            names,
            state: OwnedTargetState::Changed(SkillInstallError::SkillChanged),
        }));
    };
    let state = if sha256(&body) == marker.skill_sha256 {
        OwnedTargetState::Complete
    } else {
        OwnedTargetState::Changed(SkillInstallError::SkillChanged)
    };
    Ok(Some(OwnedTarget {
        marker,
        names,
        state,
    }))
}

fn parse_canonical_marker(body: &[u8]) -> Option<InstallMarker> {
    let marker = serde_json::from_slice::<InstallMarker>(body).ok()?;
    (canonical_marker_json(&marker).ok()?.as_bytes() == body).then_some(marker)
}

pub(super) fn read_checked_regular_file_if_present(
    path: &Path,
    max_bytes: u64,
    changed_error: SkillInstallError,
) -> Result<Option<Vec<u8>>, SkillInstallError> {
    read_checked_regular_file_if_present_inner(path, max_bytes, changed_error, || {})
}

#[cfg(test)]
pub(super) fn read_checked_regular_file_if_present_after(
    path: &Path,
    max_bytes: u64,
    changed_error: SkillInstallError,
    after_metadata: impl FnOnce(),
) -> Result<Option<Vec<u8>>, SkillInstallError> {
    read_checked_regular_file_if_present_inner(path, max_bytes, changed_error, after_metadata)
}

fn read_checked_regular_file_if_present_inner(
    path: &Path,
    max_bytes: u64,
    changed_error: SkillInstallError,
    after_metadata: impl FnOnce(),
) -> Result<Option<Vec<u8>>, SkillInstallError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(SkillInstallError::InstallNotOwned),
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Ok(None);
    }
    if metadata.len() > max_bytes {
        return Err(changed_error);
    }
    #[cfg(windows)]
    let checked_file = open_checked_windows_file(path, max_bytes, changed_error)?;
    after_metadata();
    #[cfg(not(windows))]
    let file = open_same_regular_file(path, &metadata, changed_error)?;
    #[cfg(windows)]
    let file = open_same_regular_file(path, &checked_file, changed_error)?;
    let opened_metadata = file
        .metadata()
        .map_err(|_| SkillInstallError::InstallNotOwned)?;
    if !opened_metadata.file_type().is_file() || opened_metadata.len() > max_bytes {
        return Err(changed_error);
    }
    let limit = usize::try_from(max_bytes).map_err(|_| changed_error)?;
    let mut body = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut body)
        .map_err(|_| SkillInstallError::InstallNotOwned)?;
    if body.len() > limit {
        return Err(changed_error);
    }
    Ok(Some(body))
}

#[cfg(not(windows))]
fn open_same_regular_file(
    path: &Path,
    checked: &fs::Metadata,
    changed_error: SkillInstallError,
) -> Result<fs::File, SkillInstallError> {
    let file = crate::evidence_import::open_import_file(path).map_err(|_| changed_error)?;
    let opened = file.metadata().map_err(|_| changed_error)?;
    if !opened.file_type().is_file() {
        return Err(changed_error);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if opened.dev() != checked.dev() || opened.ino() != checked.ino() {
            return Err(changed_error);
        }
    }
    Ok(file)
}

#[cfg(windows)]
fn open_checked_windows_file(
    path: &Path,
    max_bytes: u64,
    changed_error: SkillInstallError,
) -> Result<fs::File, SkillInstallError> {
    use std::os::windows::fs::MetadataExt;

    let file = crate::evidence_import::open_import_file(path).map_err(|_| changed_error)?;
    let metadata = file.metadata().map_err(|_| changed_error)?;
    if !metadata.file_type().is_file()
        || metadata.file_attributes()
            & windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT
            != 0
        || metadata.len() > max_bytes
    {
        return Err(changed_error);
    }
    Ok(file)
}

#[cfg(windows)]
fn open_same_regular_file(
    path: &Path,
    checked: &fs::File,
    changed_error: SkillInstallError,
) -> Result<fs::File, SkillInstallError> {
    let file = crate::evidence_import::open_import_file(path).map_err(|_| changed_error)?;
    if windows_file_identity(&file, changed_error)?
        != windows_file_identity(checked, changed_error)?
    {
        return Err(changed_error);
    }
    Ok(file)
}

#[cfg(windows)]
fn windows_file_identity(
    file: &fs::File,
    changed_error: SkillInstallError,
) -> Result<(u32, u64), SkillInstallError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` owns a valid handle for the duration of the call and
    // `information` points to writable storage of the required type.
    let succeeded =
        unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut information) };
    if succeeded == 0 {
        return Err(changed_error);
    }
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok((information.dwVolumeSerialNumber, file_index))
}

pub(super) fn directory_names(target: &Path) -> Result<BTreeSet<String>, SkillInstallError> {
    let mut names = BTreeSet::new();
    for entry in fs::read_dir(target).map_err(|_| SkillInstallError::InstallNotOwned)? {
        if names.len() == MAX_INSTALL_ENTRIES {
            return Err(SkillInstallError::UnexpectedContents);
        }
        let name = entry
            .map_err(|_| SkillInstallError::InstallNotOwned)?
            .file_name()
            .into_string()
            .map_err(|_| SkillInstallError::UnexpectedContents)?;
        names.insert(name);
    }
    Ok(names)
}

pub(super) fn directory_identity(
    path: &Path,
    error: SkillInstallError,
) -> Result<DirectoryIdentity, SkillInstallError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| error)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(error);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(DirectoryIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        Ok(DirectoryIdentity)
    }
}

pub(super) fn ensure_directory_identity(
    path: &Path,
    expected: DirectoryIdentity,
    error: SkillInstallError,
) -> Result<(), SkillInstallError> {
    if directory_identity(path, error)? == expected {
        Ok(())
    } else {
        Err(error)
    }
}

/// Verified result of moving one checked target out of Codex's loadable namespace.
///
/// This capability is deliberately neither serializable nor debuggable because it
/// owns absolute local paths and inode identities used by destructive follow-up work.
pub(super) struct QuarantinedTarget {
    path: PathBuf,
    original: PathBuf,
    root: PathBuf,
    root_identity: DirectoryIdentity,
    identity: DirectoryIdentity,
}

impl QuarantinedTarget {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

pub(super) fn quarantine_target(target: &Path) -> Result<QuarantinedTarget, SkillInstallError> {
    let root = target.parent().ok_or(SkillInstallError::InvalidRoot)?;
    let root_identity = directory_identity(root, SkillInstallError::InvalidRoot)?;
    let target_identity = directory_identity(target, SkillInstallError::InstallNotOwned)?;
    let quarantined = root.join(format!("{QUARANTINE_PREFIX}{}", Uuid::new_v4()));
    #[cfg(unix)]
    fs::create_dir(&quarantined).map_err(|_| SkillInstallError::WriteFailed)?;
    #[cfg(windows)]
    if fs::symlink_metadata(&quarantined).is_ok() {
        return Err(SkillInstallError::WriteFailed);
    }
    // Safe std APIs do not expose a portable renameat/unlinkat pair. Checking
    // directory identity immediately around the rename narrows a same-user
    // namespace-swap race. Signed markers prevent a replacement from being
    // accepted as an owned install.
    if ensure_directory_identity(root, root_identity, SkillInstallError::InvalidRoot).is_err()
        || ensure_directory_identity(target, target_identity, SkillInstallError::InstallNotOwned)
            .is_err()
    {
        #[cfg(unix)]
        let _ = fs::remove_dir(&quarantined);
        return Err(SkillInstallError::InstallNotOwned);
    }
    fs::rename(target, &quarantined).map_err(|error| {
        #[cfg(unix)]
        let _ = fs::remove_dir(&quarantined);
        if error.kind() == std::io::ErrorKind::NotFound {
            SkillInstallError::InstallNotOwned
        } else {
            SkillInstallError::WriteFailed
        }
    })?;
    let moved = QuarantinedTarget {
        path: quarantined,
        original: target.to_path_buf(),
        root: root.to_path_buf(),
        root_identity,
        identity: target_identity,
    };
    if ensure_directory_identity(root, root_identity, SkillInstallError::InvalidRoot).is_err()
        || ensure_directory_identity(
            moved.path(),
            target_identity,
            SkillInstallError::InstallNotOwned,
        )
        .is_err()
    {
        let _ = restore_quarantine(&moved);
        return Err(SkillInstallError::InstallNotOwned);
    }
    Ok(moved)
}

pub(super) fn retain_quarantined_package(
    quarantined: &QuarantinedTarget,
    expected_names: &BTreeSet<String>,
) {
    if expected_names != &BTreeSet::from([MARKER_NAME.to_string(), SKILL_FILE_NAME.to_string()]) {
        return;
    }
    for (source, retained) in [
        (SKILL_FILE_NAME, RETAINED_SKILL_NAME),
        (MARKER_NAME, RETAINED_MARKER_NAME),
    ] {
        let source = quarantined.path.join(source);
        let retained = quarantined.path.join(retained);
        if fs::symlink_metadata(&retained).is_err() {
            let _ = fs::rename(source, retained);
        }
    }
    let _ = sync_directory(&quarantined.path);
    let _ = sync_directory(&quarantined.root);
}

pub(super) fn restore_quarantine(quarantined: &QuarantinedTarget) -> Result<(), SkillInstallError> {
    ensure_directory_identity(
        &quarantined.root,
        quarantined.root_identity,
        SkillInstallError::InvalidRoot,
    )?;
    ensure_directory_identity(
        &quarantined.path,
        quarantined.identity,
        SkillInstallError::InstallNotOwned,
    )?;
    #[cfg(unix)]
    {
        match fs::create_dir(&quarantined.original) {
            Ok(()) => {}
            Err(_) => return Err(SkillInstallError::WriteFailed),
        }
        if ensure_directory_identity(
            &quarantined.root,
            quarantined.root_identity,
            SkillInstallError::InvalidRoot,
        )
        .is_err()
            || ensure_directory_identity(
                &quarantined.path,
                quarantined.identity,
                SkillInstallError::InstallNotOwned,
            )
            .is_err()
        {
            let _ = fs::remove_dir(&quarantined.original);
            return Err(SkillInstallError::InstallNotOwned);
        }
        if fs::rename(&quarantined.path, &quarantined.original).is_err() {
            let _ = fs::remove_dir(&quarantined.original);
            return Err(SkillInstallError::WriteFailed);
        }
    }
    #[cfg(windows)]
    {
        if fs::symlink_metadata(&quarantined.original).is_ok()
            || fs::rename(&quarantined.path, &quarantined.original).is_err()
        {
            return Err(SkillInstallError::WriteFailed);
        }
    }
    ensure_directory_identity(
        &quarantined.original,
        quarantined.identity,
        SkillInstallError::InstallNotOwned,
    )?;
    sync_directory(&quarantined.root)
}
