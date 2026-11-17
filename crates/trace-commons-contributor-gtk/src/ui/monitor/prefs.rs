//! The window's pane choices, kept across launches.
//!
//! The Mac keeps these in the person's defaults (`monitor.showsMap`,
//! `monitor.showsInspector`, `monitor.mapWidth`, `monitor.mainWidth`,
//! `monitor.panesSeeded`, `monitor.showsGraph`). This shell has no
//! preferences database: what it remembers about its own window it keeps as
//! one small JSON file beside `onboarded.json`
//! (`ui::onboarding::completion_file`) and `legacy-move-dismissed.json`, and
//! this is a third file in that directory. Nothing here is the daemon's
//! setting and nothing is written to `daemon-settings.json`.
//!
//! Unreadable, absent or damaged is the defaults, unseeded; a failed write
//! is dropped. Neither can keep the window from opening.

use super::pane_layout::{self, Visibility};

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PanePrefs {
    pub shows_map: bool,
    pub shows_inspector: bool,
    pub shows_graph: bool,
    pub map_width: f64,
    pub main_width: f64,
    /// False until the window has seeded the two panes from the room its
    /// monitor has. After that the person's own choice is restored.
    pub seeded: bool,
}

impl Default for PanePrefs {
    fn default() -> Self {
        Self {
            shows_map: true,
            shows_inspector: true,
            shows_graph: true,
            map_width: pane_layout::MAP_WIDTH,
            main_width: pane_layout::MAIN_WIDTH,
            seeded: false,
        }
    }
}

impl PanePrefs {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        serde_json::from_slice::<Self>(bytes)
            .unwrap_or_default()
            .sanitized()
    }

    /// Widths a file could not have got from this window are the defaults:
    /// not finite, or under the pane's floor.
    fn sanitized(mut self) -> Self {
        if !self.map_width.is_finite() || self.map_width < pane_layout::MAP_MIN_WIDTH {
            self.map_width = pane_layout::MAP_WIDTH;
        }
        if !self.main_width.is_finite() || self.main_width < pane_layout::MAIN_WIDTH {
            self.main_width = pane_layout::MAIN_WIDTH;
        }
        self
    }

    /// Seeds the two panes from the monitor's width, once. `demanded` keeps
    /// a narrow first layout from closing the inspector over something it
    /// must show. Returns whether anything was seeded.
    pub fn seed(&mut self, monitor_width: f64, demanded: bool) -> bool {
        if self.seeded {
            return false;
        }
        let Visibility { map, inspector } = pane_layout::first_launch(monitor_width);
        self.shows_map = map;
        self.shows_inspector = inspector || demanded;
        self.seeded = true;
        true
    }

    pub fn load() -> Self {
        file()
            .and_then(|path| std::fs::read(path).ok())
            .map(|bytes| Self::from_bytes(&bytes))
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let Some(path) = file() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_vec(self) {
            let _ = std::fs::write(path, json);
        }
    }
}

/// `TC_MONITOR_WINDOW_STATE` names the file for a test or a capture run, so
/// neither reads nor writes the person's own.
fn file() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("TC_MONITOR_WINDOW_STATE") {
        return Some(std::path::PathBuf::from(path));
    }
    // A test build never reads or writes the person's own file.
    if cfg!(test) {
        return None;
    }
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|dir| dir.join("trace-commons").join("monitor-window.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_on_disk_is_every_pane_at_its_default_and_unseeded() {
        let prefs = PanePrefs::from_bytes(b"");
        assert_eq!(prefs, PanePrefs::default());
        assert!(prefs.shows_map && prefs.shows_inspector && prefs.shows_graph);
        assert_eq!(prefs.map_width, pane_layout::MAP_WIDTH);
        assert_eq!(prefs.main_width, pane_layout::MAIN_WIDTH);
        assert!(!prefs.seeded);
        assert_eq!(PanePrefs::from_bytes(b"not json"), PanePrefs::default());
    }

    #[test]
    fn the_persons_choices_round_trip() {
        let chosen = PanePrefs {
            shows_map: false,
            shows_inspector: true,
            shows_graph: false,
            map_width: 620.0,
            main_width: 512.0,
            seeded: true,
        };
        let bytes = serde_json::to_vec(&chosen).unwrap();
        assert_eq!(PanePrefs::from_bytes(&bytes), chosen);
    }

    #[test]
    fn a_file_from_an_older_build_keeps_what_it_has() {
        let prefs = PanePrefs::from_bytes(br#"{"shows_map":false,"seeded":true}"#);
        assert!(!prefs.shows_map);
        assert!(prefs.seeded);
        assert!(prefs.shows_inspector);
        assert_eq!(prefs.map_width, pane_layout::MAP_WIDTH);
    }

    #[test]
    fn a_width_under_a_panes_floor_is_the_default() {
        let prefs = PanePrefs::from_bytes(br#"{"map_width":12,"main_width":-4}"#);
        assert_eq!(prefs.map_width, pane_layout::MAP_WIDTH);
        assert_eq!(prefs.main_width, pane_layout::MAIN_WIDTH);
    }

    #[test]
    fn the_first_launch_seeds_from_the_monitor_and_only_once() {
        let mut prefs = PanePrefs::default();
        assert!(prefs.seed(1000.0, false));
        assert!(!prefs.shows_map, "under 1100 there is no room for the map");
        assert!(prefs.shows_inspector);
        assert!(prefs.seeded);
        // The person's later choice is theirs: a wider monitor reseeds nothing.
        prefs.shows_inspector = false;
        assert!(!prefs.seed(2560.0, false));
        assert!(!prefs.shows_map);
        assert!(!prefs.shows_inspector);

        let mut narrow = PanePrefs::default();
        narrow.seed(800.0, false);
        assert!(!narrow.shows_map && !narrow.shows_inspector);
        let mut wide = PanePrefs::default();
        wide.seed(1920.0, false);
        assert!(wide.shows_map && wide.shows_inspector);
    }

    #[test]
    fn a_narrow_first_layout_never_closes_the_inspector_over_a_demand() {
        let mut prefs = PanePrefs::default();
        prefs.seed(800.0, true);
        assert!(prefs.shows_inspector);
        assert!(!prefs.shows_map);
    }
}
