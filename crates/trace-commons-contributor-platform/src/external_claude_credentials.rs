//! Narrow read-only access to Claude Code's legacy Keychain credential.
//!
//! The provider owns these entries. This module never writes, copies to another
//! store, or prints them; background callers suppress authentication UI per query.

const PREFIX: &str = "Claude Code-credentials";
#[cfg(all(
    not(target_os = "macos"),
    not(any(test, feature = "test-credential-store"))
))]
const MAX_BYTES: usize = 1024 * 1024;

// A canceled async waiter cannot cancel SecItemCopyMatching. Keep this guard
// on the OS-query thread, so a retry cannot stack a second dialog for the item.
#[cfg(any(test, not(feature = "test-credential-store")))]
fn with_pending<T>(
    service: &str,
    operation: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    use std::collections::BTreeSet;
    use std::sync::{Mutex, OnceLock};
    static PENDING: OnceLock<Mutex<BTreeSet<String>>> = OnceLock::new();
    let pending = PENDING.get_or_init(|| Mutex::new(BTreeSet::new()));
    if !pending
        .lock()
        .map_err(|_| "unavailable")?
        .insert(service.to_owned())
    {
        return Err("keychain_read_pending");
    }
    struct Pending<'a> {
        service: &'a str,
        pending: &'a Mutex<BTreeSet<String>>,
    }
    impl Drop for Pending<'_> {
        fn drop(&mut self) {
            self.pending
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .remove(self.service);
        }
    }
    let _guard = Pending { service, pending };
    operation()
}

/// Read one exact Claude service, optionally allowing a user-triggered prompt.
/// Tests and dependent test builds are always refused before touching Keychain.
pub fn read(service: &str, interactive: bool) -> Result<Option<Vec<u8>>, &'static str> {
    if !valid_service(service) {
        return Err("unavailable");
    }
    #[cfg(any(test, feature = "test-credential-store"))]
    {
        let _ = interactive;
        Err("keychain_needs_permission")
    }
    #[cfg(not(any(test, feature = "test-credential-store")))]
    {
        with_pending(service, || read_native(service, interactive))
    }
}

/// Read only the item's modification identity, never the secret. This lets a
/// user authorize one secret read while the collector checks account stability
/// afterward without opening a second password dialog.
pub fn version(service: &str) -> Result<Option<String>, &'static str> {
    if !valid_service(service) {
        return Err("unavailable");
    }
    #[cfg(any(test, feature = "test-credential-store"))]
    {
        Err("keychain_needs_permission")
    }
    #[cfg(all(target_os = "macos", not(any(test, feature = "test-credential-store"))))]
    {
        with_pending(service, || version_native(service))
    }
    #[cfg(all(
        not(target_os = "macos"),
        not(any(test, feature = "test-credential-store"))
    ))]
    {
        Ok(None)
    }
}

#[cfg(all(target_os = "macos", not(any(test, feature = "test-credential-store"))))]
#[path = "external_claude_keychain.rs"]
mod native;

#[cfg(all(target_os = "macos", not(any(test, feature = "test-credential-store"))))]
fn version_native(service: &str) -> Result<Option<String>, &'static str> {
    native::version(service)
}

#[cfg(any(test, all(target_os = "macos", not(feature = "test-credential-store"))))]
fn acl_allows(selector: u16, trusted_applications: Option<&[Option<bool>]>) -> bool {
    selector == 0
        && trusted_applications.is_none_or(|applications| applications.contains(&Some(true)))
}

// This gate is pure so tests can prove rejected/unknown ACL evidence never
// reaches a secret query. Interactive permission remains an explicit action.
#[cfg(any(test, all(target_os = "macos", not(feature = "test-credential-store"))))]
fn permitted_secret<T>(
    interactive: bool,
    preflight: impl FnOnce() -> bool,
    read: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    if !interactive && !preflight() {
        return Err("keychain_needs_permission");
    }
    read()
}

fn valid_service(service: &str) -> bool {
    service == PREFIX
        || service
            .strip_prefix(&format!("{PREFIX}-"))
            .is_some_and(|suffix| {
                suffix.len() == 8
                    && suffix
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
}

#[cfg(all(target_os = "macos", not(any(test, feature = "test-credential-store"))))]
fn read_native(service: &str, interactive: bool) -> Result<Option<Vec<u8>>, &'static str> {
    native::read(service, interactive)
}

#[cfg(all(
    not(target_os = "macos"),
    not(any(test, feature = "test-credential-store"))
))]
fn read_native(_service: &str, _interactive: bool) -> Result<Option<Vec<u8>>, &'static str> {
    let _ = MAX_BYTES;
    Ok(None)
}

/// NFC form used by Claude Code when naming a custom-profile Keychain item.
/// The exact input spelling is retained apart from Unicode normalization.
pub fn normalized_profile_directory(value: &str) -> Result<String, &'static str> {
    if value.len() > 32_768 || value.as_bytes().contains(&0) {
        return Err("unavailable");
    }
    #[cfg(target_os = "macos")]
    {
        normalization::nfc(value)
    }
    #[cfg(not(target_os = "macos"))]
    {
        // Other platforms do not read Claude's macOS Keychain.
        Ok(value.to_owned())
    }
}

#[cfg(target_os = "macos")]
mod normalization {
    use std::ffi::{c_char, c_void};
    type CFIndex = isize;
    const UTF8: u32 = 0x0800_0100;
    const NFC: isize = 2;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(
            allocator: *const c_void,
            bytes: *const u8,
            length: CFIndex,
            encoding: u32,
            external_representation: u8,
        ) -> *const c_void;
        fn CFStringCreateMutableCopy(
            allocator: *const c_void,
            capacity: CFIndex,
            value: *const c_void,
        ) -> *mut c_void;
        fn CFStringNormalize(value: *mut c_void, form: isize);
        fn CFStringGetLength(value: *const c_void) -> CFIndex;
        fn CFStringGetMaximumSizeForEncoding(length: CFIndex, encoding: u32) -> CFIndex;
        fn CFStringGetCString(
            value: *const c_void,
            output: *mut c_char,
            capacity: CFIndex,
            encoding: u32,
        ) -> u8;
        fn CFRelease(value: *const c_void);
    }

    struct OwnedString(*const c_void);
    impl Drop for OwnedString {
        fn drop(&mut self) {
            // SAFETY: every OwnedString is a non-null owned Create-rule value.
            unsafe { CFRelease(self.0) };
        }
    }

    pub(super) fn nfc(value: &str) -> Result<String, &'static str> {
        // SAFETY: UTF-8 bytes and length are valid for the call; CreateWithBytes
        // copies them. Null selects the default allocator, not a borrowed one.
        let source = unsafe {
            CFStringCreateWithBytes(
                std::ptr::null(),
                value.as_ptr(),
                value.len() as isize,
                UTF8,
                0,
            )
        };
        if source.is_null() {
            return Err("unavailable");
        }
        let source = OwnedString(source);
        // SAFETY: source remains alive and is a CFString from the Create call.
        let mutable = unsafe { CFStringCreateMutableCopy(std::ptr::null(), 0, source.0) };
        if mutable.is_null() {
            return Err("unavailable");
        }
        let owned = OwnedString(mutable);
        // SAFETY: mutable is an owned mutable CFString; form 2 is Apple's NFC.
        unsafe { CFStringNormalize(mutable, NFC) };
        // SAFETY: owned holds the normalized CFString through both size queries.
        let capacity =
            unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(owned.0), UTF8) };
        let capacity = usize::try_from(capacity)
            .ok()
            .and_then(|n| n.checked_add(1))
            .filter(|n| *n <= 131_073)
            .ok_or("unavailable")?;
        let mut bytes = vec![0u8; capacity];
        // SAFETY: output is writable for capacity bytes, including NUL, and the
        // source remains owned. Capacity fits CFIndex due to the bound above.
        let copied = unsafe {
            CFStringGetCString(owned.0, bytes.as_mut_ptr().cast(), capacity as isize, UTF8)
        };
        if copied == 0 {
            return Err("unavailable");
        }
        let length = bytes.iter().position(|b| *b == 0).ok_or("unavailable")?;
        bytes.truncate(length);
        String::from_utf8(bytes).map_err(|_| "unavailable")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_claude_keychain_is_never_read_by_tests() {
        assert_eq!(read(PREFIX, false), Err("keychain_needs_permission"));
        assert_eq!(read(PREFIX, true), Err("keychain_needs_permission"));
        assert_eq!(version(PREFIX), Err("keychain_needs_permission"));
        assert_eq!(read("another-app-service", true), Err("unavailable"));
    }

    #[test]
    fn pending_native_query_survives_waiter_timeout_and_coalesces_retries() {
        let service = "Claude Code-credentials-f1f2f3f4";
        let (started, ready) = std::sync::mpsc::channel();
        let (release, unblock) = std::sync::mpsc::channel();
        let (finished, complete) = std::sync::mpsc::channel();
        let job = std::thread::spawn(move || {
            let result = with_pending(service, || {
                started.send(()).unwrap();
                unblock.recv().unwrap();
                Ok(())
            });
            finished.send(()).unwrap();
            result
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(matches!(
            complete.recv_timeout(std::time::Duration::from_millis(10)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        // The UI's wait has ended, but the native operation is unanswered.
        assert_eq!(
            with_pending(service, || Ok(())),
            Err("keychain_read_pending")
        );
        assert_eq!(
            with_pending("Claude Code-credentials-f1f2f3f5", || Ok(())),
            Ok(())
        );
        release.send(()).unwrap();
        assert_eq!(job.join().unwrap(), Ok(()));
        assert_eq!(with_pending(service, || Ok(())), Ok(()));
    }

    #[test]
    fn decrypt_acl_requires_zero_prompt_selector_and_proven_running_signature() {
        assert!(acl_allows(0, None));
        assert!(!acl_allows(1, None));
        assert!(!acl_allows(0, Some(&[])));
        assert!(!acl_allows(0, Some(&[None, Some(false)])));
        assert!(!acl_allows(1, Some(&[Some(true)])));
        assert!(acl_allows(0, Some(&[None, Some(false), Some(true)])));
    }

    #[test]
    fn background_secret_query_is_never_attempted_without_proven_acl() {
        let secret_reads = std::cell::Cell::new(0);
        assert_eq!(
            permitted_secret(
                false,
                || false,
                || {
                    secret_reads.set(secret_reads.get() + 1);
                    Ok(())
                }
            ),
            Err("keychain_needs_permission")
        );
        assert_eq!(secret_reads.get(), 0);
        assert_eq!(
            permitted_secret(
                false,
                || true,
                || {
                    secret_reads.set(secret_reads.get() + 1);
                    Ok(())
                }
            ),
            Ok(())
        );
        assert_eq!(secret_reads.get(), 1);
        assert_eq!(
            permitted_secret(
                true,
                || panic!("interactive read must not require pre-approved ACL"),
                || Ok(())
            ),
            Ok(())
        );
    }

    #[test]
    fn external_claude_service_is_exactly_scoped() {
        assert!(valid_service(PREFIX));
        assert!(valid_service("Claude Code-credentials-5e4465c4"));
        assert!(!valid_service("Claude Code-credentials-5E4465C4"));
        assert!(!valid_service("Claude Code-credentials-5e4465c4-extra"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn external_claude_profile_normalization_preserves_path_spelling() {
        assert_eq!(
            normalized_profile_directory("/tmp/cafe\u{301}/").unwrap(),
            "/tmp/caf\u{e9}/"
        );
        assert_eq!(
            normalized_profile_directory("~/a/../profile/").unwrap(),
            "~/a/../profile/"
        );
        assert_eq!(
            normalized_profile_directory("bad\0path"),
            Err("unavailable")
        );
    }
}
