//! Who owns the installed bytes.
//!
//! The governing rule of this subsystem is that whoever installed the binary
//! owns replacing it. This module answers that question from the running
//! executable's own path alone -- no network, no registry, no package-manager
//! invocation -- so the answer costs nothing and cannot be influenced by
//! anything remote.

use std::path::{Path, PathBuf};

/// The path segment run that means winget placed these bytes. Winget installs
/// portable packages under `%LOCALAPPDATA%\Microsoft\WinGet\Packages\<id>\`.
pub const WINGET_PACKAGES_MARKER: &str = r"\microsoft\winget\packages\";

/// What to tell a contributor whose copy winget owns. Printed verbatim; it is
/// a command, not a path, so it is safe to surface.
pub const WINGET_UPGRADE_COMMAND: &str = "winget upgrade TraceCommons.Contributor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    /// We placed these bytes (`install.ps1` into
    /// `%LOCALAPPDATA%\Programs\TraceCommons`, or `install.sh` into
    /// `~/.local/bin`), so we replace them.
    SelfManaged,
    /// Winget placed these bytes. Defer: a self-swap here leaves winget's
    /// registry record stale, so it would offer a phantom upgrade
    /// indefinitely and `winget upgrade --all` would fight us.
    WingetManaged,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// `std::env::current_exe` failed. Refuse rather than guess: every
    /// downstream decision -- defer or swap, and which file to swap -- is
    /// derived from this path.
    #[error("update_source_exe_path_unavailable")]
    ExePathUnavailable,
}

/// Classify an executable path without touching the filesystem.
pub fn classify(exe: &Path) -> InstallSource {
    // Normalize separators and case before matching. Windows paths are
    // case-insensitive, winget's own casing has varied across releases, and a
    // path may arrive with either separator depending on how the process was
    // launched.
    let normalized = exe.to_string_lossy().replace('/', "\\").to_lowercase();
    if normalized.contains(WINGET_PACKAGES_MARKER) {
        InstallSource::WingetManaged
    } else {
        InstallSource::SelfManaged
    }
}

/// Classify the running executable, and return its path.
pub fn detect() -> Result<(InstallSource, PathBuf), SourceError> {
    let exe = std::env::current_exe().map_err(|_| SourceError::ExePathUnavailable)?;
    // Canonicalize when possible so a symlinked launcher is classified by
    // where the bytes actually live. A failure here is not fatal: the
    // uncanonicalized path is still the path we would swap.
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    Ok((classify(&exe), exe))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_winget_package_path_is_winget_managed() {
        let p = Path::new(
            r"C:\Users\ada\AppData\Local\Microsoft\WinGet\Packages\TraceCommons.Contributor_Microsoft.Winget.Source_8wekyb3d8bbwe\trace-commons-contributor.exe",
        );
        assert_eq!(classify(p), InstallSource::WingetManaged);
    }

    #[test]
    fn the_marker_match_is_case_insensitive() {
        // Windows paths are case-insensitive and winget's own casing has
        // changed across releases ("Winget" vs "WinGet"). Matching on one
        // spelling would silently turn a defer into a self-swap, which is
        // the exact case that leaves winget offering a phantom upgrade
        // forever.
        let p = Path::new(
            r"c:\users\ada\appdata\local\microsoft\winget\packages\x\trace-commons-contributor.exe",
        );
        assert_eq!(classify(p), InstallSource::WingetManaged);
    }

    #[test]
    fn a_forward_slash_spelling_of_the_marker_still_matches() {
        let p = Path::new(
            "C:/Users/ada/AppData/Local/Microsoft/WinGet/Packages/x/trace-commons-contributor.exe",
        );
        assert_eq!(classify(p), InstallSource::WingetManaged);
    }

    #[test]
    fn the_install_ps1_location_is_ours_to_replace() {
        let p = Path::new(
            r"C:\Users\ada\AppData\Local\Programs\TraceCommons\trace-commons-contributor.exe",
        );
        assert_eq!(classify(p), InstallSource::SelfManaged);
    }

    #[test]
    fn the_install_sh_location_is_ours_to_replace() {
        let p = Path::new("/home/ada/.local/bin/trace-commons-contributor");
        assert_eq!(classify(p), InstallSource::SelfManaged);
    }

    #[test]
    fn a_project_directory_that_merely_mentions_winget_is_not_a_winget_install() {
        // The marker is the full segment run, not the word. A developer
        // running a debug build out of a checkout named "winget-work" owns
        // their own binary.
        let p = Path::new("/home/ada/src/winget-work/target/debug/trace-commons-contributor");
        assert_eq!(classify(p), InstallSource::SelfManaged);
    }

    #[test]
    fn detect_reports_a_real_path_for_the_running_test_binary() {
        let (_source, exe) = detect().expect("current_exe is available under cargo test");
        assert!(exe.is_absolute(), "exe path must be absolute");
    }
}
