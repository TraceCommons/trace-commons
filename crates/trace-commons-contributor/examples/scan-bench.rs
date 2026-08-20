//! Time one `discover()` pass over a session root.
//!
//! An example rather than a test because the thing being measured is wall
//! clock and resident memory against a corpus far larger than any fixture a
//! test should carry. Generate a tree with
//! `scripts/bench/gen-session-fixture.py <dir>` and run this under
//! `/usr/bin/time -l` to get peak RSS alongside the elapsed time.
//!
//! The generator mirrors the size distribution measured on a real machine
//! rather than inventing one, because that distribution is the whole point:
//! the median rollout is under a megabyte and the tail runs to hundreds, so
//! a fixture of twenty small files reproduces nothing. Every test in this
//! repo passed while the shipped client held 4.4GB resident.
//!
//! The pass under test is discovery, not load: `watcher::tick` calls
//! `discover()` on every poll, before any size or mtime gating can skip a
//! file, so whatever discovery costs is paid every `poll_interval_secs`.

use std::path::PathBuf;
use std::time::Instant;

use trace_commons_contributor::source::TraceSource;
use trace_commons_contributor::source::codex::CodexSource;

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: scan-bench <session-root>");
    let source = CodexSource::new(PathBuf::from(&root));

    let passes: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    for pass in 1..=passes {
        let started = Instant::now();
        let refs = source.discover().expect("discover");
        let elapsed = started.elapsed();
        let with_cwd = refs.iter().filter(|r| r.cwd.is_some()).count();
        let bytes: u64 = refs.iter().map(|r| r.size_bytes).sum();
        println!(
            "pass {pass}: {} sessions, {} with cwd, {:.2}GB on disk, {:.2}s",
            refs.len(),
            with_cwd,
            bytes as f64 / 1e9,
            elapsed.as_secs_f64()
        );
    }
}
