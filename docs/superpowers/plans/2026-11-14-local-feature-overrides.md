# Local Feature Overrides Implementation Plan

> **For agentic workers:** Use the approved repository dynamic workflow, delegate independent implementation and review tasks, then integrate and review once. User authorized implementation; no further design approval is pending.

**Goal:** Expose durable per-machine feature toggles in every build environment.
**Architecture:** Daemon-owned overrides layered above verified environment decisions, constrained to product flags supported by the build. CLI mutations and config/environment loading share one runtime; native UI reads the existing effective snapshot.
**Tech Stack:** Existing Rust, serde, local authenticated IPC; no new dependencies.
**Spec:** ../specs/2026-11-14-local-feature-overrides-design.md

## Global Constraints
- All environments including production. No permission grants. No new notices.
- Rust commands: RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0.
- Repository multi-task rule: write tests per task, run consolidated suites after implementation; at most cargo check per task.
- No Settings controls or other UI additions; user explicitly narrowed configuration to CLI/config/environment.

## Review Focus
- Same effective boolean but changed local selection must increment generation.
- Remote disable/expiry cannot erase local choice; clearing must use current remote state.
- Unsupported, privacy-sensitive, corrupt or wrong-profile storage must never enable.
- Persistence failure must preserve both effective value and selected override.
- CLI/IPC profile mismatch must be rejected before writing.

### Task 1: Rust runtime, storage and CLI
Files: contributor/src/feature_flags/{mod.rs,local.rs,tests.rs}, contributor/src/daemon/ipc.rs and tests, contributor/src/cli/feature_flags.rs.
Interfaces: Snapshot fields and feature_flags_override_set contract exactly as spec; Runtime::set_local_override(id: &str, value: Option<bool>, now: DateTime<Utc>) -> anyhow::Result<Snapshot>.
- [x] Write behavior tests for the review focus, persistence, next-start and all environments.
- [x] Implement bounded profile-scoped persistence, evaluation and generation behavior.
- [x] Add daemon mutation with identity validation and change event, CLI set on/off/default and diagnostics.
- [x] Compile changed crates; leave full test run to integration.

### Task 2: Environment variable and direct config support
Files: contributor/src/feature_flags/{environment.rs,environment_tests.rs,mod.rs,local.rs}, cli/feature_flags.rs, operator docs.
Interfaces: TRACE_COMMONS_FEATURE_FLAGS JSON object parsed once by Runtime::for_store; Snapshot environment_variable_overrides and environment_variable_error; env > local > remote > default.
- [x] Remove superseded Swift Settings implementation and shared-copy changes.
- [x] Write tests for precedence, next-start latching, invalid maps and config restart.
- [x] Implement bounded duplicate-safe parsing and diagnostic metadata, without persisting process choices.

### Task 3: Integration and verification
- [x] Update operator guide for precedence, persistence and CLI usage.
- [x] Run fmt, targeted protocol/contributor tests, contributor full tests and clippy, FFI build, Swift full suite and wording checks.
- [x] Review full diff with independent reviewer, fix material findings, rerun affected checks.
- [x] Verify built CLI with an isolated staging daemon and toggle round trip; record installed app remains old until upgraded.

## Validation results

- Contributor targets: 5,758 passed, 11 ignored. The initial dispatcher-count regression was fixed; a credential cleanup timeout passed in isolation and the full rerun with eight threads.
- FFI tested separately: 298 passed. Combining contributor and FFI tests unifies the contributor test-credential-store feature and intentionally makes the real entitlement probe unavailable; the standalone suite and rebuilt FFI probe pass.
- Staging feature-flag tests: 36 passed, including real CLI/daemon persistence and process environment loading. Non-test staging CLI build and read-only environment-override smoke passed.
- License boundary: 4 passed. Formatting, diff whitespace and client clippy checks passed.
- Standard full Swift suite passed against the rebuilt production FFI. Strict Swift warnings-as-errors is blocked by two existing ImplicitStrongCapture warnings in unchanged AccountLoginChangeViews.swift with local Swift 6.4.2. The moved-worktree Swift package cache was reset before testing.
- Independent full-diff review found no material issues. No Settings/shared-copy changes, installed app changes, feed publication or deployment.
