//! Round-trips the C ABI from a Rust integration test that links the crate
//! as an `rlib` (see the crate's `Cargo.toml` comment on why `rlib` is
//! included alongside `cdylib`/`staticlib`). Every helper here frees every
//! string it receives, so a leak-detector run over this file is a genuine
//! check of the ownership rule stated in `include/trace_commons.h`.

use std::ffi::{CStr, CString, c_char};
use std::path::Path;

use trace_commons_contributor_ffi::{
    tc_call, tc_daemon_start, tc_daemon_stop, tc_handle, tc_preview_open, tc_string_free,
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

fn stop(h: *mut tc_handle) {
    unsafe { tc_daemon_stop(h) };
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
