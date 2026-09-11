//! Round-trips the C ABI from a Rust integration test that links the crate
//! as an `rlib` (see the crate's `Cargo.toml` comment on why `rlib` is
//! included alongside `cdylib`/`staticlib`). Every helper here frees every
//! string it receives, so a leak-detector run over this file is a genuine
//! check of the ownership rule stated in `include/trace_commons.h`.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use trace_commons_contributor_ffi::{
    TC_CONTRIBUTION_CONTROL_CONTRIBUTE, TC_CONTRIBUTION_CONTROL_NONE, TC_CREDENTIAL_ACTION_CANCEL,
    TC_CREDENTIAL_ACTION_FORGET, TC_CREDENTIAL_ACTION_NONE, TC_CREDENTIAL_ACTION_OBTAIN,
    TC_HARNESS_PLAN_CHANGES, TC_HARNESS_PLAN_ENTRY_UNUSABLE, TC_HARNESS_PLAN_NO_CONFIG_PATH,
    TC_HARNESS_PLAN_NOOP, TC_HARNESS_PLAN_NOT_INSTALLED, TC_HARNESS_PLAN_UNKNOWN,
    TC_HARNESS_PLAN_UNPARSEABLE, TC_HARNESS_STATE_ACTIVITY_SHARED, TC_HARNESS_STATE_ANSWERING,
    TC_HARNESS_STATE_CONNECTED_NO_CALLS, TC_HARNESS_STATE_NOT_CONNECTED, TC_HARNESS_STATE_UNKNOWN,
    TC_PRIVATE_INFERENCE_TONE_ATTENTION, TC_PRIVATE_INFERENCE_TONE_CLEAR,
    TC_PRIVATE_INFERENCE_TONE_HELD, TC_PRIVATE_INFERENCE_TONE_NEUTRAL,
    TC_PRIVATE_INFERENCE_TONE_REFUSED, TC_WITNESS_STATE_ABSENT, TC_WITNESS_STATE_NOT_ENROLLED,
    TC_WITNESS_STATE_PINNED, TC_WITNESS_STATE_REFUSING_INFERENCE_RECEIPTS_MISSING,
    TC_WITNESS_STATE_REFUSING_PIN_MALFORMED, TC_WITNESS_STATE_REFUSING_UNPINNED,
    TC_WITNESS_STATE_UNREADABLE, TC_WITNESS_TONE_ATTENTION, TC_WITNESS_TONE_CLEAR,
    TC_WITNESS_TONE_HELD, TC_WITNESS_TONE_NEUTRAL, TC_WITNESS_TONE_REFUSED, tc_call,
    tc_certificate_list_title, tc_certificate_row_line, tc_consent_copy, tc_consent_gate_help,
    tc_contribution_attestation_line, tc_contribution_attestation_reason_line,
    tc_contribution_attestation_tone, tc_contribution_eligibility_control,
    tc_contribution_eligibility_line, tc_contribution_eligibility_reason_line,
    tc_contribution_eligibility_tone, tc_contribution_group_control, tc_contribution_withheld_line,
    tc_daemon_start, tc_daemon_start_with_settings, tc_daemon_stop, tc_discover_sources, tc_handle,
    tc_handle_free, tc_invite_issuer_host, tc_last_error, tc_near_ai_credential_action,
    tc_near_ai_credential_state_line, tc_near_ai_credential_state_tone, tc_near_ai_enroll_line,
    tc_near_ai_enroll_tone, tc_preview, tc_preview_body, tc_preview_open, tc_preview_search,
    tc_preview_summary_json, tc_preview_turns_json, tc_private_inference_copy,
    tc_private_inference_quit_needs_notice, tc_private_inference_serving_line,
    tc_private_inference_should_offer, tc_private_inference_state_line,
    tc_private_inference_state_tone, tc_public_run_copy, tc_public_run_error_line,
    tc_public_run_validate_editor, tc_routing_copy, tc_routing_discovery_line,
    tc_routing_last_checked, tc_routing_state_line, tc_routing_state_tone, tc_routing_token_line,
    tc_routing_tool_tone, tc_routing_tool_word, tc_routing_unreachable_line,
    tc_scrub_detector_names, tc_search_original, tc_session_detail_error_line,
    tc_source_check_line, tc_string_free, tc_subscribe, tc_unsubscribe, tc_witness_clear,
    tc_witness_configure, tc_witness_copy, tc_witness_last_result_json,
    tc_witness_last_result_line, tc_witness_last_result_tone, tc_witness_state_line,
    tc_witness_state_tone, tc_witness_status_json, tc_witness_trust_state,
};
use trace_commons_contributor_ffi::{
    tc_harness_action_available, tc_harness_last_call_line, tc_harness_outcome_line,
    tc_harness_plan_outcome_code, tc_harness_spend_line, tc_harness_state_code,
    tc_harness_state_line,
};
use trace_commons_contributor_ffi::{
    tc_near_ai_balance_action, tc_near_ai_balance_amount, tc_near_ai_balance_limit_line,
    tc_near_ai_balance_observed_line, tc_near_ai_balance_remaining_line,
    tc_near_ai_balance_spent_line, tc_near_ai_balance_state_line, tc_near_ai_balance_state_tone,
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
fn search_original_refuses_a_dead_handle() {
    let needle = cstr_str("anything");
    let id = cstr_str("11111111-1111-1111-1111-111111111111");
    let n = unsafe { tc_search_original(std::ptr::null_mut(), id.as_ptr(), needle.as_ptr()) };
    assert_eq!(n, -1, "a null handle must be refused, not dereferenced");
}

#[test]
fn search_original_refuses_an_unparseable_entry_id() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let needle = cstr_str("anything");
    let id = cstr_str("not-a-uuid");
    let n = unsafe { tc_search_original(h, id.as_ptr(), needle.as_ptr()) };
    assert_eq!(n, -1);
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
}

/// An entry the daemon does not know is an error, never a zero. Reporting
/// "not in this session" for a lookup that never happened would be the most
/// dangerous wrong answer this call can give.
#[test]
fn search_original_refuses_an_unknown_entry_rather_than_reporting_zero() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let needle = cstr_str("anything");
    let id = cstr_str("11111111-1111-1111-1111-111111111111");
    let n = unsafe { tc_search_original(h, id.as_ptr(), needle.as_ptr()) };
    assert_eq!(n, -1);
    unsafe { tc_daemon_stop(h) };
    unsafe { tc_handle_free(h) };
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
fn inference_consent_round_trips_without_changing_proxy_or_other_permissions() {
    let dir = tempfile::tempdir().unwrap();
    let handle = start(dir.path());
    let read = |method: &str, params: &str| {
        let frame: serde_json::Value = serde_json::from_str(&call(handle, method, params)).unwrap();
        assert!(frame.get("error").is_none_or(serde_json::Value::is_null));
        frame["result"].clone()
    };
    let baseline = read("get_settings", "{}");
    assert_eq!(baseline["ironwire_attested_bodies"], false);

    for enabled in [true, false] {
        let mut expected = baseline.clone();
        expected["ironwire_attested_bodies"] = enabled.into();
        let patch = serde_json::json!({"ironwire_attested_bodies": enabled}).to_string();
        assert_eq!(read("set_settings", &patch), expected);
        assert_eq!(read("get_settings", "{}"), expected);
        let store =
            trace_commons_contributor::config::ConfigStore::open(dir.path().into()).unwrap();
        let saved =
            trace_commons_contributor::daemon::settings::DaemonSettings::load(&store).unwrap();
        assert_eq!(saved.ironwire_attested_bodies, enabled);
        assert!(saved.ironwire.is_none());
        assert!(saved.near_ai.is_none());
    }
    stop(handle);
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
fn a_cline_source_check_crosses_the_abi_as_the_rust_sentence() {
    // Cline has a settings row the way Gemini CLI does, and its key crosses
    // this boundary the same way. Against the shared function, not against
    // words written here, for the reason the Claude case gives.
    use trace_commons_contributor::source_copy::{SourceTool, source_check_line};
    let tool = cstr_str("cline");
    for mode in ["watch", "unset", "off"] {
        let line =
            take_owned(unsafe { tc_source_check_line(tool.as_ptr(), cstr_str(mode).as_ptr()) });
        assert_eq!(line, source_check_line(SourceTool::Cline, mode));
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

    let discovered = take_owned(tc_routing_discovery_line(9143));
    assert!(discovered.contains("9143"), "{discovered}");
    // 0 is "nothing was discovered", which is the ordinary machine and not
    // an error. It must not become "port 0", and it must not wrap.
    let nothing = take_owned(tc_routing_discovery_line(0));
    assert!(!nothing.contains('0'), "{nothing}");
    assert_ne!(discovered, nothing);
    assert_eq!(nothing, take_owned(tc_routing_discovery_line(70_000)));
    assert_eq!(nothing, take_owned(tc_routing_discovery_line(-1)));

    let when = cstr_str("an hour ago");
    let checked = take_owned(unsafe { tc_routing_last_checked(when.as_ptr()) });
    assert_eq!(checked, "Last checked an hour ago");

    for sentence in [
        &named,
        &unnamed,
        &with_port,
        &no_port,
        &discovered,
        &nothing,
        &checked,
    ] {
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
const TONE_ATTENTION: i32 = 3;

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
    assert_eq!(
        state_line("token_unreadable"),
        copy::IRONWIRE_STATE_TOKEN_UNREADABLE
    );
    // The state that used to arrive as the unknown one. It must not read as
    // off across the boundary either: this is the sentence a contributor
    // with the switch on sees.
    assert_ne!(state_line("token_unreadable"), copy::IRONWIRE_STATE_OFF);

    // A new nonempty state is unavailable; legacy empty input retains its
    // existing behavior. None falls through to an "on" sentence.
    assert_eq!(state_line("something_new"), copy::IRONWIRE_STATE_UNKNOWN);
    assert_eq!(state_line("unknown"), copy::IRONWIRE_STATE_UNKNOWN);
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
    // Neither the calm tone nor the all-clear one, across the ABI.
    assert_eq!(tone("token_unreadable"), TONE_ATTENTION);
    assert_ne!(tone("token_unreadable"), TONE_CLEAR);
    assert_ne!(tone("token_unreadable"), TONE_HELD);
    assert_ne!(tone("token_unreadable"), TONE_NEUTRAL);

    for (state, expected_line) in [
        ("not_declared", copy::IRONWIRE_STATE_OFF),
        ("awaiting_rows", copy::IRONWIRE_STATE_WAITING),
        ("rows_seen", copy::IRONWIRE_STATE_READING),
        ("token_unreadable", copy::IRONWIRE_STATE_TOKEN_UNREADABLE),
        // Empty input keeps the documented legacy behavior; nonempty
        // unknown labels must not imply Off or diagnose a token failure.
        ("", copy::IRONWIRE_STATE_OFF),
        ("unknown", copy::IRONWIRE_STATE_UNKNOWN),
        ("ROWS_SEEN", copy::IRONWIRE_STATE_UNKNOWN),
        ("a_state_from_a_later_daemon", copy::IRONWIRE_STATE_UNKNOWN),
    ] {
        let expected = match copy::ironwire_state_tone(state) {
            copy::StateTone::Neutral => TONE_NEUTRAL,
            copy::StateTone::Held => TONE_HELD,
            copy::StateTone::Clear => TONE_CLEAR,
            copy::StateTone::Attention => TONE_ATTENTION,
        };
        assert_eq!(tone(state), expected, "{state:?}");
        assert_eq!(state_line(state), expected_line, "{state:?}");
        // Neutral is a presentation category, not proof of Off. Both Off
        // and unavailable are neutral, but their sentences remain distinct.
        assert_eq!(
            tone(state) == TONE_NEUTRAL,
            expected_line == copy::IRONWIRE_STATE_OFF
                || expected_line == copy::IRONWIRE_STATE_UNKNOWN,
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

// ---------------------------------------------------------------------------
// The redaction witness
// ---------------------------------------------------------------------------

/// Write an enrolled contributor config into `dir`, with `witness` as given.
///
/// The witness calls all read and write the contributor config, so a test
/// needs one on disk. Nothing here starts a daemon.
fn write_enrolled_config(
    dir: &Path,
    witness: Option<trace_commons_contributor::config::WitnessSettings>,
) {
    let store = trace_commons_contributor::config::ConfigStore::open(dir.to_path_buf()).unwrap();
    let cfg = trace_commons_contributor::config::ContributorConfig {
        inference_receipt_endpoint: None,
        inference_receipt_check_attestation: false,
        schema_version: trace_commons_contributor::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION
            .to_string(),
        issuer_url: "https://issuer.example".into(),
        ingest_url: "https://ingest.example".into(),
        audience: "trace-commons-ingest".into(),
        tenant_id: "tenant".into(),
        instance_id: "instance".into(),
        user_subject: "subject".into(),
        device_key_id: "device".into(),
        consent_scopes: vec![],
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness,
    };
    store.save_config(&cfg).unwrap();
}

/// One measurement set in `ExpectedMeasurements`' own spelling.
fn a_pin() -> String {
    format!("mrtd={}", "ab".repeat(48))
}

fn witness_settings(
    measurements: Vec<String>,
) -> trace_commons_contributor::config::WitnessSettings {
    trace_commons_contributor::config::WitnessSettings {
        admission_evidence: false,
        url: "https://witness.example".into(),
        signing_address: "0xfeed".into(),
        expected_measurements: measurements,
    }
}

fn witness_status_json(dir: &Path) -> serde_json::Value {
    let path = cstr(dir);
    let mut err: *mut c_char = std::ptr::null_mut();
    let raw = unsafe { tc_witness_status_json(path.as_ptr(), &mut err) };
    assert!(!raw.is_null(), "status json failed: {:?}", last_error());
    let text = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_string();
    unsafe { tc_string_free(raw) };
    serde_json::from_str(&text).unwrap()
}

fn last_result_json() -> serde_json::Value {
    let raw = tc_witness_last_result_json();
    assert!(!raw.is_null());
    let text = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_string();
    unsafe { tc_string_free(raw) };
    serde_json::from_str(&text).unwrap()
}

/// The header's `#define`s must be the numbers the library returns.
///
/// The two are hand-synced -- the header is written by hand -- so a value
/// changed on one side and not the other is a shipped app rendering the
/// wrong state with no build failure anywhere. This parses the header rather
/// than restating the numbers, so it cannot pass by agreeing with itself.
#[test]
fn the_headers_witness_state_defines_match_the_library() {
    let expected = [
        ("TC_WITNESS_STATE_ABSENT", TC_WITNESS_STATE_ABSENT),
        ("TC_WITNESS_STATE_PINNED", TC_WITNESS_STATE_PINNED),
        (
            "TC_WITNESS_STATE_REFUSING_UNPINNED",
            TC_WITNESS_STATE_REFUSING_UNPINNED,
        ),
        (
            "TC_WITNESS_STATE_REFUSING_PIN_MALFORMED",
            TC_WITNESS_STATE_REFUSING_PIN_MALFORMED,
        ),
        (
            "TC_WITNESS_STATE_REFUSING_INFERENCE_RECEIPTS_MISSING",
            TC_WITNESS_STATE_REFUSING_INFERENCE_RECEIPTS_MISSING,
        ),
        (
            "TC_WITNESS_STATE_NOT_ENROLLED",
            TC_WITNESS_STATE_NOT_ENROLLED,
        ),
        ("TC_WITNESS_STATE_UNREADABLE", TC_WITNESS_STATE_UNREADABLE),
    ];

    let header = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/include/trace_commons.h"
    ))
    .unwrap();

    let mut found = 0usize;
    for (name, value) in expected {
        let mut hit = None;
        for line in header.lines() {
            let Some(rest) = line.strip_prefix("#define ") else {
                continue;
            };
            let mut parts = rest.split_whitespace();
            if parts.next() == Some(name) {
                hit = parts.next().map(|v| v.parse::<i32>().unwrap());
                break;
            }
        }
        let hit = hit.unwrap_or_else(|| panic!("{name} is not #defined in the header"));
        assert_eq!(
            hit, value,
            "{name} disagrees between the header and the ABI"
        );
        found += 1;
    }
    assert_eq!(
        found, 7,
        "the header scan matched nothing and passed anyway"
    );
}

/// The bug this whole surface exists to prevent, asserted directly.
#[test]
fn no_witness_and_an_unpinned_witness_are_different_states() {
    let absent = tempfile::tempdir().unwrap();
    write_enrolled_config(absent.path(), None);
    let unpinned = tempfile::tempdir().unwrap();
    write_enrolled_config(unpinned.path(), Some(witness_settings(vec![])));

    let absent_path = cstr(absent.path());
    let unpinned_path = cstr(unpinned.path());
    let absent_state = unsafe { tc_witness_trust_state(absent_path.as_ptr()) };
    let unpinned_state = unsafe { tc_witness_trust_state(unpinned_path.as_ptr()) };

    assert_eq!(absent_state, TC_WITNESS_STATE_ABSENT);
    assert_eq!(unpinned_state, TC_WITNESS_STATE_REFUSING_UNPINNED);
    assert_ne!(
        absent_state, unpinned_state,
        "a client with no witness redacts locally and works; a client with an \
         unpinned witness refuses every submission. Rendering them the same shows \
         'all fine' through a total upload outage."
    );

    // And the JSON says so too, without a shell having to infer it from the
    // url field being present.
    let absent_json = witness_status_json(absent.path());
    let unpinned_json = witness_status_json(unpinned.path());
    assert_eq!(absent_json["state"], serde_json::json!("absent"));
    assert_eq!(absent_json["refusal"], serde_json::Value::Null);
    assert_eq!(
        unpinned_json["state"],
        serde_json::json!("refusing_unpinned")
    );
    assert_eq!(
        unpinned_json["refusal"],
        serde_json::json!("witness_expected_measurement")
    );
}

#[test]
fn a_malformed_pin_is_not_the_unpinned_state() {
    let dir = tempfile::tempdir().unwrap();
    write_enrolled_config(
        dir.path(),
        Some(witness_settings(vec!["mrtd=not-hex".into()])),
    );
    let path = cstr(dir.path());
    assert_eq!(
        unsafe { tc_witness_trust_state(path.as_ptr()) },
        TC_WITNESS_STATE_REFUSING_PIN_MALFORMED
    );
    let json = witness_status_json(dir.path());
    assert_eq!(json["state"], serde_json::json!("refusing_pin_malformed"));
    assert_eq!(
        json["pinned_measurement_count"],
        serde_json::json!(1),
        "the contributor wrote one pin; reporting zero would read as the unpinned refusal"
    );
}

#[test]
fn an_unenrolled_and_an_unreadable_directory_are_neither_absent() {
    let empty = tempfile::tempdir().unwrap();
    let empty_path = cstr(empty.path());
    assert_eq!(
        unsafe { tc_witness_trust_state(empty_path.as_ptr()) },
        TC_WITNESS_STATE_NOT_ENROLLED
    );
    assert_eq!(last_error().as_deref(), Some("witness-not-enrolled"));

    let broken = tempfile::tempdir().unwrap();
    std::fs::write(broken.path().join("contributor.json"), "{ not json").unwrap();
    let broken_path = cstr(broken.path());
    let state = unsafe { tc_witness_trust_state(broken_path.as_ptr()) };
    assert_eq!(state, TC_WITNESS_STATE_UNREADABLE);
    assert_ne!(
        state, TC_WITNESS_STATE_ABSENT,
        "a config that cannot be read is a client whose behaviour is unknown, \
         not a client redacting locally"
    );
}

#[test]
fn configuring_a_witness_round_trips_and_clearing_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    write_enrolled_config(dir.path(), None);
    let path = cstr(dir.path());
    let url = cstr_str("https://witness.example");
    let address = cstr_str("0xfeed");
    let pins = cstr_str(&serde_json::to_string(&[a_pin()]).unwrap());

    let mut err: *mut c_char = std::ptr::null_mut();
    let rc = unsafe {
        tc_witness_configure(
            path.as_ptr(),
            url.as_ptr(),
            address.as_ptr(),
            pins.as_ptr(),
            &mut err,
        )
    };
    assert_eq!(rc, 0, "configure failed: {:?}", last_error());
    assert!(err.is_null());

    assert_eq!(
        unsafe { tc_witness_trust_state(path.as_ptr()) },
        TC_WITNESS_STATE_PINNED
    );
    let json = witness_status_json(dir.path());
    assert_eq!(json["state"], serde_json::json!("pinned"));
    assert_eq!(
        json["state_code"],
        serde_json::json!(TC_WITNESS_STATE_PINNED)
    );
    assert_eq!(json["url"], serde_json::json!("https://witness.example"));
    assert_eq!(json["signing_address"], serde_json::json!("0xfeed"));
    assert_eq!(json["pinned_measurement_count"], serde_json::json!(1));

    // Clearing is 1 the first time and 0 the second: idempotent, and the
    // return distinguishes "removed one" from "there was none".
    let mut err: *mut c_char = std::ptr::null_mut();
    assert_eq!(unsafe { tc_witness_clear(path.as_ptr(), &mut err) }, 1);
    assert_eq!(unsafe { tc_witness_clear(path.as_ptr(), &mut err) }, 0);
    assert!(err.is_null());
    assert_eq!(
        unsafe { tc_witness_trust_state(path.as_ptr()) },
        TC_WITNESS_STATE_ABSENT
    );
}

/// The ABI refuses to create the refusing state it can report.
#[test]
fn configure_will_not_write_a_witness_that_refuses_everything() {
    let dir = tempfile::tempdir().unwrap();
    write_enrolled_config(dir.path(), None);
    let path = cstr(dir.path());
    let url = cstr_str("https://witness.example");
    let address = cstr_str("0xfeed");

    for (pins, expected) in [
        ("[]", "witness-pin-required"),
        (r#"["mrtd=not-hex"]"#, "witness-pin-malformed"),
        (r#"["   "]"#, "witness-pin-required"),
        ("not json", "witness-pins-invalid-json"),
        (r#"{"a":1}"#, "witness-pins-invalid-json"),
    ] {
        let pins_c = cstr_str(pins);
        let mut err: *mut c_char = std::ptr::null_mut();
        let rc = unsafe {
            tc_witness_configure(
                path.as_ptr(),
                url.as_ptr(),
                address.as_ptr(),
                pins_c.as_ptr(),
                &mut err,
            )
        };
        assert_eq!(rc, -1, "{pins} was accepted");
        assert!(!err.is_null(), "{pins} set no error");
        let label = unsafe { CStr::from_ptr(err) }.to_str().unwrap().to_string();
        unsafe { tc_string_free(err) };
        assert_eq!(label, expected, "wrong label for {pins}");
        assert_eq!(
            unsafe { tc_witness_trust_state(path.as_ptr()) },
            TC_WITNESS_STATE_ABSENT,
            "{pins} was rejected but still changed the config"
        );
    }
}

#[test]
fn configure_rejects_a_url_without_a_scheme_and_an_empty_signer() {
    let dir = tempfile::tempdir().unwrap();
    write_enrolled_config(dir.path(), None);
    let path = cstr(dir.path());
    let pins = cstr_str(&serde_json::to_string(&[a_pin()]).unwrap());

    for (url, address, expected) in [
        ("witness.example", "0xfeed", "witness-url-invalid"),
        ("https://", "0xfeed", "witness-url-invalid"),
        ("file:///etc/passwd", "0xfeed", "witness-url-invalid"),
        (
            "https://witness.example",
            "   ",
            "witness-signing-address-invalid",
        ),
    ] {
        let url_c = cstr_str(url);
        let address_c = cstr_str(address);
        let mut err: *mut c_char = std::ptr::null_mut();
        let rc = unsafe {
            tc_witness_configure(
                path.as_ptr(),
                url_c.as_ptr(),
                address_c.as_ptr(),
                pins.as_ptr(),
                &mut err,
            )
        };
        assert_eq!(rc, -1, "{url} / {address:?} was accepted");
        let label = unsafe { CStr::from_ptr(err) }.to_str().unwrap().to_string();
        unsafe { tc_string_free(err) };
        assert_eq!(label, expected);
    }
}

#[test]
fn witness_calls_refuse_null_and_unenrolled_without_crashing() {
    assert_eq!(
        unsafe { tc_witness_trust_state(std::ptr::null()) },
        TC_WITNESS_STATE_UNREADABLE
    );

    let mut err: *mut c_char = std::ptr::null_mut();
    assert!(unsafe { tc_witness_status_json(std::ptr::null(), &mut err) }.is_null());
    assert!(!err.is_null());
    unsafe { tc_string_free(err) };

    // Not enrolled: configure and clear both refuse by name rather than
    // creating a config file out of nowhere.
    let dir = tempfile::tempdir().unwrap();
    let path = cstr(dir.path());
    let url = cstr_str("https://witness.example");
    let address = cstr_str("0xfeed");
    let pins = cstr_str(&serde_json::to_string(&[a_pin()]).unwrap());
    let mut err: *mut c_char = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            tc_witness_configure(
                path.as_ptr(),
                url.as_ptr(),
                address.as_ptr(),
                pins.as_ptr(),
                &mut err,
            )
        },
        -1
    );
    unsafe { tc_string_free(err) };
    let mut err: *mut c_char = std::ptr::null_mut();
    assert_eq!(unsafe { tc_witness_clear(path.as_ptr(), &mut err) }, -1);
    unsafe { tc_string_free(err) };

    // A NULL err out-param is not a crash on any of them.
    assert!(unsafe { tc_witness_status_json(path.as_ptr(), std::ptr::null_mut()) }.is_null());
    assert_eq!(
        unsafe { tc_witness_clear(path.as_ptr(), std::ptr::null_mut()) },
        -1
    );
    assert_eq!(
        unsafe {
            tc_witness_configure(
                path.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        },
        -1
    );
}

/// A shell that has just started must not claim anything about a submission
/// it did not see, and every key must be present so it never has to guess.
#[test]
fn the_last_result_is_total_and_starts_unobserved() {
    let json = last_result_json();
    for key in [
        "outcome",
        "certificate_obtained",
        "certificate_verified",
        "refusal",
        "n_of_m",
    ] {
        assert!(json.get(key).is_some(), "{key} missing from {json}");
    }
    // This test binary makes no submission, so the only two values that can
    // appear here are "not_observed" and -- if another test in this binary
    // ever starts submitting -- one of the observed ones. Asserting the
    // shape rather than pinning the value keeps this from becoming an
    // order-dependent failure, while still proving the outcome is one of the
    // four the header documents.
    let outcome = json["outcome"].as_str().unwrap();
    assert!(
        ["not_observed", "local_redaction", "certified", "refused"].contains(&outcome),
        "undocumented outcome {outcome}"
    );
    assert_eq!(
        outcome, "not_observed",
        "no submission has been made in this process"
    );
    assert_eq!(json["certificate_obtained"], serde_json::json!(false));
    assert_eq!(json["n_of_m"], serde_json::Value::Null);
}

/// Read every `#define NAME <int>` in this crate's header copy.
fn header_defines() -> std::collections::BTreeMap<String, i32> {
    let header = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/include/trace_commons.h"
    ))
    .unwrap();
    let mut out = std::collections::BTreeMap::new();
    for line in header.lines() {
        let Some(rest) = line.strip_prefix("#define ") else {
            continue;
        };
        let mut parts = rest.split_whitespace();
        let (Some(name), Some(value)) = (parts.next(), parts.next()) else {
            continue;
        };
        if let Ok(value) = value.parse::<i32>() {
            out.insert(name.to_string(), value);
        }
    }
    out
}

/// The witness tone values must share nothing with the routing tone values.
///
/// The routing tone has no refused case and its consumers map anything they
/// do not recognise to neutral, so an overlapping witness numbering would
/// let a cross-wired shell render "nothing is being sent" as "nothing to
/// say". Parsed out of the header rather than restated, so it cannot pass by
/// agreeing with itself.
#[test]
fn the_witness_tones_never_collide_with_the_routing_tones() {
    let defines = header_defines();
    let witness: Vec<(&String, &i32)> = defines
        .iter()
        .filter(|(name, _)| name.starts_with("TC_WITNESS_TONE_"))
        .collect();
    let routing: Vec<(&String, &i32)> = defines
        .iter()
        .filter(|(name, _)| name.starts_with("TC_ROUTING_TONE_"))
        .collect();
    assert_eq!(witness.len(), 5, "the witness tone scan found nothing");
    assert_eq!(routing.len(), 4, "the routing tone scan found nothing");

    for (wname, wvalue) in &witness {
        for (rname, rvalue) in &routing {
            assert_ne!(
                wvalue, rvalue,
                "{wname} and {rname} share a value, so a shell that cross-wires the two \
                 tone mappers renders a refusal as something else"
            );
        }
    }

    // And the header's numbers are the library's numbers.
    assert_eq!(defines["TC_WITNESS_TONE_NEUTRAL"], TC_WITNESS_TONE_NEUTRAL);
    assert_eq!(defines["TC_WITNESS_TONE_HELD"], TC_WITNESS_TONE_HELD);
    assert_eq!(defines["TC_WITNESS_TONE_CLEAR"], TC_WITNESS_TONE_CLEAR);
    assert_eq!(
        defines["TC_WITNESS_TONE_ATTENTION"],
        TC_WITNESS_TONE_ATTENTION
    );
    assert_eq!(defines["TC_WITNESS_TONE_REFUSED"], TC_WITNESS_TONE_REFUSED);
}

/// A refusing state must cross the ABI as a refusal, not as attention.
#[test]
fn every_refusing_state_crosses_the_abi_as_refused() {
    for (code, refusing) in [
        (TC_WITNESS_STATE_ABSENT, false),
        (TC_WITNESS_STATE_PINNED, false),
        (TC_WITNESS_STATE_REFUSING_UNPINNED, true),
        (TC_WITNESS_STATE_REFUSING_PIN_MALFORMED, true),
        (TC_WITNESS_STATE_REFUSING_INFERENCE_RECEIPTS_MISSING, true),
        (TC_WITNESS_STATE_NOT_ENROLLED, false),
        (TC_WITNESS_STATE_UNREADABLE, true),
    ] {
        let tone = tc_witness_state_tone(code);
        if refusing {
            assert_eq!(
                tone, TC_WITNESS_TONE_REFUSED,
                "state {code} sends nothing at all and must not be painted as attention"
            );
        } else {
            assert_ne!(
                tone, TC_WITNESS_TONE_REFUSED,
                "state {code} is not a refusal"
            );
        }
    }
    assert_eq!(
        tc_witness_state_tone(TC_WITNESS_STATE_ABSENT),
        TC_WITNESS_TONE_NEUTRAL
    );
    assert_eq!(
        tc_witness_state_tone(TC_WITNESS_STATE_PINNED),
        TC_WITNESS_TONE_CLEAR
    );
}

/// A state this build cannot name fails closed.
#[test]
fn an_unnameable_state_has_no_sentence_and_a_refused_tone() {
    for code in [5, 99, -3, i32::MIN, i32::MAX] {
        let line = tc_witness_state_line(code);
        assert!(
            line.is_null(),
            "state {code} was given a sentence this build cannot have"
        );
        assert_eq!(last_error().as_deref(), Some("witness-state-unknown"));
        assert_eq!(
            tc_witness_state_tone(code),
            TC_WITNESS_TONE_REFUSED,
            "an unnameable state painted as anything but refused reads as 'all is well' \
             on a surface about whether sessions leave the machine"
        );
    }
}

/// Every state a shell can be handed has its own sentence, from the core.
#[test]
fn every_state_crosses_the_abi_with_its_own_sentence() {
    let mut seen: Vec<String> = Vec::new();
    for code in [
        TC_WITNESS_STATE_ABSENT,
        TC_WITNESS_STATE_PINNED,
        TC_WITNESS_STATE_REFUSING_UNPINNED,
        TC_WITNESS_STATE_REFUSING_PIN_MALFORMED,
        TC_WITNESS_STATE_REFUSING_INFERENCE_RECEIPTS_MISSING,
        TC_WITNESS_STATE_NOT_ENROLLED,
        TC_WITNESS_STATE_UNREADABLE,
    ] {
        let raw = tc_witness_state_line(code);
        assert!(!raw.is_null(), "state {code} has no sentence");
        let line = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_string();
        unsafe { tc_string_free(raw) };
        assert!(!line.is_empty());
        assert!(
            !seen.contains(&line),
            "state {code} reuses another sentence"
        );
        // The two states a shell would otherwise conflate must not read
        // alike, and only the refusal announces the outage.
        if code == TC_WITNESS_STATE_REFUSING_UNPINNED {
            assert!(line.starts_with("Nothing is being sent."), "{line}");
        }
        if code == TC_WITNESS_STATE_ABSENT {
            assert!(!line.contains("Nothing is being sent"), "{line}");
        }
        seen.push(line);
    }
}

/// The words come from the core, and a shell is handed all of them at once.
#[test]
fn the_witness_copy_call_carries_the_whole_card() {
    let raw = tc_witness_copy();
    assert!(!raw.is_null());
    let text = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_string();
    unsafe { tc_string_free(raw) };
    let json: serde_json::Value = serde_json::from_str(&text).unwrap();
    let object = json.as_object().unwrap();
    for key in [
        "heading",
        "intro",
        "certificate_means",
        "measurements_note",
        "url_title",
        "signing_address_title",
        "measurements_title",
        "configure",
        "clear",
        "clear_note",
        "applies_at_once",
        "inference_heading",
        "inference_disclosure",
        "inference_capture_note",
        "inference_scope_note",
        "inference_enable",
        "inference_disable",
        "inference_confirm",
        "inference_cancel",
        "inference_enabled",
        "inference_disabled",
        "inference_save_failed",
    ] {
        let value = object
            .get(key)
            .unwrap_or_else(|| panic!("{key} is missing, so a shell writes it itself"));
        assert!(!value.as_str().unwrap().is_empty(), "{key} is empty");
    }
    // The one sentence that must be here verbatim: everything else on this
    // card is wording, but this is the limit of what a certificate claims.
    assert!(
        object["certificate_means"]
            .as_str()
            .unwrap()
            .contains("not a statement that a session is clean")
    );
}

/// The prose form of the last result is a sentence, not a label, and never
/// the word a certificate does not earn.
#[test]
fn the_last_result_line_is_prose_and_never_says_attested() {
    let raw = tc_witness_last_result_line();
    assert!(!raw.is_null());
    let line = unsafe { CStr::from_ptr(raw) }.to_str().unwrap().to_string();
    unsafe { tc_string_free(raw) };
    assert!(!line.is_empty());
    let lowered = line.to_lowercase();
    assert!(!lowered.contains("attested"));
    assert!(
        !lowered.contains("_"),
        "an operator label is not wording: {line}"
    );
    // No submission has been made in this process, so the tone is the
    // waiting one and emphatically not the reassuring one.
    assert_eq!(tc_witness_last_result_tone(), TC_WITNESS_TONE_HELD);
    assert_ne!(tc_witness_last_result_tone(), TC_WITNESS_TONE_CLEAR);
}

/// The pinned entries must go back through `tc_witness_configure` unchanged.
///
/// This is the whole point of returning them: a shell pre-fills its editor
/// from `pinned_measurements` and hands that array straight back. If the
/// round trip were not exact, a contributor who opened the settings screen
/// and pressed save without touching the box would have silently rewritten
/// their own pins.
#[test]
fn the_pinned_measurements_round_trip_through_configure_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    write_enrolled_config(dir.path(), None);
    let path = cstr(dir.path());
    let url = cstr_str("https://witness.example");
    let address = cstr_str("0xfeed");

    let original = vec![
        format!("mrtd={},mrconfigid={}", "ab".repeat(48), "cd".repeat(48)),
        format!("mrtd={}", "ef".repeat(48)),
    ];
    let pins = cstr_str(&serde_json::to_string(&original).unwrap());
    let mut err: *mut c_char = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            tc_witness_configure(
                path.as_ptr(),
                url.as_ptr(),
                address.as_ptr(),
                pins.as_ptr(),
                &mut err,
            )
        },
        0,
        "configure failed: {:?}",
        last_error()
    );

    let first = witness_status_json(dir.path());
    let read_back: Vec<String> =
        serde_json::from_value(first["pinned_measurements"].clone()).unwrap();
    assert_eq!(
        read_back, original,
        "what came back is not what was stored, so an editor pre-filled from it \
         would save something else"
    );

    // Hand exactly what was read straight back, the way a shell will.
    let again = cstr_str(&serde_json::to_string(&read_back).unwrap());
    let mut err: *mut c_char = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            tc_witness_configure(
                path.as_ptr(),
                url.as_ptr(),
                address.as_ptr(),
                again.as_ptr(),
                &mut err,
            )
        },
        0,
        "the entries this ABI returned were rejected by the call that takes them"
    );
    let second = witness_status_json(dir.path());
    assert_eq!(
        first, second,
        "a save that changed nothing changed the stored configuration"
    );
}

/// The count and the list are one answer, and the list is what the editor
/// shows.
#[test]
fn the_status_payload_carries_the_entries_and_a_sentence_for_the_count() {
    let dir = tempfile::tempdir().unwrap();
    let pin = format!("mrtd={}", "ab".repeat(48));
    write_enrolled_config(
        dir.path(),
        Some(witness_settings(vec![pin.clone(), pin.clone()])),
    );
    let json = witness_status_json(dir.path());
    assert_eq!(
        json["pinned_measurements"],
        serde_json::json!([pin, pin]),
        "the editor has nothing to pre-fill from"
    );
    assert_eq!(
        json["pinned_measurements"].as_array().unwrap().len(),
        json["pinned_measurement_count"].as_u64().unwrap() as usize,
        "the count and the list are two different answers"
    );
    assert_eq!(
        json["pinned_measurement_line"],
        serde_json::json!("2 measurements are pinned."),
        "a bare numeral makes a shell write the words itself"
    );

    // No witness: an empty list, and no sentence to print about a count of
    // pins on something that does not exist.
    let absent = tempfile::tempdir().unwrap();
    write_enrolled_config(absent.path(), None);
    let json = witness_status_json(absent.path());
    assert_eq!(json["pinned_measurements"], serde_json::json!([]));
    assert_eq!(json["pinned_measurement_line"], serde_json::Value::Null);
}

/// A stored entry this build cannot parse is still shown, so it can be fixed.
#[test]
fn a_malformed_pin_is_readable_so_a_contributor_can_repair_it() {
    let dir = tempfile::tempdir().unwrap();
    write_enrolled_config(
        dir.path(),
        Some(witness_settings(vec!["mrtd=not-hex".into()])),
    );
    let json = witness_status_json(dir.path());
    assert_eq!(json["state"], serde_json::json!("refusing_pin_malformed"));
    assert_eq!(
        json["pinned_measurements"],
        serde_json::json!(["mrtd=not-hex"]),
        "the unreadable entry is the one a contributor most needs to see; hiding it \
         deletes their work on the next save"
    );
    assert_eq!(
        json["pinned_measurement_line"],
        serde_json::json!("One measurement is pinned.")
    );

    // And handing that entry back is still refused: the read is permissive,
    // the write is not.
    let path = cstr(dir.path());
    let url = cstr_str("https://witness.example");
    let address = cstr_str("0xfeed");
    let pins = cstr_str(r#"["mrtd=not-hex"]"#);
    let mut err: *mut c_char = std::ptr::null_mut();
    assert_eq!(
        unsafe {
            tc_witness_configure(
                path.as_ptr(),
                url.as_ptr(),
                address.as_ptr(),
                pins.as_ptr(),
                &mut err,
            )
        },
        -1
    );
    let label = unsafe { CStr::from_ptr(err) }.to_str().unwrap().to_string();
    unsafe { tc_string_free(err) };
    assert_eq!(label, "witness-pin-malformed");
}

#[test]
fn private_inference_quit_notice_tracks_actual_owned_work() {
    let call = |on, state: &str| {
        let state = cstr_str(state);
        unsafe { tc_private_inference_quit_needs_notice(on, state.as_ptr()) }
    };
    assert_eq!(call(0, "stopping"), 1);
    assert_eq!(call(1, "off"), 0);
    assert_eq!(call(1, "running_elsewhere"), 0);
    assert_eq!(call(0, "running_no_backends"), 1);
    assert_eq!(call(1, "future"), 1);
    assert_eq!(
        unsafe { tc_private_inference_quit_needs_notice(0, std::ptr::null()) },
        0
    );
}

#[test]
fn the_consent_bundle_crossing_the_abi_is_the_one_in_the_rust() {
    // The whole point of the slice: the sentences a shell renders at the
    // moment of consent are the ones this repo defines, not a transcription
    // in Swift or C# that stops matching the day one of them changes.
    let json = take_owned(tc_consent_copy());
    let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    // Compared against the payload itself, not against words written here:
    // pinning the literals in this test would be the same transcription bug
    // one layer down. The shells' own suites pin nothing either -- they
    // assert the field set, which is what a rename must break.
    let expected = serde_json::to_value(trace_commons_contributor::consent_copy::consent_copy())
        .expect("the payload serialises");
    assert_eq!(
        parsed, expected,
        "the ABI must hand over the payload unchanged"
    );
}

#[test]
fn the_gate_help_branch_crosses_the_abi() {
    use trace_commons_contributor::consent_copy as copy;
    assert_eq!(take_owned(tc_consent_gate_help(1)), copy::GATE_READY_HELP);
    assert_eq!(
        take_owned(tc_consent_gate_help(0)),
        copy::GATE_NOT_PINNED_HELP
    );
}

#[test]
fn an_unknown_pinned_value_is_not_told_the_button_is_armed() {
    // Fail-closed, and specific to this surface: routing's unknown values
    // answer Neutral because Neutral claims nothing, but there is no
    // sentence here that claims nothing. The one that claims less is the
    // one an unknown value gets.
    use trace_commons_contributor::consent_copy as copy;
    for value in [-1, 2, 7, i32::MIN, i32::MAX] {
        assert_eq!(
            take_owned(tc_consent_gate_help(value)),
            copy::GATE_NOT_PINNED_HELP,
            "{value} must not arm the button"
        );
    }
}

// --- the private-inference surface ---------------------------------------

/// Every sentence the offer needs arrives in one payload, finished.
///
/// The field set is compared against the Rust's own serialisation rather
/// than a list kept here, so a field the Rust grows and this test forgot
/// cannot pass by being absent from both.
#[test]
fn the_private_inference_payload_crosses_whole_and_finished() {
    let json = take_owned(tc_private_inference_copy());
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");

    let expected = serde_json::to_value(
        trace_commons_contributor::private_inference_copy::private_inference_copy(),
    )
    .expect("the Rust payload serialises");
    assert_eq!(value, expected, "the export is not the shared payload");

    let fields = value.as_object().expect("an object");
    for (field, value) in fields {
        let text = value.as_str().expect("every field is a string");
        assert!(!text.trim().is_empty(), "{field} arrived empty");
        for marker in ["{}", "{port}", "%@", "%s", "%d"] {
            assert!(
                !text.contains(marker),
                "{field} crossed as a template: {text}"
            );
        }
    }

    // The sentence this whole surface exists to print. Named here rather
    // than asserted as "some field mentions accounts": a shell that dropped
    // it would still render an offer, and the offer would be a lie by
    // omission.
    let exposure = fields["offer_exposure"].as_str().unwrap();
    assert!(exposure.contains("anything else running"), "{exposure}");
    assert!(exposure.contains("accounts"), "{exposure}");
}

#[test]
fn the_public_run_payload_and_error_tables_cross_whole_and_finished() {
    let json = take_owned(tc_public_run_copy());
    let value: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    let expected = serde_json::to_value(trace_commons_contributor::public_run::public_run_copy())
        .expect("the Rust payload serialises");
    assert_eq!(value, expected, "the export is not the shared payload");

    let fields = value.as_object().expect("an object");
    for (field, value) in fields {
        if field == "reuse_permissions" {
            let choices = value.as_array().expect("reuse permissions are a list");
            assert_eq!(choices.len(), 2);
            continue;
        }
        let text = value.as_str().expect("every copy field is a string");
        assert!(!text.trim().is_empty(), "{field} arrived empty");
        for marker in ["{}", "%@", "%s", "%d"] {
            assert!(
                !text.contains(marker),
                "{field} crossed as a template: {text}"
            );
        }
    }

    let session_label = CString::new("session-detail-not-found").expect("c string");
    let publication_label = CString::new("public-run-conflict").expect("c string");
    assert_eq!(
        take_owned(unsafe { tc_session_detail_error_line(session_label.as_ptr()) }),
        trace_commons_contributor::public_run::session_detail_error_line(
            "session-detail-not-found"
        )
    );
    assert_eq!(
        take_owned(unsafe { tc_public_run_error_line(publication_label.as_ptr()) }),
        trace_commons_contributor::public_run::publication_error_line("public-run-conflict")
    );

    let editor = CString::new(
        serde_json::json!({
            "title": "  Repair a stalled upload  ",
            "outcome_summary": "The upload completed.",
            "correction_excerpt": null,
            "workflow": "Renew the session, then retry once.",
            "reuse_permission": "cc_by_4_0",
            "evidence": [{"event_id": "00000000-0000-0000-0000-000000000000", "excerpt": "The retry succeeded."}],
            "source": "https://tracecommons.ai/runs/run-source-workflow"
        })
        .to_string(),
    )
    .expect("editor input");
    let validation = take_owned(unsafe { tc_public_run_validate_editor(editor.as_ptr()) });
    let validation: serde_json::Value = serde_json::from_str(&validation).expect("validation JSON");
    assert!(validation.get("error").is_none());
    assert_eq!(validation["draft"]["title"], "Repair a stalled upload");
    assert_eq!(validation["draft"]["source_slug"], "run-source-workflow");
}

#[test]
fn the_public_run_validator_rejects_null_malformed_and_oversized_inputs() {
    let parse = |value: String| {
        serde_json::from_str::<serde_json::Value>(&value).expect("validator response is JSON")
    };

    let null_result = parse(take_owned(unsafe {
        tc_public_run_validate_editor(std::ptr::null())
    }));
    assert!(null_result.get("draft").is_none());
    assert!(null_result["error"].as_str().is_some());

    for bytes in [b"{bad".as_slice(), [0xff_u8].as_slice()] {
        let input = CString::new(bytes).expect("synthetic invalid input");
        let result = parse(take_owned(unsafe {
            tc_public_run_validate_editor(input.as_ptr())
        }));
        assert!(result.get("draft").is_none());
        assert!(result["error"].as_str().is_some());
    }

    let oversized_title = "t".repeat(101);
    let input = CString::new(
        serde_json::json!({
            "title": oversized_title,
            "outcome_summary": "The synthetic task completed.",
            "correction_excerpt": null,
            "workflow": "Apply the bounded synthetic steps.",
            "reuse_permission": "cc_by_4_0",
            "evidence": [{"event_id": "00000000-0000-0000-0000-000000000000", "excerpt": "The synthetic result was observed."}],
            "source": ""
        })
        .to_string(),
    )
    .expect("oversized editor input");
    let raw_result = take_owned(unsafe { tc_public_run_validate_editor(input.as_ptr()) });
    let result = parse(raw_result.clone());
    assert!(result.get("draft").is_none());
    assert!(result["error"].as_str().is_some());
    assert!(!raw_result.contains(&oversized_title));
}

/// The state sentence and its tone are one decision, and they cross as one.
///
/// Asserted against the Rust's own two functions, so this proves the exports
/// ARE the shared tables rather than a second copy that happens to agree.
#[test]
fn the_private_inference_branch_tables_cross_the_abi() {
    use trace_commons_contributor::private_inference_copy as copy;
    let line = |state: &str| {
        let state = cstr_str(state);
        take_owned(unsafe { tc_private_inference_state_line(state.as_ptr()) })
    };
    let tone = |state: &str| {
        let state = cstr_str(state);
        unsafe { tc_private_inference_state_tone(state.as_ptr()) }
    };

    for state in [
        "off",
        "stopping",
        "running",
        "running_no_backends",
        "running_elsewhere",
        "port_in_use",
        "start_failed",
        "crashed",
        "",
        "RUNNING",
        "a_state_from_a_later_daemon",
    ] {
        assert_eq!(line(state), copy::state_line(state), "{state:?}");
        let expected = match copy::state_tone(state) {
            copy::PrivateInferenceTone::Neutral => TC_PRIVATE_INFERENCE_TONE_NEUTRAL,
            copy::PrivateInferenceTone::Held => TC_PRIVATE_INFERENCE_TONE_HELD,
            copy::PrivateInferenceTone::Clear => TC_PRIVATE_INFERENCE_TONE_CLEAR,
            copy::PrivateInferenceTone::Attention => TC_PRIVATE_INFERENCE_TONE_ATTENTION,
            copy::PrivateInferenceTone::Refused => TC_PRIVATE_INFERENCE_TONE_REFUSED,
        };
        assert_eq!(tone(state), expected, "{state:?}");
    }

    // Exactly one state may be painted as working, and it is not the one
    // with nowhere to send a call.
    assert_eq!(tone("running"), TC_PRIVATE_INFERENCE_TONE_CLEAR);
    assert_ne!(
        tone("running_no_backends"),
        TC_PRIVATE_INFERENCE_TONE_CLEAR,
        "a listener with nowhere to send is not a working light"
    );
    for failure in ["port_in_use", "start_failed", "crashed"] {
        assert_eq!(
            tone(failure),
            TC_PRIVATE_INFERENCE_TONE_REFUSED,
            "{failure}"
        );
        assert!(line(failure).contains("off and on again"), "{failure}");
    }

    // A state this build has never heard of, and no pointer at all, claim
    // nothing rather than falling through to the working light.
    assert_eq!(
        tone("a_state_from_a_later_daemon"),
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL
    );
    assert_eq!(
        unsafe { tc_private_inference_state_tone(std::ptr::null()) },
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL
    );
    assert_eq!(
        take_owned(unsafe { tc_private_inference_state_line(std::ptr::null()) }),
        copy::STATE_UNREPORTED
    );
}

/// The credential row crosses whole: the sentence, the tone and the button.
///
/// Pinned against the Rust tables rather than against literals, so this
/// cannot become a second place any of the three is decided.
#[test]
fn the_credential_state_crosses_with_its_tone_and_its_action() {
    use trace_commons_contributor::private_inference_copy as copy;
    let line = |state: &str| {
        let state = cstr_str(state);
        take_owned(unsafe { tc_near_ai_credential_state_line(state.as_ptr()) })
    };
    let action = |state: &str| {
        let state = cstr_str(state);
        unsafe { tc_near_ai_credential_action(state.as_ptr()) }
    };
    let tone = |state: &str| {
        let state = cstr_str(state);
        unsafe { tc_near_ai_credential_state_tone(state.as_ptr()) }
    };

    for state in [
        "absent",
        "obtaining",
        "failed",
        "cancelled",
        "present",
        "",
        "PRESENT",
        "a_state_from_a_later_daemon",
    ] {
        assert_eq!(line(state), copy::credential_state_line(state), "{state:?}");
        let expected = match copy::credential_action(state) {
            copy::CredentialAction::None => TC_CREDENTIAL_ACTION_NONE,
            copy::CredentialAction::Obtain => TC_CREDENTIAL_ACTION_OBTAIN,
            copy::CredentialAction::Cancel => TC_CREDENTIAL_ACTION_CANCEL,
            copy::CredentialAction::Forget => TC_CREDENTIAL_ACTION_FORGET,
        };
        assert_eq!(action(state), expected, "{state:?}");
    }

    // A key that is here is the only settled state, and the only one whose
    // button removes rather than mints.
    assert_eq!(tone("present"), TC_PRIVATE_INFERENCE_TONE_CLEAR);
    assert_eq!(action("present"), TC_CREDENTIAL_ACTION_FORGET);
    assert_eq!(tone("obtaining"), TC_PRIVATE_INFERENCE_TONE_HELD);

    // Nothing read offers the button that opens a browser, and no pointer at
    // all is the same answer.
    for unread in ["", "a_state_from_a_later_daemon"] {
        assert_eq!(action(unread), TC_CREDENTIAL_ACTION_NONE, "{unread:?}");
        assert_ne!(line(unread), copy::CREDENTIAL_ABSENT, "{unread:?}");
        assert_ne!(line(unread), copy::CREDENTIAL_PRESENT, "{unread:?}");
        assert_ne!(tone(unread), TC_PRIVATE_INFERENCE_TONE_CLEAR, "{unread:?}");
    }
    assert_eq!(
        unsafe { tc_near_ai_credential_action(std::ptr::null()) },
        TC_CREDENTIAL_ACTION_NONE
    );
    assert_eq!(
        take_owned(unsafe { tc_near_ai_credential_state_line(std::ptr::null()) }),
        copy::CREDENTIAL_UNREPORTED
    );
    assert_eq!(
        unsafe { tc_near_ai_credential_state_tone(std::ptr::null()) },
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL
    );
}

/// The eligibility row crosses whole: the sentence, the tone, the control and
/// the reason.
///
/// Pinned against the Rust tables rather than against literals, so this
/// cannot become a second place any of the four is decided.
#[test]
fn the_eligibility_state_crosses_with_its_tone_its_control_and_its_reason() {
    use trace_commons_contributor::private_inference_copy as copy;
    let line = |state: &str| {
        let state = cstr_str(state);
        take_owned(unsafe { tc_contribution_eligibility_line(state.as_ptr()) })
    };
    let control = |state: &str| {
        let state = cstr_str(state);
        unsafe { tc_contribution_eligibility_control(state.as_ptr()) }
    };
    let tone = |state: &str| {
        let state = cstr_str(state);
        unsafe { tc_contribution_eligibility_tone(state.as_ptr()) }
    };
    let reason = |label: &str| {
        let label = cstr_str(label);
        take_owned(unsafe { tc_contribution_eligibility_reason_line(label.as_ptr()) })
    };

    for state in [
        "eligible",
        "ineligible_permanent",
        "ineligible_configuration",
        "unknown",
        "",
        "ELIGIBLE",
        "a_state_from_a_later_daemon",
    ] {
        assert_eq!(
            line(state),
            copy::eligibility_state_line(state),
            "{state:?}"
        );
        let expected = match copy::eligibility_control(state) {
            copy::ContributionControl::None => TC_CONTRIBUTION_CONTROL_NONE,
            copy::ContributionControl::Contribute => TC_CONTRIBUTION_CONTROL_CONTRIBUTE,
        };
        assert_eq!(control(state), expected, "{state:?}");
    }

    // Only an eligible session is offered, and only the one state with
    // something to do about it reads as actionable.
    assert_eq!(control("eligible"), TC_CONTRIBUTION_CONTROL_CONTRIBUTE);
    assert_eq!(tone("eligible"), TC_PRIVATE_INFERENCE_TONE_CLEAR);
    assert_eq!(
        tone("ineligible_configuration"),
        TC_PRIVATE_INFERENCE_TONE_ATTENTION
    );
    assert_eq!(
        tone("ineligible_permanent"),
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL
    );

    // Nothing read is offered, borrows no ineligibility, and a null pointer
    // is the same answer.
    for unread in ["", "a_state_from_a_later_daemon"] {
        assert_eq!(control(unread), TC_CONTRIBUTION_CONTROL_NONE, "{unread:?}");
        assert_eq!(line(unread), copy::ELIGIBILITY_UNKNOWN, "{unread:?}");
        assert_ne!(
            line(unread),
            copy::ELIGIBILITY_INELIGIBLE_PERMANENT,
            "{unread:?}"
        );
        assert_ne!(tone(unread), TC_PRIVATE_INFERENCE_TONE_CLEAR, "{unread:?}");
    }
    assert_eq!(
        unsafe { tc_contribution_eligibility_control(std::ptr::null()) },
        TC_CONTRIBUTION_CONTROL_NONE
    );
    assert_eq!(
        take_owned(unsafe { tc_contribution_eligibility_line(std::ptr::null()) }),
        copy::ELIGIBILITY_UNKNOWN
    );
    assert_eq!(
        unsafe { tc_contribution_eligibility_tone(std::ptr::null()) },
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL
    );

    // Every reason the daemon can produce crosses; anything else, including
    // a null pointer, is the empty string a shell renders nothing for.
    for label in trace_commons_contributor::daemon::contribution_eligibility::ALL_REASONS {
        assert_eq!(
            reason(label),
            copy::eligibility_reason_line(label),
            "{label}"
        );
        assert!(!reason(label).is_empty(), "{label}");
    }
    for unknown in ["", "a_reason_from_a_later_daemon"] {
        assert_eq!(reason(unknown), "", "{unknown:?}");
    }
    assert_eq!(
        take_owned(unsafe { tc_contribution_eligibility_reason_line(std::ptr::null()) }),
        ""
    );
}

/// The attestation mark crosses whole: the sentence, the tone and the reason.
///
/// Pinned against the Rust tables rather than against literals, so this
/// cannot become a second place any of the three is decided. **There is no
/// control accessor**, and that is the contract: the mark describes the trace
/// and offers nothing to press. A shell that drew a button from it would be
/// inventing an action out of a description.
#[test]
fn the_attestation_mark_crosses_with_its_tone_and_its_reason() {
    use trace_commons_contributor::private_inference_copy as copy;
    let line = |mark: &str| {
        let mark = cstr_str(mark);
        take_owned(unsafe { tc_contribution_attestation_line(mark.as_ptr()) })
    };
    let tone = |mark: &str| {
        let mark = cstr_str(mark);
        unsafe { tc_contribution_attestation_tone(mark.as_ptr()) }
    };
    let reason = |label: &str| {
        let label = cstr_str(label);
        take_owned(unsafe { tc_contribution_attestation_reason_line(label.as_ptr()) })
    };

    for mark in [
        "attested",
        "unattested_permanent",
        "unattested_configuration",
        "unknown",
        "",
        "ATTESTED",
        "a_mark_from_a_later_daemon",
    ] {
        assert_eq!(line(mark), copy::attestation_state_line(mark), "{mark:?}");
    }

    // The positive case reads as good news, the configuration mark as the one
    // with something to do, and a permanently unattested session is NOT
    // painted as a failure.
    assert_eq!(tone("attested"), TC_PRIVATE_INFERENCE_TONE_CLEAR);
    assert_eq!(
        tone("unattested_configuration"),
        TC_PRIVATE_INFERENCE_TONE_ATTENTION
    );
    for neutral in ["unattested_permanent", "unknown"] {
        assert_eq!(
            tone(neutral),
            TC_PRIVATE_INFERENCE_TONE_NEUTRAL,
            "{neutral:?}"
        );
        assert_ne!(
            tone(neutral),
            TC_PRIVATE_INFERENCE_TONE_REFUSED,
            "{neutral:?}"
        );
    }

    // Nothing read borrows another mark's sentence, and a null pointer is the
    // same answer.
    for unread in ["", "a_mark_from_a_later_daemon"] {
        assert_eq!(line(unread), copy::ATTESTATION_UNKNOWN, "{unread:?}");
        assert_ne!(
            line(unread),
            copy::ATTESTATION_UNATTESTED_PERMANENT,
            "{unread:?}"
        );
        assert_ne!(line(unread), copy::ATTESTATION_ATTESTED, "{unread:?}");
        assert_ne!(tone(unread), TC_PRIVATE_INFERENCE_TONE_CLEAR, "{unread:?}");
    }
    assert_eq!(
        take_owned(unsafe { tc_contribution_attestation_line(std::ptr::null()) }),
        copy::ATTESTATION_UNKNOWN
    );
    assert_eq!(
        unsafe { tc_contribution_attestation_tone(std::ptr::null()) },
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL
    );

    // Every reason the daemon can produce crosses, and none of them arrives
    // as the eligibility surface's wording -- a shell calling the wrong
    // accessor would tell an invited contributor their session cannot be
    // sent.
    for label in trace_commons_contributor::daemon::attestation_mark::ALL_REASONS {
        assert_eq!(
            reason(label),
            copy::attestation_reason_line(label),
            "{label}"
        );
        assert!(!reason(label).is_empty(), "{label}");
        assert!(
            !reason(label).to_lowercase().contains("be sent"),
            "{label} speaks about sending"
        );
    }
    for unknown in ["", "a_reason_from_a_later_daemon"] {
        assert_eq!(reason(unknown), "", "{unknown:?}");
    }
    assert_eq!(
        take_owned(unsafe { tc_contribution_attestation_reason_line(std::ptr::null()) }),
        ""
    );
}

/// The group control crosses, and an absent count is not a zero.
///
/// The trap this pins: a shell passing `0` for an absent `contributable_count`
/// would refuse a submit control to an invited contributor whose sessions are
/// all perfectly sendable.
#[test]
fn a_group_control_tells_an_absent_count_from_a_zero() {
    let control = tc_contribution_group_control;

    // The question applies.
    assert_eq!(control(5, 0), TC_CONTRIBUTION_CONTROL_NONE);
    assert_eq!(control(5, 1), TC_CONTRIBUTION_CONTROL_CONTRIBUTE);
    assert_eq!(control(5, 5), TC_CONTRIBUTION_CONTROL_CONTRIBUTE);
    // Absent, spelled as any negative value.
    for absent in [-1i64, -7, i64::MIN] {
        assert_eq!(
            control(5, absent),
            TC_CONTRIBUTION_CONTROL_CONTRIBUTE,
            "absent must not read as zero ({absent})"
        );
        assert_eq!(control(0, absent), TC_CONTRIBUTION_CONTROL_NONE);
    }
}

/// The withheld-count line crosses, counts, and stays silent at zero.
#[test]
fn the_withheld_line_crosses_and_says_nothing_at_zero() {
    use trace_commons_contributor::private_inference_copy as copy;
    let line = |n: i64| take_owned(tc_contribution_withheld_line(n));

    for n in [0i64, 1, 2, 13, 114] {
        assert_eq!(line(n), copy::group_withheld_line(n as u64), "{n}");
    }
    assert_eq!(line(0), "", "zero explains no gap");
    // A negative count is nobody's honest answer, and it must not become a
    // huge one through a wrapping conversion.
    assert_eq!(line(-1), "", "a negative count says nothing");
    assert!(line(4).contains('4'));
    assert!(!line(4).is_empty());
}

/// The contribution control numbering shares no number with a credential
/// action or a tone. Both blocks have a "nothing" member, and one collision
/// draws a sign-in button on a queue row.
#[test]
fn the_contribution_controls_share_no_number_with_an_action_or_a_tone() {
    for control in [
        TC_CONTRIBUTION_CONTROL_NONE,
        TC_CONTRIBUTION_CONTROL_CONTRIBUTE,
    ] {
        for other in [
            TC_CREDENTIAL_ACTION_NONE,
            TC_CREDENTIAL_ACTION_OBTAIN,
            TC_CREDENTIAL_ACTION_CANCEL,
            TC_CREDENTIAL_ACTION_FORGET,
            TC_PRIVATE_INFERENCE_TONE_NEUTRAL,
            TC_PRIVATE_INFERENCE_TONE_HELD,
            TC_PRIVATE_INFERENCE_TONE_CLEAR,
            TC_PRIVATE_INFERENCE_TONE_ATTENTION,
            TC_PRIVATE_INFERENCE_TONE_REFUSED,
        ] {
            assert_ne!(control, other);
        }
    }
}

/// The action numbering shares no number with any tone that crosses this ABI.
#[test]
fn the_credential_actions_share_no_number_with_a_tone() {
    for action in [
        TC_CREDENTIAL_ACTION_NONE,
        TC_CREDENTIAL_ACTION_OBTAIN,
        TC_CREDENTIAL_ACTION_CANCEL,
        TC_CREDENTIAL_ACTION_FORGET,
    ] {
        for tone in [
            TONE_NEUTRAL,
            TONE_HELD,
            TONE_CLEAR,
            TONE_ATTENTION,
            TC_WITNESS_TONE_NEUTRAL,
            TC_WITNESS_TONE_HELD,
            TC_WITNESS_TONE_CLEAR,
            TC_WITNESS_TONE_ATTENTION,
            TC_WITNESS_TONE_REFUSED,
            TC_PRIVATE_INFERENCE_TONE_NEUTRAL,
            TC_PRIVATE_INFERENCE_TONE_HELD,
            TC_PRIVATE_INFERENCE_TONE_CLEAR,
            TC_PRIVATE_INFERENCE_TONE_ATTENTION,
            TC_PRIVATE_INFERENCE_TONE_REFUSED,
        ] {
            assert_ne!(
                action, tone,
                "an action and a tone sharing a number is a button drawn from a colour"
            );
        }
    }
}

/// The tone numbering is disjoint from the two that already cross this ABI.
///
/// Not a style rule. A shell that fed a private-inference tone to the
/// routing mapper would get NEUTRAL for a refusal, and one that fed it to
/// the witness mapper would get a witness meaning; a disjoint range makes
/// both mistakes wrong for every value rather than only for the dangerous
/// one.
#[test]
fn the_private_inference_tones_share_no_number_with_another_surface() {
    let mine = [
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL,
        TC_PRIVATE_INFERENCE_TONE_HELD,
        TC_PRIVATE_INFERENCE_TONE_CLEAR,
        TC_PRIVATE_INFERENCE_TONE_ATTENTION,
        TC_PRIVATE_INFERENCE_TONE_REFUSED,
    ];
    let others = [
        TONE_NEUTRAL,
        TONE_HELD,
        TONE_CLEAR,
        TONE_ATTENTION,
        TC_WITNESS_TONE_NEUTRAL,
        TC_WITNESS_TONE_HELD,
        TC_WITNESS_TONE_CLEAR,
        TC_WITNESS_TONE_ATTENTION,
        TC_WITNESS_TONE_REFUSED,
    ];
    for value in mine {
        assert!(
            !others.contains(&value),
            "{value} is another surface's tone"
        );
    }
}

/// The port sentence names the bound port, and says nothing when there is
/// none -- including for the 0 a caller passes for JSON `null`.
#[test]
fn the_serving_sentence_names_a_port_or_is_empty() {
    assert!(take_owned(tc_private_inference_serving_line(8463)).contains("8463"));
    assert_eq!(take_owned(tc_private_inference_serving_line(0)), "");
    assert_eq!(take_owned(tc_private_inference_serving_line(-1)), "");
    assert_eq!(take_owned(tc_private_inference_serving_line(70_000)), "");
}

/// When to ask crosses the ABI, so three shells cannot disagree about
/// whether a contributor has already been asked.
#[test]
fn whether_to_offer_crosses_the_abi() {
    assert_eq!(tc_private_inference_should_offer(0, 0), 1, "fresh install");
    assert_eq!(
        tc_private_inference_should_offer(1, 0),
        0,
        "a remembered answer is not re-asked"
    );
    assert_eq!(
        tc_private_inference_should_offer(0, 1),
        0,
        "nobody is offered what they already have"
    );
    assert_eq!(tc_private_inference_should_offer(1, 1), 0);
    // Any non-zero is true, as the header says.
    assert_eq!(tc_private_inference_should_offer(2, 0), 0);
}

#[test]
fn private_inference_write_confirmation_preserves_absent_values_and_rejects_invalid_inputs() {
    use trace_commons_contributor_ffi::tc_private_inference_write_confirmed as confirmed;
    for requested in [-1, 0, 1] {
        for seen in [-1, 0, 1] {
            for on in [-1, 0, 1] {
                assert_eq!(
                    confirmed(requested, seen, on),
                    i32::from(seen == 1 && (requested == -1 || requested == on))
                );
            }
        }
    }
    for invalid in [-2, 2, i32::MAX] {
        assert_eq!(confirmed(invalid, 1, 1), 0);
        assert_eq!(confirmed(1, invalid, 1), 0);
        assert_eq!(confirmed(1, 1, invalid), 0);
    }
}

/// The harness state mapper crosses the ABI, and it never invents the one
/// value that claims a call was served.
#[test]
fn the_harness_state_table_crosses_the_abi() {
    use trace_commons_contributor::harness_state::HarnessState;
    let code = |state: &str| {
        let state = cstr_str(state);
        unsafe { tc_harness_state_code(state.as_ptr()) }
    };

    for state in [
        HarnessState::NotConnected,
        HarnessState::ConnectedNoCalls,
        HarnessState::Answering,
        HarnessState::ActivityShared,
        HarnessState::Unknown,
    ] {
        let expected = match state {
            HarnessState::NotConnected => TC_HARNESS_STATE_NOT_CONNECTED,
            HarnessState::ConnectedNoCalls => TC_HARNESS_STATE_CONNECTED_NO_CALLS,
            HarnessState::Answering => TC_HARNESS_STATE_ANSWERING,
            HarnessState::ActivityShared => TC_HARNESS_STATE_ACTIVITY_SHARED,
            HarnessState::Unknown => TC_HARNESS_STATE_UNKNOWN,
        };
        assert_eq!(code(state.label()), expected, "{state:?}");
    }

    // A shared family is its own value. A shell that rendered it as
    // ANSWERING would name a tool the ledger cannot name.
    assert_ne!(
        code("activity_shared"),
        TC_HARNESS_STATE_ANSWERING,
        "a shared protocol family must not be painted as one tool answering"
    );

    // A label from a later daemon, a wrong case, and no pointer at all: none
    // of them may become the working light.
    for unknown in ["a_state_from_a_later_daemon", "ANSWERING", ""] {
        assert_eq!(code(unknown), TC_HARNESS_STATE_UNKNOWN, "{unknown:?}");
    }
    assert_eq!(
        unsafe { tc_harness_state_code(std::ptr::null()) },
        TC_HARNESS_STATE_UNKNOWN
    );
}

/// The harness state SENTENCE crosses the ABI too, not only the code.
///
/// Without this call each shell keeps its own map from a state onto one of
/// the payload's sentences -- Swift, C# and Rust, three copies of one
/// decision. Every value here is checked against the payload rather than a
/// literal, so this test cannot become the place a sentence is written.
#[test]
fn the_harness_state_sentence_crosses_the_abi() {
    use trace_commons_contributor::harness_state::HarnessState;
    let line = |state: &str| {
        let state = cstr_str(state);
        take_owned(unsafe { tc_harness_state_line(state.as_ptr()) })
    };
    let copy = trace_commons_contributor::private_inference_copy::private_inference_copy();

    assert_eq!(
        line(HarnessState::NotConnected.label()),
        copy.harness_not_connected
    );
    assert_eq!(
        line(HarnessState::ConnectedNoCalls.label()),
        copy.harness_connected_nothing_seen
    );
    assert_eq!(
        line(HarnessState::Answering.label()),
        copy.harness_answering
    );

    // The two that must claim nothing, and must borrow nothing to do it.
    // `activity_shared` taking the answering sentence would say a call from
    // *this tool* arrived, which is the one thing the state denies knowing.
    for silent in [
        HarnessState::ActivityShared.label(),
        HarnessState::Unknown.label(),
        "a_state_from_a_later_daemon",
        "ANSWERING",
        "",
    ] {
        let sentence = line(silent);
        assert_eq!(sentence, "", "{silent:?} grew a sentence");
        assert_ne!(sentence, copy.harness_answering, "{silent:?}");
        assert_ne!(sentence, copy.harness_connected_nothing_seen, "{silent:?}");
    }
    assert_eq!(
        take_owned(unsafe { tc_harness_state_line(std::ptr::null()) }),
        ""
    );
}

/// The plan outcome's SENTENCE crosses the ABI too, not only the code.
///
/// Four non-committable outcomes had no sentence at all before this, so a
/// preview for one of them opened with a title, a path, no changes and a way
/// out. Every value here is checked against the payload rather than a
/// literal, so this test cannot become the place a sentence is written.
#[test]
fn the_harness_plan_outcome_sentence_crosses_the_abi() {
    use trace_commons_contributor::harness_state::PlanOutcome;
    let line = |outcome: &str| {
        let outcome = cstr_str(outcome);
        take_owned(unsafe { tc_harness_outcome_line(outcome.as_ptr()) })
    };
    let copy = trace_commons_contributor::private_inference_copy::private_inference_copy();

    assert_eq!(
        line(PlanOutcome::Noop.label()),
        copy.harness_plan_nothing_to_change
    );
    assert_eq!(
        line(PlanOutcome::Unparseable.label()),
        copy.harness_unreadable_config
    );
    assert_eq!(
        line(PlanOutcome::NotInstalled.label()),
        copy.harness_not_installed
    );
    assert_eq!(
        line(PlanOutcome::EntryUnusable.label()),
        copy.harness_plan_entry_unusable
    );
    assert_eq!(
        line(PlanOutcome::NoConfigPath.label()),
        copy.harness_plan_no_config_path
    );

    // A plan that has changes shows them; a sentence above them would be the
    // app narrating its own list. And an outcome from a later daemon may not
    // borrow the nearest refusal.
    for silent in [
        PlanOutcome::Changes.label(),
        "an_outcome_from_a_later_daemon",
        "NOOP",
        "",
    ] {
        assert_eq!(line(silent), "", "{silent:?} grew a sentence");
    }
    assert_eq!(
        take_owned(unsafe { tc_harness_outcome_line(std::ptr::null()) }),
        ""
    );
}

/// The when-line crosses the ABI, so all three shells can draw it.
///
/// It used to exist in Rust and stop there: the GTK shell links the crate
/// and rendered it, and macOS and Windows drew nothing. Absence is encoded
/// the way `tc_private_inference_serving_line` encodes it -- an
/// out-of-range integer is the empty string, drawn as no line at all --
/// rather than as a second convention.
#[test]
fn the_harness_last_call_sentence_crosses_the_abi() {
    assert!(take_owned(tc_harness_last_call_line(0)).contains("just now"));
    assert!(take_owned(tc_harness_last_call_line(59)).contains("just now"));
    assert!(take_owned(tc_harness_last_call_line(60)).contains("1 minute ago"));
    assert!(take_owned(tc_harness_last_call_line(120)).contains("2 minutes ago"));
    assert!(take_owned(tc_harness_last_call_line(3_600)).contains("1 hour ago"));
    assert!(take_owned(tc_harness_last_call_line(86_400)).contains("1 day ago"));
    assert!(take_owned(tc_harness_last_call_line(172_800)).contains("2 days ago"));

    // Nothing to report is out of range, and draws no line.
    for absent in [-1, -2, i64::MIN] {
        assert_eq!(
            take_owned(tc_harness_last_call_line(absent)),
            "",
            "{absent}"
        );
    }

    // Whatever a shell computes, it is the shared sentence.
    for seconds in [0_u64, 30, 90, 7_200, 900_000] {
        assert_eq!(
            take_owned(tc_harness_last_call_line(
                i64::try_from(seconds).expect("fits")
            )),
            trace_commons_contributor::private_inference_copy::harness_last_call_line(Some(
                seconds
            ))
        );
    }
}

/// The amount sentence crosses the ABI, and NOT-KNOWN stays not-known.
///
/// The one assertion here that is worth more than the rest is the last one:
/// a negative `micros` -- the value a shell passes when the daemon said the
/// figure was unmeasured -- comes back as the empty string, which is drawn
/// as no line at all. If it ever came back as `$0.00` the surface would be
/// telling every contributor with no readable ledger that they had spent
/// nothing today.
#[test]
fn the_harness_spend_sentence_crosses_the_abi() {
    assert!(take_owned(tc_harness_spend_line(1_230_000)).contains("$1.23"));
    assert!(take_owned(tc_harness_spend_line(0)).contains("$0.00"));
    assert!(take_owned(tc_harness_spend_line(1)).contains("less than $0.01"));
    // The window is in the sentence, not left to a shell to add.
    assert!(take_owned(tc_harness_spend_line(0)).contains("since midnight"));

    // Not known is out of range, and draws no line. Never a zero.
    for absent in [-1, -2, i64::MIN] {
        let line = take_owned(tc_harness_spend_line(absent));
        assert_eq!(line, "", "{absent} rendered a figure: {line}");
        assert!(!line.contains("$"), "{absent} rendered money: {line}");
    }
    assert_ne!(
        take_owned(tc_harness_spend_line(0)),
        take_owned(tc_harness_spend_line(-1)),
        "a measured zero and an unmeasured figure render the same"
    );

    // Whatever a shell holds, it is the shared sentence.
    for micros in [0_u64, 1, 4_999, 5_000, 1_230_000, 999_999_999] {
        assert_eq!(
            take_owned(tc_harness_spend_line(i64::try_from(micros).expect("fits"))),
            trace_commons_contributor::private_inference_copy::harness_spend_line(Some(micros))
        );
    }
}

/// The plan-outcome mapper crosses the ABI, and keeps the refusal that needs
/// a human distinct from the one that needs nothing.
#[test]
fn the_harness_plan_outcome_table_crosses_the_abi() {
    let code = |outcome: &str| {
        let outcome = cstr_str(outcome);
        unsafe { tc_harness_plan_outcome_code(outcome.as_ptr()) }
    };

    assert_eq!(code("changes"), TC_HARNESS_PLAN_CHANGES);
    assert_eq!(code("noop"), TC_HARNESS_PLAN_NOOP);
    assert_eq!(code("unparseable"), TC_HARNESS_PLAN_UNPARSEABLE);
    assert_eq!(code("not_installed"), TC_HARNESS_PLAN_NOT_INSTALLED);
    assert_eq!(code("entry_unusable"), TC_HARNESS_PLAN_ENTRY_UNUSABLE);
    assert_eq!(code("no_config_path"), TC_HARNESS_PLAN_NO_CONFIG_PATH);

    // The distinction the design is emphatic about.
    assert_ne!(
        code("unparseable"),
        code("noop"),
        "a file we refused to rewrite is not the same as nothing to change"
    );

    for unknown in ["an_outcome_from_a_later_daemon", "CHANGES", ""] {
        assert_eq!(code(unknown), TC_HARNESS_PLAN_UNKNOWN, "{unknown:?}");
    }
    assert_eq!(
        unsafe { tc_harness_plan_outcome_code(std::ptr::null()) },
        TC_HARNESS_PLAN_UNKNOWN
    );
}

/// The two availability rules cross the ABI, including the one that is easy
/// to get backwards.
#[test]
fn the_harness_action_table_crosses_the_abi() {
    let available = |action: &str, installed: i32, connected: i32| {
        let action = cstr_str(action);
        unsafe { tc_harness_action_available(action.as_ptr(), installed, connected) }
    };

    // A tool that is not installed cannot be connected.
    assert_eq!(available("connect", 0, 0), 0);
    assert_eq!(available("connect", 1, 0), 1);
    // Nor can one that already is.
    assert_eq!(available("connect", 1, 1), 0);
    // But a connected tool can always be taken back off, installed or not:
    // uninstalling it does not remove the line we put in its config.
    assert_eq!(available("disconnect", 0, 1), 1);
    assert_eq!(available("disconnect", 1, 0), 0);

    // An action this build does not know, and no pointer at all, offer
    // nothing rather than offering a write.
    assert_eq!(available("take-over", 1, 0), 0);
    assert_eq!(available("", 1, 0), 0);
    assert_eq!(
        unsafe { tc_harness_action_available(std::ptr::null(), 1, 0) },
        0
    );
}

/// The harness numbering is disjoint from every other range on this ABI.
///
/// Not a style rule: a shell that fed a harness state to the private
/// inference tone mapper, or the reverse, would render one surface's value as
/// another's meaning. Disjoint ranges make that mistake wrong for every value
/// rather than only for the dangerous one.
#[test]
fn the_harness_codes_do_not_collide_with_the_other_ranges() {
    let harness = [
        TC_HARNESS_STATE_UNKNOWN,
        TC_HARNESS_STATE_NOT_CONNECTED,
        TC_HARNESS_STATE_CONNECTED_NO_CALLS,
        TC_HARNESS_STATE_ANSWERING,
        TC_HARNESS_STATE_ACTIVITY_SHARED,
        TC_HARNESS_PLAN_UNKNOWN,
        TC_HARNESS_PLAN_CHANGES,
        TC_HARNESS_PLAN_NOOP,
        TC_HARNESS_PLAN_UNPARSEABLE,
        TC_HARNESS_PLAN_NOT_INSTALLED,
        TC_HARNESS_PLAN_ENTRY_UNUSABLE,
        TC_HARNESS_PLAN_NO_CONFIG_PATH,
    ];
    let others = [
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL,
        TC_PRIVATE_INFERENCE_TONE_HELD,
        TC_PRIVATE_INFERENCE_TONE_CLEAR,
        TC_PRIVATE_INFERENCE_TONE_ATTENTION,
        TC_PRIVATE_INFERENCE_TONE_REFUSED,
        TC_WITNESS_TONE_NEUTRAL,
        TC_WITNESS_TONE_HELD,
        TC_WITNESS_TONE_CLEAR,
        TC_WITNESS_TONE_ATTENTION,
        TC_WITNESS_TONE_REFUSED,
    ];
    for code in harness {
        assert!(
            !others.contains(&code),
            "{code} collides with another range"
        );
    }
    // And the two harness ranges do not collide with each other.
    let mut seen = harness.to_vec();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), harness.len(), "two harness codes share a value");
}

/// The scale crosses the ABI and is READ, not assumed.
///
/// The trap this pins: three shells dividing by a constant of their own.
/// `scale` is on the wire because a daemon may change it, and the same
/// integer at three scales has to give three answers on the far side of the
/// ABI as well as on this one.
#[test]
fn the_balance_scale_crosses_the_abi_and_is_read() {
    assert_eq!(
        take_owned(tc_near_ai_balance_amount(1, 8_500_000_000, 9)),
        "$8.50"
    );
    assert_eq!(
        take_owned(tc_near_ai_balance_amount(1, 8_500_000_000, 6)),
        "$8500.00"
    );
    assert_eq!(take_owned(tc_near_ai_balance_amount(1, 850, 2)), "$8.50");
    // Rounding is down, so a figure crossing the ABI is never larger than
    // the figure that arrived.
    assert_eq!(
        take_owned(tc_near_ai_balance_amount(1, 9_996_000_000, 9)),
        "$9.99"
    );
}

/// A null remaining figure does not cross the ABI as `$0.00`.
///
/// `remaining_nanos` is nullable even in the `known` state -- the ordinary
/// case for an account nobody capped -- and zero is a real balance meaning
/// the money is gone. The two must never render alike.
#[test]
fn a_null_remaining_balance_never_crosses_as_zero() {
    let absent = take_owned(tc_near_ai_balance_remaining_line(0, 0, 9));
    assert!(
        !absent.is_empty(),
        "a null remaining figure needs a sentence"
    );
    assert!(!absent.contains('$'), "a null carried an amount: {absent}");
    assert!(
        !absent.contains("0.00"),
        "a null rendered as zero: {absent}"
    );

    let zero = take_owned(tc_near_ai_balance_remaining_line(1, 0, 9));
    assert!(
        zero.contains("$0.00"),
        "a real zero must still show: {zero}"
    );
    assert_ne!(absent, zero, "a null and a zero render the same");

    // The bare amount accessor answers the empty string for a null, and an
    // empty string is drawn as no figure -- never as zero.
    let bare = take_owned(tc_near_ai_balance_amount(0, 0, 9));
    assert_eq!(bare, "");
    assert_ne!(bare, take_owned(tc_near_ai_balance_amount(1, 0, 9)));

    // A negative figure is a real overdrawn balance, not an absence: the
    // reason `present` is its own argument rather than an out-of-range
    // integer.
    let overdrawn = take_owned(tc_near_ai_balance_remaining_line(1, -1_000_000_000, 9));
    assert!(
        overdrawn.contains("-$1.00"),
        "an overdrawn account was lost: {overdrawn}"
    );

    // The other two figures use the empty-string convention, and a zero is
    // still not an absence there either.
    assert_eq!(take_owned(tc_near_ai_balance_limit_line(0, 0, 9)), "");
    assert_eq!(take_owned(tc_near_ai_balance_spent_line(0, 0, 9)), "");
    assert!(take_owned(tc_near_ai_balance_spent_line(1, 0, 9)).contains("$0.00"));
}

/// A state this build has never heard of borrows nobody's sentence, and is
/// offered no button.
///
/// The button is the one that opens a browser and mints a key. Offering it
/// for a state nobody read is how a contributor ends up with a second key.
#[test]
fn an_unrecognised_balance_state_borrows_nothing_across_the_abi() {
    let known: Vec<String> = [
        "no_session",
        "session_expired",
        "no_organization",
        "unavailable",
    ]
    .into_iter()
    .map(|s| {
        let c = cstr_str(s);
        take_owned(unsafe { tc_near_ai_balance_state_line(c.as_ptr()) })
    })
    .collect();
    let later = cstr_str("a_balance_state_from_a_later_daemon");
    let unknown = take_owned(unsafe { tc_near_ai_balance_state_line(later.as_ptr()) });
    assert!(!unknown.is_empty());
    for sentence in &known {
        assert_ne!(&unknown, sentence, "an unknown state borrowed: {unknown}");
    }
    // A NULL pointer is a missing state, and gets the "does not report"
    // sentence rather than any of the above.
    let missing = take_owned(unsafe { tc_near_ai_balance_state_line(std::ptr::null()) });
    assert!(!missing.is_empty());
    assert_ne!(missing, unknown);
    for sentence in &known {
        assert_ne!(&missing, sentence);
    }

    for state in [
        "a_balance_state_from_a_later_daemon",
        "known",
        "unavailable",
        "no_organization",
    ] {
        let c = cstr_str(state);
        assert_eq!(
            unsafe { tc_near_ai_balance_action(c.as_ptr()) },
            TC_CREDENTIAL_ACTION_NONE,
            "{state} was offered an action"
        );
    }
    // `known` is the one state that reads as settled, and it means the READ
    // succeeded -- nothing here judges the amount.
    for state in [
        "a_balance_state_from_a_later_daemon",
        "unavailable",
        "no_organization",
        "no_session",
    ] {
        let c = cstr_str(state);
        assert_ne!(
            unsafe { tc_near_ai_balance_state_tone(c.as_ptr()) },
            TC_PRIVATE_INFERENCE_TONE_CLEAR,
            "{state} read as settled",
        );
    }
    assert_eq!(
        unsafe { tc_near_ai_balance_action(std::ptr::null()) },
        TC_CREDENTIAL_ACTION_NONE
    );
    for state in ["no_session", "session_expired"] {
        let c = cstr_str(state);
        assert_eq!(
            unsafe { tc_near_ai_balance_action(c.as_ptr()) },
            TC_CREDENTIAL_ACTION_OBTAIN,
            "{state} must offer the sign-in"
        );
    }
    let known_state = cstr_str("known");
    assert_eq!(
        unsafe { tc_near_ai_balance_state_tone(known_state.as_ptr()) },
        TC_PRIVATE_INFERENCE_TONE_CLEAR
    );
    // "known" is the state whose row is figures, so it carries no sentence.
    assert_eq!(
        take_owned(unsafe { tc_near_ai_balance_state_line(known_state.as_ptr()) }),
        ""
    );
}

/// The age says when THIS COMPUTER asked, and absence draws nothing.
#[test]
fn the_balance_age_crosses_the_abi() {
    assert_eq!(
        take_owned(tc_near_ai_balance_observed_line(0)),
        "Asked for just now."
    );
    assert_eq!(
        take_owned(tc_near_ai_balance_observed_line(120)),
        "Asked for 2 minutes ago."
    );
    for absent in [-1, i64::MIN] {
        assert_eq!(take_owned(tc_near_ai_balance_observed_line(absent)), "");
    }
}

/// The certificate-held list's reading is chosen once, behind the ABI.
///
/// **The argument is `admission_evidence_required` verbatim, not its
/// negation.** That flag is true for a contributor who signed up through NEAR
/// -- who has NO invite -- and false for one enrolled on an invite. So the
/// true arm is the CANDIDATE reading, which looks backwards until you know
/// which way the flag points.
///
/// This exists so that no shell contains the choice. Three shells each
/// writing `flag ? candidate : attested` is three chances to swap them, and a
/// swapped reading tells a contributor with no invite that their session
/// carries cryptographic proof when nothing has attested it.
#[test]
fn the_certificate_reading_is_chosen_behind_the_abi_and_never_by_a_shell() {
    let uninvited = take_owned(tc_certificate_row_line(1));
    let invited = take_owned(tc_certificate_row_line(0));

    assert_ne!(
        uninvited, invited,
        "one sentence is answering for both readings"
    );

    // Stated as outcomes rather than as a mapping: an assertion that merely
    // repeats the implementation would be equally happy with both arms
    // written backwards.
    assert!(
        !uninvited.to_lowercase().contains("signed proof"),
        "a contributor with no invite was promised signed proof: {uninvited}"
    );
    assert!(
        uninvited.to_lowercase().contains("put forward"),
        "a contributor with no invite was not told they can put it forward: {uninvited}"
    );
    assert!(
        invited.to_lowercase().contains("signed proof"),
        "an invited contributor was not told what the certificate carries: {invited}"
    );

    assert_ne!(
        take_owned(tc_certificate_list_title(1)),
        take_owned(tc_certificate_list_title(0))
    );

    // Anything that is not an explicit zero is the flag being set. A shell
    // passing a native bool widened to some other non-zero value must not
    // silently get the invited reading.
    for set in [1, 2, i32::MAX] {
        assert_eq!(take_owned(tc_certificate_row_line(set)), uninvited);
    }
    assert_eq!(take_owned(tc_certificate_row_line(0)), invited);

    // A NEGATIVE VALUE IS A CALLER ERROR, and it resolves to the reading that
    // CLAIMS LESS. Nothing should send one -- a widened bool is 0 or 1 -- but
    // the two arms are not equally safe when something does. The candidate
    // reading says a session can be put forward; the attested reading asserts
    // a security property. A malformed argument must not be able to produce
    // the claim, so this direction is pinned rather than left to whichever
    // way the comparison happened to be written.
    for malformed in [-1, i32::MIN] {
        assert_eq!(
            take_owned(tc_certificate_row_line(malformed)),
            uninvited,
            "a malformed argument produced the reading that asserts attestation"
        );
    }
}

/// The ten login-enrolment refusals reach ten sentences, and none is silent.
///
/// **Unlike an attestation reason, the empty string is never right here.** A
/// reason this build cannot name has nothing honest to add to a mark that
/// already said the true thing; a refusal this build cannot name is the whole
/// of what the contributor is being told, and silence would leave a control
/// that did nothing and said nothing.
#[test]
fn every_login_enrolment_refusal_reaches_its_own_sentence() {
    use trace_commons_contributor::private_inference_copy as copy;
    let labels = [
        copy::NEAR_AI_ENROLL_ALREADY_ENROLLED,
        copy::NEAR_AI_ENROLL_NO_SESSION,
        copy::NEAR_AI_ENROLL_ENDPOINT_REFUSED,
        copy::NEAR_AI_ENROLL_TOKEN_UNAVAILABLE,
        copy::NEAR_AI_ENROLL_START_FAILED,
        copy::NEAR_AI_ENROLL_COMMONS_UNREACHABLE,
        copy::NEAR_AI_ENROLL_COMMONS_UNSUPPORTED,
        copy::NEAR_AI_ENROLL_INVALID,
        copy::NEAR_AI_ENROLL_VERIFICATION_FAILED,
        copy::NEAR_AI_ENROLL_UNAVAILABLE,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for label in labels {
        let line = take_owned(unsafe {
            std::ffi::CString::new(label)
                .map(|c| tc_near_ai_enroll_line(c.as_ptr()))
                .unwrap()
        });
        assert!(
            !line.is_empty(),
            "{label} reached no sentence across the ABI"
        );
        assert!(
            seen.insert(line),
            "{label} shares a sentence with another refusal"
        );
    }

    // A NULL, an empty label and one this build has never seen all reach the
    // generic sentence rather than nothing.
    let generic = take_owned(unsafe {
        std::ffi::CString::new(copy::NEAR_AI_ENROLL_UNAVAILABLE)
            .map(|c| tc_near_ai_enroll_line(c.as_ptr()))
            .unwrap()
    });
    assert_eq!(
        take_owned(unsafe { tc_near_ai_enroll_line(std::ptr::null()) }),
        generic
    );
    for unknown in ["", "near_ai_enroll_from_a_newer_daemon"] {
        let c = std::ffi::CString::new(unknown).unwrap();
        assert_eq!(
            take_owned(unsafe { tc_near_ai_enroll_line(c.as_ptr()) }),
            generic
        );
    }
}

/// The one refusal with a step to take reads as such, and the one that
/// refused nothing does not read as a refusal.
#[test]
fn the_login_enrolment_tones_point_at_the_step_rather_than_the_wall() {
    use trace_commons_contributor::private_inference_copy as copy;
    let tone = |label: &str| {
        let c = std::ffi::CString::new(label).unwrap();
        unsafe { tc_near_ai_enroll_tone(c.as_ptr()) }
    };

    // Not signed in yet: something to do, so it must not be painted as a wall.
    assert_eq!(
        tone(copy::NEAR_AI_ENROLL_NO_SESSION),
        TC_PRIVATE_INFERENCE_TONE_ATTENTION,
        "a contributor who has simply not signed in was shown a refusal"
    );
    // Already joined: the outcome they wanted.
    assert_eq!(
        tone(copy::NEAR_AI_ENROLL_ALREADY_ENROLLED),
        TC_PRIVATE_INFERENCE_TONE_CLEAR,
        "being already joined was painted as a failure"
    );
    for refused in [
        copy::NEAR_AI_ENROLL_COMMONS_UNREACHABLE,
        copy::NEAR_AI_ENROLL_COMMONS_UNSUPPORTED,
        copy::NEAR_AI_ENROLL_VERIFICATION_FAILED,
    ] {
        assert_eq!(
            tone(refused),
            TC_PRIVATE_INFERENCE_TONE_REFUSED,
            "{refused}"
        );
    }
    // A NULL label claims nothing good.
    assert_eq!(
        unsafe { tc_near_ai_enroll_tone(std::ptr::null()) },
        TC_PRIVATE_INFERENCE_TONE_REFUSED
    );
}

#[test]
fn queue_outcome_abi_preserves_known_and_unknown_send_state() {
    use std::ffi::{CStr, CString};
    for label in [
        "dismissed-by-contributor",
        "expired-without-decision",
        "admission_refused",
        "unknown",
    ] {
        let input = CString::new(label).unwrap();
        unsafe {
            let output = trace_commons_contributor_ffi::tc_queue_outcome_line(input.as_ptr());
            assert!(!output.is_null());
            assert_eq!(
                CStr::from_ptr(output).to_str().unwrap(),
                trace_commons_contributor::private_inference_copy::queue_outcome_line(label)
            );
            trace_commons_contributor_ffi::tc_string_free(output);
        }
    }
    unsafe {
        let output = trace_commons_contributor_ffi::tc_queue_outcome_line(std::ptr::null());
        assert_eq!(
            CStr::from_ptr(output).to_str().unwrap(),
            "Status unavailable"
        );
        trace_commons_contributor_ffi::tc_string_free(output);
    }
}
