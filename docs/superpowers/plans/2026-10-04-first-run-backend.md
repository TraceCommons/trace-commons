# First-Run Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the native macOS first run everything Ron's #1030 design needs that is not a view: the first-run copy, a past-session picker backend, folder recognition and a declared trajectory folder for "add your tool", and Swift client calls for the grant, the invite and Flow 1.

**Architecture:** One PR on `main`, independent of the glass stack. Rust daemon and core changes in `crates/trace-commons-contributor`, C exports in `crates/trace-commons-contributor-ffi`, and the Swift client layer only (`macos/Sources/TCBridge`, `TCShellCore`, `TraceCommonsApp/DaemonClient*.swift`). No view, no `AppModel` flow and no copy authored in Swift. The UI port (the replacement #1235, plan `2026-10-04-native-first-run-port.md`) consumes it.

**Tech Stack:** Rust 2024 (contributor crate, serde, the daemon IPC), the C ABI, Swift 6 package in `macos/` (XCTest). No new dependencies.

**Spec:** Ron's #1030 design (`origin/ftux:docs/superpowers/specs/2026-09-25-ftux-glass-flows-design.md`, screens in `origin/ftux:tauri-desktop/frontend/src/features/ftux/`), the owner decisions of 2026-09-28 (six points reconciling #1030 with the consent spec) and 2026-10-04 (below).

## Global Constraints

- Owner decisions 2026-10-04: the past-session picker's selection is a person-made `approve` that sends after the undo window and survives later mode changes; rows of never-queued sessions show date and size only (nothing is loaded before the person chooses); "Include every past session" is an explicit selection of each session, never `include_backlog`; "add your tool" is layout-only recognition for the parsed kinds plus a declared Letta Trajectory export folder, no new parsers, a folder that matches nothing is refused.
- The core owns copy. Swift authors no sentence (`ShellWordingTests.wordingBaseline` is floor and ceiling; a new Swift file gets zero). Every new core table is pinned in `crates/trace-commons-contributor-ffi/tests/swift_copy_surface_is_central.rs`.
- Hash-only and label-only: no session path, folder path or caller string in a log line, an audit row or an IPC response field other than the existing display fields. Session identity on the wire is an opaque id.
- Fail closed: an unknown id, project or folder is refused with a fixed label, never answered with an empty list.
- Discovery never opens a session file before the person consents (`source/discovery.rs` module doc). Folder recognition is layout-only.
- Both C header copies change together; `abi_header_surface.rs::the_macos_copy_is_a_byte_for_byte_copy` holds them equal (edit `crates/trace-commons-contributor-ffi/include/trace_commons.h`, then copy it to `macos/Sources/CTraceCommons/include/trace_commons.h`).
- Every new IPC method is added to `ipc::METHODS` (`daemon/ipc.rs:296`) and documented in `docs/contributor-daemon-ipc-v1_1.md`.
- Commit subjects are short imperatives, no prefixes, no emojis. AGPL headers are not needed (the contributor crates are MIT OR Apache-2.0). Tick the licensing attestation in the PR.
- Verify before the PR: `cargo fmt --all -- --check`; `cargo clippy -p trace-commons-contributor -p trace-commons-contributor-ffi --all-targets -- -D warnings` plus the repo allow-list; `cargo test -p trace-commons-contributor`; `cargo test -p trace-commons-contributor-ffi`; `cargo build -p trace-commons-contributor-ffi && (cd macos && swift test)`.

## Review Focus

1. **A session still being written is picked** (the person ticks today's session while the agent runs): it must be skipped with `session-still-active`, never sent half-written. Pinned in Task 3 (`include_skips_a_session_that_is_not_quiescent`).
2. **A folder switched to Never between listing and Continue**: the include must refuse every session of that folder (`project-mode-never`) and write no audit row. Pinned in Task 3 (`include_is_refused_after_the_folder_turns_never`).
3. **A picked folder that holds two kinds' layouts or none** (a home directory, an empty folder, `~/Downloads`): recognition returns every match or none, never a guess, and never opens a file. Pinned in Task 5 (`describe_folder_never_opens_a_file`, `describe_folder_reports_nothing_for_an_unrelated_folder`).
4. **A malformed or foreign id in `session_ids`** (an id from another project, a path, an empty string): refused as a whole with `session-id-unrecognized` before any approval. Pinned in Task 3 (`include_refuses_the_whole_call_on_one_foreign_id`).
5. **The first run lists before the first discovery pass** (Rules shown seconds after Folders started the daemon): the listing walks the declared sources itself, so it is complete without waiting for the watcher. Pinned in Task 2 (`list_past_sessions_lists_sessions_the_watcher_has_not_seen`).

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/trace-commons-contributor/src/first_run_copy.rs` (new) | Ron's #1030 first-run wording as one serializable table |
| `crates/trace-commons-contributor/src/daemon/past_sessions.rs` (new) | Listing past sessions per project and including a chosen subset; pure over the source adapters, queue, policy |
| `crates/trace-commons-contributor/src/daemon/ipc.rs` | Dispatch and the two handlers' request parsing; `grant_automatic` untouched |
| `crates/trace-commons-contributor/src/daemon/queue.rs` | `Queue::revive_expired` |
| `crates/trace-commons-contributor/src/source/discovery.rs` | `describe_folder` (layout-only recognition) |
| `crates/trace-commons-contributor/src/daemon/settings.rs`, `source/mod.rs` | `trajectory_source` declaration |
| `crates/trace-commons-contributor-ffi/src/lib.rs`, both `trace_commons.h` | `tc_first_run_copy_json`, `tc_describe_folder` |
| `macos/Sources/TCBridge/TCCoreCopy.swift`, `TCDiscovery.swift`, new `TCFlow1.swift`, new `TCInvite.swift` | Raw bridges |
| `macos/Sources/TCShellCore/FirstRunCopy.swift`, `PastSession.swift`, `Flow1.swift` (new) | Decoders and models |
| `macos/Sources/TCShellCore/FolderMatch.swift` (new) | Decodes `tc_describe_folder` keeping every row, trajectory included (Task 5; `SourceCandidate.Wire` widened from private to internal for it) |
| `macos/Sources/TraceCommonsApp/DaemonClient+FirstRun.swift` (new) | `listPastSessions`, `includePastSessions`, `grantAutomatic`, `inviteLookup` |

---

### Task 1: The first-run copy export

**Files:**
- Create: `crates/trace-commons-contributor/src/first_run_copy.rs` (register `pub mod first_run_copy;` in `src/lib.rs`)
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs` (beside `tc_onboarding_copy`, line 5244), both `trace_commons.h`
- Create: `macos/Sources/TCShellCore/FirstRunCopy.swift`; modify `macos/Sources/TCBridge/TCCoreCopy.swift`
- Test: `first_run_copy.rs` unit tests; `crates/trace-commons-contributor-ffi/tests/abi.rs`; `swift_copy_surface_is_central.rs`; `macos/Tests/TCBridgeTests/FirstRunCopyExportTests.swift` (new, pattern `OnboardingCopyExportTests.swift`); `ShellWordingTests.rustOwnedSurfaces`

**Interfaces:**
- Produces: `pub fn first_run_copy() -> FirstRunCopy` (`#[derive(serde::Serialize)]`, snake_case keys, `&'static str` fields, `{tool}`/`{host}`/`{count}`/`{folder}`/`{name}`/`{max}`/`{selected}`/`{total}` placeholders the shell fills); C `char* tc_first_run_copy_json(void);`; Swift `TCCoreCopy.firstRunCopyJSON() -> String?` and `public struct FirstRunCopy: Decodable` with `static func decode(_ json: String) -> FirstRunCopy?`.

The table holds Ron's visible strings verbatim, grouped as nested structs: `frame` (tier names "Quick setup", "Custom setup", step labels Join/Folders/Tools/Rules/Uses, "Custom setup instead", "Continue", "Answer every tool above to continue"), `join` (`join-screen.tsx` lines 21-236: title, body, invite eyebrow/placeholder/"Look up"/joined line/invite error from `ftux-api.ts:43`, passkey card, near.ai card, "Signed in", the no-sharing quiet line, skip note, "Skip: watch only", the signed-out notice `ftux-page.tsx:292`), `folders` (`tool-screens.tsx:73-99`, `tool-row.tsx`: title, body, loading line, "Watch this folder", "I don’t use it", "Choose a different folder for {tool}", "Get {tool}", "Install it, then this row asks again."), `tools` (`tool-screens.tsx:102-148`: title, add-tile line, "Added by you"; the add-tile caption is **replaced** by "OpenCode, a moved Claude Code or Codex folder, or a folder of exported traces" and a refusal "Trace Commons can't read this folder yet."), `rules` (`rules-screen.tsx:138-237`: title, loading, empty, "Repos found in {tools} sessions", "Past sessions, by folder", "{selected} of {total} selected", "Include every past session in {folder}", "Show all {count}", "Show fewer", "{count} · rule is Never"), `uses` (`uses-screen.tsx:32-217`: title, eyebrow, "required", "All optional uses", the optional-uses expander lines, the handle line and its caption, "Sharing", "Loading sharing copy…", "Sharing copy unavailable. Starting is disabled.", the footer note, "Start sharing"), `passkey` (`passkey-flow.tsx` P-1, P-2, P-5, P-7 and the two name errors from `ftux-model.ts:248-250`; P-3/P-4/P-6 are system sheets and have no row), `private_ai` (`private-ai-card.tsx` fallbacks only; the card's words stay `tc_private_inference_copy`).
Two departures from Ron's strings, both owner rules: no "Share automatically" anywhere (the Sharing and rule choices come from `tc_contribution_mode_copy_json`: Ask me / Automatic / Never), and no scope title or description here (they stay in the core's consent tables; that includes the handle line and its caption, which are the `public_attribution` scope's title and description, so the port reads them from `ScopeCopy`). The invite placeholder carries no code (Ron's preview showed a mock one). The joined line keeps Ron's pay range as a hole, `Joined {host} · {pay_range}`, so the shell fills the range and appends nothing. Placeholders in the table are pinned in `swift_copy_surface_is_central.rs` with each `{name}` replaced by the scanner's interpolation hole, so a Swift literal that re-authors one is caught. Drop the preview-only strings ("PREVIEW · MOCK DATA", "Simulated…", the preview error at `ftux-api.ts:106`).

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn first_run_copy_names_both_tiers_and_never_says_share_automatically() {
    let json = serde_json::to_string(&first_run_copy()).unwrap();
    assert!(json.contains("\"Quick setup\"") && json.contains("\"Custom setup\""));
    assert!(!json.contains("Share automatically"));
    assert!(!json.contains("Theia") && !json.contains("SSH"));
}
#[test]
fn every_first_run_string_is_non_empty() { /* walk serde_json::Value, assert no "" leaf */ }
```
In `abi.rs`: `tc_first_run_copy_json` returns JSON equal to `serde_json::to_value(first_run_copy())`. In `FirstRunCopyExportTests.swift`: `FirstRunCopy.decode(TCCoreCopy.firstRunCopyJSON()!)` is non-nil and `frame.quickSetup == "Quick setup"`.

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p trace-commons-contributor first_run_copy` — Expected: FAIL, unresolved module.

- [ ] **Step 3: Implement the table, the export, the header lines, the bridge and the decoder**

Copy the crate header to the macOS copy after editing. Add `add("first_run_copy::first_run_copy", table(serde_json::to_value(first_run_copy())))` to `pinned_sentences()` and `tc_first_run_copy_json` to `BRIDGE_ONLY` (the UI PR moves it to `SURFACES`). Add `FirstRunCopy.swift` to `rustOwnedSurfaces`.

- [ ] **Step 4: Run them to see them pass**

Run: `cargo test -p trace-commons-contributor first_run_copy && cargo test -p trace-commons-contributor-ffi && cargo build -p trace-commons-contributor-ffi && (cd macos && swift test --filter "FirstRunCopyExportTests|ShellWordingTests")` — Expected: PASS.

- [ ] **Step 5: Commit** — `git commit -m "Export Ron's first-run wording from the core"`

---

### Task 2: List a project's past sessions

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/past_sessions.rs` (register in `daemon/mod.rs`)
- Modify: `daemon/ipc.rs` (dispatch beside `"list_pending"` at `handle_request`, line ~2569, and in `handle_request_async`; `METHODS`)
- Test: `daemon/watcher.rs` test module (pattern `list_pending_filters_to_one_project_for_the_picker`, line 4889, with `WatcherFixture`, `write_session`, `settle`, `ipc_ok`)

**Interfaces:**
- Consumes: `TraceSource::discover()` via `all_sources(&settings.source_roots(..))`; `resolve_cwd` keying (`policy::project_key` for a cwd); `Queue` entries; `policy::project_key_for_id(id, &known_keys(..))`.
- Produces: `pub fn session_id_for(path: &Path) -> String` (sha256 over the path bytes, first 32 hex chars, the `project_id_for` shape) and `pub fn list_past_sessions(shared: &DaemonShared, project_key: &str, now: DateTime<Utc>) -> Vec<PastSessionRow>`; IPC `list_past_sessions {project_id}` → `{"sessions": [row], "total": n, "project_mode": "notify_only"|"auto_upload"|"ignore"|null}`.
  `PastSessionRow { session_id, entry_id: Option<String>, state: "pending"|"approved"|"expired"|"not_queued"|"never"|"still_active", selectable: bool, started_at: Option<_>, duration_secs: Option<i64>, title: Option<String>, size_bytes, source }`. Queued rows (`pending`, `approved`, `expired`) carry `entry_value`'s `started_at`, `duration_secs`, `title`; `not_queued` rows carry `SessionRef.started_at` (or file mtime) and `size_bytes` only, title and duration `null` (owner decision). Kept and dismissed sessions are not listed. A Never folder lists rows with `state: "never"`, `selectable: false`. A session that fails `eligibility::evaluate` lists as `still_active`, `selectable: false`. Sorted newest first.

- [ ] **Step 1: Write the failing tests**

`list_past_sessions_lists_queued_expired_and_unqueued_sessions` (three sessions in one project: one pending, one expired via `Queue::expire`, one written but never visited; assert three rows with the three states and that the unqueued row has `title == null`); `list_past_sessions_lists_sessions_the_watcher_has_not_seen` (declare the source, write sessions, call before any `settle`: rows present); `list_past_sessions_refuses_an_unknown_project` (`project-id-unrecognized`); `list_past_sessions_marks_a_never_folder_unselectable`; `list_past_sessions_puts_no_path_in_the_response` (serialize the response, assert the fixture's tmp dir string is absent).

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test -p trace-commons-contributor list_past_sessions` — Expected: FAIL, unknown method.

- [ ] **Step 3: Implement `list_past_sessions` and `handle_list_past_sessions(shared, req) -> Response`**

Walk the declared sources' `discover()` (not the cwd cache, which misses unvisited sessions), group by the session's project key, join against the queue by path, and never call `load`.

- [ ] **Step 4: Run them to see them pass** — same command, Expected: PASS.

- [ ] **Step 5: Commit** — `git commit -m "List a project's past sessions for the first-run picker"`

---

### Task 3: Include a chosen subset of past sessions

**Files:**
- Modify: `daemon/past_sessions.rs`, `daemon/queue.rs`, `daemon/ipc.rs`
- Test: `daemon/watcher.rs` test module; `daemon/queue.rs` tests

**Interfaces:**
- Consumes: Task 2's `session_id_for`, `list_past_sessions`; `watcher::tick_paths` (line 507) or the `visit_session` load path for `not_queued` rows; `Queue::approve` (line 1567) as a person; the `handle_approve` preview pinning (`build_and_pin_preview`) and its `bulk-approved` audit shape.
- Produces: `Queue::revive_expired(&mut self, entry_id: Uuid, now: DateTime<Utc>) -> bool` (Expired → Pending, `discovered_at = now`, as `undo_keep` dates its return); IPC `include_past_sessions {project_id, session_ids: [String]}` → `{"approved": n, "skipped": [{"session_id", "label"}]}`; audit row `past-sessions-included` (labels and counts only) written before the first approval.

Rules: validate the whole call first (unknown or foreign id → refuse all with `session-id-unrecognized`; Never folder → `project-mode-never`; a Never contribution override → the existing refusal); at most 500 ids per call (`too-many-sessions`); an explicit selection bypasses `load_can_land`'s 500-entry cap but never the quiescence check (`session-still-active` skip); kept and dismissed sessions are never revived; held-for-review entries are skipped (`held-for-review`) as `group_selection` does.

- [ ] **Step 1: Write the failing tests**

`include_approves_only_the_chosen_subset` (three sessions, include two, assert two `Approved`, one untouched); `include_survives_switching_the_folder_to_ask_me` (include, then `set_project_mode notify_only`, entries still approved); `include_skips_a_session_that_is_not_quiescent`; `include_is_refused_after_the_folder_turns_never`; `include_refuses_the_whole_call_on_one_foreign_id` (no entry approved, no audit row); `include_revives_an_expired_session`; `include_lands_past_the_queue_cap` (fill to `DEFAULT_MAX_QUEUE_ENTRIES`, include one unqueued session, assert approved); `include_writes_the_audit_row_first` (fail the audit write, assert nothing approved); `revive_expired_returns_false_for_a_kept_entry` (queue.rs).

- [ ] **Step 2: Run them to see them fail** — `cargo test -p trace-commons-contributor include_` — Expected: FAIL.

- [ ] **Step 3: Implement `revive_expired`, `include_past_sessions(shared, project_key, ids, now) -> IncludeOutcome` and `handle_include_past_sessions`**

- [ ] **Step 4: Run them to see them pass** — Expected: PASS, and `cargo test -p trace-commons-contributor` stays green.

- [ ] **Step 5: Commit** — `git commit -m "Approve a chosen subset of past sessions"`

---

### Task 4: Past sessions in the Swift client

**Files:**
- Create: `macos/Sources/TCShellCore/PastSession.swift`, `macos/Sources/TraceCommonsApp/DaemonClient+FirstRun.swift`
- Test: `macos/Tests/TraceCommonsAppTests/DaemonClientFirstRunTests.swift` (pattern: the existing `DaemonClient` request-encoding tests); `macos/Tests/TCBridgeTests/` integration against the real dylib (pattern `LiveDaemonClientIntegrationTests.testAnUnknownProjectIsTheDaemonsRefusalNotUnreachable`)

**Interfaces:**
- Produces: `public struct PastSession: Decodable, Identifiable { id (session_id), entryID: String?, state: State, selectable: Bool, startedAt: Date?, durationSecs: Int?, title: String?, sizeBytes: Int, source: String }` with `enum State: String { pending, approved, expired, notQueued = "not_queued", never, stillActive = "still_active" }` (unknown raw value decodes as `.never`, unselectable: fail closed); `public struct PastSessionList: Decodable { sessions: [PastSession]; total: Int; projectMode: String? }`; `struct IncludeOutcome: Decodable { approved: Int; skipped: [Skip] }`; `DaemonClient.listPastSessions(projectID: String) throws -> PastSessionList`, `DaemonClient.includePastSessions(projectID: String, sessionIDs: [String]) throws -> IncludeOutcome`, both over `call(_:params:as:)` (DaemonClient.swift:948).

- [ ] **Step 1: Write the failing tests** — `test_listPastSessionsSendsItsMethodAndProjectID`, `test_anUnknownStateIsUnselectable`, `test_includeSendsTheChosenIDsInOrder`, and the dylib test `test_anUnknownProjectIsRefusedNotEmpty`.
- [ ] **Step 2: Run** `cargo build -p trace-commons-contributor-ffi && (cd macos && swift test --filter "DaemonClientFirstRunTests")` — Expected: FAIL.
- [ ] **Step 3: Implement the models and the two methods.**
- [ ] **Step 4: Run** the same — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Call the past-session picker from the macOS client"`

---

### Task 5: Recognise a picked folder

**Files:**
- Modify: `crates/trace-commons-contributor/src/source/discovery.rs` (beside `describe_opencode`, line 207), ffi `lib.rs` (beside `tc_discover_opencode_export`, line 2225), both headers, `macos/Sources/TCBridge/TCDiscovery.swift`
- Test: `discovery.rs` tests; `abi.rs`; `macos/Tests/TCShellCoreTests/SourceCandidateTests.swift`

**Interfaces:**
- Produces: `pub fn describe_folder(path: &Path) -> Vec<SourceCandidate>` — runs each parsed kind's layout walk (Claude `<encoded-cwd>/<uuid>.jsonl`, Codex `YYYY/MM/DD/rollout-*.jsonl`, Gemini `<project>/chats/session-*.json`, Cline `<id>/<id>.messages.json`, OpenCode flat `*.json`, trajectory flat `*.json`/`*.jsonl`) and returns one candidate per kind whose layout matches, empty when none; never opens a file. C `char* tc_describe_folder(const char* path);` (JSON array; NULL only on a NULL or non-UTF-8 argument). Swift `TCDiscovery.describeFolderJSON(_ path: String) -> String?`, decoded with a new `FolderMatch.decodeList(from:)` (`macos/Sources/TCShellCore/FolderMatch.swift`) that keeps every row: `.source(SourceKind)`, `.trajectory`, or `.unrecognised(slug)`, so a flat `.json` folder reaches the shell as two matches to ask between and a `.jsonl`-only export as one trajectory match. Not `SourceCandidate.decodeList(from:)`, which drops a kind it cannot name and would turn the OpenCode/trajectory ambiguity into one confident OpenCode row. The Swift `SourceKind` gains no case; Task 6's `SessionRoots.trajectory` key is where a chosen trajectory match is declared.

- [ ] **Step 1: Write the failing tests** — one fixture per kind (`describe_folder_recognises_a_codex_store`, …); `describe_folder_reports_nothing_for_an_unrelated_folder`; `describe_folder_reports_both_kinds_for_a_flat_json_folder` (OpenCode and trajectory, the ambiguity the shell resolves by asking); `describe_folder_never_opens_a_file` (files with mode 000: the call still succeeds and counts them).
- [ ] **Step 2: Run** `cargo test -p trace-commons-contributor describe_folder` — Expected: FAIL.
- [ ] **Step 3: Implement** by reusing each adapter's existing walk predicates, as `describe_opencode` does; add the export, header lines and bridge.
- [ ] **Step 4: Run** the Rust tests, `cargo test -p trace-commons-contributor-ffi`, and `swift test --filter SourceCandidateTests` — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Recognise a picked folder by its layout"`

**Accepted deviations (Task 5, 2026-10-05; the OpenCode over-256 offer-then-refuse case is a named follow-up in the PR):**
- *Interface.* Decoded with the new `FolderMatch.decodeList(from:)`, not `SourceCandidate.decodeList(from:)` as first planned; the latter drops the trajectory row. Amended in the Interfaces line above and in `2026-10-04-native-first-run-port.md` Task 7.
- *One budget for every row.* `describe_folder`'s OpenCode row counts flat `*.json` under the same 65,536-entry budget as the trajectory row, not `describe_opencode`'s 256-entry count. With the smaller cap a large flat folder (`~/Downloads`) dropped the OpenCode row and returned one confident trajectory match. Consequence, not solved here: the folder screen can offer OpenCode for a folder of more than 256 entries that the OpenCode adapter then refuses at `discover()` (`opencode-discovery-entry-budget`). Whether that refusal should block the offer, as a separate refusal row or label, is an open design decision.
- *Claude Code stem rule.* The Claude Code layout requires a 36-character hyphenated UUID stem, which the adapter's own `discover` does not (it reads any top-level `*.jsonl` in a project dir). Intentional: a `.jsonl` two levels down is too common a shape to call a Claude Code store. Recognition can refuse a folder the adapter would read; real stores name sessions by UUID.
- *Stray top-level `.json`.* OpenCode and trajectory match by name suffix, as their layouts are defined, so a home directory or `~/Downloads` holding `package.json` or `.claude.json` reports both kinds and the shell asks rather than refusing. Dotfiles are not skipped, because the trajectory reader does not skip them. The Tools-screen port (`native-first-run-port` Task 7) should expect this.

---

### Task 6: A declared trajectory folder

**Files:**
- Modify: `daemon/settings.rs` (field beside `opencode_source`, line 475; `source_roots` line 1204; `source_settings_key` line 1334; the partial parser near line 1506; `get_settings` mode), `source/mod.rs` (`SourceRoots`, `all_sources`), `daemon/watcher.rs` (arming), `macos/Sources/TCShellCore/SessionRoots.swift` (settings JSON key)
- Test: `settings.rs` tests (pattern `opencode_export_declaration_is_explicit_and_old_settings_stay_off`); `ipc.rs` `the_settings_blob_reports_source_modes_and_never_a_source_path` (line 8037); `watcher.rs` staging no-arm tests; `macos/Tests/TCShellCoreTests/SessionRootsTests.swift`

**Interfaces:**
- Produces: `pub trajectory_source: Option<SourceDeclaration>` (absent builds nothing, `Undeclared::Nothing`; not part of `roots_declared`); `trajectory_source_mode` in `get_settings`; a declared trajectory folder is read with the strict trajectory reader alongside the staging folder; its sessions are **never armed for automatic upload** (same rule as staged trajectories, `watcher.rs:1323`): they always wait for a person. Swift `SessionRoots.trajectory: SourceChoice` emitted as `"trajectory_source"`.

- [ ] **Step 1: Write the failing tests** — `trajectory_declaration_is_explicit_and_old_settings_stay_off`; extend the never-a-path test to declare a trajectory folder and assert its path is absent and `trajectory_source_mode == "watch"`; `a_declared_trajectory_session_is_never_armed` (folder on Automatic, session waits for a person); `roots_declared_ignores_the_trajectory_folder`; Swift `test_trajectoryWatchEncodesItsKey`.
- [ ] **Step 2: Run** `cargo test -p trace-commons-contributor trajectory_` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the Rust and Swift tests — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Let a person declare a folder of exported traces"`

---

### Task 7: The grant, Flow 1 and the invite in the Swift client

**Files:**
- Create: `macos/Sources/TCBridge/TCFlow1.swift`, `macos/Sources/TCBridge/TCInvite.swift`, `macos/Sources/TCShellCore/Flow1.swift`
- Modify: `macos/Sources/TraceCommonsApp/DaemonClient+FirstRun.swift`; `swift_copy_surface_is_central.rs` (`BRIDGE_ONLY` already lists `tc_automatic_contribution_copy_json`; no change until a screen renders it)
- Test: `macos/Tests/TCBridgeTests/Flow1BridgeTests.swift`, `InviteHostBridgeTests.swift`; `DaemonClientFirstRunTests.swift`

**Interfaces:**
- Consumes: C `tc_flow1_grant_request_json(progress_json)` (ffi lib.rs:6078) → `{ready, blockers, witness_signing_address}`; `tc_invite_issuer_host(invite)` (lib.rs:1362, NULL on any rejection); daemon `grant_automatic {confirmed: true, witness_signing_address: String|null}` (ipc.rs:3468) → `{granted, granted_at, on_disk_recorded}` with refusals `automatic-grant-confirmation-required`, `automatic-grant-scopes-not-chosen`, `automatic-grant-witness-required`, `automatic-grant-witness-changed`, `arming-terms-unavailable`; daemon `invite_lookup` (main `ipc.rs`, served since #1187).
- Produces: `TCFlow1.grantRequestJSON(progressJSON: String) -> String?`; `public struct Flow1Progress: Encodable { connected, scopesSaved, path ("ask_first"|"automatic"), scrubDisclosureSeen, witnessDisclosureSeen, witnessShown: String? }` (snake_case keys matching core `flow1::Flow1Progress`) and `public struct Flow1GrantRequest: Decodable { ready: Bool; blockers: [String]; witnessSigningAddress: String? }`; `TCInvite.issuerHost(_ invite: String) -> String?`; `DaemonClient.grantAutomatic(witnessSigningAddress: String?) throws -> AutomaticGrant`, `DaemonClient.inviteLookup(_ invite: String) throws -> DaemonData.InviteLookup` (the model in `TCShellCore/DataContract/DaemonDataModels.swift:1322`).

- [ ] **Step 1: Write the failing tests** — `test_aGrantRequestIsNotReadyUntilScopesAreSaved` (encode progress with `scopesSaved: false`, assert `ready == false` and a blocker), `test_aMalformedInviteHasNoHost`, `test_aValidInviteNamesItsHost` (fixture invite from the ffi tests), `test_grantAutomaticSendsConfirmedAndTheWitness` (null witness encodes as JSON null, not omitted).
- [ ] **Step 2: Run** `cargo build -p trace-commons-contributor-ffi && (cd macos && swift test --filter "Flow1BridgeTests|InviteHostBridgeTests|DaemonClientFirstRunTests")` — Expected: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** the same — Expected: PASS.
- [ ] **Step 5: Commit** — `git commit -m "Reach the grant, Flow 1 and invite lookup from the macOS client"`

---

### Task 8: Document and verify

**Files:**
- Modify: `docs/contributor-daemon-ipc-v1_1.md` (method table and a section each for `list_past_sessions`, `include_past_sessions`, the `trajectory_source` declaration)

- [ ] **Step 1: Write the doc sections** (request, response, every refusal label, the no-path rule).
- [ ] **Step 2: Run the full gate** from Global Constraints — Expected: every command passes; paste the summaries into the PR.
- [ ] **Step 3: Open the PR** against `main`, titled "Give the first run its backend: past sessions, folder recognition, trajectory folders and client calls", linking #1030 and the UI plan; tick the licensing attestation.
- [ ] **Step 4: Commit** — `git commit -m "Document the first-run daemon methods"`
