# GUI Contributor Verdict Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a contributor answer "did this session do what you asked?" on the
approval control in the desktop shells, and carry that answer to
`OutcomeMetadata.task_success`.

**Architecture:** The verdict is collected on the approval control, recorded on
the queue entry as a wire-name string, and applied to the stored redacted
envelope by the uploader after its digest check passes. It is an
approval-derived output, not a drift guard.

**Tech Stack:** Rust (contributor crate, GTK shell), Swift (macOS shell), C#
(Windows shell). JSON-RPC over a unix socket / named pipe.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-26-gui-contributor-verdict-design.md`.
- **Branch base: `claude/contributor-verdict` (PR #421).** This work depends on
  `ContributorVerdict` and its `Partly` variant. Do not branch from `main`.
- PostgreSQL-only repo; no libsql feature flags. Not relevant here but do not
  add any.
- No emojis in commits, PRs, code, or docs.
- Hash-only / label-only logging. A verdict is a fixed label and may be logged
  as such; never log envelope content alongside it.
- Verify with `RUSTFLAGS="-D warnings"`. Plain `cargo check` does not apply
  `-D warnings`; CI does.
- Clippy allow-list, used verbatim:
  `-A clippy::type_complexity -A clippy::collapsible_if
  -A clippy::manual_option_as_slice -A clippy::useless_vec
  -A clippy::redundant_pattern_matching`
- Run `cargo fmt --all` before committing. A post-edit formatter hook runs in
  this repo; check `git show --stat` after every commit to confirm it did not
  rewrite unrelated files.
- Wire names are exactly `worked`, `partly`, `failed`. An unrecognised value is
  refused, never coerced to `Unknown`.

## Prerequisite: already done, do not redo

The three protocol-side changes the spec calls for are **already pushed to PR
#421** (commit `fe831242`):

1. `ContributorVerdict` has a `Partly` variant and `parse` accepts `"partly"`.
2. `outcome()` writes `task_success` only; `user_feedback` stays
   `UserFeedback::None`.
3. `a_verdict_reaches_the_outcome` covers all three states and asserts the
   absence of a feedback signal.

Confirm before starting, and do not re-implement:

```bash
git log --oneline -1 claude/contributor-verdict
grep -n 'Partly' crates/trace-commons-contributor/src/envelope.rs
```

---

### Task 1: Record the verdict on the queue entry

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/queue.rs`
- Test: `crates/trace-commons-contributor/src/daemon/queue.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `QueueEntry.approved_verdict: Option<String>`, and
  `Queue::approve(&mut self, entry_id: Uuid, scopes: &[String], inputs:
  Option<&str>, verdict: Option<&str>, approved_at: Option<DateTime<Utc>>) ->
  bool`. Note `verdict` is inserted BEFORE `approved_at`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module in `queue.rs`:

```rust
#[test]
fn an_approval_records_the_verdict_it_was_given() {
    let mut q = Queue::default();
    let id = seed_pending(&mut q);

    assert!(q.approve(id, &["debugging_evaluation".to_string()], None, Some("failed"), None));

    let e = q.get(id).expect("entry");
    assert_eq!(e.approved_verdict.as_deref(), Some("failed"));
}

/// Absence is not failure. An approval with no verdict leaves the field
/// `None`, which the uploader reads as `TaskSuccess::Unknown` and submits
/// normally.
///
/// This is deliberately NOT the fail-closed reading its neighbours get.
/// `approved_scopes` and `approved_inputs` are drift guards, and `None` on
/// either means "unknown, so re-ask". `approved_verdict` cannot drift,
/// because approving is what produces it.
#[test]
fn an_approval_without_a_verdict_records_none_and_still_approves() {
    let mut q = Queue::default();
    let id = seed_pending(&mut q);

    assert!(q.approve(id, &["debugging_evaluation".to_string()], None, None, None));

    let e = q.get(id).expect("entry");
    assert_eq!(e.approved_verdict, None);
    assert_eq!(e.state, QueueState::Approved);
}
```

If `seed_pending` does not already exist in this test module, use whatever
helper the neighbouring tests use to create a `Pending` entry and return its
`entry_id`; do not invent a new one.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-contributor --lib daemon::queue::tests::an_approval_records_the_verdict_it_was_given`
Expected: FAIL to compile — `approve` takes 4 arguments, and `approved_verdict`
does not exist on `QueueEntry`.

- [ ] **Step 3: Add the field**

In `QueueEntry`, immediately after the `approved_scopes` field (around
`queue.rs:87`):

```rust
    /// The verdict the contributor gave when they approved this entry:
    /// `worked`, `partly`, or `failed`. `None` means they did not answer.
    ///
    /// Read the neighbours carefully before changing this. `approved_scopes`
    /// and `approved_inputs` are DRIFT GUARDS: they record ambient inputs as
    /// of approval so the uploader can refuse if either moved before it
    /// sent, and `None` on an approved entry means "unknown, so re-ask" and
    /// fails closed.
    ///
    /// This field is the opposite kind of thing. It is an OUTPUT of the
    /// approval act, not configuration that could change underneath it, so
    /// it cannot drift between approval and send. `None` means the
    /// contributor did not answer, which is `TaskSuccess::Unknown`, and the
    /// entry submits normally.
    ///
    /// It must NOT be folded into `preview::input_fingerprint`. Doing so
    /// would fail-close every approval made before this field existed.
    ///
    /// Stored as the wire name rather than an enum, matching
    /// `approved_scopes`, so the on-disk queue does not depend on a Rust
    /// type's serialisation.
    #[serde(default)]
    pub approved_verdict: Option<String>,
```

- [ ] **Step 4: Thread it through `approve`**

Change the signature and body at `queue.rs:565`:

```rust
    pub fn approve(
        &mut self,
        entry_id: Uuid,
        scopes: &[String],
        inputs: Option<&str>,
        verdict: Option<&str>,
        approved_at: Option<DateTime<Utc>>,
    ) -> bool {
        let Some(e) = self.entries.iter_mut().find(|e| e.entry_id == entry_id) else {
            return false;
        };
        if e.state != QueueState::Pending {
            return false;
        }
        e.state = QueueState::Approved;
        e.reason_label = None;
        e.approved_scopes = Some(scopes.to_vec());
        e.approved_inputs = inputs.map(str::to_string);
        e.approved_verdict = verdict.map(str::to_string);
        e.approved_at = approved_at;
        true
    }
```

- [ ] **Step 5: Clear it wherever approval is cleared**

`approved_scopes` is set back to `None` at `queue.rs:682` and `queue.rs:943`
(revoked and undone approvals). Add `e.approved_verdict = None;` immediately
after each of those two lines. A revoked approval must not leave a verdict
behind for a later, different approval to inherit.

- [ ] **Step 6: Fix every struct literal and call site**

`QueueEntry` literals needing `approved_verdict: None` added: `queue.rs:302`,
`queue.rs:1056`, `ipc.rs:2822`, `ipc.rs:2923`, `ipc.rs:3013`, `ipc.rs:3479`.
Search for others rather than trusting this list:

```bash
grep -rn 'approved_scopes: None' crates/trace-commons-contributor/src/
```

The one non-test caller of `approve` is `ipc.rs:1679`; pass `None` for now —
Task 2 replaces it.

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p trace-commons-contributor --lib daemon::queue::`
Expected: PASS, including both new tests.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/queue.rs \
        crates/trace-commons-contributor/src/daemon/ipc.rs
git commit -m "Record the contributor verdict on the queue entry"
git show --stat HEAD
```

---

### Task 2: Accept the verdict over IPC

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (`handle_approve`, from `ipc.rs:1351`)
- Test: `crates/trace-commons-contributor/src/daemon/ipc.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `Queue::approve(.., verdict: Option<&str>, ..)` from Task 1.
- Produces: the `approve` method accepts an optional `outcome` string
  parameter, alongside any of `all`, `project_id`, `entry_id`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `ipc.rs`, following the shape of the existing
`approve` tests (they build a request with `req("approve", json!({...}))` and
call `handle_request_async`):

```rust
#[tokio::test]
async fn an_approval_carries_its_verdict_to_the_entry() {
    let s = seeded_shared_with_one_pending();
    let entry_id = first_pending_id(&s);

    let r = handle_request_async(
        &s,
        &req(
            "approve",
            serde_json::json!({"entry_id": entry_id.to_string(), "outcome": "partly"}),
        ),
    )
    .await;
    assert!(r.error.is_none(), "approve should succeed: {:?}", r.error);

    let q = s.queue.lock().expect("queue lock");
    assert_eq!(q.get(entry_id).expect("entry").approved_verdict.as_deref(), Some("partly"));
}

/// A bulk approval applies one verdict to every entry it covers. This is a
/// coverage-over-precision tradeoff taken deliberately; see the spec.
#[tokio::test]
async fn a_bulk_approval_applies_its_verdict_to_every_entry() {
    let s = seeded_shared_with_two_pending();

    let r = handle_request_async(
        &s,
        &req("approve", serde_json::json!({"all": true, "outcome": "worked"})),
    )
    .await;
    assert!(r.error.is_none());

    let q = s.queue.lock().expect("queue lock");
    for e in q.all() {
        assert_eq!(e.approved_verdict.as_deref(), Some("worked"));
    }
}

/// A typo must not silently submit the run as `Unknown`. Same rule the
/// `--outcome` flag applies, at the IPC boundary.
#[tokio::test]
async fn an_unrecognised_verdict_is_refused_and_approves_nothing() {
    let s = seeded_shared_with_one_pending();
    let entry_id = first_pending_id(&s);

    let r = handle_request_async(
        &s,
        &req(
            "approve",
            serde_json::json!({"entry_id": entry_id.to_string(), "outcome": "sucess"}),
        ),
    )
    .await;

    assert!(r.error.is_some(), "an unknown verdict must be refused");
    let q = s.queue.lock().expect("queue lock");
    assert_eq!(
        q.get(entry_id).expect("entry").state,
        QueueState::Pending,
        "a refused call must approve nothing"
    );
}
```

Reuse whatever seeding helpers the surrounding `approve` tests already use
(`ipc.rs:2890` and `ipc.rs:3493` call `approve` with `{"all": true}` and will
show you the local idiom). Do not add new helpers if equivalents exist.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::tests::an_unrecognised_verdict_is_refused_and_approves_nothing`
Expected: FAIL — the `outcome` parameter is ignored, so the call succeeds and
the entry becomes `Approved`.

- [ ] **Step 3: Parse and validate the parameter**

In `handle_approve`, immediately after `all` is read (`ipc.rs:1352-1356`) and
BEFORE any queue mutation or the bulk-approval audit write:

```rust
    // Validated up front, before anything is approved and before the
    // bulk-approval audit row is written. A refused call must approve
    // nothing and leave no record of a batch that did not happen.
    //
    // Refused rather than ignored: a contributor who answered meant to say
    // something, and coercing a typo to `Unknown` would silently discard
    // the answer. Same rule the `--outcome` flag applies.
    let verdict = match req.params.get("outcome") {
        None => None,
        Some(v) => {
            let Some(name) = v.as_str() else {
                return Response::err(req.id, ERR_BAD_PARAMS, ERR_BAD_VERDICT);
            };
            if crate::envelope::ContributorVerdict::parse(name).is_none() {
                return Response::err(req.id, ERR_BAD_PARAMS, ERR_BAD_VERDICT);
            }
            Some(name.to_string())
        }
    };
```

Add the error constant beside the other `ERR_` constants in this file:

```rust
const ERR_BAD_VERDICT: &str = "outcome must be worked, partly or failed";
```

- [ ] **Step 4: Pass it to `approve`**

At `ipc.rs:1679`, replace the `None` placeholder Task 1 left:

```rust
        if queue.approve(id, &scopes, inputs.as_deref(), verdict.as_deref(), Some(approved_at)) {
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::`
Expected: PASS, including all three new tests and every pre-existing `approve`
test.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/ipc.rs
git commit -m "Accept a contributor verdict on the approve method"
git show --stat HEAD
```

---

### Task 3: Apply the verdict to the envelope that is actually sent

This is the task the whole feature turns on. Read the spec section "Why not
SubmitOptions.verdict" before starting.

**Files:**
- Modify: `crates/trace-commons-contributor/src/envelope.rs` (beside `apply_granted_scopes`, `envelope.rs:568`)
- Modify: `crates/trace-commons-contributor/src/daemon/uploader.rs` (`approved_envelope_for`, `uploader.rs:298`)
- Modify: `crates/trace-commons-contributor/src/daemon/approved_envelope.rs` (module doc only)
- Test: `crates/trace-commons-contributor/src/daemon/uploader.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `QueueEntry.approved_verdict` from Task 1.
- Produces: `pub fn apply_verdict(envelope: &mut TraceContributionEnvelope,
  verdict: ContributorVerdict)` in `crate::envelope`.

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `uploader.rs`. Existing tests at `uploader.rs:1154`,
`:1220` and `:1274` already seed a stored approved envelope with
`approved_envelope::save`; copy that setup exactly rather than building one
inline.

```rust
/// The verdict must reach the envelope that is actually sent.
///
/// The daemon does not rebuild at upload time -- it sends the stored bytes --
/// so a verdict routed through `SubmitOptions` would pass every fresh-build
/// test and be dropped on exactly this path. That was the original design
/// error; this test is its regression guard.
#[test]
fn a_verdict_reaches_a_stored_envelope() {
    let (store, mut entry, envelope) = seeded_stored_envelope();
    entry.approved_verdict = Some("failed".to_string());

    let uploader = test_uploader(&store);
    let sent = uploader
        .approved_envelope_for(&entry)
        .expect("load succeeds")
        .expect("an envelope is stored");

    assert_eq!(sent.outcome.task_success, TaskSuccess::Failure);
    // The stored bytes themselves are untouched.
    let _ = envelope;
}

#[test]
fn no_verdict_leaves_a_stored_envelope_unknown() {
    let (store, entry, _) = seeded_stored_envelope();
    assert_eq!(entry.approved_verdict, None);

    let uploader = test_uploader(&store);
    let sent = uploader
        .approved_envelope_for(&entry)
        .expect("load succeeds")
        .expect("an envelope is stored");

    assert_eq!(sent.outcome.task_success, TaskSuccess::Unknown);
}

/// A verdict is a judgement about the task, not content, so it must not
/// move either consent declaration. #421 pins this for the build path
/// (`a_verdict_declares_no_content`); this extends it to the daemon path,
/// where the verdict is stamped onto an already-redacted envelope and could
/// otherwise disturb flags that were derived before it arrived.
#[test]
fn a_verdict_moves_neither_consent_flag_on_a_stored_envelope() {
    let (store, mut entry, stored) = seeded_stored_envelope();
    let before = (
        stored.consent.message_text_included,
        stored.consent.tool_payloads_included,
    );
    entry.approved_verdict = Some("failed".to_string());

    let uploader = test_uploader(&store);
    let sent = uploader
        .approved_envelope_for(&entry)
        .expect("load succeeds")
        .expect("an envelope is stored");

    assert_eq!(
        (
            sent.consent.message_text_included,
            sent.consent.tool_payloads_included
        ),
        before,
        "a verdict is not content and must not move a consent declaration"
    );
}

/// The digest check guards this crate's own storage and must keep running
/// against the bytes AS STORED. A verdict applied before it would make a
/// tampered file pass.
#[test]
fn a_tampered_stored_envelope_is_still_refused_with_a_verdict_present() {
    let (store, mut entry, _) = seeded_stored_envelope();
    entry.approved_verdict = Some("worked".to_string());
    entry.previewed_envelope_digest = Some("0".repeat(64));

    let uploader = test_uploader(&store);
    assert!(uploader.approved_envelope_for(&entry).is_err());
}
```

Name the seeding helper `seeded_stored_envelope` and the uploader constructor
`test_uploader` only if no equivalents exist; if the module already has them
under other names, use those.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::uploader::tests::a_verdict_reaches_a_stored_envelope`
Expected: FAIL — `task_success` is `Unknown`, because nothing applies the
verdict yet.

- [ ] **Step 3: Add `apply_verdict`**

In `envelope.rs`, directly after `apply_granted_scopes` (which ends around
`envelope.rs:577`):

```rust
/// Stamp the contributor's verdict onto an already-redacted envelope.
///
/// The daemon path cannot supply a verdict at build time: the envelope is
/// built for the preview, before the contributor has answered, and the
/// upload sends those stored bytes rather than rebuilding. So the verdict is
/// applied here, the same post-redaction mutation `apply_granted_scopes`
/// performs.
///
/// Writes `task_success` only. `user_feedback` is a different question --
/// satisfaction rather than completion -- and is deliberately left alone;
/// see `ContributorVerdict::outcome`.
pub fn apply_verdict(envelope: &mut TraceContributionEnvelope, verdict: ContributorVerdict) {
    envelope.outcome.task_success = match verdict {
        ContributorVerdict::Worked => TaskSuccess::Success,
        ContributorVerdict::Partly => TaskSuccess::Partial,
        ContributorVerdict::Failed => TaskSuccess::Failure,
    };
}
```

`TaskSuccess` is already imported in `envelope.rs`. If the compiler says
otherwise, add it to the existing
`trace_commons_protocol::trace_contribution::{...}` import block rather than
adding a new `use` line.

- [ ] **Step 4: Apply it after the digest check**

In `approved_envelope_for` (`uploader.rs:298`), replace the final
`Ok(Some(stored))` with:

```rust
        // AFTER the digest check, never before it. The check is a
        // consistency check on this crate's own storage and has to run
        // against the bytes as stored; applying the verdict first would make
        // a truncated or crossed-over file pass.
        //
        // This is the one deliberate divergence from "the upload sends
        // precisely the stored bytes", and it is bounded to
        // `outcome.task_success`. See this module's doc note.
        let mut stored = stored;
        if let Some(name) = entry.approved_verdict.as_deref()
            && let Some(verdict) = crate::envelope::ContributorVerdict::parse(name)
        {
            crate::envelope::apply_verdict(&mut stored, verdict);
        }
        Ok(Some(stored))
```

An unparseable stored verdict is ignored rather than refused: the IPC boundary
already validates, so a bad value here means a hand-edited queue file, and
refusing the upload would strand the entry.

- [ ] **Step 5: Amend the `approved_envelope` module doc**

`approved_envelope.rs` currently claims, without qualification, that the upload
sends precisely the stored bytes. That is now false in one bounded way. Add
after the paragraph ending `"stops being an equality check and becomes
literally true."`:

```rust
//! One bounded exception. The uploader stamps the contributor's verdict onto
//! `outcome.task_success` after loading and digest-checking these bytes, so
//! the envelope sent differs from the envelope stored by exactly that field.
//! The verdict is collected at approval time, after the preview was rendered,
//! and it is an output of the approval rather than an input that existed when
//! the preview was built. The digest pin therefore describes the previewed
//! bytes; it is not a claim about the final wire bytes. See
//! `envelope::apply_verdict`.
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p trace-commons-contributor --lib daemon::uploader::`
Expected: PASS, including all three new tests.

- [ ] **Step 7: Run the whole crate**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: PASS, 0 failed.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/envelope.rs \
        crates/trace-commons-contributor/src/daemon/uploader.rs \
        crates/trace-commons-contributor/src/daemon/approved_envelope.rs
git commit -m "Stamp the verdict onto the envelope the daemon actually sends"
git show --stat HEAD
```

---

### Task 4: Collect the verdict in the GTK shell

The GTK shell is the only one of the three that compiles in this environment.
macOS and Windows are Task 5, deliberately split so this one can be reviewed
and merged on working, verifiable code.

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/queue.rs` (approve call at `queue.rs:935`)
- Modify: `crates/trace-commons-contributor-gtk/src/ui/preview.rs` (approve call at `preview.rs:971`)
- Test: `crates/trace-commons-contributor-gtk/src/ui/queue.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: the `approve` method's `outcome` parameter from Task 2.
- Produces: nothing later tasks depend on.

- [ ] **Step 1: Read the surrounding code first**

The GTK shell is a separate cargo workspace with its own lockfile and an
offline flatpak vendor set. Both drift when a dependency changes.

**Add no new dependencies in this task.** Build the control from widgets
already in use in these two files.

Read both call sites and the `app.call` helper before writing anything:

```bash
sed -n '900,980p' crates/trace-commons-contributor-gtk/src/ui/queue.rs
sed -n '940,1000p' crates/trace-commons-contributor-gtk/src/ui/preview.rs
```

- [ ] **Step 2: Write the failing test**

Test the parameter construction, not the widgets. Add to `queue.rs`:

```rust
#[test]
fn an_approve_call_carries_the_selected_verdict() {
    let params = approve_params(ApproveTarget::Entry(TEST_ENTRY_ID), Some("partly"));
    assert_eq!(params["entry_id"], TEST_ENTRY_ID.to_string());
    assert_eq!(params["outcome"], "partly");
}

/// No selection sends no parameter at all, rather than a null or an empty
/// string. The daemon distinguishes absent from unrecognised.
#[test]
fn an_approve_call_with_no_verdict_omits_the_parameter() {
    let params = approve_params(ApproveTarget::Entry(TEST_ENTRY_ID), None);
    assert!(params.get("outcome").is_none());
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p trace-commons-contributor-gtk approve_params`
Expected: FAIL — `approve_params` does not exist.

- [ ] **Step 4: Extract the parameter builder**

Add to `queue.rs`, and use it at both `queue.rs:935` and `preview.rs:971` so the
two call sites cannot drift:

```rust
pub(crate) enum ApproveTarget {
    All,
    Project(String),
    Entry(Uuid),
}

/// Build the `approve` parameters. `verdict` is omitted entirely when the
/// contributor did not answer: the daemon distinguishes an absent parameter
/// (`TaskSuccess::Unknown`) from an unrecognised one (refused).
pub(crate) fn approve_params(target: ApproveTarget, verdict: Option<&str>) -> serde_json::Value {
    let mut params = match target {
        ApproveTarget::All => serde_json::json!({"all": true}),
        ApproveTarget::Project(key) => serde_json::json!({"project_id": key}),
        ApproveTarget::Entry(id) => serde_json::json!({"entry_id": id.to_string()}),
    };
    if let Some(name) = verdict {
        params["outcome"] = serde_json::Value::String(name.to_string());
    }
    params
}
```

- [ ] **Step 5: Add the control**

On the approval surface, above the approve button, add three toggle buttons in
a single-selection group labelled `Worked`, `Partly`, `Failed`, plus the
question text `Did this session do what you asked?`.

None selected is the default and stays valid — the approve button is never
disabled by this control. Add a caption under the group:

`Optional. This is recorded as the trace outcome; the preview above does not show it.`

That caption is load-bearing, not decoration: the spec exempts the outcome
fields from the shown-bytes claim, and this sentence is where that exemption
is disclosed to the contributor. Do not drop or soften it.

Follow the existing brand styling in these files. The app matches the
community site rather than the GNOME system accent.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p trace-commons-contributor-gtk`
Expected: PASS.

- [ ] **Step 7: Confirm the GTK lockfile did not move**

```bash
git status --short
```

Expected: no change to `Cargo.lock` or the flatpak vendor manifest. If either
moved, a dependency was added — revert and rebuild the control from existing
widgets.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor-gtk/src/ui/queue.rs \
        crates/trace-commons-contributor-gtk/src/ui/preview.rs
git commit -m "Collect a contributor verdict on the GTK approval control"
git show --stat HEAD
```

---

### Task 5: Collect the verdict in the macOS and Windows shells

**Ship this as a separate pull request.** Neither shell compiles in this
environment; both are verified only by CI (`macOS app tests` runs `swift test`
on `macos-26`; `windows contributor app` runs the .NET tests). Landing them
with Tasks 1-4 would put unverifiable code in the same review as the code that
was verified locally.

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/DaemonClient.swift:200` and `:212`
- Modify: `macos/Sources/TraceCommonsApp/AppModel.swift:750` and `:764`
- Modify: `windows/src/TraceCommons.Interop/DaemonProtocol.cs:40` and its approve call sites
- Test: `macos/Tests/` and the Windows test project

**Interfaces:**
- Consumes: the `approve` method's `outcome` parameter from Task 2.

- [ ] **Step 1: Extend the Swift client**

`DaemonClient.approve(entryID:)` and `approve(projectID:)` each gain a
`verdict: String?` parameter, appended so existing call sites are unambiguous:

```swift
func approve(entryID: String, verdict: String? = nil) throws -> ApproveResponse {
    var params = ["entry_id": entryID]
    if let verdict { params["outcome"] = verdict }
    return try call("approve", params: params, as: ApproveResponse.self)
}
```

Mirror this for `approve(projectID:)`.

- [ ] **Step 2: Write the Swift test**

```swift
func testApproveOmitsOutcomeWhenNoVerdictGiven() throws {
    let params = DaemonClient.approveParams(entryID: "e1", verdict: nil)
    XCTAssertNil(params["outcome"])
}

func testApproveCarriesTheVerdict() throws {
    let params = DaemonClient.approveParams(entryID: "e1", verdict: "partly")
    XCTAssertEqual(params["outcome"], "partly")
}
```

Extract `approveParams` as a static function so it is testable without a live
socket.

- [ ] **Step 3: Add the SwiftUI control**

Three-option picker, no selection by default, same question text and same
caption as Task 4 Step 5. Do not disable the approve button.

- [ ] **Step 4: Mirror both in the Windows shell**

Same parameter, same three options, same caption, same optionality, in
`DaemonProtocol.cs` and its approve call sites, with equivalent tests in the
Windows test project.

- [ ] **Step 5: Push and verify on CI**

These cannot be verified locally. Push the branch, open the pull request, and
confirm `macOS app tests` and `windows contributor app` both pass before
asking for review. Do not claim either shell works until those two jobs are
green.

---

## Verification before claiming done

Run all of these and paste the actual output:

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo test -p trace-commons-contributor-gtk
```

Confirm `git show --stat` on each commit touched only the intended files.
