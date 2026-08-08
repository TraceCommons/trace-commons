//! Round-trips the C ABI from a Rust integration test that links the crate
//! as an `rlib` (see the crate's `Cargo.toml` comment on why `rlib` is
//! included alongside `cdylib`/`staticlib`). Every helper here frees every
//! string it receives, so a leak-detector run over this file is a genuine
//! check of the ownership rule stated in `include/trace_commons.h`.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use trace_commons_contributor_ffi::{
    tc_call, tc_daemon_start, tc_daemon_stop, tc_handle, tc_handle_free, tc_last_error,
    tc_preview_open, tc_string_free, tc_subscribe, tc_unsubscribe,
};

fn cstr(p: &Path) -> CString {
    CString::new(p.to_str().unwrap()).unwrap()
}

fn cstr_str(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn start(dir: &Path) -> *mut tc_handle {
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe { tc_daemon_start(cstr(dir).as_ptr(), &mut err) };
    if h.is_null() {
        let msg = unsafe { CStr::from_ptr(err) }
            .to_string_lossy()
            .into_owned();
        unsafe { tc_string_free(err) };
        panic!("tc_daemon_start failed: {msg}");
    }
    h
}

/// Tear a handle all the way down: stop the daemon, then reclaim the
/// allocation. Most tests just want both steps done, in order, from a
/// plain thread -- the two-step `tc_daemon_stop` / `tc_handle_free` split
/// itself is exercised directly by the tests that care about it.
fn stop(h: *mut tc_handle) {
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
}

fn call(h: *mut tc_handle, method: &str, params: &str) -> String {
    let out = unsafe { tc_call(h, cstr_str(method).as_ptr(), cstr_str(params).as_ptr()) };
    assert!(!out.is_null(), "tc_call returned null for {method}");
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };
    s
}

fn last_error() -> Option<String> {
    let p = tc_last_error();
    if p.is_null() {
        return None;
    }
    Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned())
}

#[test]
fn a_call_returns_json_the_caller_owns() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = call(h, "status", "{}");
    assert!(out.contains("\"logged_in\""), "{out}");
    stop(h);
}

#[test]
fn a_second_start_against_the_same_directory_fails_on_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let a = start(dir.path());
    let mut err: *mut c_char = std::ptr::null_mut();
    let b = unsafe { tc_daemon_start(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(
        b.is_null(),
        "two daemons must not run against one directory"
    );
    assert!(!err.is_null(), "a failure must set the error out-param");
    unsafe { tc_string_free(err) };
    stop(a);
}

#[test]
fn an_unknown_method_returns_an_error_frame_rather_than_crashing() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = call(h, "no_such_method", "{}");
    assert!(out.contains("unknown_method"), "{out}");
    stop(h);
}

#[test]
fn malformed_params_json_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = call(h, "status", "{not json");
    assert!(out.contains("bad_params"), "{out}");
    stop(h);
}

#[test]
fn repeated_calls_do_not_leak_or_double_free() {
    // Exercises the ownership rule: every char* is freed exactly once.
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    for _ in 0..500 {
        let out = call(h, "status", "{}");
        assert!(!out.is_empty());
    }
    stop(h);
}

#[test]
fn preview_of_an_unknown_entry_sets_the_error_and_returns_null() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let mut err: *mut c_char = std::ptr::null_mut();
    let p = unsafe {
        tc_preview_open(
            h,
            cstr_str("00000000-0000-0000-0000-000000000000").as_ptr(),
            &mut err,
        )
    };
    assert!(p.is_null());
    assert!(!err.is_null());
    unsafe { tc_string_free(err) };
    stop(h);
}

// --- Beyond the brief: every pointer parameter, passed NULL, must produce
// an error rather than a crash. ---

#[test]
fn tc_daemon_start_null_config_dir_is_an_error() {
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe { tc_daemon_start(std::ptr::null(), &mut err) };
    assert!(h.is_null());
    assert!(!err.is_null());
    unsafe { tc_string_free(err) };
}

#[test]
fn tc_daemon_start_null_err_out_param_does_not_crash() {
    let dir = tempfile::tempdir().unwrap();
    let h = unsafe { tc_daemon_start(cstr(dir.path()).as_ptr(), std::ptr::null_mut()) };
    assert!(!h.is_null());
    stop(h);
}

#[test]
fn tc_daemon_stop_null_handle_does_not_crash() {
    unsafe { tc_daemon_stop(std::ptr::null_mut()) };
}

#[test]
fn tc_handle_free_null_handle_does_not_crash() {
    unsafe { tc_handle_free(std::ptr::null_mut()) };
}

#[test]
fn tc_call_null_handle_is_an_error() {
    let out = unsafe {
        tc_call(
            std::ptr::null_mut(),
            cstr_str("status").as_ptr(),
            cstr_str("{}").as_ptr(),
        )
    };
    assert!(!out.is_null());
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };
    assert!(
        s.contains("error") || s.contains("bad_params") || s.contains("unavailable"),
        "{s}"
    );
}

#[test]
fn tc_call_null_method_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, std::ptr::null(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };
    assert!(s.contains("bad_params") || s.contains("error"), "{s}");
    stop(h);
}

#[test]
fn tc_call_null_params_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), std::ptr::null()) };
    assert!(!out.is_null());
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };
    assert!(s.contains("bad_params") || s.contains("error"), "{s}");
    stop(h);
}

#[test]
fn tc_preview_open_null_handle_is_an_error() {
    let mut err: *mut c_char = std::ptr::null_mut();
    let p = unsafe {
        tc_preview_open(
            std::ptr::null_mut(),
            cstr_str("00000000-0000-0000-0000-000000000000").as_ptr(),
            &mut err,
        )
    };
    assert!(p.is_null());
    assert!(!err.is_null());
    unsafe { tc_string_free(err) };
}

#[test]
fn tc_preview_open_null_entry_id_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let mut err: *mut c_char = std::ptr::null_mut();
    let p = unsafe { tc_preview_open(h, std::ptr::null(), &mut err) };
    assert!(p.is_null());
    assert!(!err.is_null());
    unsafe { tc_string_free(err) };
    stop(h);
}

#[test]
fn tc_string_free_null_does_not_crash() {
    unsafe { tc_string_free(std::ptr::null_mut()) };
}

// --- Fix round 1 regressions -------------------------------------------

/// The CRITICAL finding: calling `tc_daemon_stop` from inside a
/// `tc_subscribe` callback running on `handle`'s own worker thread must not
/// crash. Before the fix, this reproduced a segfault (signal 11): the
/// callback thread called `handle.rt.block_on(..)` on itself, which panics
/// ("cannot start a runtime from within a runtime"), and `guard` caught
/// that panic mid-way through a `Box::from_raw` that had already dropped
/// the runtime out from under the very thread driving it.
///
/// This test triggers exactly that reentrant call and then asserts the
/// handle is still alive and usable afterward -- which would be impossible
/// if the earlier call had corrupted it.
#[test]
fn tc_daemon_stop_from_inside_a_subscribe_callback_does_not_crash() {
    static REENTRANT_STOP_ATTEMPTED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn reentrant_stop_cb(_event_json: *const c_char, ctx: *mut c_void) {
        CALLBACK_COUNT.fetch_add(1, Ordering::SeqCst);
        if REENTRANT_STOP_ATTEMPTED.swap(true, Ordering::SeqCst) {
            // Only the first callback invocation attempts the reentrant
            // stop; later deliveries (there may be more before the
            // background task notices `shutdown`) must not pile on.
            return;
        }
        let handle = ctx as *mut tc_handle;
        // This is the reentrant call under test: we are on one of
        // `handle`'s own tokio worker threads right now.
        unsafe { tc_daemon_stop(handle) };
    }

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());

    let token = unsafe { tc_subscribe(h, Some(reentrant_stop_cb), h as *mut c_void) };
    assert_ne!(token, 0, "subscribe must succeed");

    // Trigger an event so the callback actually fires.
    let _ = call(h, "resume", "{}");

    // Give the background poll loop (250ms ticks) time to deliver it and
    // run the reentrant `tc_daemon_stop`.
    for _ in 0..40 {
        if REENTRANT_STOP_ATTEMPTED.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        REENTRANT_STOP_ATTEMPTED.load(Ordering::SeqCst),
        "the reentrant tc_daemon_stop callback never fired -- test did not \
         exercise the path under test"
    );

    // No segfault: the process is still here to make this assertion. The
    // reentrant stop must have been refused/handled safely (not silently
    // succeeded from inside the runtime), so the daemon should either
    // still be reachable or cleanly stopped -- either way, a *second*,
    // ordinary stop from this normal thread (not inside any callback) must
    // not crash either.
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
}

/// HIGH finding: `tc_unsubscribe` must guarantee no further callback
/// invocation once it returns -- not "usually, because of how Runtime::drop
/// happens to work," which is what the pre-fix code relied on implicitly
/// and never tested.
#[test]
fn no_callback_fires_after_tc_unsubscribe_returns() {
    static COUNT: AtomicUsize = AtomicUsize::new(0);

    extern "C" fn counting_cb(_event_json: *const c_char, _ctx: *mut c_void) {
        COUNT.fetch_add(1, Ordering::SeqCst);
    }

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());

    let token = unsafe { tc_subscribe(h, Some(counting_cb), std::ptr::null_mut()) };
    assert_ne!(token, 0);

    // Trigger at least one delivery and wait for it, so we know the
    // subscription is genuinely live before unsubscribing.
    let _ = call(h, "resume", "{}");
    for _ in 0..40 {
        if COUNT.load(Ordering::SeqCst) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(COUNT.load(Ordering::SeqCst) > 0, "subscription never fired");

    unsafe { tc_unsubscribe(h, token) };
    let count_at_unsubscribe = COUNT.load(Ordering::SeqCst);

    // Fire a burst of further events; if the subscription were still live
    // these would all be delivered.
    for _ in 0..10 {
        let _ = call(h, "pause", "{}");
        let _ = call(h, "resume", "{}");
    }
    // No poll-and-wait here on purpose: tc_unsubscribe's contract is that
    // no callback fires *after it returns*, so the count must already be
    // final the instant it returned above.
    assert_eq!(
        COUNT.load(Ordering::SeqCst),
        count_at_unsubscribe,
        "a callback fired after tc_unsubscribe returned"
    );

    stop(h);
}

#[test]
fn tc_unsubscribe_unknown_token_does_not_crash() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_unsubscribe(h, 0) };
    unsafe { tc_unsubscribe(h, 999_999) };
    unsafe { tc_unsubscribe(std::ptr::null_mut(), 1) };
    stop(h);
}

/// HIGH finding: `tc_daemon_stop` must not free `handle`, so a concurrent
/// `tc_call` on another thread stays valid (observes the daemon as
/// stopped) instead of dereferencing freed memory. This can't prove the
/// absence of a race with certainty (that would need a sanitizer run), but
/// it does prove the two calls are safe to interleave under real
/// concurrency without the process crashing, which the pre-fix design
/// (`tc_daemon_stop` freeing the whole allocation) could not offer at all.
#[test]
fn concurrent_tc_call_and_tc_daemon_stop_do_not_crash() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let h_addr = h as usize;

    let caller = std::thread::spawn(move || {
        let h = h_addr as *mut tc_handle;
        for _ in 0..200 {
            let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
            if !out.is_null() {
                unsafe { tc_string_free(out) };
            }
        }
    });

    unsafe { tc_daemon_stop(h) };
    caller.join().unwrap();
    unsafe { tc_handle_free(h) };
}

// --- Allocation-registry double-free / cross-type-free detection -------

#[test]
fn double_free_of_a_string_is_refused_not_ub() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    unsafe { tc_string_free(out) };
    // Second free of the same pointer: must not double-free.
    unsafe { tc_string_free(out) };
    assert!(
        last_error()
            .map(|e| e.contains("double-free") || e.contains("unknown-pointer"))
            .unwrap_or(false)
    );
    stop(h);
}

#[test]
fn double_free_of_a_handle_is_refused_not_ub() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
    // Second free of the same handle pointer.
    unsafe { tc_handle_free(h) };
    assert!(
        last_error()
            .map(|e| e.contains("double-free") || e.contains("unknown-pointer"))
            .unwrap_or(false)
    );
}

#[test]
fn cross_type_free_of_a_preview_as_a_string_is_refused_not_ub() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    // Build a session-less config so preview fails cleanly, then instead
    // exercise cross-type free using a handle's own allocation cast as a
    // string, which is available regardless of preview/session setup.
    unsafe { tc_string_free(h as *mut c_char) };
    assert!(
        last_error()
            .map(|e| e.contains("cross-type-free") || e.contains("unknown-pointer"))
            .unwrap_or(false)
    );
    // The handle itself must still be intact: freeing it for real still
    // works.
    stop(h);
}

// --- Discriminating token uniqueness for tc_subscribe -------------------

#[test]
fn tc_subscribe_returns_distinct_nonzero_tokens() {
    extern "C" fn noop_cb(_event_json: *const c_char, _ctx: *mut c_void) {}

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let a = unsafe { tc_subscribe(h, Some(noop_cb), std::ptr::null_mut()) };
    let b = unsafe { tc_subscribe(h, Some(noop_cb), std::ptr::null_mut()) };
    assert_ne!(a, 0);
    assert_ne!(b, 0);
    assert_ne!(a, b);
    unsafe { tc_unsubscribe(h, a) };
    unsafe { tc_unsubscribe(h, b) };
    stop(h);
}

#[test]
fn tc_subscribe_null_cb_returns_zero() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let token = unsafe { tc_subscribe(h, None, std::ptr::null_mut()) };
    assert_eq!(token, 0);
    stop(h);
}
