# NEAR AI saved accounts and managed sessions implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship `near-ai` and Trace Commons UI launches with saved accounts and one live, shared managed-session list, including sessions started in a terminal.

**Architecture:** A contributor-daemon account/session service owns defaults, immutable launch selections, and session lifecycle. The CLI supervises native tools using isolated profiles; desktop launchers invoke the same supervisor in an external terminal. IronWire supplies explicit profile config edits and isolated NEAR AI routing; native subscriptions stay with their native tools.

**Tech Stack:** Existing Rust workspace, local JSON IPC/C ABI, SwiftUI, WinUI/C#, GTK/Rust, platform native credential stores and terminal launchers.

**Spec:** `docs/superpowers/specs/2026-10-05-saved-model-accounts-design.md`

## Global Constraints

- CLI brand is **NEAR AI** and executable is `near-ai`; retain `trace-commons-contributor` as a compatible entry point using the same implementation.
- Every `near-ai launch` must register with the daemon and appear in the Trace Commons UI. No daemon means an actionable refusal, not an untracked launch.
- Selection changes apply to new sessions only. No fallback between accounts/providers; no daemon-global environment mutation.
- Subscription tokens remain in native storage and refresh through native tools. Persistent API keys require an OS secret store; unavailable storage fails closed.
- No new dependency without explicit human approval. No AGPL dependency in a permissive crate; leave the license-boundary expected sets unchanged.
- Managed launches do not grant contribution consent, arm folders, or collect terminal transcripts.
- Preserve external-config preview, parsing, occupied-slot, and stale-preimage protections.
- Show unknown liveness as unknown. Opening a terminal is not evidence that the native process started.
- Implementation happens in the existing isolated Trace Commons worktree. IronWire changes are authorized but must use an isolated upstream worktree after reading its full repository guidance.
- Preserve native byte passthrough, issuer-bound credentials, consent, and capability refusal in IronWire. Never invent quota or forge a client identity.
- macOS, Windows, and GTK are delivery scope; unsupported platform adapters remain visibly unavailable until verified.

## Review Focus

- An external terminal never redeems its launch ticket: the UI reconciles rather than retrying into two sessions (Tasks 2, 6).
- The daemon restarts while a CLI-launched session runs: the same row reconnects without losing its captured account (Tasks 2, 5, 10).
- Project settings or inherited credentials override a subscription: refuse the conflict without changing administrator policy (Tasks 3, 5).
- Two account labels match or differ only by case: do not select an unintended account; use exact unique labels or opaque IDs (Tasks 1, 5).
- A native process exits and its PID is reused: do not attach another process to the old session or allow unsafe account deletion (Tasks 2, 5).

## File and contract map

New contributor modules live under `src/managed/`: `mod.rs` (public types),
`accounts.rs` (metadata/defaults), `sessions.rs` (registry/tickets), `profiles.rs`
(native auth/config), `secrets.rs` (OS-store abstraction), `supervisor.rs`
(native process lifecycle), `terminal.rs` (external-terminal adapters), and
`copy.rs` (shared UI language). `src/daemon/managed.rs` contains IPC handlers.
`src/cli.rs` holds the existing CLI entry implementation shared by two thin
binary entry points. This limited extraction is required for the executable
rename; do not restructure unrelated daemon or command code.

Shared serializable types in `managed/mod.rs`:

- `ToolId::{Claude, Codex}`, `ConnectionKind::{NearAi, Subscription, ApiKey}`.
- `AccountId(Uuid)`, `SessionId(Uuid)` and `Generation(u64)`; opaque wire strings for IDs.
- `AccountView { id, tool, connection, label, auth_state, verified_at }` with no tokens or native credential paths.
- `Selection { tool, connection, account_id, generation }`.
- `LaunchRequest { request_id, tool, connection, account_id, cwd, expected_generation, save_default }`.
- `SessionView { id, tool, connection, account_id, account_label, selection_generation, project_label, cwd, started_at, state, exit_code, can_focus }`.
- `SessionState::{Starting, Running, Exited, Failed, Unknown}`; account readiness is distinct.
- `ManagedSnapshot { revision, accounts, defaults, sessions, capabilities }`.
- `ManagedError` is a fixed-code enum including `Conflict`, `NotFound`, `AmbiguousAccount`, `StorageUnavailable`, `UnsupportedVersion`, `SignInRequired`, `DaemonUnavailable`, `LaunchUnknown`.

`cwd` is permitted only in access-controlled local state and local IPC; events
carry the revision and opaque IDs rather than paths, labels, or secrets.
All interfaces below use these types and the existing `ConfigStore`.

## Task 1: Account metadata, selections, and CLI branding

**Files:** Create `crates/trace-commons-contributor/src/managed/{mod,accounts}.rs`, `src/cli.rs`, `src/bin/near-ai.rs`, `tests/managed_accounts.rs`, `tests/near_ai_cli.rs`; modify `src/lib.rs`, `src/bin/trace-commons-contributor.rs` and crate `Cargo.toml` only as needed for binary discovery.

**Interfaces:** `AccountStore::open(&ConfigStore) -> Result<AccountStore, ManagedError>`; `list() -> Vec<AccountView>`; `resolve(ToolId, &str) -> Result<AccountId, ManagedError>`; `select(Selection, Generation) -> Result<Selection, ManagedError>`; `cli::run() -> anyhow::Result<()>`. Both binary mains call the same entry implementation. Internal profile/secret references are not fields of `AccountView`.

- [ ] Write `saved_accounts_survive_restart`, `tool_defaults_are_independent`, `stale_generation_cannot_replace_default`, `ambiguous_label_requires_id`, and `account_views_never_serialize_secrets`. Assert two persisted accounts remain distinct and changing Claude leaves Codex unchanged.
- [ ] Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test managed_accounts`; record the missing-feature failure before implementation.
- [ ] Implement versioned metadata persistence using the repository's existing atomic-write/private-directory patterns. Keep account labels exact and allow duplicate labels only with ambiguity refusal at lookup.
- [ ] Write and run CLI tests first: both executable names accept existing arguments and return the same JSON shape, while new `near-ai --help` identifies NEAR AI. Extract the command parser/dispatch into `cli.rs` without behavior changes to existing commands.
- [ ] Run both new integration suites and existing CLI tests, then commit the scoped files.

## Task 2: Durable session registry and daemon IPC

**Files:** Create `src/managed/sessions.rs`, `src/daemon/managed.rs`, `tests/managed_sessions.rs`; modify `src/daemon/{mod,ipc}.rs` and managed module exports.

**Interfaces:** `SessionRegistry::prepare(LaunchRequest) -> Result<PreparedLaunch, ManagedError>`; `redeem(LaunchTicket, HelperIdentity) -> Result<LaunchContext, ManagedError>`; `report(SessionId, HelperProof, LifecycleReport) -> Result<(), ManagedError>`; `snapshot() -> ManagedSnapshot`. `PreparedLaunch` contains a session ID and expiring one-use ticket; Debug is redacted. `LaunchContext` is a helper-only response, excluded from normal snapshot/FFI access.

IPC methods: `managed_snapshot`, `managed_account_add`, `managed_account_rename`, `managed_account_reconnect`, `managed_account_remove`, `managed_select`, `managed_launch_prepare`, `managed_launch_redeem`, `managed_session_report`, `managed_session_dismiss`, `managed_session_focus`. Emit `managed_changed` with `{ revision }` after each committed visible transition. Feature methods must enter the existing advertised-method contract.

- [ ] Write `cli_preparation_is_visible_in_snapshot`, `duplicate_request_creates_one_session`, `changed_payload_cannot_reuse_request_id`, `ticket_is_single_use_and_expires`, `wrong_helper_cannot_report`, `restart_retains_session_account`, and `unknown_session_blocks_profile_removal`. Assert Starting is visible before redemption; Running only follows a valid child-start report.
- [ ] Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test managed_sessions` and record failures.
- [ ] Implement durable registry transitions under one service lock with bounded ticket lifetime, private session state, revision increments, authenticated local transport, and helper ownership checks. Persist a digest for helper reconnection proof, never a raw reusable token. Expiry of an unredeemed ticket proves no managed child started; uncertain redeemed tickets become Unknown.
- [ ] Add handler tests using the real `handle_local` dispatch: unsupported/malformed calls refuse; snapshots/events never contain ticket material or secrets. Account deletion/reconnect must consult session state, including Unknown.
- [ ] Run session tests and daemon IPC tests; commit.

## Task 3: Native profiles and saved credential lifecycle

**Files:** Create `src/managed/{profiles,secrets}.rs`, `tests/managed_profiles.rs`; extend account handlers from Task 2.

**Interfaces:** `NativeProfiles::begin_login(AccountId) -> Result<LoginLaunch, ManagedError>`; `inspect(AccountId) -> Result<AuthStatus, ManagedError>`; `prepare_environment(&Selection, &Path) -> Result<PreparedEnvironment, ManagedError>`; `remove_idle(AccountId) -> Result<(), ManagedError>`. `SecretStore::{put,get,delete}` takes an opaque account ID; returned secret values have redacted Debug and are never serializable.

- [ ] Write tests for two independent profile roots, reconnect preserving other accounts, cancellation preserving defaults, conflicting inherited API keys/base URLs, managed-policy refusal, locked OS storage, restrictive permissions, and path traversal/symlink refusal. Assert no native credential file is copied from the real home.
- [ ] Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test managed_profiles`; record failures.
- [ ] Implement profile-root selection and native-login adapters with child-only environment overrides. Use fixture native tools for automated tests. Inspect actual supported native-tool status/version interfaces before defining the minimum-version capability table; unknown versions do not gain a Ready label.
- [ ] Implement OS secret-store adapters using available platform capabilities with secrets on protected input, never argv. If a new dependency is necessary, present the exact dependency and reason for the repository-required approval before adding it. Do not substitute plaintext persistence.
- [ ] Run tests plus controlled native-version compatibility checks with user-driven login. Record independently verified profile/Keychain isolation; leave unverified adapters explicitly unavailable. Commit.

## Task 4: IronWire profile targets and NEAR route isolation

**Files (IronWire):** Modify `crates/ironwire_agents/src/{tools,claude_settings,codex_config}.rs`; `crates/ironwire_proxy/src/embed.rs` only where needed; add `crates/ironwire_agents/tests/profile_targets.rs`, `crates/ironwire_proxy/tests/managed_routes.rs`. Read full `docs/DESIGN.md`, `docs/TRUST.md`, and applicable guidance before modifying upstream.

**Interfaces:** Add `tools::plan_connect_at(id: &str, config_path: &Path, port: u16, catalog: &Catalog) -> Result<Planned, Error>` and `plan_disconnect_at(id: &str, config_path: &Path, catalog: &Catalog) -> Result<Planned, Error>`. Existing APIs retain discovered-default behavior. Contributor `ManagedRoute::start(&Selection) -> Result<ManagedRoute, ManagedError>` owns one isolated proxy home/lifetime for a NEAR session, with only its explicit NEAR secret source and no ambient credential discovery. `endpoint()` returns its loopback facade and `close()` ends it after native exit.

- [ ] Establish an isolated IronWire worktree from the revision selected for integration; inspect the difference between the contributor pin `6c5d628` and local upstream `b1ecde4` before choosing an update. Do not edit Cargo's cached checkout.
- [ ] Write tests that targeting profile A never reads/writes profile B or the real default, stale/unparseable/occupied config refuses, and external endpoint/auth settings are not overwritten. Run `cargo test -p ironwire_agents --test profile_targets` red before implementation.
- [ ] Implement explicit config target APIs, keeping existing callers unchanged. Add exact versioned upstream contract documentation and run the agent suite.
- [ ] Test two isolated embedded NEAR routes with different fixture keys and ambient native credentials. Assert each fixture upstream receives only its own key; revoked/missing credentials never fall through; native passthrough and capability refusal remain intact. Run `cargo test -p ironwire_proxy --test managed_routes` red, implement only missing isolation controls, then run relevant proxy conformance tests.
- [ ] Commit upstream changes. Integrate the exact revision into contributor `Cargo.toml`, root and GTK `Cargo.lock`, and GTK Flatpak `cargo-sources.json` using the existing vendor workflow. Run embed and profile integration tests against that revision before committing the pin.

## Task 5: CLI managed launch and native-process supervisor

**Files:** Create `src/managed/supervisor.rs`, `tests/managed_launch.rs`; modify `src/cli.rs`, `src/daemon/client.rs`, and managed IPC implementations.

**Interfaces:** `supervisor::run(store: ConfigStore, request: LaunchRequest) -> Result<i32, ManagedError>` and `supervisor::redeem_and_run(store: ConfigStore, ticket: LaunchTicket) -> Result<i32, ManagedError>`. CLI subcommands are `launch`, `accounts`, `sessions`, and hidden `redeem-launch`. Add typed account CRUD/default and session-list client wrappers over existing local IPC; no filesystem fallback when daemon is absent.

- [ ] Write a child fixture that reports its cwd/account marker and waits for input. `cli_launch_appears_in_daemon_snapshot` must spawn the actual `near-ai` executable against a fixture daemon and assert the same session ID is observable through UI-facing `managed_snapshot` while the child lives.

  Required assertions after the fixture child acknowledges startup (the test
  harness resolves `started_id` through the prepared launch response):

  ```rust
  let row = snapshot.sessions.iter().find(|s| s.id == started_id).unwrap();
  assert_eq!(row.state, SessionState::Running);
  assert_eq!(row.tool, ToolId::Claude);
  assert_eq!(row.account_id, personal_id);
  assert_eq!(row.cwd, project_dir);
  // After native exit and the helper's exit acknowledgement:
  let row = final_snapshot.sessions.iter().find(|s| s.id == started_id).unwrap();
  assert_eq!(row.state, SessionState::Exited);
  assert_eq!(row.exit_code, Some(7));
  ```
- [ ] Add tests for default/explicit cwd, exact unique label lookup, missing daemon, unsupported native tool, preserved native exit code, interrupt forwarding, restart/reconnect, two concurrent accounts, and PID reuse. Account/profile changes must not alter the first child's environment.
- [ ] Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test managed_launch` red, then implement the supervisor with inherited terminal I/O, native executable resolution, prepared account/route environment, and real lifecycle reports. Keep helper control credentials out of the native child's environment.
- [ ] Add CLI account add/list/rename/reconnect/remove/select and session-list commands using the shared service. Hidden input or explicit stdin accepts API keys; no secret argument flag. Test JSON redaction and non-interactive ambiguity errors.
- [ ] Re-run launch/account/CLI suites and daemon socket permission tests; commit. This task is incomplete if the real CLI process does not appear in the shared UI-facing snapshot.

## Task 6: UI launch handoff and terminal lifecycle

**Files:** Create `src/managed/terminal.rs`, `tests/managed_terminal.rs`; extend daemon managed handlers and contributor installation discovery as required.

**Interfaces:** `TerminalAdapter::launch(PreparedLaunch) -> Result<TerminalRef, ManagedError>`; `focus(&TerminalRef) -> Result<(), ManagedError>`; `capabilities() -> TerminalCapabilities`. Terminal references are opaque and platform-specific; no tool command assembled from arbitrary user text.

- [ ] Write tests for launch without acknowledgement, one-use ticket redemption, paths with spaces/metacharacters, terminal unavailable, duplicate submission, focus unavailable, and a terminal that exits before redemption. Assert focus never triggers a new launch.
- [ ] Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test managed_terminal` red.
- [ ] Implement terminal adapters invoking the absolute packaged `near-ai` helper plus opaque ticket. Resolve and validate the executable identity; terminal command construction is platform-tested and contains no secret environment. Report Starting until Task 5 acknowledges the child.
- [ ] Exercise platform handoff to a fixture interactive child, including UI close and daemon restart. Mark missing/unverified capabilities explicitly rather than optimistic success; commit.

## Task 7: Shared FFI surface and macOS UI

**Files:** Create contributor `src/managed/copy.rs`, `macos/Sources/TCShellCore/ManagedSessionsSurface.swift`, `macos/Sources/TraceCommonsApp/Views/ManagedSessionsView.swift`, `macos/Tests/TCShellCoreTests/ManagedSessionsSurfaceTests.swift`, `macos/Tests/TraceCommonsAppTests/ManagedSessionsTests.swift`; modify contributor FFI contracts/header as necessary, app `AppModel.swift`, `DaemonClient.swift`, and `Views/PrivateInferenceView.swift`.

**Interfaces:** `ManagedSessionsSurface` decodes `ManagedSnapshot`; `DaemonClient.managedSnapshot()` retrieves it; `AppModel.refreshManagedSessions()` replaces state only with a newer revision. Handle `managed_changed` with a fresh snapshot, plus initial load and reconnect refresh; missing events cannot permanently hide CLI sessions. Launch sheet creates `LaunchRequest`, then invokes prepare and terminal handoff exactly once.

- [ ] Write decoding tests for unknown enum values, missing capabilities, revision ordering, and secret-free payloads; unknown values remain unavailable/unknown, never Ready.
- [ ] Write app tests `cliSessionAppearsWithoutUILaunch`, `accountSwitchDoesNotRelabelRunningSession`, `reopenLoadsRunningSessions`, `launchUnknownDoesNotOfferBlindRetry`, and `addAccountReturnsToLaunchSheet`. Use real Rust-produced contract fixtures and an injected daemon client.
- [ ] Run `swift test --package-path macos --filter ManagedSessions` red after building the existing FFI dependency. Implement New session, native folder picker, connection/account picker, one-launch/default distinction, inline errors, and shared account management.
- [ ] Render Managed sessions with project/tool/account/connection/state, empty and unknown states, Show terminal and Launch another. Integrate event and reconnect refresh; UI launch is not a prerequisite for displaying a row.
- [ ] Run focused Swift and FFI tests; render and inspect launch sheet/session-list screenshots and keyboard accessibility. Verify a real fixture `near-ai launch` enters and exits the displayed list; commit.

## Task 8: Windows managed launch and session UI

**Files:** Create `windows/src/TraceCommons.Interop/ManagedSessionsSurface.cs`, `windows/src/TraceCommons.App/Controls/ManagedSessionsView.xaml` and `.xaml.cs`, `windows/src/TraceCommons.App/ViewModels/ManagedSessionsViewModel.cs`, `windows/tests/TraceCommons.Interop.Tests/ManagedSessionsTests.cs`; modify existing PrivateInference view/model and interop client to consume shared methods.

**Interfaces:** Same JSON contracts and revision semantics as Task 7; no independent credential or session database.

- [ ] Add failing tests for a CLI-originated snapshot row, immutable captured account, unknown status, event/reconnect refresh, and disabled unsupported capabilities. Run `dotnet test windows/tests/TraceCommons.Interop.Tests --filter ManagedSessions`.
- [ ] Implement New session, folder picker, account management, session list, and terminal actions using daemon state/copy. Keep names and statuses consistent with macOS.
- [ ] On Windows, verify the helper can redeem over the user-scoped named pipe and another OS user cannot. Exercise interactive launch, exit code, terminal focus capability, paths with spaces, credential-store ACLs, and daemon restart.
- [ ] Run Windows interop/UI build checks and existing named-pipe ACL verification, record runtime evidence, then commit. Cross-compilation alone does not complete the platform gate.

## Task 9: GTK managed launch and session UI

**Files:** Create `crates/trace-commons-contributor-gtk/src/ui/managed_sessions.rs`; modify GTK `src/{backend,model,update,worker}.rs`, `src/ui/{mod,private_inference}.rs`; add `tests/managed_sessions.rs` in the GTK workspace.

**Interfaces:** Same daemon methods, snapshots, and revision semantics as Tasks 7–8. Linux terminal/secret-store capabilities arrive from the backend rather than assumed by the view.

- [ ] Write failing tests for CLI-originated sessions, refresh/reconnect, captured account identity, unknown lifecycle state, launch errors, and unavailable credential storage. Run `cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --test managed_sessions`.
- [ ] Implement the launch form, folder selection, account management, session list and terminal actions. Read source-of-truth copy/status from the shared backend.
- [ ] Verify on Linux with a supported terminal and secret store; verify Flatpak host-launch constraints and refuse unsupported handoff clearly rather than escaping the sandbox implicitly.
- [ ] Run GTK tests/build and inspect rendered UI states; commit with platform evidence.

## Task 10: Packaging and end-to-end release gates

**Files:** Modify `.github/workflows/release-contributor.yml`, `.github/workflows/release-apps.yml`, `scripts/install.sh`, `scripts/install.ps1`, relevant package manifests/completion generation, and `README.md`; create `docs/operator/managed-sessions.md`, `scripts/ci/managed-launch-smoke.sh` and platform smoke equivalents where needed.

**Interfaces:** Packaged `near-ai` and the compatibility entry point resolve to the same implementation/config/daemon. The GUI locates the packaged helper explicitly. Existing contributor commands and configuration remain compatible.

- [ ] Add installer fixture tests for clean install, upgrade, legacy name retention, and unrelated `near-ai` name collision. Run them red before editing packaging/install scripts.
- [ ] Update release artifacts, installer checks, help/examples and completions. Do not overwrite an unrelated executable, rename the Rust crate, or migrate existing configuration locations solely for branding.
- [ ] Add a smoke test using the actual packaged CLI, daemon transport, native child fixture and each UI adapter: Starting then Running appears without a UI launch; switch account and start a second session; first identity is unchanged; exit is reflected; UI reopen and daemon restart reconcile the same IDs.
- [ ] Run appropriate Rust, FFI, Swift, Windows and GTK suites. Run `cargo fmt --all -- --check`, warning-denying contributor/FFI/server checks and repository clippy gates, and `cargo test -p trace-commons-server --test license_boundary` without editing its expected sets. Run all four `cargo deny` license variants for changed dependencies. Read current CI workflow for additional affected gates.
- [ ] Document exact native versions and platform limitations from compatibility checks. Separate fixture evidence from real provider sign-in/inference evidence; live model calls require an explicit test decision and must not happen implicitly during account listing.
- [ ] Perform a whole-branch review against every spec section, fix findings with regression tests, and commit documentation/evidence. Do not claim the feature complete until CLI-to-UI visibility and every advertised platform/account mode have passed their gates. Creating PRs, merging, or publishing requires the applicable user instruction.

## Execution handoff

Recommended method: native execution in this session, following the tasks in
order, with a whole-branch review. The contracts are tightly coupled and each
task depends on the daemon/account/session types above. Do not dispatch
implementation agents unless the user chooses that method or applicable
execution/review skills explicitly require them.

Review this plan before product implementation. The user's latest requirement
is pinned in Tasks 2, 5, 7, 8, 9, and 10: sessions managed through the CLI are
visible in the Trace Commons UI, not only sessions opened by the UI itself.
