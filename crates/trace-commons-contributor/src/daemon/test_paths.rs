//! Absolute paths shaped for the host platform, for tests.
//!
//! Tests in this crate used to spell fixture working directories as
//! `/Users/testuser/code/api`. On Windows that string has a root but no
//! prefix, so `Path::is_absolute` is false, `normalize_project_key` refuses
//! it, and every such session falls into the unknown bucket -- which turned
//! a dozen assertions about labels, policy, and opt-in into assertions
//! about `unknown-project`.
//!
//! Gating those tests on Unix would have deleted the Windows coverage
//! instead of fixing it. These helpers keep the coverage by spelling the
//! fixture path the way the host does: `C:\Users\testuser\code\api` on
//! Windows, `/Users/testuser/code/api` everywhere else.

/// An absolute path for the host platform, built from `/`-separated
/// segments (`"Users/testuser/code/api"`).
///
/// On Windows this is anchored at `C:`, the drive every runner has; the
/// path need not exist, since the callers use it as a recorded cwd.
pub(crate) fn abs(segments: &str) -> String {
    let segments = segments.trim_start_matches('/');
    #[cfg(windows)]
    {
        format!(r"C:\{}", segments.replace('/', "\\"))
    }
    #[cfg(not(windows))]
    {
        format!("/{segments}")
    }
}

/// [`abs`], escaped for embedding inside a JSON string literal.
///
/// The Windows form contains backslashes, and a fixture that pasted them
/// raw into a `.jsonl` line would be emitting invalid JSON (or, worse,
/// valid JSON with `\U` swallowed).
pub(crate) fn abs_json(segments: &str) -> String {
    json_escaped(&abs(segments))
}

/// An already-built path, escaped for embedding inside a JSON string
/// literal. A no-op on the platforms whose separator is `/`.
pub(crate) fn json_escaped(path: &str) -> String {
    path.replace('\\', "\\\\")
}

/// The project key [`abs`] normalizes to: the same path, case-folded on
/// the platforms whose filesystems are case-insensitive.
///
/// Deliberately not a call to `normalize_project_key` -- a test that
/// compared one normalization against another would agree with itself no
/// matter what normalization did.
pub(crate) fn abs_key(segments: &str) -> String {
    let path = abs(segments);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        path.to_lowercase()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absolute_path_is_absolute_on_this_platform() {
        assert!(std::path::Path::new(&abs("Users/testuser/code/api")).is_absolute());
    }

    #[test]
    fn the_json_form_survives_a_round_trip() {
        let line = format!(r#"{{"cwd":"{}"}}"#, abs_json("Users/testuser/code/api"));
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["cwd"], abs("Users/testuser/code/api"));
    }

    #[test]
    fn the_key_form_matches_what_normalization_produces() {
        assert_eq!(
            crate::daemon::policy::project_key_for(Some(&abs("Users/testuser/code/api"))),
            abs_key("Users/testuser/code/api")
        );
    }
}
