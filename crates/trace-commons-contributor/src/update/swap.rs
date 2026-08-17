//! Replacing the installed binary.
//!
//! Unix replaces the directory entry with a rename, which is atomic within a
//! filesystem and is why a process already running the old image keeps
//! working: it holds the inode, not the name. Windows cannot unlink a running
//! image at all, so the running file is renamed aside first and then deleted
//! -- or, when it is still mapped, scheduled for deletion at the next boot.
//!
//! Both paths require `new_binary` and `target` to be on the same
//! filesystem. Staging is expected to place the downloaded artifact beside
//! the target for exactly this reason; a cross-filesystem rename fails
//! explicitly here rather than leaving the install half-applied.

use std::path::Path;

use super::fetch::VerifiedArtifact;

#[derive(Debug, thiserror::Error)]
pub enum SwapError {
    #[error("update_swap_io_failed")]
    Io,
}

/// Move the verified binary into `target`, replacing whatever is there.
///
/// `new_binary` can only be a [`VerifiedArtifact`], which only
/// `fetch::download_verified` can produce outside of tests. That is what
/// makes it impossible to reach this function with bytes that were never
/// checked against the signed manifest.
pub fn swap_in_place(new_binary: &VerifiedArtifact, target: &Path) -> Result<(), SwapError> {
    let new_binary = new_binary.path();
    if !new_binary.is_file() {
        return Err(SwapError::Io);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new_binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|_| SwapError::Io)?;
        // fsync before the rename that makes this the live binary: the
        // executable bytes must be durable before anything can be launched
        // from this path.
        std::fs::File::open(new_binary)
            .and_then(|f| f.sync_all())
            .map_err(|_| SwapError::Io)?;
        // A plain rename atomically replaces `target`'s directory entry if
        // one exists, so there is no window in which `target` is briefly
        // missing (unlike unlink-then-rename). A process that already holds
        // the old file open keeps running against the old inode either way.
        // Same-filesystem is required for the rename to be atomic at all;
        // when it is not, this fails closed rather than leaving neither
        // binary fully in place.
        std::fs::rename(new_binary, target).map_err(|_| SwapError::Io)?;
        return Ok(());
    }
    #[cfg(windows)]
    {
        let aside = target.with_extension(format!(
            "old-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let moved_aside = match std::fs::rename(target, &aside) {
            Ok(()) => true,
            // Nothing to move aside is not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(SwapError::Io),
        };
        if let Err(_e) = std::fs::rename(new_binary, target) {
            // Put the working binary back. Leaving the install with no
            // executable at all is a strictly worse outcome than not
            // updating.
            if moved_aside {
                let _ = std::fs::rename(&aside, target);
            }
            return Err(SwapError::Io);
        }
        if moved_aside {
            schedule_delete(&aside);
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(SwapError::Io)
}

/// Delete the displaced binary, or arrange for the OS to do it at the next
/// boot when it is still mapped by the running process.
///
/// Best effort by design: the swap has already succeeded at this point, and a
/// leftover `.old-<n>` file is a wart, not a failure to report to a
/// contributor.
#[cfg(windows)]
fn schedule_delete(path: &Path) {
    use std::os::windows::ffi::OsStrExt;

    if std::fs::remove_file(path).is_ok() {
        return;
    }
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    // A NULL destination with MOVEFILE_DELAY_UNTIL_REBOOT registers the path
    // in PendingFileRenameOperations for deletion at the next boot. Requires
    // administrator rights on some systems; a failure is ignored for the same
    // reason the remove_file failure above is.
    unsafe {
        let _ = windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            wide.as_ptr(),
            std::ptr::null(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_DELAY_UNTIL_REBOOT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_bytes_replace_the_old_ones() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        let staged = d.path().join("staged-binary");
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();
        let staged = VerifiedArtifact::for_test(staged);

        swap_in_place(&staged, &target).expect("swap");

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(
            !staged.path().exists(),
            "the staged file is consumed by the swap"
        );
    }

    #[test]
    fn a_missing_target_is_still_installed_into() {
        // First install through the updater, or a target somebody removed
        // between staging and applying. Placing the verified binary is still
        // the right outcome.
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        let staged = d.path().join("staged-binary");
        std::fs::write(&staged, b"new").unwrap();
        let staged = VerifiedArtifact::for_test(staged);

        swap_in_place(&staged, &target).expect("swap");
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn a_missing_staged_binary_is_an_error_and_leaves_the_target_alone() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        std::fs::write(&target, b"old").unwrap();
        let staged = VerifiedArtifact::for_test(d.path().join("nope"));

        assert!(swap_in_place(&staged, &target).is_err());
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"old",
            "a failed swap must not destroy the working binary"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_installed_binary_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        let staged = d.path().join("staged-binary");
        std::fs::write(&staged, b"new").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();
        let staged = VerifiedArtifact::for_test(staged);

        swap_in_place(&staged, &target).expect("swap");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "installed mode was {:o}", mode & 0o777);
    }
}
