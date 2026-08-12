# The consent moment, rebuilt

The path from "look inside" to "contribute" is the only thing between a
private transcript and a public commons. An independent design review named
four defects in it. All four were real. This is what was checked, what
changed, and what the new gate does and does not establish.

## The four defects, verified against the code

1. **Return was bound to `Contribute`.** `PreviewSheet.footer` carried
   `.keyboardShortcut(.defaultAction)` on the approve button. The one
   irreversible action in the product was one keystroke from a hand resting
   on the keyboard. **Real.**

2. **`Contribute` enabled the instant metadata loaded.** The only guard was
   `.disabled(summary == nil)`. A contributor could approve without ever
   opening "Exactly what would be sent". **Real.**

3. **Chained approvals.** `advance()` closed the preview handle, cleared the
   state, and put the next waiting session into the same sheet with
   `Contribute` under the same pixels. A second click -- or a second Return,
   given defect 1 -- sent a different transcript than the one just looked at.
   **Real.**

4. **The undo affordance rendered behind the sheet.** `UndoBar` is a child of
   `QueueContent`. Because the sheet stayed open across `advance()`, the sole
   recovery path was underneath a modal for the whole chained-approval run.
   **Real.**

Also reported and also real: `QueueRow.actions` bound `.defaultAction` to
"Look inside", so every row in the queue registered the shortcut. With two
rows waiting, neither row could say what a keystroke would open. "Look
inside" is not irreversible, so this was an ambiguity rather than a hazard --
but it is gone.

## The read gate: first screen plus explicit acknowledgement

`Contribute` now enables only when all three hold:

- the preview summary has loaded (as before);
- the redacted transcript has actually been on screen -- `TranscriptTab`
  calls back on appear, and the sheet records it;
- the contributor has ticked an acknowledgement themselves.

In the footer, above the buttons and below the canonical scrubbing sentence,
two checkbox lines sit in the state they are really in:

```
[ ] Open "Exactly what would be sent" and look at it.
[ ] I have looked at what would be sent, and I understand scrubbing is
    pattern-based and may have missed something.
Contribute stays off until both are done. Looking at the first screen is what
this checks -- it cannot check that you read all of it, and it does not claim
to.
```

The first line is a button: clicking it switches to the transcript tab, and
its box fills once the tab has rendered. The second is disabled until the
first is satisfied, so the acknowledgement cannot be made before there is
anything to acknowledge. Both boxes change shape as well as colour, so the
state survives greyscale and colour-blindness.

Neither flag is persisted, neither is pre-ticked, and both are plain
`@State` on a sheet that now handles exactly one session -- so every entry
starts the gate from zero. An acknowledgement that carried over would not be
an acknowledgement.

**What this gate establishes, honestly.** That the redacted body was put in
front of the contributor and that they said out loud what scrubbing does not
guarantee. It does not establish that they read it. A stricter
paged-and-scrolled gate was considered and rejected: real traces on this
pilot run to 169 KB, that is a long drag, and a gate anyone can defeat by
throwing the scrollbar at the end verifies nothing while reading, to
everyone downstream, as though it verified reading. The disabled-state copy
says this in the interface, not only here.

**Why hand-drawn checkboxes.** `ImageRenderer` will not rasterize
NSView-backed controls, so a `Toggle` here would appear as a yellow
placeholder in every verification screenshot of the most safety-relevant
control in the product -- while being perfectly fine in the running app. The
boxes are SwiftUI `Image(systemName:)` inside plain buttons, the same
treatment `ConsentScopesView` already uses for permissions, so what runs is
what gets captured. (The yellow bar still visible in the captured preview
sheet is the search `TextField`, which is NSView-backed and pre-existing.)

## Return

Return is now bound to exactly one thing in this shell: **Undo**, on the
recovery bar. The safe action is the default one.

- `PreviewSheet` -- `Contribute` has no keyboard shortcut.
- `QueueRow` -- "Look inside" has no keyboard shortcut.
- `UndoBar` -- `Undo` carries `.defaultAction`.

The onboarding screens (`OnboardingWelcome`, `OnboardingConnect`,
`OnboardingProjects`, `OnboardingPrivacyScan`, `ConsentScopes`,
`OnboardingDone`) still bind Return to their Continue buttons. Those are
reversible navigation steps, nothing leaves the machine on any of them, and
they were left alone.

## One sheet, one session

`advance()` is gone. `Contribute` and "Not this one" both call `dismiss()`,
returning the contributor to the queue. Three sessions are three deliberate
trips through the preview, not one sheet that keeps reloading under a
stationary button. The sheet's `remaining` list and its "N more after this"
line went with it -- with no chaining, that count was describing a flow that
no longer exists.

This is also what fixes defect 4 structurally rather than cosmetically: with
the sheet closed, the recovery bar at the head of the queue is on screen at
the moment it is needed.

## The recovery surface counts something real

The old bar showed `Sending <project>… [Undo] (4)`, counting a five-second
window down to zero and then vanishing.

That five seconds was invented here. The real deadline is the daemon's next
upload sweep: `drain_approved` claims everything in `Approved` on a poll
tick (`poll_interval_secs`, default 60). This process cannot see when that
tick lands -- the socket's `status` and `get_settings` views expose the
digest interval and the queue TTL but not the poll interval, and
`list_pending` returns only `Pending` entries, so an approved entry drops out
of everything the app can observe the moment it is approved. Counting down to
zero told a contributor the window had closed when it usually had not, and
said nothing about the case that actually matters: a sweep firing one second
after the click. The self-test has been recording exactly that race for a
while -- `after undo: back-in-pending=false` with "Too late to undo" in
`last action error`.

So the bar now counts UP from a real instant, and stays:

```
[clock] Approved northwind-billing. Still on this machine.   held 12s
The watcher sends approved sessions on its next sweep. This app cannot see
when that lands, so it does not pretend to count it down: undo works until
the sweep starts, and says so plainly if it is already too late.
[ Undo ]  [ Let it send ]
```

`Undo` is unchanged underneath -- `cancel`, which returns the entry to
pending, refused once the daemon has claimed it, with the existing plain
sentence when it loses the race. "Let it send" only puts the notice away; it
touches the daemon not at all. The display counter stops at 120s (`held
120s+`) so a ticker does not run for the life of the process, but the
affordance itself does not expire, because a recovery path that removes
itself while recovery is still possible is worse than no timer at all.

## Verification

- `cd macos && swift build` -- clean, from a clean baseline build before any
  edits.
- Screenshots regenerated in both appearances via
  `macos/scripts/run-demo.sh` (which always rebuilds now) and inspected. The
  preview sheet renders the gate correctly in dark and light: both boxes
  drawn, second line dimmed until the first is met, `Contribute` visibly
  disabled, `TC.primaryFill` / `TC.primaryLabel` untouched. The queue window
  is unchanged apart from the removed shortcut, which is not visible in a
  still.
- Self-test passes in both runs and contains no trace content: `opening
  prompt: chars=97 nonempty=true`, no prompt text, `undo=true` after approve.
  Nothing the self-test observes changed shape.

### Not verified

- The **satisfied** state of the gate, and the recovery bar itself, are not
  in any captured image. `DebugScreenshot.swift` renders the preview sheet
  from a fixture in its default state and raises no undo, and that file is
  out of bounds for this change. Both compile and both are covered by the
  self-test's approve/undo pass, but neither has been photographed and
  neither was clicked through by hand in this pass.
- No change was made to expose `poll_interval_secs` over the socket. Doing so
  would let the recovery bar name the worst-case deadline instead of
  describing it. That is daemon-side plumbing and was deliberately left for a
  separate change.
