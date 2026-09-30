//! The release pipeline's launch check, answered by the Tauri app.
//!
//! `scripts/ci/verify-macos-entitlements.sh` launches the signed app with
//! `TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT` set and waits for the file it
//! names. An app carrying the keychain entitlement without a profile that
//! grants it is killed by the kernel at exec and never writes the file, which
//! is the failure the check exists for; an app that launches but cannot reach
//! the store writes `unreachable code=N`. The native shell answers the same
//! variable through the FFI self-check, with the same report lines.
//!
//! This runs before the Tauri builder, so the check opens no window, starts no
//! daemon, and is not forwarded to an already-running instance by the
//! single-instance plugin. It reads a reference that was never stored and
//! writes nothing to the store.

use std::path::PathBuf;

const CHECK_OUT: &str = "TRACE_COMMONS_CREDENTIAL_STORE_CHECK_OUT";

/// If the launch check was requested, answer it and return `true`; the
/// caller then exits without starting the app.
pub fn run_if_requested() -> bool {
    let Some(path) = requested_path(std::env::var_os(CHECK_OUT)) else {
        return false;
    };
    let code = trace_commons_contributor::daemon::credential_store_self_check();
    // A write failure leaves the file empty, which the verifier reports as a
    // failure; there is nothing more useful to do with it here.
    let _ = std::fs::write(path, report(code));
    true
}

fn requested_path(value: Option<std::ffi::OsString>) -> Option<PathBuf> {
    value.filter(|path| !path.is_empty()).map(PathBuf::from)
}

/// The same lines `macos/Sources/TraceCommonsApp/SelfTest.swift` writes.
fn report(code: i32) -> String {
    if code == 0 {
        "reachable\n".to_owned()
    } else {
        format!("unreachable code={code}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::{report, requested_path};
    use std::ffi::OsString;

    #[test]
    fn reports_match_the_native_shell_and_the_verifier() {
        // The verifier passes only on a line starting `reachable`.
        assert_eq!(report(0), "reachable\n");
        assert_eq!(report(1), "unreachable code=1\n");
        assert_eq!(report(2), "unreachable code=2\n");
        assert!(!report(1).starts_with("reachable"));
    }

    #[test]
    fn an_unset_or_empty_variable_does_not_request_the_check() {
        assert_eq!(requested_path(None), None);
        assert_eq!(requested_path(Some(OsString::new())), None);
        assert_eq!(
            requested_path(Some(OsString::from("/tmp/out"))),
            Some("/tmp/out".into())
        );
    }
}
