//! The words of the Settings window's General section that the Mac spells
//! in Swift (`SettingsWords`, macos/Sources/TraceCommonsApp/Views/
//! SettingsSections.swift:191-196): the section's name and the appearance
//! choice's three values. The Windows app draws General from these
//! (Windows parity spec W17-D1 (a)), so the words reach it over the C ABI
//! (`tc_settings_general_copy_json`) rather than being typed in C#.
//!
//! The heading is the approved `appearance_copy::APPEARANCE_HEADING`. The
//! other four are the Mac's words moved here, DRAFT, NEEDS APPROVAL as core
//! words.

use crate::appearance_copy::APPEARANCE_HEADING;

/// The General section's words, as one JSON object over the C ABI.
#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct SettingsGeneralCopy {
    /// The section's name in the list and over its body. DRAFT, NEEDS APPROVAL.
    pub general: &'static str,
    /// The appearance choice's heading and accessible name (approved
    /// 2026-11-17 in `appearance_copy`).
    pub appearance: &'static str,
    /// Always light. DRAFT, NEEDS APPROVAL.
    pub light: &'static str,
    /// Always dark. DRAFT, NEEDS APPROVAL.
    pub dark: &'static str,
    /// Follow the system's choice. DRAFT, NEEDS APPROVAL.
    pub system: &'static str,
}

/// The General section's words.
pub fn settings_general_copy() -> SettingsGeneralCopy {
    SettingsGeneralCopy {
        general: "General",
        appearance: APPEARANCE_HEADING,
        light: "Light",
        dark: "Dark",
        system: "System",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Mac's literals (SettingsSections.swift:191-196), word for word.
    #[test]
    fn the_words_are_the_macs() {
        let copy = settings_general_copy();
        assert_eq!(copy.general, "General");
        assert_eq!(copy.appearance, "Appearance");
        assert_eq!(copy.light, "Light");
        assert_eq!(copy.dark, "Dark");
        assert_eq!(copy.system, "System");
    }

    #[test]
    fn the_json_names_every_field() {
        let json = serde_json::to_value(settings_general_copy()).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys.len(), 5);
        for key in ["general", "appearance", "light", "dark", "system"] {
            assert!(keys.contains(&key), "{key}");
        }
    }
}
