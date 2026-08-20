# Task 4 report: Windows -- the two controls

Branch: `task4-windows-submit`, forked from `b87aff70` on `spec-one-click-submit`
(Task 1's toast renderer, including `windows/src/TraceCommons.Interop/SubmitToast.cs`
and `windows/tests/TraceCommons.Interop.Tests/SubmitToastTests.cs`, was already
present at that commit).

Note: `task-4-brief.md` referenced in the dispatch did not exist anywhere in
this repo (checked working tree and full `git log --all`). Work proceeded from
`docs/superpowers/plans/2026-08-20-one-click-submit-shells.md` ("Task 4:
Windows -- the two controls") and `docs/superpowers/specs/2026-08-20-one-click-submit-design.md`
directly.

## Status

Complete for the piece that can be verified here (`TraceCommons.Interop`):
compiles clean under `TreatWarningsAsErrors`, all new tests pass, mutation
tests confirmed teeth. The `TraceCommons.App` (WinUI) changes are written but
**uncompiled and unverified** -- see "What is unverified" below.

## What changed

### 1. Interop layer (verified: compiles, tests pass)

- `windows/src/TraceCommons.Interop/ApprovalHold.cs` -- widened the `approve`
  response model:
  - `Approved` changed from `int` to `ulong` (matches the wire's `u64`; no
    call site outside this file ever read the field, confirmed by grep).
  - Added `Flagged` (`ulong`), `Redactions` (`Dictionary<string, uint>`),
    `Skipped` (`List<SubmitSkip>`), all `[JsonPropertyName]`-mapped to the
    daemon's shape.
  - Added `RedactionsTotal` (sums the map -- what the toast actually names).
  - Added `Toast` (`SubmitToast`) -- assembles `SubmitToast.Render` from the
    decoded fields. This is the only place in the Windows client that builds
    a toast from a real daemon response.
  - Added `ApprovedEntryIds(IEnumerable<string> candidateEntryIds)` -- the
    response never lists which entries it approved, only which it skipped
    (`entry_id` + `reason_label` pairs), so this recovers the approved set by
    subtracting `Skipped` from a caller-supplied candidate list. This is what
    lets a project-group submit's Undo recall the right entries through the
    per-entry `cancel` method.
  - Added `SubmitSkip` (`EntryId`, `ReasonLabel`).
  - `Parse` is unchanged: an error frame (including a `bad_params` refusal
    for an unrecognised `entry_id` or `project_id`) still yields `null`, and
    that null is the refusal signal -- there is no result to build a toast
    from, so a refusal can never be misread as "0 sessions sent".

- `windows/src/TraceCommons.Interop/SubmitParams.cs` (new) -- `SubmitParams.ForEntry`
  and `SubmitParams.ForProject`, the two mutually-exclusive `approve` request
  shapes (`{"entry_id": ...}` / `{"project_id": ...}`), each rejecting an
  empty id with `ArgumentException` rather than sending it. Centralizes the
  request-building decision so the row and project-group call sites cannot
  drift into two spellings of the same request.

### 2. Tests (verified: all pass, mutation-tested)

- `windows/tests/TraceCommons.Interop.Tests/PreviewTests.cs` -- extended
  `ApprovalHoldTests` with:
  - `AnUnrecognisedProjectIsRefusedNotSkipped` -- a `bad_params` error frame
    for a project id yields `null` from `Parse`, same as for an entry id.
  - `TheFullSubmitResponseDecodes` -- the full wire shape (`approved`,
    `flagged`, `redactions`, `skipped`, `hold_secs`, `hold_until`) decodes,
    and `RedactionsTotal`/`Toast.Line`/`Toast.OfferUndo` are all correct
    against a realistic multi-field response.
  - `ApprovedEntryIdsIsTheCandidatesMinusTheSkipped` -- candidates minus
    skipped ids, in order.
  - `ZeroApprovedOffersNoUndoEvenWithSkips` -- exercises the Task 1 defect
    fix on the real decoded type, not bare counts.
- `windows/tests/TraceCommons.Interop.Tests/SubmitParamsTests.cs` (new) --
  4 tests: each builder emits only its own key, and each rejects an empty id.

Baseline at `b87aff70` (clean, `TC_FFI_LIB_DIR` unset): 333 tests total, 307
passed, 26 failed (all `NativeRoundTripTests`, which need the Rust FFI
library and are expected to fail without it -- confirmed unchanged before and
after this branch's work).

After this branch: 337 total, 311 passed, 26 failed (same 26 native tests,
untouched). Net: +4 tests in `ApprovalHoldTests`, +4 in the new
`SubmitParamsTests`, all passing.

Command used, exactly:
```
export PATH="/opt/homebrew/opt/dotnet@8/bin:$PATH"
cd windows/tests/TraceCommons.Interop.Tests && dotnet test --nologo
```
Final run: `Failed! - Failed: 26, Passed: 311, Skipped: 0, Total: 337`

### Mutation testing (teeth proof)

Two mutations, each compiles cleanly and changes real behavior:

1. `ApprovalHold.ApprovedEntryIds` -- dropped the `!` negation
   (`candidateEntryIds.Where(id => skippedIds.Contains(id))` instead of
   `!skippedIds.Contains(id)`), inverting approved/skipped.
2. `SubmitParams.ForProject` -- sent `entry_id` instead of `project_id`.

Result with both mutations applied (filtered to the affected tests):
```
Failed TraceCommons.Interop.Tests.SubmitParamsTests.ForProjectSendsOnlyTheProjectId
  KeyNotFoundException: The given key was not present in the dictionary.
Failed TraceCommons.Interop.Tests.ApprovalHoldTests.ApprovedEntryIdsIsTheCandidatesMinusTheSkipped
  Assert.Equal() Failure: Collections differ
  Expected: string[] ["a", "c"]
  Actual:   List<string> ["b"]
Failed! - Failed: 2, Passed: 3, Skipped: 0, Total: 5
```
Both reverted; re-run confirmed `Passed: 5, Failed: 0, Total: 5`.

### 3. App project (UNVERIFIED -- see below)

- `windows/src/TraceCommons.App/ViewModels/QueueEntryViewModel.cs` -- added
  `ProjectId` (from `QueueEntry.ProjectId`, empty-string fallback). This is
  the id a project-group submit must send; `ProjectLabel` is a display string
  only, never sent to the daemon.

- `windows/src/TraceCommons.App/ViewModels/MainViewModel.cs`:
  - `_undoEntryId` (string) generalized to `_undoEntryIds` (`IReadOnlyList<string>`)
    throughout (`OnDecidedAsync`, `UndoAsync`, `ClearUndo`), because a
    project-group submit's Undo has to recall more than one entry and `cancel`
    only ever takes one `entry_id` at a time.
  - `_undoProjectLabel` replaced with `_undoNoticeLine`; `UndoHeadline` now
    returns the actual `SubmitToast.Line` (e.g. "Sent. Scrubbing removed 4
    things. 1 flagged.") instead of the old fixed "Approved {project}. Still
    on this machine." sentence. This is a genuine copy change to the existing
    preview-approve flow, not only the new one -- both paths decode the same
    `ApprovalHold`, so both now render the same toast contract.
  - New `SubmitEntryAsync(QueueEntryViewModel entry)` -- calls `approve` with
    `SubmitParams.ForEntry`, renders the toast via `ApplySubmitOutcome`.
  - New `SubmitProjectAsync(string projectId)` -- calls `approve` with
    `SubmitParams.ForProject`; reads the project's candidate entry ids from
    `Pending` **before** the call (since `approve` can move entries out of
    `Pending` by the time it returns); refuses to call the daemon at all for
    an empty/whitespace id.
  - New private `ApplySubmitOutcome(DaemonResponse, IReadOnlyList<string>)` --
    the shared tail of both: `ApprovalHold.Parse` returning `null` (any error
    frame, including `bad_params`) is shown as "That couldn't be sent just
    now. Nothing has been sent." -- never as a toast. A non-null hold sets
    `Notice` to `hold.Toast.Line` and arms Undo (via `hold.ApprovedEntryIds`)
    exactly when `hold.Toast.OfferUndo && hold.IsLive(...)`.
  - `UndoAsync` now loops `cancel` over every id in `_undoEntryIds`
    (best-effort per entry, since there is no batch `cancel`), and reports
    "Undone.", "Too late to undo.", or a truthful partial-count message,
    rather than assuming all-or-nothing.

- `windows/src/TraceCommons.App/MainWindow.xaml` / `MainWindow.xaml.cs`:
  - Added a **Submit** button per queue row (`OnSubmitEntry` ->
    `MainViewModel.SubmitEntryAsync`), and a **Submit project** button per row
    (`OnSubmitProject` -> `MainViewModel.SubmitProjectAsync(entry.ProjectId)`).
  - Updated a stale comment on `OnLookInside` and the row's action-group XAML
    comment that asserted "the row has NO Contribute button, and never will"
    -- that invariant is exactly what this spec reverses (the daemon now
    builds and pins an envelope inside `approve` itself for anything
    unpreviewed, so a row submit no longer sends bytes nobody was shown).
  - **Design simplification, called out explicitly**: the queue is a flat
    `ListView` with no group headers today (nothing in this codebase groups
    rows by `project_id`). Building real WinUI group headers
    (`CollectionViewSource`/`GroupStyle`) would be a materially larger,
    equally unverifiable change and was judged out of proportion to this
    task. "Submit project" is therefore reachable from each row rather than
    from a group header, and approves every pending entry sharing that row's
    `ProjectId` in one `approve` call -- functionally the project-group
    action the spec asks for, just without a visual group.

## What is unverified

Everything in `windows/src/TraceCommons.App/` (`MainWindow.xaml`,
`MainWindow.xaml.cs`, `MainViewModel.cs`, `QueueEntryViewModel.cs`) is
**uncompiled**. `XamlCompiler.exe` is a Windows-only binary that runs before
the C# compile step for a WinUI project, and this machine cannot run it, so
nothing here has ever built, let alone executed. Checked by hand instead:
- `MainWindow.xaml` parses as well-formed XML (`xml.etree.ElementTree`) after
  fixing two `--` sequences my first draft introduced inside XAML comments
  (illegal per the XML spec -- comments may not contain `--` except at the
  very end; the rest of the file avoids the construct entirely, which is how
  the bug was caught).
- Every C# edit was checked by hand for balanced braces, correct method
  signatures against call sites, and no leftover reference to the removed
  `_undoEntryId`/`_undoProjectLabel` fields (confirmed by grep: zero hits).
- `SubmitParams`, `ApprovalHold` (including the new `Toast`,
  `ApprovedEntryIds`, `RedactionsTotal`), and `DaemonProtocol.Methods.Approve`/
  `.Cancel` are all real, compiled, tested members of `TraceCommons.Interop`
  as of this branch -- so the App-side call sites reference things that
  actually exist and actually work, even though the calling code itself has
  not been compiled.

None of this substitutes for a Windows build. The `TreatWarningsAsErrors`
risk the brief calls out (an unused import becoming a build failure) applies
squarely to the App-project files above and was not exercisable here.

## Commits

1. `407a62b4` -- Decode the full approve response and build the two request shapes
   (`ApprovalHold.cs`, `SubmitParams.cs` (new), `PreviewTests.cs`,
   `SubmitParamsTests.cs` (new))
2. `56de27d4` -- Wire Submit and Submit project into the WinUI shell
   (`MainWindow.xaml`, `MainWindow.xaml.cs`, `MainViewModel.cs`,
   `QueueEntryViewModel.cs`)

## Concerns / open questions for the next reviewer

1. **"Submit project" has no group header.** As above -- it is a per-row
   button that acts on the whole project, not a header-level action. If the
   product intent is a real visual grouping, that is a separate, larger,
   Windows-only-verifiable task.
2. **`UndoHeadline` copy changed for the existing preview-approve flow, not
   only the new one.** `_undoNoticeLine` now always carries the real toast
   sentence instead of the old fixed "Approved {project}. Still on this
   machine." This seemed correct (both paths decode the same response
   shape and the spec's toast contract should not have two different
   English renderings depending on which button asked), but it is a visible
   copy change to code this task was not explicitly asked to touch, and it
   is one of the uncompiled pieces.
3. **Partial-undo wording is invented, not spec'd.** The spec's Undo section
   only covers the single-entry case (`approved > 0`, one `cancel`). For a
   project-group Undo where some but not all recalls land (a sweep can claim
   entries individually while the loop is still running), I wrote "Undone for
   N of M; the rest had already gone out." This is truthful but not reviewed
   against any normative copy -- worth a second look before it ships.
4. **`bad_params` behavior asserted from the daemon plan, not from a live
   daemon.** The dispatch said an unrecognised `project_id` is refused as
   `bad_params`; `docs/superpowers/plans/2026-08-20-one-click-submit-daemon.md`
   (Task 3, step 5) actually shows an unrecognised `project_id` returning
   `approved: 0` as a *non-error* ("a client can race a sweep... zero
   approved is an outcome, not a fault"), while `bad_params` there is
   reserved for naming neither `entry_id` nor `project_id` at all. The
   Windows-side handling here is safe either way -- a genuine `bad_params`
   error frame is read as a refusal (never a skip), and a `0`-approved
   success response renders correctly as "Nothing sent." via the ordinary
   toast path -- but the two cases are NOT the same thing, and the dispatch's
   framing conflates them. Flagging for the daemon-side implementer to
   confirm actual behavior once Task 3 of the daemon plan lands.
