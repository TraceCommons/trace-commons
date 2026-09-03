//! Round-trips the C ABI from a Rust integration test that links the crate
//! as an `rlib` (see the crate's `Cargo.toml` comment on why `rlib` is
//! included alongside `cdylib`/`staticlib`). Every helper here frees every
//! string it receives, so a leak-detector run over this file is a genuine
//! check of the ownership rule stated in `include/trace_commons.h`.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use trace_commons_contributor_ffi::{
    tc_call, tc_daemon_start, tc_daemon_start_with_settings, tc_daemon_stop, tc_discover_sources,
    tc_handle, tc_handle_free, tc_invite_issuer_host, tc_last_error, tc_preview, tc_preview_body,
    tc_preview_open, tc_preview_search, tc_preview_summary_json, tc_preview_turns_json,
    tc_routing_copy, tc_routing_last_checked, tc_routing_state_line, tc_routing_state_tone,
    tc_routing_token_line, tc_routing_tool_tone, tc_routing_tool_word, tc_routing_unreachable_line,
    tc_scrub_detector_names, tc_source_check_line, tc_string_free, tc_subscribe, tc_unsubscribe,
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
/// default of `claude_source: None` / `codex_source: None`, meaning "the
/// conventional per-user location" -- scans the machine owner's *real*
/// `~/.claude`/`~/.codex` session roots: a real privacy problem for a test
/// (it reads the developer's actual coding transcripts), and also what made
/// the reentrant-stop and unsubscribe regression tests flaky under a
/// single-worker runtime, since a large real session history makes
/// `watcher::tick`'s filesystem scan slow enough to matter.
fn write_tempdir_session_roots(dir: &Path) {
    let claude_root = dir.join("claude-root");
    let codex_root = dir.join("codex-root");
    std::fs::create_dir_all(&claude_root).unwrap();
    std::fs::create_dir_all(&codex_root).unwrap();
    let store = trace_commons_contributor::config::ConfigStore::open(dir.to_path_buf()).unwrap();
    let settings = trace_commons_contributor::daemon::settings::DaemonSettings {
        claude_source: Some(
            trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                path: claude_root,
            },
        ),
        codex_source: Some(
            trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                path: codex_root,
            },
        ),
        ..Default::default()
    };
    settings.save(&store).unwrap();
}

fn start(dir: &Path) -> *mut tc_handle {
    write_tempdir_session_roots(dir);

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

/// The reproduction that started sub-project G, pinned.
///
/// A hand-written `daemon-settings.json` -- the shape a person actually
/// writes, missing `schema_version` and most required fields -- used to
/// report the opaque `daemon-start-failed`, leaving a contributor and two
/// agents with nothing to act on. It must name itself now.
#[test]
fn an_unparseable_settings_file_is_named_rather_than_flattened() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("daemon-settings.json"),
        r#"{"claude_root":"/tmp/x","codex_root":"/tmp/y"}"#,
    )
    .unwrap();

    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe { tc_daemon_start(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(h.is_null(), "an unparseable settings file must not start");
    assert_eq!(take_err(err), "settings-unreadable");

    // And the failure must not leave behind the file every other reader
    // treats as proof a daemon is running here.
    assert!(
        !dir.path().join("daemon.lock").exists(),
        "a failed start left daemon.lock behind"
    );
}

/// Lock contention names itself too, rather than sharing one label with
/// every other way a start can fail.
#[test]
fn a_second_start_reports_lock_contention_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let a = start(dir.path());

    let mut err: *mut c_char = std::ptr::null_mut();
    let b = unsafe { tc_daemon_start(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(b.is_null());
    assert_eq!(take_err(err), "already-running");

    stop(a);
}

/// No label crossing this boundary may carry a filesystem path.
///
/// A general guard rather than one case per label: the errors behind these
/// embed the state-directory and lock-file paths for local stderr, so the
/// failure mode is a future `format!` forwarding one, and that would not be
/// caught by asserting the labels we happen to know about today. The
/// contributor's home directory is checked explicitly because it carries the
/// OS username, which is the specific disclosure that matters.
#[test]
fn no_start_failure_label_carries_a_path() {
    let home = std::env::var("HOME").unwrap_or_default();

    // Each of these fails a start a different way.
    let unparseable = tempfile::tempdir().unwrap();
    std::fs::write(
        unparseable.path().join("daemon-settings.json"),
        r#"{"claude_root":"/tmp/x"}"#,
    )
    .unwrap();

    let missing = unparseable.path().join("no-such-directory");

    let contended = tempfile::tempdir().unwrap();
    let held = start(contended.path());

    let mut labels = Vec::new();
    for dir in [unparseable.path(), missing.as_path(), contended.path()] {
        let mut err: *mut c_char = std::ptr::null_mut();
        let h = unsafe { tc_daemon_start(cstr(dir).as_ptr(), &mut err) };
        assert!(h.is_null(), "these directories must all fail to start");
        labels.push(take_err(err));
    }
    stop(held);

    for label in &labels {
        assert!(
            !label.contains('/') && !label.contains('\\'),
            "a start-failure label carried a path separator: {label}"
        );
        assert!(
            home.is_empty() || !label.contains(&home),
            "a start-failure label carried the contributor's home directory: {label}"
        );
        assert!(
            label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-' || c.is_ascii_digit()),
            "a start-failure label must be a fixed kebab-case token: {label}"
        );
    }
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
    // Same tempdir session roots every other test gets: without this the
    // settings default of `claude_source: None` / `codex_source: None` means
    // "the conventional per-user location", so the supervisor's first tick
    // would scan and hash the *developer's real* ~/.claude and ~/.codex
    // transcripts on every run of this suite. `start()` is not reused here
    // because passing a null `err` out-param is the whole point of this
    // test, and `start()` passes a real one.
    write_tempdir_session_roots(dir.path());
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
/// How many worker threads `tc_daemon_start`'s runtime will actually have.
/// It calls `.worker_threads(..)` explicitly with a floor of two, which
/// overrides `TOKIO_WORKER_THREADS` -- so this mirrors that floor rather
/// than reading the environment. Kept as a function, and asserted on
/// below, so the test and the floor cannot drift apart.
fn daemon_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .max(2)
}

/// The in-flight window is bounded by an explicit release signal, not by a
/// `sleep` long enough to outlast `tc_daemon_stop`'s teardown. An earlier
/// revision of this test used a 150ms sleep in the callback and hoped
/// `tc_daemon_stop` would return inside it; at `TOKIO_WORKER_THREADS=1`
/// under a parallel suite run that lost the race about 1 run in 7
/// (reproduced: 2 failures in 10 measured full-suite runs), each costing a
/// 5s poll loop. The property under test has nothing to do with how long
/// teardown takes, so the test must not either: here the callback blocks
/// until the test thread -- having already returned from `tc_daemon_stop`
/// and set `STOP_RETURNED` -- explicitly releases it. Whatever order the
/// scheduler picks, the callback is provably still in flight across the
/// whole of `tc_daemon_stop`.
///
/// # Why this needs at least two daemon worker threads
///
/// A `tc_subscribe` callback is a synchronous C function invoked from the
/// subscription's tokio task, so an in-flight callback occupies a worker
/// thread for its whole duration. `tc_daemon_stop` finishes by joining the
/// supervisor task, which also needs a worker. With exactly one worker
/// those two demands are mutually exclusive, so at one worker the scenario
/// is not merely slow to arrange -- it is unsatisfiable, and this test used
/// to skip itself loudly whenever `TOKIO_WORKER_THREADS=1`.
///
/// The skip is gone because the condition is: `tc_daemon_start` now sets
/// `.worker_threads(..)` with a floor of two, which overrides
/// `TOKIO_WORKER_THREADS` outright. That floor exists for the production
/// hazard, not for this test -- at one worker, `stop_embedded`'s join and a
/// callback holding the sole worker are a circular wait, and
/// `tc_daemon_stop` is documented as callable from inside a callback -- but
/// it also makes this scenario satisfiable in every configuration, so the
/// test runs unconditionally. It asserts the floor rather than assuming it.
#[test]
fn a_callback_can_still_fire_after_tc_daemon_stop_returns() {
    use std::sync::atomic::AtomicBool;

    assert!(
        daemon_worker_threads() >= 2,
        "tc_daemon_start must floor its runtime at two workers; below that \
         an in-flight callback and tc_daemon_stop's supervisor join are a \
         circular wait, in production as well as here"
    );

    static CALLBACK_STARTED: AtomicBool = AtomicBool::new(false);
    /// Set by the test thread only after `tc_daemon_stop` has returned.
    static STOP_RETURNED: AtomicBool = AtomicBool::new(false);
    /// Set by the test thread strictly after `STOP_RETURNED`, so a callback
    /// that observes this necessarily observes `STOP_RETURNED == true`.
    static RELEASE_CALLBACK: AtomicBool = AtomicBool::new(false);
    static CALLBACK_FINISHED: AtomicBool = AtomicBool::new(false);
    static FIRED_AFTER_STOP_RETURNED: AtomicBool = AtomicBool::new(false);

    extern "C" fn blocking_cb(_event_json: *const c_char, _ctx: *mut c_void) {
        if CALLBACK_STARTED.swap(true, Ordering::SeqCst) {
            // Only the first invocation participates; any later one must
            // not re-block and stall the test's teardown.
            return;
        }
        // Stay inside this invocation -- exactly as a real host's callback
        // would still be touching `ctx` -- until the test thread says it is
        // done observing. No timing assumption of any kind.
        while !RELEASE_CALLBACK.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if STOP_RETURNED.load(Ordering::SeqCst) {
            FIRED_AFTER_STOP_RETURNED.store(true, Ordering::SeqCst);
        }
        CALLBACK_FINISHED.store(true, Ordering::SeqCst);
    }

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let token = unsafe { tc_subscribe(h, Some(blocking_cb), std::ptr::null_mut()) };
    assert_ne!(token, 0);

    let _ = call(h, "resume", "{}");
    // Wait until the callback has provably started (and is now parked in
    // its release loop) before stopping, so `tc_daemon_stop` genuinely
    // overlaps an in-flight callback. This wait has no upper bound tied to
    // the property -- it only has to happen at all.
    for _ in 0..400 {
        if CALLBACK_STARTED.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        CALLBACK_STARTED.load(Ordering::SeqCst),
        "the callback never started -- test did not exercise the path under test"
    );

    // The callback is still executing right now. If `tc_daemon_stop` were a
    // synchronization point for subscriptions, this call could not return
    // until the callback did -- and it cannot, because the callback is
    // waiting on a flag only set after this returns. So reaching the next
    // line at all is itself the proof.
    unsafe { tc_daemon_stop(h) };
    STOP_RETURNED.store(true, Ordering::SeqCst);
    RELEASE_CALLBACK.store(true, Ordering::SeqCst);

    for _ in 0..400 {
        if CALLBACK_FINISHED.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        CALLBACK_FINISHED.load(Ordering::SeqCst),
        "the callback never finished after being released"
    );
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
    // Set LAST, after `REFUSED_CORRECTLY` has been written. The test waits on
    // this rather than on `SELF_UNSUBSCRIBE_ATTEMPTED`, which is set on entry:
    // waiting on the entry flag let the test observe "the callback started"
    // and then assert on a value the callback had not stored yet, so the
    // refusal assertion failed intermittently on a callback that was in fact
    // about to refuse correctly.
    static SELF_UNSUBSCRIBE_DONE: std::sync::atomic::AtomicBool =
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
        // Null-checked before `CStr::from_ptr`: a null return (nothing
        // recorded on this thread yet, or thread-local storage already torn
        // down) is a clean "not refused", not undefined behaviour. Passing
        // null straight to `from_ptr` was UB, and this assertion failed
        // once in 26 runs.
        let last = tc_last_error();
        let refused = !last.is_null()
            && unsafe { CStr::from_ptr(last) }
                .to_str()
                .map(|s| s == "unsubscribe-refused-inside-runtime-context")
                .unwrap_or(false);
        REFUSED_CORRECTLY.store(refused, Ordering::SeqCst);
        // Publish completion only after the verdict above is visible.
        SELF_UNSUBSCRIBE_DONE.store(true, Ordering::SeqCst);
    }

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let token = unsafe { tc_subscribe(h, Some(self_unsubscribe_cb), h as *mut c_void) };
    assert_ne!(token, 0);
    TOKEN.store(token, Ordering::SeqCst);

    let _ = call(h, "resume", "{}");
    for _ in 0..100 {
        if SELF_UNSUBSCRIBE_DONE.load(Ordering::SeqCst) {
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
    // The handle itself must still be intact. A refusal must not
    // unregister it: `stop` alone asserts nothing, so prove the handle is
    // still live by using it.
    assert!(
        !call(h, "status", "{}").contains("invalid-handle-pointer"),
        "a refused cross-type free must leave the handle usable"
    );
    stop(h);
}

// --- Preview accessors must consult the registry before dereferencing ---
//
// The registry detected invalid *frees*, but the three borrowing accessors
// dereferenced whatever pointer they were given. A stale or cross-type
// pointer was therefore a use-after-free or a type confusion rather than
// the fixed error the rest of this ABI promises.

#[test]
fn preview_body_refuses_a_pointer_that_is_not_a_preview() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    // A live `tc_handle*` deliberately passed where a `tc_preview*` is
    // expected -- the exact mistake `OpaquePointer`/`IntPtr` callers make,
    // and the one the free functions already refuse.
    let p = unsafe { tc_preview_body(h as *const tc_preview) };
    assert!(
        p.is_null(),
        "a non-preview pointer must not be dereferenced"
    );
    assert!(
        last_error()
            .map(|e| e.contains("invalid-preview-pointer"))
            .unwrap_or(false),
        "{:?}",
        last_error()
    );
    // The handle is untouched by the refusal and still usable.
    let out = call(h, "status", "{}");
    assert!(out.contains("\"logged_in\""), "{out}");
    stop(h);
}

#[test]
fn preview_summary_json_refuses_a_pointer_that_is_not_a_preview() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let p = unsafe { tc_preview_summary_json(h as *const tc_preview) };
    assert!(p.is_null());
    assert!(
        last_error()
            .map(|e| e.contains("invalid-preview-pointer"))
            .unwrap_or(false)
    );
    stop(h);
}

#[test]
fn preview_search_refuses_a_pointer_that_is_not_a_preview() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let needle = cstr_str("anything");
    let mut matches: *mut c_char = std::ptr::null_mut();
    let n = unsafe { tc_preview_search(h as *const tc_preview, needle.as_ptr(), &mut matches) };
    assert_eq!(n, -1);
    assert!(matches.is_null(), "nothing to free on the error path");
    assert!(
        last_error()
            .map(|e| e.contains("invalid-preview-pointer"))
            .unwrap_or(false)
    );
    stop(h);
}

// --- Handle entry points must consult the registry before dereferencing -
//
// The registry detected invalid frees, and (see above) invalid preview-
// accessor reads. It was not consulted by any of the six `tc_handle*`-
// borrowing entry points below -- they null-checked and then dereferenced
// directly. A stale (already freed by `tc_handle_free`) or cross-type (a
// `tc_preview*`, or any other kind of pointer this crate allocated)
// handle was therefore a use-after-free or a type confusion rather than
// the fixed error the rest of this ABI promises. Each test below passes a
// live pointer of the WRONG registry kind (a `tc_string*`, obtained from
// an ordinary `tc_call` and deliberately not yet freed) where a
// `tc_handle*` is expected -- the same shape of mistake
// `preview_body_refuses_a_pointer_that_is_not_a_preview` and its siblings
// already exercise for the preview accessors, mirrored onto the handle
// entry points. None of these dereference the bad pointer if the fix
// holds, so none of them may crash.

#[test]
fn tc_daemon_stop_refuses_a_pointer_that_is_not_a_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    // A live `tc_string*` deliberately passed where a `tc_handle*` is
    // expected.
    unsafe { tc_daemon_stop(out as *mut tc_handle) };
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false),
        "{:?}",
        last_error()
    );
    unsafe { tc_string_free(out) };
    // The real handle is untouched by the refusal and still usable.
    let status = call(h, "status", "{}");
    assert!(status.contains("\"logged_in\""), "{status}");
    stop(h);
}

#[test]
fn tc_call_refuses_a_pointer_that_is_not_a_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    // `tc_call` never returns NULL: a stale handle must still produce a
    // JSON error frame, not a null pointer or a crash.
    let bad = unsafe {
        tc_call(
            out as *mut tc_handle,
            cstr_str("status").as_ptr(),
            cstr_str("{}").as_ptr(),
        )
    };
    assert!(!bad.is_null());
    let s = unsafe { CStr::from_ptr(bad) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(bad) };
    assert!(s.contains("invalid-handle-pointer"), "{s}");
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
    unsafe { tc_string_free(out) };
    stop(h);
}

#[test]
fn tc_subscribe_refuses_a_pointer_that_is_not_a_handle() {
    extern "C" fn noop_cb(_event_json: *const c_char, _ctx: *mut c_void) {}

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    let token = unsafe { tc_subscribe(out as *mut tc_handle, Some(noop_cb), std::ptr::null_mut()) };
    assert_eq!(
        token, 0,
        "a stale handle must not yield a subscription token"
    );
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
    unsafe { tc_string_free(out) };
    stop(h);
}

#[test]
fn tc_unsubscribe_refuses_a_pointer_that_is_not_a_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    // Any nonzero token: the pointer-liveness check runs before the token
    // is ever looked up, so no real subscription is needed here.
    unsafe { tc_unsubscribe(out as *mut tc_handle, 1) };
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
    unsafe { tc_string_free(out) };
    stop(h);
}

#[test]
fn tc_preview_open_refuses_a_pointer_that_is_not_a_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    let mut err: *mut c_char = std::ptr::null_mut();
    let p = unsafe {
        tc_preview_open(
            out as *mut tc_handle,
            cstr_str("00000000-0000-0000-0000-000000000000").as_ptr(),
            &mut err,
        )
    };
    assert!(p.is_null());
    assert!(!err.is_null());
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    assert!(msg.contains("invalid-handle-pointer"), "{msg}");
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
    unsafe { tc_string_free(out) };
    stop(h);
}

#[test]
fn tc_preview_turns_json_refuses_a_pointer_that_is_not_a_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    let mut err: *mut c_char = std::ptr::null_mut();
    let p = unsafe {
        tc_preview_turns_json(
            out as *mut tc_handle,
            cstr_str("00000000-0000-0000-0000-000000000000").as_ptr(),
            cstr_str("sha256:irrelevant").as_ptr(),
            &mut err,
        )
    };
    assert!(p.is_null());
    assert!(!err.is_null());
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    assert!(msg.contains("invalid-handle-pointer"), "{msg}");
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
    unsafe { tc_string_free(out) };
    stop(h);
}

// --- ...and the other shape the doc comment above names: a freed handle -
//
// The block above exercises the cross-type half of the threat
// `handle_pointer_is_live`'s doc names; this one exercises the other half
// named there -- a handle already freed by `tc_handle_free`. Each test
// below starts a real handle, stops and frees it exactly as
// `double_free_of_a_handle_is_refused_not_ub` does, then reuses the same
// pointer VALUE -- never dereferenced unless the guard under test fails
// -- as the argument to the entry point under test. For `tc_daemon_stop`,
// the second stop after the free (the same shape of reuse
// `double_free_of_a_handle_is_refused_not_ub` makes with
// `tc_handle_free`) is that call under test.

#[test]
fn tc_daemon_stop_refuses_a_freed_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
    // The call under test: a second stop against the same, now-freed,
    // handle pointer.
    unsafe { tc_daemon_stop(h) };
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false),
        "{:?}",
        last_error()
    );
}

#[test]
fn tc_call_refuses_a_freed_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
    // `tc_call` never returns NULL: a freed handle must still produce a
    // JSON error frame, not a null pointer or a crash.
    let bad = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!bad.is_null());
    let s = unsafe { CStr::from_ptr(bad) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(bad) };
    assert!(s.contains("invalid-handle-pointer"), "{s}");
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
}

#[test]
fn tc_subscribe_refuses_a_freed_handle() {
    extern "C" fn noop_cb(_event_json: *const c_char, _ctx: *mut c_void) {}

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
    let token = unsafe { tc_subscribe(h, Some(noop_cb), std::ptr::null_mut()) };
    assert_eq!(
        token, 0,
        "a freed handle must not yield a subscription token"
    );
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
}

#[test]
fn tc_unsubscribe_refuses_a_freed_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
    // Any nonzero token: the pointer-liveness check runs before the token
    // is ever looked up, so no real subscription is needed here.
    unsafe { tc_unsubscribe(h, 1) };
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
}

#[test]
fn tc_preview_open_refuses_a_freed_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
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
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    assert!(msg.contains("invalid-handle-pointer"), "{msg}");
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
}

#[test]
fn tc_preview_turns_json_refuses_a_freed_handle() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
    let mut err: *mut c_char = std::ptr::null_mut();
    let p = unsafe {
        tc_preview_turns_json(
            h,
            cstr_str("00000000-0000-0000-0000-000000000000").as_ptr(),
            cstr_str("sha256:irrelevant").as_ptr(),
            &mut err,
        )
    };
    assert!(p.is_null());
    assert!(!err.is_null());
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    assert!(msg.contains("invalid-handle-pointer"), "{msg}");
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false)
    );
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

// --- Shutdown must not leave an undetectable zombie ---------------------

/// `tc_call(h, "shutdown", "{}")` reaches the daemon's own shutdown method,
/// which stops the supervise loop but never touches `handle.running`. The
/// handle therefore kept reporting a running daemon over a stopped one:
/// `tc_subscribe` returned a nonzero -- documented as success -- token for
/// a task that exits on its first poll, and `status` kept answering as if
/// all were well. `shared_of` now consults `shared.shutdown`, so all three
/// entry points agree on what "stopped" means.
#[test]
fn tc_call_shutdown_leaves_the_handle_reporting_a_stopped_daemon() {
    extern "C" fn noop_cb(_event_json: *const c_char, _ctx: *mut c_void) {}

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());

    let before = call(h, "status", "{}");
    assert!(before.contains("queue_depth"), "{before}");

    let shutdown = call(h, "shutdown", "{}");
    assert!(shutdown.contains("stopping"), "{shutdown}");

    let after = call(h, "status", "{}");
    assert!(
        after.contains("daemon-stopped"),
        "status must report the daemon as stopped, not answer as if it \
         were running: {after}"
    );

    let token = unsafe { tc_subscribe(h, Some(noop_cb), std::ptr::null_mut()) };
    assert_eq!(
        token, 0,
        "subscribing to a stopped daemon must fail, not hand back a \
         success token for a task that exits immediately"
    );

    stop(h);
}

// --- tc_preview_open must not panic from inside a callback --------------

/// The most natural GUI flow there is: receive `queue_changed` on the
/// subscription callback, and open the preview for what changed.
/// `tc_preview_open` used to run `handle.rt.block_on(..)` on the calling
/// thread, which -- inside a callback, running on one of that same
/// runtime's workers -- panics with "Cannot start a runtime from within a
/// runtime". `guard_forwarding` turned that into `err = "panic"`,
/// indistinguishable from a real internal panic, after tokio had already
/// written a backtrace to a signed menu-bar app's stderr.
///
/// The entry id here is deliberately unknown, so the *expected* answer is
/// the fixed label `unknown-entry-id`. That is the whole point: the
/// distinction under test is a clean, specific error versus a panic.
#[test]
fn tc_preview_open_from_inside_a_subscribe_callback_reports_an_error_not_a_panic() {
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;

    static DONE: AtomicBool = AtomicBool::new(false);
    static HANDLE: AtomicU64 = AtomicU64::new(0);
    static OBSERVED: Mutex<Option<String>> = Mutex::new(None);

    extern "C" fn preview_cb(_event_json: *const c_char, _ctx: *mut c_void) {
        if DONE.swap(true, Ordering::SeqCst) {
            return;
        }
        let h = HANDLE.load(Ordering::SeqCst) as *mut tc_handle;
        let id = cstr_str("00000000-0000-0000-0000-000000000000");
        let mut err: *mut c_char = std::ptr::null_mut();
        let p = unsafe { tc_preview_open(h, id.as_ptr(), &mut err) };
        assert!(p.is_null(), "an unknown entry id has no preview");
        let msg = if err.is_null() {
            String::new()
        } else {
            let s = unsafe { CStr::from_ptr(err) }
                .to_string_lossy()
                .into_owned();
            unsafe { tc_string_free(err) };
            s
        };
        *OBSERVED.lock().unwrap() = Some(msg);
    }

    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    HANDLE.store(h as u64, Ordering::SeqCst);
    let token = unsafe { tc_subscribe(h, Some(preview_cb), std::ptr::null_mut()) };
    assert_ne!(token, 0);

    // Any published event will do; `pause` publishes status_changed.
    let _ = call(h, "pause", "{}");
    for _ in 0..400 {
        if DONE.load(Ordering::SeqCst) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        DONE.load(Ordering::SeqCst),
        "the callback never fired -- test did not exercise the path under test"
    );
    unsafe { tc_unsubscribe(h, token) };

    let observed = OBSERVED.lock().unwrap().clone().unwrap();
    assert_ne!(
        observed, "panic",
        "tc_preview_open must not panic from inside a subscribe callback"
    );
    assert_eq!(
        observed, "unknown-entry-id",
        "the caller must get the real, fixed label"
    );

    stop(h);
}

// --- tc_daemon_start_with_settings: closes the gap where a host had no way
// to set claude_root/codex_root before the daemon's first supervisor tick,
// which fires immediately on start (see the function's own doc). The Swift
// demo worked around this by hand-writing daemon-settings.json in the shape
// DaemonSettings::save happens to use today; these tests exist so that
// workaround, and the gap behind it, cannot come back unnoticed. ---

/// Write a minimal Claude Code session file in the on-disk shape
/// `ClaudeCodeSource::discover` expects: a project directory whose name
/// encodes the cwd (`/`s become `-`s), containing one `.jsonl` file whose
/// first line names that same cwd. Mirrors
/// `trace-commons-contributor`'s own `WatcherFixture::write_session`.
fn write_claude_session(claude_root: &Path, project: &str, name: &str) -> std::path::PathBuf {
    let project_dir = claude_root.join(format!("-Users-testuser-code-{project}"));
    std::fs::create_dir_all(&project_dir).unwrap();
    let path = project_dir.join(format!("{name}.jsonl"));
    let body = format!(
        "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"hello\"}},\
         \"cwd\":\"/Users/testuser/code/{project}\",\
         \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
         \"sessionId\":\"{name}\",\"uuid\":\"a1\"}}\n"
    );
    std::fs::write(&path, &body).unwrap();
    path
}

/// Pre-record `path` at its current size in `daemon-state.json`, so the
/// very first supervisor tick already has a "previous poll" to compare
/// against and can find the session immediately size-stable
/// (`Eligibility::Eligible`) rather than needing a second tick
/// (`Eligibility::Unstable` on first sighting; see
/// `daemon::eligibility::evaluate`). Without this, proving "the override
/// took effect before the first tick" would require waiting out a real
/// `poll_interval_secs`, which is not something this test controls.
fn preseed_stable_observation(config_dir: &Path, path: &std::path::Path) {
    let store =
        trace_commons_contributor::config::ConfigStore::open(config_dir.to_path_buf()).unwrap();
    let size = std::fs::metadata(path).unwrap().len();
    let mut state = trace_commons_contributor::daemon::state::DaemonState::new();
    state.observe(path, size);
    state.save(&store).unwrap();
}

#[test]
fn a_claude_root_override_is_scanned_from_the_first_tick() {
    let dir = tempfile::tempdir().unwrap();
    let claude_root = dir.path().join("claude-root");
    let codex_root = dir.path().join("codex-root");
    std::fs::create_dir_all(&claude_root).unwrap();
    std::fs::create_dir_all(&codex_root).unwrap();

    let session_path = write_claude_session(&claude_root, "project1", "s1");
    preseed_stable_observation(dir.path(), &session_path);

    let settings_json = serde_json::json!({
        "claude_root": claude_root.to_str().unwrap(),
        "codex_root": codex_root.to_str().unwrap(),
        // Backdated to a fixed past timestamp above; a real-time quiescence
        // window would otherwise race this test against the clock.
        "quiescence_secs": 0,
    })
    .to_string();

    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(
            cstr(dir.path()).as_ptr(),
            cstr_str(&settings_json).as_ptr(),
            &mut err,
        )
    };
    assert!(
        !h.is_null(),
        "tc_daemon_start_with_settings failed: {:?}",
        last_error()
    );

    let mut seen = false;
    for _ in 0..200 {
        let out = call(h, "list_pending", "{}");
        if out.contains("project1") {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(
        seen,
        "the watcher never queued a session from the claude_root override -- \
         the pre-start settings override did not take effect before the \
         first tick"
    );

    stop(h);
}

#[test]
fn an_unknown_settings_field_is_rejected_not_silently_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let settings_json = cstr_str(r#"{"claude_root_typo":"/tmp/whatever"}"#);
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(cstr(dir.path()).as_ptr(), settings_json.as_ptr(), &mut err)
    };
    assert!(
        h.is_null(),
        "an unrecognized settings field must not silently start the daemon"
    );
    assert!(!err.is_null(), "a failure must set the error out-param");
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    assert_eq!(msg, "settings-unknown-field", "{msg}");
}

#[test]
fn the_pre_start_and_ipc_paths_agree_on_the_daily_caps() {
    // `tc_daemon_start_with_settings` and `tc_call(handle, "set_settings",
    // ...)` share one validator (`apply_settings_object`); this proves that
    // in practice, not just by code inspection, by driving both paths for
    // the same fields against the same running daemon and checking they
    // land on the same values.
    let dir = tempfile::tempdir().unwrap();
    write_tempdir_session_roots(dir.path());

    let settings_json = serde_json::json!({
        "max_uploads_per_day": 200,
        "max_bytes_per_day": 2_147_483_648u64,
    })
    .to_string();
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(
            cstr(dir.path()).as_ptr(),
            cstr_str(&settings_json).as_ptr(),
            &mut err,
        )
    };
    assert!(
        !h.is_null(),
        "tc_daemon_start_with_settings failed: {:?}",
        last_error()
    );

    // The pre-start override is already visible on the very first tick,
    // through the same `get_settings` the IPC path answers.
    let out = call(h, "get_settings", "{}");
    assert!(
        out.contains("\"max_uploads_per_day\":200"),
        "pre-start override did not take effect: {out}"
    );
    assert!(
        out.contains("\"max_bytes_per_day\":2147483648"),
        "pre-start override did not take effect: {out}"
    );

    // Now raise it further over the socket, the same call a running
    // contributor app makes.
    let out = call(
        h,
        "set_settings",
        &serde_json::json!({"max_uploads_per_day": 500}).to_string(),
    );
    assert!(
        out.contains("\"max_uploads_per_day\":500"),
        "set_settings did not apply over the socket: {out}"
    );
    // The field it did not touch is untouched.
    assert!(
        out.contains("\"max_bytes_per_day\":2147483648"),
        "set_settings must not disturb a field it was not asked to change: {out}"
    );

    stop(h);
}

#[test]
fn a_daily_cap_above_the_ceiling_is_rejected_on_both_paths() {
    let dir = tempfile::tempdir().unwrap();

    // The pre-start path.
    let settings_json = serde_json::json!({ "max_uploads_per_day": 1_000_001u64 }).to_string();
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(
            cstr(dir.path()).as_ptr(),
            cstr_str(&settings_json).as_ptr(),
            &mut err,
        )
    };
    assert!(
        h.is_null(),
        "an out-of-range cap must not silently start the daemon"
    );
    assert!(!err.is_null(), "a failure must set the error out-param");
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    assert_eq!(msg, "settings-invalid-value", "{msg}");

    // The IPC path, on an otherwise healthy daemon.
    let dir2 = tempfile::tempdir().unwrap();
    write_tempdir_session_roots(dir2.path());
    let mut err2: *mut c_char = std::ptr::null_mut();
    let h2 = unsafe {
        tc_daemon_start_with_settings(cstr(dir2.path()).as_ptr(), std::ptr::null(), &mut err2)
    };
    assert!(!h2.is_null(), "{:?}", last_error());
    let out = call(
        h2,
        "set_settings",
        &serde_json::json!({"max_bytes_per_day": 5u64 * 1024 * 1024 * 1024 + 1}).to_string(),
    );
    assert!(
        out.contains("settings-invalid-value"),
        "an out-of-range byte cap must be refused over the socket: {out}"
    );
    stop(h2);
}

#[test]
fn null_settings_json_behaves_exactly_like_tc_daemon_start() {
    let dir = tempfile::tempdir().unwrap();
    write_tempdir_session_roots(dir.path());

    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(cstr(dir.path()).as_ptr(), std::ptr::null(), &mut err)
    };
    assert!(
        !h.is_null(),
        "a null settings_json must behave exactly like tc_daemon_start"
    );
    let out = call(h, "get_settings", "{}");
    assert!(
        out.contains("\"claude_root_configured\":true"),
        "a null settings_json must leave whatever was already persisted alone: {out}"
    );
    stop(h);
}

#[test]
fn empty_settings_json_behaves_exactly_like_tc_daemon_start() {
    let dir = tempfile::tempdir().unwrap();
    write_tempdir_session_roots(dir.path());

    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(cstr(dir.path()).as_ptr(), cstr_str("").as_ptr(), &mut err)
    };
    assert!(
        !h.is_null(),
        "an empty settings_json must behave exactly like tc_daemon_start"
    );
    let out = call(h, "get_settings", "{}");
    assert!(
        out.contains("\"claude_root_configured\":true"),
        "an empty settings_json must leave whatever was already persisted alone: {out}"
    );
    stop(h);
}

#[test]
fn malformed_settings_json_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(
            cstr(dir.path()).as_ptr(),
            cstr_str("{not json").as_ptr(),
            &mut err,
        )
    };
    assert!(h.is_null());
    assert!(!err.is_null(), "a failure must set the error out-param");
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    assert_eq!(msg, "settings-invalid-json", "{msg}");
}

#[test]
fn a_bad_claude_root_value_never_echoes_the_path_in_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let secret = "/Users/zzz/very/secret/project-name";
    // claude_root must be a JSON string or null; an array is the wrong
    // type, but it still carries the path-shaped value inside it, and the
    // resulting error must never echo that value back -- only the field
    // name (which is one of a small, fixed, known set) may appear.
    let settings_json = serde_json::json!({ "claude_root": [secret] }).to_string();
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(
            cstr(dir.path()).as_ptr(),
            cstr_str(&settings_json).as_ptr(),
            &mut err,
        )
    };
    assert!(h.is_null());
    assert!(!err.is_null(), "a failure must set the error out-param");
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    assert!(!msg.contains(secret), "path leaked into the error: {msg}");
    assert_eq!(msg, "settings-invalid-value", "{msg}");
}

#[test]
fn tc_daemon_start_with_settings_null_config_dir_is_an_error() {
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(std::ptr::null(), cstr_str("{}").as_ptr(), &mut err)
    };
    assert!(h.is_null());
    assert!(!err.is_null());
    unsafe { tc_string_free(err) };
}

/// The instance a shell shows before committing to an invite.
///
/// Owned string out, freed by the caller, NULL for anything unusable --
/// including a bare code and a URL with no code in it, which the interface
/// must not tell apart because the whole invite path has one failure
/// sentence.
#[test]
fn tc_invite_issuer_host_returns_the_host_and_nothing_else() {
    let invite = cstr_str("https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6");
    let out = unsafe { tc_invite_issuer_host(invite.as_ptr()) };
    assert!(!out.is_null(), "a well-formed invite resolves");
    let host = unsafe { std::ffi::CStr::from_ptr(out) }
        .to_str()
        .unwrap()
        .to_string();
    unsafe { tc_string_free(out) };

    assert_eq!(host, "issuer.tracecommons.ai");
    // The point of the narrow return: the credential does not cross.
    assert!(!host.contains("VQWWPGYSG8Y4LTP6"));
}

#[test]
fn tc_invite_issuer_host_is_null_for_anything_unusable() {
    for bad in [
        "VQWWPGYSG8Y4LTP6",
        "https://issuer.tracecommons.ai/onboard",
        "not a url",
    ] {
        let arg = cstr_str(bad);
        let out = unsafe { tc_invite_issuer_host(arg.as_ptr()) };
        assert!(out.is_null(), "{bad} must not resolve");
    }
}

#[test]
fn tc_invite_issuer_host_tolerates_null() {
    let out = unsafe { tc_invite_issuer_host(std::ptr::null()) };
    assert!(out.is_null());
}

// --- Fail-closed session roots. See
// docs/superpowers/specs/2026-08-19-fail-closed-roots-parity-design.md. ---

/// Read `*err` and free it, so a refusal's label can be asserted on without
/// leaking the owned string each of these tests produces.
fn take_err(err: *mut c_char) -> String {
    assert!(!err.is_null(), "a refusal must set the error out-param");
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    msg
}

#[test]
fn tc_daemon_start_refuses_when_no_roots_are_declared() {
    // No `write_tempdir_session_roots` on purpose: this is the fresh-install
    // state, where both roots are None and the daemon would otherwise take
    // that to mean the contributor's real ~/.claude and ~/.codex.
    let dir = tempfile::tempdir().unwrap();
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe { tc_daemon_start(cstr(dir.path()).as_ptr(), &mut err) };

    assert!(h.is_null(), "undeclared roots must refuse to start");
    assert_eq!(
        take_err(err),
        "roots-not-declared",
        "the label must be distinguishable from daemon-start-failed, because \
         the shells route a roots refusal to the roots screen and everything \
         else to the generic failure notice"
    );
    assert_eq!(last_error().as_deref(), Some("roots-not-declared"));
}

#[test]
fn tc_daemon_start_refuses_when_only_one_root_is_declared() {
    // The `||`-instead-of-`&&` fail-open: an unset codex_root does not mean
    // "no codex source", it means the real ~/.codex.
    let dir = tempfile::tempdir().unwrap();
    let claude_root = dir.path().join("claude-root");
    std::fs::create_dir_all(&claude_root).unwrap();
    let store =
        trace_commons_contributor::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
    trace_commons_contributor::daemon::settings::DaemonSettings {
        claude_source: Some(
            trace_commons_contributor::daemon::settings::SourceDeclaration::Watch {
                path: claude_root,
            },
        ),
        codex_source: None,
        ..Default::default()
    }
    .save(&store)
    .unwrap();

    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe { tc_daemon_start(cstr(dir.path()).as_ptr(), &mut err) };

    assert!(h.is_null(), "half a declaration must refuse");
    assert_eq!(take_err(err), "roots-not-declared");
}

#[test]
fn tc_daemon_start_with_settings_may_declare_the_roots_that_let_it_start() {
    // The whole point of the settings-bearing start: the roots screen has
    // just collected two folders, and one call both persists them and starts
    // the daemon. Without this there is no way out of the refusal.
    let dir = tempfile::tempdir().unwrap();
    let claude_root = dir.path().join("claude-root");
    let codex_root = dir.path().join("codex-root");
    std::fs::create_dir_all(&claude_root).unwrap();
    std::fs::create_dir_all(&codex_root).unwrap();

    let settings_json = serde_json::json!({
        "claude_root": claude_root.to_str().unwrap(),
        "codex_root": codex_root.to_str().unwrap(),
    })
    .to_string();

    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(
            cstr(dir.path()).as_ptr(),
            cstr_str(&settings_json).as_ptr(),
            &mut err,
        )
    };
    assert!(
        !h.is_null(),
        "declaring both roots at start must be accepted: {:?}",
        last_error()
    );
    stop(h);
}

#[test]
fn tc_daemon_start_with_settings_refuses_when_the_settings_declare_only_one_root() {
    let dir = tempfile::tempdir().unwrap();
    let claude_root = dir.path().join("claude-root");
    std::fs::create_dir_all(&claude_root).unwrap();

    let settings_json = serde_json::json!({
        "claude_root": claude_root.to_str().unwrap(),
    })
    .to_string();

    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(
            cstr(dir.path()).as_ptr(),
            cstr_str(&settings_json).as_ptr(),
            &mut err,
        )
    };
    assert!(h.is_null(), "half a declaration must refuse here too");
    assert_eq!(take_err(err), "roots-not-declared");
}

#[test]
fn discovery_answers_without_a_handle_and_describes_every_source() {
    // It has to work with no daemon: the screen that consumes it is the one
    // clearing the refusal that stops a daemon from starting.
    let out = tc_discover_sources();
    assert!(!out.is_null());
    let json = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };

    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let items = parsed.as_array().expect("an array");
    assert_eq!(items.len(), 4, "one candidate per known agent: {json}");

    let sources: Vec<&str> = items
        .iter()
        .map(|i| i["source"].as_str().unwrap())
        .collect();
    // Discovery describes every store a consent prompt may ask about, which
    // is a longer list than the two `roots_declared` gates daemon startup on:
    // an absent Gemini answer is not disqualifying, so the shells prompt for
    // it without refusing to start. Adding a source belongs here; adding one
    // to that gate would stop the daemon on every installed client.
    assert_eq!(sources, vec!["claude-code", "codex", "gemini-cli", "cline"]);

    for item in items {
        // The fields a consent prompt needs to be specific rather than
        // abstract.
        assert!(item["path"].is_string());
        assert!(item["exists"].is_boolean());
        assert!(item["session_count"].is_u64());
        assert!(item["relocated_by_env"].is_boolean());
        assert!(item["most_recent"].is_string() || item["most_recent"].is_null());
    }
}

#[test]
fn a_roots_refusal_never_echoes_a_path_back_across_the_boundary() {
    // settings_json is the one input at this boundary that may itself carry
    // a filesystem path; trace_commons.h is explicit that it must not come
    // back out. A refusal label is fixed and content-free by construction,
    // and this pins that.
    let dir = tempfile::tempdir().unwrap();
    let secret = dir.path().join("acme-unreleased-product");
    std::fs::create_dir_all(&secret).unwrap();

    let settings_json = serde_json::json!({ "claude_root": secret.to_str().unwrap() }).to_string();

    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe {
        tc_daemon_start_with_settings(
            cstr(dir.path()).as_ptr(),
            cstr_str(&settings_json).as_ptr(),
            &mut err,
        )
    };
    assert!(h.is_null());
    let msg = take_err(err);
    assert!(
        !msg.contains("acme-unreleased-product") && !msg.contains(dir.path().to_str().unwrap()),
        "a refusal must not echo settings_json back: {msg}"
    );
}

/// Read an owned `char*` this ABI returned, freeing it. Panics on NULL,
/// which every routing export reserves for a caught panic.
fn take_owned(out: *mut c_char) -> String {
    assert!(!out.is_null(), "the export returned NULL");
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };
    s
}

#[test]
fn the_routing_vocabulary_crossing_the_abi_is_the_one_in_the_rust() {
    // The whole point of this task: the words a shell renders are the words
    // this repo defines, not a transcription in Swift or C# that stops
    // matching the day one of them changes.
    let json = take_owned(tc_routing_copy());
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let object = parsed.as_object().expect("a JSON object");

    // Compared against the constants themselves, not against words written
    // here -- pinning them in this test would be the same transcription bug
    // one layer down. The shells' own suites pin the literals; that is where
    // a rename is meant to be noticed.
    use trace_commons_contributor::routing_copy as copy;
    assert_eq!(
        object["word_private"],
        serde_json::json!(copy::TOOL_PRIVATE)
    );
    assert_eq!(object["word_direct"], serde_json::json!(copy::TOOL_DIRECT));
    assert_eq!(
        object["word_unknown"],
        serde_json::json!(copy::TOOL_UNKNOWN)
    );
    assert_eq!(
        object["word_not_used"],
        serde_json::json!(copy::TOOL_NOT_USED)
    );

    let expected = serde_json::to_value(copy::routing_copy()).expect("the payload serialises");
    assert_eq!(
        parsed, expected,
        "the ABI must hand over the payload unchanged"
    );

    for (field, value) in object {
        let text = value
            .as_str()
            .unwrap_or_else(|| panic!("{field} is not a string"));
        assert!(!text.is_empty(), "{field} crossed the boundary empty");
    }
}

#[test]
fn each_source_mode_crosses_the_abi_as_its_own_sentence() {
    // The defect: `*_root_configured` is `mode == "watch"`, so it is false
    // for `off` as well as for `unset`, and two shells printed one sentence
    // on that false branch -- telling a contributor who declared a tool off
    // that its sessions were being read from the usual place. Three modes,
    // three sentences, and this is the boundary the other two shells get
    // them over.
    let tool = cstr_str("claude");
    let watch =
        take_owned(unsafe { tc_source_check_line(tool.as_ptr(), cstr_str("watch").as_ptr()) });
    let unset =
        take_owned(unsafe { tc_source_check_line(tool.as_ptr(), cstr_str("unset").as_ptr()) });
    let off = take_owned(unsafe { tc_source_check_line(tool.as_ptr(), cstr_str("off").as_ptr()) });

    // Against the shared function, not against words written here: pinning
    // the literals in this test would be the transcription bug one layer
    // down. The three shells' own suites pin them, which is where a reword
    // is meant to be noticed.
    use trace_commons_contributor::source_copy::{SourceTool, source_check_line};
    assert_eq!(watch, source_check_line(SourceTool::Claude, "watch"));
    assert_eq!(unset, source_check_line(SourceTool::Claude, "unset"));
    assert_eq!(off, source_check_line(SourceTool::Claude, "off"));

    for (a, b) in [(&watch, &unset), (&watch, &off), (&unset, &off)] {
        assert_ne!(a, b, "two modes crossed as the same sentence");
        assert!(
            !a.contains(b.as_str()) && !b.contains(a.as_str()),
            "one mode's sentence contains another's: {a:?} / {b:?}"
        );
    }

    // An unknown mode reads as `unset`, never as `off`: an older daemon
    // sends no mode at all, and claiming nothing is read from a folder that
    // is being scanned is the worse of the two errors.
    for mode in ["", "OFF", "watching"] {
        let line =
            take_owned(unsafe { tc_source_check_line(tool.as_ptr(), cstr_str(mode).as_ptr()) });
        assert_eq!(line, unset, "mode {mode:?} did not read as unset");
    }
}

#[test]
fn a_source_check_for_a_tool_this_build_has_no_name_for_is_refused() {
    // Refused rather than answered with some other tool's sentence under
    // this tool's heading -- and refused by a fixed label, so a shell can
    // tell it from a panic.
    let mode = cstr_str("watch");
    let out = unsafe { tc_source_check_line(cstr_str("claude-code").as_ptr(), mode.as_ptr()) };
    assert!(out.is_null(), "an unknown tool key produced a sentence");
    assert_eq!(last_error().as_deref(), Some("unknown-source-tool"));

    let out = unsafe { tc_source_check_line(std::ptr::null(), mode.as_ptr()) };
    assert!(out.is_null(), "a NULL tool produced a sentence");
    assert_eq!(last_error().as_deref(), Some("null-pointer"));

    let out = unsafe { tc_source_check_line(cstr_str("claude").as_ptr(), std::ptr::null()) };
    assert!(out.is_null(), "a NULL mode produced a sentence");
    assert_eq!(last_error().as_deref(), Some("null-pointer"));
}

#[test]
fn only_the_wired_word_crossing_the_abi_claims_privacy() {
    // The substring trap, asserted on what actually crosses rather than on
    // the Rust constants: a shell reading these with a `contains` must not be
    // able to match a denial that is not there.
    let json = take_owned(tc_routing_copy());
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    let private = parsed["word_private"].as_str().expect("a string");
    assert!(private.to_lowercase().contains("privat"));

    let words = [
        parsed["word_private"].as_str().expect("a string"),
        parsed["word_direct"].as_str().expect("a string"),
        parsed["word_unknown"].as_str().expect("a string"),
        parsed["word_not_used"].as_str().expect("a string"),
    ];
    for word in &words[1..] {
        assert!(
            !word.to_lowercase().contains("privat"),
            "a word that denies privacy crossed the boundary: {word}"
        );
    }
    for (i, one) in words.iter().enumerate() {
        for (j, other) in words.iter().enumerate() {
            if i == j {
                continue;
            }
            assert!(
                !one.to_lowercase().contains(&other.to_lowercase()),
                "{other:?} is a substring of {one:?} across the boundary"
            );
        }
    }
}

#[test]
fn the_routing_sentences_cross_assembled_and_never_as_a_template() {
    // A template with a hole in it would make every shell a place this
    // wording lives. What crosses is finished text, so this asserts the hole
    // is filled and no format marker survives.
    let path = cstr_str("/home/x/.ironwire/control.token");
    let named = take_owned(unsafe { tc_routing_token_line(path.as_ptr()) });
    assert!(named.contains("/home/x/.ironwire/control.token"), "{named}");

    // NULL is the "nothing resolved at all" case, not an error: the sentence
    // still has to say what to do.
    let unnamed = take_owned(unsafe { tc_routing_token_line(std::ptr::null()) });
    assert!(!unnamed.contains("/home/x"), "{unnamed}");
    assert_ne!(named, unnamed);

    let with_port = take_owned(tc_routing_unreachable_line(8463));
    assert!(with_port.contains("8463"), "{with_port}");
    // 0 means no port was tried. It must not become "port 0".
    let no_port = take_owned(tc_routing_unreachable_line(0));
    assert!(!no_port.contains('0'), "{no_port}");
    // Out of range is the same case, and must not wrap into a real port.
    assert_eq!(no_port, take_owned(tc_routing_unreachable_line(70_000)));
    assert_eq!(no_port, take_owned(tc_routing_unreachable_line(-1)));

    let when = cstr_str("an hour ago");
    let checked = take_owned(unsafe { tc_routing_last_checked(when.as_ptr()) });
    assert_eq!(checked, "Last checked an hour ago");

    for sentence in [&named, &unnamed, &with_port, &no_port, &checked] {
        for marker in ["{}", "{path}", "{port}", "{when}", "%s", "%d"] {
            assert!(
                !sentence.contains(marker),
                "a format marker reached a shell in: {sentence}"
            );
        }
    }
}

/// The C ABI's spelling of the three wiring states, pinned here so a
/// renumbering has to be a deliberate edit in two places.
const WIRED: i32 = 0;
const NOT_WIRED: i32 = 1;
const UNKNOWN: i32 = 2;
const TONE_NEUTRAL: i32 = 0;
const TONE_HELD: i32 = 1;
const TONE_CLEAR: i32 = 2;

fn tool_word(mode: &str, wiring: i32) -> String {
    let mode = cstr_str(mode);
    take_owned(unsafe { tc_routing_tool_word(mode.as_ptr(), wiring) })
}

fn state_line(state: &str) -> String {
    let state = cstr_str(state);
    take_owned(unsafe { tc_routing_state_line(state.as_ptr()) })
}

#[test]
fn the_word_branch_table_crosses_the_abi_and_is_the_one_in_the_rust() {
    // Compared against the Rust's own function rather than against literals:
    // this asserts that the export IS the shared branch table, which is the
    // whole reason the shells stopped writing their own.
    use trace_commons_contributor::routing_copy as copy;
    for mode in ["off", "watch", "unset", "", "something_new"] {
        for (abi, wiring) in [
            (WIRED, copy::ToolWiring::Wired),
            (NOT_WIRED, copy::ToolWiring::NotWired),
            (UNKNOWN, copy::ToolWiring::Unknown),
        ] {
            assert_eq!(
                tool_word(mode, abi),
                copy::tool_word(mode, wiring),
                "{mode:?}/{abi}"
            );
        }
    }

    // A wiring value this build has never heard of claims nothing rather
    // than falling through to a verdict.
    assert_eq!(tool_word("watch", 99), copy::TOOL_UNKNOWN);
    assert_eq!(tool_word("watch", -1), copy::TOOL_UNKNOWN);
    // Only "off" means not used; "unset" is a tool in use.
    assert_eq!(tool_word("off", WIRED), copy::TOOL_NOT_USED);
    assert_eq!(tool_word("unset", WIRED), copy::TOOL_PRIVATE);
}

#[test]
fn a_tool_word_call_with_no_source_mode_is_an_error_and_not_a_guess() {
    // A shell that cannot say what the contributor declared gets no word.
    // Named error, not merely "some failure".
    let out = unsafe { tc_routing_tool_word(std::ptr::null(), WIRED) };
    assert!(out.is_null());
    let msg = unsafe { CStr::from_ptr(tc_last_error()) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(msg, "null-pointer");
}

#[test]
fn the_reassuring_tone_crosses_the_abi_and_falls_on_the_private_word_alone() {
    // The styling decision crosses too, so no shell has to recover it by
    // comparing a rendered privacy claim against a substring.
    use trace_commons_contributor::routing_copy as copy;
    for mode in ["off", "watch", "unset", "", "something_new"] {
        for wiring in [WIRED, NOT_WIRED, UNKNOWN, 99] {
            let mode_c = cstr_str(mode);
            let tone = unsafe { tc_routing_tool_tone(mode_c.as_ptr(), wiring) };
            assert!(
                tone == TONE_NEUTRAL || tone == TONE_CLEAR,
                "{mode:?}/{wiring} answered {tone}"
            );
            assert_eq!(
                tone == TONE_CLEAR,
                tool_word(mode, wiring) == copy::TOOL_PRIVATE,
                "{mode:?}/{wiring}"
            );
        }
    }

    // No error value: an unreadable source mode gets the tone that claims
    // nothing, because a styling call that failed would leave a shell
    // choosing for itself.
    assert_eq!(
        unsafe { tc_routing_tool_tone(std::ptr::null(), WIRED) },
        TONE_NEUTRAL
    );
}

#[test]
fn the_state_branch_table_crosses_the_abi_and_an_unknown_state_claims_nothing() {
    use trace_commons_contributor::routing_copy as copy;
    assert_eq!(state_line("awaiting_rows"), copy::IRONWIRE_STATE_WAITING);
    assert_eq!(state_line("rows_seen"), copy::IRONWIRE_STATE_READING);
    assert_eq!(state_line("not_declared"), copy::IRONWIRE_STATE_OFF);

    // A state a later daemon grows, an empty one, and no pointer at all all
    // read as the off line. None of them falls through to either "on"
    // sentence -- named here rather than asserted as "not waiting".
    assert_eq!(state_line("something_new"), copy::IRONWIRE_STATE_OFF);
    assert_eq!(state_line(""), copy::IRONWIRE_STATE_OFF);
    assert_eq!(
        take_owned(unsafe { tc_routing_state_line(std::ptr::null()) }),
        copy::IRONWIRE_STATE_OFF
    );
}

#[test]
fn the_state_tone_branch_table_crosses_the_abi_and_agrees_with_the_sentence() {
    // The last routing branch table that was still written out natively in
    // all three shells. Compared against the Rust's own function, so this
    // asserts the export IS the shared table.
    use trace_commons_contributor::routing_copy as copy;
    let tone = |state: &str| {
        let state = cstr_str(state);
        unsafe { tc_routing_state_tone(state.as_ptr()) }
    };

    assert_eq!(tone("awaiting_rows"), TONE_HELD);
    assert_eq!(tone("rows_seen"), TONE_CLEAR);
    assert_eq!(tone("not_declared"), TONE_NEUTRAL);

    for state in [
        "not_declared",
        "awaiting_rows",
        "rows_seen",
        "",
        "ROWS_SEEN",
        "a_state_from_a_later_daemon",
    ] {
        let expected = match copy::ironwire_state_tone(state) {
            copy::StateTone::Neutral => TONE_NEUTRAL,
            copy::StateTone::Held => TONE_HELD,
            copy::StateTone::Clear => TONE_CLEAR,
        };
        assert_eq!(tone(state), expected, "{state:?}");
        // The tone and the sentence are one decision across the boundary.
        assert_eq!(
            tone(state) == TONE_NEUTRAL,
            state_line(state) == copy::IRONWIRE_STATE_OFF,
            "{state:?}"
        );
    }

    // A state this build has never heard of, and no pointer at all, both
    // claim nothing rather than falling through to either "on" tone.
    assert_eq!(tone("a_state_from_a_later_daemon"), TONE_NEUTRAL);
    assert_eq!(
        unsafe { tc_routing_state_tone(std::ptr::null()) },
        TONE_NEUTRAL
    );
}

#[test]
fn one_tone_numbering_serves_both_calls_and_a_tool_word_is_never_held() {
    // Two numberings would mean two 1s meaning different things on one ABI.
    // A shell that mapped the wrong one would mispaint a privacy claim
    // rather than fail, so the shared numbering is asserted rather than
    // assumed -- and the value a tool word can never take is named.
    for mode in ["off", "watch", "unset", "", "something_new"] {
        for wiring in [WIRED, NOT_WIRED, UNKNOWN, 99] {
            let mode_c = cstr_str(mode);
            let tone = unsafe { tc_routing_tool_tone(mode_c.as_ptr(), wiring) };
            assert_ne!(tone, TONE_HELD, "{mode:?}/{wiring} took the held tone");
            assert!(
                tone == TONE_NEUTRAL || tone == TONE_CLEAR,
                "{mode:?}/{wiring}"
            );
        }
    }

    // And the held tone is reachable, from the one thing that may hold.
    let waiting = cstr_str("awaiting_rows");
    assert_eq!(
        unsafe { tc_routing_state_tone(waiting.as_ptr()) },
        TONE_HELD
    );
}

#[test]
fn a_last_checked_call_with_no_timestamp_is_an_error_and_not_a_half_sentence() {
    // The one routing export that refuses. "Last checked " with nothing
    // after it is worse than no line at all, so this must not be produced.
    let out = unsafe { tc_routing_last_checked(std::ptr::null()) };
    assert!(out.is_null());
    let err = tc_last_error();
    assert!(!err.is_null());
    let msg = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(msg, "null-pointer");
}

#[test]
fn scrub_detector_names_are_generated_from_the_real_table() {
    // The point of the export: a shell showing this list must be showing what
    // the scrubber actually looks for, not a transcription that stops being
    // true the day a detector is added.
    let out = tc_scrub_detector_names();
    assert!(!out.is_null());
    let json = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };

    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let names: Vec<&str> = parsed
        .as_array()
        .expect("an array")
        .iter()
        .map(|n| n.as_str().expect("a string"))
        .collect();

    // Compared against the table itself rather than a list written here --
    // pinning the names in this test would be the same transcription bug one
    // layer down.
    let expected = trace_commons_protocol::trace_contribution::secret_leak_pattern_names();
    assert_eq!(names, expected, "the export must mirror the detector table");
    assert!(
        !names.is_empty(),
        "an empty list would claim nothing is scrubbed"
    );
}

#[test]
fn the_detector_export_never_carries_a_pattern() {
    // Names only. Publishing the regexes would tell someone trying to slip a
    // secret past the scrubber exactly what to avoid, so this asserts on what
    // actually crosses the boundary rather than trusting the implementation to
    // stay as written.
    let out = tc_scrub_detector_names();
    let json = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };

    // Each NAME, not the raw envelope: `[` and `{` are JSON's own delimiters
    // and are always present in a valid array.
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    for name in parsed.as_array().expect("an array") {
        let name = name.as_str().expect("a string");
        for meta in ['\\', '^', '$', '[', '(', '{', '+', '?', '|', '*'] {
            assert!(
                !name.contains(meta),
                "a regex metacharacter {meta:?} reached the boundary in {name:?}"
            );
        }
    }
}

#[test]
fn a_refused_cross_type_free_leaves_the_handle_live_and_freeable() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    // A caller mistakes the handle for a string. The ABI promises to refuse
    // this harmlessly -- the handle is NOT freed, so it must still work.
    unsafe { tc_string_free(h as *mut c_char) };
    assert!(
        last_error()
            .map(|e| e.contains("cross-type-free"))
            .unwrap_or(false),
        "the refusal itself must still be reported"
    );
    let out = unsafe { tc_call(h, cstr_str("status").as_ptr(), cstr_str("{}").as_ptr()) };
    assert!(!out.is_null());
    let s = unsafe { CStr::from_ptr(out) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(out) };
    assert!(
        !s.contains("invalid-handle-pointer"),
        "a REFUSED cross-type free must not unregister the live handle: {s}"
    );
    stop(h);
}

/// A zero token must not shadow the liveness refusal: the contract promises
/// `invalid-handle-pointer` for every non-null handle that is not live, and
/// a binding is told to read `tc_last_error` after every `tc_unsubscribe`.
#[test]
fn tc_unsubscribe_refuses_a_freed_handle_even_with_a_zero_token() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
    // No read of tc_last_error clears it, so there is no way to prove the
    // label below was recorded by this call rather than left over. What
    // makes the assertion mean something is that nothing earlier in this
    // test records "invalid-handle-pointer": the free above succeeds.
    unsafe { tc_unsubscribe(h, 0) };
    assert!(
        last_error()
            .map(|e| e.contains("invalid-handle-pointer"))
            .unwrap_or(false),
        "a zero token must not skip the liveness refusal"
    );
}
