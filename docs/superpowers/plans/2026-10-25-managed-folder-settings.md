# Managed folder settings implementation plan

**Goal:** Remember model, trace collection and CLI options per canonical folder and agent. Account selection remains explicit in the interface or CLI.

**Architecture:** A canonical daemon setting stores folder/tool profiles, shared by the native UI, CLI launches and config schema/show/preview/apply. New folders inherit global CLI options, model default and collection off. Lookup restores the launch form; preparing a new coding session remembers its choices. Reset removes only that folder/tool profile. Existing sessions and sign-in are unaffected.

**Constraints:** No new dependencies. Preserve routing isolation, current consent gates, explicit account selection, idempotent preparation and captured launch arguments. Rust commands use RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0.

## Backend and CLI

- [x] Add failing tests for folder/tool isolation, canonical paths, persistence, reset, explicit empty options, idempotency and sign-in exclusion.
- [x] Implement shared setting, lookup/reset IPC, optional launch options and config schema/preview/apply integration.
- [x] Restore compatible CLI choices only when not explicitly overridden; offer explicit default-model and collection-off flags.
- [x] Verify contributor tests, Clippy, formatting, license boundary and fresh FFI; refresh real-daemon fixtures where fields change.

## Native macOS

- [x] Add decoder and draft tests, including stale callbacks when folder/account changes.
- [x] Restore remembered model, collection and CLI options on folder/tool selection; preserve explicit account choice.
- [x] Edit CLI options in the launch dialog and reset folder choices; block launch while lookup is pending or failed.
- [x] Preserve compatibility with older daemons and require a new choice for an unavailable model.
- [x] Verify focused and full Swift tests, shared-copy and daemon contract coverage.

## Explicit collection repair (user clarification)

- [x] Show Collect trace for review on the folder selection screen for all three agents.
- [x] Capture only newly created Claude/Codex native transcripts after opted-in exit; preserve original logs and exclude older opted-out files.
- [x] Route explicit native and OpenCode exports into ordinary review ingestion even when automatic source discovery is off.
- [x] Protect account-level native profile isolation when collecting; expose a specific busy error without blocking unrelated accounts or both-opt-out concurrency.
- [x] Verify temporary-daemon pending-queue admission and fake native launch collection; preserve the existing 500-entry queue bound.

## Integration

- [x] Independent review, fix material findings and verify changes.
- [x] Record validation and commit the isolated feature branch.

## Verified results

- Contributor verification: 4,308 passed and 9 ignored across completed targets. The full command found two stale library fixtures/contracts and the old OpenCode source-off expectation; all three exact reruns passed after test-only updates. No remaining failed targets.
- Source parser crate: 236 passed.
- Folder persistence/config/rollback/isolation coverage: 5 focused folder tests and 9 lifecycle tests passed, plus dedicated model/CLI configuration checks.
- Explicit collection integration: 4 passed, covering actual fake-native CLI/supervisor on/off capture for Claude/Codex, older opted-out file exclusion, real Pending queue admission and explicit OpenCode exports with automatic source discovery off.
- Full macOS suite: 3,137 tests, 9 skipped, no failures. Subsequent folder rendering, decoder and refreshed-sample contract checks passed after small layout/test updates.
- Contributor/source all-targets Clippy with repository allowances, formatting, diff checks, fresh FFI build and four license-boundary tests passed.
- Recorded samples regenerated through the actual temporary daemon; recorder drift passed.
- Wipe regression extended and passed: both managed staging directories are deleted while native agent originals are retained.
- Independent review findings resolved: preference-save failure rolls back an unredeemed preparation; native collection respects account exclusivity and staged-byte validation.

## Live diagnostic boundary

Read-only audit found both production and staging at the 500-entry counted queue
cap (Pending plus Approved excluding person-included entries). New captured
exports remain local until capacity is available. No live settings, queues,
uploads or credentials were changed. No release/deployment was performed.
