# One-click submit, and submitting a whole project

Status: design, approved 2026-08-20. Not yet implemented.

## The problem

A contributor cannot send a queued session without opening it. The queue row
offers `Look inside` and `Not this one`; there is no way to say yes from the
row, and no way to say yes to everything in a project.

That is more than a missing button. `preview` is where the redacted envelope
is built, persisted, and pinned to the entry via `previewed_envelope_digest`
(`daemon/queue.rs:433`). The uploader rebuilds the envelope at send time and
compares it to that pin; a mismatch re-offers the entry rather than uploading
it (`daemon/uploader.rs:22`). An approval with no pin therefore has no
artifact behind it, and fail-closes. Looking is not merely encouraged by the
UI -- it is how the bytes come to exist.

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

> Sent -- scrubbing removed 4 things, 1 flagged.  [Undo]

The signal follows the click and precedes the send. Nothing has left the
machine while the toast is up; `Undo` is the existing revoke path, which
clears the pin. A confirmation dialog was considered and rejected: it makes
the common case two clicks to show information the hold window can show
anyway, and the hold is already implemented and already undoable.

Bulk is the same gesture at the project level:

> Sent 47 sessions from frobnicator -- scrubbing removed 213 things, 3
> flagged.  [Undo]

**Flagged entries are included, not held back.** This was decided
deliberately. The alternative -- excluding them and saying "3 need a look" --
keeps the reason-to-look attached to the sessions worth looking at. It was
rejected in favour of doing what the contributor asked.

The consequence belongs in writing: high-risk submissions are quarantined
server-side, the quarantine queue has never been worked, and a one-click bulk
submit is the most efficient way yet built to add to it. Whoever picks up that
backlog should know the volume has a new source.

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
  rather than the re-offer it fail-closes into today. This is the whole
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
