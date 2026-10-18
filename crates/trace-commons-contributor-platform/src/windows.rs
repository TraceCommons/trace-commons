//! Win32 helpers shared by the contributor's named-pipe transport and the
//! Insights store's owner-only DACL.
//!
//! Moved here from `trace_commons_contributor::daemon::win_pipe`, which
//! re-exports [`current_user_sid_string`] at its old path. The pipe itself
//! stays there.

use std::ffi::c_void;

use anyhow::{Context, Result, bail};
use windows_sys::Win32::Foundation::{HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::{TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// The current process's user SID, as an SDDL string.
///
/// This is the identity the pipe is restricted to. Deriving it from the
/// process token rather than from a username avoids every ambiguity around
/// domain accounts, renamed accounts, and localised well-known group names.
pub fn current_user_sid_string() -> Result<String> {
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle that needs no
    // closing, and `token` is a valid out-pointer.
    let opened = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if opened == 0 {
        bail!("cannot open the process token to determine the pipe's owner");
    }
    let _guard = TokenHandle(token);

    // Ask for the size first: TOKEN_USER is variable-length because it ends
    // in a SID, whose length depends on the account's domain.
    let mut needed: u32 = 0;
    // SAFETY: a null buffer with a zero length is the documented way to
    // query the required size; the call is expected to fail.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
    if needed == 0 {
        bail!("cannot size the process token's user information");
    }
    let mut buf = vec![0u8; needed as usize];
    // SAFETY: `buf` is `needed` bytes, which is the size the previous call
    // asked for.
    let got = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr() as *mut c_void,
            needed,
            &mut needed,
        )
    };
    if got == 0 {
        bail!("cannot read the process token's user information");
    }

    // SAFETY: on success the buffer holds a TOKEN_USER, whose first field is
    // the SID_AND_ATTRIBUTES we need.
    let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut raw: *mut u16 = std::ptr::null_mut();
    // SAFETY: `token_user.User.Sid` points into `buf`, which outlives this
    // call, and `raw` is a valid out-pointer.
    let converted = unsafe { ConvertSidToStringSidW(token_user.User.Sid, &mut raw) };
    if converted == 0 || raw.is_null() {
        bail!("cannot render the process token's user SID");
    }
    // SAFETY: `raw` is a NUL-terminated wide string allocated by
    // `ConvertSidToStringSidW`.
    let len = unsafe {
        let mut n = 0usize;
        while *raw.add(n) != 0 {
            n += 1;
        }
        n
    };
    // SAFETY: `raw` is valid for `len` u16s, established immediately above.
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(raw, len) })
        .context("the user SID is not valid UTF-16");
    // SAFETY: `ConvertSidToStringSidW` documents `LocalFree` as its release
    // function. Freed on both the ok and error paths because `sid` is not
    // returned until after this point.
    unsafe { LocalFree(raw as *mut c_void) };
    sid
}

/// Closes a token handle on drop.
struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: `self.0` is a handle from `OpenProcessToken` and this
            // is the only place it is closed.
            unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
        }
    }
}

// `GetTokenInformation` lives in Win32_Security but is not re-exported at the
// path the rest of this module uses, so it is named explicitly here.
use windows_sys::Win32::Security::GetTokenInformation;
