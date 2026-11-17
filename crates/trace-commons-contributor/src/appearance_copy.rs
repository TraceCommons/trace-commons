//! The Linux app's Appearance choice: Standard or the Omarchy skin.
//!
//! Only the GTK shell offers this today (spec
//! `docs/superpowers/specs/2026-11-17-linux-omarchy-skin-design.md`). The
//! words still live here rather than in the shell, so a macOS or Windows
//! counterpart reads the same ones. Plain constants, deliberately outside
//! `shell_words_copy::ShellWordsCopy` and the C ABI: nothing that decodes
//! those payloads changes.
//!
//! Every entry is DRAFT, NEEDS APPROVAL. None is approved by the owner yet.

/// DRAFT, NEEDS APPROVAL. Heading of the Appearance card in Settings, and
/// the accessible name of its choice.
pub const APPEARANCE_HEADING: &str = "Appearance";

/// DRAFT, NEEDS APPROVAL. The choice's value for the app's own colours.
pub const APPEARANCE_STANDARD: &str = "Standard";

/// DRAFT, NEEDS APPROVAL. The choice's value for the active Omarchy theme's
/// colours. Omarchy is a product name.
pub const APPEARANCE_OMARCHY: &str = "Omarchy";

/// DRAFT, NEEDS APPROVAL. Shown under the choice only while Omarchy is
/// chosen and no valid Omarchy palette has been read since launch.
pub const APPEARANCE_OMARCHY_UNAVAILABLE: &str =
    "No Omarchy theme found. Using the standard appearance.";

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
}
