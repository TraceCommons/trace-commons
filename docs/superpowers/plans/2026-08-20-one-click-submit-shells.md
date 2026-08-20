# One-click submit: the three shells Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A Submit action on each queue row and each project group, in all three shells, rendering the daemon's counts as a toast with Undo.

**Architecture:** The daemon already does the work. `approve` takes an
`entry_id` or a `project_id`, builds and pins an envelope for anything
unpreviewed, and returns `approved`, `flagged`, `redactions` and `skipped`.
Each shell adds two controls and one renderer. No shell gains a concept, and
no shell decides anything the daemon has already decided.

**Tech Stack:** Rust/GTK4, Swift/SwiftUI, C#/WinUI. No new dependencies.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-20-one-click-submit-design.md`. Its
  "The toast: normative copy" section is the contract. TRANSCRIBE the strings;
  do not reword them in one shell. Every divergence in this product began with
  a screen whose words were specified nowhere.
- The four toast clauses, in order: what was sent; what scrubbing did (always,
  including zero); what was flagged (only when > 0); what was not sent (only
  when non-empty). Undo only when `approved > 0`.
- Skip reasons render as the spec's human labels, never the wire label and
  never an entry id.
- Hash-only, label-only: counts, fixed labels, opaque ids only. No path, no
  trace content.
- Per-shell tooling: GTK `RUSTFLAGS="-D warnings"` + repo clippy allow-list
  unwidened + `cargo fmt` for that crate; macOS `swift test` with
  `TC_FFI_LIB_DIR` set; Windows `dotnet test` on the Interop project with
  .NET SDK 8 on PATH.
- No emojis. Repo-relative paths only. `git show --stat` after each commit.
- Baselines: GTK 136 tests, Swift 68, .NET Interop 322, workspace 73 targets.

## Existing interfaces

- `approve` params: `{"entry_id": <uuid>}` or `{"project_id": "proj_..."}`.
  Mutually exclusive; an unrecognised id of either kind is refused.
- Response: `{"approved": u64, "flagged": u64, "redactions": {String: u32},
  "skipped": [{"entry_id": String, "reason_label": String}], "hold_secs": u64,
  "hold_until": String|null}`.
- Undo is the existing `cancel` method, which now clears the pin.
- All three shells already call `approve` with an `entry_id` after a preview:
  GTK `ui/preview.rs:986`, macOS `DaemonClient.swift:159`, Windows
  `DaemonProtocol.Approve`.

---

### Task 1: the shared toast renderer, once per shell

**Files:**
- Create: `crates/trace-commons-contributor-gtk/src/toast.rs`
- Create: `macos/Sources/TCShellCore/SubmitToast.swift`
- Create: `windows/src/TraceCommons.Interop/SubmitToast.cs`
- Test: the unit test file beside each

**Interfaces:**
- Produces, in each language, one pure function from the approve response to a
  toast string plus a bool for whether Undo is offered. No I/O, no UI types --
  this is the piece that must be identical across three languages, so it must
  be testable without a display.

- [ ] **Step 1: Write the failing tests from the spec's worked examples**

All four spec examples, in each language. GTK first:

```rust
#[test]
fn the_spec_worked_examples_render_exactly() {
    assert_eq!(
        toast(1, 4, 1, &[]).line,
        "Sent. Scrubbing removed 4 things. 1 flagged."
    );
    assert_eq!(
        toast(47, 213, 3, &[]).line,
        "Sent 47 sessions. Scrubbing removed 213 things. 3 flagged."
    );
    assert_eq!(
        toast(44, 213, 0, &["envelope-too-large", "envelope-too-large", "envelope-too-large"]).line,
        "Sent 44 sessions. Scrubbing removed 213 things. 3 not sent: too large to send."
    );
    assert_eq!(
        toast(0, 0, 0, &["not-pending", "not-pending"]).line,
        "Nothing sent. Scrubbing matched nothing. 2 not sent: already decided."
    );
}

#[test]
fn undo_is_offered_only_when_something_was_sent() {
    assert!(toast(1, 0, 0, &[]).offer_undo);
    assert!(!toast(0, 0, 0, &["not-pending"]).offer_undo);
}

#[test]
fn a_wire_label_never_reaches_the_contributor() {
    let line = toast(0, 0, 0, &["envelope-too-large"]).line;
    assert!(!line.contains("envelope-too-large"), "wire label leaked: {line}");
    assert!(line.contains("too large to send"));
}
```

- [ ] **Step 2: Run them and watch them fail**

Expected: FAIL, no such function.

- [ ] **Step 3: Implement the four clauses**

Follow the spec's table exactly. Distinct reasons, comma-separated, in the
spec's listed order. Singular and plural forms as written.

- [ ] **Step 4: Run them and watch them pass**

- [ ] **Step 5: Prove the tests have teeth**

Apply a mutation that COMPILES CLEANLY and leaves the behaviour wrong -- for
example, drop the `flagged` clause, or emit the wire label instead of the human
one. Confirm failure, restore, confirm green. Report the output. A mutation the
compiler rejects proves nothing.

- [ ] **Step 6: Commit**

```bash
git add <the renderer and its test>
git commit -m "Render the submit toast from the daemon's counts"
git show --stat HEAD
```

---

### Task 2: GTK -- the two controls, and the undo defect

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/queue.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/ui/preview.rs:986-1000`
- Test: the crate's existing test module, plus a container screenshot

**Interfaces:**
- Consumes: Task 1's `toast()`.

- [ ] **Step 1: Fix the undo defect first, with a test**

`preview.rs` calls `offer_undo` on any `Ok`, ignoring `approved`. Write a test
that a response with `approved: 0` does not offer undo. Watch it fail. Fix by
routing through Task 1's `offer_undo`. Watch it pass.

- [ ] **Step 2: Add Submit to the queue row**

Beside `Look inside` and `Not this one`. One click: call `approve` with the
entry id, render the toast, offer Undo for `hold_secs`.

- [ ] **Step 3: Add Submit to the project group**

Calls `approve` with `project_id`. Same renderer, same Undo.

- [ ] **Step 4: Photograph it**

`scripts/linux-build.sh` in the container against Xvfb, as the existing
harnesses do. Look at the image: the row's three controls should not crowd, and
the toast must be legible at its longest form (the 47-session example). No host
`screencapture` -- everything inside the container.

- [ ] **Step 5: Verify and commit**

GTK baseline 136; hold or beat it. `RUSTFLAGS="-D warnings"`, clippy
unwidened, `cargo fmt` for that crate, `css_contract` green.

---

### Task 3: macOS -- the two controls

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/QueueView.swift`
- Modify: `macos/Sources/TraceCommonsApp/AppModel.swift:467`
- Modify: `macos/Sources/TraceCommonsApp/DaemonClient.swift:159`
- Test: `macos/Tests/TCShellCoreTests/`

**Interfaces:**
- Consumes: Task 1's `SubmitToast`.

- [ ] **Step 1: Widen the client**

`DaemonClient.approve` currently sends only `entry_id` and decodes only
`approved`. Add the `project_id` form and decode the full response. Test the
decode against a literal JSON fixture matching the daemon's shape.

- [ ] **Step 2: Submit on the row and on the project group**

- [ ] **Step 3: Photograph both**

Build with `make-app-bundle.sh`, launch BY EXPLICIT PATH, throwaway state dir
under /tmp with a SHORT path (104-byte socket limit). Locate the window through
the accessibility API AT THE MOMENT OF CAPTURE and assert which process owns it.
Never full-screen, never remembered coordinates.

- [ ] **Step 4: Verify and commit**

Swift baseline 68; workspace 73.

---

### Task 4: Windows -- the two controls

**Files:**
- Modify: `windows/src/TraceCommons.App/` (queue view and its view model)
- Modify: `windows/src/TraceCommons.Interop/DaemonProtocol.cs`
- Test: `windows/tests/TraceCommons.Interop.Tests/`

**Interfaces:**
- Consumes: Task 1's `SubmitToast`.

- [ ] **Step 1: Widen the interop layer and test it**

The Interop project compiles and its tests run locally with .NET SDK 8 on
PATH. Put every decision there, not in XAML: the XAML layer compiles only on
Windows and executes nowhere before a contributor runs it.

- [ ] **Step 2: Submit on the row and on the project group**

- [ ] **Step 3: Verify what can be verified, and say what cannot**

`dotnet test` on Interop; baseline 322. The WinUI half cannot be compiled off
Windows -- `XamlCompiler.exe` is a Windows-only binary that runs before the C#
compile. State plainly which lines are uncompiled.

- [ ] **Step 4: Commit**

Write it to compile clean under `TreatWarningsAsErrors`: an unused import is a
build failure on the Windows box, not a lint.

---

### Task 5: one string, three shells

**Files:**
- Create: `crates/trace-commons-contributor/tests/toast_parity.rs`

- [ ] **Step 1: Write a test that fails if the three renderers disagree**

The three implementations are in three languages and cannot share code. Pin
them to the spec instead: a table of the spec's worked examples, asserted
against the GTK renderer, plus a check that the macOS and Windows test suites
contain the same expected strings verbatim. A grep-based assertion is
legitimate here -- the failure it catches is a reworded string, and that is
exactly what a grep sees.

- [ ] **Step 2: Prove it fails**

Reword one string in one shell's test. Confirm the parity test fails naming
that shell. Restore.

- [ ] **Step 3: Commit**

---

## What this plan does not cover

The verification campaign. These are three UIs that no person has driven; the
runbook at `docs/operator/client-end-to-end-verification.md` exists for exactly
that and has never been run.
