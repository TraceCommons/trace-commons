# IronWire routing on Windows and macOS Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Windows and macOS the routing surface GTK now has, and make all three shells say the same words for the same reason.

**Architecture:** The vocabulary and the sentence-building move to a shared, FFI-reachable home so the three shells render one source of truth rather than three copies. macOS additionally gains the runtime settings call it has never made.

**Tech Stack:** Rust (`trace-commons-contributor`, `-contributor-ffi`), Swift (`macos/`), C# / WinUI (`windows/`).

**Spec:** [`docs/superpowers/specs/2026-09-02-ironwire-front-end-design.md`](../specs/2026-09-02-ironwire-front-end-design.md) — read "The organising principle", "Changes apply immediately" and "Per-shell notes".

## What exists, verified on this tree

GTK shipped the surface in PR #545: a declared token directory, discovery via
`~/.ironwire/endpoint.json`, `probe_routing` with three outcomes,
`probe_routed_tools` for per-tool state, a hot-swappable ledger, and a status
block with `not_declared` / `awaiting_rows` / `rows_seen`.

**The daemon side is done and shell-agnostic.** Everything below is presentation.

Two facts about macOS, checked rather than assumed:

- **The app has never called `set_settings`.** `"set_settings"` appears only in
  `CTraceCommons/include/trace_commons.h` (documentation) and
  `tc-ffi-demo/main.swift` (a demo). Declarations enter at daemon start through
  `tc_daemon_start_with_settings`.
- **But the mechanism exists.** `TCBridge/TCDaemon.swift:196` calls the generic
  `tc_call(h, cMethod, cParams)`, and the app already drives five methods
  through it — `set_project_mode`, `set_consent_scopes`, `set_public_profile`,
  `get_settings`, `get_public_profile`.

So macOS needs a **call site**, not a bridge. That is materially cheaper than
the "macOS is the expensive one" estimate this plan was scoped under, and it is
why Task 2 is small.

## The parity problem, and the precedent for solving it

GTK's routing copy lives in `crates/trace-commons-contributor-gtk/src/copy.rs`,
which Swift and C# cannot link. The default outcome is three hand-maintained
copies of one vocabulary — and PR #545's own report flagged that the shells
already "say different things about the same declaration".

**This repo has already refused that once.** `windows/src/TraceCommons.Interop/ScrubDetectorCopy.cs`
opens with:

> THE LIST IS GENERATED, NOT WRITTEN HERE. Its contents come from the scrubber's
> own detector table, reached through `tc_scrub_detector_names()`, because a
> hand-maintained list of what is removed is exactly the kind of claim that
> silently stops being true.

The same argument applies with more force here, because the routing words are
**claims about privacy**. Three copies of "Via IronWire" is a nuisance; three
copies that drift into three different claims about where someone's code went is
a defect.

## Global Constraints

- **Say nothing the evidence does not support.** `wired` is true for any
  loopback host on any port whose path is `/anthropic`, so it cannot
  distinguish our daemon from another local proxy. **"Via IronWire" asserts a
  property of the local hop; "Private" asserts one of the destination.** Only
  the first is supported. The words "Private" and "Not private" must not
  reappear in any shell.
- **No money, anywhere.** There is no rate: emission is a flat constant the
  pilot sets to zero, the graded score is shadow-only, settlement is disabled,
  and credits are documented as non-transferable.
- **Nothing about corpora, credits, ownership or contribution.** This surface is
  for someone with no invite. Not greyed out, not as a teaser.
- **No restart notice.** Declarations apply on the next poll. If a shell needs
  to say otherwise, that is a finding, not a sentence.
- **`awaiting_rows` is not a fault.** A contributor who just changed a setting
  sees it until the next tick.
- **`last_refresh_at` is per-process.** Never an install date or a
  "connected since".
- **Every negative assertion names its specific error variant or value.** Not
  `assert!(x.is_err())`.
- **Mutation-check every guard.** Break it, watch it go red, revert, report the
  actual output. **If a mutation passes, that is a finding, not a relief** —
  three passed today for three different reasons: an empty fixture, a literal
  that was also the expected value, and two orderings that produced identical
  bytes.
- No emojis. Short imperative commit subjects.
- **Prefix every command with its own explicit `cd`, and use absolute paths in
  edits.** The agent working directory defaults to a different live worktree and
  can revert mid-task with no signal.

---

### Task 1: One vocabulary, three shells

**Files:**
- Modify: `crates/trace-commons-contributor/src/` (a new copy module)
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs` and its C header
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs` (consume, not duplicate)

**Interfaces:**
- Produces: the routing vocabulary and the sentence builders, reachable from Swift and C#

**Decide before building, and record the decision.** Follow
`tc_scrub_detector_names()` as the precedent, but check what it actually does
before copying its shape — an FFI call per string may be wrong where a single
call returning the set is right. Whether the *sentences* (which interpolate a
path, a port, a timestamp) cross the boundary as templates or as fully-formed
strings is the real question. **A template a shell fills in is a fourth place
the wording can drift.**

If you conclude the vocabulary should cross but the sentences should not, say
why, and say what stops the sentences drifting instead.

**GTK must consume the shared source rather than keep its own copy.** If GTK
keeps a parallel definition, this task has produced a fourth copy and made
things worse.

The forbidden-word sweep in `copy.rs` walks a marked region via `include_str!`.
**If the strings move, that sweep must move with them or it silently covers
nothing** — the exact failure it was written to replace.

- [ ] **Step 1: Read `tc_scrub_detector_names` end to end** — Rust, header, Swift and C# call sites — and write down the shape it uses.
- [ ] **Step 2: Write the failing tests** — the vocabulary is identical across the boundary; no word contains any other, both directions, case-insensitively; the forbidden-word sweep still covers every rendered string.
- [ ] **Step 3-5: Run, implement, run**
- [ ] **Step 6: Mutation-check** — change one word on the Rust side only and confirm a shell-side test fails. **If nothing fails, the shells are not consuming the shared source** and the task has not been done.
- [ ] **Step 7: Commit**

---

### Task 2: macOS learns to change a setting while running

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/` (the daemon client), `macos/Sources/TCBridge/TCDaemon.swift` if needed

**Interfaces:**
- Produces: a `setSettings` path on the macOS daemon client

The generic `tc_call` already exists at `TCDaemon.swift:196` and already carries
five methods. This adds a sixth. **Verify that list yourself** — it was read
from a grep, and a method reached another way would change the shape.

**Why this is required rather than nice.** Without it a declaration only takes
effect at daemon start, so a contributor changing a port would be told to
restart — and no other shell says that, because Task 3 of the GTK plan removed
the need. A shell that alone demands a restart is a shell whose users conclude
the feature is broken.

- [ ] **Step 1: Write the failing test** — the client sends `set_settings` with exactly the declared key and value, and rejects a malformed one rather than sending it.
- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** — send a different method name and confirm the test fails. Then confirm no *other* call site changed behaviour: the five existing methods must be untouched, and a test should say so.
- [ ] **Step 6: Commit**

---

### Task 3: The macOS routing surface

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/`, `macos/Sources/TCShellCore/`
- Create: a routing copy file beside the existing `*Copy.swift` files, consuming Task 1's source

Four surfaces, the same four GTK has:

- **The declaration** — a toggle, a port field enabled only when on, and an
  optional token-directory field. Default the port to the conventional value but
  **write nothing until the contributor acts**: `None` means off, and a
  displayed default must never become a declaration.
- **The probe result**, on save. Three outcomes, three strings, and the
  token-unreadable one **names the absolute path** the daemon reported. On macOS
  this is the likely failure, not an exotic one, because a GUI-launched daemon
  never sees `$IRONWIRE_HOME`.
- **The status line**, from the three states.
- **Per-tool words**, from `probe_routed_tools` — never from the declaration.
  A tool IronWire has never heard of reads as unknown. There is no `gemini` row
  upstream at all, so Gemini CLI legitimately reads unknown on a machine where
  it is installed and in use.

- [ ] **Step 1: Write the failing tests** — state-to-copy mapping for each of the three probe outcomes and three status states, including that the path propagates; and that per-tool words come from the tools answer rather than the switch.
- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** — map `awaiting_rows` to the error copy and confirm a test fails; render a tool word from the declaration and confirm a test fails.
- [ ] **Step 6: Commit**

---

### Task 4: The Windows routing surface

**Files:**
- Modify: `windows/src/TraceCommons.Interop/`, `windows/src/.../Controls/SettingsView.xaml` and its view model
- Create: a routing copy file beside the existing `*Copy.cs` files, consuming Task 1's source

Windows already writes settings live — `SettingsProtocol` serialises exactly one
`set_settings` key per user edit — so unlike macOS there is no enabling work.

Same four surfaces as Task 3, same rules. **Read Task 3's implementation before
starting**: where the two shells can share a decision they should, and where
they diverge the divergence should be a platform fact rather than an accident.

- [ ] **Step 1: Write the failing tests** — as Task 3, in the Windows test project.
- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** — as Task 3, plus: change a word on the Rust side and confirm the Windows test notices.
- [ ] **Step 6: Commit**

---

## Not in this plan

- **The unlock and ownership surfaces.** They need attested sessions on the
  contributor's machine — the witness service, or IronWire capturing NEAR AI
  receipts.
- **A `wired_to_us` distinction.** IronWire cannot currently tell its own daemon
  from another loopback proxy on `/anthropic`, and nothing on its settings
  response carries a port. Raised upstream; the copy in this plan is written to
  survive the answer either way.
- **`cost_usd` in any shell.** Separate work, and it lands in the session
  moment rather than this surface.
