//! A test build never resolves the developer's real home directory for
//! session sources.
//!
//! Undeclared Claude Code and Codex roots fall back to `~/.claude/projects`
//! and `~/.codex/sessions` (the CLI's long-standing behavior). A test that
//! forgot to declare its sources used to scan those real folders. Under the
//! test-only `test-credential-store` feature, which this integration test is
//! built with through the crate's self dev-dependency (and which integration
//! tests need, since they link the library without `cfg(test)`), the sources
//! crate resolves that fallback against an empty per-process folder instead.

use std::path::{Path, PathBuf};

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon::settings::DaemonSettings;
use trace_commons_contributor::source;

fn real_home() -> Option<PathBuf> {
    dirs::home_dir().filter(|home| home.parent().is_some())
}

fn under(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

#[test]
fn the_source_home_is_an_empty_folder_away_from_the_real_home() {
    let home = source::home_dir();
    assert_eq!(home, source::test_home_dir());
    assert!(home.starts_with(std::env::temp_dir()));
    assert!(!home.exists(), "nothing creates the test home");
    if let Some(real) = real_home() {
        assert_ne!(home, real);
    }
}

/// The daemon's own path: a fresh store with nothing declared still builds
/// the Claude Code and Codex fallbacks, and both land in the test home.
#[test]
fn undeclared_daemon_sources_never_resolve_the_real_home() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let identities = DaemonSettings::default()
        .source_roots(&store)
        .source_identities();
    let test_home = source::test_home_dir();
    for name in [source::SOURCE_CLAUDE_CODE, source::SOURCE_CODEX] {
        let root = identities
            .get(name)
            .unwrap_or_else(|| panic!("{name}'s undeclared fallback is still built"));
        assert!(
            under(Path::new(root), &test_home),
            "{name} resolved outside the test home: {root}"
        );
    }
    if let Some(real) = real_home() {
        for (name, root) in &identities {
            assert!(
                !root.contains(&*real.to_string_lossy()),
                "{name} resolved under the real home: {root}"
            );
        }
    }
}

/// The setup probe reads the same home and none of the environment.
#[test]
fn the_machine_probe_never_reads_the_real_home() {
    let test_home = source::test_home_dir();
    for candidate in source::discovery::probe_this_machine() {
        assert!(
            under(&candidate.path, &test_home),
            "{} probed outside the test home: {}",
            candidate.source,
            candidate.path.display()
        );
        assert!(!candidate.exists);
    }
}
