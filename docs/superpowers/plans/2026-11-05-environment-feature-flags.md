# Environment Feature Flags Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Enable whole-environment feature changes on compatible installed builds through versioned remote manifests.

**Architecture:** A permissive protocol module validates manifests and resolves flags. The contributor daemon owns refresh, cache, and snapshots; CLI and Swift consume that authority through existing IPC/FFI. Build profiles constrain remote enables, and a separate publication command validates and replaces one environment's manifest.

**Tech Stack:** Existing Rust serde/serde_json, chrono, sha2, reqwest and Tokio; Swift Foundation/Observation; Python standard library and existing publication tooling. No added dependencies.

**Spec:** `docs/superpowers/specs/2026-11-05-environment-feature-flags-design.md`

Status: implemented, independently reviewed, and locally validated on 2026-11-05.

## Global Constraints

- Feature environments are `production`, `staging`, and `testing`; testing uses the staging service channel and app identity.
- Production release builds cannot select the testing profile.
- New experimental flags default false. Build capabilities bound all remote enables.
- Refresh every five minutes with jitter; requests have a five-second timeout and a 64 KiB streamed size limit. Maximum manifest validity is seven days.
- Cache keys include environment and profile digest. Rollbacks publish a higher revision.
- The daemon owns evaluation; Swift never starts another evaluator or poller.
- No live publication, existing-feature migration, or hosted ingest runtime integration in this branch.
- Preserve channel isolation, permissive/AGPL licensing, UI copy guidance, and the existing client entry points.
- Every Cargo command uses `RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0` as required by the owner's supplied instructions.
- Write tests per task, but run the combined Rust/Swift/CI verification once after the final task, per repository multi-task guidance. Per-task compile checks are optional.
- At execution, use `.claude/bin/tc-worktree environment-feature-flags` to obtain an isolated checkout; retain the approved spec and this plan there. Do not modify unrelated primary-checkout files.

## Review Focus

- A same-revision response with different bytes must not replace accepted content, including after restart.
- Expiry and a backward wall-clock jump must not make an expired cached enable valid again within a running process.
- Concurrent refresh, restart, and shutdown must not leave detached pollers or stale cache writers.
- A GUI attaching to an older daemon must see unsupported/unavailable flags as disabled rather than guess its own values.
- A stale CLI or FFI artifact must fail bundle agreement even when the service channel matches.

## File structure

- `crates/trace-commons-contributor/feature-flags/registry.json`, `profiles/{production,staging,testing}.json`, and `manifests/{production,staging,testing}.json`: reviewed source data. Initial registry/profiles contain no product gates; manifest examples are explicitly non-published until issued with fresh timestamps/revision.
- `crates/trace-commons-protocol/src/feature_flags/{mod,manifest,evaluate,tests}.rs`: typed contract, strict parsing, registry/profile validation, pure resolution, and test fixtures. Export from `src/lib.rs`.
- `crates/trace-commons-contributor/src/feature_flags/{mod,cache,transport,runtime,tests}.rs`: embedded profile selection, cache, bounded HTTP, runtime lifecycle. Export from `src/lib.rs`.
- `crates/trace-commons-contributor/src/{channel,cli}.rs` and `src/cli/feature_flags.rs`: channel-owned feed locations and CLI commands.
- `crates/trace-commons-contributor/src/daemon/{mod,ipc}.rs`: authoritative runtime ownership and additive IPC methods/events.
- `crates/trace-commons-contributor-ffi/src/lib.rs`: build-profile identity accessor; use existing `tc_call`/subscription for runtime snapshots.
- `macos/Sources/CTraceCommons/include/trace_commons.h`, `TCBridge/TCFeatureFlags.swift`, `TCShellCore/FeatureFlags.swift`, and `TraceCommonsApp/AppModel+FeatureFlags.swift`: FFI declaration, typed snapshots, and observed state. Extend existing AppModel subscription/initial refresh and wake observer.
- `macos/scripts/{channel,make-app-bundle,info-plist}.sh`, staging and release workflows: profile propagation and artifact agreement checks.
- `scripts/feature-flags/{publish.py,test_publish.py}` and `scripts/ci/test-feature-profiles.sh`: explicit publication and packaging regression checks.
- `docs/operator/feature-flags.md` and operator index: registry, gating, rollout, diagnostics, expiry, and rollback guide.

### Task 1: Wire contract and pure evaluator

**Interfaces:** `FeatureEnvironment`, `FlagDefinition`, `FlagRegistry`, `BuildProfile`, `ValidatedManifest`, `FlagSnapshot`; `parse_manifest(bytes: &[u8], environment: FeatureEnvironment, now: DateTime<Utc>) -> Result<ValidatedManifest, FlagError>`; `evaluate(registry: &FlagRegistry, profile: &BuildProfile, manifest: Option<&ValidatedManifest>, now: DateTime<Utc>) -> FlagSnapshot`.

- [x] Write `manifest_rejects_invalid_contract`: reject wrong environment/schema, duplicate root/map fields, nonboolean flags, future issue time, expired timestamps, expiry before issue, validity over seven days, and bodies over 65,536 bytes. Accept unknown flag IDs only with boolean values.
- [x] Write `resolution_is_a_complete_snapshot`: an omitted flag uses its compiled default, an unknown flag does not appear, and a requested unsupported capability stays false. A fixture supported flag toggles false/true/false across revisions.
- [x] Implement typed serde models with duplicate-detecting map visitors, bounded parse, and registry/profile validation. Use UTC RFC3339 timestamps and u64 revisions. Define canonical profile digest as SHA-256 of deterministic serde serialization of a validated profile plus its registry/schema version. Treat required capabilities and remote-enable allowlist as separate checks.
- [x] Add profile data and protocol module export without dependencies or AGPL headers. Add tests to the existing harness if integration test files are introduced.

### Task 2: Build identity, transport, and durable cache

**Interfaces:** `embedded_profile() -> Result<BuildProfile>`; `build_profile_identity() -> BuildProfileIdentity`; `FlagCache::load(now)`, `FlagCache::accept(manifest, now)`; `ManifestTransport::fetch() -> Result<Vec<u8>>`. Production transport has no arbitrary URL parameter exposed through user configuration.

- [x] Write `profile_selection_respects_service_channel`: default production/staging map correctly; compile-selected testing is allowed only with staging channel; unknown profile fails. Use compile-time `option_env!("TC_FEATURE_PROFILE")`, not a runtime environment read. Track profile source through `include_str!`.
- [x] Write `cache_preserves_revision_across_expiry_and_restart`: revisions 10, 9, changed-content 10, and 11 yield accept/reject/reject/accept; expired data cannot reset the revision watermark. Identical revision/content is idempotent. Scope cache by profile digest/environment.
- [x] Write `bad_response_preserves_valid_cache` and `cache_write_is_atomic`: malformed/truncated/wrong-environment data and disk failures cannot publish a partial snapshot or erase a good persisted cache. Cache load reads at most its fixed bounded envelope size. Persist accepted bytes, digest, revision watermark, and identity together.
- [x] Write bounded local transport tests for timeout, redirects, HTTP error, oversized streamed response with no content length, and no credential/cookie headers. Test transport injection is an internal test API only.
- [x] Implement fixed channel-owned feed locations using the existing public update buckets: production `https://storage.googleapis.com/tracecommons-flatpak/feature-flags/production.json`; staging `https://storage.googleapis.com/tc-staging-updates-20261022/feature-flags/staging.json`; testing the same staging bucket at `feature-flags/testing.json`. Put literals only in the appropriate channel definitions and the publisher's destination table. Reuse existing reqwest chunk reads, disable redirects, and create an anonymous client. No new ingest route is needed. Verify against channel-host/release checks before settling these source locations.
- [x] Implement cache writes with existing cross-platform atomic-file patterns and profile SHA identity. Keep revision/content watermark even when an expired snapshot falls back to defaults. In-process expiry is irreversible for that accepted snapshot; use monotonic elapsed time to prevent backwards clock jumps extending its remaining validity.

### Task 3: Daemon ownership, lifecycle, and diagnostics

**Interfaces:** `FeatureFlagRuntime::snapshot() -> FlagSnapshot`, `request_refresh()`, `run(shutdown)`; IPC read method `feature_flags_snapshot`, nonblocking wake signal `feature_flags_refresh`, event `feature_flags_changed`. Snapshot includes environment/profile digest, sampled time, source, revision/freshness, effective decisions, capability blocks, and restart-pending decisions.

- [x] Write `startup_latches_cache_before_fetch`, `live_expiry_changes_snapshot_offline`, and `next_start_changes_are_pending`: cached/default values latch before nonblocking startup fetch; expiry changes live decisions and pending restart values; a daemon restart adopts validated cached pending values.
- [x] Write `refresh_is_single_flight_and_shutdown_owned`: periodic/startup/wake triggers coalesce, no fetch exceeds five seconds, cancelled runtime stops its fetch and writes, supervisor restart cannot duplicate pollers, and dry-run/sample paths never access a live feed.
- [x] Initialize state from bounded cache in `DaemonShared::load`. Own one refresh future within the existing supervisor lifetime, using cancellation-safe structured concurrency rather than an untracked spawned task. Use 300-second intervals with bounded jitter that keeps total propagation target within six minutes, and separate timers for expiry.
- [x] Expose snapshots through additive IPC and events. Synchronous getters only clone immutable in-memory snapshots. Reads never fetch or write. A refresh signal cannot accept caller-supplied flags or URLs. Declare event capability through existing opt-in subscription conventions.
- [x] Add `near-ai feature-flags` (human-readable and global `--json`), `feature-flags build-profile --json`, and `feature-flags validate --file <path> --environment <name>`. Prefer daemon authority; if unavailable, use shared cache/default evaluator and label the source and sampling time. Local validation never fetches or publishes.
- [x] Add CLI tests for absent/older daemon and profile mismatch: no local guessed enable, clear diagnostics, and no fallback from a reachable incompatible daemon to a different evaluator.

### Task 4: Native macOS consumption and package agreement

**Interfaces:** C `char *tc_feature_profile_json(void)` freed by `tc_string_free`; Swift `FeatureFlagSnapshot: Decodable, Equatable, Sendable` and `isEnabled(_ id: String) -> Bool`; `TCDaemon.featureFlagsSnapshot()` via existing `tc_call`.

- [x] Write Swift decode tests for missing/unknown flags and malformed snapshots; unsupported/missing daemon method yields unavailable state with all queries false. Swift performs no profile evaluation, defaults table, or networking.
- [x] Add bridge snapshot request, AppModel initial read/event handling, and wake refresh signaling through existing lifetime-managed observers. Typed state updates occur on the main actor. Duplicate events do not regress a newer snapshot; compare a daemon-instance ID plus increasing snapshot generation, not manifest revision alone (expiry changes decisions at the same revision).
- [x] Add the build identity C accessor, header declaration, Swift decode and agreement tests. Keep existing channel agreement independent and required.
- [x] Propagate `TC_FEATURE_PROFILE` consistently to CLI and FFI builds and embed `TCFeatureEnvironment`/`TCFeatureProfileDigest` in the plist. Extend package verification to compare CLI, FFI, and plist. Preserve universal binary checks and check both architectures' identity.
- [x] Add packaging regressions: matching channel but mismatched profile rejected; testing plus production channel rejected; matching staging/testing profile accepted; omitted profile preserves existing production/staging selection. No existing feature is hidden or disabled by this task.

### Task 5: Validation, explicit publication, and operator instructions

**Interfaces:** `python3 scripts/feature-flags/publish.py --environment <env> --manifest <path> --cli <binary>` defaults to a dry-run report; `--publish` is explicit. Destination is fixed by environment, never an arbitrary command or URL. As with `scripts/campaigns/publish.py`, use `gcloud storage` with an optional already-authenticated `--account`; do not change account defaults.

- [x] Implement publisher by invoking the Rust CLI validator, validating publisher-known IDs against the source registry, checking target environment/revision/freshness, and showing the prior/new values. Revalidate immediately before upload, reject lower/same changed revision, use conditional atomic object replacement so concurrent publishers cannot silently overwrite each other, and set a bounded CDN cache lifetime (at most 60 seconds).
- [x] Use fixed `gs://` destinations corresponding exactly to Task 2's public HTTPS URLs. Read object metadata/content using existing gcloud credentials; generation zero represents an absent object. Upload the validated immutable local bytes with the observed generation as a precondition and cache control `public,max-age=60`. Confirm installed gcloud command syntax/help during implementation. Do not create buckets, change ACLs, grant roles, or publish during development. The guide must include read-only verification of public availability as a separate operational gate.
- [x] Write publisher tests using a fake deployment adapter: dry-run sends no writes; wrong environment/unknown IDs/stale revision rejected; compare-and-swap conflict rejected; successful publish changes only its fixed environment target. Record Git commit, manifest hash, revision, and value diff in command output.
- [x] Add CI invocations to existing Rust/client/script jobs without changing required-check names. Explicitly select production profile in production release paths and staging profile in staging paths; allow testing only as a staging build input. Ordinary build workflows never call publish.
- [x] Write the operator guide and index entry: add a registry entry/profile capability; gate UI and the behavior entry point; migrate storage compatibly; inspect current/pending decisions; publish higher-revision rollback; account for offline and restart behavior; retire flags after compatibility window. Document server integration and Windows/Linux UI follow-ups.

### Task 6: Whole-path verification and review

- [x] Add an end-to-end fixture using a test registry and injected local server/clock: supported flag off/on/off without rebuild, unsupported enable blocked, restart adopting pending values, and offline expiry reverting live flags. Verify CLI/IPC/FFI/Swift representations agree, including attached versus embedded daemon paths. Fixture gates stay out of the shipped registry.
- [x] Run final Rust tests once with fixed flags: `cargo test -p trace-commons-protocol`, `cargo test -p trace-commons-contributor`, `cargo test -p trace-commons-contributor-ffi`, and `cargo test -p trace-commons-server --test it -- 'license_boundary::'`. Include the staging profile/channel test build and compile-selected testing profile checks, plus production/testing refusal tests.
- [x] Run `cargo fmt --all -- --check`, relevant package Clippy with the repository allow-list, `cargo build -p trace-commons-contributor-ffi`, then `swift test` in `macos/` against that fresh dylib. Run publisher tests, profile packaging tests, and existing channel/release separation checks (including their self-tests where supported).
- [x] If dependency manifests changed despite the no-new-dependency design, audit the diff for authorization and run all four required cargo-deny license configurations. Never edit license-boundary expected sets to make a new edge pass.
- [x] Request a fresh whole-branch review. Address regressions and rerun affected verification only. Report the branch/worktree, tests, and separately any unfinished live serving, publication, deployment, or platform gates. No release or production flag change is authorized by local implementation approval.


## Completed validation and repair notes

- Final contributor suite: 5,515 passed, 10 ignored. FFI: 295 passed.
- Protocol: 676 passed. License boundary: 4 passed; no dependencies changed.
- Final native run: 3,518 XCTest cases executed (10 skipped), zero failures;
  eight Swift Testing cases passed against the freshly rebuilt FFI.
- Clippy, formatting, publisher/profile/CLI tests, plist checks and release
  separation checks passed. Testing-profile runtime tests and staging FFI tests
  passed; the incompatible production/testing profile failed compilation as intended.
- Independent review found missing flag refresh on native transport gaps. Its
  regression failed for both gap events before the handler fix and passed afterward.
- Native verification exposed an attached callback lifetime defect. The C ABI
  regression proved unsubscribe returned before an active callback finished.
  The repair gates queued readers, drains current and retired callbacks, preserves
  nonblocking callback-side clear, and disables failed subscription candidates.
  An additional focused review identified the replacement case; its regression
  also failed before the retained-sink fix and passed afterward. Full Rust/native
  suites passed after the finalized repair.
- Existing global manifest ignores required a narrow exception so the three
  reviewed, dated example manifests are tracked. They have not been published.
- No hosted CI run, merge, live flag publication, or production deployment was
  performed. Server runtime and Windows/Linux native UI integration remain outside v1.
