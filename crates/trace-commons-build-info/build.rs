//! Resolves the commit and build time that every Trace Commons binary reports.
//!
//! The ordering here is the whole point of the crate. `.gcloudignore` excludes
//! `.git/`, so Cloud Build compiles from a source tarball with no git metadata
//! at all: a build script that only shelled out to git would resolve to
//! "unknown" on precisely the builds that get deployed. So the environment
//! variable is the primary source and git is the local-developer fallback.
//!
//! Nothing here ever substitutes a semver for a commit. The crate version does
//! not move when a deploy does, so reporting it as identity would be worse than
//! reporting nothing.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // What makes this build script re-run, and therefore what makes a stamped
    // binary go stale. Naming any `rerun-if-changed` path opts out of cargo's
    // default "re-run when any file in the package changed", so build.rs is
    // listed explicitly to get that back for this file.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/iso8601.rs");
    println!("cargo:rerun-if-env-changed=TRACE_COMMONS_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

    let commit = resolve_commit();
    let build_time = resolve_build_time();

    println!("cargo:rustc-env=TRACE_COMMONS_BUILD_INFO_COMMIT={commit}");
    println!("cargo:rustc-env=TRACE_COMMONS_BUILD_INFO_TIME={build_time}");
}

fn resolve_commit() -> String {
    if let Some(commit) = std::env::var("TRACE_COMMONS_BUILD_COMMIT")
        .ok()
        .and_then(|value| sanitize(&value))
    {
        return commit;
    }

    if let Some(commit) = commit_from_git() {
        return commit;
    }

    // Last resort. An honest "unknown" is recoverable -- an operator knows to
    // go look elsewhere -- where a plausible-looking wrong value is not.
    "unknown".to_string()
}

/// Read the commit from the working tree, for local developer builds. Also
/// registers the git files that decide the answer, so committing or switching
/// branches re-stamps the binary instead of leaving a stale commit behind.
fn commit_from_git() -> Option<String> {
    let git_dir = git_dir()?;
    let head_path = git_dir.join("HEAD");
    if !head_path.exists() {
        return None;
    }
    println!("cargo:rerun-if-changed={}", head_path.display());

    // A checked-out branch's tip lives in a ref file that HEAD points at, and
    // that file is what changes on a new commit -- HEAD itself only changes on
    // a branch switch. Both have to be watched. `packed-refs` is watched too
    // because a freshly cloned or packed repository keeps the ref there and has
    // no loose file to watch.
    if let Ok(head) = std::fs::read_to_string(&head_path)
        && let Some(reference) = head.trim().strip_prefix("ref: ")
    {
        let ref_path = git_dir.join(reference);
        if ref_path.exists() {
            println!("cargo:rerun-if-changed={}", ref_path.display());
        }
        let packed = git_dir.join("packed-refs");
        if packed.exists() {
            println!("cargo:rerun-if-changed={}", packed.display());
        }
    }

    let output = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    sanitize(&String::from_utf8_lossy(&output.stdout))
}

/// Locate the `.git` directory for the crate being built. In a worktree,
/// `.git` is a file naming the real directory, so the plain-directory case is
/// not enough on its own.
fn git_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").ok()?);
    let mut dir: Option<&Path> = Some(manifest_dir.as_path());
    while let Some(current) = dir {
        let candidate = current.join(".git");
        if candidate.is_dir() {
            return Some(candidate);
        }
        if candidate.is_file()
            && let Ok(contents) = std::fs::read_to_string(&candidate)
            && let Some(path) = contents.trim().strip_prefix("gitdir: ")
        {
            let path = PathBuf::from(path);
            return Some(if path.is_absolute() {
                path
            } else {
                current.join(path)
            });
        }
        dir = current.parent();
    }
    None
}

/// Accept a commit-ish identifier, or nothing. The value can be a short SHA, a
/// tag, or a Cloud Build id, so this is a character allowlist rather than a hex
/// test. It also keeps the value safe to paste into a Rust string literal and
/// into a `cargo:` directive line.
fn sanitize(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 64 {
        return None;
    }
    if trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
    {
        Some(trimmed.to_string())
    } else {
        None
    }
}

/// The build timestamp, ISO-8601 in UTC. `SOURCE_DATE_EPOCH` wins when set,
/// because that is the standard reproducible-builds lever and a build that
/// opts into it must not be re-stamped with wall-clock time.
///
/// Note that this is the time the build script last ran, not the time the
/// binary was last linked: the rerun rules above deliberately keep it from
/// re-running on every unrelated edit. It answers "roughly when was this
/// built", and the commit answers "what is this".
fn resolve_build_time() -> String {
    let seconds = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|value| value.trim().parse::<i64>().ok())
        .unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
    format_iso8601_utc(seconds)
}

// Shared with the library so the date arithmetic is unit-tested rather than
// only exercised by whatever date the build happens on.
include!("src/iso8601.rs");
