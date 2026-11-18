# Ranked Skills Home and Creator Follow Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development with disjoint file ownership. One coordinator owns builds of this shared worktree.

- [x] Define immutable ranking context/API and write ranking behavior tests; write Follow state/confirmation tests and daemon mixed-feed/budget tests before implementation. Run coordinated RED checks.
- [x] Implement bounded on-device candidate ranking and stable diversity selection; preserve existing fact/coverage generation.
- [x] Implement mixed Home creator eligibility and shared 48KiB event budgeting while preserving explicit Following and 180KiB catalog bounds.
- [x] Implement shared Follow pill/model pending and failure state; wire Home and Following consistently.
- [x] Render ranked stories before creator suggestions, retaining per-story time-window context and existing actions/copy guidance.
- [x] Run final focused Rust/Swift checks once after integration, plus required license gate and formatting. Replay current retained data and inspect native images. Independent review before a PR; report source/build/release separately.

## Verification

- Observed the new behavior tests fail before implementation, including the final source-variety, confirmed-follow/refresh-error, and weak-candidate integration regressions.
- Native Skills suite: 119 passed, including retained snapshot replay and 440/900-point native renders. Inspected both images locally.
- Full Skills daemon module: 67 passed; one explicit live-network qualification test remains ignored. Shared copy: 5 passed. License boundary: 4 passed.
- Rebuilt the Rust FFI and native test products; Clippy passed for both touched Rust crates with the existing allow-list. `cargo fmt --all -- --check` and `git diff --check` passed.
- Independent review found and verified the fix for premature usage candidate truncation. Final review has no blocking findings.
- No private replay data is included in source. No app installation, preference writes, server deployment, merge or release occurred. Other desktop shells and the full server/ingest suites were not exercised locally.
