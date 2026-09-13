//! Builds a complete skill package outside Codex's loadable namespace and
//! publishes it with an exclusive directory rename.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::skill_loop::install::ownership::{
    DirectoryIdentity, directory_identity, directory_names, ensure_directory_identity,
};
use crate::skill_loop::install::{
    MARKER_NAME, SKILL_FILE_NAME, SkillInstallError, set_private_directory_permissions,
    sync_directory,
};

const STAGING_PREFIX: &str = ".trace-commons-staging-";

/// An unpublished sibling directory. Dropping it removes only the exact
/// unchanged directory and regular files created by this transaction.
pub(super) struct StagedInstall {
    path: PathBuf,
    identity: DirectoryIdentity,
    published: bool,
}

impl Drop for StagedInstall {
    fn drop(&mut self) {
        if !self.published {
            cleanup_staging_directory(&self.path, self.identity);
        }
    }
}

pub(super) fn stage_install(
    root: &Path,
    install_id: Uuid,
    marker: &[u8],
    skill: &[u8],
) -> Result<StagedInstall, SkillInstallError> {
    let path = root.join(format!("{STAGING_PREFIX}{install_id}"));
    fs::create_dir(&path).map_err(|_| SkillInstallError::WriteFailed)?;
    if set_private_directory_permissions(&path).is_err() {
        let _ = fs::remove_dir(&path);
        return Err(SkillInstallError::WriteFailed);
    }
    let identity = directory_identity(&path, SkillInstallError::WriteFailed)?;
    let staged = StagedInstall {
        path,
        identity,
        published: false,
    };
    write_new_private_file(&staged.path.join(MARKER_NAME), marker)?;
    write_new_private_file(&staged.path.join(SKILL_FILE_NAME), skill)?;
    sync_directory(&staged.path)?;
    Ok(staged)
}

pub(super) fn publish_staged_install(
    mut staged: StagedInstall,
    target: &Path,
    root_identity: DirectoryIdentity,
) -> Result<DirectoryIdentity, SkillInstallError> {
    let root = target.parent().ok_or(SkillInstallError::InvalidRoot)?;
    ensure_directory_identity(root, root_identity, SkillInstallError::InvalidRoot)?;
    exclusive_publish_directory(&staged.path, target)?;
    ensure_directory_identity(target, staged.identity, SkillInstallError::WriteFailed)?;
    staged.published = true;
    Ok(staged.identity)
}

fn write_new_private_file(path: &Path, body: &[u8]) -> Result<(), SkillInstallError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| SkillInstallError::WriteFailed)?;
    file.write_all(body)
        .and_then(|_| file.sync_all())
        .map_err(|_| SkillInstallError::WriteFailed)
}

fn exclusive_publish_directory(source: &Path, destination: &Path) -> Result<(), SkillInstallError> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::CString;
        use std::os::raw::{c_char, c_int, c_uint};
        use std::os::unix::ffi::OsStrExt;

        unsafe extern "C" {
            fn renamex_np(old: *const c_char, new: *const c_char, flags: c_uint) -> c_int;
        }

        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| SkillInstallError::WriteFailed)?;
        let destination = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| SkillInstallError::WriteFailed)?;
        // RENAME_EXCL prevents replacement of any existing target pathname.
        let result = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), 0x0000_0004) };
        rename_result(result)
    }
    #[cfg(target_os = "linux")]
    {
        use std::ffi::CString;
        use std::os::raw::{c_char, c_int, c_uint};
        use std::os::unix::ffi::OsStrExt;

        unsafe extern "C" {
            fn renameat2(
                old_dir_fd: c_int,
                old: *const c_char,
                new_dir_fd: c_int,
                new: *const c_char,
                flags: c_uint,
            ) -> c_int;
        }

        let source = CString::new(source.as_os_str().as_bytes())
            .map_err(|_| SkillInstallError::WriteFailed)?;
        let destination = CString::new(destination.as_os_str().as_bytes())
            .map_err(|_| SkillInstallError::WriteFailed)?;
        // AT_FDCWD and RENAME_NOREPLACE. Unsupported kernels fail closed.
        let result = unsafe {
            renameat2(
                -100,
                source.as_ptr(),
                -100,
                destination.as_ptr(),
                0x0000_0001,
            )
        };
        rename_result(result)
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Foundation::{
            ERROR_ALREADY_EXISTS, ERROR_FILE_EXISTS, GetLastError,
        };
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;

        let mut source = source.as_os_str().encode_wide().collect::<Vec<_>>();
        source.push(0);
        let mut destination = destination.as_os_str().encode_wide().collect::<Vec<_>>();
        destination.push(0);
        // Zero flags excludes MOVEFILE_REPLACE_EXISTING, making publication
        // fail closed whenever the reviewed target pathname already exists.
        let moved = unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) };
        if moved != 0 {
            return Ok(());
        }
        match unsafe { GetLastError() } {
            ERROR_ALREADY_EXISTS | ERROR_FILE_EXISTS => Err(SkillInstallError::Occupied),
            _ => Err(SkillInstallError::WriteFailed),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (source, destination);
        Err(SkillInstallError::WriteFailed)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn rename_result(result: std::os::raw::c_int) -> Result<(), SkillInstallError> {
    if result == 0 {
        return Ok(());
    }
    if std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists {
        Err(SkillInstallError::Occupied)
    } else {
        Err(SkillInstallError::WriteFailed)
    }
}

fn cleanup_staging_directory(staging: &Path, identity: DirectoryIdentity) {
    if ensure_directory_identity(staging, identity, SkillInstallError::WriteFailed).is_err() {
        return;
    }
    let Ok(names) = directory_names(staging) else {
        return;
    };
    if !names
        .iter()
        .all(|name| name == MARKER_NAME || name == SKILL_FILE_NAME)
    {
        return;
    }
    for name in names {
        if ensure_directory_identity(staging, identity, SkillInstallError::WriteFailed).is_err() {
            return;
        }
        let path = staging.join(name);
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return;
        };
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return;
        }
        if fs::remove_file(path).is_err() {
            return;
        }
    }
    if ensure_directory_identity(staging, identity, SkillInstallError::WriteFailed).is_ok() {
        let _ = fs::remove_dir(staging);
    }
}
