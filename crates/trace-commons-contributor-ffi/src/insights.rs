//! Handle-free Insights bridge. No daemon or contributor account is created.
use super::*;
use trace_commons_contributor::insights::service::{MAX_REQUEST_BYTES, dispatch_json};

/// Execute a local Insights request without starting a daemon or enrollment.
/// Returns owned JSON on success, NULL and an owned fixed error label on failure.
/// Free either owned string with `tc_string_free`. Clears `*err` on success.
///
/// The request is UTF-8 JSON, at most 65536 bytes, without a trailing NUL:
/// `{"store_dir":"/chosen/store","operation":{"type":"list"}}`.
/// Analyze: `{"type":"analyze","source":"codex","file":"/chosen/file","save":false}`.
/// Explain/delete: `{"type":"explain","id":"..."}` / `{"type":"delete","id":"..."}`.
/// User assessment: `{"type":"annotate","id":"...","category":"docs","outcome":"partial"}`.
/// Clear assessment: `{"type":"clear_annotation","id":"..."}`.
/// Omit store_dir to use the shared platform local-data Insights directory.
/// Runs synchronous bounded-source local IO: call off the UI thread; closing a
/// window does not cancel a started operation. Keep buffers alive until return.
///
/// # Safety
/// A non-null `request` must point to `request_len` readable bytes for the call.
/// `err`, if non-null, must point to writable pointer storage, with no unfreed
/// prior owned string. NULL requests and oversized lengths are refused before
/// any request memory is read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_insights_call(
    request: *const u8,
    request_len: usize,
    err: *mut *mut c_char,
) -> *mut c_char {
    guarded_string(err, || {
        if !err.is_null() {
            unsafe { *err = std::ptr::null_mut() };
        }
        // Every fallible operation inside this forwarding guard emits only a
        // fixed label. Source paths and parser details never cross the ABI.
        let result = guard_forwarding(|| {
            if request_len > MAX_REQUEST_BYTES {
                anyhow::bail!("insights-request-too-large");
            }
            if request.is_null() {
                anyhow::bail!("insights-request-null");
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

    fn failure(bytes: *const u8, len: usize, expected: &str) {
        let mut error = std::ptr::null_mut();
        let response = unsafe { tc_insights_call(bytes, len, &mut error) };
        assert!(response.is_null());
        assert!(!error.is_null());
        assert_eq!(unsafe { CStr::from_ptr(error) }.to_str().unwrap(), expected);
        unsafe { tc_string_free(error) };
    }

    #[test]
    fn invalid_requests_are_bounded_and_owned_errors_are_freeable() {
        failure(std::ptr::null(), 0, "insights-request-null");
        // Even NULL cannot be dereferenced when the supplied bound is invalid.
        failure(
            std::ptr::null(),
            MAX_REQUEST_BYTES + 1,
            "insights-request-too-large",
        );
        failure([0xff].as_ptr(), 1, "insights-request-invalid-utf8");
        let malformed = br#"{"secret":"do-not-echo"}"#;
        failure(
            malformed.as_ptr(),
            malformed.len(),
            "insights-request-invalid",
        );
        assert!(unsafe { tc_insights_call(std::ptr::null(), 0, std::ptr::null_mut()) }.is_null());
    }

    #[test]
    fn account_free_list_returns_owned_json() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({"store_dir":temp.path().join("insights"),"operation":{"type":"list"}})).unwrap();
        let mut error = std::ptr::null_mut();
        let result = unsafe { tc_insights_call(bytes.as_ptr(), bytes.len(), &mut error) };
        assert!(error.is_null());
        assert!(!result.is_null());
        let value: serde_json::Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        assert_eq!(value, serde_json::json!({"type":"list","insights":[]}));
        unsafe { tc_string_free(result) };
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
}
