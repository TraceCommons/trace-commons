# One-click submit: daemon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `approve` able to send a session nobody opened, and able to take a whole project at once, returning what redaction found.

**Architecture:** `approve` stops meaning "mark approved" and starts meaning
"ensure a pinned envelope exists, then approve". The build already exists as
`build_and_pin_preview`; approve calls it for entries with no pin. A
`project_id` parameter joins the existing entry-id and `all` forms. The
response carries the aggregate signal so a client needs no second call. All of
it lands in `daemon/ipc.rs` beside the handler it changes.

**Tech Stack:** Rust, tokio, serde_json. No new dependencies.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-20-one-click-submit-design.md`.
- Hash-only, label-only: no path, URL, token, or trace content in any error
  string, audit row, or response field.
- `RUSTFLAGS="-D warnings"` for every check and test.
- Clippy with the repo allow-list, unwidened: `-A clippy::type_complexity
  -A clippy::collapsible_if -A clippy::manual_option_as_slice
  -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- `cargo fmt --all` before every commit; `git show --stat` after, because the
  repo is not rustfmt-clean and the post-edit hook can balloon a small diff.
- No emojis. Repo-relative paths only.
- Baselines to hold: workspace 73 test targets green.

## Existing interfaces this plan builds on

- `build_and_pin_preview(shared: &DaemonShared, entry_id: Uuid, entry:
  &queue::QueueEntry, cfg: Option<&ContributorConfig>) -> Result<(PreviewSummary,
  String, TraceContributionEnvelope), (&'static str, &'static str)>`
  — `ipc.rs:1344`. Pins only when `summary.enrolled`.
- `PreviewSummary` — `preview.rs:271`: `redactions: BTreeMap<String, u32>`,
  `pii_labels_present: Vec<String>`, `residual_risk: String`, `enrolled: bool`.
- `Queue::pending(&self) -> Vec<&QueueEntry>` — `queue.rs:315`.
- `Queue::approve(&mut self, entry_id: Uuid, scopes: &[String], inputs:
  Option<&str>, approved_at: Option<DateTime<Utc>>) -> bool` — `queue.rs:381`.
  Returns false unless the entry is `Pending`.
- `QueueEntry::previewed_envelope_digest: Option<String>` — `queue.rs:125`.
- `project_id_for(&e.project_key)` — the id `entry_value` publishes.

---

### Task 1: move approve onto the async dispatcher

`approve` lives in the SYNC `handle_request` (`ipc.rs:571`), and
`build_and_pin_preview` is async and only reachable from
`handle_request_async` (`ipc.rs:1207`). Every later task in this plan needs
that await, so this move comes first and lands on its own.

Production is unaffected: the module header at `ipc.rs:81` states
`handle_request_async` is the complete dispatcher and every transport runs it,
falling through to `handle_request` for sync methods. Only test call sites
change.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (the `"approve"` arm at 808; the dispatcher at 1207)
- Modify: the two sync approve call sites in `mod tests` (`ipc.rs:2264`, `ipc.rs:2863`)

**Interfaces:**
- Produces: `async fn handle_approve(shared: &DaemonShared, req: &Request) -> Response`, registered as `"approve" => handle_approve(shared, req).await`.

- [ ] **Step 1: Move the arm verbatim**

Cut the whole `"approve" => { ... }` body out of `handle_request` into a new
`async fn handle_approve` beside `handle_preview`, changing nothing inside it
yet. Register it in `handle_request_async` above the `_ =>` fallthrough.

- [ ] **Step 2: Watch the two existing tests fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::tests`
Expected: FAIL — `bulk_approval_over_the_socket_is_now_allowed_and_appends_an_audit_entry`
and its sibling call `handle_request` with `"approve"`, which now returns
method-not-found.

That failure is the point: it proves the move actually changed the routing.

- [ ] **Step 3: Point them at the async dispatcher**

Make both `#[tokio::test]` and call
`handle_request_async(&s, &req("approve", ...)).await`.

- [ ] **Step 4: Run the crate**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: PASS, same count as before the move.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/ipc.rs
git commit -m "Move approve onto the async dispatcher"
git show --stat HEAD
```

---

### Task 2: approve builds and pins when nothing previewed

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (`handle_approve`, from Task 1)
- Test: `crates/trace-commons-contributor/tests/daemon_ipc_contract.rs`

**Why the test lives there:** `build_and_pin_preview` calls `source.load(...)`,
so the test needs a real session file on disk and an enrolled config. The unit
`mod tests` fixture `shared()` (`ipc.rs:2073`) is an empty store with neither.
`tests/daemon_ipc_contract.rs` already seeds loadable sessions and drives the
daemon over its socket; extend its existing fixture rather than building a
second one. Note that file is `#![cfg(unix)]` — the socket path is unix-only,
which is correct for this test.

**Interfaces:**
- Consumes: `build_and_pin_preview`, `Queue::approve`, `QueueEntry::previewed_envelope_digest`.
- Produces: `approve` leaves every entry it approved with `previewed_envelope_digest.is_some()`.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn approving_without_a_preview_still_pins_an_envelope() {
    // The uploader rebuilds and compares against the pin; an approval with
    // no pin means the uploader builds a fresh envelope and sends it
    // (uploader.rs:191 -> submit.rs:518), so an unpinned approval uploads
    // bytes nobody was shown and reports success.
    let (daemon, store) = enrolled_daemon_with_one_pending_session().await;
    let id = first_pending_entry_id(&daemon).await;
    assert!(
        queue_entries(&daemon)
            .await
            .iter()
            .find(|e| e.entry_id == id)
            .expect("entry")
            .previewed_envelope_digest
            .is_none(),
        "fixture must start unpinned or this test proves nothing"
    );

    call(&daemon, "approve", serde_json::json!({ "entry_id": id })).await;

    let entries = queue_entries(&daemon).await;
    let e = entries.iter().find(|e| e.entry_id == id).expect("entry");
    assert!(
        e.previewed_envelope_digest.is_some(),
        "approve must build and pin an envelope when no preview ran"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::tests::approving_without_a_preview_still_pins_an_envelope`
Expected: FAIL — `previewed_envelope_digest` is `None`, because today's approve only sets state.

- [ ] **Step 3: Build and pin before approving**

In the `"approve"` arm, after `ids` is resolved and before the approve loop,
build for anything unpinned. Take no queue lock across the await:

```rust
// Entries nobody previewed have no artifact behind them. Build one now:
// the uploader rebuilds at send time and compares against this pin, so an
// approval without it is refused and re-offered rather than sent.
let unpinned: Vec<(Uuid, super::queue::QueueEntry)> = {
    let queue = shared.queue.lock().expect("queue lock");
    ids.iter()
        .filter_map(|id| queue.all().iter().find(|e| e.entry_id == *id))
        .filter(|e| e.previewed_envelope_digest.is_none())
        .map(|e| (e.entry_id, e.clone()))
        .collect()
};
let mut built: BTreeMap<String, u32> = BTreeMap::new();
let mut flagged = 0usize;
let mut skipped: Vec<(Uuid, &'static str)> = Vec::new();
for (id, entry) in unpinned {
    match build_and_pin_preview(shared, id, &entry, cfg.as_ref()).await {
        Ok((summary, _body, _envelope)) => {
            for (k, v) in &summary.redactions {
                *built.entry(k.clone()).or_default() += v;
            }
            if !summary.pii_labels_present.is_empty() {
                flagged += 1;
            }
        }
        Err((_code, label)) => skipped.push((id, label)),
    }
}
let skipped_ids: std::collections::HashSet<Uuid> =
    skipped.iter().map(|(id, _)| *id).collect();
```

Then skip `skipped_ids` in the approve loop.

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::tests::approving_without_a_preview_still_pins_an_envelope`
Expected: PASS

- [ ] **Step 5: Prove the pin is the one the uploader accepts**

```rust
#[tokio::test]
async fn an_unpreviewed_approval_is_not_re_offered_by_the_uploader() {
    let (daemon, store) = enrolled_daemon_with_one_pending_session().await;
    let id = first_pending_entry_id(&daemon).await;
    call(&daemon, "approve", serde_json::json!({ "entry_id": id })).await;

    let entries = queue_entries(&daemon).await;
    let e = entries.iter().find(|e| e.entry_id == id).expect("entry");
    let pinned = e.previewed_envelope_digest.clone().expect("pinned");
    let saved = trace_commons_contributor::daemon::approved_envelope::load(&store, id)
        .expect("load")
        .expect("an envelope must be on disk, not only a digest");
    assert_eq!(
        trace_commons_contributor::daemon::preview::envelope_digest(&saved).expect("digest"),
        pinned,
        "the pinned digest must name the bytes actually persisted"
    );
}
```

- [ ] **Step 6: Run both, then the crate**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: PASS, no regression against the pre-change count.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/ipc.rs
git commit -m "Build and pin an envelope when approving something unpreviewed"
git show --stat HEAD
```

---

### Task 3: approve takes a project

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (the `"approve"` arm, `ids` resolution)
- Test: same file's `mod tests`

**Interfaces:**
- Consumes: `Queue::pending`, `project_id_for`.
- Produces: `approve {"project_id": "proj_..."}` approves that project's pending entries and no others.

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn approving_a_project_takes_that_project_and_no_other() {
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await; // 2 in one, 1 in another
    let target = first_pending_project_id(&daemon).await;

    let v = call(&daemon, "approve", serde_json::json!({ "project_id": target })).await;
    assert!(v["approved"].as_u64().unwrap_or(0) > 0, "nothing approved: {v}");

    for e in queue_entries(&daemon).await {
        let want = project_id_for(&e.project_key) == target;
        assert_eq!(
            e.state == trace_commons_contributor::daemon::queue::QueueState::Approved,
            want,
            "entry in {} should{} be approved",
            e.project_label,
            if want { "" } else { " not" }
        );
    }
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::tests::approving_a_project_takes_that_project_and_no_other`
Expected: FAIL — `project_id` is ignored, so `parse_entry_id` errors and nothing is approved.

- [ ] **Step 3: Resolve ids from the project**

Replace the `ids` resolution so the three forms are explicit and mutually exclusive:

```rust
let project_id = req.params.get("project_id").and_then(|v| v.as_str());
let ids: Vec<Uuid> = if all {
    queue.pending().iter().map(|e| e.entry_id).collect()
} else if let Some(pid) = project_id {
    // Only Pending: an entry already approved has had its terms fixed, and
    // a project-wide call must not silently re-pin them.
    queue
        .pending()
        .iter()
        .filter(|e| project_id_for(&e.project_key) == pid)
        .map(|e| e.entry_id)
        .collect()
} else {
    match parse_entry_id(&req.params) {
        Ok(id) => vec![id],
        Err(e) => return e,
    }
};
```

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::tests::approving_a_project_takes_that_project_and_no_other`
Expected: PASS

- [ ] **Step 5: Pin the empty case**

```rust
#[tokio::test]
async fn approving_a_project_with_nothing_pending_is_not_an_error() {
    // A client can race a sweep. Zero approved is an outcome, not a fault.
    let (daemon, _store) = enrolled_daemon_with_sessions_in_two_projects().await;
    let v = call(&daemon, "approve", serde_json::json!({ "project_id": "proj_0000000000000000" })).await;
    assert_eq!(v["approved"].as_u64(), Some(0));
}
```

- [ ] **Step 6: Run the crate**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: PASS

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/ipc.rs
git commit -m "Let approve take a project id"
git show --stat HEAD
```

---

### Task 4: approve reports what it found

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (the `"approve"` arm's response)
- Modify: `docs/contributor-daemon-ipc-v1_1.md`
- Test: same file's `mod tests`

**Interfaces:**
- Consumes: the `built` and `flagged` accumulators from Task 2, `skipped` from Task 2.
- Produces: the approve response shape three shells render:
  `{"approved": u64, "skipped": [{"entry_id": Uuid, "reason_label": String}],
    "redactions": {String: u32}, "flagged": u64}`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn approve_reports_counts_a_client_can_show_without_asking_again() {
    let (daemon, _store) = enrolled_daemon_with_one_pending_session().await;
    let id = first_pending_entry_id(&daemon).await;
    let v = call(&daemon, "approve", serde_json::json!({ "entry_id": id })).await;
    assert_eq!(v["approved"].as_u64(), Some(1));
    assert!(v["redactions"].is_object(), "counts drive the toast");
    assert!(v["flagged"].is_u64());
    assert!(v["skipped"].is_array());
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::tests::approve_reports_counts_a_client_can_show_without_asking_again`
Expected: FAIL — today's response has no such fields.

- [ ] **Step 3: Return the aggregate**

```rust
// The signal the contributor sees instead of a preview. Counts and labels
// only: a redaction count names a category, never the text it removed.
Response::ok(
    req.id,
    serde_json::json!({
        "approved": approved_count,
        "flagged": flagged,
        "redactions": built,
        "skipped": skipped
            .iter()
            .map(|(id, label)| serde_json::json!({
                "entry_id": id,
                "reason_label": label,
            }))
            .collect::<Vec<_>>(),
    }),
)
```

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc::tests::approve_reports_counts_a_client_can_show_without_asking_again`
Expected: PASS

- [ ] **Step 5: Pin that a skip is never silent**

```rust
#[tokio::test]
async fn a_partial_batch_accounts_for_every_entry_it_was_given() {
    let (daemon, _store) = enrolled_daemon_with_one_good_and_one_oversized_session().await;
    let v = call(&daemon, "approve", serde_json::json!({ "all": true })).await;
    let approved = v["approved"].as_u64().expect("approved");
    let skipped = v["skipped"].as_array().expect("skipped").len() as u64;
    assert_eq!(approved + skipped, 2, "every entry attempted must be accounted for");
    for s in v["skipped"].as_array().unwrap() {
        let label = s["reason_label"].as_str().expect("label");
        assert!(!label.contains('/'), "a reason label must not carry a path: {label}");
    }
}
```

- [ ] **Step 6: Document the shape**

In `docs/contributor-daemon-ipc-v1_1.md`, under `approve`: record the three
parameter forms (`entry_id`, `all`, `project_id`), the response fields, and
that approve now builds and pins an envelope for anything unpreviewed — so a
client that never calls `preview` still produces an upload the uploader
accepts.

- [ ] **Step 7: Run the crate and commit**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
cargo fmt --all
git add crates/trace-commons-contributor/src/daemon/ipc.rs docs/contributor-daemon-ipc-v1_1.md
git commit -m "Report what approve built, and document the three forms"
git show --stat HEAD
```

---

### Task 5: the CLI can submit without looking

**Files:**
- Modify: `crates/trace-commons-contributor/src/commands.rs` (the approve command, near line 2094)
- Test: `crates/trace-commons-contributor/src/commands.rs` (its `mod tests`)

**Interfaces:**
- Consumes: the `project_id` parameter from Task 3.
- Produces: `trace-commons-contributor approve --project <id>`; existing
  `--all` and positional-id forms unchanged.

This task exists because it makes the feature usable and testable before any
shell work, and because the CLI is the surface this project's own operators
actually drive.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn approve_accepts_a_project_or_an_id_or_all_but_not_two_at_once() {
    let err = approve_args_error(&["--all", "--project", "proj_abc"]);
    assert!(
        err.contains("one of"),
        "ambiguous selection must be refused, got: {err}"
    );
}
```

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p trace-commons-contributor --lib commands::tests::approve_accepts_a_project_or_an_id_or_all_but_not_two_at_once`
Expected: FAIL — no `--project` flag exists.

- [ ] **Step 3: Add the flag and the refusal**

Add `--project <PROJECT_ID>` beside `--all`, refuse more than one selector
with `give exactly one of: an entry id, --all, or --project <id>`, and send
`project_id` in the params.

- [ ] **Step 4: Run it and watch it pass**

Run: `cargo test -p trace-commons-contributor --lib commands::tests::approve_accepts_a_project_or_an_id_or_all_but_not_two_at_once`
Expected: PASS

- [ ] **Step 5: Full workspace**

Run: `RUSTFLAGS="-D warnings" cargo test --workspace`
Expected: 73 test targets green, no regression.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-contributor/src/commands.rs
git commit -m "Add approve --project to the CLI"
git show --stat HEAD
```

---

## Fixtures this plan requires

Three helpers, added to `tests/daemon_ipc_contract.rs` alongside its existing
session seeding. Each returns a started daemon and its store:

- `enrolled_daemon_with_one_pending_session()` — one loadable session, an
  enrolled config, one `Pending` entry with `previewed_envelope_digest: None`.
- `enrolled_daemon_with_sessions_in_two_projects()` — three sessions under two
  distinct project keys, so a project filter has something to exclude.
- `enrolled_daemon_with_one_good_and_one_oversized_session()` — one loadable
  session and one over `claude_code::GROUP_RAW_BYTE_BUDGET` (64 MB), which the
  scan-streaming change declines by name. Generate the oversized one rather
  than committing it.

Plus these accessors, if the file does not already have equivalents — check
before adding: `call(&daemon, method, params) -> serde_json::Value` (sends over
the socket and returns `result`, panicking on an error frame),
`first_pending_entry_id(&daemon) -> Uuid`,
`first_pending_project_id(&daemon) -> String`, and
`queue_entries(&daemon) -> Vec<QueueEntry>`.

## What this plan does not cover

The three shells. They consume Tasks 2-4 and are thin: a row action calling
`approve` with an entry id, a project action calling it with a project id, and
a toast rendering the returned counts over `approval_hold_secs` with Undo
mapped to the existing revoke path. That is a separate plan, written once this
one lands, so the shells are built against an interface that exists rather
than one that is still moving.
