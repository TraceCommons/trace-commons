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
//! # Panic safety
//!
//! Every exported function wraps its body in [`std::panic::catch_unwind`]
//! via the [`guard`] helper. A Rust panic must never unwind across the FFI
//! boundary into Swift, C#, or C -- that is undefined behaviour on every one
//! of those callers. A caught panic becomes an ordinary error string.
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
//! language boundary a GUI might log or display. Rather than rewrite those
//! internal messages, this crate never forwards their `Display` text: it
//! converts every failure from those call sites to one of a small set of
//! fixed labels chosen here.

// `tc_handle` / `tc_preview` are named to match the C header
// (`include/trace_commons.h`) and the Swift/C# callers that bind to it
// verbatim, not Rust naming conventions.
#![allow(non_camel_case_types)]

use std::ffi::{CStr, CString, c_char, c_void};
use std::panic::UnwindSafe;
use std::sync::Mutex;

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::EmbeddedDaemon;
use trace_commons_contributor::daemon::ipc::{self, ERR_BAD_PARAMS};

/// Catches a panic and turns any failure -- panic or ordinary `Err` -- into
/// a `String`. Never let a panic propagate past this: the whole point is
/// that nothing may unwind into the caller's language runtime.
///
/// Kept generic exactly as sketched in the design brief; callers that need
/// to keep the underlying `anyhow::Error`'s `Display` text off the wire (it
/// may embed a filesystem path -- see the module doc) discard `Err(e)` here
/// and substitute a fixed label of their own instead of propagating it.
fn guard<T>(f: impl FnOnce() -> anyhow::Result<T> + UnwindSafe) -> Result<T, String> {
    match std::panic::catch_unwind(f) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{e:#}")),
        // A Rust panic must never unwind into Swift, C#, or C.
        Err(_) => Err("panic".to_string()),
    }
}

thread_local! {
    static LAST_ERROR: std::cell::RefCell<Option<CString>> = const { std::cell::RefCell::new(None) };
}

/// Records `label` as this thread's last error and returns a borrowed
/// pointer into it, matching [`tc_last_error`]'s contract: valid until the
/// next call on this thread that sets it again.
fn set_last_error(label: &str) {
    let c = CString::new(sanitize_for_c(label)).unwrap_or_else(|_| CString::new("error").unwrap());
    LAST_ERROR.with(|cell| *cell.borrow_mut() = Some(c));
}

/// Strips embedded NULs, which cannot appear in a C string; nothing else in
/// a fixed label ever needs stripping.
fn sanitize_for_c(s: &str) -> String {
    s.replace('\0', "")
}

/// Allocate an owned, caller-freed C string from `s`, on the caller's
/// behalf. Used for every `char*` this crate returns.
fn to_owned_cstring(s: &str) -> *mut c_char {
    CString::new(sanitize_for_c(s))
        .unwrap_or_else(|_| CString::new("encoding-error").unwrap())
        .into_raw()
}

/// Write `msg` through an out-param error pointer, if the caller gave us
/// one. Null-checked: a caller that passes NULL for `err` (it is optional
/// per the header) must not be dereferenced.
///
/// # Safety
/// `err`, if non-null, must point to valid, writable `*mut c_char` storage.
unsafe fn set_out_err(err: *mut *mut c_char, msg: &str) {
    set_last_error(msg);
    if !err.is_null() {
        unsafe {
            *err = to_owned_cstring(msg);
        }
    }
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

/// A fixed, safe label for any failure whose underlying `anyhow::Error`
/// might embed a filesystem path (state-directory or lock-file operations).
/// See the module doc's "What never crosses this boundary" section.
const ERR_DAEMON_START_FAILED: &str = "daemon-start-failed";

/// Build a `Response`-shaped JSON error frame with a fixed code, matching
/// the shape `trace_commons_contributor::daemon::ipc::Response` serializes
/// to, for failures this crate must synthesize itself (malformed params,
/// null pointers) rather than ones produced by `handle_local`.
fn error_frame(code: &str, message: &str) -> String {
    serde_json::json!({
        "id": 0,
        "error": { "code": code, "message": message },
    })
    .to_string()
}

/// The daemon handle: an owned tokio runtime plus the running daemon's
/// shared state and background task handles.
///
/// `tc_daemon_start` returns `Box::into_raw(Box::new(tc_handle { .. }))`;
/// every other function taking a `*mut tc_handle` borrows it and
/// `tc_daemon_stop` is the only function that consumes and frees it
/// (`Box::from_raw`).
pub struct tc_handle {
    rt: tokio::runtime::Runtime,
    // `None` only ever transiently, while `tc_daemon_stop` is consuming it.
    embedded: Mutex<Option<EmbeddedDaemon>>,
}

/// Opaque preview handle returned by `tc_preview_open`.
pub struct tc_preview {
    body: CString,
    summary_json: CString,
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
        let dir = unsafe { borrow_str(config_dir) }?;
        let store = ConfigStore::open(std::path::PathBuf::from(dir))?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let embedded = rt.block_on(trace_commons_contributor::daemon::start_embedded(
            store, false,
        ))?;
        Ok(tc_handle {
            rt,
            embedded: Mutex::new(Some(embedded)),
        })
    });
    match outcome {
        Ok(handle) => Box::into_raw(Box::new(handle)),
        Err(_) => {
            // Never forward the underlying anyhow text here: both
            // `ConfigStore::open` and `daemon::start_embedded` embed the
            // state-directory / lock-file path in their error context for
            // CLI/local-stderr consumption, and that must not cross this
            // boundary. See the module doc.
            unsafe { set_out_err(err, ERR_DAEMON_START_FAILED) };
            std::ptr::null_mut()
        }
    }
}

/// Stop the daemon loop and free the handle. Safe to call with NULL (no-op).
///
/// # Safety
/// `handle`, if non-null, must be a pointer previously returned by
/// `tc_daemon_start` and not already passed to `tc_daemon_stop`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_daemon_stop(handle: *mut tc_handle) {
    if handle.is_null() {
        return;
    }
    let _ = guard(|| {
        let handle = unsafe { Box::from_raw(handle) };
        let embedded = handle
            .embedded
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some(embedded) = embedded {
            let _ = handle.rt.block_on(embedded.shutdown());
        }
        Ok(())
    });
}

/// Same request handlers the socket serves, called in-process. Returns a
/// NUL-terminated JSON response the caller owns; free with
/// [`tc_string_free`]. Never returns NULL: every failure mode, including a
/// NULL `handle`/`method`/`params_json`, is reported as a JSON error frame
/// rather than a null pointer or a crash.
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
        if handle.is_null() {
            return Ok(error_frame(ERR_BAD_PARAMS, "null-handle"));
        }
        let handle = unsafe { &*handle };
        let method = match unsafe { borrow_str(method) } {
            Ok(m) => m,
            Err(_) => return Ok(error_frame(ERR_BAD_PARAMS, "invalid-method")),
        };
        let params_str = match unsafe { borrow_str(params_json) } {
            Ok(p) => p,
            Err(_) => return Ok(error_frame(ERR_BAD_PARAMS, "invalid-params")),
        };
        let params: serde_json::Value = match serde_json::from_str(params_str) {
            Ok(v) => v,
            Err(_) => return Ok(error_frame(ERR_BAD_PARAMS, "invalid-params-json")),
        };
        let shared = {
            let guard = handle.embedded.lock().unwrap_or_else(|p| p.into_inner());
            match guard.as_ref() {
                Some(e) => std::sync::Arc::clone(&e.shared),
                None => return Ok(error_frame("unavailable", "daemon-stopped")),
            }
        };
        let response = ipc::handle_local(&shared, method, params);
        Ok(serde_json::to_string(&response)
            .unwrap_or_else(|_| error_frame("unavailable", "serialize-failed")))
    });
    let body = outcome.unwrap_or_else(|e| error_frame("unavailable", &e));
    to_owned_cstring(&body)
}

/// Register an event callback, invoked on a background thread with a JSON
/// event frame each time the daemon publishes one (queue changes, status
/// changes, digests due, and so on). `ctx` is passed back unchanged.
///
/// The callback runs until the daemon is stopped; it is not itself
/// unwind-safe across languages, so a callback that panics on the Swift/C#
/// side is that side's problem, exactly as an FFI callback always is -- this
/// crate cannot catch a panic that never entered Rust.
///
/// # Safety
/// `handle` must be a live pointer from `tc_daemon_start`. `cb` must be a
/// valid function pointer for the lifetime of the subscription. `ctx`, if
/// non-null, must remain valid for as long as `cb` may be invoked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_subscribe(
    handle: *mut tc_handle,
    cb: Option<extern "C" fn(event_json: *const c_char, ctx: *mut c_void)>,
    ctx: *mut c_void,
) {
    let _ = guard(|| {
        if handle.is_null() {
            return Ok(());
        }
        let handle = unsafe { &*handle };
        let Some(cb) = cb else {
            return Ok(());
        };
        let shared = {
            let guard = handle.embedded.lock().unwrap_or_else(|p| p.into_inner());
            match guard.as_ref() {
                Some(e) => std::sync::Arc::clone(&e.shared),
                None => return Ok(()),
            }
        };
        // Raw pointers are not `Send`; `ctx` is a caller-supplied opaque
        // token the caller promised (per this function's safety contract)
        // stays valid, so it is sound to hand across the spawned task.
        struct SendPtr(*mut c_void);
        unsafe impl Send for SendPtr {}
        let ctx = SendPtr(ctx);

        handle.rt.spawn(async move {
            let ctx = ctx;
            let mut rx = shared.events.subscribe();
            loop {
                if shared.shutdown.load(std::sync::atomic::Ordering::Relaxed) {
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
                    Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
                    Err(_timeout) => continue,
                }
            }
        });
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
/// unknown `entry_id`.
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
    let outcome = guard(|| {
        if handle.is_null() {
            anyhow::bail!("null-handle");
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
        let shared = {
            let guard = handle.embedded.lock().unwrap_or_else(|p| p.into_inner());
            match guard.as_ref() {
                Some(e) => std::sync::Arc::clone(&e.shared),
                None => anyhow::bail!("daemon-stopped"),
            }
        };
        let (summary, body) = handle
            .rt
            .block_on(ipc::open_preview(&shared, id))
            .map_err(|label| anyhow::anyhow!("{label}"))?;
        let summary_json = serde_json::to_string(&summary)?;
        Ok(tc_preview {
            body: CString::new(sanitize_for_c(&body))?,
            summary_json: CString::new(sanitize_for_c(&summary_json))?,
        })
    });
    match outcome {
        Ok(preview) => Box::into_raw(Box::new(preview)),
        Err(e) => {
            // `open_preview`'s errors are already fixed labels (see its
            // doc comment in the daemon crate), so forwarding them here is
            // safe -- unlike `tc_daemon_start`, nothing upstream of this
            // call embeds a path.
            unsafe { set_out_err(err, &e) };
            std::ptr::null_mut()
        }
    }
}

/// The redacted transcript, UTF-8. Borrowed: valid until `tc_preview_free`.
///
/// # Safety
/// `preview` must be a live pointer from `tc_preview_open` (or NULL, which
/// returns NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_body(preview: *const tc_preview) -> *const c_char {
    if preview.is_null() {
        return std::ptr::null();
    }
    let preview = unsafe { &*preview };
    preview.body.as_ptr()
}

/// Counts, sizes, and the opening prompt, as JSON. Borrowed: valid until
/// `tc_preview_free`.
///
/// # Safety
/// `preview` must be a live pointer from `tc_preview_open` (or NULL, which
/// returns NULL).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_summary_json(preview: *const tc_preview) -> *const c_char {
    if preview.is_null() {
        return std::ptr::null();
    }
    let preview = unsafe { &*preview };
    preview.summary_json.as_ptr()
}

/// Search the redacted body for `needle`, a local scan over the in-memory
/// string (no protocol -- see the design's rationale for why preview is
/// in-process). Returns the number of matches, or -1 on error. On success,
/// `*matches_json` is set to an owned JSON array of byte offsets; free with
/// `tc_string_free`. On error, `*matches_json` (if non-null) is set to
/// NULL -- there is nothing to free.
///
/// # Safety
/// `preview` must be a live pointer from `tc_preview_open`. `needle` must be
/// a valid NUL-terminated C string. `matches_json`, if non-null, must point
/// to writable `*mut c_char` storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_search(
    preview: *const tc_preview,
    needle: *const c_char,
    matches_json: *mut *mut c_char,
) -> i32 {
    let outcome = guard(|| {
        if preview.is_null() {
            anyhow::bail!("null-preview");
        }
        let preview = unsafe { &*preview };
        let needle = unsafe { borrow_str(needle) }?;
        if needle.is_empty() {
            return Ok((0usize, "[]".to_string()));
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
        let count = offsets.len();
        let json = serde_json::to_string(&offsets)?;
        Ok((count, json))
    });
    match outcome {
        Ok((count, json)) => {
            if !matches_json.is_null() {
                unsafe { *matches_json = to_owned_cstring(&json) };
            }
            count as i32
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

/// Free a preview handle. Safe to call with NULL (no-op). Invalidates every
/// `const char*` previously returned by `tc_preview_body` /
/// `tc_preview_summary_json` for this handle.
///
/// # Safety
/// `preview`, if non-null, must be a pointer previously returned by
/// `tc_preview_open` and not already freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_preview_free(preview: *mut tc_preview) {
    if preview.is_null() {
        return;
    }
    let _ = guard(|| {
        drop(unsafe { Box::from_raw(preview) });
        Ok(())
    });
}

/// Free a string returned by this library. Safe to call with NULL (no-op).
/// This is the only valid way to free any `char*` this crate returns; do
/// not free it with the caller's own allocator.
///
/// # Safety
/// `s`, if non-null, must be a pointer previously returned by a function in
/// this crate as an owned `char*`, and must not already have been freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_string_free(s: *mut c_char) {
    if s.is_null() {
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
    LAST_ERROR.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|c| c.as_ptr())
            .unwrap_or(std::ptr::null())
    })
}
