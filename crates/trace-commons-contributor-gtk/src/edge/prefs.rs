//! The edge rail's switch, kept across launches.
//!
//! The Mac's `EdgeRailPreference`: off until the contributor turns it on in
//! Settings, under the defaults key `edge_rail_enabled`. A shell
//! preference, not a daemon setting -- the rail only reads, and nothing the
//! core does depends on it being shown. This shell has no preferences
//! database, so it is one small JSON file beside `monitor-window.json`
//! (`ui::monitor::prefs`), with the Mac's key as its field. Unreadable,
//! absent or damaged is off; a failed write is dropped.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EdgePrefs {
    pub edge_rail_enabled: bool,
}

impl EdgePrefs {
    pub fn from_bytes(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
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

/// `TC_EDGE_RAIL_STATE` names the file for a test or a capture run, so
/// neither reads nor writes the person's own.
fn file() -> Option<std::path::PathBuf> {
    if let Some(path) = std::env::var_os("TC_EDGE_RAIL_STATE") {
        return Some(std::path::PathBuf::from(path));
    }
    if cfg!(test) {
        return None;
    }
    crate::shell_state_file("edge-rail.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rail_is_off_until_turned_on() {
        assert!(!EdgePrefs::default().edge_rail_enabled);
        assert!(!EdgePrefs::from_bytes(b"").edge_rail_enabled);
        assert!(!EdgePrefs::from_bytes(b"not json").edge_rail_enabled);
        assert!(!EdgePrefs::from_bytes(b"{}").edge_rail_enabled);
    }

    #[test]
    fn the_choice_is_remembered_under_the_macs_key() {
        let on = EdgePrefs {
            edge_rail_enabled: true,
        };
        let bytes = serde_json::to_vec(&on).unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"edge_rail_enabled":true}"#
        );
        assert_eq!(EdgePrefs::from_bytes(&bytes), on);
        let off = EdgePrefs::from_bytes(br#"{"edge_rail_enabled":false}"#);
        assert!(!off.edge_rail_enabled);
    }

    #[test]
    fn a_test_build_never_touches_the_persons_file() {
        if std::env::var_os("TC_EDGE_RAIL_STATE").is_none() {
            assert_eq!(file(), None);
        }
    }
}
