# Ignore a project from the Waiting screen

## Problem

The Waiting screen groups queued sessions by project and offers `Submit all`
on each project header. There is no counterpart: a contributor looking at a
group of traces from a project they do not want to contribute has no way to
say so from where they are standing.

The capability exists — `ProjectMode::Ignore` ("Never offer sessions from
this project at all", `daemon/policy.rs:41`) is reachable from Settings on
all three shells and from onboarding — but not from the screen where the
decision is actually prompted.

### The defect underneath it

Surfacing the existing control would not be enough, because it does not do
what its name says.

`set_project_mode` touches the queue only to relabel project display names
(`daemon/ipc.rs`, the `known_keys` / `set_project_label` block). It never
removes entries. The watcher consults `ProjectMode::Ignore` at *offer* time
(`daemon/watcher.rs:434`, `:479`), so ignoring a project stops future
offers and leaves everything already queued sitting in front of the
contributor.

So today: ignore a project from Settings, return to Waiting, and the traces
are still there. This is the same shape as the dismissal defect fixed in
#396 — a decision the contributor made that visibly fails to take effect on
what is in front of them — and it is why the feature reads as missing even
though it ships.

## Non-goals

- Not a new IPC method. `set_project_mode` already carries this decision.
- Not a change to what `Ignore` means for future sessions.
- Not per-session ignore. That is `dismiss`, and #396 settled its semantics.
- Not a change to Settings or onboarding placement. They keep their
  controls; they simply inherit the corrected behaviour.

## Design

### 1. Daemon: purge on ignore

When `set_project_mode` sets a project to `Ignore`, every `Pending` entry
for that project moves to `Refused` with a new reason label, inside the same
critical section that sets the mode. The response reports the count.

```rust
pub const REASON_PROJECT_IGNORED: &str = "project-ignored";
```

**Scope: `Pending` only.** `Approved` and `Uploading` entries are left
alone. An approval is a decision the contributor already made about a
specific set of bytes under a specific set of consent scopes; a later
project-level preference does not silently retract it. This is the line
`Queue::cancel` already draws when it refuses to unwind an entry past
`Approved`, and the reasoning is the same — an undo racing an in-flight
upload is indistinguishable from data loss.

The visible consequence must be stated in the UI rather than discovered: a
project with three pending and one approved loses three cards and still
uploads one.

**This label must not be `REASON_DISMISSED`.** Since #396, that label is
path-keyed and permanent: `Queue::dismissed_at_path` suppresses a
conversation forever, before the load. If project-ignore reused it, then
un-ignoring the project via "Ask again" would silently restore nothing,
because every purged session would remain individually suppressed at its
path. `project-ignored` deliberately sits outside that path — a verdict
about a project's current queue, not a decision about each conversation —
so un-ignoring lets the watcher re-offer on its next pass. That is what
makes recovery real rather than nominal, and it is the same distinction
`REASON_TOO_LARGE` documents.

Because the daemon owns this, Settings, onboarding, and the CLI inherit it.
Ignoring from Settings today leaves the same orphaned cards; after this it
does not.

### 2. IPC

No new method and no breaking change. `set_project_mode` returns:

```json
{ "ok": true, "purged": 12 }
```

Existing clients that ignore the field keep working. `docs/contributor-daemon-ipc-v1_1.md`
gains the field and a note that `Ignore` now clears pending entries.

The confirmation dialog needs the count *before* it acts, so the UI computes
it from the queue it already holds and uses `purged` only to reconcile —
the two can differ if the queue moved between render and click, and the
response is the authority. When they differ the contributor is told, in a
line that lives in the same tested copy unit as the confirmation body; when
they agree nothing is said.

Un-ignoring is the other half of the same handler: leaving `Ignore` drops
that project's `project-ignored` rows so the watcher offers them again. It
has to remove them rather than re-state them, because a `Refused` row keeps
its path and observation and `Queue::unchanged_offer_at_path` matches a
non-live entry on that observation — deliberately, so a pipeline refusal is
not re-offered every poll. Left in place, the row silently suppresses the
re-offer of any session that never changes again, which is every finished
one. Dismissals and pipeline refusals are untouched by it.

### 3. The button

`ProjectQueueGroup`'s header gains an Ignore action beside `Submit all`.

**Visibility deliberately differs from `Submit all`.** `Submit all` hides at
`count == 1` because the row's own `Submit` already does the same thing at
the same scope. Ignore has no row-level equivalent — it is a statement about
the project, not about a trace — so it renders whenever the group renders,
single-entry groups included.

**Weight: secondary and destructive-toned, never primary.** It sits beside
an action that uploads the same traces. Two adjacent controls that operate
on one set of traces and do opposite things must not look alike.

### 4. Confirmation

Pressing Ignore opens a confirmation naming the count.

The action is recoverable, so a dialog is not obviously warranted — the
argument for it is the neighbour. `Submit all` and `Ignore project` sit
inches apart, act on the same traces, and do opposite things. A misclicked
Ignore is undoable from Settings; a misclicked `Submit all` is not. The
asymmetry is the justification, not the destructiveness.

Copy, per shell, in a tested unit:

- Button: `Ignore project`
- Title: `Ignore {project}?`
- Body: `This removes {N} waiting {trace|traces} and stops this project
  being offered. Nothing already submitted is affected. You can undo this in
  Settings.`
- Confirm: `Ignore project` / Cancel: `Cancel`

Singular and plural are written out rather than assembled. The last two
sentences are load-bearing: one bounds the blast radius, the other names the
way back, which is what allows the action itself to be quiet.

**When nothing is pending**, the body drops its first clause entirely and
reads `Stops this project being offered. Nothing already submitted is
affected. You can undo this in Settings.` A group can render with zero
pending entries — every remaining card approved or uploading — and
"removes 0 waiting traces" would be both wrong and alarming. The
confirmation still appears: the contributor is still changing a standing
preference, and a control that sometimes prompts and sometimes does not is
worse than one that always does.

The copy lives in a tested unit per shell — `copy.rs` in GTK, a
`SubagentCopy`-style helper on macOS and Windows — for the reason #398
established: three shells drift, and plural agreement is exactly what drifts
first.

### 5. Windows: guard the dialog

Issue **#316** — "Two unguarded `ContentDialog.ShowAsync()` calls can crash
the Windows app" — is open. This adds a third dialog. It is guarded from the
outset, and the two existing calls are fixed in the same change: the same
few lines, and leaving a known crash beside new code that reproduces it
would be indefensible.

## Testing

**Daemon**
- Ignoring purges `Pending` and reports the count.
- `Approved` and `Uploading` survive; a mixed-state project loses only its
  pending entries.
- The label is `REASON_PROJECT_IGNORED`, and specifically not
  `REASON_DISMISSED`.
- Un-ignoring lets the watcher re-offer a purged session. This is the test
  that proves recovery is real; without it the confirmation copy lies.
- A pipeline `Refused` entry in the same project is not disturbed.

**Shells**
- Copy unit per shell: plural agreement, the zero-waiting case (ignoring a
  project with nothing queued must not say "removes 0 traces"), and the
  reconciliation line (silent when the counts agree; names both when they
  do not).
- Windows: the new dialog is guarded.

Verified by code review rather than by a test, because no shell has a
harness that renders a group header: that Ignore is built at every group
size while `Submit all` is still built only above one, and that Ignore sits
to the right of `Submit all` on all three. The visibility rule is one
condition per shell in the header builder (`project_header` in GTK,
`group.count > 1` in `QueueView`, `ShowSubmitAll` / `ShowIgnoreProject` in
`QueueGroupViewModel`) and is read there.

macOS and GTK are verifiable locally. Windows XAML compiles only in CI —
that gets stated plainly in the PR rather than implied.

## Consequences

A contributor can decline a project from where the decision is put to them.
The existing Settings and onboarding controls start doing what they say.
And one more silent gap closes — after #396 (a decline that came back),
#397 (a refusal that left no record), #398 (a conversation trimmed without
saying so) and #399 (a session that vanished), this removes the last case in
the queue UI where a contributor's decision does not visibly take effect.
