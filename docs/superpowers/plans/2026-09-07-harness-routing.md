# Per-Harness Routing in the Model Calls Destination — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the Model calls destination lead with the harnesses on this
machine — each one showing whether its config sends calls here and whether a
call has actually been answered — and let a contributor connect or disconnect
one at a time, seeing the exact file change before it is written. The listener
switch stops being the first question and becomes a kill switch.

**Architecture:** The mechanism already exists upstream. `ironwire_agents` (a
crate this workspace's lock already carries, transitively through
`ironwire_proxy`) answers "which tools does this machine have, are they pointed
at us, and what would it take to change that", and carries the three rules that
make editing somebody else's config acceptable. This plan does **not**
reimplement any of it. It adds:

1. wording, in `private_inference_copy.rs`, for a surface that may not say
   `route`;
2. a daemon module, `daemon/harness.rs`, that calls `ironwire_agents` in
   process, joins each tool to the hosted listener's ledger to answer "has a
   call actually arrived", and exposes three IPC methods;
3. two branch tables across the C ABI (`harness_state_line` / `_state_tone`),
   plus an assembled last-seen line and the first-connect gate predicate;
4. the list, the preview and the confirm in each of the three shells.

The daemon does the plan and the commit itself rather than calling IronWire's
`POST /_ironwire/tools`, because that endpoint plans **and commits in one
request** and hands the changes back afterwards. The whole point of this slice
is that the contributor sees the change before it is written, so the in-process
`plan_connect` → show → `commit` sequence is the only one that satisfies it.
See "Corrections to the spec" below.

**Tech Stack:** Rust (`trace-commons-contributor`, `-contributor-ffi`,
`-contributor-gtk`), Swift/SwiftUI (macOS), C#/WinUI (Windows),
GTK4/libadwaita (Linux).

**Spec:** `docs/superpowers/specs/2026-09-07-harness-routing-design.md`

**Predecessor plan:** `docs/superpowers/plans/2026-09-07-private-inference-top-level.md`
— its "Traps found while implementing Tasks 1-2" are carried forward into the
Global Constraints below rather than left to be rediscovered.

---

## Corrections to the spec, verified against the pinned revision

The spec was written from a partial read. Four of its claims are wrong or
incomplete, and each changes what gets built. All four were checked against
`~/.cargo/git/checkouts/ironwire-35fda924badcb7e8/90c9ff9/`, whose
`crates/ironwire_agents` is byte-identical (`diff -r`) to the `ed53375` checkout
the spec quotes, and which is the revision pinned in
`crates/trace-commons-contributor/Cargo.toml`
(`rev = "90c9ff946ee424977f7a7d8a97440264559fddd4"`).

1. **"the control API is only `/_ironwire/health` and `/_ironwire/status`" — no.**
   `ironwire_proxy/src/control.rs:494-511` routes `/status`, `/backends`, `/pin`,
   `/settings`, `/privacy`, `/consent`, `/tools`, `/probe`, `/log`, `/events`,
   `/health`. `POST /_ironwire/tools` takes `{id, connect}` and calls
   `plan_connect`/`plan_disconnect` **followed immediately by `commit`**,
   returning `changes`, `occupied` and `backup` after the fact. This daemon
   already reads `GET /_ironwire/settings` for a tool list, in
   `handle_probe_routed_tools` (`daemon/ipc.rs:4335`). The spec's conclusion still
   holds — there is no per-tool pass-through, and connect/disconnect are config
   writes — but the reason to work in process is *preview*, not absence.

2. **Attribution comes from `facade`, not `path`.** `RoutedExchange.facade`
   (`crates/trace-commons-contributor/src/routing/mod.rs:56`) is the string the
   proxy stamps at `facade/anthropic.rs:180` and `facade/openai.rs:235`: exactly
   `"anthropic"` or `"openai"`. It is the protocol family directly, with no
   parsing. Use it.

3. **A catalog tool's family IS knowable.** `AgentEntry.settings` is a
   `Vec<AgentSetting>`, and `AgentSetting` carries `facade: Facade`
   (`ironwire_catalog/src/schema.rs:216-221`, `Facade` at `:178`). So every
   tool — built-in or catalog — has a declarable family. The ambiguity the spec
   describes is real but narrower than stated: it is *two connected tools of the
   same family*, never *an unknown family*.

4. **`commit` writes a backup, once.** `tools::commit` writes
   `<config>.<ext>.ironwire-backup` on the first write and **never overwrites
   it**, so the preserved copy is always the file as it was before IronWire ever
   touched it. It returns `Ok(None)` when nothing was backed up. The surface may
   name that file; it must not promise a fresh backup per change.

One more thing the plan assumes and the spec does not say:
`ironwire_catalog::CATALOG_PUBLIC_KEY` is `[0u8; 32]`, a deliberate placeholder,
so **every** signed catalog fails verification today and `CatalogStore::load`
degrades to the built-ins. On a real machine right now the list is exactly
Claude Code and Codex. The copy that says what the list is
(`HARNESS_LIST_SCOPE`) is therefore load-bearing, not decoration.

---

## Global Constraints

- **Every user-facing string comes from
  `crates/trace-commons-contributor/src/private_inference_copy.rs`,** between its
  `PRIVATE-INFERENCE-SURFACE-BEGIN` / `-END` markers. The sweep
  `the_offer_surface_says_nothing_it_should_not` reads that module's own source
  and fails on `ironwire`, `iron wire`, `proxy`, `backend`, `route`, `endpoint`,
  `localhost`, `private`, `secure`, `encrypt`, `anonym`, `protect`, `credit`,
  `earn`. **`route` is banned**, so nothing on this surface may say "routes
  through", "routing", "re-route" or "the route". Say a tool **sends its calls
  here** and this computer **answers** them. The sweep also asserts the constants
  did not move out from between the markers, so relocating a string to dodge it
  is itself a failure. It splits on `"`, so it sees string literals and not doc
  comments — a banned word in a `///` line passes the sweep and is still wrong.

- **Tool names are IronWire's, not ours.** "Claude Code" and "Codex" come from
  `Tool.name` at runtime. No shell and no copy constant spells a tool name. The
  empty-list sentence in particular must describe what was looked for without
  naming the tools, or it goes stale the day the catalog grows.

- **Adding a copy field means changing FOUR places together**, or something
  fails — three loudly and one silently:
  1. Rust: the `PrivateInferenceCopy` struct
     (`private_inference_copy.rs:354`), the `private_inference_copy()`
     constructor (`:406`), and the pinned field count in
     `every_sentence_arrives_finished` (`:722`) — **27 when this plan was
     written; 45 as shipped.** Read the assertion rather than trusting this
     number: it moved four times during implementation.
  2. Swift: the `PrivateInferenceCopy` struct and its `CodingKeys`
     (`macos/Sources/TCShellCore/PrivateInferenceSurface.swift:9`, `:44`), and
     the all-or-nothing sentinel payload in
     `macos/Tests/TCShellCoreTests/PrivateInferenceSurfaceTests.swift` — decoding
     is all-or-nothing, so a missing key fails roughly eleven tests at once.
  3. C#: the `PrivateInferenceCopy` record
     (`windows/src/TraceCommons.Interop/PrivateInferenceCopy.cs`) **and** its
     `Sentences` array (`:127`). `EveryExportedFieldIsDecodedAndNoneIsInvented`
     (`windows/tests/TraceCommons.Interop.Tests/PrivateInferenceTests.cs:52`)
     asserts set equality in **both** directions; Windows goes red the moment
     Rust grows a key it has not got.
  4. `docs/contributor-daemon-ipc-v1_1.md:1671` enumerates the payload fields and
     states the count. **Not test-enforced** — it goes stale in silence.

- **A sentence that interpolates does NOT go in the payload.** `serving_line` is
  the precedent: assembled on the Rust side, exported as its own ABI call, and
  pushed into the sweep's string list by hand. `harness_last_seen_line` follows
  it exactly, and does not touch the field count.

- **`ShellWordingTests.swift` and `ShellWordingTests.cs` hold a per-file
  authored-sentence count that is a CEILING AND A FLOOR both.** A new view file
  must author **zero** sentences — take every word from the copy payload — and
  must not be added to the baseline at all. Removing wording requires lowering
  the recorded number in the same commit. Numbers are **measured** with
  `TC_WORDING_DUMP=1`, never typed. Note that
  `TraceCommonsApp/Views/PrivateInferenceView.swift` is absent from the baseline
  today; that is the state to preserve.

- **The C ABI header exists in TWO copies and this slice adds functions, so both
  change together:**
  - `crates/trace-commons-contributor-ffi/include/trace_commons.h`
  - `macos/Sources/CTraceCommons/include/trace_commons.h`

  Two tests enforce it. `tests/header.rs::both_header_copies_declare_the_same_abi`
  compares declarations with comments stripped and whitespace normalised — the
  copies carry different prose deliberately, so this is not a byte-for-byte file
  comparison, and prose that contradicts the ABI is exactly the failure it cannot
  catch. `tests/abi_header_surface.rs` parses `src/lib.rs` and requires **both**
  headers to declare that set with those signatures, panicking on anything it
  cannot account for.

- **Only the `Clear` tone may be painted as working.** Indicators derive from the
  tone (`PrivateInferenceTone::reads_as_working`, `readsAsWorking`), never from a
  settings boolean and never from a `wired` flag. `wired` proves a file has a
  value in it; it proves nothing about a call.

- **The three IronWire rules are the safety model and must not be routed
  around:** never rewrite a file we cannot parse; fill an empty slot but leave a
  full one alone (report it, never overwrite); remove only what we put there. No
  code in this slice may add a force, an overwrite, or a "take it over anyway"
  path.

- **One tool per action.** Nothing writes more than one tool's config in a single
  action — no "connect all".

- **Hash-only / label-only operational surfaces.** A config path is shown to the
  contributor in the UI; it does not go into a daemon log line or an audit row.
  Nothing logs a tool's config *contents*.

- **The GTK crate is a SEPARATE cargo workspace with its own lockfile.** A root
  `--workspace` check does not cover it, and neither does a root `cargo test`.

- **Verify with `RUSTFLAGS="-D warnings"`.** Plain `cargo check` does not apply
  it; CI does.

- **A new direct dependency needs explicit approval before it is added.**
  `ironwire_agents` is the only one here. It is already in `Cargo.lock` at the
  same pinned revision as `ironwire_proxy`, so promoting it to a direct
  dependency should add **zero** packages — Task 2 measures that rather than
  asserting it.

---

### Task 1 (IN FLIGHT — verify against what landed): Harness wording

> **Another agent is implementing this concurrently.** Before writing anything,
> read `private_inference_copy.rs` on the branch and check which of these
> constants already exist. If they do, reconcile names and move on; do not
> implement it twice, and do not rename what landed just because this plan
> spells it differently.

Adds every sentence the harness surface prints, plus the two branch tables the
shells will reach across the ABI, plus one assembled line and one predicate.
Nothing here renders.

**Files:**
- Modify: `crates/trace-commons-contributor/src/private_inference_copy.rs`
- Modify: `macos/Sources/TCShellCore/PrivateInferenceSurface.swift`
- Modify: `macos/Tests/TCShellCoreTests/PrivateInferenceSurfaceTests.swift`
- Modify: `windows/src/TraceCommons.Interop/PrivateInferenceCopy.cs`
- Modify: `docs/contributor-daemon-ipc-v1_1.md`

**Interfaces:**
- Produces: `harness_state_line(label: &str) -> &'static str`
- Produces: `harness_state_tone(label: &str) -> PrivateInferenceTone`
- Produces: `harness_last_seen_line(when: &str) -> String`
- Produces: `harness_connect_needs_exposure(offer_answered: bool, listener_on: bool) -> bool`
- Produces: the label constants `HARNESS_NOT_INSTALLED`, `HARNESS_NOT_CONNECTED`,
  `HARNESS_CONNECTED_UNSEEN`, `HARNESS_ANSWERING`, `HARNESS_ANSWERING_SHARED`,
  `HARNESS_SLOT_TAKEN`, `HARNESS_CONFIG_UNREADABLE`
- Produces: new `PrivateInferenceCopy` fields. Planned as 27 → 44; shipped
  as 45, after review added a not-installed sentence and an outcome table

**Conflicts with:** Task 3 (both touch `PrivateInferenceSurface.swift` and
`PrivateInferenceCopy.cs`) and Task 7 (both touch
`docs/contributor-daemon-ipc-v1_1.md`). Land 1 before 3; land 7 last.

- [ ] **Step 1: Write the failing Rust tests**

In the test module of `private_inference_copy.rs`:

```rust
#[test]
fn every_harness_state_has_a_sentence_and_only_answering_reads_as_working() {
    for label in [
        HARNESS_NOT_INSTALLED,
        HARNESS_NOT_CONNECTED,
        HARNESS_CONNECTED_UNSEEN,
        HARNESS_ANSWERING,
        HARNESS_ANSWERING_SHARED,
        HARNESS_SLOT_TAKEN,
        HARNESS_CONFIG_UNREADABLE,
    ] {
        assert!(
            !harness_state_line(label).trim().is_empty(),
            "{label} has no sentence"
        );
    }
    assert!(harness_state_tone(HARNESS_ANSWERING).reads_as_working());
    for label in [
        HARNESS_NOT_INSTALLED,
        HARNESS_NOT_CONNECTED,
        HARNESS_CONNECTED_UNSEEN,
        HARNESS_ANSWERING_SHARED,
        HARNESS_SLOT_TAKEN,
        HARNESS_CONFIG_UNREADABLE,
        "",
        "a_state_from_a_later_daemon",
    ] {
        assert!(
            !harness_state_tone(label).reads_as_working(),
            "{label:?} must not be painted as working"
        );
    }
}

/// A config already sending calls here is not evidence that a call arrived,
/// and the two must not collapse into one sentence.
#[test]
fn connected_and_answering_are_different_sentences() {
    assert_ne!(
        harness_state_line(HARNESS_CONNECTED_UNSEEN),
        harness_state_line(HARNESS_ANSWERING)
    );
    assert_ne!(
        harness_state_line(HARNESS_ANSWERING),
        harness_state_line(HARNESS_ANSWERING_SHARED)
    );
}

/// Refused deliberately, and distinguishable from "nothing to change".
#[test]
fn an_unreadable_config_is_refused_rather_than_silent() {
    assert_eq!(
        harness_state_tone(HARNESS_CONFIG_UNREADABLE),
        PrivateInferenceTone::Refused
    );
    assert_ne!(
        harness_state_line(HARNESS_CONFIG_UNREADABLE),
        HARNESS_PREVIEW_NOTHING_TO_DO
    );
}

/// The exposure question is asked once, on the first connect, and never as a
/// standalone switch. Already on: nothing to ask. Already answered: nothing to
/// ask again.
#[test]
fn the_exposure_question_gates_only_the_first_connect() {
    assert!(harness_connect_needs_exposure(false, false));
    assert!(!harness_connect_needs_exposure(true, false));
    assert!(!harness_connect_needs_exposure(false, true));
    assert!(!harness_connect_needs_exposure(true, true));
}

#[test]
fn the_last_seen_sentence_names_a_time_or_says_nothing() {
    assert!(harness_last_seen_line("2 minutes ago").contains("2 minutes ago"));
    assert_eq!(harness_last_seen_line(""), "");
    assert_eq!(harness_last_seen_line("   "), "");
}
```

- [ ] **Step 2: Run them and watch them fail**

Run: `cargo test -p trace-commons-contributor private_inference_copy`
Expected: FAIL — `cannot find value HARNESS_NOT_INSTALLED in this scope`.

- [ ] **Step 3: Add the labels, the sentences and the constants**

All of it **between the markers**. Wire-side labels first, beside the existing
`LABEL_*` re-exports:

```rust
/// The tool is not on this machine, so nothing can be set up for it.
pub const HARNESS_NOT_INSTALLED: &str = "not_installed";
/// Its settings file does not send calls here.
pub const HARNESS_NOT_CONNECTED: &str = "not_connected";
/// Its settings file sends calls here, and no call has arrived yet.
///
/// One label rather than two, deliberately. "Needs restarting" and "connected
/// but nothing seen" are the same observation from this side: we cannot see
/// whether the tool is running, only that its file is right and nothing has
/// come in. A second label would be a claim about a process we never looked at.
pub const HARNESS_CONNECTED_UNSEEN: &str = "connected_unseen";
/// A call of this tool's kind arrived, and it is the only connected tool that
/// makes that kind.
pub const HARNESS_ANSWERING: &str = "answering";
/// A call of this tool's kind arrived, and more than one connected tool makes
/// that kind, so it cannot be said which one sent it.
pub const HARNESS_ANSWERING_SHARED: &str = "answering_shared";
/// A setting it would need already has a value the contributor put there. It
/// was left exactly as it is.
pub const HARNESS_SLOT_TAKEN: &str = "slot_taken";
/// Its settings file could not be read as the format it claims to be, so
/// nothing was changed.
pub const HARNESS_CONFIG_UNREADABLE: &str = "config_unreadable";
```

Then the sentences. Every one avoids `route`, `proxy`, `endpoint`, `localhost`
and the rest of the swept list:

```rust
const HARNESS_SECTION_TITLE: &str = "Tools on this computer";
/// What the list is, said out loud. A short list that explains nothing is
/// indistinguishable from a broken one.
const HARNESS_LIST_SCOPE: &str =
    "These are the tools this app knows how to set up. Others may be installed \
     and are not shown.";
const HARNESS_NONE_FOUND: &str =
    "None of the tools this app knows how to set up was found here. It looks for \
     each one's own settings file, and for its name among the programs installed \
     on this computer.";
const HARNESS_STATE_NOT_INSTALLED: &str = "Not found on this computer.";
const HARNESS_STATE_NOT_CONNECTED: &str = "Its settings send its calls somewhere else.";
const HARNESS_STATE_CONNECTED_UNSEEN: &str =
    "Its settings send its calls here. Nothing has arrived yet — if it was \
     already running, start it again.";
const HARNESS_STATE_ANSWERING: &str = "Answering its calls.";
const HARNESS_STATE_ANSWERING_SHARED: &str =
    "Calls of this kind are being answered. More than one tool here makes that \
     kind, so this cannot tell which one sent them.";
const HARNESS_STATE_SLOT_TAKEN: &str =
    "One of its settings already has a value you put there. It was left as it is.";
const HARNESS_STATE_CONFIG_UNREADABLE: &str =
    "Its settings file could not be read, so nothing was changed.";
const HARNESS_STATE_UNREPORTED: &str = "Nothing has been reported about this one.";
const HARNESS_CONNECT_ACTION: &str = "Send its calls here";
const HARNESS_DISCONNECT_ACTION: &str = "Stop sending its calls here";
const HARNESS_CONFIG_PATH_LABEL: &str = "Its settings file";
const HARNESS_COMMAND_LABEL: &str = "Or do it yourself, with this";
const HARNESS_PREVIEW_TITLE: &str = "The change to its settings file";
const HARNESS_PREVIEW_CONFIRM: &str = "Make this change";
const HARNESS_PREVIEW_CANCEL: &str = "Leave it alone";
pub const HARNESS_PREVIEW_NOTHING_TO_DO: &str =
    "Its settings already say this. There is nothing to change.";
const HARNESS_PREVIEW_OCCUPIED: &str =
    "This setting already has a value you put there, so it was left alone:";
const HARNESS_PREVIEW_BACKUP: &str =
    "A copy of the file as it was before this app first changed it is kept beside it.";
const HARNESS_PLAN_STALE: &str =
    "The file changed while you were looking at this. Nothing was written — \
     open it again.";
/// The caveat that keeps the activity column honest.
const HARNESS_ATTRIBUTION_CAVEAT: &str =
    "What arrives says which kind of call it is, not which tool sent it.";
const HARNESS_FIRST_CONNECT: &str =
    "Before any tool can send its calls here, this computer has to start \
     answering them.";
```

The branch tables, next to `state_line` / `state_tone`, and taking the same
input so the sentence and the colour cannot drift:

```rust
/// The sentence for one harness state label.
///
/// ONE BRANCH TABLE, NOT FOUR — the rule `state_line` states. An unfamiliar
/// label is unreported, never "not connected": a state this build has not heard
/// of is not evidence that a file says nothing.
#[must_use]
pub fn harness_state_line(label: &str) -> &'static str {
    match label {
        HARNESS_NOT_INSTALLED => HARNESS_STATE_NOT_INSTALLED,
        HARNESS_NOT_CONNECTED => HARNESS_STATE_NOT_CONNECTED,
        HARNESS_CONNECTED_UNSEEN => HARNESS_STATE_CONNECTED_UNSEEN,
        HARNESS_ANSWERING => HARNESS_STATE_ANSWERING,
        HARNESS_ANSWERING_SHARED => HARNESS_STATE_ANSWERING_SHARED,
        HARNESS_SLOT_TAKEN => HARNESS_STATE_SLOT_TAKEN,
        HARNESS_CONFIG_UNREADABLE => HARNESS_STATE_CONFIG_UNREADABLE,
        _ => HARNESS_STATE_UNREPORTED,
    }
}

/// The tone [`harness_state_line`]'s sentence is painted in.
///
/// [`PrivateInferenceTone::Clear`] for [`HARNESS_ANSWERING`] alone.
/// [`HARNESS_ANSWERING_SHARED`] is `Held` and not `Clear`: calls of that kind
/// are arriving, and painting THIS row as working would attribute them to a
/// tool nothing identified.
#[must_use]
pub fn harness_state_tone(label: &str) -> PrivateInferenceTone {
    match label {
        HARNESS_ANSWERING => PrivateInferenceTone::Clear,
        HARNESS_CONNECTED_UNSEEN | HARNESS_ANSWERING_SHARED => PrivateInferenceTone::Held,
        HARNESS_SLOT_TAKEN => PrivateInferenceTone::Attention,
        HARNESS_CONFIG_UNREADABLE => PrivateInferenceTone::Refused,
        _ => PrivateInferenceTone::Neutral,
    }
}

/// When a call of this tool's kind was last answered, as a finished sentence.
///
/// Finished on this side from a rendered time, the way [`serving_line`] is
/// finished from a port. Empty for an empty input, and a shell draws nothing at
/// all for an empty string rather than a blank row.
#[must_use]
pub fn harness_last_seen_line(when: &str) -> String {
    if when.trim().is_empty() {
        return String::new();
    }
    format!("Last one answered {when}")
}

/// Whether connecting a tool has to ask the exposure question first.
///
/// The listener is open to everything on this machine, not only to the tool
/// being connected, and that does not follow from "send this tool's calls
/// here". So it is asked as a gate on the FIRST connect, where it is finally
/// about something concrete -- and never again, and never as a switch flipped
/// into a void.
#[must_use]
pub fn harness_connect_needs_exposure(offer_answered: bool, listener_on: bool) -> bool {
    !offer_answered && !listener_on
}
```

Add the 17 payload fields to `PrivateInferenceCopy` and to
`private_inference_copy()`: `harness_section_title`, `harness_list_scope`,
`harness_none_found`, `harness_connect_action`, `harness_disconnect_action`,
`harness_config_path_label`, `harness_command_label`, `harness_preview_title`,
`harness_preview_confirm`, `harness_preview_cancel`,
`harness_preview_nothing_to_do`, `harness_preview_occupied`,
`harness_preview_backup`, `harness_plan_stale`, `harness_attribution_caveat`,
`harness_first_connect`, `harness_state_unreported`. The seven state sentences
reach shells through `harness_state_line`, matching how the existing state
sentences work.

- [ ] **Step 4: Raise the pinned field count and extend the sweep**

In `every_sentence_arrives_finished`, `27` becomes `44` (45 as shipped). In
`the_offer_surface_says_nothing_it_should_not`, push the new assembled sentences
and every harness label through the table, so a banned word in one of them
cannot hide behind a function:

```rust
        strings.push(harness_last_seen_line("2 minutes ago"));
        for label in [
            HARNESS_NOT_INSTALLED,
            HARNESS_NOT_CONNECTED,
            HARNESS_CONNECTED_UNSEEN,
            HARNESS_ANSWERING,
            HARNESS_ANSWERING_SHARED,
            HARNESS_SLOT_TAKEN,
            HARNESS_CONFIG_UNREADABLE,
            "a_state_from_a_later_daemon",
        ] {
            strings.push(harness_state_line(label).to_string());
        }
```

- [ ] **Step 5: Run the Rust tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor private_inference`
Expected: PASS, including the sweep. If the sweep fails on `route`, the sentence
is wrong, not the sweep.

- [ ] **Step 6: Mirror the payload on both far sides**

Swift: 17 `public let` properties on `PrivateInferenceCopy` and 17 `CodingKeys`
cases, plus every new key in the sentinel payload in
`PrivateInferenceSurfaceTests.swift` — decoding is all-or-nothing and roughly
eleven tests fail together if one is missing.

C#: 17 `[JsonPropertyName("...")] public string ... { get; init; } = string.Empty;`
properties on the record **and** 17 entries appended to `Sentences`.
`EveryExportedFieldIsDecodedAndNoneIsInvented` compares the two sets in both
directions; `EverySentenceArrivesFinished` walks `Sentences`.

- [ ] **Step 7: Update the field inventory in the contract document**

`docs/contributor-daemon-ipc-v1_1.md:1671` says "these 27 fixed string fields"
and lists them. Change the count to 44 and add the seventeen names to the list.
Nothing tests this; it goes stale in silence if skipped.

- [ ] **Step 8: Run all three sides**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor private_inference
cd macos && swift test --filter PrivateInference
```
And the Windows interop tests (CI's `windows contributor crate tests` job).
Expected: PASS on all three.

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-contributor/src/private_inference_copy.rs \
        macos/Sources/TCShellCore/PrivateInferenceSurface.swift \
        macos/Tests/TCShellCoreTests/PrivateInferenceSurfaceTests.swift \
        windows/src/TraceCommons.Interop/PrivateInferenceCopy.cs \
        docs/contributor-daemon-ipc-v1_1.md
git commit -m "Give the harness list its words"
```

---

### Task 2 (IN FLIGHT — verify against what landed): The daemon's harness module and its three IPC methods

> **Another agent is implementing this concurrently.** Read
> `crates/trace-commons-contributor/src/daemon/` and `METHODS` on the branch
> first. If `harness.rs` exists, reconcile against it rather than rewriting it.

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/harness.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (module list)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (`METHODS`,
  dispatch, three handlers, three refusal labels)
- Modify: `crates/trace-commons-contributor/Cargo.toml` (`ironwire_agents`)

**Interfaces:**
- Consumes: `ironwire_agents::tools::{all, plan_connect, plan_disconnect, commit, Tool, Planned, Error}`;
  `ironwire_catalog::{CATALOG_PUBLIC_KEY, CatalogStore, schema::{Catalog, Facade}}`;
  `super::private_inference::{ironwire_home, OwnedEndpoint, effective_metadata_declaration}`;
  `super::settings::ironwire_ledger_for`; Task 1's `HARNESS_*` labels.
- Produces:
  - `pub struct FacadeSighting { pub facade: String, pub at: DateTime<Utc> }`
  - `pub struct HarnessRow { id, name, config_path, installed, wired, connect_command, facades, state, last_seen }`
  - `pub fn catalog() -> Catalog`
  - `pub fn facades_for(tool_id: &str, catalog: &Catalog) -> Vec<&'static str>`
  - `pub fn state_for(installed: bool, wired: bool, occupied: bool, unreadable: bool, sighting: Option<DateTime<Utc>>, family_is_shared: bool) -> &'static str`
  - `pub fn rows(catalog: &Catalog, sightings: &[FacadeSighting]) -> Vec<HarnessRow>`
  - `pub fn plan_digest(planned: &Planned) -> String`
  - IPC methods `harnesses`, `plan_harness`, `commit_harness`.

**Conflicts with:** Task 7 (`docs/contributor-daemon-ipc-v1_1.md`). Task 3's ABI
tests call into this crate, so land 2 before 3.

- [ ] **Step 1: Get the dependency approved, then measure it**

`ironwire_agents` is not a direct dependency today. It IS in `Cargo.lock` at
`rev = 90c9ff946ee424977f7a7d8a97440264559fddd4`, pulled in by `ironwire_proxy`,
so promoting it should add nothing. **Surface it for explicit approval before
adding it**, with the measurement, not after:

```bash
cargo tree -p trace-commons-contributor -e normal --prefix none \
  | awk '{print $1}' | sort -u | wc -l    # before
```
Then add, pinned to the identical revision, with the reason in a comment beside
the `ironwire_proxy` entry:

```toml
# The tool catalogue and the polite config edits behind the harness list.
# Already in the lock through ironwire_proxy at this exact revision, so this
# promotes a transitive dependency to a direct one and resolves no new package
# (measured: <N> deduplicated normal-graph names before and after).
#
# Pinned to the same revision as ironwire_proxy deliberately. The two crates
# share ironwire_catalog types across this seam; two revisions would be two
# incompatible `Catalog`s that still compile.
ironwire_agents = { git = "https://github.com/nearai/ironwire", rev = "90c9ff946ee424977f7a7d8a97440264559fddd4" }
```
Re-run the count and put the real number in the comment. If it moved, stop and
report rather than proceeding.

- [ ] **Step 2: Write the failing state-table test**

New `crates/trace-commons-contributor/src/daemon/harness.rs`, test module:

```rust
#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::*;
    use crate::private_inference_copy::*;

    fn at(minute: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 7, 12, minute, 0)
            .single()
            .expect("a real time")
    }

    /// The three states the spec names, and the four that are not the happy path.
    #[test]
    fn the_state_table_never_claims_more_than_it_knows() {
        // Not on the machine at all.
        assert_eq!(state_for(false, false, false, false, None, false), HARNESS_NOT_INSTALLED);
        // Installed, its file points elsewhere.
        assert_eq!(state_for(true, false, false, false, None, false), HARNESS_NOT_CONNECTED);
        // Its file is right and nothing has come in. NOT "answering".
        assert_eq!(state_for(true, true, false, false, None, false), HARNESS_CONNECTED_UNSEEN);
        // A call of its kind arrived, and it is the only connected tool of that kind.
        assert_eq!(state_for(true, true, false, false, Some(at(3)), false), HARNESS_ANSWERING);
        // Same, but two connected tools share the kind: no attribution.
        assert_eq!(
            state_for(true, true, false, false, Some(at(3)), true),
            HARNESS_ANSWERING_SHARED
        );
        // A slot the contributor filled outranks everything below it.
        assert_eq!(state_for(true, false, true, false, None, false), HARNESS_SLOT_TAKEN);
        // An unreadable file outranks even that: we know nothing about the slots.
        assert_eq!(state_for(true, false, true, true, None, false), HARNESS_CONFIG_UNREADABLE);
    }

    /// Not installed cannot be connected, whatever else is true of it.
    #[test]
    fn a_tool_that_is_not_installed_is_never_reported_as_connected() {
        assert_eq!(
            state_for(false, true, false, false, Some(at(3)), false),
            HARNESS_NOT_INSTALLED
        );
    }

    /// With no catalog -- which is every machine today, because the catalog
    /// verifying key is a placeholder -- the list is the two built-ins.
    #[test]
    fn the_list_degrades_to_the_built_in_tools() {
        let rows = rows(&ironwire_catalog::schema::Catalog::default(), &[]);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["claude", "codex"]);
    }

    /// Attribution is by protocol family, and the family of each built-in is
    /// fixed. A catalog tool declares its own, on every setting it writes.
    #[test]
    fn each_built_in_tool_declares_one_family() {
        let catalog = ironwire_catalog::schema::Catalog::default();
        assert_eq!(facades_for("claude", &catalog), vec!["anthropic"]);
        assert_eq!(facades_for("codex", &catalog), vec!["openai"]);
        assert!(facades_for("something-nobody-shipped", &catalog).is_empty());
    }
}
```

- [ ] **Step 3: Run it and watch it fail**

Run: `cargo test -p trace-commons-contributor daemon::harness`
Expected: FAIL — `unresolved module daemon::harness`.

- [ ] **Step 4: Write the module**

```rust
//! Which tools on this machine send their calls here, and whether one ever did.
//!
//! Everything about locating a tool's config, working out what to change in it
//! and changing it politely is `ironwire_agents`'. Three rules come with it and
//! are the reason editing a file we do not own is acceptable at all: never
//! rewrite a file we cannot parse, fill an empty slot but leave a full one
//! alone, and remove only what we put there. Nothing here works around any of
//! them, and there is deliberately no force path.
//!
//! # Why the plan and the commit happen here and not over the wire
//!
//! IronWire's control API does have `POST /_ironwire/tools`, and it plans and
//! commits in one request, handing the changes back afterwards. This surface
//! exists so the contributor sees the change BEFORE it is written, so the
//! in-process sequence -- plan, show, commit the same plan -- is the only one
//! that satisfies it.
//!
//! # `wired` is not `answering`
//!
//! `Tool.wired` says a config file has the right value in it. It says nothing
//! about a call. The evidence that a call was answered is a row in the hosted
//! listener's own ledger, and that row names a protocol family, never a tool.
//! So attribution is exact only while the connected tools of a family number
//! one, and [`state_for`] answers `HARNESS_ANSWERING_SHARED` rather than
//! guessing when they do not.

use chrono::{DateTime, Utc};
use ironwire_agents::tools::{self, Planned, Tool};
use ironwire_catalog::schema::{Catalog, Facade};

use crate::private_inference_copy::{
    HARNESS_ANSWERING, HARNESS_ANSWERING_SHARED, HARNESS_CONFIG_UNREADABLE,
    HARNESS_CONNECTED_UNSEEN, HARNESS_NOT_CONNECTED, HARNESS_NOT_INSTALLED, HARNESS_SLOT_TAKEN,
};

/// One call of one protocol family, as the ledger recorded it.
///
/// A two-field projection of `RoutedExchange` rather than the row itself: this
/// module needs a family and a time, and taking the whole row would make every
/// test here construct twenty fields it does not read.
#[derive(Debug, Clone)]
pub struct FacadeSighting {
    /// `"anthropic"` or `"openai"`, exactly as the proxy stamped it.
    pub facade: String,
    pub at: DateTime<Utc>,
}

/// One tool, as a screen needs to describe it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HarnessRow {
    pub id: String,
    /// IronWire's name for it. Never spelled in our copy or in a shell.
    pub name: String,
    /// Shown always, not on demand: a tool nobody expected to be set up is a
    /// question about WHICH FILE, every time.
    pub config_path: Option<String>,
    pub installed: bool,
    pub wired: bool,
    pub connect_command: String,
    pub facades: Vec<&'static str>,
    /// One of the `HARNESS_*` labels. A shell renders it through the shared
    /// branch table and never matches on it to pick a colour.
    pub state: String,
    pub last_seen: Option<DateTime<Utc>>,
}

/// The catalog in force, which on every machine today is the built-ins.
///
/// `ironwire_catalog::CATALOG_PUBLIC_KEY` is a deliberate `[0u8; 32]`
/// placeholder until release signing exists, so every signed document fails
/// verification and `load` degrades to the compiled-in defaults. Reading the
/// file anyway is not pointless: the day that key becomes real, this reads the
/// same cache the hosted listener does, from the same home, with no change here.
#[must_use]
pub fn catalog() -> Catalog {
    let Some(home) = super::private_inference::ironwire_home() else {
        return Catalog::default();
    };
    ironwire_catalog::CatalogStore::load(
        ironwire_catalog::CATALOG_PUBLIC_KEY,
        &home.join("catalog.json"),
    )
    .current()
    .clone()
}

/// Which protocol families a tool's calls arrive on.
///
/// The built-ins are fixed because their setup is more than one key and lives
/// in IronWire's code rather than in a document. A catalog tool states its own:
/// every `AgentSetting` carries a `facade`.
#[must_use]
pub fn facades_for(tool_id: &str, catalog: &Catalog) -> Vec<&'static str> {
    fn name(facade: Facade) -> &'static str {
        match facade {
            Facade::Anthropic => "anthropic",
            Facade::OpenAi => "openai",
        }
    }
    match tool_id {
        "claude" => vec![name(Facade::Anthropic)],
        "codex" => vec![name(Facade::OpenAi)],
        other => {
            let Some(agent) = catalog.agents().into_iter().find(|a| a.id == other) else {
                return Vec::new();
            };
            let mut facades: Vec<&'static str> =
                agent.settings.iter().map(|s| name(s.facade)).collect();
            facades.sort_unstable();
            facades.dedup();
            facades
        }
    }
}

/// The one label for one tool, from everything known about it.
///
/// Ordered most-certain first. An unreadable file wins over an occupied slot
/// because a file we cannot parse tells us nothing about its slots; an occupied
/// slot wins over `not_connected` because "we left your value alone" is the
/// answer, not "it is not set up".
#[must_use]
pub fn state_for(
    installed: bool,
    wired: bool,
    occupied: bool,
    unreadable: bool,
    sighting: Option<DateTime<Utc>>,
    family_is_shared: bool,
) -> &'static str {
    if !installed {
        return HARNESS_NOT_INSTALLED;
    }
    if unreadable {
        return HARNESS_CONFIG_UNREADABLE;
    }
    if occupied {
        return HARNESS_SLOT_TAKEN;
    }
    if !wired {
        return HARNESS_NOT_CONNECTED;
    }
    match sighting {
        Some(_) if family_is_shared => HARNESS_ANSWERING_SHARED,
        Some(_) => HARNESS_ANSWERING,
        None => HARNESS_CONNECTED_UNSEEN,
    }
}

/// Every tool, with what is true of it right now.
///
/// `installed` and `wired` are read from the filesystem on every call and never
/// cached: a tool that rewrites its own config underneath us must make this
/// surface correct itself rather than keep an old answer.
#[must_use]
pub fn rows(catalog: &Catalog, sightings: &[FacadeSighting]) -> Vec<HarnessRow> {
    let tools = tools::all(catalog);

    // How many CONNECTED tools claim each family. One is attributable; more
    // than one is not, and the row says so instead of picking.
    let mut connected_per_facade: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for tool in &tools {
        if tool.wired {
            for facade in facades_for(&tool.id, catalog) {
                *connected_per_facade.entry(facade).or_default() += 1;
            }
        }
    }

    tools
        .into_iter()
        .map(|tool: Tool| {
            let facades = facades_for(&tool.id, catalog);
            let last_seen = sightings
                .iter()
                .filter(|s| facades.iter().any(|f| *f == s.facade))
                .map(|s| s.at)
                .max();
            let shared = facades
                .iter()
                .any(|f| connected_per_facade.get(f).copied().unwrap_or(0) > 1);
            // A plan is worked out but never committed, purely to learn whether
            // the file parses and whether a slot is already spoken for. Nothing
            // is written; `commit` is the only thing that writes. The port here
            // is a placeholder: it appears only in a value we discard.
            let (occupied, unreadable) = match tools::plan_connect(&tool.id, 1, catalog) {
                Ok(planned) => (!planned.occupied.is_empty(), false),
                Err(tools::Error::Edit(_)) => (false, true),
                Err(_) => (false, false),
            };
            HarnessRow {
                state: state_for(
                    tool.installed,
                    tool.wired,
                    occupied,
                    unreadable,
                    last_seen,
                    shared,
                )
                .to_string(),
                config_path: tool.config_path.map(|p| p.display().to_string()),
                id: tool.id,
                name: tool.name,
                installed: tool.installed,
                wired: tool.wired,
                connect_command: tool.connect_command,
                facades,
                last_seen,
            }
        })
        .collect()
}

/// A fingerprint of exactly what was shown to the contributor.
///
/// `Planned` holds the file's before and after privately and cannot travel to a
/// shell and back, so the commit re-plans. Between the preview and the confirm
/// the tool may rewrite its own config -- which is the risk this whole surface
/// documents -- and committing a plan that no longer matches what was on screen
/// would write a change nobody agreed to. So the shell hands this back and a
/// mismatch is refused, never reconciled.
#[must_use]
pub fn plan_digest(planned: &Planned) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(planned.path.display().to_string().as_bytes());
    for change in &planned.changes {
        hasher.update([0u8]);
        hasher.update(change.as_bytes());
    }
    for (slot, current) in &planned.occupied {
        hasher.update([1u8]);
        hasher.update(slot.as_bytes());
        hasher.update([2u8]);
        hasher.update(current.as_bytes());
    }
    hex::encode(hasher.finalize())
}
```

Register it in `daemon/mod.rs` beside `private_inference`:

```rust
pub mod harness;
```

- [ ] **Step 5: Run the module tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor daemon::harness`
Expected: PASS.

- [ ] **Step 6: Write the failing IPC tests**

In `ipc.rs`'s test module, following the shape of the existing method tests
(`daemon/ipc.rs:8823` is the nearest one):

```rust
#[test]
fn the_harness_methods_are_advertised() {
    for method in ["harnesses", "plan_harness", "commit_harness"] {
        assert!(METHODS.contains(&method), "{method} is not in METHODS");
    }
}

/// A tool nothing has heard of is refused by label, never by message.
#[test]
fn a_tool_nothing_knows_cannot_be_planned() {
    let shared = test_shared();
    let response = handle_request(
        &shared,
        &Request {
            id: 1,
            method: "plan_harness".to_string(),
            params: serde_json::json!({ "id": "no-such-tool", "connect": true }),
        },
    );
    assert_eq!(
        response.error.as_ref().map(|e| e.message.as_str()),
        Some(ERR_HARNESS_UNKNOWN)
    );
}

/// A stale plan is refused rather than reconciled: the contributor agreed to
/// the change they were shown, not to whatever the file says now.
#[test]
fn a_commit_whose_plan_no_longer_matches_is_refused() {
    let shared = test_shared();
    let response = handle_request(
        &shared,
        &Request {
            id: 1,
            method: "commit_harness".to_string(),
            params: serde_json::json!({
                "id": "claude",
                "connect": false,
                "digest": "0000000000000000000000000000000000000000000000000000000000000000"
            }),
        },
    );
    assert_eq!(
        response.error.as_ref().map(|e| e.message.as_str()),
        Some(ERR_HARNESS_PLAN_STALE)
    );
}

/// Connecting needs a port, and there is one only while this computer is
/// answering calls. Disconnecting needs none and is allowed either way, which
/// is exactly when somebody wants it.
#[test]
fn connecting_with_no_listener_is_refused_and_disconnecting_is_not() {
    let shared = test_shared();
    let connect = handle_request(
        &shared,
        &Request {
            id: 1,
            method: "plan_harness".to_string(),
            params: serde_json::json!({ "id": "claude", "connect": true }),
        },
    );
    assert_eq!(
        connect.error.as_ref().map(|e| e.message.as_str()),
        Some(ERR_HARNESS_NOT_LISTENING)
    );
    let disconnect = handle_request(
        &shared,
        &Request {
            id: 2,
            method: "plan_harness".to_string(),
            params: serde_json::json!({ "id": "claude", "connect": false }),
        },
    );
    assert!(disconnect.error.is_none(), "{disconnect:?}");
}
```

Use whatever the file's existing test-fixture constructor is called instead of
`test_shared()`; do not add a second one.

- [ ] **Step 7: Add the methods**

Three names in `METHODS`, in alphabetical position: `"commit_harness"`,
`"harnesses"`, `"plan_harness"`. Five refusal labels beside the existing ones:

```rust
/// No tool by that id, or the tool's config could not be located.
pub const ERR_HARNESS_UNKNOWN: &str = "harness-unknown";
/// The file changed between the preview and the confirm. Nothing was written.
pub const ERR_HARNESS_PLAN_STALE: &str = "harness-plan-stale";
/// This computer is not answering calls, so there is no port to send them to.
pub const ERR_HARNESS_NOT_LISTENING: &str = "harness-not-listening";
/// The file could not be read as the format it claims to be. Deliberately
/// distinct from "nothing to change".
pub const ERR_HARNESS_UNREADABLE: &str = "harness-unreadable";
/// The change was worked out and the write failed. Never carries the error text.
pub const ERR_HARNESS_WRITE_FAILED: &str = "harness-write-failed";
```

Handlers, on the synchronous path — every one of these reads or writes local
files and opens no connection, which is exactly why `discover_routing` answers
synchronously:

```rust
/// Every tool, and what is true of it right now.
///
/// Read on every call, never cached, so a tool that rewrote its own config
/// under us corrects itself the next time the list is drawn.
fn handle_harnesses(shared: &DaemonShared, req: &Request) -> Response {
    let catalog = super::harness::catalog();
    let sightings = harness_sightings(shared);
    Response::ok(
        req.id,
        serde_json::json!({ "harnesses": super::harness::rows(&catalog, &sightings) }),
    )
}

/// What connecting or disconnecting one tool would change, without changing it.
fn handle_plan_harness(shared: &DaemonShared, req: &Request) -> Response {
    let Some(id) = req.params.get("id").and_then(serde_json::Value::as_str) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "harness-id-required");
    };
    let connect = req
        .params
        .get("connect")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let catalog = super::harness::catalog();
    let planned = match plan_one(shared, req, id, connect, &catalog) {
        Ok(planned) => planned,
        Err(response) => return *response,
    };
    Response::ok(
        req.id,
        serde_json::json!({
            "id": id,
            "connect": connect,
            "path": planned.path.display().to_string(),
            "changes": planned.changes,
            "occupied": planned
                .occupied
                .iter()
                .map(|(slot, current)| serde_json::json!({ "slot": slot, "current": current }))
                .collect::<Vec<_>>(),
            "noop": planned.is_noop(),
            "digest": super::harness::plan_digest(&planned),
        }),
    )
}

/// Make the change the contributor was shown, and only that one.
fn handle_commit_harness(shared: &DaemonShared, req: &Request) -> Response {
    let Some(id) = req.params.get("id").and_then(serde_json::Value::as_str) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "harness-id-required");
    };
    let Some(digest) = req.params.get("digest").and_then(serde_json::Value::as_str) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "harness-digest-required");
    };
    let connect = req
        .params
        .get("connect")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let catalog = super::harness::catalog();
    let planned = match plan_one(shared, req, id, connect, &catalog) {
        Ok(planned) => planned,
        Err(response) => return *response,
    };
    if super::harness::plan_digest(&planned) != digest {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_HARNESS_PLAN_STALE);
    }
    if planned.is_noop() {
        return Response::ok(req.id, serde_json::json!({ "id": id, "noop": true }));
    }
    // The path is shown on the contributor's own screen. It does not go in a
    // log line, and neither does anything the file contains.
    let backup = match ironwire_agents::tools::commit(&planned) {
        Ok(backup) => backup,
        Err(_) => return Response::err(req.id, ERR_UNAVAILABLE, ERR_HARNESS_WRITE_FAILED),
    };
    Response::ok(
        req.id,
        serde_json::json!({
            "id": id,
            "noop": false,
            "backup": backup.map(|path| path.display().to_string()),
        }),
    )
}

/// One plan, with the port this daemon's own listener is on.
///
/// Connect needs a port, and there is one only while this daemon is answering
/// calls. Disconnect needs none -- it removes what we put there -- so it is
/// allowed while the listener is off, which is exactly when somebody wants it.
///
/// The refusal is boxed for the reason `probe_credential`'s is: a `Response` is
/// large, and an unboxed one on the error arm makes this `Result` large too.
fn plan_one(
    shared: &DaemonShared,
    req: &Request,
    id: &str,
    connect: bool,
    catalog: &ironwire_catalog::schema::Catalog,
) -> Result<ironwire_agents::tools::Planned, Box<Response>> {
    use ironwire_agents::tools;
    let planned = if connect {
        let port = shared
            .private_inference_endpoint
            .lock()
            .expect("proxy endpoint lock")
            .as_ref()
            .map(|owned| owned.port);
        let Some(port) = port else {
            return Err(Box::new(Response::err(
                req.id,
                ERR_UNAVAILABLE,
                ERR_HARNESS_NOT_LISTENING,
            )));
        };
        tools::plan_connect(id, port, catalog)
    } else {
        tools::plan_disconnect(id, catalog)
    };
    planned.map_err(|error| {
        Box::new(match error {
            tools::Error::UnknownTool(_) | tools::Error::NoPath(_) => {
                Response::err(req.id, ERR_BAD_PARAMS, ERR_HARNESS_UNKNOWN)
            }
            // A file we cannot parse. Refused, by name, and distinguishable
            // from "nothing to change".
            tools::Error::Edit(_) => {
                Response::err(req.id, ERR_UNAVAILABLE, ERR_HARNESS_UNREADABLE)
            }
        })
    })
}
```

`handle_request`'s match gains:

```rust
        "harnesses" => handle_harnesses(shared, req),
        "plan_harness" => handle_plan_harness(shared, req),
        "commit_harness" => handle_commit_harness(shared, req),
```

`harness_sightings` reads the hosted listener's own ledger through the seam that
already exists: `private_inference::effective_metadata_declaration(declared,
enabled, owned)` synthesises a declaration from `OwnedEndpoint` when the switch
is on, and `settings::ironwire_ledger_for` turns that into a
`crate::routing::RoutingLedger`. Call `exchanges_since(Utc::now() -
Duration::hours(24))` and project each row to `FacadeSighting { facade:
row.facade, at: row.started_at }`. Carry nothing else off the row: this module
needs a family and a time, and a model name or a session id has no business on
this surface.

- [ ] **Step 8: Run**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor daemon::
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
```
Expected: PASS. `hello` reports `METHODS`, and a test checks that list against
the contract document — if it fails, Task 7's doc edit is a prerequisite, not
optional.

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-contributor/Cargo.toml Cargo.lock \
        crates/trace-commons-contributor/src/daemon/harness.rs \
        crates/trace-commons-contributor/src/daemon/mod.rs \
        crates/trace-commons-contributor/src/daemon/ipc.rs
git commit -m "Answer which tools send their calls here, and what changing that would take"
```

---

### Task 3 (IN FLIGHT — verify against what landed): The ABI exports and both headers

Four exports, so no shell recovers a sentence or a colour by matching on a label.

**Files:**
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs`
- Modify: `crates/trace-commons-contributor-ffi/include/trace_commons.h`
- Modify: `macos/Sources/CTraceCommons/include/trace_commons.h`
- Modify: `crates/trace-commons-contributor-ffi/tests/abi.rs`
- Modify: `macos/Sources/TCShellCore/PrivateInferenceSurface.swift`
- Modify: `windows/src/TraceCommons.Interop/PrivateInferenceSurface.cs`,
  `windows/src/TraceCommons.Interop/NativeMethods.cs`

**Interfaces:**
- Consumes: Task 1's `harness_state_line`, `harness_state_tone`,
  `harness_last_seen_line`, `harness_connect_needs_exposure`.
- Produces:
  - `char* tc_harness_state_line(const char* state)`
  - `int32_t tc_harness_state_tone(const char* state)`
  - `char* tc_harness_last_seen_line(const char* when)`
  - `int32_t tc_harness_connect_needs_exposure(int32_t answered, int32_t on)`
- Produces, Swift: `PrivateInferenceCalls.harnessStateLine`, `.harnessStateTone`,
  `.harnessLastSeenLine`, `.harnessConnectNeedsExposure`, and
  `PrivateInferenceSurface.harnessTone(_:calls:) -> PrivateInferenceTone`.
- Produces, C#: `PrivateInferenceSurface.HarnessStateLine(string)`,
  `.HarnessStateTone(string)`, `.HarnessLastSeenLine(string)`,
  `.HarnessConnectNeedsExposure(bool, bool)`.

**Conflicts with:** Task 1 (`PrivateInferenceSurface.swift`,
`PrivateInferenceSurface.cs` neighbourhood). Land after Tasks 1 and 2. Tasks 4,
5 and 6 all consume this; none may start before it.

- [ ] **Step 1: Write the failing ABI test**

In `crates/trace-commons-contributor-ffi/tests/abi.rs`:

```rust
/// The harness sentence and its colour come from one table, reached through
/// the ABI. A shell that recovered the tone by reading the sentence would be
/// matching on text.
#[test]
fn the_harness_branch_tables_cross_the_abi() {
    use trace_commons_contributor::private_inference_copy::*;

    for (label, expected_tone) in [
        (HARNESS_ANSWERING, TC_PRIVATE_INFERENCE_TONE_CLEAR),
        (HARNESS_CONNECTED_UNSEEN, TC_PRIVATE_INFERENCE_TONE_HELD),
        (HARNESS_ANSWERING_SHARED, TC_PRIVATE_INFERENCE_TONE_HELD),
        (HARNESS_SLOT_TAKEN, TC_PRIVATE_INFERENCE_TONE_ATTENTION),
        (HARNESS_CONFIG_UNREADABLE, TC_PRIVATE_INFERENCE_TONE_REFUSED),
        (HARNESS_NOT_INSTALLED, TC_PRIVATE_INFERENCE_TONE_NEUTRAL),
        (HARNESS_NOT_CONNECTED, TC_PRIVATE_INFERENCE_TONE_NEUTRAL),
    ] {
        let c = std::ffi::CString::new(label).expect("a label with no NUL");
        assert_eq!(unsafe { tc_harness_state_tone(c.as_ptr()) }, expected_tone, "{label}");
        let line = unsafe { tc_harness_state_line(c.as_ptr()) };
        assert!(!line.is_null());
        assert!(!take_owned(line).trim().is_empty(), "{label} has no sentence");
    }

    // NULL and a label from a later daemon are neutral, never clear.
    assert_eq!(
        unsafe { tc_harness_state_tone(std::ptr::null()) },
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL
    );
    let later = std::ffi::CString::new("a_state_from_a_later_daemon").expect("no NUL");
    assert_eq!(
        unsafe { tc_harness_state_tone(later.as_ptr()) },
        TC_PRIVATE_INFERENCE_TONE_NEUTRAL
    );
}

#[test]
fn the_first_connect_gate_crosses_the_abi() {
    assert_eq!(unsafe { tc_harness_connect_needs_exposure(0, 0) }, 1);
    assert_eq!(unsafe { tc_harness_connect_needs_exposure(1, 0) }, 0);
    assert_eq!(unsafe { tc_harness_connect_needs_exposure(0, 1) }, 0);
    assert_eq!(unsafe { tc_harness_connect_needs_exposure(1, 1) }, 0);
}

#[test]
fn the_last_seen_line_is_empty_for_an_empty_time() {
    let empty = std::ffi::CString::new("").expect("no NUL");
    assert_eq!(take_owned(unsafe { tc_harness_last_seen_line(empty.as_ptr()) }), "");
    assert_eq!(
        take_owned(unsafe { tc_harness_last_seen_line(std::ptr::null()) }),
        ""
    );
}
```

Reuse whatever the file's existing owned-string helper is called instead of
`take_owned`; do not introduce a second one.

- [ ] **Step 2: Run it and watch it fail**

Run: `cargo test -p trace-commons-contributor-ffi the_harness`
Expected: FAIL — `cannot find function tc_harness_state_tone`.

- [ ] **Step 3: Add the exports**

Directly after `tc_private_inference_serving_line` in `src/lib.rs`, following its
exact shape:

```rust
/// The sentence for one harness state label.
///
/// Exported for the reason [`tc_private_inference_state_line`] is: the label is
/// a wire value and the sentence is wording, and a shell that composed the
/// second from the first would be a fourth place the wording lives.
///
/// Answers the "nothing reported" sentence for a NULL, non-UTF-8 or unknown
/// label -- never "not connected", which would be a claim about a file nobody
/// read.
///
/// Returns an owned string; free it with [`tc_string_free`]. NULL only on a
/// caught panic.
///
/// # Safety
/// `state`, if non-null, must point to a valid, NUL-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_harness_state_line(state: *const c_char) -> *mut c_char {
    guarded_string_no_err(|| {
        let state = if state.is_null() {
            ""
        } else {
            unsafe { borrow_str(state) }.unwrap_or("")
        };
        Ok(to_owned_cstring(
            trace_commons_contributor::private_inference_copy::harness_state_line(state),
        ))
    })
}
```

`tc_harness_state_tone` mirrors `tc_private_inference_state_tone`, mapping onto
the same `TC_PRIVATE_INFERENCE_TONE_*` range (20–24) and falling back to
`TC_PRIVATE_INFERENCE_TONE_NEUTRAL`. A **fourth** disjoint range would be a new
mapper to cross-wire; this surface shares the private-inference palette by
design, and the disjointness that matters is from `TC_ROUTING_TONE_*` and
`TC_WITNESS_TONE_*`, which it inherits.

`tc_harness_last_seen_line` mirrors `tc_private_inference_serving_line` but takes
a `const char*` and returns `""` for NULL, non-UTF-8 or blank.
`tc_harness_connect_needs_exposure` mirrors `tc_private_inference_should_offer`,
including its 0 / non-zero treatment of both arguments.

- [ ] **Step 4: Declare all four in BOTH headers**

- `crates/trace-commons-contributor-ffi/include/trace_commons.h`
- `macos/Sources/CTraceCommons/include/trace_commons.h`

```c
/* The sentence for one harness state label, and the tone it is painted in.
 *
 * Take the SAME input, so the sentence and the colour stay in step by
 * construction. Do not recover the tone by reading the sentence.
 *
 * tc_harness_state_line returns an owned string; free it with tc_string_free.
 * tc_harness_state_tone answers one of the TC_PRIVATE_INFERENCE_TONE_* values,
 * and _NEUTRAL for a NULL, non-UTF-8 or unrecognised label. Only
 * TC_PRIVATE_INFERENCE_TONE_CLEAR may be painted as working: a config file
 * holding the right value is not evidence that a call was ever answered. */
char*       tc_harness_state_line(const char* state);
int32_t     tc_harness_state_tone(const char* state);

/* When a call of this tool's kind was last answered, as a finished sentence.
 * `when` is a rendered time. A NULL, non-UTF-8 or blank `when` gives the empty
 * string; draw nothing at all for it, not a blank row.
 * Returns an owned string; free it with tc_string_free. */
char*       tc_harness_last_seen_line(const char* when);

/* Whether connecting a tool must put the exposure question first.
 * answered is get_settings's private_inference_offer_seen; on is its
 * private_inference. Both are booleans as 0 or non-zero. Non-zero when the
 * question has to be asked and accepted before the connect proceeds. */
int32_t     tc_harness_connect_needs_exposure(int32_t answered, int32_t on);
```

The two copies carry different prose deliberately and the guards strip comments,
so the prose need not match word for word — but prose that contradicts the ABI is
the one failure no test catches. Write the same claims into both.

- [ ] **Step 5: Run the header guards**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi --test header
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi --test abi_header_surface
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi --test abi
```
Expected: PASS. `both_header_copies_declare_the_same_abi` catches a copy you
forgot; `abi_header_surface` catches a signature either copy got wrong.

- [ ] **Step 6: Bind them on both far sides**

Swift: four closures on `PrivateInferenceCalls` (init parameters and stored
properties), plus:

```swift
    /// The tone one harness row is painted in.
    ///
    /// A separate entry point from `tone(_:calls:)` and not a reuse: they take
    /// different labels from different tables, and a shell that fed a harness
    /// label to the listener's table would get `.neutral` for every value --
    /// the exact failure `RoutingTone.fromABI` is documented for.
    public static func harnessTone(
        _ label: String, calls: PrivateInferenceCalls
    ) -> PrivateInferenceTone {
        PrivateInferenceTone.fromABI(calls.harnessStateTone(label))
    }
```

C#: four entries in `NativeMethods.cs` matching the existing
`tc_private_inference_state_line` entry exactly, and four wrappers on
`PrivateInferenceSurface` that take the owned string through
`NativeMethods.TakeOwnedString`.

- [ ] **Step 7: Run**

```bash
cd macos && swift test --filter PrivateInference
```
Plus the Windows interop tests. Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/trace-commons-contributor-ffi/ \
        macos/Sources/CTraceCommons/include/trace_commons.h \
        macos/Sources/TCShellCore/PrivateInferenceSurface.swift \
        windows/src/TraceCommons.Interop/
git commit -m "Carry the harness sentence, its tone and the first-connect gate across the ABI"
```

---

### Task 4: macOS — the harness list, the preview and the first-connect gate

**Runs in parallel with Tasks 5 and 6.** Disjoint directory: `macos/`.
**Conflicts with:** nothing in 5, 6 or 7. Depends on 1, 2 and 3.

**Files:**
- Create: `macos/Sources/TCShellCore/HarnessSurface.swift`
- Create: `macos/Tests/TCShellCoreTests/HarnessSurfaceTests.swift`
- Modify: `macos/Sources/TraceCommonsApp/Views/PrivateInferenceView.swift`
- Modify: `macos/Sources/TraceCommonsApp/AppModel.swift`

**Interfaces:**
- Consumes: Task 3's four calls through `PrivateInferenceCalls`; the
  `harnesses`, `plan_harness` and `commit_harness` methods through `tc_call`;
  Task 1's copy fields.
- Produces: `HarnessRow`, `HarnessPlan`, `HarnessOccupied`,
  `HarnessSurface.rows(fromJSON:) -> [HarnessRow]`,
  `HarnessSurface.plan(fromJSON:) -> HarnessPlan?`.

- [ ] **Step 1: Write the failing surface test**

`macos/Tests/TCShellCoreTests/HarnessSurfaceTests.swift`:

```swift
import XCTest
@testable import TCShellCore

final class HarnessSurfaceTests: XCTestCase {
    /// A row this build cannot read must not silently become a connected one.
    func testAMalformedRowIsDroppedRatherThanGuessed() {
        let rows = HarnessSurface.rows(fromJSON: #"{"harnesses":[{"name":"x"}]}"#)
        XCTAssertEqual(rows.count, 0)
    }

    /// The file path is part of the row, always. A tool nobody expected to be
    /// set up is a question about which file, every time.
    func testTheRowCarriesTheFileItWouldChange() {
        let json = """
        {"harnesses":[{"id":"claude","name":"Claude Code","installed":true,
          "wired":false,"connect_command":"ironwire connect claude",
          "config_path":"/Users/x/.claude/settings.json",
          "facades":["anthropic"],"state":"not_connected","last_seen":null}]}
        """
        let rows = HarnessSurface.rows(fromJSON: json)
        XCTAssertEqual(rows.count, 1)
        XCTAssertEqual(rows[0].configPath, "/Users/x/.claude/settings.json")
    }

    /// A plan with nothing in it is its own message, not an empty confirmation.
    func testANoopPlanIsDistinctFromAChange() {
        let noop = HarnessSurface.plan(
            fromJSON: #"{"id":"claude","connect":true,"path":"/p","changes":[],"occupied":[],"noop":true,"digest":"ab"}"#)
        XCTAssertEqual(noop?.isNoop, true)
        let real = HarnessSurface.plan(
            fromJSON: #"{"id":"claude","connect":true,"path":"/p","changes":["set a thing"],"occupied":[],"noop":false,"digest":"cd"}"#)
        XCTAssertEqual(real?.isNoop, false)
        XCTAssertEqual(real?.changes, ["set a thing"])
    }

    /// An occupied slot survives to the screen, never swallowed. This is the
    /// rule most likely to be lost to a well-meaning simplification.
    func testAnOccupiedSlotSurvivesToTheScreen() {
        let plan = HarnessSurface.plan(
            fromJSON: """
            {"id":"claude","connect":true,"path":"/p","changes":[],
             "occupied":[{"slot":"env.ANTHROPIC_BASE_URL","current":"https://theirs.example"}],
             "noop":true,"digest":"ef"}
            """)
        XCTAssertEqual(plan?.occupied.first?.slot, "env.ANTHROPIC_BASE_URL")
        XCTAssertEqual(plan?.occupied.first?.current, "https://theirs.example")
    }

    /// A tool that is not installed offers no action at all, and is still listed.
    func testANotInstalledToolCannotBeConnected() {
        let row = HarnessRow(
            id: "codex", name: "Codex", configPath: nil, installed: false,
            wired: false, connectCommand: "ironwire connect codex",
            facades: ["openai"], state: "not_installed", lastSeen: nil)
        XCTAssertFalse(row.isActionable)
    }
}
```

- [ ] **Step 2: Run and watch it fail**

Run: `cd macos && swift test --filter HarnessSurfaceTests`
Expected: FAIL — `cannot find 'HarnessSurface' in scope`.

- [ ] **Step 3: Write `HarnessSurface.swift`**

Pure decoding and one predicate. **It authors no sentence**, so it must not
appear in `ShellWordingTests`'s baseline:

```swift
import Foundation

/// One tool, as the daemon described it.
///
/// A carrier. Every word shown about a row comes from the shared copy payload
/// or from `tc_harness_state_line`; `name` is IronWire's, not ours to restate.
public struct HarnessRow: Decodable, Equatable, Sendable {
    public let id: String
    public let name: String
    public let configPath: String?
    public let installed: Bool
    public let wired: Bool
    public let connectCommand: String
    public let facades: [String]
    /// One of the daemon's harness labels. Rendered through the shared table;
    /// never matched on in this shell to pick a colour or a sentence.
    public let state: String
    public let lastSeen: Date?

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case id, name, installed, wired, facades, state
        case configPath = "config_path"
        case connectCommand = "connect_command"
        case lastSeen = "last_seen"
    }

    /// Whether this row may offer a connect or disconnect at all.
    ///
    /// A tool that is not on this machine is listed and disabled rather than
    /// hidden: hiding it makes "you do not have it" indistinguishable from
    /// "this app has never heard of it".
    public var isActionable: Bool { installed }
}

/// One slot the plan refused to take over, and what is in it.
public struct HarnessOccupied: Decodable, Equatable, Sendable {
    public let slot: String
    public let current: String
}

/// A change that has been worked out and not made.
public struct HarnessPlan: Decodable, Equatable, Sendable {
    public let id: String
    public let connect: Bool
    public let path: String
    /// IronWire's own words for what would change. Rendered verbatim.
    public let changes: [String]
    public let occupied: [HarnessOccupied]
    public let isNoop: Bool
    /// Handed back on the commit. A mismatch means the file moved under us.
    public let digest: String

    public enum CodingKeys: String, CodingKey, CaseIterable {
        case id, connect, path, changes, occupied, digest
        case isNoop = "noop"
    }
}

/// Decoding only. A payload this build cannot read is "no evidence about any
/// tool", never a verdict -- the rule the daemon's own `routed_tools` states.
public enum HarnessSurface {
    public static func rows(fromJSON json: String) -> [HarnessRow] {
        struct Envelope: Decodable { let harnesses: [HarnessRow] }
        guard let data = json.data(using: .utf8),
              let envelope = try? decoder().decode(Envelope.self, from: data)
        else { return [] }
        return envelope.harnesses
    }

    public static func plan(fromJSON json: String) -> HarnessPlan? {
        guard let data = json.data(using: .utf8) else { return nil }
        return try? decoder().decode(HarnessPlan.self, from: data)
    }

    private static func decoder() -> JSONDecoder {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return decoder
    }
}
```

- [ ] **Step 4: Render the list at the top of the destination**

In `PrivateInferenceContent.content(_:)`, the harness section goes **above** the
switch. The switch stays, moved below the list and unchanged in behaviour — it
is now the kill switch, which is what it always was, and the spec puts any
change to its behaviour out of scope.

Per row: `row.name`; `copy.harnessConfigPathLabel` and `row.configPath`; the
sentence `model.privateInferenceCalls.harnessStateLine(row.state)` painted with
`PrivateInferenceIndicator.palette(PrivateInferenceSurface.harnessTone(row.state, calls: model.privateInferenceCalls))`;
the last-seen line when `harnessLastSeenLine` returns non-empty; a `Button`
labelled `copy.harnessConnectAction` or `copy.harnessDisconnectAction` chosen on
`row.wired`, `.disabled(!row.isActionable)`; and `row.connectCommand` verbatim
under `copy.harnessCommandLabel`, monospaced and selectable — it is a command,
not prose.

The section carries `copy.harnessSectionTitle`, `copy.harnessListScope` and
`copy.harnessAttributionCaveat`. An empty list renders `copy.harnessNoneFound`.
The list is re-fetched on every appearance and on every daemon status event, not
cached.

- [ ] **Step 5: Preview before commit**

Acting on a row calls `plan_harness`, then presents a sheet showing
`copy.harnessPreviewTitle`, `plan.path`, every entry of `plan.changes` verbatim
(they are already phrased in words — do not rewrite them), every `plan.occupied`
entry under `copy.harnessPreviewOccupied`, `copy.harnessPreviewBackup`, and the
buttons `copy.harnessPreviewConfirm` / `copy.harnessPreviewCancel`. When
`plan.isNoop` the sheet shows `copy.harnessPreviewNothingToDo` and offers only
the cancel — never an empty confirmation.

Confirming calls `commit_harness` with `plan.digest`. On `harness-plan-stale`,
show `copy.harnessPlanStale`, re-fetch the list, and do not retry.

- [ ] **Step 6: The first connect asks the exposure question**

Before the first `plan_harness` with `connect: true`, if
`model.privateInferenceCalls.harnessConnectNeedsExposure(offerSeen, listenerOn)`
is true, present `copy.harnessFirstConnect` and `copy.offerExposure` in full with
`copy.offerAccept` / `copy.offerDecline`. Accepting writes through
`PrivateInferenceSurface.settingsParams(on: true)` and only then proceeds.
Declining writes the marker **alone** — `offerParams(accepted: false)` — and
connects nothing.

`shouldOffer(answered, on) == !answered && !on` must still hold and the first-run
offer must still appear exactly once. Run the existing offer tests.

- [ ] **Step 7: Run, including the wording ratchet**

```bash
cd macos && swift test
cd macos && TC_WORDING_DUMP=1 swift test --filter ShellWordingTests
```
Expected: PASS, and the dump must **not** list
`TCShellCore/HarnessSurface.swift` or
`TraceCommonsApp/Views/PrivateInferenceView.swift`. If it does, a sentence was
authored in Swift; move it into Task 1's copy module. Do not add a baseline entry.

- [ ] **Step 8: Commit**

```bash
git add macos/Sources/ macos/Tests/
git commit -m "List the tools on this Mac and show the file change before making it"
```

---

### Task 5: Windows — the harness list, the preview and the first-connect gate

**Runs in parallel with Tasks 4 and 6.** Disjoint directory: `windows/`.
Depends on 1, 2 and 3.

**Files:**
- Create: `windows/src/TraceCommons.Interop/HarnessSurface.cs`
- Create: `windows/tests/TraceCommons.Interop.Tests/HarnessSurfaceTests.cs`
- Modify: `windows/src/TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs`
- Modify: `windows/src/TraceCommons.App/MainWindow.xaml`,
  `windows/src/TraceCommons.App/MainWindow.xaml.cs`

**Interfaces:**
- Consumes: Task 3's four wrappers on `PrivateInferenceSurface`; Task 1's copy
  properties; the three IPC methods through `DaemonProtocol`.
- Produces: `HarnessRow`, `HarnessOccupied`, `HarnessPlan`,
  `HarnessSurface.ParseRows(string?) -> IReadOnlyList<HarnessRow>`,
  `HarnessSurface.ParsePlan(string?) -> HarnessPlan?`.

- [ ] **Step 1: Write the failing tests**

Mirror Task 4's cases exactly — two shells decoding the same payload by
different rules would produce two behaviours nobody could compare:

```csharp
[Fact]
public void AMalformedRowIsDroppedRatherThanGuessed()
{
    Assert.Empty(HarnessSurface.ParseRows("""{"harnesses":[{"name":"x"}]}"""));
}

[Fact]
public void TheRowCarriesTheFileItWouldChange()
{
    IReadOnlyList<HarnessRow> rows = HarnessSurface.ParseRows(
        """
        {"harnesses":[{"id":"claude","name":"Claude Code","installed":true,
          "wired":false,"connect_command":"ironwire connect claude",
          "config_path":"C:\\Users\\x\\.claude\\settings.json",
          "facades":["anthropic"],"state":"not_connected","last_seen":null}]}
        """);
    Assert.Single(rows);
    Assert.Equal(@"C:\Users\x\.claude\settings.json", rows[0].ConfigPath);
}

[Fact]
public void ANoopPlanIsDistinctFromAChange()
{
    HarnessPlan? noop = HarnessSurface.ParsePlan(
        """{"id":"claude","connect":true,"path":"/p","changes":[],"occupied":[],"noop":true,"digest":"ab"}""");
    Assert.True(noop!.IsNoop);
    HarnessPlan? real = HarnessSurface.ParsePlan(
        """{"id":"claude","connect":true,"path":"/p","changes":["set a thing"],"occupied":[],"noop":false,"digest":"cd"}""");
    Assert.False(real!.IsNoop);
    Assert.Equal(new[] { "set a thing" }, real.Changes);
}

[Fact]
public void AnOccupiedSlotSurvivesToTheScreen()
{
    HarnessPlan? plan = HarnessSurface.ParsePlan(
        """{"id":"claude","connect":true,"path":"/p","changes":[],"occupied":[{"slot":"env.ANTHROPIC_BASE_URL","current":"https://theirs.example"}],"noop":true,"digest":"ef"}""");
    Assert.Equal("env.ANTHROPIC_BASE_URL", plan!.Occupied[0].Slot);
    Assert.Equal("https://theirs.example", plan.Occupied[0].Current);
}

[Fact]
public void ANotInstalledToolCannotBeConnected()
{
    var row = new HarnessRow { Id = "codex", Name = "Codex", Installed = false };
    Assert.False(row.IsActionable);
}

/// Only the clear tone may be painted as working, on a harness row as anywhere.
[Fact]
public void OnlyAnAnsweringHarnessReadsAsWorking()
{
    const int clear = 22; // TC_PRIVATE_INFERENCE_TONE_CLEAR
    Assert.Equal(clear, PrivateInferenceSurface.HarnessStateTone("answering"));
    foreach (string label in new[]
    {
        "not_installed", "not_connected", "connected_unseen", "answering_shared",
        "slot_taken", "config_unreadable", "", "a_state_from_a_later_daemon",
    })
    {
        Assert.NotEqual(clear, PrivateInferenceSurface.HarnessStateTone(label));
    }
}
```

- [ ] **Step 2: Run and watch them fail**

Run the `windows contributor crate tests` job's command on the Windows dev VM
through `win-exec.sh` — do not build a second harness for it.
Expected: FAIL — `HarnessSurface` does not exist.

- [ ] **Step 3: Write `HarnessSurface.cs`**

The records mirror Task 4's Swift types field for field, with
`[JsonPropertyName]` for `config_path`, `connect_command`, `last_seen` and
`noop`, and `IsActionable => Installed` carrying the same comment about why a
missing tool is listed and disabled rather than hidden. Both parsers return an
empty list / `null` on malformed input rather than throwing.

- [ ] **Step 4: The pane, the dialog and the gate**

The pane puts the list above the existing switch, with the same content as
macOS: name, `HarnessConfigPathLabel` plus path, the state sentence and its
tone, the last-seen line, an action button disabled when `!Installed`, and
`ConnectCommand` verbatim in a monospaced selectable box. Read
`MainWindow.xaml.cs:934` (`ShowPrivateInferencePane`) and
`ViewModels/PrivateInferenceViewModel.cs` and follow the existing pane shape
rather than inventing one.

The preview is a `ContentDialog` carrying `HarnessPreviewTitle`, the path, the
changes verbatim, the occupied slots under `HarnessPreviewOccupied`,
`HarnessPreviewBackup`, and the two buttons. A no-op dialog shows
`HarnessPreviewNothingToDo` and offers only the cancel. `harness-plan-stale`
shows `HarnessPlanStale` and re-fetches.

`PrivateInferenceSurface.HarnessConnectNeedsExposure(offerSeen, listenerOn)`
gates the first `plan_harness` with `connect: true`, reusing `OfferExposure` /
`OfferAccept` / `OfferDecline` and the existing `SerializeOfferAnswer` path in
`AnswerPrivateInferenceOfferAsync`.

Every string is read from `PrivateInferenceCopy`. **No XAML literal.**

- [ ] **Step 5: Run, including the wording ratchet**

Run the `windows contributor app` and `windows contributor crate tests` jobs'
commands. `ShellWordingTests.cs` must not gain an entry and no number may rise;
the new files author zero sentences. Measure with the dump switch rather than
typing a number.

- [ ] **Step 6: Commit**

```bash
git add windows/src/ windows/tests/
git commit -m "List the tools on this PC and show the file change before making it"
```

---

### Task 6: GTK — the harness list, the preview and the first-connect gate

**Runs in parallel with Tasks 4 and 5.** Disjoint directory:
`crates/trace-commons-contributor-gtk/`. Depends on 1 and 2 (not on 3: GTK
bypasses the C ABI and links the Rust crate natively).

GNOME has no system tray (`ui/mod.rs`), so this window must carry the whole
capability: nothing here may be reachable only from the tray.

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/private_inference.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/model.rs`

**Interfaces:**
- Consumes: `trace_commons_contributor::private_inference_copy::{harness_state_line, harness_state_tone, harness_last_seen_line, harness_connect_needs_exposure, HARNESS_*}`;
  `app.call("harnesses" | "plan_harness" | "commit_harness", ...)`.
- Produces: `model::Harness`, `model::HarnessOccupied`, `model::HarnessPlan`
  (serde `Deserialize`), and `ui::private_inference::render_harnesses`.

- [ ] **Step 1: Extend the re-export block**

In `copy.rs`, inside the existing "Model calls on this computer" block, keeping
the note that `state_line` and `state_tone` are a pair used as one:

```rust
pub use trace_commons_contributor::private_inference_copy::{
    HARNESS_ANSWERING, HARNESS_ANSWERING_SHARED, HARNESS_CONFIG_UNREADABLE,
    HARNESS_CONNECTED_UNSEEN, HARNESS_NOT_CONNECTED, HARNESS_NOT_INSTALLED,
    HARNESS_SLOT_TAKEN, harness_connect_needs_exposure, harness_last_seen_line,
    harness_state_line, harness_state_tone,
};
```

Add the seventeen new payload constants to the existing `pub use` list beside
`SETTINGS_TITLE`, named `PRIVATE_INFERENCE_HARNESS_*` to match the block's
convention.

- [ ] **Step 2: Write the failing tests**

`private_inference.rs` already carries source-scanning self-tests (`:368`,
`:393`). Add two in the same shape:

```rust
/// No sentence about a harness is authored in this shell. The words come from
/// the shared module, the same one the other two shells reach across the ABI.
#[test]
fn the_harness_rows_author_no_wording() {
    let source = include_str!("private_inference.rs");
    let body = source
        .split("pub fn render_harnesses(")
        .nth(1)
        .expect("render_harnesses is present");
    for literal in body.split('"').skip(1).step_by(2) {
        // CSS class names and widget property names are not wording. A
        // sentence is: it has a space in it and it is not a class name.
        assert!(
            !literal.contains(' ') || literal.starts_with("tc-"),
            "a sentence is authored here rather than read from copy: {literal:?}"
        );
    }
}

/// Only an answering harness is painted as working, here as everywhere.
#[test]
fn only_an_answering_harness_reads_as_working() {
    assert!(copy::harness_state_tone(copy::HARNESS_ANSWERING).reads_as_working());
    for label in [
        copy::HARNESS_NOT_INSTALLED,
        copy::HARNESS_NOT_CONNECTED,
        copy::HARNESS_CONNECTED_UNSEEN,
        copy::HARNESS_ANSWERING_SHARED,
        copy::HARNESS_SLOT_TAKEN,
        copy::HARNESS_CONFIG_UNREADABLE,
    ] {
        assert!(!copy::harness_state_tone(label).reads_as_working(), "{label}");
    }
}
```

- [ ] **Step 3: Run and watch them fail**

Run: `cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml private_inference`
Expected: FAIL — `render_harnesses is present` panics.

- [ ] **Step 4: Build the list**

A `harnesses: gtk::Box` field on `PrivateInferenceView`, appended **above** the
card holding the switch, rebuilt on every render for the same reason the status
box is: the list is read each time it is shown, so a tool that rewrote its own
config corrects itself.

`render_harnesses` fetches through `app.call("harnesses", serde_json::json!({}),
...)` and, per row, appends a `style::card` containing: `row.name` (IronWire's,
never ours); the path row labelled from copy; a
`tone_row(copy::harness_state_line(&row.state), indicator_tone(copy::harness_state_tone(&row.state)))`;
the last-seen meta line when `copy::harness_last_seen_line(&rendered_time)` is
non-empty; a `gtk::Button` labelled from copy with
`set_sensitive(row.installed)`; and `row.connect_command` in a monospaced
selectable `gtk::Label`.

The section header is `copy::PRIVATE_INFERENCE_HARNESS_SECTION_TITLE`, with
`HARNESS_LIST_SCOPE` as body and `HARNESS_ATTRIBUTION_CAVEAT` as meta, and
`HARNESS_NONE_FOUND` when the list is empty.

- [ ] **Step 5: Preview before commit**

An `adw::MessageDialog` following `present_what_gets_removed`'s shape
(`ui/onboarding.rs:475`): heading `HARNESS_PREVIEW_TITLE`, body the path, an
extra child listing `changes` verbatim and `occupied` under
`HARNESS_PREVIEW_OCCUPIED`, a meta line `HARNESS_PREVIEW_BACKUP`, and responses
`HARNESS_PREVIEW_CANCEL` / `HARNESS_PREVIEW_CONFIRM`. A no-op plan shows
`HARNESS_PREVIEW_NOTHING_TO_DO` with the cancel response only. Confirming calls
`commit_harness` with the digest; `harness-plan-stale` shows `HARNESS_PLAN_STALE`
and re-renders.

- [ ] **Step 6: The first connect asks the exposure question**

`copy::harness_connect_needs_exposure(settings.private_inference_offer_seen,
settings.private_inference)` gates it, showing `HARNESS_FIRST_CONNECT` and
`PRIVATE_INFERENCE_OFFER_EXPOSURE` with the existing accept/decline wording, and
writing through the existing `send` path so the switch and the marker stay on one
code path.

- [ ] **Step 7: Run — in the GTK workspace**

```bash
RUSTFLAGS="-D warnings" cargo check --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cargo clippy --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```
This crate is a **separate cargo workspace with its own lockfile**; a root
`--workspace` run does not cover it. If Task 2's new direct dependency moved the
root lock, this lock moves too and both must be committed — and the Flatpak
offline vendor set drifts with them.

- [ ] **Step 8: Commit**

```bash
git add crates/trace-commons-contributor-gtk/
git commit -m "List the tools on this computer and show the file change before making it"
```

---

### Task 7: The contract document

Depends on Tasks 1 and 2 having landed. **Conflicts with Task 1** (same file);
land it last among the doc-touching tasks.

**Files:**
- Modify: `docs/contributor-daemon-ipc-v1_1.md`
- Modify: `crates/trace-commons-contributor/src/routing/mod.rs` (one stale
  sentence)

- [ ] **Step 1: Document the three methods**

In the method table beside `discover_routing`:

| method | params | result | notes |
|---|---|---|---|
| `harnesses` | none | `{"harnesses": [...]}` | read from the filesystem on every call, never cached |
| `plan_harness` | `id`, `connect` | `{id, connect, path, changes, occupied, noop, digest}` | writes nothing |
| `commit_harness` | `id`, `connect`, `digest` | `{id, noop, backup}` | writes only the plan whose digest matches |

- [ ] **Step 2: Document the harness state labels**

A table shaped like the `private_inference_state` one: `not_installed`,
`not_connected`, `connected_unseen`, `answering`, `answering_shared`,
`slot_taken`, `config_unreadable`, plus what an absent or unrecognised label
means. State plainly that `wired` proves a config value and never a call, that
attribution is by protocol family (`facade`, one of `anthropic` or `openai`), and
that `answering_shared` exists because two connected tools of one family cannot
be told apart — and name the upstream change that would fix it: a per-tool path
chosen at connect time, which `plan_connect(id, port, catalog)` does not expose.

- [ ] **Step 3: Document the refusals**

`harness-unknown`, `harness-plan-stale`, `harness-not-listening`,
`harness-unreadable`, `harness-write-failed` — each a fixed label, never a
message body.

- [ ] **Step 4: Correct one stale sentence**

`crates/trace-commons-contributor/src/routing/mod.rs:33` says "this crate takes
no dependency on IronWire". It has taken one since the private-inference slice,
and now takes two. Correct the sentence; keep the reason `RoutedExchange` is a
local type deserialised from the proxy's JSON, which is still true and still
worth saying.

- [ ] **Step 5: Run the contract test**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor ipc`
Expected: PASS. A test checks `METHODS` against this document; a method added in
Task 2 and not documented here fails it.

- [ ] **Step 6: Commit**

```bash
git add docs/contributor-daemon-ipc-v1_1.md \
        crates/trace-commons-contributor/src/routing/mod.rs
git commit -m "Write down what the harness methods answer"
```

---

### Task 8: The whole-tree verification pass

Nothing new is written here. This exists because every earlier task verified only
its own slice, and three of them ran in parallel.

- [ ] **Step 1: The two configurations a workspace check misses**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
RUSTFLAGS="-D warnings" cargo test --workspace --no-run
RUSTFLAGS="-D warnings" cargo check --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```
A `--workspace` run hides feature unification and misses the GTK workspace
entirely; both broke CI before.

- [ ] **Step 2: The permissive-crate standalone build**

`trace-commons-contributor` is MIT OR Apache-2.0 and must build alone with
`--no-default-features`. `ironwire_agents` is new to its graph, so this is the
job most likely to catch it:

```bash
cargo check -p trace-commons-contributor --no-default-features
```

- [ ] **Step 3: Licences, all four runs**

`ironwire_agents` is a new direct dependency, and `--features` is a **global**
flag on `cargo deny`, before the subcommand:

```bash
cargo deny check licenses
cargo deny --features near-ai-scorer check licenses
cargo deny --features local-gpu-models check licenses
cargo deny --all-features check licenses
```

- [ ] **Step 4: The licence boundary**

```bash
cargo test -p trace-commons-server --test license_boundary
```
`trace-commons-contributor` must not have gained anything AGPL. If it fails,
remove the dependency; do not edit the expected sets.

- [ ] **Step 5: Both header copies, and the ABI they describe**

```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi --test header
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi --test abi_header_surface
```

- [ ] **Step 6: The three shells**

```bash
cargo build -p trace-commons-contributor-ffi
cd macos && swift test
```
Plus the Windows app and interop test jobs, and the GTK workspace's own
`cargo test`.

- [ ] **Step 7: The wording ratchets, measured**

```bash
cd macos && TC_WORDING_DUMP=1 swift test --filter ShellWordingTests
```
and the Windows equivalent. Confirm no new file appears in either dump and no
baseline number was raised. Any number that had to move is lowered
deliberately, in the commit that moved the copy.

- [ ] **Step 8: Format and lint**

```bash
cargo fmt --all
cargo clippy -p trace-commons-contributor -p trace-commons-contributor-ffi --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
git show --stat HEAD
```
The repo is not rustfmt-clean, so the post-edit hook can turn a one-line edit
into a whole-file diff. Check `git show --stat` after every commit and split a
formatting-only change out if one appeared.

- [ ] **Step 9: Commit anything the pass moved**

```bash
git commit -am "Bring the harness slice's formatting and locks into line"
```

---

## Self-review

**Spec coverage.**

| Spec item | Task |
|---|---|
| The list as the destination's primary surface | 4, 5, 6 (Step 4 in each) |
| Name, installed, wired, config path shown always | 2 (`HarnessRow`), 4/5/6 |
| Not-installed listed and disabled, never hidden | 2 (`state_for`), 4 Step 1, 5 Step 1 |
| Degrades to the two built-ins, and says so | 2 (`the_list_degrades_to_the_built_in_tools`), 1 (`HARNESS_LIST_SCOPE`) |
| Connect/disconnect previewed, never immediate | 2 (`plan_harness` writes nothing), 4/5/6 Step 5 |
| `changes` in IronWire's own words | 4/5/6 Step 5 — rendered verbatim |
| `occupied` shown, never swallowed, never overwritten | 1 (`HARNESS_PREVIEW_OCCUPIED`, `HARNESS_SLOT_TAKEN`), 2 (`state_for`), 4/5 Step 1 |
| `is_noop()` gets its own message | 1 (`HARNESS_PREVIEW_NOTHING_TO_DO`), 4/5 Step 1 |
| Exposure question moved to a first-connect gate | 1 (`harness_connect_needs_exposure`), 3 (ABI), 4/5/6 Step 6 |
| Master switch reduced to a kill switch | 4/5/6 Step 4 — the switch moves below the list, behaviour unchanged (spec puts behaviour changes out of scope) |
| Three-valued state including "has a call arrived" | 1 (labels), 2 (`state_for`, ledger sightings) |
| Restart-needed | 1 — folded into `HARNESS_CONNECTED_UNSEEN`, with the reason recorded on the constant |
| Unparseable config refused, distinguishable from no-op | 1 (`an_unreadable_config_is_refused_rather_than_silent`), 2 (`state_for` ordering, `ERR_HARNESS_UNREADABLE`) |
| Nothing detected says what was looked for | 1 (`HARNESS_NONE_FOUND`) |
| Attribution approximate, described as such | 1 (`HARNESS_ATTRIBUTION_CAVEAT`, `HARNESS_ANSWERING_SHARED`), 2 (`facades_for`, `rows`), 7 |
| `connect_command` shown verbatim as a fallback | 4/5/6 Step 4 |
| `wired` read each time, never cached | 2 (module doc + `rows`), 4/5/6 |
| No shell authors a harness string | 6 Step 2 (`the_harness_rows_author_no_wording`), 4/5 (`ShellWordingTests`) |
| Banned words, `route` included, absent | 1 Step 4 (sweep extension) |
| Disconnect removes only what IronWire wrote | Upstream `claude_settings::disconnect` / `codex_config::disconnect`; not reimplemented, and Task 2's module doc says so explicitly |
| A tool that is not installed cannot be connected | 2 (`a_tool_that_is_not_installed_is_never_reported_as_connected`), 4/5 (`isActionable` / `IsActionable`) |
| One tool per action | Global Constraints; `plan_one` takes a single id |

Two spec items are deliberately **not** implemented: the per-tool pass-through
(does not exist upstream; the spec names it as a future upstream fix) and a
per-tool path chosen at connect time (`plan_connect(id, port, catalog)` does not
expose one, so exact attribution stays out of reach — Task 7 records why).

**Placeholder scan.** No "TBD", no "add error handling", no "similar to Task N".
Every task carries literal code or names the exact file, symbol and line to
follow. Task 5's steps are lighter on literal C# than Tasks 4 and 6 for the
reason the predecessor plan gives: the WinUI XAML and view-model sources were not
read in full while planning. The implementer should read
`MainWindow.xaml.cs:934` (`ShowPrivateInferencePane`) and
`ViewModels/PrivateInferenceViewModel.cs` and follow the existing pane and dialog
shapes rather than inventing new ones; the tests in Task 5 Step 1 are literal and
are the contract.

**Type consistency.** `HarnessRow` has the same nine fields in Rust
(`daemon/harness.rs`), Swift (`HarnessSurface.swift`) and C#
(`HarnessSurface.cs`), with the wire keys `config_path`, `connect_command` and
`last_seen` snake-cased in all three. `HarnessPlan` has the same seven (`id`,
`connect`, `path`, `changes`, `occupied`, `noop`, `digest`) and `HarnessOccupied`
the same two (`slot`, `current`). `state` is a `String`/`string` on every side
and is never matched on outside the shared branch tables. Tones reuse the
existing `TC_PRIVATE_INFERENCE_TONE_*` range (20–24) rather than opening a fourth
range, and `reads_as_working` / `readsAsWorking` stay the only predicate an
indicator asks. `harness_connect_needs_exposure(bool, bool) -> bool` matches
`should_offer`'s shape exactly and crosses the ABI as
`(int32_t, int32_t) -> int32_t`, like `tc_private_inference_should_offer`.
`harness_state_line` and `harness_state_tone` take the same `&str` / `const char*`
as `state_line` and `state_tone` do, so the sentence and the colour stay in step
by construction.

**Ordering and parallelism.**

- Task 1 → Task 3 (the Swift and C# copy types move first).
- Task 2 → Task 3 (its ABI tests call the crate) and → Task 7.
- Tasks 1, 2, 3 all → Tasks 4 and 5. Task 6 needs only 1 and 2, since GTK links
  the Rust crate natively and never touches the C ABI.
- **Tasks 4, 5 and 6 run in parallel**, on `macos/`, `windows/` and
  `crates/trace-commons-contributor-gtk/` respectively. They share no file.
- Task 7 lands after 1 and 2 (both touch `docs/contributor-daemon-ipc-v1_1.md`).
- Task 8 last.

Tasks 1, 2 and 3 are **in flight with other agents**. Verify against what landed
before implementing any of them; reconcile names rather than renaming what
shipped.
