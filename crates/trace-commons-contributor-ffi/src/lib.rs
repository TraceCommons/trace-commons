//! The C ABI for `trace-commons-contributor`'s background daemon.
//!
//! A native application shell (SwiftUI on macOS, WinUI on Windows, GTK on
//! Linux) hosts the daemon loop in-process rather than shipping and
//! notarizing a second binary. This crate is the seam: it wraps the existing
//! Rust daemon (`trace_commons_contributor::daemon`) behind a small `extern
//! "C"` surface callable from Swift via a bridging header and from C# via
//! P/Invoke, and it contains no logic of its own beyond that translation.
//!
//! # Ownership rule (stated once, obeyed everywhere)
//!
//! Every `char*` **returned** by this library is owned by the caller and
//! freed with [`tc_string_free`]. Every `const char*` is **borrowed** and
//! valid only until the handle that owns it (`tc_handle` or `tc_preview`) is
//! freed. There are no other lifetime rules anywhere in this crate.
//!
//! `tc_daemon_start` returns a `tc_handle*`; `tc_daemon_stop` tears the
//! running daemon down but does **not** free that pointer (a concurrent
//! `tc_call`/`tc_preview_open`/`tc_subscribe` on another thread stays valid
//! and simply observes the daemon as stopped, rather than dereferencing
//! freed memory); [`tc_handle_free`] is the only function that reclaims it,
//! and -- like every free function here -- must not be called concurrently
//! with any other call still using the same pointer.
//!
//! # Panic safety
//!
//! Every exported function wraps its body in [`std::panic::catch_unwind`]
//! via the [`guard`] helper. A Rust panic must never unwind across the FFI
//! boundary into Swift, C#, or C -- that is undefined behaviour on every one
//! of those callers. A caught panic becomes an ordinary error string, and
//! every step that follows catching it -- building that string, writing an
//! out-param -- is audited (see each such call site) not to itself panic,
//! so nothing after the `catch_unwind` boundary can defeat it either.
//!
//! # What never crosses this boundary
//!
//! No path, token, URL, or trace/session content appears in any error
//! string returned by this crate. The daemon's own hash-only / label-only
//! discipline (see the workspace root `CLAUDE.md`) applies here exactly as
//! it does at the socket: several of the daemon crate's own internal error
//! messages embed filesystem paths for CLI/local-stderr consumption (e.g.
//! `ConfigStore::open`, `daemon::start_embedded`'s lock-file context), which
//! is fine for that surface but not safe to forward verbatim across a
//! language boundary a GUI might log or display. [`guard`] therefore
//! discards the underlying `anyhow::Error`'s `Display` text by default and
//! substitutes a fixed label; [`guard_forwarding`] is the explicit,
//! documented-per-call-site opt-in for a closure every one of whose error
//! paths is already known to produce a fixed, safe label.
//!
//! ## The preview exemption
//!
//! [`tc_preview_body`] **is** trace content, and that is deliberate. It is
//! the single exemption to the rule above, and it exists because a
//! contributor cannot consent to sending something they cannot see -- an
//! approval given against a size and a project name is not an informed one.
//! The exemption is bounded, and the bounds are the whole point:
//!
//! * **Post-redaction only.** The body is whatever the real redaction
//!   pipeline produced; raw session text never reaches this boundary.
//! * **Only for an entry the caller already holds.** It is reachable only
//!   through a `tc_preview*` the caller opened for a specific `entry_id`;
//!   there is no bulk or ambient content read.
//! * **Never onward.** It is never written to a log line, an audit entry, a
//!   history record, notification text, or a receipt. Nothing in this crate
//!   or the daemon copies it into any of those.
//!
//! Opening a preview also has a **side effect at rest**: the daemon writes
//! the redacted envelope it just built into the contributor's own 0700
//! state directory (0600, atomic) and the upload then sends exactly those
//! bytes, so that what was shown here and what leaves the machine cannot
//! diverge through a privacy filter that does not reproduce its own output.
//! Those bytes are post-redaction, one file per previewed entry, capped by
//! the envelope size limit, deleted as soon as the entry resolves, and
//! removed on logout. They never come back across this ABI: the only
//! content this boundary serves is the body returned to the caller who
//! asked for it.
//!
//! Everywhere else the rule remains absolute: no path, token, URL, or trace
//! content in any other returned string, on any error path, at any time.
//!
//! Preview content is also held to a stricter rule than "no *raw*
//! path/token": [`tc_preview_open`] fails outright, rather than silently
//! editing the text, if the body contains a byte that cannot cross the
//! boundary as `char*` (a NUL) -- preview exists precisely so it can never
//! disagree with what an upload sends, and a silently-truncated-or-stripped
//! body would violate that guarantee without telling anyone.

// `tc_handle` / `tc_preview` are named to match the C header
// (`include/trace_commons.h`) and the Swift/C# callers that bind to it
// verbatim, not Rust naming conventions.
#![allow(non_camel_case_types)]

use std::collections::HashMap;
use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::UnwindSafe;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::ipc::{self, ERR_BAD_PARAMS, Response};
use trace_commons_contributor::daemon::settings::{
    DaemonSettings, apply_settings_object, roots_declared,
};
use trace_commons_contributor::daemon::{EmbeddedDaemon, StartFailure};

/// Catches a panic; on an ordinary `Err`, returns a fixed, content-free
/// label rather than forwarding the underlying `anyhow::Error`'s `Display`
/// text, which might embed a path or other content unsafe to cross this
/// boundary (see the module doc). This is the default every call site
/// should use.
fn guard<T>(f: impl FnOnce() -> anyhow::Result<T> + UnwindSafe) -> Result<T, String> {
    guard_with(f, |_e| "operation-failed".to_string())
}

/// Like [`guard`], but forwards the underlying error's `Display` text
/// instead of a fixed label. Only for a closure every one of whose error
/// paths is already known, and documented at the call site, to produce a
/// fixed, safe label of its own -- so that forwarding it verbatim is exactly
/// as safe as [`guard`]'s default, not a way to route around it.
fn guard_forwarding<T>(f: impl FnOnce() -> anyhow::Result<T> + UnwindSafe) -> Result<T, String> {
    guard_with(f, |e| format!("{e:#}"))
}

fn guard_with<T>(
    f: impl FnOnce() -> anyhow::Result<T> + UnwindSafe,
    on_err: impl FnOnce(anyhow::Error) -> String,
) -> Result<T, String> {
    match std::panic::catch_unwind(f) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(on_err(e)),
        // A Rust panic must never unwind into Swift, C#, or C.
        Err(_) => Err("panic".to_string()),
    }
}

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

/// Records `label` as this thread's last error. Uses `try_with`, not
/// `with`: a thread whose thread-local storage is already being torn down
/// (most notably during process/thread exit, when a `Drop` impl elsewhere
/// happens to call into this crate) would otherwise make `.with()` panic on
/// an ordinary error-reporting path -- exactly the kind of panic
/// `catch_unwind` is here to prevent, so the thing preventing it must not
/// itself be a fresh source of one. On that failure this is a silent no-op:
/// there is no thread-local left to record into, and no safe way to make
/// one.
fn set_last_error(label: &str) {
    let c = CString::new(sanitize_for_c(label)).unwrap_or_else(|_| CString::new("error").unwrap());
    let _ = LAST_ERROR.try_with(|cell| *cell.borrow_mut() = Some(c));
}

/// Strips embedded NULs, which cannot appear in a C string. Used only for
/// strings this crate constructs itself (fixed labels, JSON frames it
/// builds) -- never for preview content, which fails instead of being
/// silently edited; see [`tc_preview_open`].
fn sanitize_for_c(s: &str) -> String {
    s.replace('\0', "")
}

/// Allocate an owned, caller-freed C string from `s`, on the caller's
/// behalf, and register it so [`tc_string_free`] can detect a double-free
/// or a pointer of the wrong allocation kind rather than acting on it
/// blindly. Used for every `char*` this crate returns except
/// `tc_preview_open`'s body/summary (owned by the `tc_preview`, not
/// separately allocated).
fn to_owned_cstring(s: &str) -> *mut c_char {
    let ptr = CString::new(sanitize_for_c(s))
        .unwrap_or_else(|_| CString::new("encoding-error").unwrap())
        .into_raw();
    registry_insert(ptr as usize, AllocKind::String);
    ptr
}

/// Build a `Response`-shaped JSON error frame with a fixed code, using the
/// real `ipc::Response`/`ipc::IpcError` types (not a hand-rolled
/// `serde_json::json!` of the same shape) so this crate's synthesized
/// frames -- malformed params, null pointers -- cannot drift from what
/// `handle_local` actually serializes.
fn error_frame(code: &str, message: &str) -> String {
    let response = Response::err(0, code, message);
    serde_json::to_string(&response).unwrap_or_else(|_| {
        format!(
            "{{\"id\":0,\"error\":{{\"code\":\"{}\",\"message\":\"serialize-failed\"}}}}",
            code.replace('"', "")
        )
    })
}

// --- Allocation registry: double-free / cross-type-free / foreign-pointer
// detection --------------------------------------------------------------
//
// Every pointer this crate hands out is registered here on allocation and
// removed on free. A free function that receives a pointer not currently
// registered under its own kind -- because it was never one of ours,
// because it was already freed, or because it is a `tc_handle*` handed to
// `tc_preview_free` (or similar) -- refuses rather than acting on it: with
// only a raw pointer and no type information at the FFI boundary (Swift
// `OpaquePointer` / C# `IntPtr` give the caller no compiler help here),
// blindly calling `Box::from_raw`/`CString::from_raw` on the wrong
// allocation is undefined behaviour, not a catchable error. Turning it into
// a `tc_last_error` label plus a no-op is the only options available at
// this boundary.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AllocKind {
    String,
    Handle,
    Preview,
}

static REGISTRY: LazyLock<Mutex<HashMap<usize, AllocKind>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Records `ptr` as a live allocation of `kind`.
///
/// Recovers from a poisoned mutex the same way `registry_take` and
/// `registry_is` do. Dropping the insert instead used to cost a leak -- the
/// allocation still worked, only its free was refused as unknown. Now that
/// the six borrowing entry points consult the registry too, a dropped insert
/// would hand the caller a handle that every one of them refuses forever, so
/// the one accessor that silently gave up is the one that can least afford
/// to.
fn registry_insert(ptr: usize, kind: AllocKind) {
    let mut r = match REGISTRY.lock() {
        Ok(r) => r,
        Err(p) => p.into_inner(),
    };
    r.insert(ptr, kind);
}

/// Removes `ptr` from the registry if, and only if, it is currently
/// registered as `kind`. `Ok(())` means the normal drop that follows is
/// acting on a real, live allocation of the right type. `Err(label)` means
/// deallocating would be a double-free, a cross-type free, or a pointer
/// this crate never allocated -- the caller must not proceed.
fn registry_take(ptr: usize, kind: AllocKind) -> Result<(), &'static str> {
    let mut r = match REGISTRY.lock() {
        Ok(r) => r,
        Err(p) => p.into_inner(),
    };
    // Inspect BEFORE removing. `remove` first would delete the entry and
    // only then report the mismatch, so a refused cross-type free would
    // unregister a live allocation: the pointer stays valid but every
    // `registry_is` check on it fails from then on, and its real free is
    // refused as unknown. A refusal must leave the registry untouched.
    match r.entry(ptr) {
        std::collections::hash_map::Entry::Occupied(found) if *found.get() == kind => {
            found.remove();
            Ok(())
        }
        std::collections::hash_map::Entry::Occupied(_) => Err("cross-type-free"),
        std::collections::hash_map::Entry::Vacant(_) => Err("double-free-or-unknown-pointer"),
    }
}

/// Whether `ptr` is currently registered under `kind`, without removing it.
///
/// This is what the borrowing accessors (`tc_preview_body`,
/// `tc_preview_summary_json`, `tc_preview_search`) consult before they
/// dereference, and what `handle_pointer_is_live` consults on behalf of
/// the six `tc_handle*` entry points that borrow rather than free a
/// handle (`tc_daemon_stop`, `tc_call`, `tc_subscribe`, `tc_unsubscribe`,
/// `tc_preview_open`, `tc_preview_turns_json`). Without it they trusted
/// the caller's pointer outright, so a stale (already-freed) or
/// cross-type pointer was a use-after-free or a type confusion rather
/// than the fixed error the rest of this ABI promises.
///
/// What this can and cannot guarantee, stated honestly:
///
/// * It **can** reject a pointer this crate never allocated, one already
///   passed to its free function, and one allocated as a different kind.
/// * It **cannot** make a concurrent free safe. Between this check and the
///   dereference that follows, another thread calling `tc_preview_free` can
///   deallocate the object; the registry entry is a `usize`, not shared
///   ownership of the allocation. The ABI contract is therefore unchanged:
///   a `tc_preview*` must not be freed while another thread is inside an
///   accessor for it. The registry narrows accidental misuse to a clean
///   error; it does not replace the caller's ownership discipline.
/// * A freed address can also be *reused* by a later allocation of the same
///   kind, in which case the check passes for a pointer whose original
///   object is gone. That is inherent to address-keyed bookkeeping.
fn registry_is(ptr: usize, kind: AllocKind) -> bool {
    let r = match REGISTRY.lock() {
        Ok(r) => r,
        Err(p) => p.into_inner(),
    };
    r.get(&ptr) == Some(&kind)
}

/// A fixed, safe label for any failure whose underlying `anyhow::Error`
/// might embed a filesystem path (state-directory or lock-file operations).
/// See the module doc's "What never crosses this boundary" section.
const ERR_DAEMON_START_FAILED: &str = "daemon-start-failed";

/// The contributor has not said which session folders to watch, so the
/// daemon will not start and has scanned nothing.
///
/// Distinct from `ERR_DAEMON_START_FAILED` on purpose. That label is
/// deliberately opaque because the failures behind it may embed a path;
/// this one is a fixed, content-free fact about configuration, and the
/// application shells have to tell the two apart -- a roots refusal routes
/// the contributor to the roots screen, every other start failure to the
/// generic notice. Flattening this into `daemon-start-failed` would leave
/// the shells guessing, and the only screen that can clear the refusal
/// unreachable.
const ERR_ROOTS_NOT_DECLARED: &str = "roots-not-declared";

/// Fixed labels for the start failures the daemon can name, each mapped from
/// a [`StartFailure`] variant rather than from the error's prose.
///
/// Every one exists because a contributor facing it has a different next
/// action. `ERR_DAEMON_START_FAILED` remains for everything else, and remains
/// opaque for the reason `finish_daemon_start` documents -- these are not a
/// relaxation of that rule but the cases where a fixed, content-free fact can
/// be stated without carrying any part of the underlying error across.
const ERR_ALREADY_RUNNING: &str = "already-running";
const ERR_STATE_DIR_NOT_WRITABLE: &str = "state-directory-not-writable";
const ERR_SETTINGS_UNREADABLE: &str = "settings-unreadable";
const ERR_IPC_BIND_FAILED: &str = "ipc-bind-failed";

/// The fixed label for a start failure, or `ERR_DAEMON_START_FAILED` when the
/// daemon did not name one.
///
/// Matches on the typed variant, never on the error text: the `anyhow` chain
/// behind these embeds state-directory and lock-file paths, and reading it to
/// decide a label is one careless `format!` away from forwarding them.
fn start_failure_label(err: &anyhow::Error) -> &'static str {
    match err.downcast_ref::<StartFailure>() {
        Some(StartFailure::AlreadyRunning) => ERR_ALREADY_RUNNING,
        Some(StartFailure::StateDirectoryNotWritable) => ERR_STATE_DIR_NOT_WRITABLE,
        Some(StartFailure::SettingsUnreadable) => ERR_SETTINGS_UNREADABLE,
        Some(StartFailure::IpcBindFailed) => ERR_IPC_BIND_FAILED,
        None => ERR_DAEMON_START_FAILED,
    }
}

/// Whether this store's persisted settings declare both session roots, as
/// `daemon::settings::roots_declared` defines it -- the single definition of
/// the rule, deliberately not restated here.
///
/// A settings file that cannot be read is NOT reported as a roots refusal:
/// the contributor's answer to "which folders?" is unknown rather than
/// absent, and pointing them at the roots screen would be a guess. It falls
/// through to the opaque start failure instead, which is also what
/// `start_embedded` would have produced from the same unreadable file.
fn roots_refusal(store: &ConfigStore) -> Option<&'static str> {
    let settings = DaemonSettings::load(store).ok()?;
    if roots_declared(&settings) {
        None
    } else {
        Some(ERR_ROOTS_NOT_DECLARED)
    }
}

/// The daemon that is actually running: the pieces `daemon::start_embedded`
/// returns, plus the `JoinHandle` for the supervise-loop task this crate
/// spawns itself (see `daemon::run_supervisor`'s doc for why `tc_handle`,
/// not the daemon crate, owns that task).
struct RunningDaemon {
    embedded: EmbeddedDaemon,
    supervisor: tokio::task::JoinHandle<anyhow::Result<()>>,
}

/// The daemon handle: an owned tokio runtime plus the running daemon's
/// shared state and background task handles.
///
/// `tc_daemon_start` returns `Box::into_raw(Box::new(tc_handle { .. }))`.
/// `tc_daemon_stop` tears the daemon down (idempotent, safe from any
/// thread) without freeing this allocation; `tc_handle_free` is the only
/// function that does, and must not race a call still using the handle.
pub struct tc_handle {
    rt: tokio::runtime::Runtime,
    /// `None` once the daemon has been claimed for teardown -- by
    /// `tc_daemon_stop`, or implicitly by `tc_handle_free` tearing it down
    /// before freeing. Deliberately a plain `Option` and not a
    /// `Running/TearingDown/Stopped` state machine with a `Condvar`: see
    /// `stop_embedded`'s doc for why making a second concurrent stop *wait*
    /// for the first caller's teardown is unsound here.
    running: Mutex<Option<RunningDaemon>>,
    subscriptions: Mutex<HashMap<u64, tokio::task::JoinHandle<()>>>,
    next_subscription: AtomicU64,
}

/// Borrow the running daemon's shared state, or `None` if it has been
/// stopped (or has been claimed for teardown by a concurrent stop). The one
/// place `tc_call`, `tc_preview_open`, and `tc_subscribe` all read
/// `handle.running` from, so they agree on what "stopped" means.
///
/// `shared.shutdown` counts as stopped too, not only `running == None`.
/// A host can reach the daemon's own `"shutdown"` method through
/// `tc_call(h, "shutdown", "{}")`, which sets that flag and ends the
/// supervise loop but never touches `handle.running` -- so without this
/// check the handle stayed "running" over a daemon that was not: a
/// subsequent `tc_subscribe` returned a nonzero, documented-as-success
/// token for a task that exits on its very first poll, and `status` kept
/// reporting a healthy daemon. An undetectable zombie. Reading the flag
/// here, rather than special-casing `"shutdown"` in `tc_call`, keeps one
/// definition of "stopped" for all three entry points.
fn shared_of(handle: &tc_handle) -> Option<Arc<ipc::DaemonShared>> {
    let running = handle.running.lock().unwrap_or_else(|p| p.into_inner());
    let shared = running.as_ref().map(|r| Arc::clone(&r.embedded.shared))?;
    if shared.shutdown.load(Ordering::Relaxed) {
        return None;
    }
    Some(shared)
}

/// Opaque preview handle returned by `tc_preview_open`.
pub struct tc_preview {
    body: CString,
    summary_json: CString,
}

/// Build a running `tc_handle` from an already-open `ConfigStore` -- the
/// common tail shared by `tc_daemon_start` and
/// `tc_daemon_start_with_settings` once each has finished its own preamble
/// (nothing, for the former; applying `settings_json`, for the latter). Kept
/// as one function so the two entry points cannot drift on how the runtime,
/// `start_embedded`, and the supervisor task get wired together.
fn start_daemon_handle(store: ConfigStore) -> anyhow::Result<tc_handle> {
    // A floor of two workers, not tokio's default of "available
    // parallelism, or whatever TOKIO_WORKER_THREADS says". With exactly one
    // worker, `stop_embedded`'s join-on-the-supervisor and an in-flight
    // `tc_subscribe` callback are mutually exclusive demands on that single
    // worker -- and `tc_daemon_stop` is documented as callable from inside a
    // callback, which makes that a circular wait. It has not hung in
    // practice only because `watcher::tick` runs its scan under
    // `block_in_place`, which makes tokio spin up a transient replacement
    // worker; whether one happens to exist at that instant is a coin flip,
    // not a guarantee. Two is the smallest number that makes it one.
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .max(2);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()?;
    let embedded = rt.block_on(trace_commons_contributor::daemon::start_embedded(store))?;
    let supervisor_shared = Arc::clone(&embedded.shared);
    let supervisor = rt.spawn(trace_commons_contributor::daemon::run_supervisor(
        supervisor_shared,
        false,
    ));
    Ok(tc_handle {
        rt,
        running: Mutex::new(Some(RunningDaemon {
            embedded,
            supervisor,
        })),
        subscriptions: Mutex::new(HashMap::new()),
        next_subscription: AtomicU64::new(1),
    })
}

/// Turn a successful/failed `tc_handle` build into the out-param + return
/// value shape every `tc_daemon_start*` entry point reports, so the two
/// cannot drift on it. Never forwards `result`'s underlying anyhow text:
/// both `ConfigStore::open` and `daemon::start_embedded` embed the
/// state-directory / lock-file path in their error context for
/// CLI/local-stderr consumption, and that must not cross this boundary. See
/// the module doc.
fn finish_daemon_start(result: anyhow::Result<tc_handle>, err: *mut *mut c_char) -> *mut tc_handle {
    match result {
        Ok(handle) => {
            let ptr = Box::into_raw(Box::new(handle));
            registry_insert(ptr as usize, AllocKind::Handle);
            ptr
        }
        Err(e) => {
            let label = start_failure_label(&e);
            set_last_error(label);
            if !err.is_null() {
                unsafe { *err = to_owned_cstring(label) };
            }
            std::ptr::null_mut()
        }
    }
}

/// Runs the daemon loop on its own thread with its own runtime.
///
/// Returns NULL and sets `*err` (if `err` is non-null) on failure -- most
/// notably when another daemon already holds the exclusive lock on
/// `config_dir`, per `daemon.lock`'s existing single-instance contract. A
/// second `tc_daemon_start` against the same directory must fail rather than
/// let two loops race the same on-disk queue.
///
/// # Safety
/// `config_dir` must be a valid, NUL-terminated UTF-8 C string (or NULL).
/// `err`, if non-null, must point to writable `*mut c_char` storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_daemon_start(
    config_dir: *const c_char,
    err: *mut *mut c_char,
) -> *mut tc_handle {
    let outcome = guard(|| {
        let store_result: anyhow::Result<ConfigStore> = (|| {
            let dir = unsafe { borrow_str(config_dir) }?;
            ConfigStore::open(std::path::PathBuf::from(dir))
        })();
        let store = match store_result {
            Ok(store) => store,
            Err(e) => return Ok(finish_daemon_start(Err(e), err)),
        };

        // Fail closed before anything can scan. `tc_daemon_start` takes no
        // settings, so undeclared roots here can only mean the persisted
        // file never declared them -- there is no argument that could have.
        if let Some(label) = roots_refusal(&store) {
            set_last_error(label);
            if !err.is_null() {
                unsafe { *err = to_owned_cstring(label) };
            }
            return Ok(std::ptr::null_mut());
        }

        let result = start_daemon_handle(store);
        // Everything from here on, including turning a failure into the
        // out-param the caller sees, stays inside `guard`'s catch_unwind:
        // `set_last_error` and `to_owned_cstring` are audited not to panic
        // themselves (see their doc comments), so nothing here can escape
        // it either.
        Ok(finish_daemon_start(result, err))
    });
    outcome.unwrap_or_else(|_| {
        // A genuine caught panic (not a business-logic error, which the
        // closure above always converts to `Ok` and never lets reach
        // here). `set_last_error`/`to_owned_cstring` are audited not to
        // panic (see their doc comments), so this cannot recurse.
        set_last_error("panic");
        if !err.is_null() {
            unsafe { *err = to_owned_cstring("panic") };
        }
        std::ptr::null_mut()
    })
}

/// Fixed labels `tc_daemon_start_with_settings` can report via `*err` /
/// `tc_last_error` for a failure specific to `settings_json`, distinct from
/// `ERR_DAEMON_START_FAILED` (which stays opaque for the reason stated on
/// `finish_daemon_start`). These are safe to surface verbatim: every one is
/// already a fixed, content-free label by construction, either synthesized
/// here or produced by `daemon::settings::apply_settings_object` (see its
/// doc for why *that* function's labels never carry a value, only ever one
/// of a small fixed set of field names).
const ERR_SETTINGS_INVALID_ENCODING: &str = "settings-invalid-encoding";
const ERR_SETTINGS_INVALID_JSON: &str = "settings-invalid-json";
const ERR_SETTINGS_LOAD_FAILED: &str = "settings-load-failed";
const ERR_SETTINGS_WRITE_FAILED: &str = "settings-write-failed";

/// Apply `settings_json` (if any) onto the settings persisted in `store`,
/// before `tc_daemon_start_with_settings` calls `start_daemon_handle` --
/// see that function's doc for why this ordering is the entire point: the
/// supervisor's first tick fires immediately on start, so anything this
/// crate wants in effect for that first tick must already be on disk before
/// `start_embedded` (via `DaemonShared::load` -> `DaemonSettings::load`)
/// reads it.
///
/// A NULL `settings_json`, or one that is empty after trimming ASCII
/// whitespace, is a no-op: identical to `tc_daemon_start`, by design.
///
/// # Safety
/// `settings_json`, if non-null, must be a valid, NUL-terminated C string.
unsafe fn apply_pre_start_settings(
    store: &ConfigStore,
    settings_json: *const c_char,
) -> Result<(), &'static str> {
    if settings_json.is_null() {
        return Ok(());
    }
    let text = unsafe { borrow_str(settings_json) }.map_err(|_| ERR_SETTINGS_INVALID_ENCODING)?;
    if text.trim().is_empty() {
        return Ok(());
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| ERR_SETTINGS_INVALID_JSON)?;
    let mut settings = DaemonSettings::load(store).map_err(|_| ERR_SETTINGS_LOAD_FAILED)?;
    let changed = apply_settings_object(&mut settings, &value)?;
    if changed {
        settings
            .save(store)
            .map_err(|_| ERR_SETTINGS_WRITE_FAILED)?;
    }
    Ok(())
}

/// Like `tc_daemon_start`, but applies `settings_json` -- a JSON object of
/// `DaemonSettings` fields -- to the persisted settings *before* starting
/// the daemon, so the supervisor's first tick (which fires immediately on
/// start; see `daemon::run_supervisor`) already observes the overridden
/// `claude_root` / `codex_root` rather than whatever was previously
/// persisted, or the conventional per-user default.
///
/// This closes a real gap: `tc_call(handle, "set_settings", ...)` only
/// works on an already-running daemon, by which point the first tick has
/// already scanned whatever was on disk before this call. A host that needs
/// the watcher to scan a non-default location from the very first pass --
/// most importantly a native application that wants to watch a relocated
/// session store, or a test harness that must never scan the real
/// `~/.claude` / `~/.codex` -- had no way to do that through this ABI
/// before this function existed, short of writing `daemon-settings.json`
/// onto disk by hand in the format `DaemonSettings::save` happens to use
/// today -- undocumented, unstable, and exactly the kind of thing a C ABI
/// exists to make unnecessary.
///
/// `settings_json` accepts exactly the fields `tc_call(handle,
/// "set_settings", ...)` does -- `quiescence_secs`, `digest_interval_secs`,
/// `approval_hold_secs`, `local_notifications`, `claude_root`,
/// `codex_root`, `max_uploads_per_day`, `max_bytes_per_day` -- validated by the
/// same function (`daemon::settings::apply_settings_object`), so there is
/// one definition of "a valid settings object", not two that can drift. An
/// unrecognized top-level key, or a recognized key holding the wrong JSON
/// type, is rejected with a fixed label rather than silently ignored: a
/// misspelled `claude_root` that was silently ignored would leave the
/// daemon watching the wrong directory with no signal to the host that
/// anything went wrong, which is precisely the failure mode this function
/// exists to close off.
///
/// `settings_json` may be NULL, or (after trimming ASCII whitespace) empty,
/// meaning "use whatever is currently persisted" -- identical to
/// `tc_daemon_start`. It is not otherwise optional: malformed JSON is a
/// fixed error label, never a panic.
///
/// Returns NULL and sets `*err` (if non-null) on failure. A `settings_json`
/// problem reports one of the fixed, content-free labels documented on
/// `apply_pre_start_settings` and `daemon::settings::apply_settings_object`
/// -- deliberately never `settings_json`'s own text, since it is the one
/// input to this function that may itself contain a filesystem path, which
/// is exactly the content this boundary must never echo back. Any other
/// failure (an unavailable `config_dir`, another daemon already holding the
/// lock) reports the same opaque `"daemon-start-failed"` `tc_daemon_start`
/// does, for the same reason.
///
/// The returned handle, on success, is exactly a `tc_daemon_start` handle:
/// it is freed the same way, by `tc_handle_free`, after `tc_daemon_stop`.
///
/// # Safety
/// `config_dir` and `settings_json` must each be a valid, NUL-terminated
/// UTF-8 C string, or NULL. `err`, if non-null, must point to writable
/// `*mut c_char` storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_daemon_start_with_settings(
    config_dir: *const c_char,
    settings_json: *const c_char,
    err: *mut *mut c_char,
) -> *mut tc_handle {
    let outcome = guard(|| {
        let store_result: anyhow::Result<ConfigStore> = (|| {
            let dir = unsafe { borrow_str(config_dir) }?;
            ConfigStore::open(std::path::PathBuf::from(dir))
        })();
        let store = match store_result {
            Ok(store) => store,
            Err(_) => {
                return Ok(finish_daemon_start(
                    Err(anyhow::anyhow!("open-failed")),
                    err,
                ));
            }
        };

        // Applied, and durably saved, BEFORE `start_daemon_handle` calls
        // `start_embedded`: `DaemonShared::load` reads settings from disk
        // at that point, and the supervisor's first tick -- which fires
        // immediately, not after the first poll interval -- reads them
        // from `DaemonShared`. There is no ordering in which applying this
        // in memory only, or after `start_embedded`, would still beat that
        // first tick.
        if let Err(label) = unsafe { apply_pre_start_settings(&store, settings_json) } {
            set_last_error(label);
            if !err.is_null() {
                unsafe { *err = to_owned_cstring(label) };
            }
            return Ok(std::ptr::null_mut());
        }

        // After the pre-start settings, not before: the roots screen's whole
        // purpose is to supply the declaration in this same call, so a
        // settings object that declares both roots must be allowed to clear
        // a refusal the persisted file alone would have earned.
        if let Some(label) = roots_refusal(&store) {
            set_last_error(label);
            if !err.is_null() {
                unsafe { *err = to_owned_cstring(label) };
            }
            return Ok(std::ptr::null_mut());
        }

        let result = start_daemon_handle(store);
        Ok(finish_daemon_start(result, err))
    });
    outcome.unwrap_or_else(|_| {
        set_last_error("panic");
        if !err.is_null() {
            unsafe { *err = to_owned_cstring("panic") };
        }
        std::ptr::null_mut()
    })
}

/// Tear down the embedded daemon (idempotent) from a dedicated OS thread
/// that carries no tokio context of its own, so blocking on it with a plain
/// `JoinHandle::join` is always safe regardless of the calling thread's own
/// context -- including from inside a `tc_subscribe` callback running on
/// this handle's own runtime, where calling `handle.rt.block_on(..)`
/// directly on the calling thread previously panicked ("cannot start a
/// runtime from within a runtime"); `guard` caught that panic, but by then
/// the panic had already unwound partway through freeing the handle in the
/// old design, destroying the runtime out from under the very thread
/// driving it (a segfault, reproduced in review). This function never frees
/// `handle`, so there is nothing for that unwind to corrupt even if the
/// dedicated thread's own teardown panics.
///
/// Exactly one caller wins the `running.take()` race and performs the
/// teardown; a second, concurrent caller sees `None` and returns
/// **immediately**, without waiting for the winner's teardown to finish.
/// That is a deliberately weaker guarantee than "stop returned means
/// teardown is complete", and it is the honest one -- an earlier revision
/// made the loser block on a `Condvar` until the winner finished, which
/// was unsound in two independent ways:
///
/// 1. **Use-after-free.** If the *winning* caller is `tc_handle_free`, it
///    runs `stop_embedded`, signals, and then `Box::from_raw(handle)` --
///    freeing the very `Mutex`/`Condvar` the loser is still parked inside
///    `wait_while` on and must re-acquire before it can return. The header
///    would have been advertising that as a safe combination.
/// 2. **Runtime deadlock.** `tc_daemon_stop` is documented as callable from
///    inside a `tc_subscribe` callback, i.e. from one of `handle.rt`'s own
///    worker threads. A callback thread that lost the race would park that
///    worker while the winner's teardown (joining the supervisor task)
///    needs a free worker to make progress. At `TOKIO_WORKER_THREADS=1`
///    neither call ever returns.
///
/// So the contract is: `tc_daemon_stop` is idempotent, but it is not a
/// teardown barrier for a *second concurrent* caller, and a caller must not
/// call `tc_handle_free` concurrently with `tc_daemon_stop`. Both are
/// stated in the header.
fn stop_embedded(handle: &tc_handle) {
    let running = handle
        .running
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    let Some(running) = running else {
        return;
    };
    let _ = std::thread::spawn(move || {
        let RunningDaemon {
            embedded,
            supervisor,
        } = running;
        embedded.shared.shutdown.store(true, Ordering::Relaxed);
        embedded.shared.shutdown_signal.notify_one();
        match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let _ = rt.block_on(supervisor);
            }
            Err(_) => {
                // No runtime to join the supervisor on: abort it outright
                // rather than dropping the `JoinHandle` and leaving it
                // detached-but-still-running. `JoinHandle::abort` needs no
                // executor of its own to call.
                supervisor.abort();
            }
        }
        embedded.close();
    })
    .join();
}

/// The fixed label the handle entry points below report for a pointer
/// that is not a live `tc_handle*`.
const ERR_INVALID_HANDLE_POINTER: &str = "invalid-handle-pointer";

/// The three other reasons `tc_subscribe` returns 0.
///
/// `tc_subscribe` returns 0 for a NULL handle, a non-live handle, a NULL
/// callback, and a stopped daemon. Only the non-live case recorded a label,
/// so a host following "token == 0, read `tc_last_error`" got a stale,
/// unrelated label for the other three. Labelling all four makes that
/// contract total.
const ERR_NULL_HANDLE: &str = "null-handle";
const ERR_NULL_CALLBACK: &str = "null-callback";
const ERR_DAEMON_NOT_RUNNING: &str = "daemon-not-running";

/// Validate a borrowed `tc_handle*` before any dereference, recording the
/// fixed label itself.
///
/// Mirrors [`preview_pointer_is_live`] exactly, for the same reason and
/// against the same threat. `tc_daemon_stop`, `tc_call`, `tc_subscribe`,
/// `tc_unsubscribe`, `tc_preview_open`, and `tc_preview_turns_json` each
/// null-checked `handle` and then dereferenced it, with nothing in
/// between confirming it was ever a live `tc_handle*` at all -- a stale
/// pointer (already freed by `tc_handle_free`) or a cross-type one (a
/// `tc_preview*` passed here by mistake) was a use-after-free or a type
/// confusion, not the fixed error the free functions and the preview
/// accessors already promise. `registry_is`, not `registry_take`: every
/// one of these six functions borrows the handle rather than consuming
/// it, exactly like the preview accessors borrow the preview.
///
/// This runs *outside* [`guard`] where the existing null check already
/// does (`tc_daemon_stop`, `tc_unsubscribe`), and immediately after the
/// null check but still before the first dereference where the null
/// check already lives inside a `guard`/`guard_forwarding` closure
/// (`tc_call`, `tc_subscribe`, `tc_preview_open`,
/// `tc_preview_turns_json`) -- in both places, strictly before any use of
/// `handle` as a reference. The reason is the same one
/// `preview_pointer_is_live` gives: `guard` discards the underlying error
/// text and substitutes `"operation-failed"`, which would hide the one
/// label a host needs to tell a stale or wrong-type handle from an
/// ordinary failure. It performs no dereference and cannot panic (the
/// registry mutex is poison-tolerant), so nothing is given up by running
/// it before the panic guard.
///
/// Carries the same two caveats `registry_is`'s own doc states: it cannot
/// make a concurrent free safe (this check and the dereference that
/// follows are not atomic with a racing `tc_handle_free`), and a freed
/// address can be reused by a later `tc_handle` allocation, in which case
/// the check passes for a pointer whose original object is gone. The
/// registry narrows accidental misuse to a clean error; it does not
/// replace the caller's ownership discipline.
fn handle_pointer_is_live(handle: *const tc_handle) -> bool {
    if registry_is(handle as usize, AllocKind::Handle) {
        return true;
    }
    set_last_error(ERR_INVALID_HANDLE_POINTER);
    false
}

/// Stop the daemon loop. Idempotent, and safe to call from any thread --
/// including from inside a `tc_subscribe` callback -- and safe to call
/// concurrently with `tc_call`/`tc_preview_open`/`tc_subscribe` on other
/// threads: those observe the daemon as stopped
/// (`{"error":{"code":"unavailable","message":"daemon-stopped"}}` /
/// `entry-id-invalid`-style failure) rather than dereferencing freed
/// memory, because this function does **not** free `handle`. Call
/// `tc_handle_free` once nothing else will use `handle` again to reclaim
/// it. Safe to call with NULL (no-op).
///
/// Detects and refuses a pointer that is not a live `tc_handle*` --
/// already freed by `tc_handle_free`, or a `tc_preview*` passed here by
/// mistake -- recording the fixed label `"invalid-handle-pointer"` via
/// `tc_last_error` and returning without dereferencing it.
///
/// Idempotent, but **not a teardown barrier for a second concurrent
/// caller**: if two threads call this at once, one performs the teardown
/// and the other returns immediately, possibly before the daemon has
/// actually finished stopping. Only the call that wins that race blocks
/// until teardown completes. See `stop_embedded`'s doc for why waiting
/// would be unsound rather than merely slower. A caller must **not** call
/// `tc_handle_free` concurrently with this function.
///
/// **Not** a synchronization point for `tc_subscribe` callbacks -- see
/// `tc_subscribe`'s doc. A callback can still be invoked, using `ctx`,
/// after this function has returned; only `tc_unsubscribe` guarantees
/// otherwise.
///
/// # Safety
/// `handle`, if non-null, must be a pointer previously returned by
/// `tc_daemon_start` and not already passed to `tc_handle_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_daemon_stop(handle: *mut tc_handle) {
    if handle.is_null() {
        return;
    }
    if !handle_pointer_is_live(handle) {
        return;
    }
    let _ = guard(|| {
        let handle_ref = unsafe { &*handle };
        stop_embedded(handle_ref);
        Ok(())
    });
}

/// Free a handle. This is the only function that reclaims the allocation
/// `tc_daemon_start` returned; `tc_daemon_stop` deliberately does not (see
/// its doc). Tears the daemon down first if `tc_daemon_stop` was not
/// already called. Safe to call with NULL (no-op). Detects and refuses a
/// double free or a pointer that is not a live `tc_handle*` (for instance a
/// `tc_preview*` passed here by mistake), recording a fixed label via
/// `tc_last_error` and leaking rather than acting on it -- the only
/// available response once the type of a raw pointer cannot be trusted.
///
/// Must be called from a plain thread that is not inside any tokio runtime
/// context -- in particular, never from inside a `tc_subscribe` callback.
/// `handle` owns its own `tokio::runtime::Runtime`; dropping a `Runtime`
/// from one of its own worker threads panics, and freeing `handle` is
/// exactly where that `Runtime` gets dropped. Calling from inside a runtime
/// context refuses instead of freeing (a deliberate, one-time leak of this
/// handle, safer than the crash freeing it would otherwise risk) and
/// records why via `tc_last_error`.
///
/// # Safety
/// `handle`, if non-null, must be a pointer previously returned by
/// `tc_daemon_start`, must not already have been passed to
/// `tc_handle_free`, and no other thread may be calling into it
/// concurrently with this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_handle_free(handle: *mut tc_handle) {
    if handle.is_null() {
        return;
    }
    if tokio::runtime::Handle::try_current().is_ok() {
        set_last_error("free-refused-inside-runtime-context");
        return;
    }
    if let Err(label) = registry_take(handle as usize, AllocKind::Handle) {
        set_last_error(label);
        return;
    }
    let _ = guard(|| {
        {
            let handle_ref = unsafe { &*handle };
            stop_embedded(handle_ref);
        }
        drop(unsafe { Box::from_raw(handle) });
        Ok(())
    });
}

/// Same request handlers the socket serves, called in-process. Returns a
/// NUL-terminated JSON response the caller owns; free with
/// [`tc_string_free`]. Never returns NULL: every failure mode, including a
/// NULL `handle`/`method`/`params_json`, is reported as a JSON error frame
/// rather than a null pointer or a crash.
///
/// A non-NULL `handle` that is not a live `tc_handle*` -- already freed,
/// or a `tc_preview*` passed here by mistake -- is refused the same way:
/// a JSON error frame (`bad_params` / `"invalid-handle-pointer"`) rather than a
/// dereference of a pointer this crate cannot trust the type of.
///
/// # Safety
/// `handle` must be a live pointer from `tc_daemon_start` (or NULL).
/// `method` and `params_json`, if non-null, must be valid NUL-terminated C
/// strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_call(
    handle: *mut tc_handle,
    method: *const c_char,
    params_json: *const c_char,
) -> *mut c_char {
    let outcome = guard(|| {
        let body: String = (|| {
            if handle.is_null() {
                return error_frame(ERR_BAD_PARAMS, "null-handle");
            }
            if !handle_pointer_is_live(handle) {
                return error_frame(ERR_BAD_PARAMS, ERR_INVALID_HANDLE_POINTER);
            }
            let handle = unsafe { &*handle };
            let method = match unsafe { borrow_str(method) } {
                Ok(m) => m,
                Err(_) => return error_frame(ERR_BAD_PARAMS, "invalid-method"),
            };
            let params_str = match unsafe { borrow_str(params_json) } {
                Ok(p) => p,
                Err(_) => return error_frame(ERR_BAD_PARAMS, "invalid-params"),
            };
            let params: serde_json::Value = match serde_json::from_str(params_str) {
                Ok(v) => v,
                Err(_) => return error_frame(ERR_BAD_PARAMS, "invalid-params-json"),
            };
            let Some(shared) = shared_of(handle) else {
                return error_frame("unavailable", "daemon-stopped");
            };
            let response = ipc::handle_local(&shared, method, params);
            serde_json::to_string(&response)
                .unwrap_or_else(|_| error_frame("unavailable", "serialize-failed"))
        })();
        Ok(to_owned_cstring(&body))
    });
    outcome.unwrap_or_else(|_| to_owned_cstring(&error_frame("unavailable", "panic")))
}

/// The instance an invite link names, as an owned UTF-8 string, or NULL if
/// the argument is not a usable invite.
///
/// This exists so a shell can satisfy the shared design spec's "resolve and
/// show the instance before committing" without being handed the invite's
/// code. `ParsedInvite` carries that code, and a shell cannot leak what it
/// was never given, so only the host crosses this boundary.
///
/// Not a daemon method, deliberately. Adding one would change the pinned
/// `METHODS` array that `hello` advertises and that
/// `hello_advertises_exactly_the_documented_method_set` holds to; this is a
/// pure function of its argument and touches no daemon state, so it belongs
/// on the binding rather than the protocol.
///
/// NULL covers every rejection, matching the single failure sentence the
/// spec gives the whole invite path: the caller must not distinguish "not a
/// URL" from "no code in it", because the interface does not.
///
/// # Safety
/// `invite`, if non-null, must be a valid NUL-terminated C string. The
/// returned pointer, if non-null, is owned by the caller and must be
/// released with [`tc_string_free`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_invite_issuer_host(invite: *const c_char) -> *mut c_char {
    let outcome = guard(|| {
        if invite.is_null() {
            return Ok(std::ptr::null_mut());
        }
        let Ok(raw) = (unsafe { borrow_str(invite) }) else {
            return Ok(std::ptr::null_mut());
        };
        match trace_commons_contributor::commands::invite_issuer_host(raw) {
            Some(host) => Ok(to_owned_cstring(&host)),
            None => Ok(std::ptr::null_mut()),
        }
    });
    outcome.unwrap_or(std::ptr::null_mut())
}

/// Register an event callback, invoked on a background thread with a JSON
/// event frame each time the daemon publishes one (queue changes, status
/// changes, digests due, and so on), until `tc_unsubscribe` is called with
/// the returned token. `ctx` is passed back unchanged.
///
/// **`tc_daemon_stop` does *not* end a subscription and is not a
/// synchronization point for it.** `tc_daemon_stop` only sets a flag this
/// subscription's background task polls at most every 250ms, and any event
/// already buffered when it was called can still be delivered in that
/// window -- a callback invocation can start, and be actively running,
/// after `tc_daemon_stop` has already returned to its caller. Only
/// `tc_unsubscribe` is a real barrier. See its doc, and
/// `tests::a_callback_can_still_fire_after_tc_daemon_stop_returns` in
/// `tests/abi.rs`, which exists specifically so this claim and the actual
/// behavior cannot silently drift apart again.
///
/// Subscribing happens synchronously, before this function returns, so an
/// event published immediately after `tc_subscribe` returns is never missed
/// waiting for the background task to start polling. A burst of more than
/// 256 events between deliveries is reported to the callback as a single
/// synthetic `{"event":"lagged","data":{"skipped":N}}` frame rather than
/// silently dropped with no signal.
///
/// Returns 0 on failure -- 0 is never a valid subscription token. On
/// success, returns a nonzero token identifying this subscription for
/// `tc_unsubscribe`.
///
/// Every zero return records a fixed label retrievable with
/// `tc_last_error`, so "token == 0, read `tc_last_error`" is a total
/// contract: `"null-handle"`, `"invalid-handle-pointer"` (not a live
/// `tc_handle*`), `"null-callback"`, or `"daemon-not-running"`.
///
/// The callback runs on a background thread and is not itself unwind-safe
/// across languages: a callback that panics on the Swift/C# side is that
/// side's problem, exactly as an FFI callback always is -- this crate
/// cannot catch a panic that never entered Rust.
///
/// # Safety
/// `handle` must be a live pointer from `tc_daemon_start`. `cb` must be a
/// valid function pointer for the lifetime of the subscription. `ctx`, if
/// non-null, **must remain valid until `tc_unsubscribe` returns -- full
/// stop.** `tc_daemon_stop` returning is not sufficient; see above.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_subscribe(
    handle: *mut tc_handle,
    cb: Option<extern "C" fn(event_json: *const c_char, ctx: *mut c_void)>,
    ctx: *mut c_void,
) -> u64 {
    let outcome = guard(|| {
        if handle.is_null() {
            set_last_error(ERR_NULL_HANDLE);
            return Ok(0u64);
        }
        if !handle_pointer_is_live(handle) {
            return Ok(0u64);
        }
        let handle_ref = unsafe { &*handle };
        let Some(cb) = cb else {
            set_last_error(ERR_NULL_CALLBACK);
            return Ok(0u64);
        };
        let Some(shared) = shared_of(handle_ref) else {
            set_last_error(ERR_DAEMON_NOT_RUNNING);
            return Ok(0u64);
        };
        // Raw pointers are not `Send`; `ctx` is a caller-supplied opaque
        // token the caller promised (per this function's safety contract)
        // stays valid, so it is sound to hand across the spawned task.
        struct SendPtr(*mut c_void);
        unsafe impl Send for SendPtr {}
        let ctx = SendPtr(ctx);

        // Subscribed here, synchronously, rather than inside the spawned
        // task: `broadcast::Receiver::subscribe` starts buffering from this
        // point, so an event published between this call returning and the
        // background task's first poll is still delivered instead of lost.
        let mut rx = shared.events.subscribe();

        let token = handle_ref.next_subscription.fetch_add(1, Ordering::Relaxed);
        let shutdown = Arc::clone(&shared);

        let jh = handle_ref.rt.spawn(async move {
            let ctx = ctx;
            loop {
                if shutdown.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                match tokio::time::timeout(std::time::Duration::from_millis(250), rx.recv()).await {
                    Ok(Ok(event)) => {
                        let json = serde_json::to_string(&event).unwrap_or_default();
                        if let Ok(c) = CString::new(json) {
                            cb(c.as_ptr(), ctx.0);
                        }
                    }
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => break,
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped))) => {
                        // A gap in the event stream is itself information
                        // the host needs -- silently continuing here would
                        // drop UI updates with no signal that anything was
                        // missed.
                        let json = serde_json::json!({
                            "event": "lagged",
                            "data": { "skipped": skipped },
                        })
                        .to_string();
                        if let Ok(c) = CString::new(json) {
                            cb(c.as_ptr(), ctx.0);
                        }
                    }
                    Err(_timeout) => continue,
                }
            }
        });
        handle_ref
            .subscriptions
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(token, jh);
        Ok(token)
    });
    outcome.unwrap_or(0)
}

/// Cancel a subscription returned by `tc_subscribe`. Blocks until the
/// subscription's background task has fully stopped before returning: a
/// caller that observes `tc_unsubscribe` return can rely on no further
/// invocation of that subscription's callback, not on an implementation
/// detail of how or when `handle`'s runtime happens to be torn down later.
/// This is the **only** function with that guarantee -- `tc_daemon_stop`
/// does not synchronize with subscriptions at all (see its doc); a callback
/// can still fire well after `tc_daemon_stop` has returned. `ctx` must stay
/// valid until `tc_unsubscribe` returns, full stop.
///
/// A no-op if `token` is 0 or unknown (already unsubscribed, or never
/// valid).
///
/// Also a no-op, recording the fixed label `"invalid-handle-pointer"` via
/// `tc_last_error`, if `handle` is non-null but not a live `tc_handle*`
/// -- refused before any dereference, the same as every other entry
/// point in this file.
///
/// Must be called from a plain thread that is not inside any tokio runtime
/// context -- in particular, never from inside a `tc_subscribe` callback,
/// including that subscription's own callback unsubscribing itself. Doing
/// so blocks joining a task that can only finish by returning from the very
/// callback frame making this call, which is a permanent hang: `abort()`
/// cannot preempt a callback already inside its synchronous invocation.
/// Calling from inside a runtime context refuses instead (a no-op; the
/// token stays valid, unlike `tc_handle_free`'s deliberate leak, since
/// nothing here was allocated) and records why via `tc_last_error`.
///
/// That check cannot distinguish true reentrancy from a host calling this
/// on a thread driving its own unrelated runtime, and since this function
/// returns `void` the refusal is **silent**. A binding author must check
/// `tc_last_error` after every `tc_unsubscribe` and treat a refusal as "the
/// barrier did not hold" -- retry from a plain thread before freeing `ctx`.
/// See the header's `tc_unsubscribe` entry.
///
/// # Safety
/// `handle` must be a live pointer from `tc_daemon_start` (or NULL, a
/// no-op).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_unsubscribe(handle: *mut tc_handle, token: u64) {
    if handle.is_null() {
        return;
    }
    // Liveness first, then the token. Checking `token == 0` alongside the
    // null check would let a freed or wrong-kind handle paired with a zero
    // token return silently, with no `tc_last_error` -- and this function's
    // own contract, and the header's, promise the label for every non-null
    // handle that is not live.
    if !handle_pointer_is_live(handle) {
        return;
    }
    if token == 0 {
        return;
    }
    if tokio::runtime::Handle::try_current().is_ok() {
        // Refuse without touching `subscriptions`: a callback calling
        // `tc_unsubscribe` on its own token from inside itself would
        // otherwise abort() (which cannot preempt a task mid-synchronous-
        // callback-invocation) and then block joining a task that can only
        // finish once this very callback frame returns -- a permanent hang
        // of the calling thread and a runtime worker. Any other thread
        // still inside some tokio context (this handle's or an unrelated
        // one elsewhere in the host process) is refused too, conservatively
        // -- the check cannot tell the two cases apart, and treating both
        // as "possibly-reentrant" is safe where treating both as "safe to
        // block" is not.
        set_last_error("unsubscribe-refused-inside-runtime-context");
        return;
    }
    let _ = guard(|| {
        let handle_ref = unsafe { &*handle };
        let jh = {
            let mut subs = handle_ref
                .subscriptions
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            subs.remove(&token)
        };
        if let Some(jh) = jh {
            jh.abort();
            // A dedicated OS thread and a plain `JoinHandle::join`, exactly
            // as `stop_embedded` uses -- safe to block on regardless of
            // this calling thread's own context, and the only way to give
            // "no callback after this returns" a real guarantee rather
            // than an incidental one.
            let _ = std::thread::spawn(move || {
                if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    let _ = rt.block_on(jh);
                }
            })
            .join();
        }
        Ok(())
    });
}

/// Read the session file and run the real redaction pipeline for one queue
/// entry, entirely in-process -- this is why `preview` exists as a C ABI
/// call rather than only a socket method: the redacted body does not fit
/// the socket's 1 MiB frame cap, and computing preview locally guarantees it
/// can never disagree with what an upload sends (both run
/// `daemon::preview::build_preview`).
///
/// Returns NULL and sets `*err` (if non-null) on failure -- most commonly an
/// unknown `entry_id`, or (deliberately) a redacted body that contains a
/// byte that cannot cross this boundary as `char*` (a NUL): preview exists
/// to show exactly what an upload would send, so failing outright rather
/// than silently editing that content is the only option that keeps that
/// promise.
///
/// A non-NULL `handle` that is not a live `tc_handle*` is refused the
/// same way, before any dereference: NULL plus `*err` set to the fixed
/// label `"invalid-handle-pointer"`.
///
/// # Safety
/// `handle` must be a live pointer from `tc_daemon_start`. `entry_id` must
/// be a valid NUL-terminated C string (or NULL). `err`, if non-null, must
/// point to writable `*mut c_char` storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_open(
    handle: *mut tc_handle,
    entry_id: *const c_char,
    err: *mut *mut c_char,
) -> *mut tc_preview {
    // `guard_forwarding`, not `guard`: every error this closure can
    // produce is already a fixed, content-free label --
    // `borrow_str`'s ("null-pointer"/"invalid-utf8"), this function's own
    // ("entry-id-invalid", "daemon-stopped", "body-contains-nul"), or
    // `ipc::open_preview`'s (already `&'static str` labels by
    // construction; see its doc comment in the daemon crate). Unlike
    // `tc_daemon_start`, nothing upstream of this call embeds a path.
    let outcome = guard_forwarding(|| {
        if handle.is_null() {
            anyhow::bail!("null-handle");
        }
        if !handle_pointer_is_live(handle) {
            anyhow::bail!("{ERR_INVALID_HANDLE_POINTER}");
        }
        let handle = unsafe { &*handle };
        let entry_id = unsafe { borrow_str(entry_id) }?;
        // Inferred as `uuid::Uuid` from `ipc::open_preview`'s signature
        // below -- the `uuid` crate is a transitive dependency (via
        // `trace-commons-contributor`), not a direct one this crate names,
        // per the brief's dependency list.
        let id = entry_id
            .parse()
            .map_err(|_| anyhow::anyhow!("entry-id-invalid"))?;
        let Some(shared) = shared_of(handle) else {
            anyhow::bail!("daemon-stopped");
        };
        // A dedicated OS thread with its own runtime, exactly as
        // `stop_embedded` and `tc_unsubscribe` use, and never
        // `handle.rt.block_on(..)` on the calling thread. This was the last
        // remaining reentrant `block_on` in the crate -- the same hazard
        // already fixed for `tc_daemon_stop`, and reproduced from a C host:
        // calling `tc_preview_open` from inside a `tc_subscribe` callback
        // (the most natural GUI flow there is -- receive `queue_changed`,
        // open the preview) runs on one of `handle.rt`'s own workers, where
        // tokio panics with "Cannot start a runtime from within a runtime".
        // `guard_forwarding` caught it and returned `err = "panic"`,
        // indistinguishable from a real internal panic, after tokio had
        // already dumped a backtrace to a signed menu-bar app's stderr.
        let preview = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "runtime-unavailable")?;
            rt.block_on(ipc::open_preview(&shared, id))
        })
        .join()
        .map_err(|_| anyhow::anyhow!("preview-thread-panicked"))?;
        let (summary, body) = preview.map_err(|label| anyhow::anyhow!("{label}"))?;
        // The body is content the contributor is being asked to approve
        // for upload; silently stripping a byte that cannot cross as
        // `char*` would make preview disagree with what actually gets
        // sent (see the module doc). Fail instead.
        let body = CString::new(body).map_err(|_| anyhow::anyhow!("body-contains-nul"))?;
        // Mapped to a fixed label rather than propagated via `?`, unlike
        // the code before this fix round: `guard_forwarding` forwards
        // whatever `Display` text reaches `guard_with`, and an unmapped
        // `serde_json::Error` here would not have been one of this
        // function's audited fixed labels.
        let summary_json = serde_json::to_string(&summary)
            .map_err(|_| anyhow::anyhow!("summary-serialize-failed"))?;
        let summary_json =
            CString::new(summary_json).map_err(|_| anyhow::anyhow!("summary-contains-nul"))?;
        Ok(tc_preview { body, summary_json })
    });
    match outcome {
        Ok(preview) => {
            let ptr = Box::into_raw(Box::new(preview));
            registry_insert(ptr as usize, AllocKind::Preview);
            ptr
        }
        Err(e) => {
            set_last_error(&e);
            if !err.is_null() {
                unsafe { *err = to_owned_cstring(&e) };
            }
            std::ptr::null_mut()
        }
    }
}

/// The turn index over a redacted preview body, as an owned JSON string:
/// `{entry_id, body_digest, envelope_digest, turn_count, turns: [{index,
/// role, tool_name, byte_offset, byte_len}]}`. Free it with
/// [`tc_string_free`]. Returns NULL and sets `*err` (if non-null, also
/// owned, also freed with `tc_string_free`) on failure.
///
/// An **overlay on the body, never a replacement for it.** A window renders
/// `tc_preview_body`'s bytes verbatim and draws a separator at each
/// `byte_offset`; the offsets index that exact string. Nothing here
/// re-renders the transcript, because a prose re-render would drop the
/// fields that have no prose form (`structured_payload`, `token_counts`,
/// `latency_ms`, `cost_usd`, `failure_modes`) and so would show a
/// contributor less than the artifact an approval covers.
///
/// `body_digest` is required and is the anchor: pass
/// `"sha256:<lowercase hex>"` over the exact UTF-8 bytes of the body being
/// displayed (the string `tc_preview_body` returned). A body the daemon
/// resolves that is not that one is refused with `preview-body-changed`
/// rather than indexed -- offsets against the wrong string still *look*
/// like a transcript, which is why this is not optional. Re-open the
/// preview, take the new body, and ask again.
///
/// Carries no redacted trace text: event-type labels, tool names the
/// envelope already records as metadata, and byte offsets.
///
/// A non-NULL `handle` that is not a live `tc_handle*` is refused the
/// same way, before any dereference: NULL plus `*err` set to the fixed
/// label `"invalid-handle-pointer"`.
///
/// # Safety
/// `handle` must be a live pointer from `tc_daemon_start`. `entry_id` and
/// `body_digest` must be valid NUL-terminated C strings (or NULL, which is
/// an error). `err`, if non-null, must point to writable `*mut c_char`
/// storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_turns_json(
    handle: *mut tc_handle,
    entry_id: *const c_char,
    body_digest: *const c_char,
    err: *mut *mut c_char,
) -> *mut c_char {
    // `guard_forwarding` for the same reason `tc_preview_open` uses it:
    // every error this closure can produce is already a fixed, content-free
    // label -- `borrow_str`'s, this function's own, or
    // `ipc::open_preview_turns`'s `&'static str` labels.
    let outcome = guard_forwarding(|| {
        if handle.is_null() {
            anyhow::bail!("null-handle");
        }
        if !handle_pointer_is_live(handle) {
            anyhow::bail!("{ERR_INVALID_HANDLE_POINTER}");
        }
        let handle = unsafe { &*handle };
        let entry_id = unsafe { borrow_str(entry_id) }?;
        let digest = unsafe { borrow_str(body_digest) }?.to_string();
        let id = entry_id
            .parse()
            .map_err(|_| anyhow::anyhow!("entry-id-invalid"))?;
        let Some(shared) = shared_of(handle) else {
            anyhow::bail!("daemon-stopped");
        };
        // A dedicated OS thread with its own runtime rather than
        // `handle.rt.block_on(..)`, for the reason spelled out in
        // `tc_preview_open`: the natural GUI flow calls this from inside a
        // `tc_subscribe` callback, which runs on one of `handle.rt`'s own
        // workers, where a nested `block_on` panics.
        let turns = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "runtime-unavailable")?;
            rt.block_on(ipc::open_preview_turns(&shared, id, &digest))
        })
        .join()
        .map_err(|_| anyhow::anyhow!("turns-thread-panicked"))?;
        turns.map_err(|label| anyhow::anyhow!("{label}"))
    });
    match outcome {
        Ok(json) => to_owned_cstring(&json),
        Err(e) => {
            set_last_error(&e);
            if !err.is_null() {
                unsafe { *err = to_owned_cstring(&e) };
            }
            std::ptr::null_mut()
        }
    }
}

/// The fixed label the borrowing preview accessors report for a pointer
/// that is not a live `tc_preview*`.
const ERR_INVALID_PREVIEW_POINTER: &str = "invalid-preview-pointer";

/// Validate a borrowed `tc_preview*` before any dereference, recording the
/// fixed label itself.
///
/// This runs *outside* [`guard`] on purpose: `guard` deliberately discards
/// the underlying error text and substitutes `"operation-failed"`, which
/// would hide the one label a host needs here to tell a bad pointer from an
/// ordinary failure. It performs no dereference and cannot panic (the
/// registry mutex is poison-tolerant), so nothing is given up by running it
/// before the panic guard.
fn preview_pointer_is_live(preview: *const tc_preview) -> bool {
    if registry_is(preview as usize, AllocKind::Preview) {
        return true;
    }
    set_last_error(ERR_INVALID_PREVIEW_POINTER);
    false
}

/// The redacted transcript, UTF-8. Borrowed: valid until `tc_preview_free`.
///
/// This is the one accessor in this ABI that deliberately carries trace
/// content, post-redaction, for an entry the caller already holds a preview
/// for. See the module doc's "The preview exemption" section.
///
/// Returns NULL and records a fixed `tc_last_error` label for a pointer
/// that is not a live `tc_preview*` (already freed, never ours, or another
/// kind of handle) -- see `registry_is` for what that check does and does
/// not guarantee.
///
/// # Safety
/// `preview` must be a live pointer from `tc_preview_open` (or NULL, which
/// returns NULL), and must not be freed concurrently by another thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_body(preview: *const tc_preview) -> *const c_char {
    if preview.is_null() {
        return std::ptr::null();
    }
    if !preview_pointer_is_live(preview) {
        return std::ptr::null();
    }
    let outcome = guard(|| {
        let preview = unsafe { &*preview };
        Ok(preview.body.as_ptr())
    });
    outcome.unwrap_or_else(|e| {
        set_last_error(&e);
        std::ptr::null()
    })
}

/// Counts, sizes, and the opening prompt, as JSON. Borrowed: valid until
/// `tc_preview_free`.
///
/// Returns NULL and records a fixed `tc_last_error` label for a pointer
/// that is not a live `tc_preview*` -- see `registry_is`.
///
/// # Safety
/// `preview` must be a live pointer from `tc_preview_open` (or NULL, which
/// returns NULL), and must not be freed concurrently by another thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_summary_json(preview: *const tc_preview) -> *const c_char {
    if preview.is_null() {
        return std::ptr::null();
    }
    if !preview_pointer_is_live(preview) {
        return std::ptr::null();
    }
    let outcome = guard(|| {
        let preview = unsafe { &*preview };
        Ok(preview.summary_json.as_ptr())
    });
    outcome.unwrap_or_else(|e| {
        set_last_error(&e);
        std::ptr::null()
    })
}

/// Search the redacted body for `needle`, a local scan over the in-memory
/// string (no protocol -- see the design's rationale for why preview is
/// in-process). Matches are non-overlapping, left-to-right, and reported as
/// UTF-8 **byte** offsets (not character offsets) into the body. An empty
/// `needle` matches nothing (returns 0, `*matches_json = "[]"`) rather than
/// every position.
///
/// Returns the number of matches, or -1 on error (including a match count
/// that overflows a 32-bit count, which is reported as an error rather than
/// silently truncated). On success, `*matches_json` is set to an owned JSON
/// array of byte offsets; free with `tc_string_free`. On error,
/// `*matches_json` (if non-null) is set to NULL -- there is nothing to
/// free.
///
/// Returns -1 and records a fixed `tc_last_error` label for a pointer that
/// is not a live `tc_preview*` -- see `registry_is`.
///
/// # Safety
/// `preview` must be a live pointer from `tc_preview_open`, and must not be
/// freed concurrently by another thread. `needle` must be a valid
/// NUL-terminated C string. `matches_json`, if non-null, must point to
/// writable `*mut c_char` storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_search(
    preview: *const tc_preview,
    needle: *const c_char,
    matches_json: *mut *mut c_char,
) -> i32 {
    if !preview.is_null() && !preview_pointer_is_live(preview) {
        if !matches_json.is_null() {
            unsafe { *matches_json = std::ptr::null_mut() };
        }
        return -1;
    }
    let outcome = guard(|| {
        if preview.is_null() {
            anyhow::bail!("null-preview");
        }
        let preview = unsafe { &*preview };
        let needle = unsafe { borrow_str(needle) }?;
        if needle.is_empty() {
            return Ok((0i32, "[]".to_string()));
        }
        let body = preview
            .body
            .to_str()
            .map_err(|_| anyhow::anyhow!("invalid-utf8"))?;
        let mut offsets = Vec::new();
        let mut start = 0usize;
        while let Some(pos) = body[start..].find(needle) {
            let abs = start + pos;
            offsets.push(abs);
            start = abs + needle.len().max(1);
            if start > body.len() {
                break;
            }
        }
        let count =
            i32::try_from(offsets.len()).map_err(|_| anyhow::anyhow!("too-many-matches"))?;
        let json = serde_json::to_string(&offsets)?;
        Ok((count, json))
    });
    match outcome {
        Ok((count, json)) => {
            if !matches_json.is_null() {
                unsafe { *matches_json = to_owned_cstring(&json) };
            }
            count
        }
        Err(e) => {
            set_last_error(&e);
            if !matches_json.is_null() {
                unsafe { *matches_json = std::ptr::null_mut() };
            }
            -1
        }
    }
}

/// Count occurrences of `needle` in an entry's PRE-redaction session text.
///
/// Returns the match count, or -1 on error. Reports a COUNT ONLY: no offsets,
/// no context, no bytes.
///
/// Takes a handle and an entry id rather than a `tc_preview*` deliberately.
/// `tc_preview` holds `body` and `summary_json`, both post-redaction, and must
/// not acquire pre-redaction bytes: hanging the raw session off the preview
/// would keep an unredacted transcript resident for as long as a sheet stays
/// open. The daemon reads the file, counts, and drops it.
///
/// # Safety
/// `handle` must be a live pointer from `tc_daemon_start` (or NULL, which
/// returns -1), and must not be freed concurrently by another thread.
/// `entry_id` and `needle` must be valid NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_search_original(
    handle: *mut tc_handle,
    entry_id: *const c_char,
    needle: *const c_char,
) -> i32 {
    let outcome = guard(|| {
        if handle.is_null() {
            anyhow::bail!("null-handle");
        }
        if !handle_pointer_is_live(handle) {
            anyhow::bail!("{ERR_INVALID_HANDLE_POINTER}");
        }
        let handle = unsafe { &*handle };
        // Type inferred from `ipc::search_original`'s signature below: the
        // `uuid` crate is a transitive dependency here, not a direct one this
        // crate names, so naming it would add a dependency. Same reasoning as
        // `tc_preview_open`.
        let entry_id = unsafe { borrow_str(entry_id) }?
            .parse()
            .map_err(|_| anyhow::anyhow!("entry-id-invalid"))?;
        let needle = unsafe { borrow_str(needle) }?.to_string();
        let Some(shared) = shared_of(handle) else {
            anyhow::bail!("daemon-stopped");
        };
        // A dedicated thread with its own runtime, for the same reason
        // `tc_preview_open` uses one: this is callable from inside a
        // `tc_subscribe` callback, where `block_on` on a runtime worker panics
        // with "Cannot start a runtime from within a runtime".
        let count = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(ipc::search_original(&shared, entry_id, &needle))
                .map_err(|label| anyhow::anyhow!("{label}"))
        })
        .join()
        .map_err(|_| anyhow::anyhow!("panic"))??;
        i32::try_from(count).map_err(|_| anyhow::anyhow!("too-many-matches"))
    });
    outcome.unwrap_or_else(|e| {
        set_last_error(&e);
        -1
    })
}

/// Free a preview handle. Safe to call with NULL (no-op). Invalidates every
/// `const char*` previously returned by `tc_preview_body` /
/// `tc_preview_summary_json` for this handle. Detects and refuses a double
/// free or a pointer that is not a live `tc_preview*` (for instance a
/// `tc_handle*` passed here by mistake), recording a fixed label via
/// `tc_last_error` and leaking rather than acting on it.
///
/// # Safety
/// `preview`, if non-null, must be a pointer previously returned by
/// `tc_preview_open` and not already freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_free(preview: *mut tc_preview) {
    if preview.is_null() {
        return;
    }
    if let Err(label) = registry_take(preview as usize, AllocKind::Preview) {
        set_last_error(label);
        return;
    }
    let _ = guard(|| {
        drop(unsafe { Box::from_raw(preview) });
        Ok(())
    });
}

/// Free a string returned by this library. Safe to call with NULL (no-op).
/// This is the only valid way to free any `char*` this crate returns; do
/// not free it with the caller's own allocator. Detects and refuses a
/// double free or a pointer this crate did not allocate as an owned
/// string (for instance a `tc_preview*`/`tc_handle*` passed here by
/// mistake), recording a fixed label via `tc_last_error` and leaking
/// rather than acting on it.
///
/// # Safety
/// `s`, if non-null, must be a pointer previously returned by a function in
/// this crate as an owned `char*`, and must not already have been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    if let Err(label) = registry_take(s as usize, AllocKind::String) {
        set_last_error(label);
        return;
    }
    let _ = guard(|| {
        drop(unsafe { CString::from_raw(s) });
        Ok(())
    });
}

/// The last error recorded on the calling thread, or NULL if none has been
/// recorded yet. Borrowed: valid until the next call, on this same thread,
/// to any function in this crate that records a new error.
#[unsafe(no_mangle)]
pub extern "C" fn tc_last_error() -> *const c_char {
    let outcome = guard(|| {
        LAST_ERROR
            .try_with(|cell| {
                cell.borrow()
                    .as_ref()
                    .map(|c| c.as_ptr())
                    .unwrap_or(std::ptr::null())
            })
            .map_err(|_| anyhow::anyhow!("tls-unavailable"))
    });
    // No further error recording here: this function IS the error-reporting
    // accessor, so failing to read the thread-local has nothing useful left
    // to report into.
    outcome.unwrap_or(std::ptr::null())
}

/// Borrow a `&str` from an incoming `const char*`, null-checked and
/// UTF-8-checked. Both failure modes are ordinary errors, not crashes, per
/// the non-negotiable safety rules: a caller passing NULL, or bytes that are
/// not valid UTF-8, gets an error string back.
///
/// # Safety
/// `ptr`, if non-null, must point to a valid, NUL-terminated C string whose
/// backing memory outlives the returned borrow.
unsafe fn borrow_str<'a>(ptr: *const c_char) -> anyhow::Result<&'a str> {
    if ptr.is_null() {
        anyhow::bail!("null-pointer");
    }
    let c = unsafe { CStr::from_ptr(ptr) };
    c.to_str().map_err(|_| anyhow::anyhow!("invalid-utf8"))
}

/// Describe the session stores on this machine, so a roots screen can ask
/// the contributor about something specific.
///
/// Needs no handle: it runs BEFORE any daemon exists, which is the whole
/// point -- the screen that uses it is the one clearing the refusal that
/// stops a daemon from starting.
///
/// Returns an owned JSON array; free it with [`tc_string_free`]. Each
/// element carries `source`, `path`, `exists`, `session_count`,
/// `most_recent` (RFC 3339 or null) and `relocated_by_env`.
///
/// This is the one place in this ABI that deliberately returns a filesystem
/// path. Everywhere else a path is withheld, because elsewhere the caller is
/// being told about a trace. Here the caller is the contributor's own
/// machine asking the contributor which of their own folders to watch, and a
/// consent prompt that will not name what it is asking about is not a
/// consent prompt. It reads directory entries and metadata only, and never
/// opens a session file.
///
/// Returns NULL only on a caught panic.
#[unsafe(no_mangle)]
pub extern "C" fn tc_discover_sources() -> *mut c_char {
    guard(|| {
        let found = trace_commons_contributor::source::discovery::probe_this_machine();
        let json = serde_json::to_string(&found).unwrap_or_else(|_| "[]".to_string());
        Ok(to_owned_cstring(&json))
    })
    .unwrap_or_else(|_| {
        set_last_error("panic");
        std::ptr::null_mut()
    })
}

/// Every fixed word on the routing surface, in one call.
///
/// Needs no handle: it describes the build, not a running daemon.
///
/// Returns an owned JSON object whose keys are `RoutingCopy`'s fields; free
/// it with [`tc_string_free`].
///
/// ONE CALL, NOT ONE PER STRING. `tc_scrub_detector_names` answers a single
/// question and returns a single list; this is a whole screen's wording and
/// must arrive as a set. Exporting the words one at a time would let a shell
/// take four of them and hand-write the fifth, and a hand-written word on
/// this surface is a privacy claim that silently stops matching the one the
/// other two shells print.
///
/// Returns NULL only on a caught panic.
#[unsafe(no_mangle)]
pub extern "C" fn tc_routing_copy() -> *mut c_char {
    guard(|| {
        let copy = trace_commons_contributor::routing_copy::routing_copy();
        let json = serde_json::to_string(&copy).unwrap_or_else(|_| "{}".to_string());
        Ok(to_owned_cstring(&json))
    })
    .unwrap_or_else(|_| {
        set_last_error("panic");
        std::ptr::null_mut()
    })
}

/// The C ABI's spelling of
/// [`trace_commons_contributor::routing_copy::ToolWiring`].
///
/// Anything outside 0..=1 is [`ToolWiring::Unknown`], which is the value
/// that claims nothing. That is deliberate rather than an error: a shell
/// built against a later header, or one that passed a value this build has
/// never heard of, must produce no verdict rather than a confident one.
fn tool_wiring_from_abi(value: i32) -> trace_commons_contributor::routing_copy::ToolWiring {
    use trace_commons_contributor::routing_copy::ToolWiring;
    match value {
        TC_TOOL_WIRING_WIRED => ToolWiring::Wired,
        TC_TOOL_WIRING_NOT_WIRED => ToolWiring::NotWired,
        _ => ToolWiring::Unknown,
    }
}

/// IronWire listed this tool and said it is pointed at a local address.
const TC_TOOL_WIRING_WIRED: i32 = 0;
/// IronWire listed this tool and said it is not.
const TC_TOOL_WIRING_NOT_WIRED: i32 = 1;

/// The tone values [`tc_routing_tool_tone`] and [`tc_routing_state_tone`]
/// answer in.
///
/// ONE NUMBERING FOR BOTH. A tool word can never be `HELD` -- only a daemon
/// state waits on something -- but giving the two calls separate numberings
/// would mean two `1`s meaning different things on one ABI, and a shell that
/// mapped the wrong one would mispaint a privacy claim rather than fail.
const TC_ROUTING_TONE_NEUTRAL: i32 = 0;
const TC_ROUTING_TONE_HELD: i32 = 1;
const TC_ROUTING_TONE_CLEAR: i32 = 2;

/// One tool's word, from what the contributor said about that tool's
/// sessions and what IronWire said about that tool.
///
/// `source_mode` is `get_settings`'s `*_source_mode` -- `off`, `watch` or
/// `unset`. `wiring` is `TC_TOOL_WIRING_*`; anything else is the unknown
/// state, which claims nothing.
///
/// THE BRANCH TABLE CROSSES, NOT ONLY THE WORDS. [`tc_routing_copy`] hands
/// a shell four words; without this call each shell also decides which of
/// the four a tool gets, and three native copies of that decision can drift
/// apart silently while every string stays identical. The words could not
/// drift; the branching could, in three places, and nothing in this repo
/// would have noticed.
///
/// Pair every call with [`tc_routing_tool_tone`] rather than comparing the
/// returned word against the private one. `Private` is a substring of the
/// denial that must never come back.
///
/// Returns an owned string; free it with [`tc_string_free`]. Returns NULL
/// for a NULL or non-UTF-8 `source_mode`, recording `null-pointer` or
/// `invalid-utf8`: a shell that cannot say what the contributor declared
/// should get no word rather than one built on a guess.
///
/// # Safety
/// `source_mode` must point to a valid, NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_routing_tool_word(
    source_mode: *const c_char,
    wiring: i32,
) -> *mut c_char {
    guard_forwarding(|| {
        let source_mode = unsafe { borrow_str(source_mode) }?;
        Ok(to_owned_cstring(
            trace_commons_contributor::routing_copy::tool_word(
                source_mode,
                tool_wiring_from_abi(wiring),
            ),
        ))
    })
    .unwrap_or_else(|err| {
        set_last_error(&err);
        std::ptr::null_mut()
    })
}

/// How the word [`tc_routing_tool_word`] returned is painted:
/// `TC_ROUTING_TONE_NEUTRAL` or `TC_ROUTING_TONE_CLEAR`.
///
/// Takes the same two inputs as the word, so the two stay in step by
/// construction. **A shell must not recover this by comparing the rendered
/// word against the private one** -- that is a text comparison against a
/// privacy claim, and `Private` is a substring of `Not private`.
///
/// Answers `TC_ROUTING_TONE_NEUTRAL` -- the tone that claims nothing -- for a
/// NULL or non-UTF-8 `source_mode` and on a caught panic. There is no
/// failure value: a styling call that returned an error would leave a shell
/// choosing a tone for itself, which is the thing this exists to stop.
///
/// # Safety
/// `source_mode` must point to a valid, NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_routing_tool_tone(source_mode: *const c_char, wiring: i32) -> i32 {
    use trace_commons_contributor::routing_copy::ToolTone;
    guard(|| {
        let Ok(source_mode) = (unsafe { borrow_str(source_mode) }) else {
            return Ok(TC_ROUTING_TONE_NEUTRAL);
        };
        Ok(
            match trace_commons_contributor::routing_copy::tool_tone(
                source_mode,
                tool_wiring_from_abi(wiring),
            ) {
                ToolTone::Neutral => TC_ROUTING_TONE_NEUTRAL,
                ToolTone::Clear => TC_ROUTING_TONE_CLEAR,
            },
        )
    })
    .unwrap_or(TC_ROUTING_TONE_NEUTRAL)
}

/// The daemon's routing state, in words.
///
/// Exported for the same reason [`tc_routing_tool_word`] is: the sentences
/// were already shared, but the mapping from `awaiting_rows` / `rows_seen`
/// / anything-else onto them was written out again in each shell, and three
/// copies of a branch can disagree while three copies of a string cannot.
///
/// A state this build has never heard of -- and a NULL or non-UTF-8 `state`
/// -- reads as the off line, which claims nothing. It never falls through
/// to either "on" sentence.
///
/// Returns an owned string; free it with [`tc_string_free`]. NULL only on a
/// caught panic.
///
/// # Safety
/// `state`, if non-null, must point to a valid, NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_routing_state_line(state: *const c_char) -> *mut c_char {
    guard(|| {
        // An unreadable state is a state this build does not know, and the
        // rule for those is already the safe one: say what off says.
        let state = if state.is_null() {
            ""
        } else {
            unsafe { borrow_str(state) }.unwrap_or("")
        };
        Ok(to_owned_cstring(
            trace_commons_contributor::routing_copy::ironwire_state_line(state),
        ))
    })
    .unwrap_or_else(|_| {
        set_last_error("panic");
        std::ptr::null_mut()
    })
}

/// How firmly the sentence [`tc_routing_state_line`] returned reads:
/// `TC_ROUTING_TONE_NEUTRAL`, `_HELD` or `_CLEAR`.
///
/// Exported for the reason the sentence is. This was the last routing branch
/// table still written out natively in all three shells -- `routing_tone` in
/// GTK, `tone(forState:)` in Swift, `StateTone` in C# -- three copies of one
/// decision that agreed today and could drift apart in silence tomorrow.
///
/// None of the three states is a fault, and none of them can reach a fault
/// tone through here: `awaiting_rows` is `HELD` and never an error, because a
/// reader built a moment ago starts empty by construction and that is the
/// state a contributor sees immediately after touching anything on this card.
///
/// Answers `TC_ROUTING_TONE_NEUTRAL` -- the tone that claims nothing -- for a
/// state this build has never heard of, for a NULL or non-UTF-8 `state`, and
/// on a caught panic. There is no failure value, for the reason on
/// [`tc_routing_tool_tone`].
///
/// # Safety
/// `state`, if non-null, must point to a valid, NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_routing_state_tone(state: *const c_char) -> i32 {
    use trace_commons_contributor::routing_copy::StateTone;
    guard(|| {
        // An unreadable state is a state this build does not know, and the
        // rule for those is already the safe one -- the same rule, and the
        // same fallback, as `tc_routing_state_line`.
        let state = if state.is_null() {
            ""
        } else {
            unsafe { borrow_str(state) }.unwrap_or("")
        };
        Ok(
            match trace_commons_contributor::routing_copy::ironwire_state_tone(state) {
                StateTone::Neutral => TC_ROUTING_TONE_NEUTRAL,
                StateTone::Held => TC_ROUTING_TONE_HELD,
                StateTone::Clear => TC_ROUTING_TONE_CLEAR,
            },
        )
    })
    .unwrap_or(TC_ROUTING_TONE_NEUTRAL)
}

/// The routing surface's "that file could not be used" sentence, assembled.
///
/// `token_path` may be NULL, which is the case where nothing resolved at
/// all; the sentence for that says what to do instead of naming a file it
/// does not have.
///
/// ASSEMBLED HERE, DELIBERATELY. The alternative -- exporting a template
/// with a `{path}` in it and letting each shell format it -- would make the
/// shells a fourth, fifth and sixth place this wording lives, each free to
/// drop a clause around the hole, and nothing in this repo would notice.
/// The sweep in `routing_copy` renders these sentences and checks them; it
/// can only do that for sentences finished on this side.
///
/// Returns an owned string; free it with [`tc_string_free`]. NULL only on a
/// caught panic.
///
/// # Safety
/// `token_path`, if non-null, must point to a valid, NUL-terminated C
/// string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_routing_token_line(token_path: *const c_char) -> *mut c_char {
    guard(|| {
        // A NULL path is the "nothing resolved" case and not an error. Bytes
        // that are not UTF-8 are treated the same way: this sentence exists
        // to tell somebody what to do next, and refusing to produce it
        // because a path is oddly encoded would leave the screen silent.
        let path = if token_path.is_null() {
            None
        } else {
            unsafe { borrow_str(token_path) }.ok()
        };
        Ok(to_owned_cstring(
            &trace_commons_contributor::routing_copy::ironwire_token_line(path),
        ))
    })
    .unwrap_or_else(|_| {
        set_last_error("panic");
        std::ptr::null_mut()
    })
}

/// The routing surface's "nothing answered" sentence, assembled.
///
/// `port` outside 1..=65535 -- including the 0 a caller passes for "no port
/// was tried" -- produces the sentence that names no port, rather than one
/// that names a port number nobody used.
///
/// Assembled here for the reason on [`tc_routing_token_line`].
///
/// Returns an owned string; free it with [`tc_string_free`]. NULL only on a
/// caught panic.
#[unsafe(no_mangle)]
pub extern "C" fn tc_routing_unreachable_line(port: i32) -> *mut c_char {
    guard(|| {
        let port = u16::try_from(port).ok().filter(|p| *p != 0);
        Ok(to_owned_cstring(
            &trace_commons_contributor::routing_copy::ironwire_unreachable_line(port),
        ))
    })
    .unwrap_or_else(|_| {
        set_last_error("panic");
        std::ptr::null_mut()
    })
}

/// The routing surface's discovery sentence, assembled.
///
/// `port` is what `discover_routing` reported, or 0 for a machine that
/// published no pointer. A port outside 1..=65535 is treated as 0 -- the
/// sentence for "nothing was discovered", which names no port rather than
/// naming one nothing published.
///
/// **A machine with no pointer is not an error**, and neither branch of
/// this sentence says otherwise. It is the ordinary state of a machine
/// without IronWire, which is most of them, and there is nothing for a
/// caller to handle: both branches are a real sentence.
///
/// Assembled here for the reason on [`tc_routing_token_line`].
///
/// Returns an owned string; free it with [`tc_string_free`]. NULL only on a
/// caught panic.
#[unsafe(no_mangle)]
pub extern "C" fn tc_routing_discovery_line(port: i32) -> *mut c_char {
    guard(|| {
        let port = u16::try_from(port).ok().filter(|p| *p != 0);
        Ok(to_owned_cstring(
            &trace_commons_contributor::routing_copy::ironwire_discovery_line(port),
        ))
    })
    .unwrap_or_else(|_| {
        set_last_error("panic");
        std::ptr::null_mut()
    })
}

/// The routing surface's "Last checked ..." sentence, assembled.
///
/// `when` is the shell's own humanised time -- "an hour ago", "yesterday".
/// That is the one piece of this surface each shell renders for itself,
/// because it is a rendering of a timestamp and not wording about routing.
/// The words around it are still written once, here.
///
/// A NULL or non-UTF-8 `when` returns NULL and records an error: unlike the
/// two sentences above there is no meaningful shorter form of this one --
/// "Last checked " with nothing after it is worse than no line at all -- and
/// a shell that has no timestamp should not be calling it.
///
/// Uses [`guard_forwarding`] rather than [`guard`], which the rule on that
/// function permits here: the closure's only error paths are
/// [`borrow_str`]'s two fixed labels, `null-pointer` and `invalid-utf8`.
/// Neither embeds any caller content, so forwarding them is exactly as safe
/// as the fixed label, and a shell can tell the two apart.
///
/// Returns an owned string; free it with [`tc_string_free`].
///
/// # Safety
/// `when` must point to a valid, NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_routing_last_checked(when: *const c_char) -> *mut c_char {
    guard_forwarding(|| {
        let when = unsafe { borrow_str(when) }?;
        Ok(to_owned_cstring(
            &trace_commons_contributor::routing_copy::last_checked_line(when),
        ))
    })
    .unwrap_or_else(|err| {
        set_last_error(&err);
        std::ptr::null_mut()
    })
}

/// The settings screen's session-source row for one tool, assembled.
///
/// `tool` is `claude`, `codex` or `gemini`. `source_mode` is
/// `get_settings`'s `*_source_mode` -- `watch`, `off` or `unset`.
///
/// THREE MODES, THREE SENTENCES. `*_root_configured` is `mode == "watch"`
/// and is therefore false for both `off` and `unset`; a shell that branches
/// on it tells a contributor who declared a tool OFF that their sessions are
/// being read from the usual place, which is false in the fail-open
/// direction on the one screen they would check. A shell must call this with
/// the mode word and render what comes back, not derive a second branch from
/// the boolean.
///
/// Assembled here for the reason on [`tc_routing_token_line`]. Do not
/// reassemble it from parts, and do not build the `off` line as the `unset`
/// line with a "not" in front: no word on this surface may deny a privacy
/// claim another word makes.
///
/// A mode this build does not know reads as `unset`, deliberately -- see
/// `source_copy::source_check_line`. A `tool` this build does not know is an
/// error, because there is no safe sentence for a tool with no name.
///
/// Returns an owned string; free it with [`tc_string_free`]. NULL with
/// `unknown-source-tool`, `null-pointer`, `invalid-utf8` or `panic` on
/// [`tc_last_error`].
///
/// Uses [`guard_forwarding`], which the rule on that function permits here:
/// every error label is fixed and none embeds caller content.
///
/// # Safety
/// `tool` and `source_mode` must point to valid, NUL-terminated C strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_source_check_line(
    tool: *const c_char,
    source_mode: *const c_char,
) -> *mut c_char {
    guard_forwarding(|| {
        let tool = unsafe { borrow_str(tool) }?;
        let source_mode = unsafe { borrow_str(source_mode) }?;
        let tool = trace_commons_contributor::source_copy::SourceTool::from_key(tool)
            .ok_or_else(|| anyhow::anyhow!("unknown-source-tool"))?;
        Ok(to_owned_cstring(
            &trace_commons_contributor::source_copy::source_check_line(tool, source_mode),
        ))
    })
    .unwrap_or_else(|err| {
        set_last_error(&err);
        std::ptr::null_mut()
    })
}

/// The names of the secret detectors the scrubber runs, so a shell can tell
/// a contributor what is removed without transcribing the list.
///
/// Needs no handle: it describes the build, not a running daemon, and the
/// screen that asks is the first one a contributor sees.
///
/// Returns an owned JSON array of strings; free it with [`tc_string_free`].
///
/// NAMES ONLY. The patterns are deliberately not exposed here and must not
/// be added: a contributor deciding whether to trust the scrubbing needs to
/// know what it looks for, but publishing the regexes would tell someone
/// trying to slip a secret past it exactly what to avoid. The generated list
/// is the point -- a hand-written copy in a shell is a privacy claim that
/// silently stops being true the day a detector is added.
///
/// Returns NULL only on a caught panic.
#[unsafe(no_mangle)]
pub extern "C" fn tc_scrub_detector_names() -> *mut c_char {
    guard(|| {
        let names = trace_commons_protocol::trace_contribution::secret_leak_pattern_names();
        let json = serde_json::to_string(&names).unwrap_or_else(|_| "[]".to_string());
        Ok(to_owned_cstring(&json))
    })
    .unwrap_or_else(|_| {
        set_last_error("panic");
        std::ptr::null_mut()
    })
}
