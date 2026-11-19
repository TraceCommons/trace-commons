# Skills Insights Implementation Plan

> **For agentic workers:** Execute the scoped parallel assignments below, then integrate and review the whole branch. Repository multi-task verification policy applies: write tests with each task and run suites once after integration.

**Goal:** Build local skill visibility, public publisher feeds and update detection inside native Insights.

**Architecture:** A permissive Rust domain module detects installations and establishes freshness. Existing counter scans collect invocation evidence. A daemon service caches fixed public publisher metadata and local follows; native SwiftUI consumes a typed snapshot.

**Tech Stack:** Existing Rust serde/sha2/reqwest and SwiftUI/Observation; no new dependencies.

**Spec:** docs/superpowers/specs/2026-11-05-insights-skills-design.md

## Global constraints

- No NEAR AI, Judgements, generated revisions, installs or automatic updates.
- Native macOS lead client; no new Tauri work.
- Never/off policies govern trace and project reads; local content never enters catalog requests.
- Source identity requires provenance, not a matching name.
- Preserve the dirty primary checkout; use `.claude/bin/tc-worktree insights-skills-v1`.
- All cargo commands use `RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0`.

## Review focus

- Copied or modified skills must not be called outdated solely because bytes differ.
- Unknown or failed coverage must not become a zero-use/current assertion.
- GitHub failures or truncated trees must retain last-known data without false freshness.
- Agent instructions embedded in user text must not count as invocation events.
- View cancellation and concurrent follow/refresh responses must not overwrite newer state.

## Tasks

- [x] Domain: `contributor-insights/src/insights/skills.rs` and submodules. Implement bounded inventory, frontmatter, provenance, catalog types and freshness. Tests cover symlinks, bounds, multiline metadata, name collisions and history-based updates.
- [x] Usage: `contributor-insights/src/insights/skill_usage.rs` and `contributor/src/daemon/insights_week.rs`. Extract explicit calls, persist bounded evidence, re-read older rows once, expose aggregate/session references. Tests cover fake user events, deduplication and policy gates.
- [x] Daemon: `contributor/src/daemon/insights_skills.rs`, `mod.rs`, `ipc.rs`. Implement `insights_skills` and `insights_skills_follow`, private bounded cache, fixed publisher fetches, follow persistence, changes feed and shared copy. Tests cover params, stale cache, failure retention and update state.
- [x] Native: typed `SkillsModels.swift`, daemon protocol/live client, observation model and Skills tab. Tests cover wire compatibility, refresh failure, follow serialization and navigation.
- [x] Integrate: shared analytics tab copy, exact wire shapes, real inventory/source checks, formatting, focused and complete relevant suites, rebuilt FFI and Swift tests.
- [x] Independent review of final diff; fix material findings with regressions. Record final validation and limitations below.

## Implementation decisions

- Reused a completed release worktree through the helper, then moved it into
  `.worktrees/insights-skills-v1` to keep edits inside writable roots.
- Usage starts with explicit harness evidence; unsupported harness schemas are
  represented as unknown coverage rather than guessed from shell commands.
- Freshness is about exact SKILL.md content, not executing a full skill bundle.
- Publisher snapshots are public data; follows and local usage remain local.

## Validation record

- Broad contributor suite: 5,514 passed, 11 ignored, no failures before the final review fixes.
- Broad support/Insights/FFI suites: 1,313 passed, 6 ignored. After review, the complete Insights suite passed 716 tests and the daemon Skills suite passed 14 tests (public-network smoke ignored in ordinary runs).
- License boundary: 4 passed. No dependencies changed.
- Client clippy: passed with the repository's existing allowances.
- Live public catalog: 58 Poteto, 15 Superpowers, 9 Slack documents fetched,
  parsed and cache-validated. Native Swift decoder accepted all 82 entries.
- Read-only live inventory: 205 skill entries, including explicitly labelled
  plugin-cache entries; 74 had source provenance; scan coverage complete.
- Initial full Swift run: one pane-width render test failed with a SwiftUI
  InvalidTransition; its isolated rerun passed. The original-three-tab full diagnostic passed; a new lifecycle probe then reproduced lingering hosted tasks. The shared render helper now ends its root lifecycle before closing; the probe passed. Final restored four-tab suite passed: 3,525 XCTest tests (10 skips), zero failures, plus 8 Swift Testing tests. The formerly failing pane test, lifecycle regression and all eight Skills renders passed in full order.
- All six initial contributor failures were resolved: two dispatch count
  contracts updated for the added APIs; four sandbox-dependent OS tests
  passed outside the sandbox. Final library suite is green.

## Bounds and limitations

- Explicit Claude invocations and conservative Codex loads are separate;
  OpenCode invocation coverage is unavailable. Usage names do not establish
  a unique installation identity.
- Public catalog entrypoint hashes do not cover scripts or the effective Rex
  CLI guide. Rex is documentation-only and has no verified freshness feed.
- First-scan history checks cover at most eight proven installed source paths
  and three older versions per refresh. Unconfirmed differences stay unknown.
- Cached plugin files are detected, but their active selection is not inferred.
- Inventory/catalog/feed wire budgets expose partial coverage when exceeded.
- UI uses four local sections and search after the live inventory exposed the
  usability cost of one long combined page.

## Independent review fixes

- Canonical read policy now applies to skill documents, directory listings,
  lock files and plugin metadata. Inventory cache identity includes excluded
  roots; changing a Never rule cannot reuse a previously permitted global
  inventory. Global Never suppresses all local inventory, retaining the public
  catalog. Unresolved exclusion roots fail closed.
- Git provenance is skipped when exclusions exist, since Git may read includes,
  alternates and worktree metadata outside the skill document's tree.
- Plugin-relative paths are not assumed to be repository-relative. Explicit
  repository-directory metadata or the verified Superpowers root mapping is
  required; ambiguous monorepo packages retain unknown provenance.
- Automatic observation preserves the six-hour catalog throttle. Only the
  explicit Refresh action forces a check.
- Reactivation waits for cancelled-but-still-running IPC reads to release the
  serial lane, then reads promptly rather than delaying five minutes.
- Claude rows do not display an unsupported zero-load count.
- The hosted render lifecycle regression failed before the helper fix and
  passed after it; image and readiness assertions were preserved.

## Final delivery

- Final client clippy, formatting and diff checks passed. FFI rebuilt after
  review fixes. No dependency or license-boundary change.
- Independent review findings were addressed with focused privacy, provenance,
  cancellation and usage-display regressions.
- UI artifacts: `/private/tmp/insights-skills-reviewed-renders/` contains all
  four sections at 440pt and 900pt; narrow and wide layouts were inspected.
- Local implementation branch: `insights-skills-v1`. No deployment or release.

## Environment-flag integration requested by owner

- [x] Stack PR #2004 on PR #2003 (`environment-feature-flags`) and preserve
  the Skills implementation in its isolated worktree.
- [x] Register `insights_skills` with live timing, false default and
  `insights_skills_v1` capability. Bump registry/profile versions together;
  all environment profiles support a future remote enable. Dated manifests
  explicitly keep it false; do not publish them.
- [x] Gate daemon entry points and background usage extraction through the
  authoritative runtime. Test off/on/off, default-off and no disabled side effects.
- [x] Gate native tab visibility, cancel hidden polling and move selection
  back to Overview on disable. Test missing/older daemon and live changes.
- [x] Re-run integration, profile/package, Rust and native checks, then update
  the stacked PR with the exact verified head.

Stacked verification: the new Runtime off/on/off endpoint and counter tests,
default-off/capability tests in all three environments, profile/publisher/CLI
tests (8), channel plist checks, clippy and license boundary (4) pass. The
production plist fixtures were updated only for the intentional v2 profile digest.
The broad Rust command initially unified the contributor's test-only credential
feature into FFI; running the FFI package separately passes all 295 tests. The
unchanged launcher PID-marker fixture raced once; its isolated regression and
entire contributor integration target (347 tests) passed on rerun. The native
flag host now has the same finite viewport as the app, and both snapshot helpers
end hosted tasks before closing. Focused native flag/pane/lifecycle checks pass;
the final full native run passed: 3,541 XCTest tests (10 skips), zero failures, plus 8 Swift Testing tests.

A focused independent review of the flag integration found no remaining important findings. All publication remains an explicit separate operation; no live flag configuration was published.
