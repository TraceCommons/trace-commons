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

pub mod autostart;
pub mod backend;
pub mod copy;
pub mod model;
pub mod notify;
pub mod original_search;
pub mod placeholders;
pub mod portal;
pub mod queue_folders;
pub mod redaction_labels;
pub mod redaction_summary;
pub mod toast;
pub mod transcript_paging;
pub mod tray;
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
