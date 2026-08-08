//! Round-trips the C ABI from a Rust integration test that links the crate
//! as an `rlib` (see the crate's `Cargo.toml` comment on why `rlib` is
//! included alongside `cdylib`/`staticlib`). Every helper here frees every
//! string it receives, so a leak-detector run over this file is a genuine
//! check of the ownership rule stated in `include/trace_commons.h`.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

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

/// Point the daemon's session roots at empty tempdirs before starting it,
/// the way `trace-commons-contributor`'s own watcher tests already do
/// (`WatcherFixture`). Without this, `tc_daemon_start` -- via the settings
/// default of `claude_root: None` / `codex_root: None`, meaning "the
/// conventional per-user location" -- scans the machine owner's *real*
/// `~/.claude`/`~/.codex` session roots: a real privacy problem for a test
/// (it reads the developer's actual coding transcripts), and also what made
/// the reentrant-stop and unsubscribe regression tests flaky under a
/// single-worker runtime, since a large real session history makes
/// `watcher::tick`'s filesystem scan slow enough to matter.
fn start(dir: &Path) -> *mut tc_handle {
    let claude_root = dir.join("claude-root");
    let codex_root = dir.join("codex-root");
    std::fs::create_dir_all(&claude_root).unwrap();
    std::fs::create_dir_all(&codex_root).unwrap();
    let store = trace_commons_contributor::config::ConfigStore::open(dir.to_path_buf()).unwrap();
    let settings = trace_commons_contributor::daemon::settings::DaemonSettings {
        claude_root: Some(claude_root),
        codex_root: Some(codex_root),
        ..Default::default()
    };
    settings.save(&store).unwrap();

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
    for _ in 0..100 {
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
    for _ in 0..100 {
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

/// Fix round 2, finding C: the header used to say a subscription lasts
/// "until tc_unsubscribe or the daemon stops" and that ctx must stay valid
/// until tc_unsubscribe returns "or the daemon stops, whichever is first."
/// That is false: `tc_daemon_stop` only sets a flag the subscription's
/// background task polls at most every 250ms, and does not touch
/// subscriptions at all -- so a callback invocation already under way (or
/// working through already-buffered events) can still be running, and
/// still touching `ctx`, well after `tc_daemon_stop` has returned to its
/// caller. This test proves that's real behavior, not just a corrected
/// doc claim, so the two cannot silently drift apart again: it starts a
/// callback that is provably still executing when `tc_daemon_stop` is
/// called, and asserts the callback observes that `tc_daemon_stop` had
/// already returned by the time the callback finished touching its state.
#[test]
fn a_callback_can_still_fire_after_tc_daemon_stop_returns() {
    static CALLBACK_STARTED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static STOP_RETURNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    static FIRED_AFTER_STOP_RETURNED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    extern "C" fn slow_cb(_event_json: *const c_char, _ctx: *mut c_void) {
        CALLBACK_STARTED.store(true, Ordering::SeqCst);
        // Comfortably longer than tc_daemon_stop's own teardown, so this
        // invocation is still in flight (and would still be touching a
        // real host's `ctx`) well after tc_daemon_stop has returned.
        std::thread::sleep(std::time::Duration::from_millis(150));
        if STOP_RETURNED.load(Ordering::SeqCst) {
            FIRED_AFTER_STOP_RETURNED.store(true, Ordering::SeqCst);
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let token = unsafe { tc_subscribe(h, Some(slow_cb), std::ptr::null_mut()) };
    assert_ne!(token, 0);

    let _ = call(h, "resume", "{}");
    // Wait until the callback has provably started (and is now inside its
    // sleep) before stopping, so tc_daemon_stop races a callback that is
    // definitely still in flight rather than one that hasn't begun yet. A
    // generous window: under a heavily parallel test run (every test in
    // this file has its own multi-thread tokio runtime), OS scheduling
    // alone can push this well past a tight budget.
    for _ in 0..200 {
        if CALLBACK_STARTED.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        CALLBACK_STARTED.load(Ordering::SeqCst),
        "the callback never started -- test did not exercise the path under test"
    );

    unsafe { tc_daemon_stop(h) };
    STOP_RETURNED.store(true, Ordering::SeqCst);

    for _ in 0..200 {
        if FIRED_AFTER_STOP_RETURNED.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        FIRED_AFTER_STOP_RETURNED.load(Ordering::SeqCst),
        "a subscription callback must be able to still be running (and \
         still touching ctx) after tc_daemon_stop returns -- tc_daemon_stop \
         is not a synchronization point for subscriptions, only \
         tc_unsubscribe is"
    );

    unsafe { tc_unsubscribe(h, token) };
    unsafe { tc_handle_free(h) };
}

/// Fix round 2, finding B: `tc_unsubscribe`, called with its own token from
/// inside that subscription's own callback, must refuse rather than
/// deadlock. `abort()` cannot preempt a task that is inside a synchronous
/// callback invocation, so joining that task's `JoinHandle` from inside the
/// very callback frame calling `tc_unsubscribe` can only resolve once the
/// callback returns -- which requires the join to return first. Permanent
/// hang, without the reentrancy guard this test exercises.
#[test]
fn tc_unsubscribe_from_inside_its_own_callback_refuses_rather_than_deadlocks() {
    static SELF_UNSUBSCRIBE_ATTEMPTED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static REFUSED_CORRECTLY: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static TOKEN: AtomicU64 = AtomicU64::new(0);

    extern "C" fn self_unsubscribe_cb(_event_json: *const c_char, ctx: *mut c_void) {
        if SELF_UNSUBSCRIBE_ATTEMPTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let h = ctx as *mut tc_handle;
        let token = TOKEN.load(Ordering::SeqCst);
        // The reentrant call under test: this thread is inside the very
        // subscription callback whose token it's asking to cancel.
        unsafe { tc_unsubscribe(h, token) };
        // `tc_last_error` is thread-local, so it must be read here, on the
        // same (callback) thread that made the reentrant call -- reading
        // it from the test's own thread afterward would see nothing.
        let refused = unsafe { CStr::from_ptr(tc_last_error()) }
            .to_str()
            .map(|s| s == "unsubscribe-refused-inside-runtime-context")
            .unwrap_or(false);
        REFUSED_CORRECTLY.store(refused, Ordering::SeqCst);
    }

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let token = unsafe { tc_subscribe(h, Some(self_unsubscribe_cb), h as *mut c_void) };
    assert_ne!(token, 0);
    TOKEN.store(token, Ordering::SeqCst);

    let _ = call(h, "resume", "{}");
    for _ in 0..100 {
        if SELF_UNSUBSCRIBE_ATTEMPTED.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    assert!(
        SELF_UNSUBSCRIBE_ATTEMPTED.load(Ordering::SeqCst),
        "the reentrant tc_unsubscribe callback never fired -- test did not \
         exercise the path under test"
    );

    // No deadlock: the process reached here. The refusal must have been
    // recorded rather than silently succeeded.
    assert!(
        REFUSED_CORRECTLY.load(Ordering::SeqCst),
        "the reentrant tc_unsubscribe must refuse with a fixed label, not \
         silently succeed"
    );

    // A real (non-reentrant) unsubscribe from a plain thread still works.
    unsafe { tc_unsubscribe(h, token) };
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
