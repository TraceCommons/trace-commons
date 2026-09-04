# Oversized traces: what the ceilings actually are, and what a contributor sees

Status: findings record, written 2026-08-21 against `main` of that day. The
measurements here are the source of the 3.4:1 raw-to-envelope ratio and the
7% oversized-session figure cited by the 2026-09-02 witness docs. The design
options were never decided; they are kept as written, not as a proposal.

Investigation findings for the question "can we flag in the UI traces that are
too big to upload?". No code was written. Every claim below carries a
`file:line` from the tree at branch `oversize-trace-findings`.

The short version is that the framing of the question does not match what the
code does. "Too big to upload" is very nearly an empty set. The losses that
actually happen to contributors today are (in descending frequency) silent
partial *scoring* on the server, silently dropped subagent transcripts on the
client, and whole sessions that never appear in the queue at all.

---

## 1. Every size ceiling, disk to accepted

| # | Ceiling | Value | Side | Defined at | Applied at |
|---|---------|-------|------|-----------|-----------|
| 1 | Codex rollout budget | 64,000,000 B | client | `crates/trace-commons-contributor/src/source/codex.rs:334` | `codex.rs:344` |
| 2 | Claude Code group raw budget | 64,000,000 B | client | `crates/trace-commons-contributor/src/source/claude_code.rs:86` | `claude_code.rs:733` (`apply_group_budget`), called from `claude_code.rs:899` |
| 3 | Envelope cap | 16,000,000 B | client | `crates/trace-commons-protocol/src/trace_contribution.rs:77` | `crates/trace-commons-contributor/src/envelope.rs:256` (pre-redaction), `envelope.rs:273` (post-redaction), `crates/trace-commons-contributor/src/daemon/approved_envelope.rs:85` / `:100`, `crates/trace-commons-contributor/src/daemon/ipc.rs:1535` |
| 4 | Privacy-filter sidecar input | 16,000,000 B (= #3) | client | `trace_contribution.rs:88` | `trace_contribution.rs:2182`; env override `TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES` at `trace_contribution.rs:2409` |
| 5 | Privacy-filter sidecar stdout | 32,000,000 B (= 2 x #4) | client | `trace_contribution.rs:93` | `trace_contribution.rs:2183` |
| 6 | NEAR AI classify window | 20,000 B | client | `crates/trace-commons-protocol/src/privacy_filter_near_ai.rs:27` | `privacy_filter_near_ai.rs:220` |
| 7 | Ingest request body | 20,971,520 B (#3 + 4 MiB) | **server** | `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:194` | `trace-commons-ingest.rs:7524` (`DefaultBodyLimit::max`) |
| 8 | Gate chunk cap | 16 chunks x 2048 target tokens x 4 chars/token ~= **131,072 chars** | **server** | `trace-commons-ingest.rs:271` / `:275`; `crates/trace-commons-gate-enclave/src/chunker.rs:15`, `:33` | `chunker.rs:160-166` (`finalize_plan`) |
| 9 | Account content read-back | 20,971,520 B (#3 + 4 MiB) | **server** | `trace-commons-ingest.rs:14331` | `trace-commons-ingest.rs:14678` |

Ordering between #3, #7 and #9 is held by compile-time asserts
(`trace_contribution.rs:95`/`:97`, `trace-commons-ingest.rs:202`,
`trace-commons-ingest.rs:14336`), so the historical 1.5 MB-vs-2 MiB drift the
doc comment describes cannot recur along that axis.

Nothing about any of these numbers appears in `docs/trace-commons.md`,
`docs/trace-commons-storage.md`, or `README.md` — verified by grep for
`16 MB`, `16_000_000`, `64 MB`, `too large`, `byte budget`. The only
contributor-facing documentation of a size effect is
`docs/contributor-daemon-ipc-v1_1.md:514-519`.

## 2. What actually happens at each ceiling

**#1 Codex rollout > 64 MB — the session vanishes, with no counter anywhere.**
`load_session` stats the file and `bail!`s (`codex.rs:344-347`). The watcher's
only call site is `let Ok(transcript) = source.load(session_ref) else { return; };`
(`crates/trace-commons-contributor/src/daemon/watcher.rs:460-462`) — a bare
`return` that does not increment `out.report.ignored` or anything else. No
queue entry, no outcome count, no log the contributor can see. The adapter's
own comment (`codex.rs:331-333`) puts this at 10 of 3,066 rollouts (0.3%) on
the measured corpus, so it is rare but not hypothetical.

**#2 Claude Code group over budget — largest subagents dropped, counted.**
`apply_group_budget` (`claude_code.rs:733-766`) sorts members by size
descending (tie-broken on file name, so the choice is deterministic) and drops
until the total fits. **The parent is never dropped** — the loop only ranges
over `members` — so a parent file alone above 64 MB is read whole regardless.
The drop count lands on `SessionTranscript::subagents_dropped`
(`claude_code.rs:982`) and is also written into a structural `subagent_group`
marker event in the envelope itself (`claude_code.rs:924-931`), so the trace
that is uploaded says it was trimmed. It reaches the queue entry at
`watcher.rs:513` and crosses IPC at `daemon/ipc.rs:637`.

**#3 Envelope cap — two different outcomes depending on which path you are on.**

- *CLI / auto-upload (armed project) path.* `submit_one` checks the raw
  contribution before the expensive redaction pass (`submit.rs:530-533`) and
  the finished envelope after (`submit.rs:554-560`, `:643-647`, `:683-686`),
  producing `SubmitOutcome::Refused { reason_label: "session-too-large" }`
  (`submit.rs:129-136`). The uploader maps that to `UploadDecision::Refused`
  (`daemon/uploader.rs:224`) and the daemon writes
  `QueueState::Refused` + `reason_label` onto the entry
  (`daemon/mod.rs:635-640`). This is persisted.
- *Daemon preview/approve path.* `build_preview` deliberately does **not**
  size-check; `approved_envelope::save` does (`approved_envelope.rs:85`), so
  the pin silently declines and `approve` re-measures
  `summary.would_send_bytes` to give the entry the fixed label
  `envelope-too-large` (`ipc.rs:1533-1538`). That label is returned in the
  `approve` response's `skipped` array **and nowhere else** — the entry is
  `continue`d over and stays `Pending`. It is therefore re-offered forever and
  will fail identically every time, with no persistent record. Contract test:
  `crates/trace-commons-contributor/tests/daemon_ipc_contract.rs:1703-1740`.

**#4/#5 Privacy-filter caps.** Equal to the envelope cap by construction, so an
envelope that passes #3 cannot fail these. A failure propagates with `?` and
surfaces as the generic `redaction-failed` label (`submit.rs:534-538`).

**#6 NEAR AI window.** Not a ceiling — a chunking window
(`privacy_filter_near_ai.rs:215-220`). It bounds request size to a hosted
endpoint that 502s above ~30 KiB (`privacy_filter_near_ai.rs:23`), not trace
size.

**#7 Ingest body limit.** A 413 from axum's `DefaultBodyLimit`. Unreachable for
any envelope the contributor built, by the compile-time assert at
`trace-commons-ingest.rs:202`. If it were ever hit, the client would report it
as a generic `http-failure`, *not* `session-too-large` — the only paths that
produce that label are local size checks and the claim-remint retry
(`submit.rs:1085`).

**#8 Gate chunk cap — silent truncation, and the one that actually bites.**
`finalize_plan` (`chunker.rs:160-166`) does `texts.truncate(cap)` and records
`chunks_capped: true` plus `dropped_chunk_count`. **The trace is accepted and
stored in full; only the scoring sees a prefix.** `chunks_capped` is carried
into the gate decision (`crates/trace-commons-server/src/trace_gate_service.rs:634`)
and persisted (`trace-commons-ingest.rs:48798`,
`crates/trace-commons-server/src/db/trace_corpus_pg.rs:5861`). Grepping the
whole ingest binary for `chunks_capped` returns exactly six hits, all writes or
test fixtures — **it is never returned in any contributor-facing response**.

**#9 Account read-back.** Over-ceiling collapses to a generic 413 "trace
content unavailable" with a hash-only `read_too_large` audit row
(`trace-commons-ingest.rs:14678-14700`). Unreachable by the assert at `:14336`.

## 3. When is each ceiling knowable?

**Knowable from a stat, before anything is read, and already on the wire.**
`SessionRef.size_bytes` for claude-code is the *whole group* total —
`claude_code.rs:409-411` folds every member's stat'd size onto the parent's —
and it is computed *before* `apply_group_budget` runs. It reaches
`QueueEntry.size_bytes` (`watcher.rs:486`) and crosses IPC at `ipc.rs:621`.
So `size_bytes > 64_000_000` is an **exact** predictor of ceilings #1 and #2,
because both of those are themselves decided on stat'd sizes. No cost at all.

**Knowable after load, exactly, and already on the wire.**
`subagents_dropped` is ground truth about what will be sent, decided at load
time precisely so preview and upload describe the same bytes
(`claude_code.rs:72-78`). On the queue entry at `ipc.rs:637`; on the preview
summary at `crates/trace-commons-contributor/src/daemon/preview.rs:579`.

**Only knowable after the full redaction + privacy-filter pass.**
`would_send_bytes` is `envelope_size(&envelope)` at
`preview.rs:546-547`, i.e. after `redact_to_envelope`. There is no cheap
approximation: even the "cheap" pre-check `raw_contribution_size_ok`
(`envelope.rs:248-259`) requires parsing the whole session and building the raw
contribution first. **So a cheap pre-flight flag for ceiling #3 does not
exist.** The only honest cheap proxy is raw size, and the raw-to-envelope ratio
is a single observation — 42 MB raw to a 2.8 MB envelope, roughly 15:1
(`claude_code.rs:64-68`) — with no measured spread.

**Not knowable client-side at all.** Ceiling #8 depends on the operator's
`TRACE_COMMONS_GATE_CHUNK_*` environment (`trace-commons-ingest.rs:260-266`),
is evaluated after decryption inside the gate, and is never reported back.

## 4. What the UI shows today

Wire shape for a queue entry is `entry_value`, `ipc.rs:614-638`:
`entry_id, session_hash, source, project_id, project_label, size_bytes,
discovered_at, state, reason_label, attempts, retry_after, submission_id,
subagent_count, subagents_dropped`.

A structural fact that shapes all of this: **`Refused` entries never reach any
client as entries.** `list_pending` (`ipc.rs:658-661`) and the `snapshot` event
(`ipc.rs:525-528`) both build from `queue.pending()`
(`crates/trace-commons-contributor/src/daemon/queue.rs:349`), which is
`QueueState::Pending` only. Refused/Expired/Superseded/Dismissed are reachable
only as an aggregate count-by-`reason_label` through `queue_outcome_counts`.
This is documented in-tree at
`crates/trace-commons-contributor-gtk/src/ui/queue.rs:383-391`.

### `size_bytes`

- **macOS**: decoded (`macos/Sources/TraceCommonsApp/Models.swift:34`, `:48`),
  formatted by `Format.bytes` (`Views/MenuBarView.swift:201-207`), but used
  **only** as a per-project sum in the menu bar (`AppModel.swift:166`, rendered
  `MenuBarView.swift:128`). The queue card never shows the entry's own size.
- **Windows**: shown per card. `QueueEntryViewModel.cs:164` (`SizeText`),
  formatter `:190-213`, bound under the eyebrow "SESSION ON DISK" at
  `MainWindow.xaml:763-766`.
- **Linux GTK**: decoded (`crates/trace-commons-contributor-gtk/src/model.rs:94-98`)
  and a `human_bytes` formatter exists (`model.rs:446`), but every UI call site
  uses *preview* numbers (`ui/queue.rs:654`, `:900`; `ui/preview.rs:697`). The
  only place `entry.size_bytes` is rendered anywhere in the crate is the debug
  probe (`src/bin/probe.rs:81`). The card shows no size until a preview lands.

### `reason_label`

**No front end shows a per-entry refusal reason** — a direct consequence of
`pending()`-only listing.

- **macOS**: field decoded (`Models.swift:37`, `:51`) with zero read sites in
  any view. The user path is the `queue_outcome_counts` rollup
  (`AppModel.swift:530-531`, `DaemonClient.swift:174-179`) rendered by
  `NotOfferedDisclosure` (`Views/QueueView.swift:90`, `:641-694`). Its mapping
  (`HealthCopy.swift:168-181`) covers 9 labels and does **not** include
  `envelope-too-large`; unmapped labels render as the bare word "Held".
- **Windows**: shown verbatim as the raw wire label
  (`QueueEntryViewModel.cs:159-162`, bound `MainWindow.xaml:780-783`) — almost
  always empty since only Pending entries are listed. `State` is likewise raw
  (`QueueEntryViewModel.cs:154`). The rollup only tidies hyphens to spaces
  (`ViewModels/HistoryViewModel.cs:498-521`), so a contributor literally sees
  "envelope too large".
- **Linux GTK**: best of the three. Rollup at `ui/queue.rs:392-421` with a
  plain-language mapping at `copy.rs:1027-1045` — but that table covers 6
  labels and `envelope-too-large` is not one of them, so it collapses to
  "Nothing was sent." with no reason.

The three vocabularies do not agree with each other.

### The approve-time skip label

This one *is* handled, identically and well, by all three: `envelope-too-large`
maps to "too large to send" in the post-approve toast —
`crates/trace-commons-contributor-gtk/src/copy.rs:1420-1427`,
`macos/Sources/TCShellCore/SubmitToast.swift:184`,
`windows/src/TraceCommons.Interop/SubmitToast.cs:194-202`. It is a transient
toast; the entry it refers to is still sitting in the queue as `Pending`.

### `subagents_dropped`

**Zero references in any front end.** `grep -rn "subagent" macos/ windows/src/
crates/trace-commons-contributor-gtk/ crates/trace-commons-contributor-ffi/`
returns only one Windows *test* fixture
(`windows/tests/TraceCommons.Interop.Tests/NativeRoundTripTests.cs:150-151`).
None of the three client models even decode the field: macOS
(`Models.swift:44-54`), Windows (`DaemonProtocol.cs:274-314`), GTK
(`model.rs:82-105`) all omit it, along with `subagent_count`.

The value is alive right up to `ipc.rs:631-637`, whose comment says "a card
showing it is the difference between a trimmed trace and a silently partial
one", and dies there. `docs/contributor-daemon-ipc-v1_1.md:514-519` says a
client **must** surface a non-zero value. **All three clients violate that
contract today.** (`subagent_count` has the parallel "should" at `:506-513`,
equally unmet.)

The C ABI is not the constraint: the only entry point is
`char* tc_call(tc_handle*, const char*, const char*)`
(`crates/trace-commons-contributor-ffi/include/trace_commons.h:322`), opaque
JSON in both directions. Every loss above is a client-side model omission.
(Incidental: the two header copies have diverged — the macOS copy at
`macos/Sources/CTraceCommons/include/trace_commons.h` is missing an entire
function's doc block present at canonical `:439-456`. Known issue, out of
scope here.)

## 5. Is the 16 MB cap ever actually hit?

Working the arithmetic through:

- The break-even redaction ratio for a 64 MB group to overrun a 16 MB envelope
  is **4:1**. The one observation on record is ~15:1
  (`claude_code.rs:64-68`). So for any *grouped* claude-code conversation the
  group budget bites first, by a wide margin, and the envelope cap is
  unreachable — **unless the true ratio for some session shape is worse than
  4:1**, which nothing in the tree measures. The in-tree test that touches this
  (`preview.rs:912-933`, a 114-member group) uses ~200-byte synthetic members
  and does not exercise a real 64 MB group, so it does not close the question.
- The one way a claude-code session *can* reach the envelope cap is a single
  parent file above the budget, because `apply_group_budget` never drops the
  parent. That case is read whole, redacted whole, and refused whole.
- Codex sessions can never reach it: anything above 64 MB is declined at load
  (`codex.rs:344`).
- Meanwhile ceiling #8 sits at roughly **128 KB of rendered event text** —
  about **125x smaller** than the envelope cap. The Codex corpus measurement in
  `codex.rs:332-333` puts the *median* rollout at 541 KB. Even after redaction
  strips JSONL structure, a large share of ordinary traces are being scored on
  a prefix only, and nothing tells the contributor.

**So the reasoning in the original framing holds, and then goes further.**
"Too big to upload" is close to an empty set. Ranked by how often a contributor
actually loses something:

1. Most traces are only **partially scored** (#8). Server-side, recorded, never
   reported. This is a credit-fairness issue, not a UI-flag issue.
2. Large Claude Code conversations **lose subagent transcripts** (#2). The
   signal exists end to end, the IPC contract mandates surfacing it, and no
   client does.
3. Codex rollouts over 64 MB **vanish with no trace at all** (#1). ~0.3%.
4. Genuinely oversized envelopes (#3) are either a transient toast plus an
   entry that will fail forever, or a `Refused` state no client lists.

## What the user-visible problem actually is

Not "some traces are too big to upload and we should warn about it". It is:

> **Traces are being silently trimmed in three different places, and the
> contributor is never told — even where the daemon already computes and ships
> the exact number.**

The most defensible reading of "flag traces that are too big" is therefore
*"flag traces that were trimmed"*, and the cheapest, most honest instance of
that is already sitting unread on the IPC wire.

---

## Design options

### Option A — Surface what already crosses the wire (recommended)

Two narrow client-side changes; no protocol change, no new daemon state.

1. All three shells decode and render a non-zero `subagents_dropped` on the
   queue card and in the preview sheet. This is already a `must` in
   `docs/contributor-daemon-ipc-v1_1.md:514-519`, so this is closing a known
   contract violation rather than adding a feature.
2. macOS and GTK render the entry's own `size_bytes` on the card, as Windows
   already does (`MainWindow.xaml:763-766`). Both already have the formatter
   (`Format.bytes`, `human_bytes`); neither calls it on the entry.

Trade-offs. Fixes loss (2). Does nothing for (1) or (3). Both numbers are exact
facts about the bytes that will actually be sent, so nothing here can lie. The
only real cost is copy: "N delegated transcripts left out to fit" has to be
written three times in three vocabularies that already disagree
(`HealthCopy.swift`, `HistoryCopy.cs`, `copy.rs`), and getting it wrong is
worse than saying nothing.

### Option B — Also make the two invisible refusals visible

Adds to A. Two independent pieces, each small:

- **B.1** Persist the approve-time skip. When `ipc.rs:1535` sees
  `would_send_bytes > MAX_ENVELOPE_BYTES`, also set `QueueState::Refused` with
  `reason_label = "envelope-too-large"` instead of leaving the entry `Pending`.
  Today that entry is re-offered forever and fails identically every time.
  Trade-off: `Refused` is terminal, so a contributor with a genuinely large
  session loses the card rather than being able to retry after (say) trimming
  the session — but retry cannot succeed today either, so this only makes the
  existing dead end legible. Also requires adding `envelope-too-large` to the
  three rollup copy tables (`HealthCopy.swift:168-181`, `copy.rs:1027-1045`,
  `HistoryViewModel.cs:498-521`), where it is currently absent in all three.
- **B.2** Stop `watcher.rs:460` swallowing a source load failure. At minimum
  increment a counter in the pass report so a >64 MB Codex rollout is
  *countable*. Trade-off: today a load failure is genuinely ambiguous
  (vanished file, unreadable, over budget) and the watcher deliberately treats
  them alike; distinguishing them means the source trait has to report *why*,
  which is a wider change than it looks. A no-new-error-type version is to
  surface the pre-load stat instead: `size_bytes > 64_000_000` is already
  known at discovery.

### Option C — Report gate chunking back to the contributor

Add `chunk_count` / `chunks_capped` to the account trace read-back or status
response so a contributor can see "only the first N of this trace was scored".

Trade-offs. This addresses the *largest* real loss, and the data is already
persisted (`trace_corpus_pg.rs:5861`). But it is a credit-model decision, not a
UI flag: it exposes gate internals to contributors, it invites disputes about
partial credit, and it is server-side work in the credit pipeline rather than
client work. It also cannot be a *pre-flight* flag under any design — the
contributor cannot know the operator's chunk configuration. Recommend
explicitly deferring it into the credit-pipeline queue rather than folding it
into a UI change.

### Recommendation

**Option A now. B.1 as a small follow-up. C deferred to the credit pipeline
with an explicit decision recorded.**

Rationale: A is the only option where the flag is exactly true by
construction, needs no new state, no protocol version bump, and no new
estimate. It also closes a documented `must`. Everything else in this
investigation either requires a measurement we do not have (the raw-to-envelope
ratio) or belongs to a different problem (credit).

**Explicitly not recommended:** a pre-flight "this trace may be too large to
upload" warning driven by raw size. It cannot be honest. The only ratio
evidence in the tree is a single 15:1 observation, the break-even is 4:1, and a
warning that fires on a trace that then uploads fine trains contributors to
ignore it.

---

## Open questions for the user

1. **Is "flag traces that were trimmed" the feature you want**, given that
   "too big to upload" is close to an empty set? This reframing is the main
   finding and everything below depends on the answer.
2. **Is partial scoring (#8, ~128 KB of scored text against a 541 KB median
   session) a known and accepted property of the gate**, or is it a surprise?
   If the latter, that is a much bigger issue than any UI flag and should be
   triaged separately. Related: does the credit model already account for
   `chunks_capped`?
3. **Should `subagents_dropped` be shown on the card, in the preview sheet, or
   both?** The card is where the consent decision is taken; the preview sheet
   is where the detail lives.
4. **Should B.1 make an oversized entry terminal (`Refused`)**, or is a
   permanently re-offered `Pending` entry preferable because a future
   larger cap would let it succeed?
5. **Do we want to measure the real raw-to-envelope ratio** before anything
   depends on it? One observation currently backs a 64 MB constant, a 16 MB
   constant, and the claim that the two cannot collide. A calibration pass over
   the pilot corpus would settle it.
6. **Is Codex's 0.3% total-invisibility (#1) worth fixing at all**, or is it
   acceptable that a pathological rollout is silently skipped?
7. **Who owns unifying the three refusal-reason vocabularies?** macOS drops
   unknown labels to "Held", GTK to "Nothing was sent.", Windows echoes the raw
   wire label. Any new label — including `envelope-too-large` — has to be added
   in three places or it degrades differently on each platform.
