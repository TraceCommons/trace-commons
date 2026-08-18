//! The build identity every Trace Commons binary reports.
//!
//! This crate exists because a deploy could not be identified. The running
//! binary carried no commit, `/health` carried no commit, and the only evidence
//! left was a file mtime on the host plus grepping the binary for a string from
//! a known commit. A semver would not have helped: these binaries are built
//! from a commit and pushed to object storage, not cut on a tag stream, and the
//! version stayed put across every change that shipped that day. The useful
//! identity is the commit.
//!
//! Resolution order lives in `build.rs`: the `TRACE_COMMONS_BUILD_COMMIT`
//! environment variable first, `git rev-parse --short HEAD` second, and the
//! literal `unknown` last.

use std::sync::OnceLock;

// Shipped code never calls this -- `build.rs` `include!`s the same file and
// formats the timestamp there. It is declared for tests only so the date
// arithmetic is covered without leaving dead code in the library.
#[cfg(test)]
mod iso8601;

/// The commit this binary was built from, or `unknown` when neither the
/// environment variable nor a git checkout was available at build time.
pub const COMMIT: &str = env!("TRACE_COMMONS_BUILD_INFO_COMMIT");

/// When this binary was built, ISO-8601 in UTC (`YYYY-MM-DDTHH:MM:SSZ`).
pub const BUILD_TIME: &str = env!("TRACE_COMMONS_BUILD_INFO_TIME");

/// The version half of a `--version` line: the caller's own package version
/// followed by the build identity. Callers pass `env!("CARGO_PKG_VERSION")`,
/// because the version that matters is theirs and not this crate's.
///
/// Returns a `&'static str` rather than a `String` because that is what clap's
/// `version` builder takes. A process has exactly one package version, so
/// caching the first answer for the life of the process is not a compromise:
/// every call in a given binary passes the same literal.
pub fn version_line(package_version: &str) -> &'static str {
    static LINE: OnceLock<String> = OnceLock::new();
    LINE.get_or_init(|| format!("{package_version} (commit {COMMIT}, built {BUILD_TIME})"))
        .as_str()
}

/// A full one-line identity, for a binary that prints its own name.
pub fn identity(package_name: &str, package_version: &str) -> String {
    format!("{package_name} {}", version_line(package_version))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_populated() {
        assert!(!COMMIT.is_empty());
        // ISO-8601 UTC to the second, and nothing else.
        assert_eq!(BUILD_TIME.len(), 20, "unexpected build time {BUILD_TIME}");
        assert!(
            BUILD_TIME.ends_with('Z'),
            "unexpected build time {BUILD_TIME}"
        );
    }

    #[test]
    fn iso8601_matches_known_instants() {
        use crate::iso8601::format_iso8601_utc;
        assert_eq!(format_iso8601_utc(0), "1970-01-01T00:00:00Z");
        // A leap day in a leap century, the case the naive formula gets wrong.
        assert_eq!(format_iso8601_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_iso8601_utc(1_755_388_800), "2025-08-17T00:00:00Z");
        assert_eq!(
            format_iso8601_utc(1_755_388_800 + 45_296),
            "2025-08-17T12:34:56Z"
        );
        // Before the epoch, so the euclidean division is not just decoration.
        assert_eq!(format_iso8601_utc(-1), "1969-12-31T23:59:59Z");
    }

    #[test]
    fn identity_carries_name_version_and_commit() {
        let line = identity("trace-commons-example", "1.2.3");
        assert!(line.starts_with("trace-commons-example 1.2.3 (commit "));
        assert!(line.contains(COMMIT));
        assert!(line.contains(BUILD_TIME));
    }
}
