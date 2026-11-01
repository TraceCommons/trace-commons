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
    fn CFDataCreate(allocator: Ref, bytes: *const u8, length: isize) -> Ref;
    fn CFPropertyListCreateWithData(
        allocator: Ref,
        data: Ref,
        options: usize,
        format: *mut isize,
        error: *mut Ref,
    ) -> Ref;
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
    static kSecACLAuthorizationPartitionID: Ref;
    static kSecCodeInfoTeamIdentifier: Ref;
    static kSecCodeInfoUnique: Ref;
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
    fn SecCodeCopySigningInformation(code: Ref, flags: u32, information: *mut Ref) -> i32;
    fn SecRequirementCopyData(requirement: Ref, flags: u32, data: *mut Ref) -> i32;
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
/// kSecCSBasicValidateOnly: check the signature against the requirement
/// without rehashing the executable or the bundle's resources.
const BASIC_VALIDATE_ONLY: u32 = (1 << 1) | (1 << 2);
/// kSecCSSigningInformation.
const SIGNING_INFORMATION: u32 = 1 << 1;

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
        interaction_not_allowed(self.0)
    }
}
#[cfg(test)]
fn interaction_not_allowed(context: Ref) -> bool {
    // SAFETY: the getter takes no arguments and returns a BOOL.
    let get: unsafe extern "C" fn(Ref, Ref) -> bool =
        unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
    unsafe { get(context, sel_registerName(c"interactionNotAllowed".as_ptr())) }
}
impl Drop for NoUiContext {
    fn drop(&mut self) {
        // SAFETY: `-release` balances the +1 from `+new`.
        let release: unsafe extern "C" fn(Ref, Ref) =
            unsafe { std::mem::transmute(objc_msgSend as unsafe extern "C" fn()) };
        unsafe { release(self.0, sel_registerName(c"release".as_ptr())) };
    }
}

thread_local! {
    /// One no-UI context per thread. Each load runs on its own leased
    /// thread, so this is one context per load, never shared across threads;
    /// it is released when the thread ends.
    static CONTEXT: Result<NoUiContext, &'static str> = NoUiContext::new();
}
/// Run `f` with this thread's no-UI context, failing closed without one.
fn with_no_ui_context<T>(
    f: impl FnOnce(&NoUiContext) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    CONTEXT.with(|context| f(context.as_ref().map_err(|reason| *reason)?))
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
/// Build a query. A kind that must not show UI fails closed without its
/// no-UI context, rather than running without one. The dictionary retains
/// the context, so one context serves every query of a read.
fn query(
    kind: Query,
    selector: Ref,
    search_list: Option<Ref>,
    context: Option<&NoUiContext>,
) -> Result<Owned, &'static str> {
    let context = match (kind, context) {
        (Query::ExplicitSecret, _) => std::ptr::null(),
        (_, Some(context)) => context.0,
        (_, None) => return Err("unavailable"),
    };
    dictionary(&query_entries(kind, selector, search_list, context))
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
    fn revision(&self) -> Result<ItemRevision, &'static str> {
        let reference = data(self.persistent_ref()?)?;
        let mut dates = Vec::new();
        for key in unsafe { [kSecAttrCreationDate, kSecAttrModificationDate] } {
            let value = self.value(key);
            if !is_type(value, unsafe { CFDateGetTypeID() }) {
                return Err("unavailable");
            }
            // Exact f64 bits preserve subsecond changes; no timestamp rounding.
            dates.push(
                unsafe { CFDateGetAbsoluteTime(value) }
                    .to_bits()
                    .to_be_bytes(),
            );
        }
        let account = cf_string(self.value(unsafe { kSecAttrAccount }))?;
        // Neither part contains raw account names or persistent references.
        // They are equality discriminators, not security digests or IDs.
        let digest = |parts: &[&[u8]]| {
            let mut material = Vec::new();
            for part in parts {
                material.extend_from_slice(&(part.len() as u64).to_be_bytes());
                material.extend_from_slice(part);
            }
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &material).to_string()
        };
        Ok(ItemRevision {
            item: digest(&[&reference, &dates[0], account.as_bytes()]),
            content: digest(&[&reference, &dates[0], &dates[1], account.as_bytes()]),
        })
    }
}
/// A CFString's UTF-8 contents, bounded.
fn cf_string(value: Ref) -> Result<String, &'static str> {
    if !is_type(value, unsafe { CFStringGetTypeID() }) {
        return Err("unavailable");
    }
    // SAFETY: a validated CFString the caller keeps alive through the copy.
    let capacity = unsafe { CFStringGetMaximumSizeForEncoding(CFStringGetLength(value), UTF8) };
    if !(0..=32768).contains(&capacity) {
        return Err("unavailable");
    }
    let mut bytes = vec![0u8; capacity as usize + 1];
    if unsafe { CFStringGetCString(value, bytes.as_mut_ptr().cast(), bytes.len() as isize, UTF8) }
        == 0
    {
        return Err("unavailable");
    }
    let end = bytes.iter().position(|b| *b == 0).ok_or("unavailable")?;
    bytes.truncate(end);
    String::from_utf8(bytes).map_err(|_| "unavailable")
}
fn metadata(
    service: &str,
    search_list: Option<Ref>,
    context: &NoUiContext,
) -> Result<Option<Metadata>, &'static str> {
    let service = string(service)?;
    let query = query(Query::Metadata, service.0, search_list, Some(context))?;
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
    static SYMBOL: std::sync::OnceLock<Option<CopyRequirement>> = std::sync::OnceLock::new();
    *SYMBOL.get_or_init(|| {
        // SAFETY: a constant system framework path and symbol name. The
        // handle is never closed, so the function pointer stays valid.
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
    })
}

/// A requirement's check of this process, remembered per requirement. Only
/// a finished answer (satisfied or `REQUIREMENT_FAILED`) is kept: the running
/// image does not change, so neither does that answer, including after the
/// app is updated on disk underneath it. Anything else is asked again.
fn remembered_check(
    memo: &std::sync::Mutex<std::collections::HashMap<Vec<u8>, i32>>,
    requirement: Vec<u8>,
    check: impl FnOnce() -> i32,
) -> i32 {
    if let Some(status) = memo.lock().ok().and_then(|m| m.get(&requirement).copied()) {
        return status;
    }
    let status = check();
    if status == 0 || status == REQUIREMENT_FAILED {
        if let Ok(mut memo) = memo.lock() {
            memo.insert(requirement, status);
        }
    }
    status
}
fn requirement_checks() -> &'static std::sync::Mutex<std::collections::HashMap<Vec<u8>, i32>> {
    static MEMO: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<Vec<u8>, i32>>> =
        std::sync::OnceLock::new();
    MEMO.get_or_init(Default::default)
}

/// This process's own partition IDs: its signing team (`teamid:`), and its
/// code-directory hash (`cdhash:`), which securityd uses for an ad-hoc or
/// unsigned build and adds to an item's list on "Always Allow".
struct OwnCode {
    team: Option<String>,
    cdhash: Option<String>,
}
fn own_code() -> &'static OwnCode {
    static OWN: std::sync::OnceLock<OwnCode> = std::sync::OnceLock::new();
    OWN.get_or_init(|| {
        let unknown = || OwnCode {
            team: None,
            cdhash: None,
        };
        let mut code = std::ptr::null();
        // SAFETY: default flags; the copy is released by Owned.
        if unsafe { SecCodeCopySelf(0, &mut code) } != 0 {
            return unknown();
        }
        let Ok(code) = Owned::new(code) else {
            return unknown();
        };
        let mut information = std::ptr::null();
        // SAFETY: a live code object; the copy is released by Owned.
        if unsafe { SecCodeCopySigningInformation(code.0, SIGNING_INFORMATION, &mut information) }
            != 0
        {
            return unknown();
        }
        let Ok(information) = Owned::new(information) else {
            return unknown();
        };
        if !is_type(information.0, unsafe { CFDictionaryGetTypeID() }) {
            return unknown();
        }
        let team = unsafe { CFDictionaryGetValue(information.0, kSecCodeInfoTeamIdentifier) };
        let unique = unsafe { CFDictionaryGetValue(information.0, kSecCodeInfoUnique) };
        OwnCode {
            team: cf_string(team).ok().filter(|team| !team.is_empty()),
            cdhash: data(unique)
                .ok()
                .filter(|hash| !hash.is_empty())
                .map(|hash| hash.iter().map(|b| format!("{b:02x}")).collect()),
        }
    })
}

/// The partition IDs in a `partition_id` access-list entry's description:
/// a hex-encoded XML property list whose `Partitions` key holds strings.
fn partitions(description: &str) -> Option<Vec<String>> {
    if !description.len().is_multiple_of(2) || description.len() > 65_536 {
        return None;
    }
    let bytes = (0..description.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(description.get(i..i + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    // SAFETY: a bounded local buffer copied into a new CFData.
    let data =
        Owned::new(unsafe { CFDataCreate(std::ptr::null(), bytes.as_ptr(), bytes.len() as isize) })
            .ok()?;
    let list = Owned::new(unsafe {
        CFPropertyListCreateWithData(
            std::ptr::null(),
            data.0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    })
    .ok()?;
    if !is_type(list.0, unsafe { CFDictionaryGetTypeID() }) {
        return None;
    }
    let key = string("Partitions").ok()?;
    let array = unsafe { CFDictionaryGetValue(list.0, key.0) };
    if !is_type(array, unsafe { CFArrayGetTypeID() }) {
        return None;
    }
    (0..unsafe { CFArrayGetCount(array) })
        .map(|i| cf_string(unsafe { CFArrayGetValueAtIndex(array, i) }).ok())
        .collect()
}

/// One `partition_id` entry. `None` partitions: the description could not be
/// read. A process outside the listed partitions is asked for the keychain
/// password even when the access list trusts it, so only this build's own
/// partition admits it: `teamid:` for a signed build, or `cdhash:` for the
/// exact code an "Always Allow" was given to. Hashes compare without case.
fn partition_access(
    partitions: Option<&[String]>,
    team: Option<&str>,
    cdhash: Option<&str>,
) -> Access {
    let Some(partitions) = partitions else {
        return Access::Indeterminate;
    };
    if team.is_none() && cdhash.is_none() {
        return Access::Indeterminate;
    }
    let admitted = partitions.iter().any(|partition| {
        team.is_some_and(|team| partition.strip_prefix("teamid:") == Some(team))
            || cdhash.is_some_and(|hash| {
                partition
                    .strip_prefix("cdhash:")
                    .is_some_and(|listed| listed.eq_ignore_ascii_case(hash))
            })
    });
    if admitted {
        Access::Allowed
    } else {
        Access::Rejected
    }
}

/// Whether the item admits this running process without a prompt: its
/// decrypt access list trusts this code, and its partition list holds this
/// build's team. Reads the item reference and its access list only, never
/// the payload, with UI suppressed.
fn silent_access(persistent_ref: Ref, search_list: Option<Ref>, context: &NoUiContext) -> Access {
    let Some(copy_requirement) = copy_requirement() else {
        return Access::Indeterminate;
    };
    let Ok(query) = query(Query::ItemRef, persistent_ref, search_list, Some(context)) else {
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
    // SAFETY: a live SecAccess and an exported authorization constant.
    let partition = match Owned::new(unsafe {
        SecAccessCopyMatchingACLList(access.0, kSecACLAuthorizationPartitionID)
    }) {
        // An item with no partition entry predates partition lists, and
        // securityd does not ask for it; the access list alone decides.
        Ok(entries)
            if is_type(entries.0, unsafe { CFArrayGetTypeID() })
                && unsafe { CFArrayGetCount(entries.0) } == 0 =>
        {
            Access::Allowed
        }
        Err(_) => Access::Allowed,
        Ok(entries) if is_type(entries.0, unsafe { CFArrayGetTypeID() }) => {
            list_access((0..unsafe { CFArrayGetCount(entries.0) }).map(|index| {
                let entry = unsafe { CFArrayGetValueAtIndex(entries.0, index) };
                let mut applications = std::ptr::null();
                let mut description = std::ptr::null();
                let mut selector = 0u16;
                if unsafe {
                    SecACLCopyContents(entry, &mut applications, &mut description, &mut selector)
                } != 0
                {
                    return Access::Indeterminate;
                }
                let _applications = Owned::new(applications).ok();
                let listed = Owned::new(description)
                    .ok()
                    .and_then(|description| cf_string(description.0).ok())
                    .and_then(|description| partitions(&description));
                let own = own_code();
                partition_access(
                    listed.as_deref(),
                    own.team.as_deref(),
                    own.cdhash.as_deref(),
                )
            }))
        }
        Ok(_) => Access::Indeterminate,
    };
    if partition != Access::Allowed {
        return partition;
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
                let mut bytes = std::ptr::null();
                // SAFETY: a live requirement; the copy is released by Owned.
                if unsafe { SecRequirementCopyData(requirement.0, 0, &mut bytes) } != 0 {
                    return None;
                }
                let bytes = data(Owned::new(bytes).ok()?.0).ok()?;
                // SAFETY: this process's code and a live requirement. Basic
                // validation matches the signing identity securityd checks,
                // without rehashing the bundle on every call.
                Some(remembered_check(requirement_checks(), bytes, || unsafe {
                    SecCodeCheckValidity(code.0, BASIC_VALIDATE_ONLY, requirement.0)
                }))
            })
            .collect();
        entry_access(selector, Some(&statuses))
    }))
}

// errSecInteractionNotAllowed. Callers above this module see it only from a
// silent read; everywhere else it is a permission need.
use super::INTERACTION_REQUIRED;

use super::ItemRevision;

#[cfg(not(test))]
pub(super) fn version(service: &str) -> Result<Option<ItemRevision>, &'static str> {
    with_no_ui_context(|context| {
        metadata(service, None, context)
            .map_err(permission_label)?
            .map(|m| m.revision())
            .transpose()
    })
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
    // This thread's no-UI context serves every query of this read.
    with_no_ui_context(|context| read_with(service, search_list, allow_ui, context))
}
fn read_with(
    service: &str,
    search_list: Option<Ref>,
    allow_ui: bool,
    context: &NoUiContext,
) -> Result<Option<Vec<u8>>, &'static str> {
    // A locked keychain can refuse even the attribute query. On a silent
    // read that is an interaction need; with UI allowed it is final.
    let item = metadata(service, search_list, context).map_err(|reason| {
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
    if !allow_ui && silent_access(item.persistent_ref()?, search_list, context) != Access::Allowed {
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
        Some(context),
    )?;
    let Some(result) = execute(&query)? else {
        return Ok(None);
    };
    let after = metadata(service, search_list, context)
        .map_err(permission_label)?
        .map(|m| m.revision())
        .transpose()?;
    if after.as_ref() != Some(&before) {
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
    fn a_no_ui_query_without_its_context_is_refused() {
        let service = string("Claude Code-credentials-fixture").unwrap();
        for kind in [Query::Metadata, Query::ItemRef, Query::SilentSecret] {
            assert!(query(kind, service.0, None, None).is_err());
        }
        assert!(query(Query::ExplicitSecret, service.0, None, None).is_ok());
    }

    /// The live Claude Code item's `partition_id` description, recorded
    /// 2026-11-01 from its access list (attributes only).
    const LIVE_PARTITIONS: &str = "3c3f786d6c2076657273696f6e3d22312e302220656e636f64696e673d225554462d38223f3e0a3c21444f435459504520706c697374205055424c494320222d2f2f4170706c652f2f44544420504c49535420312e302f2f454e222022687474703a2f2f7777772e6170706c652e636f6d2f445444732f50726f70657274794c6973742d312e302e647464223e0a3c706c6973742076657273696f6e3d22312e30223e0a3c646963743e0a093c6b65793e506172746974696f6e733c2f6b65793e0a093c61727261793e0a09093c737472696e673e6170706c652d746f6f6c3a3c2f737472696e673e0a09093c737472696e673e7465616d69643a4b5853574a4e375759383c2f737472696e673e0a093c2f61727261793e0a3c2f646963743e0a3c2f706c6973743e0a";

    #[test]
    fn the_partition_list_is_read_from_its_description() {
        assert_eq!(
            partitions(LIVE_PARTITIONS).unwrap(),
            ["apple-tool:", "teamid:KXSWJN7WY8"]
        );
        assert!(partitions("zz").is_none());
        assert!(partitions("abc").is_none());
        assert!(partitions("3c3f").is_none());
    }

    #[test]
    fn only_this_builds_own_partition_admits_a_silent_read() {
        let listed = partitions(LIVE_PARTITIONS).unwrap();
        let team = Some("KXSWJN7WY8");
        assert_eq!(partition_access(Some(&listed), team, None), Access::Allowed);
        // Trusted by the access list is not enough: another team is outside
        // the partition list.
        assert_eq!(
            partition_access(Some(&listed), Some("OTHERTEAM1"), None),
            Access::Rejected
        );
        // An ad-hoc build is known by its code-directory hash, which an
        // "Always Allow" adds to the list.
        let hash = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            partition_access(Some(&listed), None, Some(hash)),
            Access::Rejected
        );
        let mut allowed = listed.clone();
        allowed.push(format!("cdhash:{}", hash.to_ascii_uppercase()));
        assert_eq!(
            partition_access(Some(&allowed), None, Some(hash)),
            Access::Allowed
        );
        // A process with neither cannot be matched.
        assert_eq!(
            partition_access(Some(&listed), None, None),
            Access::Indeterminate
        );
        // `apple-tool:` names Apple's tools, never this app.
        assert_eq!(
            partition_access(Some(&["apple-tool:".to_string()]), team, Some(hash)),
            Access::Rejected
        );
        // A team ID only matches whole.
        assert_eq!(
            partition_access(Some(&["teamid:KXSWJN7WY8X".to_string()]), team, None),
            Access::Rejected
        );
        assert_eq!(partition_access(None, team, None), Access::Indeterminate);
    }

    #[test]
    fn each_thread_reuses_one_no_ui_context() {
        let first = with_no_ui_context(|context| Ok(context.0 as usize)).unwrap();
        let again = with_no_ui_context(|context| Ok(context.0 as usize)).unwrap();
        assert_eq!(first, again);
        assert!(with_no_ui_context(|context| Ok(context.interaction_not_allowed())).unwrap());
        let other =
            std::thread::spawn(|| with_no_ui_context(|context| Ok(context.0 as usize)).unwrap())
                .join()
                .unwrap();
        assert_ne!(first, other);
    }

    #[test]
    fn a_finished_requirement_check_is_remembered_and_nothing_else_is() {
        let memo = std::sync::Mutex::new(std::collections::HashMap::new());
        let counter = std::cell::Cell::new(0);
        let calls = &counter;
        let check = |status| {
            move || {
                calls.set(calls.get() + 1);
                status
            }
        };
        assert_eq!(remembered_check(&memo, b"req-a".to_vec(), check(0)), 0);
        assert_eq!(remembered_check(&memo, b"req-a".to_vec(), check(-1)), 0);
        assert_eq!(calls.get(), 1);
        assert_eq!(
            remembered_check(&memo, b"req-b".to_vec(), check(REQUIREMENT_FAILED)),
            REQUIREMENT_FAILED
        );
        assert_eq!(
            remembered_check(&memo, b"req-b".to_vec(), check(0)),
            REQUIREMENT_FAILED
        );
        assert_eq!(calls.get(), 2);
        // An error (for example the code changed on disk) is asked again.
        assert_eq!(
            remembered_check(&memo, b"req-c".to_vec(), check(-67068)),
            -67068
        );
        assert_eq!(remembered_check(&memo, b"req-c".to_vec(), check(0)), 0);
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn the_symbol_lookup_is_made_once() {
        let first = copy_requirement().map(|f| f as usize);
        assert_eq!(copy_requirement().map(|f| f as usize), first);
    }

    #[test]
    fn the_no_ui_context_forbids_interaction() {
        let context = NoUiContext::new().unwrap();
        assert!(context.interaction_not_allowed());
    }

    /// The built query keeps its own reference to the context after ours is
    /// released, and an explicit read has none. Builds dictionaries only;
    /// never runs a query.
    #[test]
    fn built_no_ui_queries_retain_their_context() {
        let service = string("Claude Code-credentials-fixture").unwrap();
        for kind in [Query::Metadata, Query::ItemRef, Query::SilentSecret] {
            let built = {
                let context = NoUiContext::new().unwrap();
                query(kind, service.0, None, Some(&context)).unwrap()
            };
            let context = unsafe { CFDictionaryGetValue(built.0, kSecUseAuthenticationContext) };
            assert!(!context.is_null());
            assert!(interaction_not_allowed(context));
        }
        let context = NoUiContext::new().unwrap();
        let built = query(Query::ExplicitSecret, service.0, None, Some(&context)).unwrap();
        assert!(unsafe { CFDictionaryGetValue(built.0, kSecUseAuthenticationContext) }.is_null());
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
        let context = NoUiContext::new().unwrap();
        let found = metadata(service, Some(list.0), &context).unwrap().unwrap();
        // Runs the whole access-list walk over FFI. Whether an ad-hoc test
        // binary is admitted depends on the partition its creation recorded,
        // which this fixture has not been run to observe; no outcome is
        // asserted.
        let _ = silent_access(found.persistent_ref().unwrap(), Some(list.0), &context);
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
        let after = metadata(service, Some(list.0), &context)
            .unwrap()
            .unwrap()
            .revision()
            .unwrap();
        // An in-place write moves the content, not the item.
        assert_ne!(before.content, after.content);
        assert_eq!(before.item, after.item);
        assert!(
            metadata("different-exact-profile", Some(list.0), &context)
                .unwrap()
                .is_none()
        );
        assert_eq!(unsafe { SecKeychainLock(keychain.0.0) }, 0);
        let started = std::time::Instant::now();
        let locked = metadata(service, Some(list.0), &context);
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
