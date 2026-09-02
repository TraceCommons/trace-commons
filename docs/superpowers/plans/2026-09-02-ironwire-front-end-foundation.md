# IronWire front end: daemon foundation and the GTK shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a declared IronWire proxy falsifiable -- a contributor can declare it, be told immediately whether it worked, and see afterwards whether data is arriving -- and render that in one shell.

**Architecture:** Two new daemon capabilities (a declaration-time probe, and a routing block on the existing status payload), one new `PreviewSummary` block so routing data is named where a contributor reviews what they are sending, and the GTK settings UI that consumes all three.

**Tech Stack:** Rust. `trace-commons-contributor` daemon, `trace-commons-contributor-gtk`, existing JSON-RPC-over-socket IPC.

**Spec:** [`docs/superpowers/specs/2026-09-02-ironwire-front-end-design.md`](../specs/2026-09-02-ironwire-front-end-design.md) -- read "The problem this spec exists to solve", "The `IRONWIRE_HOME` gap", and "What actually leaves the machine" before starting.

## Hard prerequisite

**PR #513 must be merged before Task 1 begins.** It introduces the `ironwire`
settings key, `IronWireDeclaration`, `IronWireLedger` and `ironwire_ledger_for()`.
Nothing in this plan compiles without them. #513 was BEHIND `main` at the time
of writing; if it has not landed, stop and say so rather than stubbing its types.

## Why this slice

`routing/mod.rs` states the rule the client library lives by: *"Absence and
failure are the same state... Nothing here can fail a submission."* That is
right for the submission path -- a proxy that went away must never cost a
contributor a trace -- and unacceptable for a setting.

As #513 stands, a contributor can declare `ironwire` with a wrong port, get no
error, see no indicator, and have every trace silently carry no routing data.
They would reasonably conclude it works. The `$IRONWIRE_HOME` gap makes it worse:
a GUI-launched daemon does not inherit shell environment, so the variable is
unset however the contributor configured their shell, and a token read failure
is indistinguishable from "off".

This plan does not weaken the submission rule. It adds a second path --
declaring -- which must answer.

## Global Constraints

- **The submission path stays untouched.** No change in this plan may make
  `TraceSource::load`, `exchanges_since`, or `refresh` able to fail a
  submission. The probe runs only when a human asks.
- **The token is a credential.** `IronWireLedger`'s hand-written `Debug` exists
  to keep it out of logs. Never log it, never return it over IPC, never put it
  in a probe result, never store it in settings.
- **`cost_usd` is priced, not billed.** `routing/mod.rs`: subscription work is
  priced at what it *would* have cost on the meter, and **no surface may render
  it as money the contributor spent.** No copy in this plan may say "spent",
  "your cost", or read as a bill.
- **Routing data is attribution only.** It must never reach a gate, a scoring
  input, or a credit computation, and no UI may place it beside credit figures
  in a way that implies otherwise.
- **Consent copy is out of scope.** The `routing_metadata_included` flag's
  wording is pinned by `crates/trace-commons-protocol/tests/consent_policy_pin.rs`
  against the published page at `https://tracecommons.ai/legal/`, which lives in
  the **trace-commons-community repo**. Do not add, edit, or reword any
  consent-flag text. If a task appears to require it, stop and report.
- **Every negative assertion names its specific error variant or value.** Not
  `assert!(x.is_err())`. Eighteen assertions on this project turned out
  structurally incapable of failing.
- **Mutation-check every guard.** Break the thing it protects, watch it go red,
  revert, report the failure text.
- No emojis. Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- Verify with `RUSTFLAGS="-D warnings" cargo test --workspace` and **confirm the
  run terminated**. Plain `cargo check` does not apply `-D warnings`; CI does.
- **Disk:** this machine ran out of space and broke a running agent. Check
  `df -h /` before and after cargo commands. Under ~8 GB free, stop and report.

## File structure

- **Modify** `crates/trace-commons-contributor/src/daemon/ipc.rs` -- probe method, routing status block
- **Modify** `crates/trace-commons-contributor/src/daemon/settings.rs` -- token path resolution surfaced
- **Modify** `crates/trace-commons-contributor/src/daemon/preview.rs` -- `PreviewSummary` routing block
- **Modify** `crates/trace-commons-contributor-gtk/src/ui/settings.rs` -- declaration control, status line
- **Modify** `crates/trace-commons-contributor-gtk/src/ui/preview.rs` -- render the routing block
- **Modify** `crates/trace-commons-contributor-gtk/src/copy.rs` -- all wording

---

### Task 1: Ask the proxy, and say what was found

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs`

**Interfaces:**
- Produces: IPC method `probe_routing`, taking `{"port": u16}`, returning one of three outcomes

**Read `ironwire_ledger_for()` first and design to what it actually does.** The
spec could not establish, from #513's patch set, what happens when
`$IRONWIRE_HOME` is unset -- whether it falls back to a conventional directory
or simply fails. **That behaviour decides this task's shape**, so establish it
before writing anything, and say in your report what you found.

The probe needs network I/O, so it belongs with the async methods. The sync
dispatch in `ipc.rs` answers several methods with
`Response::err(req.id, ERR_UNAVAILABLE, "<name>-requires-async")` -- follow that
existing pattern rather than inventing one.

Three outcomes, each distinguishable by the caller:

- **Reachable and readable** -- the proxy answered and the token worked.
- **Token unreadable** -- carry **the absolute path that was tried**. This is
  the whole point of the task: it is the failure a contributor can fix, and
  today it is silently identical to "off".
- **Not reachable** -- carry the port that was tried.

**Never carry the token itself**, in any outcome.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn a_probe_against_a_dead_port_reports_unreachable_with_the_port() {
    // Bind and drop, so the port is real and definitely closed.
    // Assert the specific outcome and that the port appears. Not is_err().
}

#[tokio::test]
async fn a_probe_that_cannot_read_the_token_names_the_path_it_tried() {
    // The failure a contributor can act on. The assertion is that an
    // absolute path is present in the response, not merely that it failed.
}

#[tokio::test]
async fn a_probe_never_returns_the_token() {
    // Serve a reachable proxy with a known token value, probe it, and assert
    // the serialized response does not contain that value anywhere.
}

#[tokio::test]
async fn a_successful_probe_reports_reachable() {
    // Use the axum loopback-server pattern already in
    // routing/ironwire.rs's tests rather than inventing a mock.
}
```

- [ ] **Step 2: Run to verify they fail**
- [ ] **Step 3: Implement**
- [ ] **Step 4: Run to verify they pass**
- [ ] **Step 5: Mutation-check** -- make the token-unreadable outcome return a
  bare "failed" with no path, and confirm the path test goes red. Report the
  failure text. Then confirm the probe still cannot be reached from
  `TraceSource::load`: grep the submission path for the new method and show it
  is absent.
- [ ] **Step 6: Commit**

---

### Task 2: Report whether anything is arriving

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs`

**Interfaces:**
- Consumes: `IronWireLedger::has_rows()` (already `pub`, reaches nothing today)
- Produces: a `routing` block on `status_value()`

`status_value()` (`ipc.rs:521`) is a `serde_json::json!` block with additive
sub-objects -- `health` and `daily_budget` are the precedent, and
`daily_budget`'s comment explains why it is reported independently rather than
folded into `health`. Add `routing` the same way.

The block must distinguish three states, because collapsing them is the bug
this plan exists to fix:

- not declared
- declared, nothing seen yet
- declared, rows seen

`has_rows()` alone cannot express the first, and `#513`'s own doc says so: the
real distinction is `Option<Arc<IronWireLedger>>` on the daemon's shared state.

**`has_rows` is a poor health signal** -- it says data exists, not that the proxy
is answering now. If a last-successful-refresh timestamp can be added to
`IronWireLedger` without touching the submission path, do it and report it;
if it cannot be done cleanly, report why and ship the three states.

`IPC_SCHEMA` is `"trace_commons.daemon.v1_1"` (`ipc.rs:132`). **Determine
whether an additive field requires a bump** by finding what the shells do with
unknown fields, and say what you found. Do not bump it on a guess in either
direction.

- [ ] **Step 1: Write the failing tests** -- one per state, asserting the exact
  JSON shape. `#513`'s `daemon::ipc::tests::routing_transition` already
  exercises the declared/undeclared distinction; find it and build on it rather
  than duplicating it.
- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- collapse "declared, nothing seen" into "not
  declared" and confirm a test goes red. That collapse is precisely the defect.
- [ ] **Step 6: Commit**

---

### Task 3: Name routing data where a contributor reviews it

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/preview.rs`

**Interfaces:**
- Produces: a `routing` block on `PreviewSummary`

Today routing events appear in the preview only as growth in `event_count` and
`would_send_bytes`. `redactions` and `pii_labels_present` never fire on them
because `content` is always `None`. **`cost_usd` appears nowhere on any shell.**

There is direct precedent: `PreviewSummary` already carries `subagent_count`, a
per-category count added when subagent events became something a contributor
should see named. Routing takes the same shape of answer.

Add a `routing` block carrying: exchange count, distinct served models, and
total priced cost.

**The cost field's name and rendering must not read as spend.** `routing/mod.rs`
is explicit: priced at what it would have cost on the meter, never money the
contributor spent. Name the field so a UI author who never reads the spec still
cannot render it wrongly -- `priced_usd` says more than `cost_usd` does.

**Do not touch consent-flag text.** This is the preview sheet, which is
data-driven; the pinned legal wording is a separate surface and out of scope.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_preview_with_no_routing_events_reports_an_empty_routing_block() {
    // Not absent, not null -- decide which and pin it, because three shells
    // will deserialize this and a shape that changes between cases is a bug
    // in each of them.
}

#[test]
fn a_preview_counts_distinct_served_models_not_exchanges() {
    // Two exchanges on one model is one model, two exchanges.
}

#[test]
fn a_preview_totals_the_priced_cost_of_every_routing_event() { /* ... */ }

#[test]
fn a_routing_event_contributes_no_redaction_or_pii_label() {
    // content is always None, so a routing row can never carry either.
    // This is the assertion that catches someone later giving routing
    // events content.
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Verify the other shells still parse.** macOS `PreviewSummary.swift`
  and Windows `PreviewSummary.cs` mirror this type field-for-field. Establish
  whether they ignore unknown fields or fail on them, and **report what you
  found** -- if either is strict, this task breaks two shells and that is a
  finding, not something to work around here.
- [ ] **Step 6: Mutation-check** -- make the total sum raw exchange counts
  instead of priced cost and confirm the total test goes red.
- [ ] **Step 7: Commit**

---

### Task 4: The GTK surface

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/settings.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/ui/preview.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs`

**Interfaces:**
- Consumes: `probe_routing` (Task 1), the status `routing` block (Task 2), the preview `routing` block (Task 3)

GTK goes first because it is the only shell with a centralised copy module
(`src/copy.rs`, 2,594 lines) -- wording settled here is what Windows and macOS
copy later.

Four surfaces:

**The declaration.** A toggle plus a port field enabled only when on. Default
the port field to IronWire's conventional value so nobody must know it, but
**write nothing until the contributor acts** -- `None` means off, and a
displayed default must never become a declaration. Send through `set_settings`
the way `ui/settings.rs:932` already does; note that `set_settings` rejects
unknown keys, so the key must match exactly.

**The probe result**, rendered on save. Three outcomes, three strings. The
token-unreadable string must include the path the daemon reported -- do not
write a generic "check your configuration".

**The status line**, from Task 2's three states. "Declared, nothing seen yet" is
not an error and must not read as one: a proxy installed today legitimately
reports it.

**The offer.** If the daemon reports a readable token and routing is not
declared, surface routing as an offer rather than a buried setting. If there is
no token, show nothing -- a contributor without IronWire is never asked about
it.

**Restart semantics, stated.** The ledger is built once at daemon construction,
so a declaration change takes effect on restart. Say so in the copy. This is
true on every shell, not a GTK detail.

- [ ] **Step 1: Write the failing tests** -- GTK UI code here is testable at the
  copy and state-mapping level rather than by driving widgets. Test that each
  daemon state maps to the intended copy key, including that the
  token-unreadable case propagates the path. Follow whatever pattern
  `ui/settings.rs` tests already use; **do not invent a widget-driving harness.**
- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- map "declared, nothing seen yet" to the error
  copy and confirm a test goes red. Then check every new string for the words
  "spent" or "cost you"; there must be none.
- [ ] **Step 6: Commit**

---

## Not in this plan

- **Windows and macOS shells.** Their own plan, after wording settles here.
  macOS is the constrained one: it never calls `set_settings` at runtime -- its
  only IPC methods are `set_project_mode`, `set_consent_scopes`,
  `set_public_profile` -- so a declaration means a daemon restart through
  `tc_daemon_start_with_settings`.
- **Consent-flag wording.** Blocked on the policy-version question, which is
  cross-repo and has a lead time nothing here has: #513 added
  `routing_metadata_included` and its pin entry **without bumping**
  `TRACE_CONTRIBUTION_POLICY_VERSION` (still `2026-04-24`), because the pin test
  enforces bumps for scope changes, not flag additions. Whether a new content
  category may ship under an unchanged policy version is a decision for whoever
  owns the legal page.
- **Onboarding steps.** The spec's answer is detection, not a new step, and the
  offer in Task 4 is that detection.
- **Notifying a contributor whose declared proxy has gone quiet.** An open
  question in the spec; the status line is the prerequisite either way.
- **Making the ledger rebuildable without a daemon restart.** More daemon work,
  and it cannot help macOS regardless.
