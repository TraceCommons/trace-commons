//! Which of the two autostart mechanisms is in force, and never both.
//!
//! `trace-commons-contributor daemon install` already writes a systemd user
//! unit (`crates/trace-commons-contributor/src/daemon/install.rs`). This
//! module only *detects* that file; it never writes or removes it -- install
//! and uninstall stay `daemon install` / `daemon uninstall`, run from the
//! CLI, so there is exactly one writer for that path.
//!
//! When no unit is installed, this module owns a second, XDG-standard
//! mechanism: a `.desktop` file under `~/.config/autostart`. The two must
//! never coexist. Shipping both is exactly how a contributor ends up with
//! two running copies of the daemon and a confusing lock error -- the
//! Linux design spec names this failure mode explicitly. So `detect()` is
//! the single source of truth: the app writes the XDG entry only when
//! detection says no systemd unit exists, and Settings never asks with a
//! toggle for the mechanism it did not detect.
//!
//! The systemd unit's exact filename is duplicated here rather than
//! imported, because `trace-commons-contributor::daemon::install` keeps it
//! private -- there is no public constant or "is it installed" query to
//! reuse. That is a minor seam, not a contract gap: detecting a file that a
//! well-known, documented path convention names does not need the writer's
//! cooperation. See the report for the fuller list of what would need a
//! contract change instead.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

/// Must match `UNIT_FILE_NAME` in
/// `trace-commons-contributor/src/daemon/install.rs`.
const SYSTEMD_UNIT_FILE_NAME: &str = "trace-commons-contributor.service";

/// This application's own autostart entry, named after the GTK app id
/// rather than the CLI's unit name so the two are never confused for one
/// another on disk.
const XDG_AUTOSTART_FILE_NAME: &str = "ai.tracecommons.Contributor.desktop";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mechanism {
    /// `daemon install` has already been run. Settings shows this as a
    /// fact, not a toggle: there is nothing for the app to switch.
    SystemdUnit,
    /// No unit was found. The app may offer its own XDG autostart entry,
    /// and `enabled` reflects whether it currently has one.
    XdgEntry { enabled: bool },
}

fn systemd_unit_path(config_dir: &Path) -> PathBuf {
    config_dir.join("systemd/user").join(SYSTEMD_UNIT_FILE_NAME)
}

fn xdg_autostart_path(config_dir: &Path) -> PathBuf {
    config_dir.join("autostart").join(XDG_AUTOSTART_FILE_NAME)
}

/// Real detection, against the real config directory.
pub fn detect() -> Mechanism {
    match dirs::config_dir() {
        Some(dir) => detect_in(&dir),
        // No resolvable config directory at all is stranger than either
        // mechanism failing; treat it the same as "no unit, no entry" so
        // Settings can still offer the XDG path rather than going blank.
        None => Mechanism::XdgEntry { enabled: false },
    }
}

fn detect_in(config_dir: &Path) -> Mechanism {
    if systemd_unit_path(config_dir).exists() {
        return Mechanism::SystemdUnit;
    }
    Mechanism::XdgEntry {
        enabled: xdg_autostart_path(config_dir).exists(),
    }
}

/// Write the XDG autostart entry for this application. Callers are
/// responsible for having checked `detect()` first -- this does not
/// re-check, so calling it while a systemd unit is installed would create
/// exactly the two-mechanism situation this module exists to prevent.
pub fn enable_xdg_entry() -> Result<()> {
    let config_dir = dirs::config_dir().ok_or_else(|| anyhow!("no-config-directory"))?;
    enable_xdg_entry_in(&config_dir)
}

fn enable_xdg_entry_in(config_dir: &Path) -> Result<()> {
    let path = xdg_autostart_path(config_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let exe = std::env::current_exe().context("finding this executable")?;
    let entry = desktop_entry_text(&exe);
    std::fs::write(&path, entry).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

pub fn disable_xdg_entry() -> Result<()> {
    let config_dir = dirs::config_dir().ok_or_else(|| anyhow!("no-config-directory"))?;
    disable_xdg_entry_in(&config_dir)
}

fn disable_xdg_entry_in(config_dir: &Path) -> Result<()> {
    let path = xdg_autostart_path(config_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
    }
}

fn desktop_entry_text(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={}\n\
         Comment=Review and contribute Claude Code and Codex sessions in the background\n\
         Exec=\"{}\"\n\
         Terminal=false\n\
         NoDisplay=false\n\
         X-GNOME-Autostart-enabled=true\n",
        crate::copy::APP_NAME,
        exe.display(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_unit_and_no_entry_reads_as_xdg_disabled() {
        let dir = tempfile();
        assert_eq!(detect_in(&dir), Mechanism::XdgEntry { enabled: false });
    }

    #[test]
    fn a_systemd_unit_wins_even_if_an_xdg_entry_also_exists() {
        let dir = tempfile();
        std::fs::create_dir_all(dir.join("systemd/user")).unwrap();
        std::fs::write(
            systemd_unit_path(&dir),
            "[Unit]\nDescription=x\n[Service]\nExecStart=x\n",
        )
        .unwrap();
        enable_xdg_entry_in(&dir).unwrap();
        assert_eq!(detect_in(&dir), Mechanism::SystemdUnit);
    }

    #[test]
    fn enabling_then_disabling_the_xdg_entry_round_trips() {
        let dir = tempfile();
        assert_eq!(detect_in(&dir), Mechanism::XdgEntry { enabled: false });
        enable_xdg_entry_in(&dir).unwrap();
        assert_eq!(detect_in(&dir), Mechanism::XdgEntry { enabled: true });
        disable_xdg_entry_in(&dir).unwrap();
        assert_eq!(detect_in(&dir), Mechanism::XdgEntry { enabled: false });
    }

    #[test]
    fn disabling_an_entry_that_was_never_written_is_not_an_error() {
        let dir = tempfile();
        disable_xdg_entry_in(&dir).unwrap();
    }

    #[test]
    fn the_written_entry_names_this_executable_and_never_a_relative_path() {
        let dir = tempfile();
        enable_xdg_entry_in(&dir).unwrap();
        let text = std::fs::read_to_string(xdg_autostart_path(&dir)).unwrap();
        assert!(text.contains("Type=Application"));
        assert!(text.contains("X-GNOME-Autostart-enabled=true"));
        let exe = std::env::current_exe().unwrap();
        assert!(text.contains(&exe.display().to_string()));
    }

    /// A throwaway directory, cleaned up when it drops. Std-only: this
    /// module has no other reason to take a `tempfile`-family dependency.
    fn tempfile() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "trace-commons-shell-autostart-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
