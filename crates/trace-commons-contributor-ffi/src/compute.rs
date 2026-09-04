//! Independent compute ABI. No enrollment or trace daemon is required.

use super::*;
use trace_commons_contributor::compute::{ComputeCommand, ComputeController};

/// Shared navigation and loading/error copy, available without a handle.
#[unsafe(no_mangle)]
pub extern "C" fn tc_compute_copy_json() -> *mut c_char {
    guard(|| {
        Ok(to_owned_cstring(&serde_json::to_string(
            &trace_commons_contributor::compute::ComputeCopy::default(),
        )?))
    })
    .unwrap_or_else(|_| {
        set_last_error("compute-copy-failed");
        std::ptr::null_mut()
    })
}

/// Wait at most timeout_ms (clamped to 30 seconds) for worker stop. Returned
/// snapshot.worker_stopped is process evidence, distinct from drain_outcome.
/// Keep the handle alive if it is false. Call off the UI thread.
/// # Safety
/// Handle must remain alive throughout this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_compute_shutdown(
    handle: *mut tc_compute_handle,
    timeout_ms: u64,
) -> *mut c_char {
    guard(|| {
        anyhow::ensure!(
            registry_is(handle as usize, AllocKind::Compute),
            "invalid-handle"
        );
        let snapshot = unsafe { &*handle }
            .controller
            .shutdown(std::time::Duration::from_millis(timeout_ms));
        Ok(to_owned_cstring(&serde_json::to_string(&snapshot)?))
    })
    .unwrap_or_else(|_| {
        set_last_error("compute-shutdown-failed");
        std::ptr::null_mut()
    })
}

#[allow(non_camel_case_types)]
pub struct tc_compute_handle {
    controller: ComputeController,
}

/// Explicit development-only constructor; the production open never calls it.
/// Release/non-Unix builds refuse it. Configuration is strict JSON containing
/// binary, expected_sha256, coordinator and startup_timeout_secs. No worker is
/// launched until an explicit Enable or Resume command.
/// # Safety
/// config_dir and local_config_json must be valid NUL-terminated strings;
/// local_config_json is limited to 4096 bytes. err must be null or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_compute_open_local(
    config_dir: *const c_char,
    local_config_json: *const c_char,
    err: *mut *mut c_char,
) -> *mut tc_compute_handle {
    if !err.is_null() {
        unsafe { *err = std::ptr::null_mut() };
    }
    guard(|| {
        let path = unsafe { borrow_str(config_dir) }?;
        let config = serde_json::from_slice(unsafe { bounded_json_bytes(local_config_json) }?)?;
        let controller = ComputeController::open_local(std::path::Path::new(path), config)?;
        let ptr = Box::into_raw(Box::new(tc_compute_handle { controller }));
        registry_insert(ptr as usize, AllocKind::Compute);
        Ok(ptr)
    })
    .unwrap_or_else(|_| {
        set_last_error("compute-local-open-failed");
        if !err.is_null() {
            unsafe { *err = to_owned_cstring("compute-local-open-failed") };
        }
        std::ptr::null_mut()
    })
}

unsafe fn bounded_json_bytes<'a>(json: *const c_char) -> anyhow::Result<&'a [u8]> {
    anyhow::ensure!(!json.is_null(), "invalid-compute-json");
    let mut len = 0;
    while len <= 4096 && unsafe { *json.add(len) } != 0 {
        len += 1;
    }
    anyhow::ensure!(len <= 4096, "invalid-compute-json");
    Ok(unsafe { std::slice::from_raw_parts(json.cast::<u8>(), len) })
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
/// Production-open handles refuse enable/resume without a packaged backend.
/// Explicit local-development handles enqueue lifecycle work on their actor.
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
        let bytes = unsafe { bounded_json_bytes(command_json) }?;
        let command: ComputeCommand = serde_json::from_slice(bytes)?;
        let snapshot = unsafe { &*handle }.controller.command(command);
        Ok(to_owned_cstring(&serde_json::to_string(&snapshot)?))
    })
    .unwrap_or_else(|_| {
        set_last_error("compute-command-failed");
        std::ptr::null_mut()
    })
}

/// Free a compute controller after any worker has stopped. A local handle with
/// pending work or an unconfirmed stop is retained and tc_last_error is set;
/// call shutdown and retry after worker_stopped=true and command_pending=false.
///
/// # Safety
/// Must not run concurrently with any other call using this handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_compute_free(handle: *mut tc_compute_handle) {
    let _ = guard(|| {
        if handle.is_null() {
            return Ok(());
        }
        if registry_is(handle as usize, AllocKind::Compute) {
            let snapshot = unsafe { &*handle }.controller.snapshot();
            if !snapshot.worker_stopped || snapshot.command_pending {
                set_last_error("compute-worker-still-owned");
                return Ok(());
            }
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

    #[cfg(unix)]
    #[test]
    fn local_handle_cannot_be_freed_with_unconfirmed_work() {
        use std::os::unix::fs::PermissionsExt;
        use trace_commons_contributor::compute::LocalWorkerConfig;
        let root = tempfile::tempdir().unwrap();
        let binary = root.path().join("sleepy-worker");
        let script = b"#!/bin/sh\nexec /bin/sleep 60\n";
        std::fs::write(&binary, script).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
        // SHA-256 of the exact public test script above.
        let expected_sha256 =
            "eb2c0b11d46d6efb3031027345fb05b0e168ed4c2244e3639b302e3e8e1361c9".into();
        let controller = ComputeController::open_local(
            root.path(),
            LocalWorkerConfig {
                binary,
                expected_sha256,
                coordinator: "ws://127.0.0.1:9999".into(),
                startup_timeout_secs: 20,
            },
        )
        .unwrap();
        controller.command(ComputeCommand::Enable {
            ram_allowance_gib: 1,
        });
        let handle = Box::into_raw(Box::new(tc_compute_handle { controller }));
        registry_insert(handle as usize, AllocKind::Compute);
        unsafe {
            tc_compute_free(handle);
            assert!(registry_is(handle as usize, AllocKind::Compute));
            let stopped = tc_compute_shutdown(handle, 10_000);
            let json: serde_json::Value =
                serde_json::from_slice(CStr::from_ptr(stopped).to_bytes()).unwrap();
            assert_eq!(json["worker_stopped"], true);
            tc_string_free(stopped);
            tc_compute_free(handle);
            assert!(!registry_is(handle as usize, AllocKind::Compute));
        }
    }

    #[test]
    fn compute_ffi_independent_and_fail_closed() {
        unsafe {
            let root = tempfile::tempdir().unwrap();
            let path = CString::new(root.path().to_str().unwrap()).unwrap();
            let mut err = std::ptr::null_mut();
            let handle = tc_compute_open(path.as_ptr(), &mut err);
            assert!(!handle.is_null());
            assert!(err.is_null());
            let copy = tc_compute_copy_json();
            assert!(!copy.is_null());
            let json: serde_json::Value =
                serde_json::from_slice(CStr::from_ptr(copy).to_bytes()).unwrap();
            assert_eq!(json["destination"], "Compute");
            assert!(json["quit_refused"].is_string());
            tc_string_free(copy);
            let stopped = tc_compute_shutdown(handle, 0);
            let json: serde_json::Value =
                serde_json::from_slice(CStr::from_ptr(stopped).to_bytes()).unwrap();
            assert_eq!(json["worker_stopped"], true);
            assert!(json["drain_outcome"].is_null());
            tc_string_free(stopped);
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
