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
    /// Somewhere we do not recognize: Homebrew's prefix, a distribution
    /// package in `/usr/bin`, a read-only Nix store path, or an
    /// `install.sh --dir` location we cannot name. Defer.
    ///
    /// This is the DEFAULT, and the direction of the default is the whole
    /// point. The spec's governing rule is "whoever installed the binary owns
    /// replacing it", so the question is not "do we know a package manager
    /// owns this?" but "do we know WE placed it?" -- and only the two paths
    /// above answer yes. Treating unknown locations as ours would clobber a
    /// Homebrew-installed CLI (this project publishes a Homebrew formula, so
    /// that is a real install path, not a hypothetical), leaving brew's
    /// version records stale in exactly the way the winget arm exists to
    /// prevent.
    ///
    /// Recognizing only the default install locations means an
    /// `install.sh --dir /opt/tc` copy is classified Unrecognized and defers
    /// rather than self-updating. That is a false negative and it is the safe
    /// direction: the contributor is told to re-run the installer, instead of
    /// having a binary replaced somewhere we cannot reason about. A future
    /// improvement is for the installers to drop a receipt file beside the
    /// binary so custom directories can be recognized positively.
    Unrecognized,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// `std::env::current_exe` failed. Refuse rather than guess: every
    /// downstream decision -- defer or swap, and which file to swap -- is
    /// derived from this path.
    #[error("update_source_exe_path_unavailable")]
    ExePathUnavailable,
}

/// The directory `install.ps1` installs into, relative to `%LOCALAPPDATA%`.
const WINDOWS_SELF_MANAGED_MARKER: &str = r"\programs\tracecommons\";

/// The directory `install.sh` installs into, relative to `$HOME`.
const UNIX_SELF_MANAGED_SUFFIX: &str = "/.local/bin";

/// Classify an executable path without touching the filesystem.
///
/// Note the shape: winget is checked first because its marker is the most
/// specific, then the two locations we own, and anything left over is
/// Unrecognized. The final arm is a deliberate refusal, not a fallback.
pub fn classify(exe: &Path) -> InstallSource {
    // Normalize separators and case before matching. Windows paths are
    // case-insensitive, winget's own casing has varied across releases, and a
    // path may arrive with either separator depending on how the process was
    // launched.
    let windows_style = exe.to_string_lossy().replace('/', "\\").to_lowercase();
    if windows_style.contains(WINGET_PACKAGES_MARKER) {
        return InstallSource::WingetManaged;
    }
    if windows_style.contains(WINDOWS_SELF_MANAGED_MARKER) {
        return InstallSource::SelfManaged;
    }
    if let Some(parent) = exe.parent() {
        if parent
            .to_string_lossy()
            .replace('\\', "/")
            .ends_with(UNIX_SELF_MANAGED_SUFFIX)
        {
            return InstallSource::SelfManaged;
        }
    }
    InstallSource::Unrecognized
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
        // running a debug build out of a checkout named "winget-work" has
        // their own binary that they own, but this is not one of our two
        // recognized install locations, so it is Unrecognized.
        let p = Path::new("/home/ada/src/winget-work/target/debug/trace-commons-contributor");
        assert_eq!(classify(p), InstallSource::Unrecognized);
    }

    #[test]
    fn detect_reports_a_real_path_for_the_running_test_binary() {
        let (_source, exe) = detect().expect("current_exe is available under cargo test");
        assert!(exe.is_absolute(), "exe path must be absolute");
    }

    #[test]
    fn a_homebrew_installed_path_is_unrecognized() {
        // This project publishes a Homebrew formula. A self-update of a
        // brew-installed copy would leave brew's version records stale.
        let p = Path::new("/opt/homebrew/bin/trace-commons-contributor");
        assert_eq!(classify(p), InstallSource::Unrecognized);
    }

    #[test]
    fn a_distro_package_path_is_unrecognized() {
        // Distribution packages in /usr/bin are not ours to replace.
        let p = Path::new("/usr/bin/trace-commons-contributor");
        assert_eq!(classify(p), InstallSource::Unrecognized);
    }

    #[test]
    fn a_custom_install_sh_dir_is_unrecognized() {
        // An install.sh --dir /opt/tc copy is Unrecognized and defers.
        // This is a false negative, but it is the safe direction.
        let p = Path::new("/opt/tc/trace-commons-contributor");
        assert_eq!(classify(p), InstallSource::Unrecognized);
    }
}
