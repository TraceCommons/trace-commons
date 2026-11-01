//! Narrow Security/CoreFoundation ABI for explicit reads and no-data revisions.
//!
//! Every query that must not show a dialog carries both
//! `kSecUseAuthenticationUIFail` and an `LAContext` whose
//! `interactionNotAllowed` is set (`kSecUseAuthenticationContext`): the UI-fail
//! policy alone has been seen to let the legacy Allow/Deny prompt through.
//! Before a payload read without UI, the item's decrypt access list is
//! checked against this process's own code signature, and the read is skipped
//! unless some entry admits this process without a prompt.
//!
//! The attributes-only query, no-UI context and access-list preflight
//! patterns are credited in ../CODEXBAR-NOTICE.
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
    static kSecUseAuthenticationContext: Ref;
    static kSecReturnRef: Ref;
    static kSecACLAuthorizationDecrypt: Ref;
    fn SecItemCopyMatching(query: Ref, result: *mut Ref) -> i32;
    fn SecKeychainItemGetTypeID() -> usize;
    fn SecKeychainItemCopyAccess(item: Ref, access: *mut Ref) -> i32;
    fn SecAccessCopyMatchingACLList(access: Ref, authorization: Ref) -> Ref;
    fn SecACLCopyContents(
        acl: Ref,
        applications: *mut Ref,
        description: *mut Ref,
        prompt_selector: *mut u16,
    ) -> i32;
    fn SecCodeCopySelf(flags: u32, code: *mut Ref) -> i32;
    fn SecCodeCheckValidity(code: Ref, flags: u32, requirement: Ref) -> i32;
}
#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Ref;
    fn sel_registerName(name: *const c_char) -> Ref;
    // Called only through a transmute to the exact signature of each message.
    fn objc_msgSend();
}
unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}
const RTLD_NOW: i32 = 0x2;
/// errSecCSReqFailed: the code ran its check to completion and does not
/// satisfy the requirement.
const REQUIREMENT_FAILED: i32 = -67050;

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
/// An `LAContext` with `interactionNotAllowed` set, released on drop. The
/// query dictionary retains it for as long as the query lives.
struct NoUiContext(Ref);
impl NoUiContext {
    fn new() -> Result<Self, &'static str> {
        use std::sync::OnceLock;
        static LOADED: OnceLock<bool> = OnceLock::new();
        let loaded = *LOADED.get_or_init(|| {
            // SAFETY: a constant system framework path; the handle is kept
            // for the life of the process.
            !unsafe {
                dlopen(
                    c"/System/Library/Frameworks/LocalAuthentication.framework/LocalAuthentication"
                        .as_ptr(),
                    RTLD_NOW,
                )
            }
            .is_null()
        });
        if !loaded {
            return Err("unavailable");
        }
        // SAFETY: a constant, NUL-terminated class name.
        let class = unsafe { objc_getClass(c"LAContext".as_ptr()) };
        if class.is_null() {
            return Err("unavailable");
        }
        // SAFETY: `+new` takes no arguments and returns a +1 object or nil.
        let new: unsafe extern "C" fn(Ref, Ref) -> Ref =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        let context = unsafe { new(class, sel_registerName(c"new".as_ptr())) };
        if context.is_null() {
            return Err("unavailable");
        }
        let context = Self(context);
        // SAFETY: `-setInteractionNotAllowed:` takes one BOOL and returns void.
        let set: unsafe extern "C" fn(Ref, Ref, bool) =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe {
            set(
                context.0,
                sel_registerName(c"setInteractionNotAllowed:".as_ptr()),
                true,
            )
        };
        Ok(context)
    }
    #[cfg(test)]
    fn interaction_not_allowed(&self) -> bool {
        // SAFETY: the getter takes no arguments and returns a BOOL.
        let get: unsafe extern "C" fn(Ref, Ref) -> bool =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { get(self.0, sel_registerName(c"interactionNotAllowed".as_ptr())) }
    }
}
impl Drop for NoUiContext {
    fn drop(&mut self) {
        // SAFETY: `-release` balances the +1 from `+new`.
        let release: unsafe extern "C" fn(Ref, Ref) =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { release(self.0, sel_registerName(c"release".as_ptr())) };
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
    /// The item's reference only, for inspecting its access list. Never
    /// asks for the payload.
    ItemRef,
    /// A payload read with authentication UI suppressed on this one query.
    SilentSecret,
    ExplicitSecret,
}
/// `context` is the no-UI `LAContext`; every kind but `ExplicitSecret`
/// carries it, and `ExplicitSecret` ignores it.
fn query_entries(
    kind: Query,
    selector: Ref,
    search_list: Option<Ref>,
    context: Ref,
) -> Vec<(Ref, Ref)> {
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
                (kSecUseAuthenticationContext, context),
            ],
            Query::ItemRef => vec![
                (kSecClass, kSecClassGenericPassword),
                (kSecValuePersistentRef, selector),
                (kSecMatchLimit, kSecMatchLimitOne),
                (kSecReturnRef, kCFBooleanTrue),
                (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
                (kSecUseAuthenticationContext, context),
            ],
            Query::SilentSecret => vec![
                (kSecClass, kSecClassGenericPassword),
                (kSecValuePersistentRef, selector),
                (kSecMatchLimit, kSecMatchLimitOne),
                (kSecReturnData, kCFBooleanTrue),
                (kSecUseAuthenticationUI, kSecUseAuthenticationUIFail),
                (kSecUseAuthenticationContext, context),
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
/// Build a query. A kind that must not show UI fails closed when its no-UI
/// context cannot be made, rather than running without one.
fn query(kind: Query, selector: Ref, search_list: Option<Ref>) -> Result<Owned, &'static str> {
    let context = match kind {
        Query::ExplicitSecret => None,
        _ => Some(NoUiContext::new()?),
    };
    let context_ref = context.as_ref().map_or(std::ptr::null(), |c| c.0);
    // The dictionary retains the context; ours is released on return.
    dictionary(&query_entries(kind, selector, search_list, context_ref))
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
    let query = query(Query::Metadata, service.0, search_list)?;
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

/// What an item's decrypt access list says about this process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Access {
    /// Some entry admits this process without a prompt.
    Allowed,
    /// Every entry was inspected and none admits it.
    Rejected,
    /// The list could not be fully inspected.
    Indeterminate,
}

/// One decrypt entry. `applications` is `None` when the entry names no
/// applications (any caller), otherwise each trusted application's check of
/// this process: `Some(0)` satisfied, `Some(REQUIREMENT_FAILED)` checked and
/// not satisfied, anything else unknown.
fn entry_access(prompt_selector: u16, applications: Option<&[Option<i32>]>) -> Access {
    // Any prompt condition may apply to this process; a background read
    // cannot prove it does not.
    if prompt_selector != 0 {
        return Access::Rejected;
    }
    let Some(applications) = applications else {
        return Access::Allowed;
    };
    if applications.contains(&Some(0)) {
        Access::Allowed
    } else if applications
        .iter()
        .all(|status| *status == Some(REQUIREMENT_FAILED))
    {
        Access::Rejected
    } else {
        Access::Indeterminate
    }
}

/// The whole list: allowed if any entry allows, rejected only if every entry
/// was inspected and rejects. An empty list proves nothing.
fn list_access(entries: impl IntoIterator<Item = Access>) -> Access {
    let mut seen = false;
    let mut incomplete = false;
    for entry in entries {
        seen = true;
        match entry {
            Access::Allowed => return Access::Allowed,
            Access::Indeterminate => incomplete = true,
            Access::Rejected => {}
        }
    }
    if !seen || incomplete {
        Access::Indeterminate
    } else {
        Access::Rejected
    }
}

/// `SecTrustedApplicationCopyRequirement`, which Security exports without a
/// public header. Resolved at run time so its absence means "cannot tell".
type CopyRequirement = unsafe extern "C" fn(Ref, *mut Ref) -> i32;
fn copy_requirement() -> Option<CopyRequirement> {
    // SAFETY: a constant system framework path and symbol name. The handle
    // is never closed, so the function pointer stays valid.
    let symbol = unsafe {
        let handle = dlopen(
            c"/System/Library/Frameworks/Security.framework/Security".as_ptr(),
            RTLD_NOW,
        );
        if handle.is_null() {
            return None;
        }
        dlsym(handle, c"SecTrustedApplicationCopyRequirement".as_ptr())
    };
    // SAFETY: the symbol has this signature in every macOS that exports it.
    (!symbol.is_null())
        .then(|| unsafe { std::mem::transmute::<*mut c_void, CopyRequirement>(symbol) })
}

/// Whether the item's decrypt access list admits this running process
/// without a prompt. Reads the item reference and its access list only,
/// never the payload, with UI suppressed.
fn decrypt_access(persistent_ref: Ref, search_list: Option<Ref>) -> Access {
    let Some(copy_requirement) = copy_requirement() else {
        return Access::Indeterminate;
    };
    let Ok(query) = query(Query::ItemRef, persistent_ref, search_list) else {
        return Access::Indeterminate;
    };
    let Ok(Some(item)) = execute(&query) else {
        return Access::Indeterminate;
    };
    if !is_type(item.0, unsafe { SecKeychainItemGetTypeID() }) {
        return Access::Indeterminate;
    }
    let mut access = std::ptr::null();
    // SAFETY: a validated keychain item; the copy is released by Owned.
    if unsafe { SecKeychainItemCopyAccess(item.0, &mut access) } != 0 {
        return Access::Indeterminate;
    }
    let Ok(access) = Owned::new(access) else {
        return Access::Indeterminate;
    };
    // SAFETY: a live SecAccess and an exported authorization constant.
    let Ok(acls) =
        Owned::new(unsafe { SecAccessCopyMatchingACLList(access.0, kSecACLAuthorizationDecrypt) })
    else {
        return Access::Indeterminate;
    };
    if !is_type(acls.0, unsafe { CFArrayGetTypeID() }) {
        return Access::Indeterminate;
    }
    let mut code = std::ptr::null();
    // SAFETY: default flags; the copy is released by Owned.
    if unsafe { SecCodeCopySelf(0, &mut code) } != 0 {
        return Access::Indeterminate;
    }
    let Ok(code) = Owned::new(code) else {
        return Access::Indeterminate;
    };
    let count = unsafe { CFArrayGetCount(acls.0) };
    list_access((0..count).map(|index| {
        // SAFETY: index is within the array's count; acls outlives the call.
        let acl = unsafe { CFArrayGetValueAtIndex(acls.0, index) };
        let mut applications = std::ptr::null();
        let mut description = std::ptr::null();
        let mut selector = 0u16;
        if unsafe { SecACLCopyContents(acl, &mut applications, &mut description, &mut selector) }
            != 0
        {
            return Access::Indeterminate;
        }
        let _description = Owned::new(description).ok();
        let Ok(applications) = Owned::new(applications) else {
            return entry_access(selector, None);
        };
        if !is_type(applications.0, unsafe { CFArrayGetTypeID() }) {
            return Access::Indeterminate;
        }
        let statuses: Vec<Option<i32>> = (0..unsafe { CFArrayGetCount(applications.0) })
            .map(|index| {
                let application = unsafe { CFArrayGetValueAtIndex(applications.0, index) };
                let mut requirement = std::ptr::null();
                // SAFETY: a trusted-application element of a live array.
                if unsafe { copy_requirement(application, &mut requirement) } != 0 {
                    return None;
                }
                let requirement = Owned::new(requirement).ok()?;
                // SAFETY: this process's code and a live requirement.
                Some(unsafe { SecCodeCheckValidity(code.0, 0, requirement.0) })
            })
            .collect();
        entry_access(selector, Some(&statuses))
    }))
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
    // Without UI, read the payload only when the access list already admits
    // this process; anything else is left to a user-pressed Refresh.
    if !allow_ui && decrypt_access(item.persistent_ref()?, search_list) != Access::Allowed {
        return Err(INTERACTION_REQUIRED);
    }
    let query = query(
        if allow_ui {
            Query::ExplicitSecret
        } else {
            Query::SilentSecret
        },
        item.persistent_ref()?,
        search_list,
    )?;
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
        let context = NoUiContext::new().unwrap();
        let entries = query_entries(Query::Metadata, service.0, None, context.0);
        unsafe {
            assert!(!entries.iter().any(|(key, _)| *key == kSecReturnData));
            assert!(entries.contains(&(kSecUseAuthenticationUI, kSecUseAuthenticationUIFail)));
            assert!(entries.contains(&(kSecAttrService, service.0)));
            assert!(entries.contains(&(kSecUseAuthenticationContext, context.0)));
            let secret = query_entries(Query::ExplicitSecret, service.0, None, context.0);
            assert!(secret.contains(&(kSecReturnData, kCFBooleanTrue)));
            assert!(!secret.iter().any(|(key, _)| *key == kSecAttrService));
            assert!(
                !secret
                    .iter()
                    .any(|(key, _)| *key == kSecUseAuthenticationUI)
            );
            assert!(
                !secret
                    .iter()
                    .any(|(key, _)| *key == kSecUseAuthenticationContext)
            );
        }
    }

    #[test]
    fn silent_secret_query_suppresses_authentication_ui() {
        let service = string("Claude Code-credentials-fixture").unwrap();
        let context = NoUiContext::new().unwrap();
        let entries = query_entries(Query::SilentSecret, service.0, None, context.0);
        unsafe {
            assert!(entries.contains(&(kSecReturnData, kCFBooleanTrue)));
            assert!(entries.contains(&(kSecUseAuthenticationUI, kSecUseAuthenticationUIFail)));
            assert!(entries.contains(&(kSecUseAuthenticationContext, context.0)));
            assert!(entries.contains(&(kSecValuePersistentRef, service.0)));
        }
    }

    #[test]
    fn the_no_ui_context_forbids_interaction() {
        let context = NoUiContext::new().unwrap();
        assert!(context.interaction_not_allowed());
    }

    #[test]
    fn the_access_list_query_asks_for_the_item_reference_never_the_payload() {
        let service = string("Claude Code-credentials-fixture").unwrap();
        let context = NoUiContext::new().unwrap();
        let entries = query_entries(Query::ItemRef, service.0, None, context.0);
        unsafe {
            assert!(entries.contains(&(kSecReturnRef, kCFBooleanTrue)));
            assert!(!entries.iter().any(|(key, _)| *key == kSecReturnData));
            assert!(entries.contains(&(kSecUseAuthenticationUI, kSecUseAuthenticationUIFail)));
            assert!(entries.contains(&(kSecUseAuthenticationContext, context.0)));
            assert!(entries.contains(&(kSecValuePersistentRef, service.0)));
        }
    }

    /// Fails if a macOS release stops exporting the symbol, which would turn
    /// every background read into a skipped one.
    #[test]
    fn the_requirement_copy_symbol_resolves() {
        assert!(copy_requirement().is_some());
    }

    #[test]
    fn only_an_entry_that_admits_this_process_without_a_prompt_allows_a_silent_read() {
        // An entry naming no applications admits any caller, unless it
        // carries a prompt condition.
        assert_eq!(entry_access(0, None), Access::Allowed);
        assert_eq!(entry_access(0x0010, None), Access::Rejected);
        // One satisfied requirement is enough.
        assert_eq!(
            entry_access(0, Some(&[Some(REQUIREMENT_FAILED), Some(0)])),
            Access::Allowed
        );
        assert_eq!(entry_access(0x0001, Some(&[Some(0)])), Access::Rejected);
        // Every requirement checked and failed: rejected. An empty list
        // admits nobody.
        assert_eq!(
            entry_access(0, Some(&[Some(REQUIREMENT_FAILED)])),
            Access::Rejected
        );
        assert_eq!(entry_access(0, Some(&[])), Access::Rejected);
        // A requirement that could not be read or checked: unknown.
        assert_eq!(
            entry_access(0, Some(&[Some(REQUIREMENT_FAILED), None])),
            Access::Indeterminate
        );
        assert_eq!(entry_access(0, Some(&[Some(-1)])), Access::Indeterminate);

        assert_eq!(
            list_access([Access::Rejected, Access::Allowed]),
            Access::Allowed
        );
        assert_eq!(
            list_access([Access::Rejected, Access::Rejected]),
            Access::Rejected
        );
        assert_eq!(
            list_access([Access::Rejected, Access::Indeterminate]),
            Access::Indeterminate
        );
        assert_eq!(list_access([]), Access::Indeterminate);
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
        // The creating process is the item's trusted application.
        let found = metadata(service, Some(list.0)).unwrap().unwrap();
        assert_eq!(
            decrypt_access(found.persistent_ref().unwrap(), Some(list.0)),
            Access::Allowed
        );
        let before = found.revision().unwrap();
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
