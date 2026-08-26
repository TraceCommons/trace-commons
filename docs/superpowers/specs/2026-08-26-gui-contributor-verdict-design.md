# Collecting a contributor verdict in the GUI shells

Design for capturing "did this session do what you asked?" in the macOS, GTK
and Windows contributor shells, and carrying that answer to
`OutcomeMetadata.task_success`.

Follows PR #421, which shipped the CLI half (`submit --outcome`) and
deliberately left the GUI shape undecided.

## The gap

`OutcomeMetadata` has modelled an outcome since v1. The contributor client
writes `OutcomeMetadata::default()` on every envelope, so `task_success` is
`Unknown` on every trace it has ever sent.

Nothing in a transcript answers the question. An agent that stops has not
thereby succeeded, and a harness that recorded no error has not thereby done
what was asked. Only the person who asked knows, so the answer is asked for
rather than inferred.

Two consumers care:

- `difficulty` (`trace_contribution.rs:1247`) scores 0.65 when
  `task_success` is `Failure` or `Partial`, against 0.35 otherwise. Note that
  `Partial` scores the same as `Failure` here, so the third state is not
  merely a softer failure.
- `canonical_error_outcome_representation`
  (`trace_contribution.rs:4905`) gates on `TaskSuccess::Failure | Partial`,
  which is dedup signal on exactly the failed traces issue #298 asked for.

Note that PR #420 independently supplies part of this: the same gate also
fires on `event.success == Some(false)`, which #420 begins populating from
failed tool results. #420 therefore covers any session with a failing tool
call. This design covers the sessions that failed without any individual
tool reporting an error, and it supplies `Partial`, which nothing else
reaches.

## Where the verdict is collected

On the approval control, applied to every entry that approval covers.

The verdict is recorded on the queue entry at the moment of approval and read
back by the uploader when it builds the envelope.

```
GUI shell (macOS / GTK / Windows)
   |  approve { all | project_id | entry_id, outcome: worked|partly|failed }
   v
daemon ipc.rs::handle_approve
   |  parse and validate; refuse an unrecognised value
   |  stamp onto every entry it approves, beside approved_scopes
   v
QueueEntry.approved_verdict: Option<String>      (new, #[serde(default)])
   |
   v
uploader.rs::approved_envelope_for
   |  load stored envelope; verify digest against previewed_envelope_digest
   |  THEN apply entry.approved_verdict to envelope.outcome.task_success
   v
upload
```

### Why not SubmitOptions.verdict

`SubmitOptions.verdict` -- the field PR #421 added -- reaches only envelopes
a run BUILDS. That is the CLI path.

The daemon does not build at upload time. `approved_envelope` stores the
redacted envelope the contributor was shown and the uploader sends exactly
those bytes, because re-deriving under `pii_filter = "near-ai"` returns
different spans for identical text: the earlier digest-pin design refused
the entry, cleared the pin, and never completed the primary consent path.
Redaction is deliberately not re-run in `approved_envelope_for`
(`uploader.rs:298`).

So a verdict routed through `SubmitOptions` would be silently dropped for
every GUI submission -- the feature would appear to work and record
`Unknown`. The verdict is instead applied to the loaded envelope after the
digest check passes, which is the same kind of post-redaction mutation
`apply_granted_scopes` (`submit.rs:1026`) already performs.

Both paths are needed and neither is redundant:

- CLI, fresh build: `SubmitOptions.verdict`, already shipped in #421.
- Daemon, stored envelope: `entry.approved_verdict`, applied after the
  digest check.

### The digest pin now describes the previewed bytes, not the sent bytes

This is the one deliberate deviation from "the upload sends precisely those
bytes", and it is bounded to `outcome.task_success`.

`approved_envelope`'s module doc currently states the stronger claim without
qualification. It must be amended in the same change, or it becomes false.
The digest re-check stays exactly as it is -- it is a consistency check on
this crate's own storage, and it must continue to run against the bytes as
stored, before the verdict is applied.

### Why not inside the preview

The preview is rendered before the contributor answers, so a verdict that
changes the envelope makes the bytes sent differ from the bytes shown by
exactly the outcome fields.

Rather than re-render (a second redaction pass per answer) or ask for the
verdict before the contributor has seen what is being sent, the outcome
fields are declared an approval-derived output: they are produced by the act
of approving, not by an input that existed beforehand. The shell says so at
the point of asking.

## The drift-guard asymmetry

`approved_verdict` must NOT be wired into `preview::input_fingerprint`, and
this is the design decision most likely to be "corrected" by someone later.

Its two neighbours on `QueueEntry` are drift guards. `approved_scopes` and
`approved_input_fingerprint` record envelope-determining inputs as of
approval so the uploader can refuse if either moved before it sent. For both,
`None` on an approved entry means "unknown, so re-ask" and fails closed.

`approved_verdict` is the opposite kind of thing. It is an output of the
approval act, not ambient configuration that could change underneath it, so
it cannot drift between approval and send. `None` means the contributor did
not answer, which is `TaskSuccess::Unknown`, and the entry submits normally.

Conflating the two would fail-close every unanswered approval, which is every
approval made before this feature exists.

The field's doc comment must state this, because every neighbouring field in
the struct reads the other way.

## Contract

Wire names are shared by the CLI flag and the IPC parameter so the two
surfaces cannot drift apart.

| Answer     | `task_success`        | `user_feedback`     |
| ---------- | --------------------- | ------------------- |
| `worked`   | `TaskSuccess::Success` | `UserFeedback::None` |
| `partly`   | `TaskSuccess::Partial` | `UserFeedback::None` |
| `failed`   | `TaskSuccess::Failure` | `UserFeedback::None` |
| absent     | `TaskSuccess::Unknown` | `UserFeedback::None` |

An unrecognised value is refused, not coerced to `Unknown`. A contributor who
typed something meant to say something. This extends the rule #421 already
applies to the CLI flag across the IPC boundary.

### The verdict writes task_success only

#421 sets both `task_success` and `user_feedback` from one answer, mapping
`worked` to `ThumbsUp` and `failed` to `ThumbsDown`.

Those are different questions. `task_success` is a fact about task
completion; `user_feedback` is satisfaction. They genuinely diverge: a run can
complete the task by a route the contributor dislikes, or fail at the task
while doing the right thing. Asserting both from one keystroke records a
satisfaction signal the contributor never gave.

`Partial` makes this concrete, because it has no honest thumb.

The verdict therefore writes `task_success` and leaves `user_feedback` as
`None`. That keeps `user_feedback` available for a real satisfaction control
later, including the `Correction` variant once the privacy pipeline can carry
contributor prose.

## Bulk approval

`approve` accepts `all`, `project_id` or `entry_id`, with that precedence.
A verdict supplied with a bulk approval applies to every entry that approval
covers.

This is a deliberate coverage-over-precision tradeoff, taken knowingly. The
cost is real and should be recorded: a twelve-session batch marked `worked`
from one click is twelve per-session assertions the contributor did not
individually make. If the resulting data proves noisy, the narrower rule is
to accept `outcome` only alongside `entry_id` and refuse it for `all` and
`project_id`, leaving bulk approvals `Unknown`.

## Changes required in PR #421

#421 should not merge as written. Three changes, all small:

1. `ContributorVerdict` gains a `Partly` variant, and `parse` accepts
   `"partly"`.
2. `outcome()` stops writing `user_feedback` and returns `UserFeedback::None`
   for every verdict.
3. `a_verdict_reaches_the_outcome` asserts the absence of a feedback signal
   rather than the thumbs mapping.

## Shell scope

Three surfaces:

- GTK (`crates/trace-commons-contributor-gtk`), Rust, builds locally.
- macOS (`macos/`), Swift. Covered by the `macOS app tests` CI job.
- Windows (`windows/`), C#. Covered by the `windows contributor` CI job.

Only the GTK shell can be compiled in the authoring environment. The Swift and
C# controls are verifiable on a pull request through those CI jobs but not at
the desk, so they land second: the daemon, IPC and GTK layer first, then the
macOS and Windows controls once CI can check them.

## Testing

The daemon layer carries the real assertions:

- A verdict reaches the entry on a single-entry approve and on a bulk approve.
- An unrecognised value is refused at the IPC boundary.
- Absence yields an outcome identical to `OutcomeMetadata::default()`.
- A verdict moves neither consent flag, extending #421's
  `a_verdict_declares_no_content` to the daemon path.
- A `None` `approved_verdict` on an approved entry still submits. This is the
  test that guards the drift-guard asymmetry above, and it is the one that
  fails if someone later folds the field into `input_fingerprint`.
- A verdict reaches an entry that uploads a STORED envelope. This is the
  regression test for the mechanism error this spec originally made: routing
  the verdict through `SubmitOptions` passes every fresh-build test and drops
  the verdict on exactly the path the GUI uses. The test must seed a stored
  approved envelope with a pinned digest, not build one inline.
- The digest re-check still refuses a stored envelope whose bytes were
  tampered with, with a verdict present. The verdict must be applied after
  that check, never before it.

## Non-goals

- No free-text correction box, so the `user_correction_value` weight
  (`trace_contribution.rs:1255`, keyed on `human_correction`) stays dead.
  Note the blocker here is narrower than it is sometimes stated: redaction
  already scrubs `human_correction` (`:3446`, `:3537`), so the missing pieces
  are a consent decision and a UI surface, not a privacy pipeline.
- No per-row verdict state in the queue UI.
- No change to redaction, to the preview pipeline, or to the envelope schema.
  `OutcomeMetadata` already models everything written here.
