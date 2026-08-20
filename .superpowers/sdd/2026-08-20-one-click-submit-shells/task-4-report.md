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

---

## Follow-up: project grouping, and three review fixes

Follow-up dispatch, approved by Zaki. Adds real project-group headers with a
"Submit all" action, and fixes three issues from the first report/pass.

### Status

Complete for the verifiable part (`TraceCommons.Interop`, now exercised
against the real native library, not just the managed tests). The
`TraceCommons.App` grouping UI is written but still uncompiled, same
limitation as before.

### What changed

**1. Real project grouping, in Interop.**

- `windows/src/TraceCommons.Interop/QueueGrouping.cs` (new) --
  `QueueGrouping.ByProject(IEnumerable<QueueEntry>)` buckets pending entries
  by `project_id` (never `project_label`), in first-seen order, returning one
  `ProjectQueueGroup` per project with `ProjectId`, `ProjectLabel`, `Count`,
  and `ShowSubmitAll` (`Count > 1`, matching the macOS shell's
  `ProjectQueueGroup` semantics -- confirmed by reading
  `macos/Sources/TraceCommonsApp/Views/QueueView.swift` on
  `impl-macos-submit-task3`; a single-entry group's own row already has a
  `Submit` that does what the group action would, so I agree with macOS's
  rule rather than deviating from it). `QueueGrouping.KeyOf(QueueEntry)`
  exposes the bucketing key itself, so a caller reconstructing which rows
  belong to a group uses the exact same rule the groups were bucketed with
  instead of restating it.
  `ProjectQueueGroup` deliberately holds counts, not entries -- a group is a
  view over the same queue data, not a second copy of it.

- `windows/tests/TraceCommons.Interop.Tests/QueueGroupingTests.cs` (new) --
  8 tests: bucket by id not label; mismatched labels under the same id still
  form one group; first-seen group order; `ShowSubmitAll` only on multi-entry
  groups; entries with no project id group together rather than vanishing;
  the label fallback chain (label -> id -> "Unknown project"); an empty
  queue produces no groups; `KeyOf` matches what `ByProject` actually
  bucketed by.

**2. App-side grouping UI (uncompiled).**

- `windows/src/TraceCommons.App/ViewModels/QueueGroupViewModel.cs` (new) --
  a thin, read-only wrapper pairing one `ProjectQueueGroup` with its rows
  (`ObservableCollection<QueueEntryViewModel>`), exposing `CountText` and
  `SubmitAllText` for display. Carries no decision `QueueGrouping` did not
  already make.
- `windows/src/TraceCommons.App/ViewModels/MainViewModel.cs` -- `Groups`
  (new `ObservableCollection<QueueGroupViewModel>`), rebuilt in
  `ReplacePending` alongside `Pending` via `QueueGrouping.ByProject` +
  `QueueGrouping.KeyOf`.
- `windows/src/TraceCommons.App/MainWindow.xaml` -- the queue's outer
  `ListView` now binds `Groups` instead of `Pending`: one item per project,
  each with a header (label, entry count, and a "Submit all (N)" button
  shown only via `ShowSubmitAll`) and a nested `ItemsControl` holding the
  unchanged per-row card `DataTemplate`. The improvised per-row "Submit
  project" button added in the first pass is removed -- it is superseded by
  the real header action, and keeping both would have let a contributor
  submit a whole project two different ways from two different controls with
  no way to tell them apart by looking. `Submit all` calls `approve` with
  `{"project_id": ...}` once, not a loop over rows (`MainViewModel.SubmitProjectAsync`,
  unchanged from the first pass, already worked this way).
- `windows/src/TraceCommons.App/MainWindow.xaml.cs` -- `OnSubmitProject`
  replaced with `OnSubmitAll` (reads the group from the header's `Tag`/
  `DataContext`, same pattern `EntryOf` already used for rows, now mirrored
  by a new `GroupOf`).

### Mutation testing (grouping)

Two mutations, both compile cleanly:

1. Bucket by `ProjectLabel` instead of `ProjectId` in `QueueGrouping.ByProject`.
2. `ShowSubmitAll => Count >= 1` instead of `Count > 1`.

Filtered run with both applied:
```
Failed QueueGroupingTests.MismatchedLabelsUnderTheSameIdStillFormOneGroup
Failed QueueGroupingTests.EntriesGroupByProjectIdNotLabel
Failed QueueGroupingTests.SubmitAllAppearsOnlyOnMultiEntryGroups
Failed QueueGroupingTests.GroupsAppearInFirstSeenOrder
Failed! - Failed: 4, Passed: 3, Skipped: 0, Total: 7
```
Both reverted; re-run confirmed `Passed: 7, Failed: 0, Total: 7`.

### Fixes from the earlier pass

- **Invented partial-undo wording, removed.** `UndoAsync`'s "Undone for N
  of M; the rest had already gone out." string is gone (I did not propose
  replacement wording for the spec -- the dispatch offered either path and
  removal was the more conservative one for a batch Undo whose behavior
  itself is already a judgment call, not something normative copy has
  addressed yet). A project-group Undo that only partially lands is now
  reported identically to a total refusal ("Too late to undo: it has already
  gone out."); only every entry actually coming back gets "Undone. It stays
  on this machine."
- **`TC_FFI_LIB_DIR` set for every test run in this pass.** `cargo build -p
  trace-commons-contributor-ffi`, then
  `export TC_FFI_LIB_DIR="$PWD/target/debug"` before `dotnet test`. Final run:
  **346 total, 346 passed, 0 failed** -- the 26 `NativeRoundTripTests` that
  failed in the first pass (no native lib available) now pass against the
  real cdylib. This supersedes the first report's 337/311/26 evidence.
- **Refusal vs. known-empty-project, confirmed distinct.** Per the
  correction, `ipc.rs:1241` refuses an unrecognised `project_id` as
  `bad_params`/`project_id_unrecognized` (an error frame); a known project
  with nothing pending succeeds with `approved: 0`. No code change was
  needed -- `ApprovalHold.Parse` already returns `null` for any error frame
  and a real `ApprovalHold` for any success, including a zero-count one --
  but the first pass never pinned the distinction with a test. Added
  `AnUnrecognisedProjectIsDistinctFromAKnownEmptyOne`, which asserts both
  halves in one place.
- **No more hardcoded toast copy in my tests.** The three
  `ApprovalHoldTests` that asserted a literal English sentence (written
  before I knew `SubmitToast.cs`'s copy was about to change) now assert
  against `SubmitToast.Render(...)`'s own output for the same arguments,
  never a string literal. `SubmitToast.cs` was not touched by this pass.

### Test summary (this pass, final)

```
export PATH="/opt/homebrew/opt/dotnet@8/bin:$PATH"
cargo build -p trace-commons-contributor-ffi
export TC_FFI_LIB_DIR="$PWD/target/debug"
cd windows/tests/TraceCommons.Interop.Tests && dotnet test --nologo
```
`Passed! - Failed: 0, Passed: 346, Skipped: 0, Total: 346`

### Commits

3. `c6998f3d` -- Group the queue by project_id, and stop asserting toast copy literals
4. `f140082d` -- Render real project group headers, and stop inventing partial-undo copy

### Concerns for the next reviewer

1. **Grouping UI is still uncompiled**, same limitation the first pass
   flagged for the rest of `TraceCommons.App`. `MainWindow.xaml` was checked
   by hand for XML well-formedness (again caught and fixed several illegal
   `--` sequences inside comments -- same mistake, same detection method, as
   the first pass; worth remembering next time I write a XAML comment) and
   the `.cs`/view-model changes were checked for balanced braces and correct
   call sites, but nothing here has run.
2. **`ItemsControl` nested inside `ListView`, not `CollectionViewSource`
   grouping.** WinUI has a native grouped-`ListView` mechanism
   (`CollectionViewSource`/`GroupStyle`/`ICollectionViewGroup`), which I did
   not use -- it does not compose cleanly with `x:Bind`'s compile-time
   binding without extra indirection, and the outer-list-of-groups,
   inner-list-of-rows shape is a closer structural match to what macOS
   actually built (nested `VStack`s) besides. If the team has a house style
   preference for `CollectionViewSource` on Windows, this should be revisited
   by someone who can actually build and look at it.
3. **The still-open concern from the first report stands**: `UndoHeadline`
   showing the real toast sentence instead of old fixed copy, for both the
   preview-approve path and the new one-click paths, is a visible copy
   change this task was not explicitly asked to make and remains uncompiled.
