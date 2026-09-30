//! The roots screen must reach recovery even when a stored OS entry is missing.

use std::ffi::{CStr, CString};

use trace_commons_contributor::config::{ConfigStore, DAEMON_SETTINGS_FILE};
use trace_commons_contributor::daemon::settings::DaemonSettings;
use trace_commons_contributor_ffi::{
    TC_CREDENTIAL_ACTION_FORGET, tc_call, tc_daemon_start_with_settings, tc_daemon_stop,
    tc_handle_free, tc_near_ai_credential_action, tc_string_free,
};

#[test]
fn roots_submission_reaches_missing_credential_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().to_owned()).unwrap();
    // A synthetic reference with no OS entry. This test never writes a secret
    // or reads a user source directory. The absent binding cannot authorize use.
    let metadata = serde_json::json!({
        "reference": {"version": 1, "id": "b2708f98-22ab-449b-b98e-dda62e41456b"},
        "session": {"stored_at": "2026-09-09T00:00:00Z"},
        "binding_digest": "0".repeat(64)
    });
    let mut settings = serde_json::to_value(DaemonSettings::default()).unwrap();
    settings["cloud_credentials"] = metadata.clone();
    store
        .write_daemon_file(
            DAEMON_SETTINGS_FILE,
            &serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
    let path = CString::new(dir.path().to_str().unwrap()).unwrap();
    let preferences = CString::new(
        serde_json::json!({
            "claude_source": {"mode": "off"},
            "codex_source": {"mode": "off"}
        })
        .to_string(),
    )
    .unwrap();
    let mut error = std::ptr::null_mut();
    let handle =
        unsafe { tc_daemon_start_with_settings(path.as_ptr(), preferences.as_ptr(), &mut error) };
    let error_label = if error.is_null() {
        None
    } else {
        let value = unsafe { CStr::from_ptr(error) }
            .to_string_lossy()
            .into_owned();
        unsafe { tc_string_free(error) };
        Some(value)
    };
    assert!(
        !handle.is_null(),
        "roots submission failed: {error_label:?}"
    );
    let method = CString::new("near_ai_credential_status").unwrap();
    let params = CString::new("{}").unwrap();
    let output = unsafe { tc_call(handle, method.as_ptr(), params.as_ptr()) };
    assert!(!output.is_null());
    let status: serde_json::Value =
        serde_json::from_slice(unsafe { CStr::from_ptr(output) }.to_bytes()).unwrap();
    unsafe {
        tc_string_free(output);
        tc_daemon_stop(handle);
        tc_handle_free(handle);
    }
    assert_eq!(status["result"]["state"], "storage_unavailable");
    let state = CString::new("storage_unavailable").unwrap();
    assert_eq!(
        unsafe { tc_near_ai_credential_action(state.as_ptr()) },
        TC_CREDENTIAL_ACTION_FORGET
    );
    let persisted: serde_json::Value = serde_json::from_slice(
        &store
            .read_daemon_file(DAEMON_SETTINGS_FILE)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["cloud_credentials"], metadata);
    assert!(
        persisted
            .get("near_ai_session")
            .is_none_or(serde_json::Value::is_null)
    );
    assert!(
        persisted
            .get("near_ai_inference")
            .is_none_or(serde_json::Value::is_null)
    );
}
