# Imported conversations reach the desktop apps — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make conversations staged by `import-antigravity` visible, previewable and submittable from the macOS, GTK and Windows apps, labelled as Antigravity rather than as the adapter that stores them.

**Architecture:** The daemon's source list gains a staging-only trajectory scope, so its existing per-tick `discover()` sweep finds staged files with no new watcher plumbing. The origin already carried on `SessionRef.declared_source` is propagated into `QueueEntry`, over IPC, and into the three shells' labels. Entries from the staging scope always enter `Pending`, never `Approved`.

**Tech Stack:** Rust 2024 (`trace-commons-contributor`), Swift (macOS), Rust/GTK (`trace-commons-contributor-gtk`), C# (`TraceCommons.Interop`).

**Spec:** `docs/superpowers/specs/2026-09-02-daemon-staging-scope-design.md`

## Global Constraints

- `RUSTFLAGS="-D warnings"` for every `cargo check`/`cargo test`. Plain `cargo check` does not catch what CI catches.
- Clippy allow-list, never widened: `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
- `cargo fmt --all` before every commit. The repo is not rustfmt-clean, so check `git show --stat` after committing to confirm the hook did not rewrite whole files.
- No emojis in commits, PRs, code or comments. Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- `QueueEntry` is persisted to `daemon-queue.jsonl`. Every new field is `#[serde(default)]` and an existing queue file must still load.
- Hash-only/label-only logging. No paths, transcript text or identifiers in new log lines.
- The daemon must not gain any ability to enumerate processes or contact the Antigravity IDE. This plan only reads a directory.

**Ruling on the spec's open question:** entries discovered in the staging scope enter `Pending` even when the project is armed for auto-upload. The contributor armed a *watched source*, not an import they may have forgotten. This is implemented in Task 4 and is the one behaviour here that a reviewer should push back on if they disagree.

---

## File Structure

| File | Responsibility in this change |
|---|---|
| `crates/trace-commons-contributor/src/daemon/settings.rs` | `source_roots` takes the store and adds the staging scope |
| `crates/trace-commons-contributor/src/daemon/watcher.rs` | Call sites; populate `declared_source`; force `Pending` for staging entries |
| `crates/trace-commons-contributor/src/daemon/ipc.rs` | Call sites; emit `declared_source` in `entry_value` |
| `crates/trace-commons-contributor/src/daemon/preview_scheduler.rs` | Call site |
| `crates/trace-commons-contributor/src/daemon/queue.rs` | `QueueEntry.declared_source` |
| `crates/trace-commons-contributor/src/daemon/history.rs` | Two `QueueEntry` constructions |
| `macos/Sources/TraceCommonsApp/Models.swift` | Decode and prefer `declared_source` |
| `crates/trace-commons-contributor-gtk/src/model.rs` | Decode and prefer `declared_source`; map `antigravity` |
| `windows/src/TraceCommons.Interop/DaemonProtocol.cs` | Decode `declared_source` |
| `windows/src/TraceCommons.App/ViewModels/QueueEntryViewModel.cs` | Prefer `declared_source` |

---

### Task 1: The daemon reads the staging directory

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs:289`
- Modify (call sites): `watcher.rs:120,194,1013,1045,2026`, `ipc.rs:1899,2104`, `preview_scheduler.rs:629`, `settings.rs:597,615,658`
- Test: `crates/trace-commons-contributor/src/daemon/settings.rs` (tests module)

**Interfaces:**
- Produces: `DaemonSettings::source_roots(&self, store: &ConfigStore) -> SourceRoots`

- [ ] **Step 1: Write the failing test**

Add to the tests module in `settings.rs`:

```rust
/// The daemon reads the staging directory `import-antigravity` writes to.
///
/// It did not, and the reason given covered only half of what it excluded:
/// a service manager's working directory means nothing to a daemon, which
/// says nothing about a fixed path under the contributor's own 0700 state
/// directory. A contributor who imported and then opened a desktop app saw
/// nothing at all.
#[test]
fn the_daemon_reads_the_trajectory_staging_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::resolve(Some(dir.path().to_path_buf())).unwrap();
    let s = DaemonSettings::default();

    let names: Vec<&str> = crate::source::all_sources(&s.source_roots(&store))
        .iter()
        .map(|s| s.name())
        .collect();
    assert!(
        names.contains(&crate::source::SOURCE_TRAJECTORY),
        "the daemon must construct a trajectory source; got {names:?}"
    );
}

/// And ONLY the staging directory. The working-directory half of
/// `TrajectorySelection::Auto` stays off, which is what the original
/// exclusion was actually about: a daemon's cwd is whatever a service
/// manager handed it.
#[test]
fn the_daemon_does_not_read_its_own_working_directory() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::resolve(Some(dir.path().to_path_buf())).unwrap();
    let s = DaemonSettings::default();

    match &s.source_roots(&store).trajectory_selection() {
        crate::source::TrajectorySelection::Auto { working_dir, staging_dir } => {
            assert!(working_dir.is_none(), "the daemon must not scan its own cwd");
            assert_eq!(
                staging_dir.as_deref(),
                Some(store.dir().join(crate::source::TRAJECTORY_STAGING_SUBDIR).as_path())
            );
        }
        other => panic!("expected an Auto staging selection, got {other:?}"),
    }
}
```

`SourceRoots` has no accessor for its trajectory selection today. Add one in
`crates/trace-commons-contributor/src/source/mod.rs`, beside `is_declared`:

```rust
/// The trajectory scope this root set carries. Read by tests that need to
/// assert WHICH scope is in play, not merely that a trajectory source was
/// constructed.
pub fn trajectory_selection(&self) -> &TrajectorySelection {
    &self.trajectory
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib the_daemon_reads_the_trajectory_staging`
Expected: compile error (`source_roots` takes no argument), then after the signature change, a real assertion failure if the scope is wrong.

- [ ] **Step 3: Change the signature and add the scope**

In `settings.rs`, replace the body of `source_roots` and extend its doc:

```rust
    /// No working-directory trajectory scope: a daemon's working directory
    /// is whatever a service manager handed it, so auto-discovery would
    /// mean nothing there.
    ///
    /// The STAGING directory is a different thing and is included. It is a
    /// fixed path under the contributor's own state directory, resolved
    /// through `ConfigStore`, created 0700 and cleared by `logout`, holding
    /// only what `import-antigravity` put there on an explicit command.
    /// Excluding it made every imported conversation invisible to all three
    /// desktop apps -- no entry, no error, no empty state naming it.
    pub fn source_roots(&self, store: &ConfigStore) -> crate::source::SourceRoots {
        crate::source::SourceRoots::new()
            .declare(
                crate::source::SOURCE_CLAUDE_CODE,
                self.claude_source.clone(),
            )
            .declare(crate::source::SOURCE_CODEX, self.codex_source.clone())
            .declare(crate::source::SOURCE_GEMINI_CLI, self.gemini_source.clone())
            .with_trajectory(crate::source::TrajectorySelection::Auto {
                working_dir: None,
                staging_dir: Some(
                    store.dir().join(crate::source::TRAJECTORY_STAGING_SUBDIR),
                ),
            })
    }
```

- [ ] **Step 4: Update every call site**

Production sites take `&shared.store`; the in-test sites take `&self.shared.store`:

- `watcher.rs:120` → `(s.max_queue_entries, s.source_roots(&shared.store))`
- `watcher.rs:194` → same (`shared` is in scope)
- `watcher.rs:1013,1045,2026` → `(s.max_queue_entries, s.source_roots(&self.shared.store))`
- `ipc.rs:1899,2104` → `(s.near_ai.clone(), s.source_roots(&shared.store))`
- `preview_scheduler.rs:629` → `(s.near_ai.clone(), s.source_roots(&shared.store))`
- `settings.rs:597` → `loaded.source_roots(&store)` (a `store` is already in scope)
- `settings.rs:658` → `s.source_roots(&store)`; build one from a `tempfile::tempdir()` if the test has none

At `ipc.rs`/`preview_scheduler.rs`/`watcher.rs` the settings lock is held while
`&shared.store` is read. `store` is a separate field, so there is no lock
ordering concern; do not restructure to release the lock first.

- [ ] **Step 5: Fix the one test whose expectation genuinely changes**

`settings.rs:615` asserts the constructed source list is exactly
`vec![SOURCE_GEMINI_CLI]`. The daemon now also constructs a trajectory
source, so update it and say why:

```rust
        let names: Vec<&str> = crate::source::all_sources(&s.source_roots(&store))
            .iter()
            .map(|s| s.name())
            .collect();
        // The trajectory source is always constructed now: the daemon reads
        // the staging directory. It is listed after the native adapters
        // because `all_sources` appends it.
        assert_eq!(
            names,
            vec![crate::source::SOURCE_GEMINI_CLI, crate::source::SOURCE_TRAJECTORY]
        );
```

- [ ] **Step 6: Run the full lib suite**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib`
Expected: PASS. Investigate any other test that enumerates source names rather than editing its expectation reflexively — a changed expectation must be explained in the test, as above.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Give the daemon the trajectory staging scope"
git show --stat HEAD
```

---

### Task 2: `QueueEntry` carries the declared origin

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/queue.rs:51` (struct)
- Modify: `crates/trace-commons-contributor/src/daemon/watcher.rs:550`
- Modify: `crates/trace-commons-contributor/src/daemon/history.rs:133,215`
- Test: `crates/trace-commons-contributor/src/daemon/queue.rs` (tests module)

**Interfaces:**
- Consumes: `SessionRef::declared_source` (added in #519)
- Produces: `QueueEntry.declared_source: Option<String>`

- [ ] **Step 1: Write the failing test**

In the `queue.rs` tests module:

```rust
/// An existing queue file has no `declared_source`, and must still load.
///
/// `daemon-queue.jsonl` is persisted state on every contributor's machine.
/// A new required field would make the daemon refuse its own queue on the
/// first run after an upgrade.
#[test]
fn a_queue_entry_written_before_declared_source_still_loads() {
    let line = r#"{"entry_id":"6f1a5f1e-0000-4000-8000-000000000000","session_hash":"sha256:x","source":"claude-code","project_key":"/p","project_label":"p","path":"/p/s.jsonl","size_bytes":1,"discovered_at":"2026-09-02T00:00:00Z","state":"pending","reason_label":null,"attempts":0,"retry_after":null,"submission_id":null,"approved_scopes":null,"approved_verdict":null,"approved_correction":null,"approved_inputs":null,"previewed_envelope_digest":null,"approved_at":null,"subagent_count":0,"subagents_dropped":0}"#;
    let entry: QueueEntry = serde_json::from_str(line).expect("an old queue entry must load");
    assert_eq!(entry.declared_source, None);
}
```

If the literal above does not match the current serialized shape, generate it
rather than hand-editing: construct a `QueueEntry`, `serde_json::to_string` it,
paste the result, then delete the `declared_source` key. Do not guess field
names.

- [ ] **Step 2: Run it to verify it fails**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib a_queue_entry_written_before_declared_source`
Expected: FAIL — `no field declared_source`.

- [ ] **Step 3: Add the field**

In `queue.rs`, after `pub source: String,`:

```rust
    /// What the transcript says it came from, when discovery knew it.
    /// Display only, and never a substitute for `source`.
    ///
    /// `source` is the ADAPTER. An imported Antigravity conversation is
    /// stored as a trajectory file and read by the `trajectory` adapter, so
    /// `source` says `trajectory` -- a word for how it is stored, not where
    /// it came from, and not the word the contributor typed to collect it.
    ///
    /// `#[serde(default)]` because `daemon-queue.jsonl` written before this
    /// field exists must still load.
    #[serde(default)]
    pub declared_source: Option<String>,
```

- [ ] **Step 4: Populate it at every construction site**

`watcher.rs:550`, inside the `QueueEntry { .. }` literal, after `source`:

```rust
        source: session_ref.source.to_string(),
        declared_source: session_ref.declared_source.clone(),
```

`history.rs:133` and `history.rs:215` construct entries from a record that has
no `SessionRef`. Set `declared_source: None` at both and note why:

```rust
                // History records predate this field and carry no origin of
                // their own; None is the honest answer, not a guess.
                declared_source: None,
```

- [ ] **Step 5: Run the tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib`
Expected: PASS. Compiler errors will point at any construction site missed.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Carry the declared origin on a queue entry"
```

---

### Task 3: The origin reaches the apps over IPC

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs:655` (`entry_value`)
- Test: `crates/trace-commons-contributor/src/daemon/ipc.rs` (tests module)

**Interfaces:**
- Produces: IPC pending entries gain `"declared_source"` beside `"source"`

- [ ] **Step 1: Write the failing test**

```rust
/// The origin has to cross the IPC boundary, not merely exist on the ref.
///
/// The equivalent hand-off is exactly what broke while `declared_source`
/// was being added in #519, so it is asserted at the boundary rather than
/// one layer below it.
#[test]
fn an_entry_reports_both_its_adapter_and_its_declared_origin() {
    let mut entry = sample_entry();
    entry.source = "trajectory".to_string();
    entry.declared_source = Some("antigravity".to_string());

    let v = entry_value(&entry);
    assert_eq!(v["source"], "trajectory", "the adapter must stay reportable");
    assert_eq!(v["declared_source"], "antigravity");
}

/// A native session declares nothing, and must not grow an empty label.
#[test]
fn an_entry_with_no_declared_origin_reports_null() {
    let entry = sample_entry();
    let v = entry_value(&entry);
    assert!(v["declared_source"].is_null());
}
```

Use whatever entry-building helper the `ipc.rs` tests already have; if there is
none, build a `QueueEntry` literal in the test rather than adding a helper for
two tests.

- [ ] **Step 2: Run to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib an_entry_reports_both_its_adapter`
Expected: FAIL — `declared_source` is null / missing.

- [ ] **Step 3: Emit the field**

In `entry_value`, after the `"source"` line:

```rust
        "source": e.source,
        // Beside `source`, never replacing it: a consumer uses the adapter
        // name to ask for the same session again.
        "declared_source": e.declared_source,
```

- [ ] **Step 4: Run the tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Report the declared origin over the daemon IPC"
```

---

### Task 4: A staged conversation always needs a decision

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/watcher.rs:546`
- Test: `crates/trace-commons-contributor/tests/daemon_reads_staged_imports.rs` (created in Task 5)

**Order note:** Task 5 creates the integration file this task's test lives in.
If executing strictly in order, write Task 5 first and fold this test into it;
the split here is by behaviour, not by file.

**Interfaces:**
- Consumes: `QueueEntry.declared_source` (Task 2)

- [ ] **Step 1: Write the failing test**

```rust
/// A conversation found in the staging directory is offered, never
/// auto-uploaded, even for a project armed for auto-upload.
///
/// This is a consent judgement, not a mechanical one. A contributor who
/// armed auto-upload did so for a watched source they had declared. An
/// imported conversation was invisible to this daemon until it upgraded;
/// taking it straight to Approved would upload, with no further prompt,
/// something they may not remember importing.
#[tokio::test]
async fn a_staged_trajectory_is_offered_even_when_the_project_is_armed() {
    // Same setup as tests/daemon_reads_staged_imports.rs (Task 5), plus:
    // arm the project the staged conversation's `meta.cwd` names.
    shared.policy.lock().unwrap().set_mode(
        "/Users/testuser/code/demo",
        ProjectMode::AutoUpload,
        chrono::Utc::now(),
    );

    // ... two ticks ...

    let queue = shared.queue.lock().unwrap();
    let entry = &queue.all()[0];
    assert_eq!(
        entry.state,
        QueueState::Pending,
        "an armed project must not arm an imported conversation"
    );

    // And the control: a native session in the same armed project DOES
    // arm, so this test is about the staging scope and not about the
    // arming path being broken outright.
}
```

Write this in the same integration file as Task 5 rather than as a unit test:
arming is a policy-plus-watcher interaction, and the unit-level watcher tests
do not carry a `ProjectPolicy`. The control assertion at the end is not
optional -- without it, a bug that disarmed everything would pass.

- [ ] **Step 2: Run it to verify it fails**

Expected: FAIL with `state: Approved`.

- [ ] **Step 3: Implement**

At `watcher.rs:546`, replace the `armed` binding:

```rust
    // A staged trajectory is never armed, whatever the project mode says.
    //
    // The daemon's only trajectory scope is the staging directory (see
    // `DaemonSettings::source_roots`), so a trajectory ref here IS an
    // import. It was invisible to this daemon before the staging scope
    // existed, and auto-uploading on first sight would send something the
    // contributor may not remember importing, with no prompt. They armed a
    // watched source; this is not one.
    let from_staging = session_ref.source == crate::source::SOURCE_TRAJECTORY;
    let armed = mode == ProjectMode::AutoUpload && !from_staging;
```

- [ ] **Step 4: Run the tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --lib`
Expected: PASS, including the existing armed-project tests, which use native sources and must be unaffected.

- [ ] **Step 5: Mutation-check the new test**

Temporarily drop `&& !from_staging`, re-run the new test, confirm it FAILS, then restore. A test that cannot fail is worse than no test.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Offer a staged conversation rather than arming it"
```

---

### Task 5: The end-to-end gap, as a test

**Files:**
- Create: `crates/trace-commons-contributor/tests/daemon_reads_staged_imports.rs`

- [ ] **Step 1: Write the test**

This is the test that fails on `main` today and is the reason the change
exists. It drives the real daemon tick, not a unit-level helper. The harness
below is the shape `tests/daemon_end_to_end_upload.rs` uses, reduced to what
this needs: no stub issuer or ingest, because nothing is uploaded.

```rust
//! A conversation staged by `import-antigravity` reaches the daemon's queue.
//!
//! This failed before the daemon had a staging scope, and failed silently:
//! no error, no empty state, the conversation simply did not exist as far
//! as any desktop app was concerned.

#![cfg(unix)]

use std::sync::Arc;

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};
use trace_commons_contributor::daemon::ipc::DaemonShared;
use trace_commons_contributor::daemon::queue::QueueState;
use trace_commons_contributor::identity::DeviceIdentity;

/// One staged conversation, in the shape `import-antigravity` writes: a
/// JSON array whose first record is the meta. Taken from
/// `antigravity/convert.rs` rather than invented, so this breaks if the
/// converter's output drifts.
fn staged_conversation() -> String {
    serde_json::json!([
        {"role": "meta", "source": "antigravity", "cwd": "/Users/testuser/code/demo"},
        {"role": "user", "content": "Tell me about this repo",
         "timestamp": "2026-08-30T10:00:00Z"},
        {"role": "assistant", "content": "It is a contributor client.",
         "timestamp": "2026-08-30T10:00:05Z"}
    ])
    .to_string()
}

#[tokio::test]
async fn a_staged_conversation_reaches_the_queue_as_antigravity() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let device = DeviceIdentity::load_or_generate(&store).unwrap();
    store
        .save_config(&ContributorConfig {
            schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id,
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: Some("127.0.0.1".into()),
            display_handle: None,
            public_bio: None,
            public_since: None,
        })
        .unwrap();

    // Exactly where the import writes: the staging folder inside the state
    // directory. Nothing declares it -- that is the point.
    let staging = store
        .dir()
        .join(trace_commons_contributor::source::TRAJECTORY_STAGING_SUBDIR);
    std::fs::create_dir_all(&staging).unwrap();
    std::fs::write(staging.join("conversation.json"), staged_conversation()).unwrap();

    let shared = Arc::new(DaemonShared::load(store).unwrap());

    // Two ticks: a first sighting is deliberately unstable.
    let now: chrono::DateTime<chrono::Utc> = "2030-01-01T00:00:00Z".parse().unwrap();
    trace_commons_contributor::daemon::watcher::tick(&shared, now)
        .await
        .unwrap();
    trace_commons_contributor::daemon::watcher::tick(&shared, now)
        .await
        .unwrap();

    let queue = shared.queue.lock().unwrap();
    let entries = queue.all();
    assert_eq!(
        entries.len(),
        1,
        "the staged conversation must be queued; got {entries:?}"
    );
    let entry = &entries[0];

    assert_eq!(
        entry.source, "trajectory",
        "the adapter that loads it is still what `source` names"
    );
    assert_eq!(
        entry.declared_source.as_deref(),
        Some("antigravity"),
        "but the queue must carry what the conversation declares itself to be"
    );
    assert_eq!(
        entry.state,
        QueueState::Pending,
        "an imported conversation is offered, never armed"
    );
}
```

If `DaemonShared::load` or `ConfigStore::open` has drifted, copy the current
form from `tests/daemon_end_to_end_upload.rs::Harness::new` rather than
adapting this by guesswork.

- [ ] **Step 2: Run it**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --test daemon_reads_staged_imports`
Expected: PASS (Tasks 1-4 are in). Then `git stash` Task 1's settings change, re-run, and confirm it FAILS — proving the test covers the gap and not something else. Restore afterwards with `git stash apply <sha>`; per this repo's rule never use bare `git stash pop`.

- [ ] **Step 3: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Pin that the daemon queues a staged conversation"
```

---

### Task 6: The three shells show the origin

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Models.swift:64-90`
- Modify: `crates/trace-commons-contributor-gtk/src/model.rs:83-134`
- Modify: `windows/src/TraceCommons.Interop/DaemonProtocol.cs:281`
- Modify: `windows/src/TraceCommons.App/ViewModels/QueueEntryViewModel.cs:142`
- Test: each shell's existing label-map test

**Interfaces:**
- Consumes: the IPC `declared_source` field (Task 3)

Batch this as one dispatch: three small same-shaped edits plus their tests.

- [ ] **Step 1: macOS**

Add the property and coding key:

```swift
    let declaredSource: String?
```
```swift
        case declaredSource = "declared_source"
```

and change `agentName` to read the origin first:

```swift
    /// "Claude Code" / "Antigravity", never the raw token.
    ///
    /// Prefers what the transcript declares over the adapter that stores
    /// it: an imported Antigravity conversation is a trajectory FILE, and
    /// calling it "Letta trajectory" names the format rather than the tool
    /// the contributor used.
    var agentName: String {
        switch declaredSource ?? source {
        case "claude-code", "claude_code": return "Claude Code"
        case "codex": return "Codex"
        case "gemini-cli", "gemini_cli": return "Gemini CLI"
        case "antigravity": return "Antigravity"
        case "trajectory", "letta_trajectory": return "Letta trajectory"
        default:
            return source
                .replacingOccurrences(of: "_", with: " ")
                .replacingOccurrences(of: "-", with: " ")
                .capitalized
        }
    }
```

Note the `default` branch still uses `source`, not the coalesced value: an
unrecognised declared value is untrusted text from a file, and title-casing it
into the UI is a different decision from mapping a known slug. Keep it.

- [ ] **Step 2: GTK**

```rust
    /// What the transcript declares itself to be, when discovery knew it.
    #[serde(default)]
    pub declared_source: Option<String>,
```
```rust
    pub fn agent_label(&self) -> &str {
        match self.declared_source.as_deref().unwrap_or(self.source.as_str()) {
            "claude-code" => "Claude Code",
            "codex" => "Codex",
            "gemini-cli" => "Gemini CLI",
            "antigravity" => "Antigravity",
            "trajectory" => "Trajectory",
            _ => self.source.as_str(),
        }
    }
```

GTK is the one shell whose fallback returns the raw slug rather than
title-casing it, which is why `antigravity` gets an explicit arm here.

- [ ] **Step 3: Windows**

```csharp
    [JsonPropertyName("declared_source")]
    public string? DeclaredSource { get; set; }
```

and in `QueueEntryViewModel.cs`, switch the label expression's subject from
`Source` to `DeclaredSource ?? Source`, adding:

```csharp
        "antigravity" => "Antigravity",
```

- [ ] **Step 4: Tests, one per shell**

Each shell already has a label-map test. Add to each: an entry whose adapter is
`trajectory` and whose declared origin is `antigravity` renders "Antigravity",
and an entry with no declared origin still renders from the adapter.

- [ ] **Step 5: Run all three suites**

```bash
cargo build -p trace-commons-contributor-ffi
(cd macos && swift test)
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
TC_FFI_LIB_DIR="$PWD/target/release" dotnet test windows/tests/TraceCommons.Interop.Tests
```

`dotnet test` needs `cargo build -p trace-commons-contributor-ffi --release`
first, in THIS worktree — a release dylib built elsewhere will not be found and
produces a wall of `DllNotFoundException` that looks like a code failure.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Show the declared origin in the three desktop shells"
```

---

### Task 7: Documentation

**Files:**
- Modify: `crates/trace-commons-contributor/README.md:249-256`

- [ ] **Step 1: State the new behaviour and its latency**

The README currently describes `import-antigravity` as staging for `submit`.
Add, in the Antigravity bullet:

- imported conversations now also appear in the desktop apps' queue;
- they appear on the daemon's next sweep rather than instantly, so an app
  opened immediately after an import may show nothing for a poll interval;
- they always arrive needing approval, even for a project set to auto-upload.

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "Say that imported conversations reach the apps"
```

---

## Final verification

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --no-default-features
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo build -p trace-commons-contributor-ffi && (cd macos && swift test)
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cargo build -p trace-commons-contributor-ffi --release
TC_FFI_LIB_DIR="$PWD/target/release" dotnet test windows/tests/TraceCommons.Interop.Tests
```

Capture a test baseline BEFORE Task 1 and compare the final failure count
against it. Never report "tests pass" from a filtered run.
