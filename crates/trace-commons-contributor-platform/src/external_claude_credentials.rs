//! Read-only access to the exact Claude Code credential selected by its owner.
//! A secret read first runs with authentication UI suppressed on that one
//! query; only the caller decides whether a user action justifies a second,
//! UI-permitted read. Metadata queries never ask for credential data and
//! always suppress authentication UI. No operation changes Keychain ACLs or
//! the process-global interaction policy.

/// Transient credential payload. Debug output is redacted and dropping the
/// buffer overwrites its initialized bytes, including after a canceled waiter.
pub struct SecretBytes(Vec<u8>);

impl SecretBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }
}
impl std::fmt::Debug for SecretBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretBytes([redacted])")
    }
}
impl std::ops::Deref for SecretBytes {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl AsRef<[u8]> for SecretBytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}
impl Drop for SecretBytes {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Best-effort erasure of an owned initialized buffer, without a new dependency.
/// Does not claim to erase allocator copies or native framework allocations.
pub fn wipe(bytes: &mut [u8]) {
    for byte in bytes {
        // SAFETY: each byte is a valid exclusive mutable reference. Volatile
        // writes prevent dead-store elimination when the allocation is freed.
        unsafe {
            std::ptr::write_volatile(byte, 0);
        }
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

const PREFIX: &str = "Claude Code-credentials";

/// The error a UI-suppressed read returns when only a dialog could grant
/// access (errSecInteractionNotAllowed). A Deny, failed authentication or a
/// missing ACL entry is `keychain_needs_permission` instead, and must not be
/// retried with UI.
pub const INTERACTION_REQUIRED: &str = "keychain_interaction_required";

/// Read a selected service. With `allow_ui` false the Security query itself
/// refuses to show a dialog, so this never prompts; with it true macOS may
/// ask, and the caller must hold an explicit user action for that.
/// The caller must deserialize only needed fields and never log these bytes.
/// Security may wait for a user dialog; use a leased dedicated OS thread.
pub fn read(service: &str, allow_ui: bool) -> Result<Option<SecretBytes>, &'static str> {
    if !valid_service(service) {
        return Err("unavailable");
    }
    #[cfg(any(test, feature = "test-credential-store"))]
    {
        let _ = allow_ui;
        Err("keychain_needs_permission")
    }
    #[cfg(all(target_os = "macos", not(any(test, feature = "test-credential-store"))))]
    {
        native::read(service, allow_ui).map(|bytes| bytes.map(SecretBytes::new))
    }
    #[cfg(all(
        not(target_os = "macos"),
        not(any(test, feature = "test-credential-store"))
    ))]
    {
        let _ = allow_ui;
        Ok(None)
    }
}

/// An item's attribute-only revision, in two parts. `item` names the item
/// itself (its persistent reference, creation date and account), so it
/// changes when the item is deleted and recreated, as a Claude Code logout
/// and login do. `content` changes on any write, including an in-place
/// update that keeps the item. Both are opaque equality discriminators that
/// carry no account name or reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRevision {
    pub item: String,
    pub content: String,
}

/// Item revision obtained without decrypting credentials. An error means
/// unknown freshness, never permission to reuse a previous identity. OS calls
/// can stall even without UI: invoke on a leased OS thread, not a status path.
pub fn version(service: &str) -> Result<Option<ItemRevision>, &'static str> {
    if !valid_service(service) {
        return Err("unavailable");
    }
    #[cfg(any(test, feature = "test-credential-store"))]
    {
        Err("keychain_needs_permission")
    }
    #[cfg(all(target_os = "macos", not(any(test, feature = "test-credential-store"))))]
    {
        native::version(service)
    }
    #[cfg(all(
        not(target_os = "macos"),
        not(any(test, feature = "test-credential-store"))
    ))]
    {
        Ok(None)
    }
}

fn valid_service(service: &str) -> bool {
    service == PREFIX
        || service
            .strip_prefix("Claude Code-credentials-")
            .is_some_and(|suffix| {
                suffix.len() == 8
                    && suffix
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
}

#[cfg(all(target_os = "macos", any(test, not(feature = "test-credential-store"))))]
#[path = "external_claude_keychain.rs"]
mod native;

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
    fn secret_debug_redacts_and_wipe_clears_bytes() {
        let secret = SecretBytes::new(b"fixture-sensitive-token".to_vec());
        assert_eq!(format!("{secret:?}"), "SecretBytes([redacted])");
        let mut bytes = vec![1, 2, 3];
        wipe(&mut bytes);
        assert_eq!(bytes, [0, 0, 0]);
    }
    #[test]
    fn tests_cannot_access_real_keychain() {
        assert!(matches!(
            read(PREFIX, true),
            Err("keychain_needs_permission")
        ));
        assert!(matches!(
            read(PREFIX, false),
            Err("keychain_needs_permission")
        ));
        assert_eq!(version(PREFIX), Err("keychain_needs_permission"));
    }
    #[test]
    fn accepts_only_exact_claude_services() {
        assert!(valid_service(PREFIX));
        assert!(valid_service("Claude Code-credentials-5e4465c4"));
        for service in [
            "another-service",
            "Claude Code-credentials-5E4465C4",
            "Claude Code-credentials-5e4465c4-extra",
        ] {
            assert!(!valid_service(service));
            assert!(matches!(read(service, true), Err("unavailable")));
            assert_eq!(version(service), Err("unavailable"));
        }
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_nfc_preserves_exact_profile_spelling() {
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
