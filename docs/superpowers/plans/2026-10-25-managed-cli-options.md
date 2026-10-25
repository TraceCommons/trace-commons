# Managed CLI options implementation plan

> Execute in the isolated managed-cli-options worktree; backend and macOS work can proceed independently against the contract below.

**Goal:** Edit saved per-agent command-line options in Settings → Tools, applying them to new managed coding sessions.

**Architecture:** The local daemon owns and validates saved raw option strings. The managed launch snapshots parsed arguments and passes them directly to the native executable, with no shell evaluation. The native macOS app edits these values using shared Rust copy.

**Constraints:** No dependencies; preserve managed authentication/profile routing and sign-in commands; no arguments in logs; Rust commands use RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0. Existing sessions are unaffected.

## Task 1: Backend

- [x] Add failing coverage for persistence, parsing quoted values, shell metacharacters as literals, malformed input and reserved routing flags.
- [x] Implement private option storage and managed_cli_options_set ({tool, options}); add cli_options map to managed_snapshot.
- [x] Capture options for new coding launches, preserving idempotent preparation and excluding login.
- [x] Run managed tests and contributor checks.

## Task 2: macOS Settings

- [x] Add decoder/draft tests for old snapshots, per-agent values and refresh during editing.
- [x] Decode optional cli_options, keep drafts in AppModel, and save through managed_cli_options_set.
- [x] Add a shared-copy card in Tools with separate agent fields, Save actions and accessible names; show read and write failures.
- [x] Verify decoder, draft, rendering and Settings regressions; build fresh FFI before app tests.

## Task 3: CLI configuration compatibility (#1644)

- [x] Store managed_cli_options in DaemonSettings; share parser and setter with UI.
- [x] Extend config schema, show, preview and apply; partial map merges preserve other agents.
- [x] Run real temporary-daemon CLI preview/apply/show regression and UI snapshot check.
- [x] Return confirmed managed revision so stale reads cannot undo an acknowledged save.

## Task 4: Integration

- [x] Review the complete diff and fix substantive findings.
- [x] Run formatting, license-boundary regression, contributor and Swift checks appropriate to the change.
- [x] Record exactly which checks passed and any environment limitations.

## Review focus

Preserve drafts across refresh/navigation; failed saves must retain text and expose retry; absent old-daemon fields must disable editing; options must never execute through a shell; authentication routing and profile isolation cannot be overridden.

## Verification

- Built on main containing merged PR #1644 (`5e3827c09`).
- Real temporary-daemon CLI configuration tests: 2 passed (schema/preview and apply/show/snapshot/launch-store consistency).
- Managed option regressions: 7 passed, including quoting, literal shell syntax, reserved overrides, persistence, snapshot stability and login isolation.
- Contributor full verification: 4,295 passed and 9 ignored across 44 completed targets. The full command found recorded-sample drift; regenerated the 12 affected sample files and reran the recorder target (5 passed, 1 ignored). No remaining failed targets.
- Full macOS suite: 3,125 tests across XCTest and Swift Testing, 9 skipped, no failures. Reran sample/decoder/managed contract tests after the recorded samples changed; passed.
- Contributor all-targets Clippy with repository allowances, cargo formatting, diff checks and license boundary (4 tests) passed.
- Independent review findings fixed: TOML escaped keys, short-option clusters and save/refresh revision ordering.

The implementation is local to this feature branch; no release or deployment was performed.
