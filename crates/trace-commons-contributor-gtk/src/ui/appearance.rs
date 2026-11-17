//! The Appearance choice -- Standard or the Omarchy skin -- and, while the
//! skin is on, keeping its palette in step with the active Omarchy theme.
//!
//! The palette itself is read and checked in [`super::omarchy`]; the CSS and
//! the drawing inks are made from it in [`super::style`]. This module owns
//! the state between them: the person's choice, the last valid palette, the
//! file monitors and the reload. Spec:
//! `docs/superpowers/specs/2026-11-17-linux-omarchy-skin-design.md`.
//!
//! The choice is the shell's own, kept as one small JSON file beside the
//! window's `monitor-window.json` (see `monitor::prefs`). It is not a daemon
//! setting, and nothing here changes what the daemon does.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use adw::prelude::*;

use super::omarchy::{self, Offered, Palette, Retained};

/// The class every toplevel carries while the skin is on. Rules only the
/// skin has are scoped under it.
pub const SKIN_CLASS: &str = "tc-omarchy";

/// How long a burst of file events is let settle before the palette is
/// read again. A theme switch is an `rm -rf` and a `mv` a few milliseconds
/// apart; reading between them would see no theme at all.
pub const DEBOUNCE: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    #[default]
    Standard,
    Omarchy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AppearancePrefs {
    pub appearance: Appearance,
}

impl AppearancePrefs {
    /// Unreadable, absent or damaged is Standard.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Self {
        std::fs::read(path)
            .map(|bytes| Self::from_bytes(&bytes))
            .unwrap_or_default()
    }

    /// A failed write is dropped: it cannot keep the window from working.
    pub fn save_to(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// `TC_APPEARANCE_STATE` names the file for a test or a capture run, so
/// neither reads nor writes the person's own.
fn prefs_file() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("TC_APPEARANCE_STATE") {
        return Some(PathBuf::from(path));
    }
    if cfg!(test) {
        return None;
    }
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|dir| dir.join("trace-commons").join("appearance.json"))
}

/// The development hook `TRACE_COMMONS_APPEARANCE` (see `style::install`):
/// `omarchy` turns the skin on for a capture run without touching the
/// person's file. `dark` and `light` are the stylesheet's, not this one's.
pub fn env_override(value: Option<&str>) -> Option<Appearance> {
    (value == Some("omarchy")).then_some(Appearance::Omarchy)
}

/// What to watch so a theme switch, an edit to the palette, or an Omarchy
/// installed after launch is noticed: each candidate's `current/` directory
/// -- the theme directory itself is deleted and replaced on a switch, so a
/// monitor on it would die with it -- or, while that does not exist yet, its
/// nearest existing ancestor up to the candidate's base directory; and the
/// resolved palette file itself, for edits in place.
pub fn watch_targets(
    dirs: &[PathBuf],
    resolved: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut targets: Vec<PathBuf> = Vec::new();
    for theme in dirs {
        // theme = <base>/omarchy/current/theme: try current, omarchy, base.
        let found = theme.ancestors().skip(1).take(3).find(|dir| exists(dir));
        if let Some(dir) = found {
            let dir = dir.to_path_buf();
            if !targets.contains(&dir) {
                targets.push(dir);
            }
        }
    }
    if let Some(dir) = resolved {
        targets.push(dir.join("colors.toml"));
    }
    targets
}

/// Only the newest scheduled re-read runs.
#[derive(Debug, Default)]
pub struct Debounce {
    generation: u64,
}

impl Debounce {
    pub fn bump(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.generation == generation
    }
}

#[derive(Default)]
struct State {
    chosen: Appearance,
    retained: Retained,
    monitors: Vec<gtk::gio::FileMonitor>,
    debounce: Debounce,
    listeners: Vec<Box<dyn Fn()>>,
    tagging: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// The palette the skin is drawing with, if the skin is on and has one.
pub fn active_palette() -> Option<Palette> {
    STATE.with(|state| {
        let state = state.borrow();
        (state.chosen == Appearance::Omarchy)
            .then(|| state.retained.current().cloned())
            .flatten()
    })
}

pub fn chosen() -> Appearance {
    STATE.with(|state| state.borrow().chosen)
}

/// Whether Appearance should explain that the standard look is showing
/// although Omarchy is chosen: no valid palette has been read since launch.
pub fn fallback_showing() -> bool {
    STATE.with(|state| {
        let state = state.borrow();
        state.chosen == Appearance::Omarchy && state.retained.current().is_none()
    })
}

/// Call `f` after every change of choice or palette. For the Settings card;
/// the handlers live as long as the application.
pub fn connect_changed(f: impl Fn() + 'static) {
    STATE.with(|state| state.borrow_mut().listeners.push(Box::new(f)));
}

/// Read the choice and, if it is Omarchy, turn the skin on. Called once by
/// `style::install` after the provider exists.
pub(super) fn start() {
    let env = std::env::var("TRACE_COMMONS_APPEARANCE").ok();
    let choice = env_override(env.as_deref()).unwrap_or_else(|| {
        prefs_file()
            .map(|path| AppearancePrefs::load_from(&path).appearance)
            .unwrap_or_default()
    });
    apply(choice);
}

/// The person chose. Saved, then applied.
pub fn choose(choice: Appearance) {
    if let Some(path) = prefs_file() {
        AppearancePrefs { appearance: choice }.save_to(&path);
    }
    apply(choice);
}

fn apply(choice: Appearance) {
    STATE.with(|state| state.borrow_mut().chosen = choice);
    match choice {
        Appearance::Omarchy => {
            ensure_window_tagging();
            reread();
        }
        Appearance::Standard => {
            STATE.with(|state| state.borrow_mut().monitors.clear());
            restore_scheme();
            super::style::reload();
            tag_windows(false);
            redraw_all();
            notify();
        }
    }
}

fn restore_scheme() {
    let manager = adw::StyleManager::default();
    manager.set_color_scheme(match std::env::var("TRACE_COMMONS_APPEARANCE").as_deref() {
        Ok("dark") => adw::ColorScheme::ForceDark,
        Ok("light") => adw::ColorScheme::ForceLight,
        _ => adw::ColorScheme::Default,
    });
}

/// Resolve and read the palette now, keep the last valid one on failure,
/// re-arm the monitors on whatever exists now, and reload if it changed.
fn reread() {
    if chosen() != Appearance::Omarchy {
        return;
    }
    let dirs = omarchy::theme_dirs(&omarchy::Env::from_process());
    let read = omarchy::resolve(&dirs);
    let resolved = read.as_ref().ok().map(|(dir, _)| dir.clone());
    let offered = STATE.with(|state| {
        state
            .borrow_mut()
            .retained
            .offer(read.map(|(_, palette)| palette))
    });
    arm(&watch_targets(&dirs, resolved.as_deref(), Path::exists));
    match offered {
        Offered::Replaced => {
            if let Some(palette) = active_palette() {
                adw::StyleManager::default().set_color_scheme(match palette.mode {
                    omarchy::Mode::Dark => adw::ColorScheme::ForceDark,
                    omarchy::Mode::Light => adw::ColorScheme::ForceLight,
                });
            }
            super::style::reload();
            tag_windows(true);
            redraw_all();
        }
        Offered::Unchanged | Offered::KeptLast => tag_windows(true),
        Offered::NoneYet => {
            restore_scheme();
            super::style::reload();
            tag_windows(false);
        }
    }
    notify();
}

fn arm(targets: &[PathBuf]) {
    let monitors = targets
        .iter()
        .filter_map(|path| {
            let file = gtk::gio::File::for_path(path);
            let monitor = if path.is_dir() {
                file.monitor_directory(
                    gtk::gio::FileMonitorFlags::WATCH_MOVES,
                    gtk::gio::Cancellable::NONE,
                )
            } else {
                file.monitor_file(
                    gtk::gio::FileMonitorFlags::NONE,
                    gtk::gio::Cancellable::NONE,
                )
            }
            .ok()?;
            monitor.connect_changed(|_, _, _, _| schedule());
            Some(monitor)
        })
        .collect();
    // Dropping the old monitors cancels them.
    STATE.with(|state| state.borrow_mut().monitors = monitors);
}

fn schedule() {
    let generation = STATE.with(|state| state.borrow_mut().debounce.bump());
    gtk::glib::timeout_add_local_once(DEBOUNCE, move || {
        let current = STATE.with(|state| state.borrow().debounce.is_current(generation));
        if current {
            reread();
        }
    });
}

fn notify() {
    // Taken out for the call so a listener may read the state.
    let listeners = STATE.with(|state| std::mem::take(&mut state.borrow_mut().listeners));
    for listener in &listeners {
        listener();
    }
    STATE.with(|state| {
        let mut state = state.borrow_mut();
        let added = std::mem::replace(&mut state.listeners, listeners);
        state.listeners.extend(added);
    });
}

fn tag_windows(on: bool) {
    for window in gtk::Window::list_toplevels() {
        if on {
            window.add_css_class(SKIN_CLASS);
        } else {
            window.remove_css_class(SKIN_CLASS);
        }
    }
}

/// Windows opened later -- dialogs, the preview -- get the class too.
fn ensure_window_tagging() {
    let already = STATE.with(|state| std::mem::replace(&mut state.borrow_mut().tagging, true));
    if already {
        return;
    }
    let toplevels = gtk::Window::toplevels();
    toplevels.connect_items_changed(|list, position, _, added| {
        if active_palette().is_none() {
            return;
        }
        for index in position..position + added {
            if let Some(window) = list.item(index).and_downcast::<gtk::Widget>() {
                window.add_css_class(SKIN_CLASS);
            }
        }
    });
}

/// Every drawn widget in every window draws again. A switch between two
/// dark themes changes no scheme, so the `dark` notifications the drawing
/// areas follow would not fire.
fn redraw_all() {
    fn walk(widget: &gtk::Widget) {
        widget.queue_draw();
        let mut child = widget.first_child();
        while let Some(current) = child {
            walk(&current);
            child = current.next_sibling();
        }
    }
    for window in gtk::Window::list_toplevels() {
        walk(&window);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_on_disk_or_a_damaged_file_is_standard() {
        assert_eq!(
            AppearancePrefs::from_bytes(b"").appearance,
            Appearance::Standard
        );
        assert_eq!(
            AppearancePrefs::from_bytes(b"{\"appearance\":\"neon\"}").appearance,
            Appearance::Standard
        );
        assert_eq!(
            AppearancePrefs::from_bytes(b"{\"appearance\":\"omarchy\"}").appearance,
            Appearance::Omarchy
        );
    }

    #[test]
    fn the_choice_round_trips_and_can_be_reversed() {
        let dir = std::env::temp_dir().join(format!(
            "tc-appearance-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        let path = dir.join("trace-commons").join("appearance.json");
        assert_eq!(
            AppearancePrefs::load_from(&path),
            AppearancePrefs::default()
        );
        AppearancePrefs {
            appearance: Appearance::Omarchy,
        }
        .save_to(&path);
        assert_eq!(
            AppearancePrefs::load_from(&path).appearance,
            Appearance::Omarchy
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"appearance\":\"omarchy\"}"
        );
        AppearancePrefs {
            appearance: Appearance::Standard,
        }
        .save_to(&path);
        assert_eq!(
            AppearancePrefs::load_from(&path).appearance,
            Appearance::Standard
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_test_build_never_touches_the_persons_file() {
        if std::env::var_os("TC_APPEARANCE_STATE").is_none() {
            assert_eq!(prefs_file(), None);
        }
    }

    #[test]
    fn only_omarchy_is_this_modules_override() {
        assert_eq!(env_override(Some("omarchy")), Some(Appearance::Omarchy));
        for other in [Some("dark"), Some("light"), Some(""), None] {
            assert_eq!(env_override(other), None, "{other:?}");
        }
    }

    #[test]
    fn the_watch_sits_above_the_theme_directory_a_switch_replaces() {
        let dirs = vec![
            PathBuf::from("/h/.local/state/omarchy/current/theme"),
            PathBuf::from("/h/.config/omarchy/current/theme"),
        ];
        // Omarchy 4.x installed: watch current/, and the palette in place.
        let exists = |p: &Path| p.starts_with("/h/.local/state") || p == Path::new("/h/.config");
        assert_eq!(
            watch_targets(
                &dirs,
                Some(Path::new("/h/.local/state/omarchy/current/theme")),
                exists
            ),
            vec![
                PathBuf::from("/h/.local/state/omarchy/current"),
                PathBuf::from("/h/.config"),
                PathBuf::from("/h/.local/state/omarchy/current/theme/colors.toml"),
            ]
        );
        // Nothing installed and no base directories: nothing to watch.
        assert!(watch_targets(&dirs, None, |_| false).is_empty());
        // The watch never climbs above a candidate's base directory.
        let only_home = |p: &Path| p == Path::new("/h");
        assert!(watch_targets(&dirs, None, only_home).is_empty());
    }

    #[test]
    fn only_the_newest_scheduled_reread_runs() {
        let mut debounce = Debounce::default();
        let first = debounce.bump();
        let second = debounce.bump();
        assert!(!debounce.is_current(first));
        assert!(debounce.is_current(second));
    }
}
