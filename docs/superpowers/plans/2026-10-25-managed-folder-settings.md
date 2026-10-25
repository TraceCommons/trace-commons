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
- [x] Protect collection isolation using exact native identities while allowing concurrent runs in the same account profile.
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
- Independent review findings resolved: preference-save failure rolls back an unredeemed preparation; native collection respects exact session ownership and staged-byte validation.

## Live diagnostic boundary

Read-only audit found both production and staging at the 500-entry counted queue
cap (Pending plus Approved excluding person-included entries). New captured
exports remain local until capacity is available. No live settings, queues,
uploads or credentials were changed. No release/deployment was performed.

## Follow-up: exact session identity instead of account serialization

The user requested concurrent managed sessions in the same account profile and
emphasized attested traces. Replace the native profile exclusivity guard with
exact ownership. Claude receives the managed UUID via --session-id. Codex 0.164+
uses its SessionStart hook, trusting only the injected tracking command and
using Codex's hook-override exclusion to select an invocation-local app server.
Do not append --no-daemon after user subcommands or force-enable an
explicitly disabled hook engine or replace custom hook configurations.

- [x] Validate exact Claude parent identity and owned subagent paths.
- [x] Record bounded, private Codex native ID/path receipts per launch.
- [x] Require receipt ID, rollout filename UUID and metadata ID agreement.
- [x] Keep preexisting-file exclusion; missing/ambiguous identity fails only collection.
- [x] Remove coding-session account serialization; preserve login locks.
- [x] Preserve original native identifiers for attestation joins and existing body-capture consent.
- [x] Verify concurrent same-profile/same-folder on/off and on/on collection isolation.
- [x] Include binding metadata in local wipe cleanup; review and run relevant tests/lint/builds.

### Exact-identity verification

- Full contributor suite: 4,319 passed, 9 ignored, exit 0. After the final small binder flag fix, all 26 affected native launch/folder/collection/binding tests passed (including 7 binding tests).
- Full source parser suite: 239 passed. Exact Claude ownership helper regressions cover parent and child identity, ambiguity and symlinks.
- Actual installed Codex 0.164 no-inference probes established distinct IDs/paths for concurrent same-profile/same-folder runs and preservation of existing trusted/untrusted hooks. Native exec/resume/fork help accepts the generated configuration.
- Codex binding receipts are bounded, private, nonce-checked, atomic and included in local wipe cleanup; preexisting resumed logs are excluded.
- Final contributor/source all-targets Clippy, license boundary (4 tests), fresh FFI build, formatting and diff checks passed.
- Independent review confirmed exact ownership and attestation identifier preservation; removed an unnecessary --no-daemon flag after native help exposed its subcommand-positioning incompatibility. Codex's hook overrides themselves select the embedded server.
- No live settings, queues, uploads or attestation-body consent were changed. OpenCode 2.x qualification remains separate; parser format support is still 1.18.29/1.19.0.
