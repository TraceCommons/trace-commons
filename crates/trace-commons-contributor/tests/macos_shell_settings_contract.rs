//! The exact settings JSON the macOS roots screen sends, run through the
//! Rust that has to accept it.
//!
//! Two implementations in two languages have to agree on one wire shape, and
//! nothing else checks that they do. The Swift side is unit-tested against
//! its own idea of the shape (`macos/Tests/TCShellCoreTests/SessionRootsTests.swift`)
//! and the Rust side against its own; both suites can be green while the
//! string one produces is a string the other rejects. So the payloads below
//! are pasted from what `SessionRoots.settingsJSON()` actually emits, not
//! rebuilt here from the same understanding that might be wrong.
//!
//! If this file fails after a change to either side, the fix is to make them
//! agree -- not to update the literals until it passes.

use trace_commons_contributor::daemon::settings::{
    DaemonSettings, SourceDeclaration, apply_settings_object, roots_declared,
};

/// Apply a settings object the way `tc_daemon_start_with_settings` does.
fn apply(json: &str) -> DaemonSettings {
    let value: serde_json::Value = serde_json::from_str(json).expect("payload is valid JSON");
    let mut settings = DaemonSettings::default();
    apply_settings_object(&mut settings, &value).expect("the Rust side must accept this payload");
    settings
}

/// Emitted by the macOS screen when the contributor watches the discovered
/// Claude store and declines Codex. Verified end to end on 2026-08-19: this
/// is what landed in `daemon-settings.json` and the daemon started on it.
const WATCH_AND_OFF: &str = r#"{"claude_source":{"mode":"watch","path":"/Users/someone/.claude/projects"},"codex_source":{"mode":"off"}}"#;

const BOTH_WATCHED: &str = r#"{"claude_source":{"mode":"watch","path":"/Users/someone/.claude/projects"},"codex_source":{"mode":"watch","path":"/Users/someone/.codex/sessions"}}"#;

const BOTH_OFF: &str = r#"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#;

#[test]
fn the_macos_watch_and_decline_payload_is_accepted_and_clears_the_refusal() {
    let settings = apply(WATCH_AND_OFF);

    assert_eq!(
        settings.claude_source,
        Some(SourceDeclaration::Watch {
            path: "/Users/someone/.claude/projects".into()
        })
    );
    assert_eq!(settings.codex_source, Some(SourceDeclaration::Off));
    assert!(
        roots_declared(&settings),
        "declining a source is an answer; the daemon must be startable on it"
    );
}

#[test]
fn declining_both_still_counts_as_declared() {
    let settings = apply(BOTH_OFF);

    assert!(
        roots_declared(&settings),
        "a contributor who uses neither agent has answered, and must not be refused forever"
    );
    // And it must genuinely watch nothing rather than falling back.
    assert_eq!(settings.claude_source, Some(SourceDeclaration::Off));
    assert_eq!(settings.codex_source, Some(SourceDeclaration::Off));
}

#[test]
fn watching_both_is_accepted() {
    let settings = apply(BOTH_WATCHED);
    assert!(roots_declared(&settings));
}

#[test]
fn the_off_declaration_never_resolves_to_a_path() {
    let settings = apply(WATCH_AND_OFF);
    // The whole point of the state: `Off` must not degrade into the
    // conventional location the way an absent key does.
    assert_eq!(
        settings
            .codex_source
            .as_ref()
            .and_then(SourceDeclaration::path),
        None
    );
}

#[test]
fn a_half_answered_screen_would_not_clear_the_refusal() {
    // The screen will not send this -- Continue stays disabled -- but if a
    // future change let it through, the refusal is the backstop and it has
    // to still be there.
    let settings = apply(r#"{"claude_source":{"mode":"watch","path":"/p"}}"#);
    assert!(!roots_declared(&settings));
}
