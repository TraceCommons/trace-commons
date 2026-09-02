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

/// Emitted by the macOS screen once it offers a Gemini row: Claude watched,
/// Codex declined, Gemini watched.
///
/// Pasted from what `SessionRoots.settingsJSON()` actually printed, per this
/// file's rule -- including `JSONSerialization`'s escaped forward slashes and
/// its key order, neither of which is what a hand-written literal would have
/// guessed. Key order is not stable across processes and does not matter to
/// the parser; the escaping is what this pinning is really for.
const WITH_GEMINI_WATCHED: &str = r#"{"gemini_source":{"mode":"watch","path":"\/Users\/someone\/.gemini\/tmp"},"claude_source":{"path":"\/Users\/someone\/.claude\/projects","mode":"watch"},"codex_source":{"mode":"off"}}"#;

/// The same screen when the contributor says they do not use Gemini.
const WITH_GEMINI_DECLINED: &str = r#"{"gemini_source":{"mode":"off"},"claude_source":{"path":"\/Users\/someone\/.claude\/projects","mode":"watch"},"codex_source":{"mode":"off"}}"#;

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

#[test]
fn the_macos_gemini_payload_is_accepted_and_carries_the_declaration() {
    // The macOS screen did not offer a Gemini row at all until this change,
    // so nothing here had ever exercised the payload it now sends. The
    // escaped forward slashes come from `JSONSerialization` and have to
    // survive into a real path.
    let settings = apply(WITH_GEMINI_WATCHED);

    assert_eq!(
        settings.gemini_source,
        Some(SourceDeclaration::Watch {
            path: "/Users/someone/.gemini/tmp".into()
        })
    );
    assert_eq!(
        settings.claude_source,
        Some(SourceDeclaration::Watch {
            path: "/Users/someone/.claude/projects".into()
        })
    );
    assert_eq!(settings.codex_source, Some(SourceDeclaration::Off));
}

#[test]
fn declining_gemini_is_recorded_as_off_not_as_absent() {
    let settings = apply(WITH_GEMINI_DECLINED);
    assert_eq!(settings.gemini_source, Some(SourceDeclaration::Off));
}

#[test]
fn a_gemini_answer_does_not_change_whether_the_roots_are_declared() {
    // `roots_declared` stays two-conjunct on purpose: an absent Gemini
    // declaration constructs no adapter, so requiring one would re-onboard
    // every contributor upgrading from a build that never asked. Answering
    // it must not be what clears the refusal, and neither must leaving it
    // blank be what causes one.
    assert!(roots_declared(&apply(WATCH_AND_OFF)));
    assert!(roots_declared(&apply(WITH_GEMINI_WATCHED)));
    assert!(roots_declared(&apply(WITH_GEMINI_DECLINED)));
}
