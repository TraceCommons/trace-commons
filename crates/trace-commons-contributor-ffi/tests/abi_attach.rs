//! `tc_daemon_attach`: the way out of `"already-running"`.
//!
//! These drive the C ABI exactly as a host does -- raw pointers, owned
//! strings, `tc_string_free` -- because the defect this call exists to fix
//! lived entirely in what a host was told. A test that reached past the ABI
//! into the Rust would not have caught it.

use std::ffi::{CStr, CString, c_char, c_void};
use std::path::Path;
use std::sync::Mutex;

use trace_commons_contributor_ffi::{
    tc_call, tc_daemon_attach, tc_daemon_start, tc_daemon_stop, tc_handle, tc_handle_free,
    tc_preview_open, tc_string_free, tc_subscribe,
};

fn cstr(p: &Path) -> CString {
    CString::new(p.to_str().unwrap()).unwrap()
}

fn cstr_str(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// Same reason `abi.rs` does this: without declared roots the daemon scans
/// the developer's real ~/.claude and ~/.codex.
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
    assert!(!h.is_null(), "fixture daemon did not start");
    h
}

fn take_err(err: *mut c_char) -> String {
    assert!(
        !err.is_null(),
        "expected a fixed label, got no error string"
    );
    let s = unsafe { CStr::from_ptr(err) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(err) };
    s
}

fn call(h: *mut tc_handle, method: &str, params: &str) -> serde_json::Value {
    let raw = unsafe { tc_call(h, cstr_str(method).as_ptr(), cstr_str(params).as_ptr()) };
    assert!(!raw.is_null(), "tc_call returned NULL, which it never may");
    let text = unsafe { CStr::from_ptr(raw) }
        .to_string_lossy()
        .into_owned();
    unsafe { tc_string_free(raw) };
    serde_json::from_str(&text).expect("tc_call returned something that is not JSON")
}

/// The whole point of the call: a second starter is refused the lock, and
/// attaching gets a handle that answers, instead of a shell reporting a
/// running watcher as absent.
#[test]
fn attaching_reaches_the_daemon_that_holds_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let started = start(dir.path());

    // What a host sees first: the start it tried is refused, by the label
    // the header documents as "not an error to repair".
    let mut err: *mut c_char = std::ptr::null_mut();
    let second = unsafe { tc_daemon_start(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(second.is_null(), "a second start took the lock");
    assert_eq!(take_err(err), "already-running");

    // And what it can now do about it.
    let mut err: *mut c_char = std::ptr::null_mut();
    let attached = unsafe { tc_daemon_attach(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(
        !attached.is_null(),
        "attach failed after already-running: {}",
        take_err(err)
    );

    let response = call(attached, "status", "{}");
    assert!(
        response.get("error").map_or(true, |e| e.is_null()),
        "attached status returned an error frame: {response}"
    );
    assert!(
        response.get("result").is_some(),
        "attached status carried no result: {response}"
    );

    unsafe { tc_daemon_stop(attached) };
    unsafe { tc_handle_free(attached) };
    unsafe { tc_daemon_stop(started) };
    unsafe { tc_handle_free(started) };
}

/// Stopping an attached handle must not stop the daemon. Asserting only the
/// refusal would pass just as well against a client that had killed it, so
/// this asserts the daemon is still answering afterwards.
#[test]
fn stopping_an_attached_handle_leaves_the_daemon_running() {
    let dir = tempfile::tempdir().unwrap();
    let started = start(dir.path());

    let mut err: *mut c_char = std::ptr::null_mut();
    let attached = unsafe { tc_daemon_attach(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(!attached.is_null());

    let refused = call(attached, "shutdown", "{}");
    assert_eq!(
        refused["error"]["message"], "attached-stop-refused",
        "an attached handle forwarded shutdown: {refused}"
    );

    unsafe { tc_daemon_stop(attached) };
    unsafe { tc_handle_free(attached) };

    // The daemon this shell attached to is still up and still answering.
    let after = call(started, "status", "{}");
    assert!(
        after.get("result").is_some(),
        "the daemon stopped when an attached handle was stopped: {after}"
    );

    unsafe { tc_daemon_stop(started) };
    unsafe { tc_handle_free(started) };
}

/// The redacted body is the in-process content exemption. An attached
/// handle must say so rather than report the daemon as stopped, which is the
/// same false statement this whole path exists to stop making.
#[test]
fn preview_on_an_attached_handle_names_the_reason() {
    let dir = tempfile::tempdir().unwrap();
    let started = start(dir.path());

    let mut err: *mut c_char = std::ptr::null_mut();
    let attached = unsafe { tc_daemon_attach(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(!attached.is_null());

    let mut perr: *mut c_char = std::ptr::null_mut();
    let preview = unsafe {
        tc_preview_open(
            attached,
            cstr_str("00000000-0000-0000-0000-000000000000").as_ptr(),
            &mut perr,
        )
    };
    assert!(
        preview.is_null(),
        "an attached handle opened a body preview"
    );
    assert_eq!(take_err(perr), "preview-requires-embedded");

    unsafe { tc_daemon_stop(attached) };
    unsafe { tc_handle_free(attached) };
    unsafe { tc_daemon_stop(started) };
    unsafe { tc_handle_free(started) };
}

static EVENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());

extern "C" fn record_event(event_json: *const c_char, _ctx: *mut c_void) {
    let text = unsafe { CStr::from_ptr(event_json) }
        .to_string_lossy()
        .into_owned();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        if let Some(name) = value.get("event").and_then(|e| e.as_str()) {
            EVENTS.lock().unwrap().push(name.to_string());
        }
    }
}

/// Subscribing over the socket delivers the `snapshot` frame the daemon
/// sends whoever just subscribed -- the courtesy the in-process path does
/// not get, and the reason an attached shell can paint without polling.
#[test]
fn an_attached_subscriber_receives_the_snapshot_push() {
    let dir = tempfile::tempdir().unwrap();
    let started = start(dir.path());
    EVENTS.lock().unwrap().clear();

    let mut err: *mut c_char = std::ptr::null_mut();
    let attached = unsafe { tc_daemon_attach(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(!attached.is_null());

    let token = unsafe { tc_subscribe(attached, Some(record_event), std::ptr::null_mut()) };
    assert_ne!(token, 0, "tc_subscribe refused an attached handle");

    // Poll for the push rather than sleeping a fixed span: the frame is
    // already in flight when `subscribe` returns, and a fixed sleep is
    // either flaky or slow.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let mut seen = false;
    while std::time::Instant::now() < deadline {
        if EVENTS.lock().unwrap().iter().any(|e| e == "snapshot") {
            seen = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(seen, "no snapshot push reached an attached subscriber");

    unsafe { tc_daemon_stop(attached) };
    unsafe { tc_handle_free(attached) };
    unsafe { tc_daemon_stop(started) };
    unsafe { tc_handle_free(started) };
}

/// Attaching where nothing is listening names that, and is distinct from
/// every start failure.
#[test]
fn attaching_with_no_daemon_says_nothing_is_listening() {
    let dir = tempfile::tempdir().unwrap();
    let mut err: *mut c_char = std::ptr::null_mut();
    let h = unsafe { tc_daemon_attach(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(h.is_null(), "attached to a daemon that does not exist");
    assert_eq!(take_err(err), "no-daemon-listening");
}
