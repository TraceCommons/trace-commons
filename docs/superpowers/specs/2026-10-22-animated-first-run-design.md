# Animated first run (macOS)

Date: 2026-10-22. Owner request: "code this up to replace the current FTUX"
after the 2026-10-21 design canvas
(https://claude.ai/artifact/7tWDBb9oCGtD3DVG775cHL, version 23).

## What it replaces

Ron's #1030 first run in a 450pt pane: Join, then Folders (Quick setup) or
Tools and Rules (Custom setup), then Uses. The native app now runs one flow
for everyone, full window, over one scene:

1. **Welcome**: the N drawn like a marker, "Welcome to Near AI", Get started.
2. **Tools**: the tool tiles burst out of the N into a ring, one sweep
   dismisses the tools not on this Mac, the rest re-space. One marker-drawn
   check per found tool (unticked is Not used, none start ticked); Continue
   and Customize wait for one tick. Customize grows the card up and opens
   the per-repo rules and past traces (Ron's Rules screen, in place): each
   repo's rule is the canvas's Ask / Automatic / Never control with no
   status dot, Automatic pickable before sign-in, and each past trace's
   switch sits before its date (Ron, 2026-10-22). The repos come from
   `list_projects` with `include_found`, so they show before any session
   has queued.
3. **Uses**: This Mac, the review gate and the commons, a soft orb
   travelling between them; the uses list and Ask / Automatic.
4. **Traces**: the watched tools' trace count filling a ring, and what the
   traces could earn: the count times an invite's range, or before one the
   core's default estimate of 3 to 8 points per accepted trace (owner,
   2026-10-22), under the same "An estimated ... not yet settled." line.
5. **Private AI**: the tools Private AI can route, the N as the hub, the
   switch.
6. **Join**: sign in with NEAR AI, last. Start sharing finishes the run.

A step thread names Tools to Join; the travelled stretch glows. The N and
the tool tiles are shared actors (`matchedGeometryEffect`) that move between
steps. Reduce Motion ends every animation in its final state.

## Decision layer

`FirstRunState`, `FirstRunNavigation`, `FirstRunPlan` and `FirstRunRunner`
keep their shape:

- `FirstRunStep` is `welcome, tools, uses, traces, privateAI, join`;
  `FirstRunTier` is gone.
- `CommitPoint.leaveTools` (Continue on Tools: roots, then the next step),
  `.declareRoots` (Customize: roots only, so the daemon can list repos),
  `.signIn` (Join: roots if changed, the invite's lookup and enroll, the
  near.ai sign-in or enrollment), `.passkeyOnJoin` and `.start` as before.
- `recordDiscovery` answers every found, unanswered tool `off`: the check
  starts unticked. `canContinue(.tools)` needs something watched.
- Start sends the rules and past traces Customize set and the Private AI
  switch on every run (they were Custom-only).
- Uses offers Automatic before sign-in, with "Turns on once you sign in.";
  the grant is still sent only at Start, with an enrollment, after both
  disclosures. The disclosures moved from Uses to Join's Start sharing.
- A repo's Automatic under Customize is offered before sign-in too (Ron,
  2026-10-22): picking it shows the core's arming confirmation, which
  records it; the row then reads "Turns on once you sign in." (Uses'
  words), and Start arms it after the consent scopes, with the enrollment
  held. Watching only holds none, so Start never arms it there.
- An invite link fills Join's invite and leaves the step where it is.
- Welcome back (P-7) is offered once the run reaches Join.

## Rulings this changes or keeps

Changed by the canvas (flag for the owner): Automatic is offered on Uses
before an enrollment (2026-10-06 kept it hidden); a tool not found leaves
the list (2026-10-15, #1419, kept every row with Install / Choose folder);
tool rows show their trace count (2026-10-08 showed none); tools start
unticked and Continue waits for one (ruled 2026-10-22); there is no
per-row "Choose a different folder" button (the add row still adds one).

Kept over the canvas: Join has no Skip and offers near.ai alone, with the
invite and passkey cards only for what is held (owner, 2026-10-12, sign-in
required); per-repo Automatic still asks the arming confirmation, and is
armed only once an enrollment is held; past traces start unticked (a pick is the person's approval);
"required" is inline text in the on colour (Ron's review of #1235, item 15).

## Copy

New keys in `first_run_copy.rs`, all DRAFT unless marked Ron's:
`welcome.{title (Ron's), body (Ron's), start}`, `frame.step_traces`,
`frame.step_private_ai`, `uses.{this_mac, you_review, the_commons,
automatic_after_sign_in}`, `traces.{title_light, title_bold, on_this_mac,
points, points_one, rate, rate_one}` and the figure `traces.estimate`
(`{min: 3, max: 8}`, the owner's),
`private_ai.{title_light, title_bold, can_connect, can_connect_one}`.
Changed: `tools.title_bold` is "tools" (Ron's), `uses.title_bold` drops its
period. Keys the macOS shell no longer reads are kept so the wire table is
unchanged.

## Open

- GTK, Windows and Tauri still run #1030's flow; they follow macOS later.
