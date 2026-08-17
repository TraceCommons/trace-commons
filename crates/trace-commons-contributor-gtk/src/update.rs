//! Updating this application, on the one platform where the application is
//! forbidden to do it itself.
//!
//! A Flatpak-confined process must not replace its own bytes. Flatpak added
//! `org.freedesktop.portal.Flatpak.UpdateMonitor` for precisely this
//! reason -- "homegrown methods of doing so are unreliable at best, and
//! insecure at worst" -- and the portal is scoped so an application can
//! only ever update *itself*, nothing else on the system. So this module
//! asks; flatpak does the work.
//!
//! **Nothing here reads an update manifest.** The release pipeline's
//! `updates/latest.json` and `updates/appcast.xml` exist for the Windows/CLI
//! self-update path and for macOS Sparkle. The portal learns what version
//! exists from the flatpak remote the app was installed from, so there is
//! no fetch, no signature check and no sha256 check on this path -- ostree
//! and flatpak do that on the far side of the portal, where the bytes
//! actually are. Wiring a manifest in here would add a network dependency
//! that verifies nothing.
//!
//! Outside a flatpak -- a build from source -- there is no portal and no
//! remote, so this module reports [`InstallKind::Unconfined`] and the window
//! says so plainly. It never installs anything in that case, and never
//! checks anything either: there is nothing for a source build to check
//! against.

use std::path::Path;

/// Where this running copy came from, decided by a local path check and
/// nothing else. No network, in keeping with the design spec's rule that
/// install-source detection is a filesystem question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Running inside a Flatpak sandbox. The portal is the update path.
    Flatpak,
    /// Built from source and run directly. Nothing here updates it.
    Unconfined,
}

/// The file every Flatpak sandbox has and nothing outside one does. Flatpak
/// writes it into the sandbox root and it carries the instance's app id,
/// branch and commit -- which is also what the portal reads to decide what
/// this caller is allowed to update.
pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";

/// The detection, with the path injected so it is testable without a real
/// sandbox and without the test ever touching `/`.
pub fn detect_install_kind_at(flatpak_info: &Path) -> InstallKind {
    if flatpak_info.exists() {
        InstallKind::Flatpak
    } else {
        InstallKind::Unconfined
    }
}

/// The detection as production runs it.
pub fn detect_install_kind() -> InstallKind {
    detect_install_kind_at(Path::new(FLATPAK_INFO_PATH))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A private directory for one test, under the system temp dir, named
    /// so two tests in the same process never collide. Deliberately not
    /// `tempfile`: that would be a new dependency for four lines.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tc-update-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_present_flatpak_info_means_confined() {
        let dir = scratch("present");
        let marker = dir.join("flatpak-info");
        std::fs::write(
            &marker,
            b"[Application]\nname=ai.tracecommons.Contributor\n",
        )
        .unwrap();
        assert_eq!(detect_install_kind_at(&marker), InstallKind::Flatpak);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_absent_flatpak_info_means_a_source_build() {
        let dir = scratch("absent");
        let marker = dir.join("flatpak-info");
        assert_eq!(detect_install_kind_at(&marker), InstallKind::Unconfined);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_production_path_is_the_sandbox_root_marker() {
        // The constant is the contract with the flatpak runtime; a typo in
        // it would make every confined run silently report Unconfined and
        // no other test would notice.
        assert_eq!(FLATPAK_INFO_PATH, "/.flatpak-info");
    }
}
