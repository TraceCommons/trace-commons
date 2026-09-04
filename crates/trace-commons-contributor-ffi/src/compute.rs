//! Independent compute ABI. No enrollment or trace daemon is required.

use super::*;
use trace_commons_contributor::compute::{ComputeCommand, ComputeController};

#[allow(non_camel_case_types)]
pub struct tc_compute_handle {
    controller: ComputeController,
}

/// Open one app-owned compute controller, restoring consent as paused. No worker
/// is launched. Failure returns NULL and a fixed error label.
///
/// # Safety
/// `config_dir` must be a valid NUL-terminated UTF-8 absolute path. `err`, if
/// non-null, must point to writable pointer storage. Free errors with tc_string_free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_compute_open(
    config_dir: *const c_char,
    err: *mut *mut c_char,
) -> *mut tc_compute_handle {
    if !err.is_null() {
        unsafe { *err = std::ptr::null_mut() };
    }
    guard(|| {
        let path = unsafe { borrow_str(config_dir) }?;
        let controller = ComputeController::open(std::path::Path::new(path))?;
        let ptr = Box::into_raw(Box::new(tc_compute_handle { controller }));
        registry_insert(ptr as usize, AllocKind::Compute);
        Ok(ptr)
    })
    .unwrap_or_else(|_| {
        set_last_error("compute-open-failed");
        if !err.is_null() {
            unsafe { *err = to_owned_cstring("compute-open-failed") };
        }
        std::ptr::null_mut()
    })
}

/// Return an owned compute snapshot JSON, or NULL with tc_last_error on failure.
///
/// # Safety
/// Handle must come from tc_compute_open and remain alive throughout this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_compute_status_json(handle: *mut tc_compute_handle) -> *mut c_char {
    guard(|| {
        anyhow::ensure!(
            registry_is(handle as usize, AllocKind::Compute),
            "invalid-handle"
        );
        let snapshot = unsafe { &*handle }.controller.snapshot();
        Ok(to_owned_cstring(&serde_json::to_string(&snapshot)?))
    })
    .unwrap_or_else(|_| {
        set_last_error("compute-status-failed");
        std::ptr::null_mut()
    })
}

/// Execute a strict tagged JSON command and return an owned snapshot. Enable
/// takes ram_allowance_gib; resume, pause, disable take no additional fields.
/// This build refuses enable/resume because no packaged backend is available.
/// Invalid inputs return NULL with a fixed tc_last_error label. Execute off the
/// UI thread: commands serialize settings I/O. Input is bounded to 4096 bytes.
///
/// # Safety
/// Handle must remain alive throughout the call. command_json must point to a
/// valid NUL-terminated string (at most 4096 bytes excluding its terminator).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_compute_command_json(
    handle: *mut tc_compute_handle,
    command_json: *const c_char,
) -> *mut c_char {
    guard(|| {
        anyhow::ensure!(
            registry_is(handle as usize, AllocKind::Compute),
            "invalid-handle"
        );
        anyhow::ensure!(!command_json.is_null(), "invalid-command");
        // Bound scanning before UTF-8/JSON parsing. A C caller still owns the
        // standard obligation that its pointer denotes a valid C string.
        let mut len = 0;
        while len <= 4096 && unsafe { *command_json.add(len) } != 0 {
            len += 1;
        }
        anyhow::ensure!(len <= 4096, "invalid-command");
        let bytes = unsafe { std::slice::from_raw_parts(command_json.cast::<u8>(), len) };
        let command: ComputeCommand = serde_json::from_slice(bytes)?;
        let snapshot = unsafe { &*handle }.controller.command(command);
        Ok(to_owned_cstring(&serde_json::to_string(&snapshot)?))
    })
    .unwrap_or_else(|_| {
        set_last_error("compute-command-failed");
        std::ptr::null_mut()
    })
}

/// Free a compute controller. This build owns no worker process.
///
/// # Safety
/// Must not run concurrently with any other call using this handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_compute_free(handle: *mut tc_compute_handle) {
    let _ = guard(|| {
        if handle.is_null() {
            return Ok(());
        }
        if registry_take(handle as usize, AllocKind::Compute).is_ok() {
            drop(unsafe { Box::from_raw(handle) });
        } else {
            set_last_error("invalid-compute-handle");
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_ffi_independent_and_fail_closed() {
        unsafe {
            let root = tempfile::tempdir().unwrap();
            let path = CString::new(root.path().to_str().unwrap()).unwrap();
            let mut err = std::ptr::null_mut();
            let handle = tc_compute_open(path.as_ptr(), &mut err);
            assert!(!handle.is_null());
            assert!(err.is_null());
            let status = tc_compute_status_json(handle);
            let json: serde_json::Value =
                serde_json::from_slice(CStr::from_ptr(status).to_bytes()).unwrap();
            assert_eq!(json["state"], "disabled");
            assert_eq!(json["available"], false);
            tc_string_free(status);
            let cmd = CString::new(r#"{"command":"enable","ram_allowance_gib":8}"#).unwrap();
            let result = tc_compute_command_json(handle, cmd.as_ptr());
            let json: serde_json::Value =
                serde_json::from_slice(CStr::from_ptr(result).to_bytes()).unwrap();
            assert_eq!(json["state"], "unavailable");
            assert_eq!(json["consent_granted"], false);
            tc_string_free(result);
            for invalid in ["{", r#"{"command":"resume","token":"sensitive"}"#] {
                let invalid = CString::new(invalid).unwrap();
                assert!(tc_compute_command_json(handle, invalid.as_ptr()).is_null());
            }
            let oversized = CString::new(" ".repeat(4097)).unwrap();
            assert!(tc_compute_command_json(handle, oversized.as_ptr()).is_null());
            tc_compute_free(handle);
            assert!(tc_compute_status_json(handle).is_null());
            tc_compute_free(handle);
            assert!(tc_compute_status_json(std::ptr::null_mut()).is_null());
            assert!(tc_compute_open(std::ptr::null(), &mut err).is_null());
            tc_string_free(err);
        }
    }
}
