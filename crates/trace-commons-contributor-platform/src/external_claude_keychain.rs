//! Narrow macOS ABI boundary for no-secret ACL preflight. All pointers below
//! originate in retained Security/CoreFoundation results, checked before use.
//! The CodexBar preflight approach is credited in ../CODEXBAR-NOTICE.
use std::ffi::{c_char, c_void};

type Ref = *const c_void;
const UTF8: u32 = 0x0800_0100;
const MAX_ACLS: isize = 64;
const MAX_BYTES: isize = 1024 * 1024;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFTypeDictionaryKeyCallBacks: c_void;
    static kCFTypeDictionaryValueCallBacks: c_void;
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
    static kSecMatchLimit: Ref;
    static kSecMatchLimitOne: Ref;
    static kSecReturnAttributes: Ref;
    static kSecReturnRef: Ref;
    static kSecReturnData: Ref;
    static kSecValueRef: Ref;
    static kSecMatchItemList: Ref;
    static kSecUseAuthenticationUI: Ref;
    static kSecUseAuthenticationUIFail: Ref;
    static kSecACLAuthorizationDecrypt: Ref;
    fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
    fn SecKeychainItemGetTypeID() -> usize;
    fn SecAccessGetTypeID() -> usize;
    fn SecACLGetTypeID() -> usize;
    fn SecTrustedApplicationGetTypeID() -> usize;
    fn SecRequirementGetTypeID() -> usize;
    fn SecKeychainItemCopyAccess(item: Ref, access: *mut Ref) -> i32;
    fn SecAccessCopyMatchingACLList(access: Ref, authorization: Ref) -> Ref;
    // SecKeychainPromptSelector is uint16 in the public macOS SDK SecACL.h.
    fn SecACLCopyContents(
        acl: Ref,
        applications: *mut Ref,
        description: *mut Ref,
        selector: *mut u16,
    ) -> i32;
    fn SecCodeCopySelf(flags: u32, code: *mut Ref) -> i32;
    fn SecCodeCheckValidity(code: Ref, flags: u32, requirement: Ref) -> i32;
}
unsafe extern "C" {
    fn dlopen(path: *const c_char, flags: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    fn dlclose(handle: *mut c_void) -> i32;
}

struct Owned(Ref);
impl Owned {
    fn new(value: Ref) -> Result<Self, &'static str> {
        if value.is_null() {
            Err("keychain_needs_permission")
        } else {
            Ok(Self(value))
        }
    }
}
impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: Owned is constructed only from non-null Create/Copy results.
        unsafe { CFRelease(self.0) };
    }
}

fn string(value: &str) -> Result<Owned, &'static str> {
    // SAFETY: valid borrowed UTF-8 is copied by CoreFoundation; length fits.
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
    // SAFETY: key/value arrays are live for this call. CFType callbacks retain
    // their values; callback symbols are opaque addresses, never reinterpreted.
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
fn is_type(value: Ref, expected: usize) -> bool {
    !value.is_null() && unsafe { CFGetTypeID(value) == expected }
}

fn metadata(service: &str) -> Result<Option<Owned>, &'static str> {
    let service = string(service)?;
    // SAFETY: Security exports these immutable CFString/CFBoolean constants.
    // This query requests only attributes/reference and explicit UI-fail; it
    // never asks Security to decrypt the credential payload.
    let query = unsafe {
        dictionary(&[
            (kSecClass, kSecClassGenericPassword),
            (kSecAttrService, service.0),
            (kSecMatchLimit, kSecMatchLimitOne),
            (kSecReturnAttributes, kCFBooleanTrue),
            (kSecReturnRef, kCFBooleanTrue),
            (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
        ])?
    };
    let mut result = std::ptr::null();
    // SAFETY: query is an owned immutable dictionary; result receives a Copy.
    let status = unsafe { SecItemCopyMatching(query.0, &mut result) };
    if status == -25300 {
        return Ok(None);
    }
    if status != 0 {
        return Err("keychain_needs_permission");
    }
    let result = Owned::new(result)?;
    // SAFETY: result is a valid owned CF object and checked before dictionary use.
    if !is_type(result.0, unsafe { CFDictionaryGetTypeID() }) {
        return Err("keychain_needs_permission");
    }
    Ok(Some(result))
}

pub(super) fn read(service: &str, interactive: bool) -> Result<Option<Vec<u8>>, &'static str> {
    let Some(attributes) = metadata(service)? else {
        return Ok(None);
    };
    // SAFETY: metadata checked the dictionary type and retains its item reference.
    let item = unsafe { CFDictionaryGetValue(attributes.0, kSecValueRef) };
    if !is_type(item, unsafe { SecKeychainItemGetTypeID() }) {
        return Err("keychain_needs_permission");
    }
    super::permitted_secret(
        interactive,
        || decrypt_allowed(item),
        || {
            // Bind the data query to the exact item whose decrypt ACL was inspected.
            // SAFETY: item remains retained by attributes; CFArray copies/retains it.
            let items = Owned::new(unsafe {
                CFArrayCreate(std::ptr::null(), &item, 1, &raw const kCFTypeArrayCallBacks)
            })?;
            let mut entries = unsafe {
                vec![
                    (kSecClass, kSecClassGenericPassword),
                    (kSecMatchItemList, items.0),
                    (kSecMatchLimit, kSecMatchLimitOne),
                    (kSecReturnData, kCFBooleanTrue),
                ]
            };
            if !interactive {
                unsafe {
                    entries.push((kSecUseAuthenticationUI, kSecUseAuthenticationUIFail));
                }
            }
            let query = dictionary(&entries)?;
            let mut data = std::ptr::null();
            // SAFETY: owned query and valid Copy output. Interactive calls may show
            // the native dialog; callers run this on a bounded dedicated OS thread.
            if unsafe { SecItemCopyMatching(query.0, &mut data) } != 0 {
                return Err("keychain_needs_permission");
            }
            let data = Owned::new(data)?;
            if !is_type(data.0, unsafe { CFDataGetTypeID() }) {
                return Err("unavailable");
            }
            // SAFETY: checked CFData is retained through length and byte copying.
            let length = unsafe { CFDataGetLength(data.0) };
            if !(0..=MAX_BYTES).contains(&length) {
                return Err("unavailable");
            }
            if length == 0 {
                return Ok(Some(Vec::new()));
            }
            let bytes = unsafe { CFDataGetBytePtr(data.0) };
            if bytes.is_null() {
                return Err("unavailable");
            }
            Ok(Some(
                unsafe { std::slice::from_raw_parts(bytes, length as usize) }.to_vec(),
            ))
        },
    )
}

pub(super) fn version(service: &str) -> Result<Option<String>, &'static str> {
    let Some(attributes) = metadata(service)? else {
        return Ok(None);
    };
    fn date(attributes: Ref, key: &str) -> Result<u64, &'static str> {
        let key = string(key)?;
        // SAFETY: attributes came from metadata; dictionary retains date value.
        let value = unsafe { CFDictionaryGetValue(attributes, key.0) };
        if !is_type(value, unsafe { CFDateGetTypeID() }) {
            return Err("keychain_needs_permission");
        }
        // Exact native timestamp bits avoid rounding two writes to one second.
        Ok(unsafe { CFDateGetAbsoluteTime(value) }.to_bits())
    }
    let account_key = string("acct")?;
    let account = unsafe { CFDictionaryGetValue(attributes.0, account_key.0) };
    if !is_type(account, unsafe { CFStringGetTypeID() }) {
        return Err("keychain_needs_permission");
    }
    // SAFETY: checked CFString remains retained by attributes throughout copying.
    let capacity = unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(account), UTF8) };
    if !(0..=32_768).contains(&capacity) {
        return Err("keychain_needs_permission");
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
        return Err("keychain_needs_permission");
    }
    let end = bytes
        .iter()
        .position(|b| *b == 0)
        .ok_or("keychain_needs_permission")?;
    let account = std::str::from_utf8(&bytes[..end]).map_err(|_| "keychain_needs_permission")?;
    Ok(Some(format!(
        "{:016x}|{:016x}|{}",
        date(attributes.0, "cdat")?,
        date(attributes.0, "mdat")?,
        account
    )))
}

fn decrypt_allowed(item: Ref) -> bool {
    // SAFETY: caller checked and retained the SecKeychainItem. Every subsequent
    // Security result is Copy-owned and its runtime CF type is checked.
    unsafe { inspect_decrypt_acl(item) }.unwrap_or(false)
}

unsafe fn inspect_decrypt_acl(item: Ref) -> Option<bool> {
    let mut access = std::ptr::null();
    // SAFETY: valid item and initialized output; Copy ownership is held below.
    if unsafe { SecKeychainItemCopyAccess(item, &mut access) } != 0 {
        return None;
    }
    let access = Owned::new(access).ok()?;
    if !is_type(access.0, unsafe { SecAccessGetTypeID() }) {
        return None;
    }
    let acls =
        Owned::new(unsafe { SecAccessCopyMatchingACLList(access.0, kSecACLAuthorizationDecrypt) })
            .ok()?;
    if !is_type(acls.0, unsafe { CFArrayGetTypeID() }) {
        return None;
    }
    let count = unsafe { CFArrayGetCount(acls.0) };
    if !(1..=MAX_ACLS).contains(&count) {
        return None;
    }
    // A stored requirement must validate the running process, not merely the
    // file currently at its path (which can change during an app update).
    let mut code = std::ptr::null();
    if unsafe { SecCodeCopySelf(0, &mut code) } != 0 {
        return None;
    }
    let code = Owned::new(code).ok()?;
    for index in 0..count {
        let acl = unsafe { CFArrayGetValueAtIndex(acls.0, index) };
        if !is_type(acl, unsafe { SecACLGetTypeID() }) {
            return None;
        }
        let mut apps = std::ptr::null();
        let mut description = std::ptr::null();
        let mut selector = 0u16;
        let status = unsafe { SecACLCopyContents(acl, &mut apps, &mut description, &mut selector) };
        let applications = (!apps.is_null()).then(|| Owned(apps));
        let _description = (!description.is_null()).then(|| Owned(description));
        if status != 0 {
            continue;
        }
        if selector != 0 {
            continue;
        }
        let Some(applications) = applications else {
            return Some(super::acl_allows(selector, None));
        };
        if !is_type(applications.0, unsafe { CFArrayGetTypeID() }) {
            return None;
        }
        let application_count = unsafe { CFArrayGetCount(applications.0) };
        if !(0..=MAX_ACLS).contains(&application_count) {
            return None;
        }
        let mut validations = Vec::new();
        for app_index in 0..application_count {
            let app = unsafe { CFArrayGetValueAtIndex(applications.0, app_index) };
            if !is_type(app, unsafe { SecTrustedApplicationGetTypeID() }) {
                return None;
            }
            validations.push(unsafe { running_process_matches(app, code.0) });
        }
        if super::acl_allows(selector, Some(&validations)) {
            return Some(true);
        }
    }
    Some(false)
}

unsafe fn running_process_matches(application: Ref, code: Ref) -> Option<bool> {
    type CopyRequirement = unsafe extern "C" fn(Ref, *mut Ref) -> i32;
    // This optional legacy Security symbol is the same ABI used by CodexBar.
    // It supplies the ACL's stored requirement; missing SPI fails closed.
    let handle = unsafe {
        dlopen(
            c"/System/Library/Frameworks/Security.framework/Security".as_ptr(),
            2,
        )
    };
    if handle.is_null() {
        return None;
    }
    struct Library(*mut c_void);
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe {
                dlclose(self.0);
            }
        }
    }
    let library = Library(handle);
    let symbol = unsafe { dlsym(library.0, c"SecTrustedApplicationCopyRequirement".as_ptr()) };
    if symbol.is_null() {
        return None;
    }
    // SAFETY: symbol name and function ABI match the Security entry point;
    // library remains loaded through invocation, and no Rust wrapper is cast.
    let copy: CopyRequirement = unsafe { std::mem::transmute(symbol) };
    let mut requirement = std::ptr::null();
    if unsafe { copy(application, &mut requirement) } != 0 {
        return None;
    }
    let requirement = Owned::new(requirement).ok()?;
    if !is_type(requirement.0, unsafe { SecRequirementGetTypeID() }) {
        return None;
    }
    Some(unsafe { SecCodeCheckValidity(code, 0, requirement.0) } == 0)
}
