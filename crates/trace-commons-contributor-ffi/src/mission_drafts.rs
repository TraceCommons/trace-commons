//! Handle-free local mission draft inbox bridge.

use super::*;
use trace_commons_contributor::mission_draft_service::{MAX_MISSION_REQUEST_BYTES, dispatch_json};

/// Execute a bounded local mission inbox request without daemon or enrollment.
/// Returns owned JSON on success, or NULL plus an owned fixed-label error.
/// Proposal content is returned only by an explicit `show` operation.
///
/// # Safety
/// A non-null `request` points to `request_len` readable bytes for the call.
/// `err`, when non-null, points to writable pointer storage without an unfreed
/// prior owned string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_mission_drafts_call(
    request: *const u8,
    request_len: usize,
    err: *mut *mut c_char,
) -> *mut c_char {
    guarded_string(err, || {
        if !err.is_null() {
            unsafe { *err = std::ptr::null_mut() };
        }
        let result = guard_forwarding(|| {
            if request_len > MAX_MISSION_REQUEST_BYTES {
                anyhow::bail!("mission-draft-request-too-large");
            }
            if request.is_null() {
                anyhow::bail!("mission-draft-request-null");
            }
            let bytes = unsafe { std::slice::from_raw_parts(request, request_len) };
            dispatch_json(bytes)
        });
        match result {
            Ok(json) => Ok(to_owned_cstring(&json)),
            Err(label) => {
                set_last_error(&label);
                if !err.is_null() {
                    unsafe { *err = to_owned_cstring(&label) };
                }
                Ok(std::ptr::null_mut())
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(request: serde_json::Value) -> Result<serde_json::Value, String> {
        let bytes = serde_json::to_vec(&request).unwrap();
        let mut error = std::ptr::null_mut();
        let result = unsafe { tc_mission_drafts_call(bytes.as_ptr(), bytes.len(), &mut error) };
        if result.is_null() {
            let label = unsafe { CStr::from_ptr(error) }
                .to_str()
                .unwrap()
                .to_owned();
            unsafe { tc_string_free(error) };
            return Err(label);
        }
        assert!(error.is_null());
        let value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        unsafe { tc_string_free(result) };
        Ok(value)
    }

    #[test]
    fn absent_list_is_account_free_and_does_not_create_state() {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("inbox");
        assert_eq!(
            call(serde_json::json!({"store_dir":store,"operation":{"type":"list"}})).unwrap(),
            serde_json::json!({"type":"list","drafts":[]})
        );
        assert!(!store.exists());
    }

    #[test]
    fn ffi_lifecycle_keeps_content_in_explicit_show() {
        let root = tempfile::tempdir().unwrap();
        let store = root.path().join("inbox");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../trace-commons-protocol/tests/fixtures/mission-draft.json");
        let imported = call(serde_json::json!({
            "store_dir":store,"operation":{"type":"import","file":fixture}
        }))
        .unwrap();
        assert!(imported["draft"]["review"].is_object());
        assert!(imported["draft"].get("proposal").is_none());
        let id = imported["draft"]["id"].as_str().unwrap();
        let listed = call(serde_json::json!({
            "store_dir":store,"operation":{"type":"list"}
        }))
        .unwrap();
        assert!(listed.to_string().find("proposal").is_none());
        let shown = call(serde_json::json!({
            "store_dir":store,"operation":{"type":"show","id":id}
        }))
        .unwrap();
        assert!(shown["draft"]["proposal"].is_object());
        assert_eq!(
            call(serde_json::json!({
                "store_dir":store,"operation":{"type":"delete","id":id}
            }))
            .unwrap()["draft"]["deleted"],
            true
        );
    }

    #[test]
    fn pointer_and_json_failures_return_owned_fixed_labels() {
        let mut error = std::ptr::null_mut();
        let result = unsafe { tc_mission_drafts_call(std::ptr::null(), 0, &mut error) };
        assert!(result.is_null());
        assert_eq!(
            unsafe { CStr::from_ptr(error) }.to_str().unwrap(),
            "mission-draft-request-null"
        );
        unsafe { tc_string_free(error) };
        assert_eq!(
            call(serde_json::json!({"operation":{"type":"unknown"}})).unwrap_err(),
            "mission-draft-request-invalid"
        );
    }
}
