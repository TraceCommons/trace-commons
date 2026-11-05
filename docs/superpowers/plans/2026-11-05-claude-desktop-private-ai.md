# Claude Desktop Private AI Foundation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Open linked draft PRs with an embedded, tested Vertex protocol foundation for Claude Desktop Code sessions.

**Architecture:** IronWire normalizes Vertex requests into its existing Messages handler. Trace Commons consumes the pinned upstream revision and verifies it through its NEAR-only managed route. Desktop setup and lifecycle remain explicit subsequent gates.

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
- [ ] Open linked draft PRs with actual verification evidence and pending Desktop lifecycle/setup/live-validation gates.

## Local verification outcome

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
