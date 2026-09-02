# IronWire front end: the daemon foundation and the private-coding app Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a declared IronWire proxy falsifiable and immediate -- a contributor declares it, is told at once whether it worked, sees afterwards whether anything is arriving, and never restarts the app -- and render that in one shell.

**Architecture:** Three daemon capabilities (a declaration-time probe, a routing block on the status payload, and a hot-swappable ledger), plus the GTK surface that consumes them.

**Tech Stack:** Rust. `trace-commons-contributor` daemon, `trace-commons-contributor-gtk`, the existing JSON-RPC-over-socket IPC.

**Spec:** [`docs/superpowers/specs/2026-09-02-ironwire-front-end-design.md`](../specs/2026-09-02-ironwire-front-end-design.md) -- read "The organising principle", "The `IRONWIRE_HOME` gap" and "Changes apply immediately" before starting.

## Scope: the private-coding app only

The spec describes three apps, one per level of access. **This plan builds only
the first**: someone routing their tools through NEAR AI, with no invite and
nothing shared. They get two tabs, Home and Tools, and the app says nothing
about corpora, credits, ownership or contributing, because none of it is
reachable for them and a greyed-out door is worse than no door.

That scoping is what keeps this plan honest: everything here is buildable
against `main` today. The unlock card, the ownership screen and any earnings
figure depend on attestation receipts and on a scoring answer we do not yet have,
and they are a later plan.

## Why this slice

`routing/mod.rs` states the rule the client library lives by: *"Absence and
failure are the same state... Nothing here can fail a submission."* That is
right for the submission path -- a proxy that vanished must never cost someone a
trace -- and unacceptable for a setting.

As `main` stands, a contributor can declare `ironwire` with a wrong port, get no
error, see no indicator, and have every trace silently carry no routing data.
And the token path makes it worse: `ironwire_ledger_for` reads `IRONWIRE_HOME`
or falls back to `~/.ironwire`, and **a GUI-launched daemon never sees that
variable**, so it always takes the fallback whatever the contributor configured
in a shell.

This plan does not weaken the submission rule. It adds a second path --
declaring -- which must answer.

## Global Constraints

- **The submission path stays untouched.** Nothing here may make
  `TraceSource::load`, `exchanges_since` or `refresh` able to fail a submission.
  The probe runs only when a human asks.
- **The token is a credential.** `IronWireLedger`'s hand-written `Debug` exists
  to keep it out of logs. Never log it, never return it over IPC, never put it
  in a probe result, never store it in settings. **The token DIRECTORY is not
  the token** -- storing a path is fine and is the point of Task 1.
- **`cost_usd` is priced, not billed.** `routing/mod.rs`: subscription work is
  priced at what it would have cost on the meter, and no surface may render it
  as money the contributor spent. Nothing in this plan should show a money
  figure at all; if you find yourself adding one, stop.
- **Routing data is attribution only** -- never a gate, scoring input or credit
  computation, and no UI may place it beside credit figures.
- **Say nothing about sharing, corpora, credits or earning.** This app's user
  cannot reach any of it. Copy that hints at it is an advert for a locked door.
- **Every negative assertion names its specific error variant or value.** Not
  `assert!(x.is_err())`. Twenty-one assertions on this project have turned out
  structurally incapable of failing.
- **Mutation-check every guard**, and prefer ground truth from outside the code
  under test: a review found two mutations surviving 46 tests because every test
  reached the function through another function that used it.
- No emojis. Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- Verify with `RUSTFLAGS="-D warnings" cargo test --workspace`, **capture
  cargo's own exit code** (a `tail` in a pipe reported success over a failed
  build on this machine), and confirm the run terminated.
- Check `df -h /` around cargo commands; this machine ran out of disk and killed
  a running agent.

## File structure

- **Modify** `crates/trace-commons-contributor/src/daemon/settings.rs` -- token directory, resolution order
- **Modify** `crates/trace-commons-contributor/src/daemon/ipc.rs` -- probe method, routing status block, hot-swappable ledger
- **Modify** `crates/trace-commons-contributor-gtk/src/ui/settings.rs` -- the Tools surface
- **Modify** `crates/trace-commons-contributor-gtk/src/copy.rs` -- all wording

---

### Task 1: Make the token reachable from a GUI

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs`

**Interfaces:**
- Produces: an optional token directory on `IronWireDeclaration::Watch`, and a documented resolution order in `ironwire_ledger_for`

**The decision this task encodes.** `ironwire_ledger_for` currently resolves
`IRONWIRE_HOME`, else `~/.ironwire`, then reads `control.token`. **Environment
variables are not set for a GUI install** -- an app launched from Finder, Dock
or a desktop entry gets the session manager's environment, not a shell profile's.
So the variable is not a configuration mechanism for the apps this plan is
about; it works for a CLI started from a shell and never for them.

Resolution order becomes: **the declared path, then `IRONWIRE_HOME`, then
`~/.ironwire`.** Settings first, because settings are the only one of the three
a GUI contributor can actually set. The variable stays supported so nothing
breaks for CLI users.

Store the **directory**, not the token. The token stays read at call time and
never enters our settings file, which is the existing rule and a good one.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_declared_token_directory_wins_over_the_environment() {
    // Both set, pointing at different directories, each holding a distinct
    // token. Assert which token the ledger was built with -- not merely that
    // one was built.
}

#[test]
fn the_environment_is_still_honoured_when_no_path_is_declared() {
    // The CLI case, which must keep working.
}

#[test]
fn a_declared_directory_with_no_token_yields_no_reader() {
    // Absence and failure stay the same state at this layer. The DIFFERENCE
    // is reported by the probe in Task 2, not here.
}
```

Reading the token back out of the built ledger needs care: `IronWireLedger`
deliberately keeps the token out of `Debug`. Add a `#[cfg(test)]` accessor
rather than weakening `Debug`, and say in your report that you did.

- [ ] **Step 2: Run to verify they fail**
- [ ] **Step 3: Implement**
- [ ] **Step 4: Run to verify they pass**
- [ ] **Step 5: Mutation-check** -- reverse the precedence so the environment
  wins, and confirm the first test goes red. Report the failure text.
- [ ] **Step 6: Commit**

---

### Task 2: Ask the proxy, and say what was found

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs`

**Interfaces:**
- Consumes: Task 1's resolution order
- Produces: IPC method `probe_routing`, taking `{"port": u16, "token_dir": Option<String>}`, returning one of three outcomes

The probe needs network I/O, so it belongs with the async methods. The sync
dispatch answers several with
`Response::err(req.id, ERR_UNAVAILABLE, "<name>-requires-async")` -- follow that
existing pattern rather than inventing one.

Three outcomes, each distinguishable by the caller:

- **Reachable and readable** -- the proxy answered and the token worked.
- **Token unreadable** -- carry **the absolute path that was tried**. This is
  the task's whole point: it is the failure a contributor can fix, and today it
  is silently identical to "off". With environment variables unavailable to a
  GUI, it is also the *likely* failure rather than an exotic one.
- **Not reachable** -- carry the port that was tried.

**Never carry the token itself**, in any outcome.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn a_probe_against_a_dead_port_reports_unreachable_with_the_port() {
    // Bind and drop, so the port is real and definitely closed.
}

#[tokio::test]
async fn a_probe_that_cannot_read_the_token_names_the_path_it_tried() {
    // Assert an absolute path is present, not merely that it failed.
}

#[tokio::test]
async fn a_probe_never_returns_the_token() {
    // Serve a reachable proxy with a known token value; assert the serialized
    // response does not contain that value anywhere.
}

#[tokio::test]
async fn a_successful_probe_reports_reachable() {
    // Use the axum loopback-server pattern already in routing/ironwire.rs's
    // tests rather than inventing a mock.
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- make the token-unreadable outcome return a
  bare failure with no path and confirm the path test goes red. Then show the
  probe is unreachable from `TraceSource::load` by grepping the submission path.
- [ ] **Step 6: Commit**

---

### Task 3: Apply immediately, and report what is arriving

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs`

**Interfaces:**
- Produces: a hot-swappable `routing` field, and a `routing` block on `status_value()`

**Two changes that belong together**, because the status block is what makes the
hot swap observable.

**The hot swap.** `DaemonShared::routing` is `Option<Arc<IronWireLedger>>`
(`ipc.rs:396`), built once at load, so a declaration change takes effect only on
restart. Asking someone to restart because they typed a port is the friction
that makes a feature feel broken. Make the field an `RwLock<Option<Arc<..>>>`;
`source_roots_with_routing` and `refresh_routing` read through the lock; the
`set_settings` handler rebuilds it from the new declaration. Three test sites
assign the field directly and follow.

A rebuilt ledger starts **cold** -- empty until the next refresh. That is
correct, and it is exactly the "declared, nothing seen yet" state below.

**The status block.** `status_value()` (`ipc.rs:592`) is a `json!` block with
additive sub-objects; `health` and `daily_budget` are the precedent. Add
`routing` the same way, distinguishing three states, because collapsing them is
the defect this plan exists to fix:

- not declared
- declared, nothing seen yet
- declared, rows seen

`has_rows()` alone cannot express the first -- #513's own doc says the real
distinction is the `Option` on shared state. **`has_rows` is also a poor health
signal**: it says data exists, not that the proxy answers now. If a
last-successful-refresh timestamp can be added without touching the submission
path, do it; if not, report why and ship the three states.

`IPC_SCHEMA` is `"trace_commons.daemon.v1_1"` (`ipc.rs:132`). **Determine
whether an additive field needs a bump** by finding what the shells do with
unknown fields, and say what you found. Do not bump on a guess in either
direction.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_declaration_change_takes_effect_without_a_restart() {
    // The point of the hot swap. Set a declaration over set_settings and
    // assert source_roots_with_routing is now routed, with no reload.
}

#[test]
fn clearing_the_declaration_drops_the_ledger() {
    // null means off, and off must actually stop reading.
}

#[test]
fn a_rebuilt_ledger_reports_declared_but_nothing_seen() {
    // Cold start is not an error state and must not be reported as one.
}
```

Plus one test per status state, asserting the exact JSON shape. #513 landed
`daemon::ipc::tests::routing_transition`; find it and build on it rather than
duplicating it.

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- collapse "declared, nothing seen" into "not
  declared" and confirm a test goes red; then make `set_settings` skip the
  rebuild and confirm the no-restart test goes red.
- [ ] **Step 6: Commit**

---

### Task 4: The GTK surface

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/settings.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs`

**Interfaces:**
- Consumes: `probe_routing` (Task 2), the status `routing` block (Task 3)

GTK first because it is the only shell with a centralised copy module
(`src/copy.rs`, 2,594 lines) -- wording settled here is what Windows and macOS
copy later.

**One tool, one word.** Each tool reads **Private**, **Not private** or **Not
used**. Not "destination", not "backend", not "route", not "proxy" -- someone
with NEAR AI alone needs exactly one concept. The technical vocabulary in the
settings file is ours; none of it belongs on screen.

Four surfaces:

**The declaration.** A toggle, a port field enabled only when on, and an
optional token-directory field. Default the port to IronWire's conventional
value so nobody must know it, but **write nothing until the contributor acts** --
`None` means off, and a displayed default must never become a declaration. Send
through `set_settings` as `ui/settings.rs:932` already does; note that
`set_settings` rejects unknown keys, so the key must match exactly.

**The probe result**, rendered on save. Three outcomes, three strings. The
token-unreadable string must include the path the daemon reported -- do not
write a generic "check your configuration". Because environment variables never
reach a GUI daemon, this is the failure a real contributor will actually hit.

**The status line**, from Task 3's three states. "Declared, nothing seen yet" is
not an error and must not read as one.

**No restart notice.** Task 3 removed the need for one. If you find yourself
writing "takes effect after restarting", something in Task 3 did not land.

- [ ] **Step 1: Write the failing tests** -- GTK code here is testable at the
  copy and state-mapping level rather than by driving widgets. Test that each
  daemon state maps to the intended copy key, including that the
  token-unreadable case propagates the path. Follow whatever pattern
  `ui/settings.rs` tests already use; **do not invent a widget-driving harness.**
- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- map "declared, nothing seen yet" to the
  error copy and confirm a test goes red. Then grep every new string for
  "restart", "spent", "cost", "route", "backend", "proxy", "corpus", "share",
  "earn" and "credit"; there must be none.
- [ ] **Step 6: Commit**

---

## Not in this plan

- **The unlock, ownership and any earnings figure.** They need attestation
  receipts on the contributor's machine, and a scoring answer we do not have:
  credit is scored server-side after submission, so a pre-share figure may not
  be honestly showable at all. Separate plan, after that is settled.
- **`PreviewSummary`'s routing block.** The preview sheet is a
  contributor-facing surface; someone with no invite never reaches it.
- **Windows and macOS.** Their own plan, after wording settles here. macOS is
  the expensive one: it never calls `set_settings` at runtime, so Task 3's hot
  swap needs a runtime settings path that shell does not have. The FFI already
  exposes the generic method, so it is a shell change and not an ABI change --
  but budget it, do not discover it.
- **Consent-flag wording.** Pinned by
  `crates/trace-commons-protocol/tests/consent_policy_pin.rs` against the
  published page at `https://tracecommons.ai/legal/`, which lives in the
  trace-commons-community repo. #513 added `routing_metadata_included` and its
  pin entry without bumping `TRACE_CONTRIBUTION_POLICY_VERSION`. Whether a new
  content category may ship under an unchanged policy version is a decision for
  whoever owns that page.
- **Verifying the join key.** #513's end-to-end test uses fixtures on both
  sides, and IronWire's `client_session_id` is unmerged upstream. Until a real
  capture confirms their id equals our `conversation_id`, this UI can be
  entirely correct and still report zero forever. **That is an argument for
  doing the capture, not for delaying this plan** -- the privacy claim is true
  either way, and the status line is what would make a zero legible.
