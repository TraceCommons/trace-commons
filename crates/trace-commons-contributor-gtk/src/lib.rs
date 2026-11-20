//! The Linux contributor shell.
//!
//! GTK 4 and libadwaita, linking `trace-commons-contributor` directly rather
//! than through the C ABI: on this platform the shell and the core are both
//! Rust, so the FFI boundary would buy nothing.
//!
//! Two things about this shell differ from its macOS and Windows siblings,
//! and both are deliberate:
//!
//! * **The window is the primary surface.** GNOME, the majority desktop, has
//!   no system tray without a user-installed extension, so a tray-first
//!   design would be invisible to most people it shipped to. Notification
//!   actions do the work a tray menu does elsewhere. A `StatusNotifierItem`
//!   tray is a bonus where it is real; nothing here requires it, and nothing
//!   here ever tells a contributor to install a shell extension.
//! * **The daemon is the primary deployment**, under the systemd user unit,
//!   and this application is an optional client over its socket. It can also
//!   host the loop for someone who wants only the app. See `backend`.

pub mod attestation;
pub mod autostart;
pub mod backend;
pub mod balance;
// Recorded sample data in place of a daemon; never in a release build.
pub mod certificate;
pub mod copy;
pub mod credit_arrival;
mod dbus;
pub mod disclosure;
pub mod document_portal;
pub mod edge;
pub mod eligibility;
pub mod feature_flags;
pub mod first_run;
pub mod home;
pub mod home_cards;
pub mod model;
pub mod notify;
pub mod original_search;
pub mod payout_link;
pub mod placeholders;
pub mod portal;
pub mod queue_folders;
pub mod redaction_labels;
pub mod redaction_summary;
#[cfg(debug_assertions)]
pub mod sample;
pub mod service;
pub mod shield;
pub mod toast;
pub mod transcript_paging;
pub mod tray;
pub mod tray_icon;
pub mod ui;
pub mod update;
pub mod worker;

/// Resolve the contributor state directory the same way the CLI does, so
/// the app and `trace-commons-contributor daemon ...` always talk about the
/// same daemon.
pub fn state_dir() -> anyhow::Result<std::path::PathBuf> {
    Ok(
        trace_commons_contributor::config::ConfigStore::resolve(None)?
            .dir()
            .to_path_buf(),
    )
}

/// Where the shell keeps one of its own small files -- the window's panes
/// (`monitor-window.json`), the rail's switch, the appearance, the Home
/// cards, the onboarding and legacy-move markers: `trace-commons/<name>`
/// under the state directory. On sample data (debug builds only) it is in
/// the run's throwaway directory instead, the one `main` gives the window's
/// direct `ConfigStore` reads, so a sample launch neither reads nor writes
/// the person's own files. Each caller's own override variable and its
/// test-build guard come first.
pub fn shell_state_file(name: &str) -> Option<std::path::PathBuf> {
    shell_state_file_in(
        name,
        sample_requested(),
        dirs::state_dir().or_else(dirs::data_local_dir),
    )
}

fn shell_state_file_in(
    name: &str,
    sample: bool,
    root: Option<std::path::PathBuf>,
) -> Option<std::path::PathBuf> {
    if sample {
        return Some(sample_scratch_dir().join(name));
    }
    root.map(|dir| dir.join("trace-commons").join(name))
}

/// The throwaway directory a sample launch keeps its state in, one per
/// process.
pub fn sample_scratch_dir() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("trace-commons-sample-{}", std::process::id()))
}

fn sample_requested() -> bool {
    #[cfg(debug_assertions)]
    {
        sample::requested()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[cfg(test)]
mod shell_state_tests {
    use super::*;

    #[test]
    fn a_sample_launch_keeps_its_window_state_out_of_the_persons_directory() {
        let real = std::path::PathBuf::from("/home/someone/.local/state");
        let path = shell_state_file_in("monitor-window.json", true, Some(real.clone())).unwrap();
        assert!(
            !path.starts_with(&real),
            "sample mode wrote under the person's state directory: {}",
            path.display()
        );
        assert_eq!(path, sample_scratch_dir().join("monitor-window.json"));
    }

    #[test]
    fn a_normal_launch_keeps_it_under_the_state_directory() {
        let real = std::path::PathBuf::from("/home/someone/.local/state");
        assert_eq!(
            shell_state_file_in("monitor-window.json", false, Some(real.clone())),
            Some(real.join("trace-commons").join("monitor-window.json"))
        );
        assert_eq!(
            shell_state_file_in("monitor-window.json", false, None),
            None
        );
    }
}
