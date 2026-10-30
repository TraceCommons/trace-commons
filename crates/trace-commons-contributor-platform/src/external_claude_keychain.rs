//! Narrow Security/CoreFoundation ABI for explicit reads and no-data revisions.
//! The attributes-only query pattern is credited in ../CODEXBAR-NOTICE.
use std::ffi::{c_char, c_void};

type Ref = *const c_void;
const UTF8: u32 = 0x0800_0100;
const MAX_BYTES: isize = 1024 * 1024;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
    #[cfg(test)]
    static kCFTypeArrayCallBacks: c_void;
    static kCFBooleanTrue: Ref;
    fn CFRelease(value: Ref);
    fn CFGetTypeID(value: Ref) -> usize;
    fn CFDictionaryGetTypeID() -> usize;
    fn CFArrayGetTypeID() -> usize;
    fn CFDataGetTypeID() -> usize;
    fn CFDateGetTypeID() -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFDictionaryCreate(
        allocator: Ref,
        keys: *const Ref,
        values: *const Ref,
        count: isize,
        key_callbacks: Ref,
        value_callbacks: Ref,
    ) -> Ref;
    fn CFDictionaryGetValue(dictionary: Ref, key: Ref) -> Ref;
    #[cfg(test)]
    fn CFArrayCreate(allocator: Ref, values: *const Ref, count: isize, callbacks: Ref) -> Ref;
    fn CFArrayGetCount(array: Ref) -> isize;
    fn CFArrayGetValueAtIndex(array: Ref, index: isize) -> Ref;
    fn CFStringCreateWithBytes(
        allocator: Ref,
        bytes: *const u8,
        length: isize,
        encoding: u32,
        external: u8,
    ) -> Ref;
    fn CFStringGetLength(value: Ref) -> isize;
    fn CFStringGetMaximumSizeForEncoding(length: isize, encoding: u32) -> isize;
    fn CFStringGetCString(value: Ref, output: *mut c_char, capacity: isize, encoding: u32) -> u8;
    fn CFDateGetAbsoluteTime(value: Ref) -> f64;
    fn CFDataGetLength(value: Ref) -> isize;
    fn CFDataGetBytePtr(value: Ref) -> *const u8;
}
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: Ref;
    static kSecClassGenericPassword: Ref;
    static kSecAttrService: Ref;
    static kSecAttrAccount: Ref;
    static kSecAttrCreationDate: Ref;
    static kSecAttrModificationDate: Ref;
    static kSecMatchLimit: Ref;
    static kSecMatchLimitAll: Ref;
    static kSecMatchLimitOne: Ref;
    static kSecReturnAttributes: Ref;
    static kSecReturnPersistentRef: Ref;
    static kSecValuePersistentRef: Ref;
    static kSecReturnData: Ref;
    static kSecMatchSearchList: Ref;
    static kSecUseAuthenticationUI: Ref;
    static kSecUseAuthenticationUIFail: Ref;
    fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
}

struct Owned(Ref);
impl Owned {
    fn new(value: Ref) -> Result<Self, &'static str> {
        if value.is_null() {
            Err("unavailable")
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: Owned contains a non-null Create/Copy result.
        unsafe { CFRelease(self.0) };
    }
}
fn string(value: &str) -> Result<Owned, &'static str> {
    // SAFETY: this bounded UTF-8 string is copied for the returned CFString.
    Owned::new(unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            value.as_ptr(),
            value.len() as isize,
            UTF8,
            0,
        )
    })
}
fn dictionary(entries: &[(Ref, Ref)]) -> Result<Owned, &'static str> {
    let keys: Vec<_> = entries.iter().map(|entry| entry.0).collect();
    let values: Vec<_> = entries.iter().map(|entry| entry.1).collect();
    // SAFETY: arrays remain live for this call; CF callbacks retain their values.
    Owned::new(unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            entries.len() as isize,
            &raw const kCFTypeDictionaryKeyCallBacks,
            &raw const kCFTypeDictionaryValueCallBacks,
        )
    })
}
#[cfg(test)]
fn array(value: Ref) -> Result<Owned, &'static str> {
    // SAFETY: callers supply a live CF object; CF callbacks retain it.
    Owned::new(unsafe {
        CFArrayCreate(
            std::ptr::null(),
            &value,
            1,
            &raw const kCFTypeArrayCallBacks,
        )
    })
}
fn is_type(value: Ref, expected: usize) -> bool {
    // SAFETY: values originate in retained CF objects or exported constants.
    !value.is_null() && unsafe { CFGetTypeID(value) == expected }
}

#[derive(Clone, Copy)]
enum Query {
    Metadata,
    /// A payload read with authentication UI suppressed on this one query.
    SilentSecret,
    ExplicitSecret,
}
fn query_entries(kind: Query, selector: Ref, search_list: Option<Ref>) -> Vec<(Ref, Ref)> {
    // SAFETY: all constants are immutable CF values exported by Security.
    let mut entries = unsafe {
        match kind {
            Query::Metadata => vec![
                (kSecClass, kSecClassGenericPassword),
                (kSecAttrService, selector),
                (kSecMatchLimit, kSecMatchLimitAll),
                (kSecReturnAttributes, kCFBooleanTrue),
                (kSecReturnPersistentRef, kCFBooleanTrue),
                (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
            ],
            Query::SilentSecret => vec![
                (kSecClass, kSecClassGenericPassword),
                (kSecValuePersistentRef, selector),
                (kSecMatchLimit, kSecMatchLimitOne),
                (kSecReturnData, kCFBooleanTrue),
                (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
            ],
            Query::ExplicitSecret => vec![
                (kSecClass, kSecClassGenericPassword),
                (kSecValuePersistentRef, selector),
                (kSecMatchLimit, kSecMatchLimitOne),
                (kSecReturnData, kCFBooleanTrue),
            ],
        }
    };
    if let Some(list) = search_list {
        // SAFETY: list is retained by caller for the lifetime of the query.
        entries.push((unsafe { kSecMatchSearchList }, list));
    }
    entries
}
fn execute(query: &Owned) -> Result<Option<Owned>, &'static str> {
    let mut result = std::ptr::null();
    // SAFETY: owned dictionary, initialized Copy output. Only ExplicitSecret
    // permits native UI. No global interaction setting changes.
    let status = unsafe { SecItemCopyMatching(query.0, &mut result) };
    let result = (!result.is_null()).then(|| Owned(result));
    match status {
        0 => result.map(Some).ok_or("unavailable"),
        -25300 => Ok(None),
        // errSecInteractionNotAllowed: a UI-suppressed query that would have
        // needed a dialog. The only status that justifies asking with UI.
        -25308 => Err(INTERACTION_REQUIRED),
        // User cancelled (Deny), authentication failed, no access: never
        // retried with UI, so a Deny is not followed by another dialog.
        -128 | -25293 | -25243 => Err("keychain_needs_permission"),
        _ => Err("unavailable"),
    }
}
struct Metadata {
    rows: Owned,
}
impl Metadata {
    fn row(&self) -> Ref {
        // SAFETY: metadata() validates a one-element array of dictionaries.
        unsafe { CFArrayGetValueAtIndex(self.rows.0, 0) }
    }
    fn value(&self, key: Ref) -> Ref {
        // SAFETY: retained dictionary, immutable exported key.
        unsafe { CFDictionaryGetValue(self.row(), key) }
    }
    fn persistent_ref(&self) -> Result<Ref, &'static str> {
        let value = self.value(unsafe { kSecValuePersistentRef });
        if is_type(value, unsafe { CFDataGetTypeID() }) {
            Ok(value)
        } else {
            Err("unavailable")
        }
    }
    fn revision(&self) -> Result<String, &'static str> {
        let mut material = data(self.persistent_ref()?)?;
        for key in unsafe { [kSecAttrCreationDate, kSecAttrModificationDate] } {
            let value = self.value(key);
            if !is_type(value, unsafe { CFDateGetTypeID() }) {
                return Err("unavailable");
            }
            // Exact f64 bits preserve subsecond changes; no timestamp rounding.
            material.extend_from_slice(
                &unsafe { CFDateGetAbsoluteTime(value) }
                    .to_bits()
                    .to_be_bytes(),
            );
        }
        let account = self.value(unsafe { kSecAttrAccount });
        if !is_type(account, unsafe { CFStringGetTypeID() }) {
            return Err("unavailable");
        }
        // SAFETY: validated CFString retained by rows throughout copying.
        let capacity =
            unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(account), UTF8) };
        if !(0..=32768).contains(&capacity) {
            return Err("unavailable");
        }
        let mut bytes = vec![0u8; capacity as usize + 1];
        if unsafe {
            CFStringGetCString(
                account,
                bytes.as_mut_ptr().cast(),
                bytes.len() as isize,
                UTF8,
            )
        } == 0
        {
            return Err("unavailable");
        }
        let end = bytes.iter().position(|b| *b == 0).ok_or("unavailable")?;
        material.extend_from_slice(&bytes[..end]);
        // Revision contains no raw account names or persistent references. This
        // is an equality discriminator, not a security digest or account ID.
        Ok(uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &material).to_string())
    }
}
fn metadata(service: &str, search_list: Option<Ref>) -> Result<Option<Metadata>, &'static str> {
    let service = string(service)?;
    let query = dictionary(&query_entries(Query::Metadata, service.0, search_list))?;
    let Some(rows) = execute(&query)? else {
        return Ok(None);
    };
    if !is_type(rows.0, unsafe { CFArrayGetTypeID() }) {
        return Err("unavailable");
    }
    // Never guess among duplicate service accounts or silently cross profiles.
    if unsafe { CFArrayGetCount(rows.0) } != 1 {
        return Err("unavailable");
    }
    let row = unsafe { CFArrayGetValueAtIndex(rows.0, 0) };
    if !is_type(row, unsafe { CFDictionaryGetTypeID() }) {
        return Err("unavailable");
    }
    Ok(Some(Metadata { rows }))
}
fn data(value: Ref) -> Result<Vec<u8>, &'static str> {
    if !is_type(value, unsafe { CFDataGetTypeID() }) {
        return Err("unavailable");
    }
    // SAFETY: validated CFData retained by the caller through the copy.
    let length = unsafe { CFDataGetLength(value) };
    if !(0..=MAX_BYTES).contains(&length) {
        return Err("unavailable");
    }
    if length == 0 {
        return Ok(Vec::new());
    }
    let bytes = unsafe { CFDataGetBytePtr(value) };
    if bytes.is_null() {
        return Err("unavailable");
    }
    Ok(unsafe { std::slice::from_raw_parts(bytes, length as usize) }.to_vec())
}

// errSecInteractionNotAllowed. Callers above this module see it only from a
// silent read; everywhere else it is a permission need.
use super::INTERACTION_REQUIRED;

#[cfg(not(test))]
pub(super) fn version(service: &str) -> Result<Option<String>, &'static str> {
    metadata(service, None)
        .map_err(permission_label)?
        .map(|m| m.revision())
        .transpose()
}
#[cfg(not(test))]
pub(super) fn read(service: &str, allow_ui: bool) -> Result<Option<Vec<u8>>, &'static str> {
    read_in(service, None, allow_ui)
}
fn permission_label(reason: &'static str) -> &'static str {
    if reason == INTERACTION_REQUIRED {
        "keychain_needs_permission"
    } else {
        reason
    }
}
fn read_in(
    service: &str,
    search_list: Option<Ref>,
    allow_ui: bool,
) -> Result<Option<Vec<u8>>, &'static str> {
    // A locked keychain can refuse even the attribute query. On a silent
    // read that is an interaction need; with UI allowed it is final.
    let item = metadata(service, search_list).map_err(|reason| {
        if allow_ui {
            permission_label(reason)
        } else {
            reason
        }
    })?;
    let Some(item) = item else {
        return Ok(None);
    };
    let before = item.revision()?;
    let query = dictionary(&query_entries(
        if allow_ui {
            Query::ExplicitSecret
        } else {
            Query::SilentSecret
        },
        item.persistent_ref()?,
        search_list,
    ))?;
    let Some(result) = execute(&query)? else {
        return Ok(None);
    };
    let after = metadata(service, search_list)
        .map_err(permission_label)?
        .map(|m| m.revision())
        .transpose()?;
    if after.as_deref() != Some(before.as_str()) {
        return Err("credential_changed");
    }
    data(result.0).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_query_cannot_request_payload_or_interactive_authentication() {
        let service = string("Claude Code-credentials-fixture").unwrap();
        let entries = query_entries(Query::Metadata, service.0, None);
        unsafe {
            assert!(!entries.iter().any(|(key, _)| *key == kSecReturnData));
            assert!(entries.contains(&(kSecUseAuthenticationUI, kSecUseAuthenticationUIFail)));
            assert!(entries.contains(&(kSecAttrService, service.0)));
            let secret = query_entries(Query::ExplicitSecret, service.0, None);
            assert!(secret.contains(&(kSecReturnData, kCFBooleanTrue)));
            assert!(!secret.iter().any(|(key, _)| *key == kSecAttrService));
            assert!(
                !secret
                    .iter()
                    .any(|(key, _)| *key == kSecUseAuthenticationUI)
            );
        }
    }

    #[test]
    fn silent_secret_query_suppresses_authentication_ui() {
        let service = string("Claude Code-credentials-fixture").unwrap();
        let entries = query_entries(Query::SilentSecret, service.0, None);
        unsafe {
            assert!(entries.contains(&(kSecReturnData, kCFBooleanTrue)));
            assert!(entries.contains(&(kSecUseAuthenticationUI, kSecUseAuthenticationUIFail)));
            assert!(entries.contains(&(kSecValuePersistentRef, service.0)));
        }
    }

    /// Uses only a disposable keychain, explicitly passed in every query. Does
    /// not alter the user's search list, default keychain, or login keychain.
    #[test]
    #[ignore = "native isolated Keychain fixture"]
    fn isolated_keychain_metadata_revision_and_locked_no_ui() {
        #[link(name = "Security", kind = "framework")]
        unsafe extern "C" {
            fn SecKeychainCreate(
                path: *const c_char,
                password_length: u32,
                password: Ref,
                prompt: u8,
                access: Ref,
                keychain: *mut Ref,
            ) -> i32;
            fn SecKeychainLock(keychain: Ref) -> i32;
            fn SecKeychainUnlock(
                keychain: Ref,
                length: u32,
                password: Ref,
                use_password: u8,
            ) -> i32;
            fn SecKeychainDelete(keychain: Ref) -> i32;
            fn SecKeychainAddGenericPassword(
                keychain: Ref,
                service_length: u32,
                service: *const u8,
                account_length: u32,
                account: *const u8,
                password_length: u32,
                password: Ref,
                item: *mut Ref,
            ) -> i32;
            fn SecKeychainItemModifyAttributesAndData(
                item: Ref,
                attributes: Ref,
                length: u32,
                data: Ref,
            ) -> i32;
            fn SecKeychainCopySearchList(result: *mut Ref) -> i32;
        }
        #[link(name = "CoreFoundation", kind = "framework")]
        unsafe extern "C" {
            fn CFEqual(left: Ref, right: Ref) -> u8;
        }
        fn search_list() -> Owned {
            let mut list = std::ptr::null();
            assert_eq!(unsafe { SecKeychainCopySearchList(&mut list) }, 0);
            Owned::new(list).unwrap()
        }
        let original_search_list = search_list();
        let temp = tempfile::tempdir().unwrap();
        let path =
            std::ffi::CString::new(temp.path().join("fixture.keychain").to_str().unwrap()).unwrap();
        let password = b"isolated-fixture-password";
        let mut keychain = std::ptr::null();
        // Apple's StorageManager::shouldAddToSearchList explicitly excludes
        // private keychains; this fixture is neither login nor System.
        assert_eq!(
            unsafe {
                SecKeychainCreate(
                    path.as_ptr(),
                    password.len() as u32,
                    password.as_ptr().cast(),
                    0,
                    std::ptr::null(),
                    &mut keychain,
                )
            },
            0
        );
        struct Cleanup(Owned);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                unsafe {
                    SecKeychainDelete(self.0.0);
                }
            }
        }
        let keychain = Cleanup(Owned::new(keychain).unwrap());
        let list = array(keychain.0.0).unwrap();
        assert_ne!(
            unsafe { CFEqual(original_search_list.0, search_list().0) },
            0
        );
        let service = "TraceCommons-Disposable-Claude-Fixture";
        let account = b"fixture-account";
        let payload = b"synthetic-payload-1";
        let mut item = std::ptr::null();
        assert_eq!(
            unsafe {
                SecKeychainAddGenericPassword(
                    keychain.0.0,
                    service.len() as u32,
                    service.as_ptr(),
                    account.len() as u32,
                    account.as_ptr(),
                    payload.len() as u32,
                    payload.as_ptr().cast(),
                    &mut item,
                )
            },
            0
        );
        let item = Owned::new(item).unwrap();
        let before = metadata(service, Some(list.0))
            .unwrap()
            .unwrap()
            .revision()
            .unwrap();
        assert_eq!(
            read_in(service, Some(list.0), true).unwrap().unwrap(),
            b"synthetic-payload-1"
        );
        // The native store's modification clock may have one-second precision.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let payload = b"synthetic-payload-2";
        assert_eq!(
            unsafe {
                SecKeychainItemModifyAttributesAndData(
                    item.0,
                    std::ptr::null(),
                    payload.len() as u32,
                    payload.as_ptr().cast(),
                )
            },
            0
        );
        let after = metadata(service, Some(list.0))
            .unwrap()
            .unwrap()
            .revision()
            .unwrap();
        assert_ne!(before, after);
        assert!(
            metadata("different-exact-profile", Some(list.0))
                .unwrap()
                .is_none()
        );
        assert_eq!(unsafe { SecKeychainLock(keychain.0.0) }, 0);
        let started = std::time::Instant::now();
        let locked = metadata(service, Some(list.0));
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        // A locked store may expose attributes or report interaction required;
        // neither outcome implies that decrypting its secret is authorized.
        assert!(locked.is_ok() || matches!(locked, Err(INTERACTION_REQUIRED)));
        assert_eq!(
            unsafe {
                SecKeychainUnlock(
                    keychain.0.0,
                    password.len() as u32,
                    password.as_ptr().cast(),
                    1,
                )
            },
            0
        );
        drop(item);
        drop(list);
        drop(keychain);
        assert_ne!(
            unsafe { CFEqual(original_search_list.0, search_list().0) },
            0
        );
    }
}
