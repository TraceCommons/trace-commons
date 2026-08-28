# Plan: shorten the Letta Trajectory conversion workflow (Devfolio item 1b)

Date: 2026-08-27
Scope: the friction the 2026-08-27 hackathon-onboarding spec explicitly left
out of scope ("Shortening the Letta-side conversion workflow (item 1b)").
All paths below are relative to the repository root.

**Status: the first two layers are done; the rest of this plan stands.**

- The unconditional bug fix landed: `system` and `observation` records are
  accepted rather than rejecting the whole file, with two conformance
  fixtures. See `crates/trace-commons-contributor/src/source/trajectory.rs`.
- The npm CLI landed and is published as `@tracecommons/trajectory-export`
  0.1.0, under `tools/trajectory-export/`. Two findings from building it that
  this plan predates: several sources are export-only and report
  `listing_unavailable` rather than listing a store -- `gemini-cli` among
  them, which means the library path cannot reach Gemini sessions unaided at
  all and a native adapter is the only practical route; and
  `normalizeTranscript` throws `missing_assistant_records` on an ordinary
  typed-then-quit session, which had to be caught so one such session cannot
  abort a whole `--all` run.
- Still open from this plan: offering the CLI upstream to Letta as a `bin`,
  and native per-harness adapters via the Slice A1 registration seam.

## 1. What the friction actually is

### The documented command does not exist

`README.md:346-350` tells contributors:

```bash
npx @letta-ai/trajectory > session.json
trace-commons-contributor submit --trajectory session.json
```

**No published version of `@letta-ai/trajectory` has ever shipped a CLI.**
Verified against the npm registry (2026-08-27): versions 0.1.0 (2026-07-21)
through 0.3.0 (2026-08-20, latest) all have `bin: null` and the package
README documents a programmatic API only. `npx @letta-ai/trajectory` fails
with npm's "could not determine executable to run" — it cannot have worked
for any contributor, ever. The 2026-07-25 design's premise of "a documented
two-step, not a runtime dependency" was never true; the second "step" is
writing a Node program. That, concretely, is why Devfolio heard "too many
additional steps."

### The steps a contributor on an unsupported harness takes today

Verified against the published package (registry metadata + the 0.3.0
tarball + the `agent-trajectory` PyPI description, all fetched 2026-08-27):

1. Install the contributor CLI (out of scope here; companion spec item 2).
2. Run the README's `npx` line. It fails (no `bin` in any version — the
   failure itself is certain, though I did not execute `npx` live).
3. Discover from Letta's README that the package is a library:
   `normalizeTranscript({ source, transcript })`. No auto-detection of the
   harness — the caller supplies the `source` slug. No interactive prompts.
   No requirement to run inside the project. Requires Node.js >= 20 (or the
   Python wrapper `agent-trajectory`, which embeds a JS bundle;
   whether the wrapper needs a Node runtime at run time is **unverified**).
4. `npm install @letta-ai/trajectory` somewhere.
5. Locate the harness's session store or produce an export from inside the
   harness. As of 0.3.0 upstream has 14 adapters (claude-code, codex,
   cursor, copilot-cli, deepagents, droid, gemini-cli, hermes, letta-code,
   omp, openclaw, opencode, openhands, pi, atif). Ten of them are backed by
   `listTrajectories()`, a local-store discovery API; five (atif,
   copilot-cli, cursor, gemini-cli, opencode) are export-only — the caller
   finds the exported file. Deep Agents goes through `normalizeCheckpoint`
   with a LangGraph SQLite store plus `threadId`.
6. Write a script: read the transcript, call `normalizeTranscript` with the
   right slug, serialize `records` to a file.
7. `trace-commons-contributor submit --trajectory session.json`.
8. Possibly get rejected anyway — see the conformance gap below.

That is "install Node, learn a library API, write code" where the README
promises one command.

### Our reader now rejects valid v1 files

Upstream's `trajectory-v1.schema.json` (shipped in the 0.3.0 tarball,
`$id: https://letta.ai/schemas/trajectory/v1.json`) now includes two record
roles the 2026-07-25 design predates: `system` and `observation`
("environment feedback that cannot be attributed to one specific tool call,
such as merged terminal output"). Our reader
(`crates/trace-commons-contributor/src/source/trajectory.rs`, role dispatch
around line 122) accepts only `meta|user|reasoning|assistant|tool` and
fails the **whole file** with `unknown_record` on anything else, per its
fail-closed contract. So a contributor who does everything right against
current upstream can still be rejected — on real sessions, since
`observation` records are exactly what adapters emit for merged terminal
output. (`system` is omitted by upstream by default, so it bites only when
a producer opts in via `filters.systemMessages: "include"`.)

### Stale claims in the companion spec

`docs/superpowers/specs/2026-08-27-hackathon-onboarding-friction-design.md`
states Trajectory "has no Gemini CLI adapter." True on the 2026-07-25 list;
false as of upstream 0.3.0 (2026-08-20), which ships
`adapters/gemini-cli/` as an export-only input contract. Slice A (native
Gemini adapter) remains justified — our native path is zero-step and reads
the local store directly, which upstream's export-only Gemini contract does
not — but the spec's factual claim needs a correction note so the next
reader doesn't re-derive a decision from a stale premise. (Slice A itself
is unchanged by this plan.)

## 2. Approaches

### Slice 0 (prerequisite under every approach): stop documenting a broken
### command, and stop rejecting valid files

Whatever else we build: the README must not prescribe a command that has
never worked, and the reader must accept schema-valid v1 files. This is a
bug-fix slice, not a workflow-shortening slice, and it is unconditional.

### Approach A (recommended): publish a thin converter CLI that wraps
### upstream

A small npm package — working name `@tracecommons/trajectory-export`,
final name pending an npm-scope decision — that is the CLI Letta never
shipped:

- `npx @tracecommons/trajectory-export@<pinned>` with no arguments probes
  the ten listable sources via `listTrajectories()`, shows the newest
  sessions per source, and normalizes the selection.
- `--source <slug> --input <file>` covers the five export-only sources and
  Deep Agents (`--source deepagents --thread <id>`).
- Output: `<source>-<short-id>.trajectory.json` written into the current
  directory — deliberately matching the `*.trajectory.json` suffix the
  companion spec's Slice B auto-discovers, so once B lands the whole flow
  is: one `npx` command, then bare `submit`.

Why this beats the alternatives:

- It makes the README's promised one-command conversion real without
  adding a subprocess, a Node dependency, or any code to the Rust CLI. The
  contributor runs npx themselves, exactly the trust posture the
  2026-07-25 design already accepted when it documented the (fictional)
  npx line.
- Letta still absorbs adapter churn — the wrapper pins one exact upstream
  version and calls two public functions. The maintenance we own is a
  couple hundred lines of argument parsing and file writing, not fourteen
  adapters.
- It works for all fourteen harnesses at once, today, including the ones
  no one has asked about yet — the capability answer, not the instance
  answer.

Costs and risks: we own an npm package (publish pipeline, scope
registration, supply-chain hygiene); Node >= 20 remains a prerequisite for
this path (it always was); the tool is a second artifact to version.

### Approach B: native Rust adapters per harness, via the Slice A1 seam

The A1 registration seam makes adapter N+1 "a module plus a table row."
Native adapters are the only genuinely zero-step path — no Node, no
conversion command at all, daemon watching included.

But the 2026-07-25 rejection still holds at fleet scale: porting fourteen
adapters permanently owns exactly the churn the Trajectory dependency
exists to shed, and upstream's own SOURCE_VERSION_AUDIT documents active
format drift. B is the right tool selectively — Gemini (Slice A, already
specced) and whichever harness next shows real contributor demand — not
the general answer to item 1b. Keep it as the escalation path per harness,
with A1 keeping its marginal cost low.

### Approach C: shell out to Node from the Rust CLI

Re-examined rather than ignored, because upstream has changed: 0.3.0 ships
`dist/python-cli.js`, a versioned stdin/stdout JSON bridge built precisely
so a non-JS host (their Python wrapper) can drive normalization as a
supervised child process. A `trace-commons-contributor` that spawned a
pinned local Node against that bridge would be a cleaner integration than
the one the 2026-07-25 design rejected.

Still rejected. The original objection — a subprocess surface in a
privacy-sensitive CLI for a soft dependency — stands, and the realistic
form of this feature is worse than the design ever considered: it would in
practice mean invoking `npx`, which downloads and executes
network-fetched, unpinned-by-default code over the contributor's raw
session text, from inside the binary whose whole design premise is that
redaction happens before anything leaves the machine. A version-pinned,
checksummed local Node bridge would mitigate that, but then we own a
runtime-provisioning problem larger than Approach A's whole surface. The
tradeoff has shifted, not flipped.

### Approach D (parallel, cheap): upstream the CLI to Letta

Offer Letta a PR adding a `bin` (the code is nearly Approach A's tool
minus our output convention). The 2026-08-27 spec rejected upstreaming a
*Gemini adapter* because it put the fix behind another project's release
cycle; the same reasoning caps D at "parallel track, not the plan of
record." If accepted, our wrapper shrinks to an alias and the README's
original line finally becomes true. Apache-2.0 both sides; no licensing
friction.

### Recommendation

Slice 0 immediately; Approach A as the deliverable; Approach D opened in
parallel; Approach B reserved per-harness on demonstrated demand. After A
plus the companion spec's Slice B, the unsupported-harness flow is two
commands and zero scripting, down from "install Node and write a program."

## 3. Implementation plan

Ordered by dependency. Tasks 1-3 are Rust-side (TDD, this repo's normal
gates). Tasks 4-7 are the converter tool. Task 8-9 are docs and upstream.

### Task 1: conformance fixtures for the new roles (failing tests first)

Add to both fixture trees (they are separate copies serving different
tests — `crates/trace-commons-contributor/fixtures/` for the unit corpus,
`crates/trace-commons-contributor/tests/fixtures/letta-conformance/` for
the producer-facing corpus):

- `ok__system-record.jsonl` — a minimal trajectory containing a `system`
  record (`role`, `content`, `timestamp`).
- `ok__observation-record.jsonl` — likewise with an `observation` record
  carrying merged-terminal-output-shaped (sanitized) content.

The conformance test (`letta_conformance_corpus_matches_expected_outcomes`
in `src/source/trajectory.rs`) derives expectations from filenames, so
these two files alone produce the failing tests: both currently die with
`unknown_record`.

Run: `cargo test -p trace-commons-contributor` (confirm the two new cases
fail, and only they fail, against the pre-change baseline).

### Task 2: accept `system` and `observation` in the reader

`crates/trace-commons-contributor/src/source/trajectory.rs`, the role
dispatch (~line 122):

- `observation` — `SessionEventKind::Opaque` with `content` **preserved**
  and `structured: {"type": "observation"}`. Observation content is real
  work product (terminal output); it flows through the client-side
  redactor like any other event text. `SessionEvent.content` already
  admits this — Opaque-with-content is a struct-supported shape even
  though `claude_code.rs` happens to use marker-only Opaques.
- `system` — `SessionEventKind::Opaque`, marker-only
  (`structured: {"type": "system"}`), content dropped. Upstream omits
  system messages by default; when present they are harness boilerplate
  or injected instructions, the lowest-value and most prompt-shaped text
  in a session. Dropping content keeps the accept/reject contract fixed
  without growing what we retain.
- Both roles still require a valid `timestamp` (schema does), and both
  still hard-reject on missing/invalid fields — the fail-closed posture
  narrows only by admitting two schema-blessed roles, not by tolerating
  malformed records.

Deliberately **not** adding `SessionEventKind::Observation`: a new event
kind changes `canonical_whole_trace_representation` output and opens
another scoring-cohort boundary (the 2026-07-25 design documents what that
costs). Opaque already exists in canonical rendering. If observation
content later proves valuable enough to distinguish, that is a separate
protocol decision with its own cohort note.

Add in-file unit tests beside the existing ones: mapping of each new role,
content preserved/dropped as specified, timestamp still required.

Update `tests/fixtures/letta-conformance/README.md` ("Contract details a
producer is likely to get wrong") to mention the two roles and the system
content-drop behavior.

Gates: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor`,
`cargo test -p trace-commons-contributor`, repo clippy invocation,
`cargo fmt --all`.

### Task 3: README correction (interim, before the tool exists)

`README.md:346-350`: remove the `npx @letta-ai/trajectory > session.json`
line — it has never worked. Interim honest wording: state that
`@letta-ai/trajectory` is a Node library (>= 20), link Letta's README for
`normalizeTranscript` / `listTrajectories`, and keep the
`submit --trajectory` half unchanged. Include a minimal working `node -e`
example so the path is at least copy-pasteable:

```bash
npm install @letta-ai/trajectory
node -e 'const {normalizeTranscript}=require("@letta-ai/trajectory");
  const fs=require("fs");
  const {records}=normalizeTranscript({source:process.argv[1],
    transcript:fs.readFileSync(process.argv[2],"utf8")});
  fs.writeFileSync("session.trajectory.json",JSON.stringify(records));' \
  <source> <transcript-file>
```

(`.trajectory.json` suffix chosen now so the same instructions survive
Slice B's auto-discovery rule.) This lands with Task 2 so the docs are
never ahead of the reader. Replaced by Task 7's one-liner when the tool
publishes.

### Task 4: decision gate — npm scope and package name

Blocking decision for Zaki before Task 5: npm scope (register
`@tracecommons` or publish unscoped), package name, and whether the tool
lives in this repo (`tools/trajectory-export/`, recommended: one review
surface, one issue tracker) or its own repo. Also per the dependency
policy: this adds a JS dependency surface (exactly one runtime dep,
`@letta-ai/trajectory`, pinned exact, Apache-2.0, 0 transitive runtime
deps — verified from the 0.3.0 tarball; devDeps are ajv/typescript only).
Surface for explicit approval; do not bury in an unrelated commit.

### Task 5: the converter tool

`tools/trajectory-export/` (pending Task 4): plain Node >= 20, no build
step, no TypeScript toolchain — a single ESM entry with `bin` wired.

Behavior:

- No args: `listTrajectories()` across the ten listable sources, newest
  first, numbered picker on a TTY; refuses (with usage) when stdout is not
  a TTY and no flags were given.
- `--source <slug>` limits probing; `--source <slug> --input <file>`
  normalizes an export (the five export-only sources); `--source
  deepagents --thread <id> [--store <path>]` routes through
  `normalizeCheckpoint`.
- `--out <dir>` (default `.`); writes `<source>-<short-id>.trajectory.json`
  and prints the path. Refuses to overwrite an existing file.
- Surfaces upstream `diagnostics` verbatim on stderr; exits nonzero on
  `NormalizationError`.
- Never makes network requests itself; touches only the local stores and
  the output directory. Error output: upstream's messages plus our own
  label-only wrapper — never session content. (Printing local paths to the
  contributor's own terminal is fine; this is a local tool, not a stored
  row.)

Tests (Node's built-in `node --test`, no extra devDeps): argument parsing,
output naming/no-overwrite, export-only routing, and one integration case
per input mode against small sanitized fixture transcripts checked in
under `tools/trajectory-export/fixtures/` (reuse the shapes from our
letta-conformance corpus; the tool's fixtures are harness-native inputs,
not trajectory outputs, so they are new files).

Supply-chain hygiene: exact-pinned dependency, committed
`package-lock.json`, `npm ci` only, npm provenance on publish, and the
README instructs the pinned invocation
(`npx @tracecommons/trajectory-export@0.1.0`) rather than floating latest.

### Task 6: CI job for the tool

`.github/workflows/ci.yml` gains one job: checkout, Node 24 (already the
runner default), `npm ci && npm test` in `tools/trajectory-export/`,
path-filtered to that directory. This is a change to a fully-gating
workflow — keep it additive, touch no existing job, hold
`actions/checkout@v6`. Publishing is a separate manual/tag-triggered
workflow, not part of PR CI.

### Task 7: README and docs, final form

- `README.md` "Contributing sessions from other harnesses": the flow
  becomes the pinned `npx` one-liner plus `submit --trajectory
  session.trajectory.json` (or bare `submit` once Slice B lands — phrase
  so it's true both before and after).
- Update the harness list: fourteen sources as of upstream 0.3.0, noting
  which are export-only.
- One-line correction note in
  `docs/superpowers/specs/2026-08-27-hackathon-onboarding-friction-design.md`
  (its "Trajectory has no Gemini CLI adapter" claim, stale as of upstream
  0.3.0) — a dated addendum, not a rewrite, so the decision record stays
  intact. Same for the 2026-07-25 spec's "documented two-step" premise
  and its eight-harness list.

### Task 8: upstream PR to Letta (parallel, non-blocking)

Offer the CLI upstream as a `bin` on `@letta-ai/trajectory` (Approach D).
Contents: Task 5's picker/flags minus our output-suffix convention. If
merged, a follow-up here swaps the README to `npx @letta-ai/trajectory`
and deprecates our wrapper with an alias notice. No task in this plan
waits on it.

### Task ordering summary

1 → 2 → 3 land together as one PR (reader conformance + honest README).
4 gates 5 → 6 → 7 (the tool PR, or two: tool then CI+docs).
8 any time after 5 exists.

## 4. What could go wrong

- **Upstream API churn.** 0.x semver; `listTrajectories` shipped within
  the last week of change. Exact pin means we never break silently; the
  cost is manual bumps. The letta-conformance corpus already catches
  output-schema drift on the Rust side; the tool's integration tests catch
  input-API drift on the JS side.
- **Schema grows again.** A future `v1` role addition re-creates today's
  `unknown_record` problem. Mitigation is procedural: the conformance
  corpus README now documents the full role set, and any upstream bump PR
  must diff `trajectory-v1.schema.json` against the vendored expectation.
  Consider (out of scope) vendoring the schema file into the fixture tree
  so the diff is mechanical.
- **npm publishing is a new operational surface.** Scope squatting (fix:
  register the scope in Task 4 before any README references it), token
  hygiene, provenance. If this is judged too heavy, the fallback is
  shipping the tool as a checked-in single file invoked via
  `npx github:...` or plain `node` — less polished, zero publish surface;
  the plan's structure survives that substitution.
- **The picker offers the contributor's whole history.** `listTrajectories`
  enumerates everything. The tool converts only the explicit selection —
  one session per pick, no select-all — mirroring the companion spec's
  reasoning about bare `submit`.
- **`observation` content raises retained-text volume.** It passes through
  the same client-side redactor as everything else, but the redactor was
  tuned on user/assistant/tool text (the 2026-07-25 design accepted the
  analogous risk for reasoning). Called out for review rather than
  silently absorbed.
- **The Python wrapper looks like a Node-free path and may not be.** Its
  wheel embeds a JS bundle; whether it executes without a Node runtime is
  unverified. The plan does not depend on it either way; do not document
  it as an alternative until verified.
- **Slice B interaction.** The `.trajectory.json` suffix convention is
  load-bearing in Task 5/7 and comes from an approved-but-unimplemented
  spec. If Slice B changes its suffix rule, the tool's default filename
  must follow; keep the constant in one place in the tool.

## 5. Evidence index (all fetched 2026-08-27)

- npm registry `@letta-ai/trajectory`: versions 0.1.0-0.3.0, every
  version `bin: null`, zero runtime deps, `engines.node >= 20`.
- 0.3.0 tarball: `dist/adapters/` (14 adapters incl. `gemini-cli/`,
  `cursor/`, `copilot-cli/`, `opencode/`, `droid/`, `omp/`, `atif/`);
  `dist/listing.js` (ten listable sources, five export-only);
  `dist/python-cli.js` (versioned stdin/stdout JSON bridge);
  `schema/trajectory-v1.schema.json` with `system` and `observation`
  in the `record.oneOf`.
- PyPI `agent-trajectory` 0.3.0 description: library API, 14-adapter
  table, `listTrajectories` docs, "regenerates the JavaScript runtime
  embedded in the Python wheel."
- This repo: `README.md:346-350` (the broken command);
  `crates/trace-commons-contributor/src/source/trajectory.rs` role
  dispatch (~line 122) rejecting unknown roles whole-file;
  `SessionEvent`/`SessionEventKind` in `source/mod.rs:82-104` (Opaque may
  carry content); both fixture trees lacking any system/observation case.
