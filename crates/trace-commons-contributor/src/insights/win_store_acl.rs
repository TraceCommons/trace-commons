//! Owner-only access control for the local Insights store on Windows.
//!
//! The Unix path creates the store `0700` and refuses to open a directory that
//! is group- or world-accessible. Windows has no mode bits, so the equivalent
//! restriction is a protected DACL carrying exactly one allow entry for the
//! current user. It is applied on every open rather than only at creation: a
//! directory created earlier, by an older build, or by another tool must not
//! stay widened just because it already exists.
//!
//! The DACL is `D:P(A;OICI;GA;;;<sid>)`:
//!   - `P`    protected, so an inheritable permissive entry from a parent
//!            directory cannot widen access after the fact.
//!   - `OICI` object and container inherit, so the index and lock files the
//!            store creates inside the directory carry the same restriction.
//!   - `GA`   generic all, for exactly one trustee: the current user.
//!
//! There is deliberately no entry for Administrators, for the reason given in
//! `daemon::win_pipe`: an administrator can already take ownership.
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use anyhow::{Result, bail};
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
    SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR,
};

/// A security descriptor owning its Win32 allocation.
struct OwnedSecurityDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for OwnedSecurityDescriptor {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from
            // `ConvertStringSecurityDescriptorToSecurityDescriptorW`, which
            // documents `LocalFree` as its release function, and this is the
            // only place it is freed.
            unsafe { LocalFree(self.0 as *mut c_void) };
        }
    }
}

pub(super) fn sddl_for(sid: &str) -> String {
    format!("D:P(A;OICI;GA;;;{sid})")
}

/// Restrict an existing store directory to the current user, or fail closed.
pub(super) fn restrict_to_current_user(dir: &Path) -> Result<()> {
    let sid = crate::daemon::win_pipe::current_user_sid_string()
        .map_err(|_| super::InsightsStoreError::PrivateDirectoryRequired)?;
    let sddl = sddl_for(&sid);
    let wide_sddl: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
    let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
    // SAFETY: `wide_sddl` is a NUL-terminated wide string that outlives the
    // call, `descriptor` is a valid out-pointer, and a null size out-pointer
    // is permitted.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide_sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    };
    if converted == 0 || descriptor.is_null() {
        bail!(super::InsightsStoreError::PrivateDirectoryRequired);
    }
    let descriptor = OwnedSecurityDescriptor(descriptor);

    let mut present = 0;
    let mut defaulted = 0;
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: `descriptor.0` is a valid self-relative security descriptor and
    // the three out-pointers are valid for the duration of the call.
    let read =
        unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) };
    if read == 0 || present == 0 || dacl.is_null() {
        bail!(super::InsightsStoreError::PrivateDirectoryRequired);
    }

    let wide_path: Vec<u16> = dir
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide_path` is a NUL-terminated wide path that outlives the
    // call, and `dacl` points into the descriptor kept alive below. Owner,
    // group and SACL are deliberately left unchanged.
    let applied = unsafe {
        SetNamedSecurityInfoW(
            wide_path.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            dacl,
            std::ptr::null(),
        )
    };
    drop(descriptor);
    if applied != ERROR_SUCCESS {
        bail!(super::InsightsStoreError::PrivateDirectoryRequired);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_store_descriptor_is_protected_and_grants_exactly_one_trustee() {
        let sddl = sddl_for("S-1-5-21-0-0-0-1000");
        assert!(sddl.starts_with("D:P("), "the DACL must be protected");
        assert_eq!(sddl.matches("(A;").count(), 1, "exactly one allow entry");
        assert!(!sddl.contains("WD"), "must never grant to Everyone");
        assert!(
            !sddl.contains("AU"),
            "must never grant to Authenticated Users"
        );
        assert!(
            sddl.contains("OICI"),
            "child files must inherit the restriction"
        );
    }

    #[test]
    fn an_existing_store_directory_is_restricted_on_open() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("insights");
        std::fs::create_dir_all(&dir).unwrap();
        restrict_to_current_user(&dir)
            .expect("the current user must be able to restrict a directory it owns");
        // The restriction must not cost the owner its own access.
        std::fs::write(dir.join("index.json"), b"{}").unwrap();
        assert_eq!(std::fs::read(dir.join("index.json")).unwrap(), b"{}");
    }
}
