# Claude Desktop Private AI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Deliver linked draft PRs with a bound Vertex gateway, durable daemon connection, and native setup for Claude Desktop Code sessions.

**Architecture:** IronWire normalizes Vertex requests into its existing Messages handler. Trace Commons owns a credential-bound, revocable NEAR-only listener and exports a manual saved-environment preset from native Settings.

**Tech Stack:** Rust, axum, reqwest, embedded IronWire, synthetic local HTTP fixtures.

**Spec:** docs/superpowers/specs/2026-11-05-claude-desktop-private-ai-design.md

## Global Constraints

- No new dependencies or private Desktop-store edits.
- No invented token counts, caller-selected account routing, or raw path identifiers in logs.
- Preserve existing Messages wire fidelity and permissive client licensing.
- Every cargo command uses RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0.
- Run final tests and CI checks after the final task; per-task compile checks only in Trace Commons, per AGENTS.md.

## Review Focus

- Malformed or oversized Vertex requests must not leak project/region strings through guard records.
- Route overrides must not bypass the host's chosen route.
- Desktop's requested Claude ID must remain distinct from the served model.
- Count failures must not become fabricated usage or retryable 503 responses.
- Existing Messages clients must retain their byte and credential behavior.

## Task 1: Upstream Vertex facade

**Files:** IronWire facade/vertex.rs, facade/anthropic.rs, facade/guard.rs, server.rs, protocol docs and conformance tests.
**Interfaces:** Exposes POST /vertex/projects/{project}/locations/{region}/publishers/anthropic/models/{model}:rawPredict and :streamRawPredict, plus terminal unsupported count-tokens:rawPredict.

- [x] Add failing HTTP conformance tests for normalization, streams, errors, credential stripping, original/served model, canonical paths and route overrides.
- [x] Normalize into the existing Messages handler without a second HTTP hop; keep native Messages unchanged.
- [x] Verify proxy tests, formatting, lint and conformance; commit upstream foundation.

## Task 2: Embedded integration

**Files:** contributor Cargo.toml; root, GTK and Tauri Cargo.lock; GTK Flatpak sources; managed/desktop_route_tests.rs; managed/route.rs.
**Interfaces:** ManagedRoute::start_at against a local synthetic NEAR API; no new production API.

- [x] Write a fixture proving a Desktop-shaped request is initially absent, then reaches only the route's NEAR backend with its host-owned key after the upstream change.
- [x] Assert count failures return 4xx and make no completion call; request auth and project/region do not enter upstream payloads.
- [x] Pin the exact upstream commit; update only the source revision in all three lockfiles and validate with locked metadata and update Flatpak source revision.
- [x] Run contributor tests/check/clippy, license-boundary and four license trees; check lock/source consistency and formatting once at the end.

## Task 3: Review and drafts

- [x] Review the actual upstream and consumer diffs with a fresh reviewer.
- [x] Address material findings and verify changed behavior.
- [x] Open linked draft PRs with actual verification evidence and pending Desktop lifecycle/setup/live-validation gates.

## Foundation verification outcome

The contributor suite passed with 5,501 tests passed and 10 ignored. The new
embedded test uses an explicit authenticated backend/model pin; a managed
route's default model alone is intentionally not a pin. Contributor/FFI checks,
contributor clippy, formatting, all four dependency-license configurations and
the four license-boundary tests passed. Three locked metadata reads and a
source-only lockfile diff check passed. macOS emitted the same compact-unwind
linker warning seen on the pre-change baseline.

The upstream foundation is [IronWire #78](https://github.com/nearai/ironwire/pull/78),
pinned at `e84e3cf198031bf267dec22d6b05c793826dbd26`. Its full all-feature suite,
all-target/all-feature lint and five facade conformance tests passed. A fresh
review found no must-fix issues. Hosted CI and real Desktop validation are
separate evidence states; no live inference or user configuration was used.

## Product follow-up tasks

The owner requested these tasks after the foundation drafts were opened. Keep
changes on the same branches and run the complete final checks after the last
task, per AGENTS.md.

### Task 4: Bound embedded gateway
- [x] Add immutable VertexGateway embedding policy and authenticate before parsing/counting.
- [x] Hide other inference facades; force exact backend/main/Haiku model without fallback.
- [x] Prove auth, override/control-pin resistance, privacy/eligibility, accounting and socket shutdown/rotation.

### Task 5: Durable daemon connection
- [x] Persist and restore a credential-bound stable loopback route; rotate/revoke safely.
- [x] Add the four Desktop IPC actions, safe observed status and explicit preset export.
- [x] Enforce model privacy/context budgets, switch/credential/dry-run boundaries and race-safe lifecycle.
- [x] Test restart, disconnect, stale starts, key changes, corrupt state, busy port and no secret/body leakage.

### Task 6: Native setup
- [x] Add ordinary tools row and setup sheet, model choices and explicit Copy/Disconnect.
- [x] Test exact IPC, decoder fail-closed behavior, clipboard intent and late-response/dismissal races.

### Task 7: Validation and review
- [x] Validate installed Desktop against synthetic fixture, including terminal HTTP 400 counting behavior and tool round trip.
- [x] Run final Rust checks/suites, FFI rebuild plus Swift tests and relevant copy/contract checks.
- [x] Fresh review, fix material findings, update draft PRs and report remaining live/release evidence precisely.

## Desktop setup verification before feature flag integration

The final IronWire revision is `e6b4fecd92d07a1cb8df63ee2abaf92b97464681`.
Its all-feature suite passed with 1,575 tests passed and two ignored; lint,
formatting and an independent 13-test Vertex conformance rerun passed. The final
contributor suite passed with 5,517 tests passed and ten ignored, including the
adopted-runtime FFI path, canceled starts and dry-run dispatch guards. Contributor
and FFI lint passed. All four dependency-license configurations, four license
boundary tests, three locked metadata reads, formatting and diff checks passed.
The pre-existing macOS compact-unwind linker warning remains.

Installed Desktop protocol validation and live NEAR gateway validation are
recorded separately in
[the validation report](../reports/2026-11-05-claude-desktop-validation.md).
A combined Desktop-to-live-NEAR session, actual full-context automatic
compaction and Desktop stop-button cancellation remain promotion gates.

Final FFI tests passed (293 tests); a fresh FFI build followed by the full Swift
suite passed, including setup recovery and render regressions. The independent
review's two findings were fixed and rechecked. Both linked PRs remain drafts;
CI, merge, release and deployment are separate states.

## Feature flag and Windows follow-up

- [x] Register the live/default-false flag and capability in all three profiles;
  keep examples disabled and bump registry/profile versions together.
- [x] Gate daemon configure/export/restore, reconcile disable/expiry promptly,
  preserve bindings across flag disable and revoke on account-policy changes.
  Test opt-in, off/default/missing, pending starts, drain, expiry and re-enable.
- [x] Gate native setup from the daemon snapshot; clear setup secrets and ignore
  late responses on disable. Test missing/off/on and in-flight disable.
- [x] Diagnose Windows failures from hosted logs; repair cleanup budgets and
  shared listener-barrier ownership, retaining lifecycle assertions. Hosted
  verification of the final combined revision is recorded in the PR.
- [x] Run final contributor/protocol/FFI tests, lint, formatting, profile scripts,
  license boundary, fresh FFI build plus Swift suite; review, update and push PR.

Local follow-up validation passed with 5,546 contributor tests, 676 protocol
tests, 295 FFI tests, four license-boundary tests, eight profile/publisher/CLI
checks and the full rebuilt native suite. Review's atomic publication finding
was repaired with a controlled interleaving regression; all affected checks
were rerun. Hosted Windows diagnosis/verification is tracked in the PR.

Final Windows repair validation passed locally with 5,546 contributor tests,
676 protocol tests, 295 FFI tests and the rebuilt full native suite, plus all
profile, license, metadata and lint checks. Upstream's exact-head Windows
lifecycle gate passed on `eeab5a61ae658842946f03c857f1f50345dd9487`.
