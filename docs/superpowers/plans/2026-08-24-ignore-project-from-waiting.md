# Ignore a project from the Waiting screen — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a contributor decline a whole project from the Waiting screen, and make ignoring a project actually clear the traces already queued from it.

**Architecture:** The defect is fixed in the daemon — `set_project_mode` purges `Pending` entries when the new mode is `Ignore` — so Settings, onboarding and the CLI inherit it. Each of the three shells then gains an `Ignore project` button beside the existing `Submit all` on the project header, behind a confirmation naming the count.

**Tech Stack:** Rust (daemon, GTK shell), Swift/SwiftUI (macOS shell), C#/WinUI 3 (Windows shell).

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-24-ignore-project-from-waiting-design.md`.
- Purge scope is **`Pending` only**. `Approved` and `Uploading` entries are never touched.
- The new reason label is `"project-ignored"` and MUST NOT be `REASON_DISMISSED` — that label is path-keyed and permanent (`Queue::dismissed_at_path`), so reusing it would make "Ask again" restore nothing.
- No new IPC method. `set_project_mode` gains a `purged` field in its response; existing clients ignoring it keep working.
- Hash-only logging: never log a filesystem path, tenant identity, or trace content. `project_label` is contributor-facing and safe; `project_key` is a path and is not.
- No emojis anywhere, including commit messages.
- Commit style: short imperative subject, no `feat:`/`fix:` prefix.
- Do not modify any `Cargo.lock`. Check `git show --stat` before every commit.
- GTK is a **separate cargo workspace**: repo-root `cargo fmt --all` and `clippy --workspace` do not see it. Verify it separately.
- Windows XAML compiles only in CI. Say so in the PR rather than implying local verification.

---

### Task 1: Daemon — the purge primitive

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/queue.rs` (add const near `REASON_TOO_LARGE` at :272; add method near `set_state` at :462)
- Test: same file, `mod tests`

**Interfaces:**
- Consumes: `QueueEntry { project_key: String, state: QueueState, reason_label: Option<String> }`, `QueueState::{Pending, Approved, Uploading, Refused}`
- Produces: `pub const REASON_PROJECT_IGNORED: &str = "project-ignored";` and `Queue::refuse_pending_for_project(&mut self, project_key: &str) -> usize` (returns how many entries moved)

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `queue.rs`:

```rust
#[test]
fn ignoring_a_project_refuses_only_its_pending_entries() {
    let mut q = Queue::default();
    q.entries.push(entry_in("/w/alpha", QueueState::Pending));
    q.entries.push(entry_in("/w/alpha", QueueState::Approved));
    q.entries.push(entry_in("/w/alpha", QueueState::Uploading));
    q.entries.push(entry_in("/w/beta", QueueState::Pending));

    let purged = q.refuse_pending_for_project("/w/alpha");

    assert_eq!(purged, 1, "only the pending entry moves");
    let alpha: Vec<_> = q.all().iter().filter(|e| e.project_key == "/w/alpha").collect();
    assert_eq!(alpha[0].state, QueueState::Refused);
    assert_eq!(alpha[0].reason_label.as_deref(), Some(REASON_PROJECT_IGNORED));
    assert_eq!(alpha[1].state, QueueState::Approved, "an approval is not retracted");
    assert_eq!(alpha[2].state, QueueState::Uploading, "an in-flight upload is not touched");
    let beta: Vec<_> = q.all().iter().filter(|e| e.project_key == "/w/beta").collect();
    assert_eq!(beta[0].state, QueueState::Pending, "another project is untouched");
}

#[test]
fn a_project_ignore_is_not_a_dismissal() {
    // REASON_DISMISSED is path-keyed and permanent. If project-ignore used
    // it, "Ask again" would restore nothing, because every purged session
    // would still be suppressed individually at its path.
    let mut q = Queue::default();
    let e = entry_in("/w/alpha", QueueState::Pending);
    let path = e.path.clone();
    q.entries.push(e);

    q.refuse_pending_for_project("/w/alpha");

    assert_ne!(REASON_PROJECT_IGNORED, REASON_DISMISSED);
    assert!(
        !q.dismissed_at_path(&path),
        "a project ignore must not suppress the path the way a dismissal does"
    );
}

#[test]
fn ignoring_a_project_with_nothing_pending_purges_nothing() {
    let mut q = Queue::default();
    q.entries.push(entry_in("/w/alpha", QueueState::Approved));
    assert_eq!(q.refuse_pending_for_project("/w/alpha"), 0);
}

#[test]
fn a_pipeline_refusal_keeps_its_own_reason() {
    // `Refused` has more than one author -- the pipeline refuses for a
    // residual secret or an unavailable privacy filter. Ignoring the project
    // must not overwrite why one of those was refused.
    let mut q = Queue::default();
    let mut refused = entry_in("/w/alpha", QueueState::Refused);
    refused.reason_label = Some("residual-secret".to_string());
    q.entries.push(refused);

    assert_eq!(q.refuse_pending_for_project("/w/alpha"), 0);
    assert_eq!(q.all()[0].reason_label.as_deref(), Some("residual-secret"));
}
```

Add this helper beside the other test helpers in that module:

```rust
fn entry_in(project_key: &str, state: QueueState) -> QueueEntry {
    let mut e = sample_entry();
    e.entry_id = Uuid::new_v4();
    e.project_key = project_key.to_string();
    e.path = std::path::PathBuf::from(format!("{project_key}/session.jsonl"));
    e.state = state;
    e
}
```

If `sample_entry()` does not exist in this module, find the existing constructor the neighbouring tests use (search `mod tests` for `QueueEntry {`) and build on that instead — do not invent a second entry-construction pattern.

- [ ] **Step 2: Run the tests to verify they fail**

```
cd /path/to/repo
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::queue::tests::ignoring_a_project
```
Expected: FAIL to compile — `cannot find function refuse_pending_for_project` and `cannot find value REASON_PROJECT_IGNORED`.

- [ ] **Step 3: Add the constant**

Insert immediately after `REASON_TOO_LARGE` (`queue.rs:272`):

```rust
/// A project the contributor has chosen to ignore; its waiting sessions are
/// cleared when the mode is set.
///
/// Unlike `REASON_DISMISSED` this label suppresses nothing at the path
/// level. See `dismissed_at_path`: a dismissal is a permanent decision about
/// one conversation, while this is a verdict on whatever that project
/// happened to have queued at the moment its mode changed. Re-offering after
/// "Ask again" is the whole point, so borrowing the dismissal label would
/// make the recovery route a lie.
pub const REASON_PROJECT_IGNORED: &str = "project-ignored";
```

- [ ] **Step 4: Add the method**

Insert after `set_state` (`queue.rs:462-467`):

```rust
    /// Refuse every `Pending` entry belonging to `project_key`, returning how
    /// many moved.
    ///
    /// `Approved` and `Uploading` are deliberately left alone. An approval is
    /// a decision the contributor already made about a specific set of bytes
    /// under a specific set of consent scopes; a later project-level
    /// preference does not silently retract it. `Queue::cancel` draws the
    /// same line, and for the same reason.
    pub fn refuse_pending_for_project(&mut self, project_key: &str) -> usize {
        let mut purged = 0usize;
        for e in self.entries.iter_mut() {
            if e.project_key == project_key && e.state == QueueState::Pending {
                e.state = QueueState::Refused;
                e.reason_label = Some(REASON_PROJECT_IGNORED.to_string());
                purged += 1;
            }
        }
        purged
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

```
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::queue::tests
```
Expected: PASS, including the three new tests and every pre-existing queue test.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/queue.rs
git commit -m "Add a queue purge for a project the contributor ignores"
```

---

### Task 2: Daemon — wire the purge into `set_project_mode`

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (the `"set_project_mode"` arm begins at :748; the queue-lock block is at roughly :862-880 and the `Response::ok` at roughly :881)
- Modify: `docs/contributor-daemon-ipc-v1_1.md`
- Test: `crates/trace-commons-contributor/src/daemon/watcher.rs` `mod tests` (it owns the fixture that drives a real daemon queue; follow `a_dismissed_session_is_not_re_offered_after_it_grows` for shape)

**Interfaces:**
- Consumes: `Queue::refuse_pending_for_project` and `REASON_PROJECT_IGNORED` from Task 1
- Produces: `set_project_mode` responds `{"ok": true, "purged": <usize>}`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `watcher.rs`:

```rust
#[tokio::test]
async fn ignoring_a_project_clears_what_is_already_waiting() {
    let f = WatcherFixture::new();
    f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
    f.settle(at("2030-01-01T00:00:00Z")).await;
    assert_eq!(f.shared.queue.lock().unwrap().pending().len(), 1);

    f.set_mode("proj", ProjectMode::Ignore);

    let queue = f.shared.queue.lock().unwrap();
    assert!(queue.pending().is_empty(), "{:?}", queue.all());
    assert_eq!(
        queue.all()[0].reason_label.as_deref(),
        Some(crate::daemon::queue::REASON_PROJECT_IGNORED)
    );
}

#[tokio::test]
async fn un_ignoring_a_project_lets_its_sessions_be_offered_again() {
    // The confirmation copy promises this is undoable in Settings. Without
    // this test that promise is unverified.
    let f = WatcherFixture::new();
    let path = f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
    f.settle(at("2030-01-01T00:00:00Z")).await;
    f.set_mode("proj", ProjectMode::Ignore);
    assert!(f.shared.queue.lock().unwrap().pending().is_empty());

    f.set_mode("proj", ProjectMode::NotifyOnly);
    f.append_to_session(&path, "proj", "22222222-2222-2222-2222-222222222222");
    f.settle(at("2030-01-02T00:00:00Z")).await;

    assert!(
        !f.shared.queue.lock().unwrap().pending().is_empty(),
        "un-ignoring must let the watcher offer the project again"
    );
}
```

`f.set_mode(..)` already exists in this fixture (used by
`a_dismissed_session_is_not_re_offered_by_a_standing_opt_in`). If it sets the
policy directly rather than going through the IPC arm, add a sibling that
calls `handle_request` with `set_project_mode` and use that instead — the
purge lives in the IPC arm, so a test that bypasses it proves nothing.

- [ ] **Step 2: Run the tests to verify they fail**

```
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::watcher::tests::ignoring_a_project
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::watcher::tests::un_ignoring_a_project
```
Expected: FAIL — the first because the entry is still `Pending`, the second only if the first fix is wrong.

- [ ] **Step 3: Purge inside the existing queue-lock block**

In the `"set_project_mode"` arm, replace the `relabelled` block with one that
also purges. It already takes the queue lock and already saves, so the purge
adds no new lock acquisition and no second write:

```rust
            // Ignoring a project clears what it already has waiting. Doing
            // it here rather than in the UI means Settings, onboarding and
            // the CLI all get it: before this, ignoring from Settings left
            // the contributor staring at the cards they had just declined.
            //
            // Pending only. See `refuse_pending_for_project`.
            let (relabelled, purged) = {
                let mut queue = shared.queue.lock().expect("queue lock");
                let purged = if mode == ProjectMode::Ignore {
                    queue.refuse_pending_for_project(&key)
                } else {
                    0
                };
                let relabelled = relabel_queue_entries(&policy, &mut queue);
                if relabelled || purged > 0 {
                    if let Err(_e) = queue.save(&shared.store) {
                        return Response::err(req.id, ERR_UNAVAILABLE, "queue-write-failed");
                    }
                }
                (relabelled, purged)
            };
            drop(policy);
            if relabelled || purged > 0 {
                shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
            }
            Response::ok(req.id, serde_json::json!({ "ok": true, "purged": purged }))
```

- [ ] **Step 4: Run the tests to verify they pass**

```
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor
```
Expected: PASS, whole crate, zero failures.

- [ ] **Step 5: Document the contract**

In `docs/contributor-daemon-ipc-v1_1.md`, find the `set_project_mode` entry and add:

```markdown
Setting a project to `ignore` also clears whatever it has waiting: every
`pending` entry for that project moves to `refused` with
`reason_label = "project-ignored"`. The response carries `purged`, the
number of entries that moved.

`approved` and `uploading` entries are deliberately untouched. An approval
is a decision already made about specific bytes under specific consent
scopes, and a project-level preference set afterwards does not retract it —
so a project with three waiting and one approved loses three and still
uploads one. A client MUST say so rather than let a contributor discover it.

`"project-ignored"` is not `"dismissed-by-contributor"`. A dismissal is
permanent and suppresses that conversation at its path forever; this is a
verdict on whatever was queued at the moment the mode changed, so setting
the project back to `notify_only` or `auto_upload` lets those sessions be
offered again.
```

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/ipc.rs \
        crates/trace-commons-contributor/src/daemon/watcher.rs \
        docs/contributor-daemon-ipc-v1_1.md
git commit -m "Clear a project's waiting sessions when it is ignored"
```

---

### Task 3: macOS — copy unit, button, confirmation

**Files:**
- Create: `macos/Sources/TCShellCore/ProjectIgnoreCopy.swift`
- Create: `macos/Tests/TCShellCoreTests/ProjectIgnoreCopyTests.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/QueueView.swift` (`ProjectQueueGroup`, :157-196; call site :121-130)
- Modify: `macos/Sources/TraceCommonsApp/AppModel.swift` (`setProjectMode` at :406)

**Interfaces:**
- Consumes: Task 2's purge; `AppModel.setProjectMode(_ project: ProjectRow, mode: ProjectMode)` at :406
- Produces: `ProjectIgnoreCopy.confirmationTitle(project:) -> String`, `ProjectIgnoreCopy.confirmationBody(project:pendingCount:) -> String`, `AppModel.ignoreProject(id:label:)`

- [ ] **Step 1: Write the failing copy tests**

`macos/Tests/TCShellCoreTests/ProjectIgnoreCopyTests.swift`:

```swift
import XCTest
@testable import TCShellCore

final class ProjectIgnoreCopyTests: XCTestCase {
    func testSingularReadsAsOneTrace() {
        let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: 1)
        XCTAssertTrue(body.contains("1 waiting trace."), body)
        XCTAssertFalse(body.contains("traces"), body)
    }

    func testPluralReadsAsManyTraces() {
        let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: 12)
        XCTAssertTrue(body.contains("12 waiting traces"), body)
    }

    func testNothingWaitingDropsTheRemovalClause() {
        // A group can render with every card approved or uploading.
        // "removes 0 waiting traces" would be wrong and alarming.
        let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: 0)
        XCTAssertFalse(body.contains("0"), body)
        XCTAssertFalse(body.lowercased().contains("removes"), body)
        XCTAssertTrue(body.contains("Stops this project being offered."), body)
    }

    func testAlwaysNamesTheWayBack() {
        for n in [0, 1, 7] {
            let body = ProjectIgnoreCopy.confirmationBody(project: "api", pendingCount: n)
            XCTAssertTrue(body.contains("undo this in Settings"), "n=\(n): \(body)")
            XCTAssertTrue(body.contains("Nothing already submitted is affected."), "n=\(n)")
        }
    }

    func testTitleNamesTheProject() {
        XCTAssertEqual(ProjectIgnoreCopy.confirmationTitle(project: "api"), "Ignore api?")
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```
cd macos
cargo build -p trace-commons-contributor-ffi --manifest-path ../Cargo.toml
TC_FFI_LIB_DIR=$(pwd)/../target/debug swift test --filter ProjectIgnoreCopyTests
```
Expected: FAIL to build — `cannot find 'ProjectIgnoreCopy' in scope`.

- [ ] **Step 3: Write the copy unit**

`macos/Sources/TCShellCore/ProjectIgnoreCopy.swift`:

```swift
import Foundation

/// Copy for declining a whole project from the Waiting screen.
///
/// A tested unit rather than string interpolation at the call site: this
/// text is written three times across three shells, and plural agreement is
/// the first thing to drift.
public enum ProjectIgnoreCopy {
    public static let buttonLabel = "Ignore project"

    public static func confirmationTitle(project: String) -> String {
        "Ignore \(project)?"
    }

    /// The removal clause is dropped entirely when nothing is waiting: a
    /// group can render with every card approved or uploading, and
    /// "removes 0 waiting traces" would be both wrong and alarming.
    ///
    /// The last two sentences are load-bearing. One bounds the blast radius,
    /// the other names the way back — which is what lets the action itself
    /// be quiet.
    public static func confirmationBody(project: String, pendingCount: Int) -> String {
        let tail = "Nothing already submitted is affected. You can undo this in Settings."
        if pendingCount <= 0 {
            return "Stops this project being offered. \(tail)"
        }
        let noun = pendingCount == 1 ? "trace" : "traces"
        return "This removes \(pendingCount) waiting \(noun) and stops this project "
            + "being offered. \(tail)"
    }
}
```

- [ ] **Step 4: Run to verify it passes**

```
TC_FFI_LIB_DIR=$(pwd)/../target/debug swift test --filter ProjectIgnoreCopyTests
```
Expected: PASS, 5 tests.

- [ ] **Step 5: Add the model action**

In `AppModel.swift`, after `setProjectMode` (:406-416):

```swift
    /// Decline a whole project from the Waiting screen.
    ///
    /// The daemon clears what that project has waiting as part of setting the
    /// mode, so this refreshes the queue as well as the project list — the
    /// cards are expected to disappear in the same round trip.
    func ignoreProject(id projectID: String, label: String) {
        perform(
            "set_project_mode",
            work: { try $0.setProjectMode(projectID: projectID, mode: .ignore) }
        ) { _ in
            self.refreshQueue()
            self.refreshProjects()
            self.refreshAudit()
        }
    }
```

- [ ] **Step 6: Add the button and confirmation**

In `QueueView.swift`, add to `ProjectQueueGroup` (after `onSubmitAll` at :166):

```swift
    let onIgnoreProject: () -> Void
```

Add the state property inside `ProjectQueueGroup`:

```swift
    @State private var confirmingIgnore = false
```

In the header `HStack`, after the `Submit all` button:

```swift
                // Shown at every count, unlike `Submit all`, which hides at
                // one because the row's own Submit already does the same
                // thing. This has no row-level equivalent: it is a statement
                // about the project, not about a trace.
                //
                // Never `.tcPrimaryAction()`. It sits beside a control that
                // uploads the very traces this removes, and two adjacent
                // actions that do opposite things must not look alike.
                Button(ProjectIgnoreCopy.buttonLabel) { confirmingIgnore = true }
                    .help("Stops this project being offered and clears what it has waiting.")
```

Attach the dialog to the same `HStack`:

```swift
            .confirmationDialog(
                ProjectIgnoreCopy.confirmationTitle(project: group.label),
                isPresented: $confirmingIgnore,
                titleVisibility: .visible
            ) {
                Button(ProjectIgnoreCopy.buttonLabel, role: .destructive, action: onIgnoreProject)
                Button("Cancel", role: .cancel) {}
            } message: {
                Text(ProjectIgnoreCopy.confirmationBody(
                    project: group.label,
                    pendingCount: entries.filter { $0.state == .pending }.count
                ))
            }
```

If `QueueEntry` on the Swift side has no `state` property, count `entries` directly — the Waiting screen is built from `awaitingDecision`, which is already pending-only. Check `AppModel.awaitingDecision` before choosing, and use whichever is actually true rather than assuming.

At the call site (:121-130), pass the new closure:

```swift
                        onIgnoreProject: { model.ignoreProject(id: group.id, label: group.label) },
```

- [ ] **Step 7: Build and run the full macOS test suite**

```
cargo build -p trace-commons-contributor-ffi --manifest-path ../Cargo.toml
TC_FFI_LIB_DIR=$(pwd)/../target/debug swift build
TC_FFI_LIB_DIR=$(pwd)/../target/debug swift test
```
Expected: build succeeds, all tests pass.

- [ ] **Step 8: Commit**

```bash
git add macos/Sources/TCShellCore/ProjectIgnoreCopy.swift \
        macos/Tests/TCShellCoreTests/ProjectIgnoreCopyTests.swift \
        macos/Sources/TraceCommonsApp/Views/QueueView.swift \
        macos/Sources/TraceCommonsApp/AppModel.swift
git commit -m "Offer Ignore project on the macOS Waiting screen"
```

---

### Task 4: GTK — copy constants, button, confirmation

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs` (near `SUBMIT_ALL` at :139)
- Modify: `crates/trace-commons-contributor-gtk/src/ui/queue.rs` (the `Submit all` button is built at :806)
- Test: `copy.rs` `mod tests`

**Interfaces:**
- Consumes: Task 2's purge
- Produces: `copy::IGNORE_PROJECT`, `copy::IGNORE_PROJECT_TOOLTIP`, `copy::ignore_project_title(project: &str) -> String`, `copy::ignore_project_body(project: &str, pending: usize) -> String`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `copy.rs`:

```rust
#[test]
fn the_ignore_confirmation_counts_in_words_a_person_can_read() {
    assert!(ignore_project_body("api", 1).contains("1 waiting trace."));
    assert!(!ignore_project_body("api", 1).contains("traces"));
    assert!(ignore_project_body("api", 12).contains("12 waiting traces"));
}

#[test]
fn the_ignore_confirmation_says_nothing_about_zero() {
    let body = ignore_project_body("api", 0);
    assert!(!body.contains('0'), "{body}");
    assert!(!body.to_lowercase().contains("removes"), "{body}");
    assert!(body.contains("Stops this project being offered."), "{body}");
}

#[test]
fn the_ignore_confirmation_always_names_the_way_back() {
    for n in [0usize, 1, 7] {
        let body = ignore_project_body("api", n);
        assert!(body.contains("undo this in Settings"), "n={n}: {body}");
        assert!(body.contains("Nothing already submitted is affected."), "n={n}");
    }
}

#[test]
fn the_ignore_title_names_the_project() {
    assert_eq!(ignore_project_title("api"), "Ignore api?");
}
```

- [ ] **Step 2: Run to verify it fails**

```
cd crates/trace-commons-contributor-gtk
RUSTFLAGS='-D warnings' cargo test copy::tests::the_ignore
```
Expected: FAIL to compile — `cannot find function ignore_project_body`.

- [ ] **Step 3: Add the copy**

In `copy.rs`, after `SUBMIT_ALL_TOOLTIP` (:140):

```rust
pub const IGNORE_PROJECT: &str = "Ignore project";
pub const IGNORE_PROJECT_TOOLTIP: &str =
    "Stops this project being offered and clears what it has waiting. \
     Anything already submitted is unaffected, and you can undo this in Settings.";

pub fn ignore_project_title(project: &str) -> String {
    format!("Ignore {project}?")
}

/// The removal clause is dropped when nothing is waiting: a group can render
/// with every card approved or uploading, and "removes 0 waiting traces"
/// would be both wrong and alarming.
pub fn ignore_project_body(project: &str, pending: usize) -> String {
    let _ = project;
    let tail = "Nothing already submitted is affected. You can undo this in Settings.";
    if pending == 0 {
        return format!("Stops this project being offered. {tail}");
    }
    let noun = if pending == 1 { "trace" } else { "traces" };
    format!("This removes {pending} waiting {noun} and stops this project being offered. {tail}")
}
```

- [ ] **Step 4: Run to verify it passes**

```
RUSTFLAGS='-D warnings' cargo test copy::tests
```
Expected: PASS.

- [ ] **Step 5: Add the button**

In `ui/queue.rs`, beside the `submit_all` button (:806-809). Read the
surrounding function first to learn how it obtains `project_id`,
`project_label` and the group's entries, and how it calls the daemon — the
`approve` call with `project_id` is the pattern to mirror. Then:

```rust
    let ignore = gtk::Button::with_label(copy::IGNORE_PROJECT);
    ignore.add_css_class("tc-chip");
    ignore.set_tooltip_text(Some(copy::IGNORE_PROJECT_TOOLTIP));
```

Do **not** give it the primary-action class `submit_all` carries.

The confirmation follows the withdrawal dialog in `ui/history.rs:606-655`,
which is this crate's established `adw::MessageDialog` pattern:

```rust
    let app_for_ignore = Rc::clone(app);
    let project_id_for_ignore = project_id.to_string();
    let project_label_for_ignore = project_label.to_string();
    let pending_count = /* number of Pending entries in this group */;
    ignore.connect_clicked(move |_| {
        let dialog = adw::MessageDialog::new(
            Some(&app_for_ignore.window),
            Some(&copy::ignore_project_title(&project_label_for_ignore)),
            Some(&copy::ignore_project_body(&project_label_for_ignore, pending_count)),
        );
        dialog.add_responses(&[("cancel", "Cancel"), ("ignore", copy::IGNORE_PROJECT)]);
        dialog.set_close_response("cancel");
        // It sits beside a control that uploads these same traces. The
        // destructive appearance is what stops the two reading alike.
        dialog.set_response_appearance("ignore", adw::ResponseAppearance::Destructive);

        let app = Rc::clone(&app_for_ignore);
        let project_id = project_id_for_ignore.clone();
        dialog.connect_response(None, move |dialog, response| {
            dialog.close();
            if response != "ignore" {
                return;
            }
            // Mirrors ui/settings.rs:901.
            app.call_daemon(
                "set_project_mode",
                serde_json::json!({ "project_id": project_id, "mode": "ignore" }),
            );
            app.refresh();
        });
        dialog.present();
    });
```

`app.call_daemon` and `app.refresh` are placeholders for whatever
`ui/settings.rs:901` and the `approve` path in this file actually call —
read both and use the real names. Do not introduce a second way to reach the
daemon.

Unlike `submit_all`, add the button regardless of how many entries the group
has.

- [ ] **Step 6: Verify the GTK crate separately**

```
cd crates/trace-commons-contributor-gtk
cargo fmt -- --check
RUSTFLAGS='-D warnings' cargo check --all-targets
RUSTFLAGS='-D warnings' cargo test
```
Expected: all clean. Then confirm the lockfile is untouched:

```bash
git status --porcelain crates/trace-commons-contributor-gtk/Cargo.lock
```
Expected: empty. If it changed, `git checkout` it — a macOS build rewrites it and breaks the flatpak `cargo-sources.json` drift check.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor-gtk/src/copy.rs \
        crates/trace-commons-contributor-gtk/src/ui/queue.rs
git commit -m "Offer Ignore project on the GTK Waiting screen"
```

---

### Task 5: Windows — dialog guard (closes #316), copy, button

**Files:**
- Create: `windows/src/TraceCommons.App/Controls/DialogGuard.cs`
- Create: `windows/src/TraceCommons.Interop/ProjectIgnoreCopy.cs`
- Create: `windows/tests/TraceCommons.Interop.Tests/ProjectIgnoreCopyTests.cs`
- Modify: `windows/src/TraceCommons.App/Controls/WithdrawDialog.cs:85`
- Modify: `windows/src/TraceCommons.App/Controls/GoPublicDialog.cs:159`
- Modify: `windows/src/TraceCommons.App/MainWindow.xaml` (project header, near the `Submit all` binding at :606)
- Modify: `windows/src/TraceCommons.App/ViewModels/MainViewModel.cs`

**Interfaces:**
- Consumes: Task 2's purge; `DaemonProtocol.Methods.SetProjectMode` (already used at `ContributorSettingsViewModel.cs:296`)
- Produces: `DialogGuard.ShowOnceAsync(ContentDialog) -> Task<ContentDialogResult>`, `ProjectIgnoreCopy.ConfirmationTitle(string)`, `ProjectIgnoreCopy.ConfirmationBody(string, int)`, `QueueGroupViewModel.IgnoreProjectCommand`

- [ ] **Step 1: Write the failing copy tests**

`windows/tests/TraceCommons.Interop.Tests/ProjectIgnoreCopyTests.cs`:

```csharp
using TraceCommons.Interop;
using Xunit;

public class ProjectIgnoreCopyTests
{
    [Theory]
    [InlineData(1, "1 waiting trace.")]
    [InlineData(12, "12 waiting traces")]
    public void CountsInWordsAPersonCanRead(int n, string expected)
    {
        Assert.Contains(expected, ProjectIgnoreCopy.ConfirmationBody("api", n));
    }

    [Fact]
    public void SingularIsNotPluralised()
    {
        Assert.DoesNotContain("traces", ProjectIgnoreCopy.ConfirmationBody("api", 1));
    }

    [Fact]
    public void NothingWaitingDropsTheRemovalClause()
    {
        var body = ProjectIgnoreCopy.ConfirmationBody("api", 0);
        Assert.DoesNotContain("0", body);
        Assert.DoesNotContain("removes", body.ToLowerInvariant());
        Assert.Contains("Stops this project being offered.", body);
    }

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    [InlineData(7)]
    public void AlwaysNamesTheWayBack(int n)
    {
        var body = ProjectIgnoreCopy.ConfirmationBody("api", n);
        Assert.Contains("undo this in Settings", body);
        Assert.Contains("Nothing already submitted is affected.", body);
    }

    [Fact]
    public void TitleNamesTheProject()
    {
        Assert.Equal("Ignore api?", ProjectIgnoreCopy.ConfirmationTitle("api"));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```
cd windows
dotnet test tests/TraceCommons.Interop.Tests
```
Expected: FAIL to compile — `ProjectIgnoreCopy` does not exist.

**If `dotnet test` cannot run** because `windows/global.json` pins an SDK you
do not have: do not edit `global.json`. Compile the changed files plus their
dependencies into a scratch xunit project against the SDK you do have, run
the tests there, and say exactly that in your report.

- [ ] **Step 3: Write the copy unit**

`windows/src/TraceCommons.Interop/ProjectIgnoreCopy.cs`:

```csharp
namespace TraceCommons.Interop;

/// <summary>
/// Copy for declining a whole project from the Waiting screen. A tested unit
/// rather than inline interpolation: this text exists in three shells and
/// plural agreement is the first thing to drift between them.
/// </summary>
public static class ProjectIgnoreCopy
{
    public const string ButtonLabel = "Ignore project";

    public static string ConfirmationTitle(string project) => $"Ignore {project}?";

    /// <summary>
    /// The removal clause is dropped when nothing is waiting: a group can
    /// render with every card approved or uploading, and "removes 0 waiting
    /// traces" would be both wrong and alarming.
    /// </summary>
    public static string ConfirmationBody(string project, int pendingCount)
    {
        const string tail =
            "Nothing already submitted is affected. You can undo this in Settings.";
        if (pendingCount <= 0)
        {
            return $"Stops this project being offered. {tail}";
        }
        var noun = pendingCount == 1 ? "trace" : "traces";
        return $"This removes {pendingCount} waiting {noun} and stops this project "
             + $"being offered. {tail}";
    }
}
```

- [ ] **Step 4: Run to verify it passes**

```
dotnet test tests/TraceCommons.Interop.Tests --filter ProjectIgnoreCopyTests
```
Expected: PASS.

- [ ] **Step 5: Write the dialog guard**

WinUI allows exactly one `ContentDialog` open per `XamlRoot`; a second
`ShowAsync()` throws and kills the process. #315 added a guard, but it lives
in `MainWindow` and cannot be reached from the static helpers in
`Controls/`. This is that guard, somewhere they can reach.

`windows/src/TraceCommons.App/Controls/DialogGuard.cs`:

```csharp
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Xaml.Controls;

namespace TraceCommons.App.Controls;

/// <summary>
/// Serializes ContentDialog display. WinUI permits one dialog per XamlRoot
/// and throws on a second ShowAsync, which takes the process with it. Every
/// dialog in this app goes through here.
/// </summary>
internal static class DialogGuard
{
    private static readonly SemaphoreSlim Gate = new(1, 1);

    /// <summary>
    /// Shows the dialog, waiting if another is already open. Returns
    /// <see cref="ContentDialogResult.None"/> if the dialog could not be
    /// shown at all — a caller must treat that as "the person did not
    /// confirm", never as consent.
    /// </summary>
    public static async Task<ContentDialogResult> ShowOnceAsync(ContentDialog dialog)
    {
        await Gate.WaitAsync().ConfigureAwait(true);
        try
        {
            return await dialog.ShowAsync();
        }
        catch (System.Exception)
        {
            // A dialog that cannot be shown must not be read as a yes.
            return ContentDialogResult.None;
        }
        finally
        {
            Gate.Release();
        }
    }
}
```

- [ ] **Step 6: Route the two unguarded sites through it**

`Controls/WithdrawDialog.cs:85` — replace `await dialog.ShowAsync() == ContentDialogResult.Primary` with:

```csharp
            return await DialogGuard.ShowOnceAsync(dialog) == ContentDialogResult.Primary;
```

`Controls/GoPublicDialog.cs:159` — replace `await dialog.ShowAsync();` with:

```csharp
            await DialogGuard.ShowOnceAsync(dialog);
```

Read both call sites first: if either already inspects the result, preserve
that logic and change only the call.

- [ ] **Step 7: Add the button and command**

In `MainViewModel.cs`, on `QueueGroupViewModel` (the type that already owns
`ShowSubmitAll`, referenced at `MainWindow.xaml:606`), add an
`IgnoreProjectCommand` that:

1. builds a `ContentDialog` with `Title = ProjectIgnoreCopy.ConfirmationTitle(ProjectLabel)`, `Content = ProjectIgnoreCopy.ConfirmationBody(ProjectLabel, PendingCount)`, `PrimaryButtonText = ProjectIgnoreCopy.ButtonLabel`, `CloseButtonText = "Cancel"`, `DefaultButton = ContentDialogButton.Close`, and the group's `XamlRoot`;
2. shows it with `DialogGuard.ShowOnceAsync`;
3. on `ContentDialogResult.Primary`, calls `DaemonProtocol.Methods.SetProjectMode` with the project id and `"ignore"`, following `ContributorSettingsViewModel.cs:296`;
4. refreshes the queue.

Add a `ShowIgnoreProject` property that is **always true**, unlike
`ShowSubmitAll` — the button renders for single-entry groups too.

If `QueueGroupViewModel` is reachable from a test project, add a test
pinning that asymmetry, since it is the one piece of the visibility rule
that can be checked without rendering:

```csharp
    [Fact]
    public void IgnoreShowsForASingleEntryGroupWhereSubmitAllDoesNot()
    {
        var group = /* a QueueGroupViewModel with exactly one entry */;
        Assert.False(group.ShowSubmitAll);
        Assert.True(group.ShowIgnoreProject);
    }
```

If it is not reachable — it lives in `TraceCommons.App`, which the Interop
test project may not reference — say so in your report rather than moving
the property somewhere artificial just to test it.

In `MainWindow.xaml`, beside the `Submit all` button (:606), add a button
bound to `IgnoreProjectCommand` with `Visibility` bound to
`ShowIgnoreProject`, matching the sibling caption-row binding style. Do not
give it the accent/primary style the submit button uses.

**XAML comments must not contain `--`** — the file's own header documents
this, and `xmllint` will reject it.

- [ ] **Step 8: Verify what can be verified**

```
cd windows
dotnet test tests/TraceCommons.Interop.Tests
xmllint --noout src/TraceCommons.App/MainWindow.xaml && echo "XAML well-formed"
```
Expected: tests pass, XAML well-formed. The WinUI XAML compile and the
rendered button cannot be checked outside Windows — CI's `windows
contributor app` job is the first real check. State that plainly.

- [ ] **Step 9: Commit**

```bash
git add windows/src/TraceCommons.App/Controls/DialogGuard.cs \
        windows/src/TraceCommons.App/Controls/WithdrawDialog.cs \
        windows/src/TraceCommons.App/Controls/GoPublicDialog.cs \
        windows/src/TraceCommons.Interop/ProjectIgnoreCopy.cs \
        windows/tests/TraceCommons.Interop.Tests/ProjectIgnoreCopyTests.cs \
        windows/src/TraceCommons.App/MainWindow.xaml \
        windows/src/TraceCommons.App/ViewModels/MainViewModel.cs
git commit -m "Offer Ignore project on the Windows Waiting screen"
```

---

### Task 6: Whole-repo verification

**Files:** none modified unless a check fails.

- [ ] **Step 1: Run every gate CI runs**

```
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server
cargo clippy --workspace --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```
Expected: all clean.

- [ ] **Step 2: Confirm no lockfile drift**

```bash
git status --porcelain | grep -E "Cargo.lock" || echo "no lockfile drift"
```
Expected: `no lockfile drift`.

- [ ] **Step 3: Confirm the whole diff**

```bash
git show --stat HEAD~4..HEAD
```
Expected: only the files named in Tasks 1-5.

---

## Notes for the PR

- Lead with the defect, not the button: ignoring a project has never cleared what it already had waiting, so Settings and onboarding have been quietly failing at this the whole time. The button is what made it visible.
- State the Pending-only rule and its visible consequence: three waiting and one approved means three disappear and one still uploads.
- Say that `"project-ignored"` is deliberately not `"dismissed-by-contributor"`, and that a test proves un-ignoring re-offers — otherwise the confirmation's promise of recovery is unverified.
- Note that this closes **#316** as a side effect, and why a new dialog forced the issue.
- State plainly that Windows XAML compiled only in CI.
