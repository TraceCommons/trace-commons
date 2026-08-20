# One-click submit, and submitting a whole project

Status: design, approved 2026-08-20. Not yet implemented.

## The problem

A contributor cannot send a queued session without opening it. The queue row
offers `Look inside` and `Not this one`; there is no way to say yes from the
row, and no way to say yes to everything in a project.

That is more than a missing button. `preview` is where the redacted envelope
is built, persisted, and pinned to the entry via `previewed_envelope_digest`
(`daemon/queue.rs:433`). The uploader compares a rebuild against that pin; a
MISMATCH re-offers the entry rather than uploading it (`daemon/uploader.rs:22`).

But an ABSENT pin is not a mismatch, and does not fail closed. Corrected
2026-08-20, after this design was written on the opposite assumption:
`approved_envelope_for` returns `Ok(None)` when there is no pin
(`uploader.rs:191`), `use_approved_envelope(None)` stores that
(`uploader.rs:291`), and `submit.rs:518` takes the `None` arm and builds a
fresh envelope and sends it.

So an approval with no pin uploads bytes nobody was shown, and reports
success. That is a stronger reason for this work than the one this spec
originally gave: it is not only that looking is how the bytes come to exist,
it is that approving without them already sends something unseen.

## What this changes

**The daemon does the work.** `approve` stops meaning "mark this approved" and
starts meaning "ensure a pinned envelope exists, then approve". Where a
preview already ran, nothing changes: the pin is already there. Where it did
not, `approve` builds the envelope, persists it through
`daemon::approved_envelope::save`, records the digest, and only then marks the
entry.

This lives in the daemon and not in three shells. Envelope construction is
already the daemon's job, the pin invariant is already its rule, and a client
driving a preview it never renders in order to obtain a side effect is the
kind of arrangement that drifts apart across platforms. The macOS, Windows and
GTK shells each gain a button; none of them gains a concept.

**`approve` gains a project filter.** It accepts one entry id or `all: true`
today (`daemon/ipc.rs:808-830`). It gains `project_id`, selecting that
project's pending entries. "Everything in this project" then means one thing
everywhere, and cannot come to mean three.

**`approve` returns the signal.** What redaction removed, how many entries
carried a flag, how many were approved, how many were skipped and why. The
client renders that without a second call.

## The interaction shape

One click. The row's `Submit` builds, pins, approves, and raises a toast for
the length of `approval_hold_secs`:

> Approved -- scrubbing removed 4 things, 1 flagged.  [Undo]

The signal follows the click and precedes the send. Nothing has left the
machine while the toast is up; `Undo` is the existing revoke path, which
clears the pin. A confirmation dialog was considered and rejected: it makes
the common case two clicks to show information the hold window can show
anyway, and the hold is already implemented and already undoable.

Bulk is the same gesture at the project level:

> Approved 47 sessions from frobnicator -- scrubbing removed 213 things, 3
> flagged.  [Undo]

**Flagged entries are included, not held back.** This was decided
deliberately. The alternative -- excluding them and saying "3 need a look" --
keeps the reason-to-look attached to the sessions worth looking at. It was
rejected in favour of doing what the contributor asked.

The consequence belongs in writing: high-risk submissions are quarantined
server-side, the quarantine queue has never been worked, and a one-click bulk
submit is the most efficient way yet built to add to it. Whoever picks up that
backlog should know the volume has a new source.

## The toast: normative copy

Every shell renders the same sentence from `approve`'s response. These strings
are the contract, not an illustration -- transcribe them, do not reword them in
one client. The vocabulary deliberately matches the count-dependent scrubbing
line the queue row already uses (`gtk/src/copy.rs`), because it is the same fact
said in fewer words.

Built in four clauses, in order. Clauses 3 and 4 appear only when non-zero.

**1. What happened.** Corrected 2026-08-20: this clause said "Sent", and that
was false. `copy.rs:192` states the contract -- "The watcher sends approved
sessions on its next sweep. Undo works until the sweep starts." At toast time
nothing has left the machine; the approval is recorded and the send happens
later. A toast reading "Sent." while offering Undo contradicts itself, and this
product does not get to be careless about that sentence in particular.

> `approved == 1`  ->  **Approved.**
> `approved > 1`   ->  **Approved {n}.**
> `approved == 0`  ->  **Nothing approved.**

**2. What scrubbing did.** Always present, including when it did nothing: a
count of zero is a fact the contributor is owed, not an absence to omit.

> `0`  ->  **Scrubbing matched nothing.**
> `n`  ->  **Scrubbing removed {n}.**

Sum the values of the `redactions` map. Do not name categories in the toast --
the preview sheet is where a contributor sees which detector fired.

**3 and 4. What was flagged, and what was not.** One clause, comma-joined, each
half present only when non-zero:

> flagged only        ->  **{n} flagged.**
> skipped only        ->  **{n} not approved: {reasons}.**
> both                ->  **{n} flagged, {m} not approved: {reasons}.**

`{reasons}` is the distinct human labels below, comma-separated, in the order
listed here. Never the raw wire label, never an entry id -- an id in a toast is
noise a contributor cannot act on.

**Length is a constraint, not a preference.** The GTK shell targets libadwaita
1.2.2 deliberately (see `scripts/linux-build.Dockerfile`: an old, widely
deployed pair). Its `adw::Toast` is single-line, does not wrap, and has no
`custom-title` before 1.4. An earlier, wordier version of this copy was
camera-verified to TRUNCATE at its longest realistic form, which loses the
skip clause -- the half a contributor most needs. Every clause above is
therefore as short as it can be while staying a sentence. Do not re-expand it
for one shell's roomier surface: the shortest shell sets the budget.

**Undo** is offered when `approved > 0`, and only then. It is present for
`hold_secs` and maps to the existing `cancel` method, which now clears the pin
so the next submit rebuilds.

Worked examples:

> Approved. Scrubbing removed 4. 1 flagged.
> Approved 47. Scrubbing removed 213. 3 flagged.
> Approved 44. Scrubbing removed 213. 3 flagged, 3 not approved: too large to send.
> Nothing approved. Scrubbing matched nothing. 2 not approved: already decided.

**Submit is not the primary action.** `Look inside` stays the row's default and
keeps its emphasis; `Submit` sits beside it as a peer. This product's argument
is "that scrubbing is good and it is not perfect -- which is why you get to look
first", and making the shortcut the recommendation contradicts it. One click is
availability; primary styling is a recommendation, and only the first was asked
for.

**A defect this fixes, not only a feature.** `gtk/src/ui/preview.rs` currently
calls `offer_undo` on any `Ok` response, ignoring `approved`. That was correct
when every approval succeeded; it is wrong now that skips exist, because a
skipped entry reads to the contributor as sent, with an undo timer behind it.
The rule above -- Undo only when `approved > 0` -- is what corrects it.

## Errors

A build can fail: an unreadable session, or one over the 64 MB
`GROUP_RAW_BYTE_BUDGET`. In a batch, some entries can fail while others
succeed. The call reports approved, skipped, and why, by label rather than by
path -- audit rows and error strings stay hash-only or label-only, as
everywhere else here.

An entry whose state moved between the client listing it and the call
arriving is skipped rather than forced. `record_previewed_envelope` already
refuses anything not `Pending` (`queue.rs:437`), and that refusal is the right
one: an entry already approved has had its terms fixed.

## Testing

The properties live in Rust, next to the invariants they protect:

- An approval with no prior preview produces an upload the uploader accepts,
  rather than the silent rebuild-and-send it does today. This is the whole
  feature; if it holds, the buttons are decoration.
- The project filter selects exactly that project's pending entries -- not
  another project's, not entries in a terminal state.
- A partial batch reports honestly: approved and skipped counts sum to what
  was attempted, and no skip is silent.
- Undo within the hold leaves nothing sent and no pin behind.
- The returned counts match the envelopes actually built.

The shells get thin coverage of the thin parts: the row action calls approve
with an id, the project action calls it with a project id, the toast renders
what came back.

## Out of scope

Arming a project so it contributes without asking already exists as
`auto_upload`, gated behind its own confirmation. This feature is retroactive
-- it acts on what is queued now -- and deliberately does not change what
happens to sessions that arrive later. A contributor who wants that should
arm the project, which says so plainly.
