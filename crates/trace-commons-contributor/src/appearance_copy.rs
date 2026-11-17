//! The Linux app's Appearance choice: Standard or the Omarchy skin.
//!
//! Only the GTK shell offers this today (spec
//! `docs/superpowers/specs/2026-11-17-linux-omarchy-skin-design.md`). The
//! words still live here rather than in the shell, so a macOS or Windows
//! counterpart reads the same ones. Plain constants, deliberately outside
//! `shell_words_copy::ShellWordsCopy` and the C ABI: nothing that decodes
//! those payloads changes.
//!
//! All four entries were approved by the owner on 2026-11-17.

/// Approved 2026-11-17. Heading of the Appearance card in Settings, and
/// the accessible name of its choice.
pub const APPEARANCE_HEADING: &str = "Appearance";

/// Approved 2026-11-17. The choice's value for the app's own colours.
pub const APPEARANCE_STANDARD: &str = "Standard";

/// Approved 2026-11-17. The choice's value for the active Omarchy theme's
/// colours. Omarchy is a product name.
pub const APPEARANCE_OMARCHY: &str = "Omarchy";

/// Approved 2026-11-17. Shown under the choice only while Omarchy is
/// chosen and no valid Omarchy palette has been read since launch: the
/// theme is missing or its palette is malformed, hence "usable".
pub const APPEARANCE_OMARCHY_UNAVAILABLE: &str =
    "No usable Omarchy theme found. Using the standard appearance.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choice_has_two_distinct_short_values() {
        assert_ne!(APPEARANCE_STANDARD, APPEARANCE_OMARCHY);
        for value in [APPEARANCE_STANDARD, APPEARANCE_OMARCHY] {
            assert_eq!(value.split_whitespace().count(), 1, "{value}");
        }
        assert!(APPEARANCE_OMARCHY_UNAVAILABLE.contains(APPEARANCE_OMARCHY));
    }

    /// The four strings as the owner approved them on 2026-11-17. The
    /// fallback line says "usable" because it shows for a theme that is
    /// missing and for one that is malformed.
    #[test]
    fn the_words_are_the_approved_ones() {
        assert_eq!(APPEARANCE_HEADING, "Appearance");
        assert_eq!(APPEARANCE_STANDARD, "Standard");
        assert_eq!(APPEARANCE_OMARCHY, "Omarchy");
        assert_eq!(
            APPEARANCE_OMARCHY_UNAVAILABLE,
            "No usable Omarchy theme found. Using the standard appearance."
        );
        assert!(!include_str!("appearance_copy.rs").contains(&["DRAFT", "NEEDS"].join(", ")));
    }
}
