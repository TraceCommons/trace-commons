# Contributor Shell UX -- Windows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the Windows contributor app to the same folder-first queue
and scrubber transparency as the other two shells.

**Architecture:** `TraceCommons.Interop` is a plain class library whose tests
run in CI; `TraceCommons.App` is the WinUI project and has no test project at
all. Every decision this plan adds goes into `Interop`, with the WinUI view
models calling it. See Global Constraints -- this is the single most
important rule in the plan.

**Tech Stack:** C# / .NET 8, WinUI 3, xUnit (`TraceCommons.Interop.Tests`),
P/Invoke into the `trace_commons_contributor_ffi` cdylib.

**Spec:** [`docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md`](../specs/2026-09-03-contributor-shell-queue-ux-design.md)

**Depends on:** [`2026-09-03-contributor-shell-ux-foundation.md`](2026-09-03-contributor-shell-ux-foundation.md)
(plan 1). **Do not start until plan 1 is merged.**

## Global Constraints

- **Only `TraceCommons.Interop` is tested.** CI's `windows contributor app`
  job runs
  `dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj`,
  and then builds and packages the WinUI app. There is **no test project for
  `TraceCommons.App`**. Consequence: logic placed in a view model is logic
  nothing checks. Put it in `Interop`; have the view model call it. This is
  why `QueueGrouping.cs` already lives there.
- **Run `dotnet` from `windows/`**, so `windows/global.json` pins the SDK.
  MSBuild otherwise selects the newest installed SDK regardless of what
  `setup-dotnet` requested.
- **The WinUI app is built with MSBuild from Visual Studio, not `dotnet
  build`** -- `dotnet build` fails with MSB4062. Follow the CI job's commands
  when building the app locally.
- **The interop tests link the real cdylib**, so
  `cargo build -p trace-commons-contributor-ffi` must run first.
- **No new NuGet packages.** If a task seems to need one, stop and ask.
- **All user-visible strings go in the `*Copy` classes in `Interop`**
  (`VerdictCopy`, `ProjectIgnoreCopy`, `HistoryCopy`, and so on), never
  inline in XAML or a view model. The three shells share wording and these
  classes are what make that checkable.
- **No emojis.**
- **`Look inside` keeps its accent styling.** Task 5 adds a second route to
  it; the button is not demoted.

## A note on the Windows dev VM

This plan cannot be verified on a Mac. Use `win-exec.sh` against the existing
Windows dev VM rather than standing up a new harness. Be aware that Defender
is policy-disabled on that VM, so any claim about scanning behavior there is
false unless you enable it and prove it with EICAR -- not something this plan
needs, but do not let a green run there be mistaken for one.

---

### Task 1: Decode the new daemon fields

**Files:**
- Modify: the queue-entry, preview-summary, history-record, and project-row
  models in `windows/src/TraceCommons.Interop/` (find them with the grep in
  Step 1)
- Test: `windows/tests/TraceCommons.Interop.Tests/DaemonFieldDecodingTests.cs` (create)

**Interfaces:**
- Consumes: plan 1 Tasks 5, 6, 7.
- Produces: `ProjectPath` and `SessionPath` on the queue entry,
  `RedactionsDistinct` on the preview summary, `ProjectId` on the history
  record, `ProjectPath` on the project row.

- [ ] **Step 1: Find the models**

```bash
grep -rln "project_label" windows/src/TraceCommons.Interop/
```

The wire shapes are decoded in that project. Note which file holds each of
the four types before editing.

- [ ] **Step 2: Write the failing tests**

Create `windows/tests/TraceCommons.Interop.Tests/DaemonFieldDecodingTests.cs`.
Match the JSON-decoding idiom the neighbouring tests use -- several of them
already parse daemon payloads, so follow whichever deserializer they call
rather than introducing a second one.

```csharp
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The daemon gained a project path, a session path, distinct redaction
/// counts, and a project id on history records. Every one must tolerate
/// absence: this app ships separately from the daemon and routinely runs
/// against an older one.
/// </summary>
public class DaemonFieldDecodingTests
{
    [Fact]
    public void AQueueEntryDecodesProjectAndSessionPaths()
    {
        var entry = QueueEntry.Parse("""
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "project_path":"~/code/repo","session_path":"~/code/repo/crates/inner",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """);
        Assert.Equal("~/code/repo", entry.ProjectPath);
        Assert.Equal("~/code/repo/crates/inner", entry.SessionPath);
    }

    [Fact]
    public void AQueueEntryFromAnOlderDaemonHasNoPaths()
    {
        var entry = QueueEntry.Parse("""
        {"entry_id":"e1","session_hash":"sha256:a","source":"claude_code",
         "project_id":"proj_abc","project_label":"repo",
         "size_bytes":12,"discovered_at":"2026-09-03T00:00:00Z",
         "state":"pending","attempts":0}
        """);
        Assert.Equal("", entry.ProjectPath);
        Assert.Null(entry.SessionPath);
    }

    [Fact]
    public void APreviewSummaryDecodesDistinctCounts()
    {
        var summary = PreviewSummary.Parse("""
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "redactions_distinct":{"local_path":12},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """);
        Assert.Equal(185, summary.Redactions["local_path"]);
        Assert.Equal(12, summary.RedactionsDistinct["local_path"]);
    }

    [Fact]
    public void APreviewSummaryFromAnOlderDaemonHasNoDistinctCounts()
    {
        var summary = PreviewSummary.Parse("""
        {"would_send_bytes":10,"raw_session_bytes":20,"event_count":3,
         "opening_prompt":"hi","redactions":{"local_path":185},
         "pii_labels_present":[],"consent_scopes":[],"residual_risk":"low"}
        """);
        Assert.Empty(summary.RedactionsDistinct);
    }

    [Fact]
    public void AHistoryRecordDecodesItsProjectId()
    {
        var record = HistoryRecord.Parse("""
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z","project_id":"proj_abc",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """);
        Assert.Equal("proj_abc", record.ProjectId);
    }

    [Fact]
    public void AHistoryRecordFromBeforeTheUpgradeHasNoProjectId()
    {
        var record = HistoryRecord.Parse("""
        {"submission_id":"11111111-1111-1111-1111-111111111111",
         "submitted_at":"2026-09-03T00:00:00Z",
         "project_label":"repo","source":"claude_code","status":"accepted",
         "consent_scopes":[],"credit_points_pending":0,"explanations":[]}
        """);
        Assert.Equal("", record.ProjectId);
    }
}
```

Replace `QueueEntry.Parse` and its siblings with whatever the real entry
points are -- Step 1's grep tells you. Do not add a `Parse` method just to
match this sketch.

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo build -p trace-commons-contributor-ffi
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
```

Expected: compile error, no member `ProjectPath`.

- [ ] **Step 4: Add the properties**

Add each property with an XML doc comment carrying the reasoning, not just
the shape. For the queue entry:

```csharp
    /// <summary>
    /// The project's folder, <c>~</c>-abbreviated, for display only.
    /// </summary>
    /// <remarks>
    /// The daemon relaxed its path rule in exactly one place to send this
    /// (<c>ipc::display_path</c>), because <see cref="ProjectLabel"/> can
    /// keep two projects distinct but can never make them identifiable, and
    /// the queue's folder rows are where that difference is decided. Never
    /// logged, never in a notification, never in a history record.
    ///
    /// Empty against a daemon predating the field; a folder row with no path
    /// renders its label alone.
    /// </remarks>
    public string ProjectPath { get; init; } = "";

    /// <summary>
    /// Where this session actually ran, when that is not the project root.
    /// </summary>
    /// <remarks>
    /// Null both when the daemon predates the field and when the session ran
    /// at the root -- the daemon sends null in the second case rather than
    /// repeating <see cref="ProjectPath"/>, so a row draws this line only
    /// when it says something.
    /// </remarks>
    public string? SessionPath { get; init; }
```

Give the other three the equivalent treatment, defaulting to `""` and an
empty dictionary so absence is the tolerated case rather than an exception.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
```

Expected: 6 new tests pass.

- [ ] **Step 6: Commit**

```bash
git add windows/src/TraceCommons.Interop/ windows/tests/TraceCommons.Interop.Tests/
git commit -m "Decode the project path, session path, distinct counts, and history project id"
```

---

### Task 2: `RedactionTally`

**Files:**
- Create: `windows/src/TraceCommons.Interop/RedactionTally.cs`
- Modify: whichever view model builds the card's removed-by-pattern text
- Test: `windows/tests/TraceCommons.Interop.Tests/RedactionTallyTests.cs` (create)

**Interfaces:**
- Consumes: `PreviewSummary.Redactions`, `.RedactionsDistinct` (Task 1).
- Produces:
  - `public static string Line(IReadOnlyDictionary<string, int> occurrences, IReadOnlyDictionary<string, int> distinct)`
  - `public static int Total(IReadOnlyDictionary<string, int> occurrences)`
  - `public const string NothingMatched = "nothing matched";`

- [ ] **Step 1: Write the failing tests**

```csharp
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The card's "removed by pattern" figure. It carries two different numbers
/// -- how many times a pattern fired, and how many distinct values that was
/// -- and dropping either one misstates the reach of scrubbing.
/// </summary>
public class RedactionTallyTests
{
    private static Dictionary<string, int> Map(params (string, int)[] pairs)
    {
        var map = new Dictionary<string, int>();
        foreach (var (k, v) in pairs) { map[k] = v; }
        return map;
    }

    [Fact]
    public void AnEmptyTallyIsNothingMatched()
    {
        Assert.Equal(RedactionTally.NothingMatched, RedactionTally.Line(Map(), Map()));
        Assert.Equal(0, RedactionTally.Total(Map()));
    }

    [Fact]
    public void LabelsAreHumanReadable()
        => Assert.Equal("3 local path", RedactionTally.Line(Map(("local_path", 3)), Map()));

    [Fact]
    public void DistinctCountsAreShownWhenTheyDifferFromOccurrences()
        => Assert.Equal(
            "185 local path (12 distinct)",
            RedactionTally.Line(Map(("local_path", 185)), Map(("local_path", 12))));

    [Fact]
    public void DistinctIsOmittedWhenEveryOccurrenceIsItsOwnValue()
        // "3 secret (3 distinct)" says the same thing twice.
        => Assert.Equal(
            "3 secret",
            RedactionTally.Line(Map(("secret", 3)), Map(("secret", 3))));

    [Fact]
    public void DistinctIsOmittedWhenTheDaemonDidNotReportIt()
        => Assert.Equal("3 secret", RedactionTally.Line(Map(("secret", 3)), Map()));

    [Fact]
    public void ADistinctCountAboveItsOccurrenceCountIsIgnored()
        // Impossible from a correct daemon; "3 secret (9 distinct)" would be
        // worse than saying nothing.
        => Assert.Equal(
            "3 secret",
            RedactionTally.Line(Map(("secret", 3)), Map(("secret", 9))));

    [Fact]
    public void TheBiggestCountLeadsAndTiesBreakOnLabel()
        => Assert.Equal(
            "185 local path  ·  3 email  ·  3 secret",
            RedactionTally.Line(Map(("secret", 3), ("local_path", 185), ("email", 3)), Map()));

    [Fact]
    public void TotalSumsOccurrencesNotDistinct()
        => Assert.Equal(5, RedactionTally.Total(Map(("a", 2), ("b", 3))));
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj --filter RedactionTallyTests
```

Expected: `The name 'RedactionTally' does not exist`.

- [ ] **Step 3: Write the implementation**

Create `windows/src/TraceCommons.Interop/RedactionTally.cs`, with a class
comment recording why two numbers exist -- the redactor mints one
placeholder per distinct value and reuses it, so one path referenced two
hundred times is two hundred occurrences and one value -- and the ordering
rule (biggest count first, ties on label, so the order is stable between
redraws). The `Line` body mirrors the macOS and GTK implementations
exactly, including omitting the distinct figure when it equals or exceeds
the occurrence count.

- [ ] **Step 4: Point the view model at it**

Replace whatever currently builds the removed-by-pattern string with a
`RedactionTally.Line(...)` call, and any total-redactions count with
`RedactionTally.Total(...)`.

- [ ] **Step 5: Run the tests and commit**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
git add windows/src/TraceCommons.Interop/RedactionTally.cs \
        windows/tests/TraceCommons.Interop.Tests/RedactionTallyTests.cs \
        windows/src/TraceCommons.App/
git commit -m "Move the removed-by-pattern figure into a tested type"
```

---

### Task 3: Folder groups and navigation

`ProjectQueueGroup` already exists in `Interop` and already carries the rule
the spec retires: `ShowSubmitAll => Count > 1`. It also holds counts only,
deliberately -- but a folder row now needs the path and the byte total too.

**Files:**
- Modify: `windows/src/TraceCommons.Interop/QueueGrouping.cs`
- Create: `windows/src/TraceCommons.Interop/QueueNavigation.cs`
- Modify: `windows/tests/TraceCommons.Interop.Tests/QueueGroupingTests.cs`
- Test: `windows/tests/TraceCommons.Interop.Tests/QueueNavigationTests.cs` (create)

**Interfaces:**
- Consumes: `QueueEntry.ProjectPath`, `.SizeBytes` (Task 1).
- Produces:
  - `ProjectQueueGroup.ProjectPath { get; }`, `.SizeBytes { get; }`
  - `ProjectQueueGroup.ShowSubmitAll` returning `true` at every count
  - `public abstract record QueueLocation` with `Root` and `Project(string ProjectId)` cases
  - `public static QueueLocation Resolve(QueueLocation location, IReadOnlyList<ProjectQueueGroup> groups)`

- [ ] **Step 1: Update the existing grouping test**

`QueueGroupingTests.cs` asserts today's rule. Find the case covering
`ShowSubmitAll` at one entry and replace it, keeping the reasoning in the
test itself:

```csharp
    /// <summary>
    /// Shown at every count, including one.
    ///
    /// The old rule hid it at one because the row's own Submit was on the
    /// same screen and did the same thing. Under the folder-first layout that
    /// row is a level down, so hiding this would mean opening a folder to do
    /// the thing the folder is offering. The rule expired with the layout it
    /// was written for.
    /// </summary>
    [Fact]
    public void ASingleEntryGroupStillOffersSubmitAll()
    {
        var groups = QueueGrouping.Group(new[] { Entry("e1", "proj_a", "api") });
        Assert.True(groups[0].ShowSubmitAll);
    }
```

Add the two new properties' tests:

```csharp
    [Fact]
    public void AGroupSumsItsMembersBytes()
    {
        var groups = QueueGrouping.Group(new[]
        {
            Entry("e1", "proj_a", "api", bytes: 30),
            Entry("e2", "proj_a", "api", bytes: 12),
        });
        Assert.Equal(42, groups[0].SizeBytes);
    }

    [Fact]
    public void AGroupTakesThePathOfItsFirstMember()
    {
        var groups = QueueGrouping.Group(new[]
        {
            Entry("e1", "proj_a", "api", path: "~/work/api"),
            Entry("e2", "proj_a", "api", path: "~/work/api"),
        });
        Assert.Equal("~/work/api", groups[0].ProjectPath);
    }
```

Extend the file's `Entry` helper with optional `bytes` and `path`
parameters rather than writing a second helper.

- [ ] **Step 2: Write the failing navigation tests**

Create `windows/tests/TraceCommons.Interop.Tests/QueueNavigationTests.cs`:

```csharp
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The queue is now two levels, and the second can be pulled out from under
/// the person standing on it: approving a folder's last session removes the
/// folder, and so does an upload finishing in the background.
/// </summary>
public class QueueNavigationTests
{
    private static IReadOnlyList<ProjectQueueGroup> Groups(params string[] ids)
        => QueueGrouping.Group(System.Array.ConvertAll(ids, id => Entry(id)));

    [Fact]
    public void RootStaysRoot()
    {
        Assert.Equal(QueueLocation.Root, QueueNavigation.Resolve(QueueLocation.Root, Groups("a")));
        Assert.Equal(QueueLocation.Root, QueueNavigation.Resolve(QueueLocation.Root, Groups()));
    }

    [Fact]
    public void AProjectThatStillExistsIsKept()
        => Assert.Equal(
            new QueueLocation.Project("a"),
            QueueNavigation.Resolve(new QueueLocation.Project("a"), Groups("a", "b")));

    /// <summary>
    /// Submit all inside a folder: the folder goes, and standing in it would
    /// show an empty pane with a back button and no explanation.
    /// </summary>
    [Fact]
    public void AProjectThatEmptiedFallsBackToRoot()
        => Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(new QueueLocation.Project("a"), Groups("b")));

    [Fact]
    public void TheLastProjectEmptyingFallsBackToRoot()
        => Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(new QueueLocation.Project("a"), Groups()));

    [Fact]
    public void ResolutionIsByIdNotLabel()
    {
        // Two projects can share a label; only the id identifies one.
        var groups = QueueGrouping.Group(new[]
        {
            Entry("e1", "proj_1", "api"),
            Entry("e2", "proj_2", "api"),
        });
        Assert.Equal(
            new QueueLocation.Project("proj_2"),
            QueueNavigation.Resolve(new QueueLocation.Project("proj_2"), groups));
        Assert.Equal(
            QueueLocation.Root,
            QueueNavigation.Resolve(new QueueLocation.Project("proj_3"), groups));
    }
}
```

Reuse `QueueGroupingTests`' entry helper by lifting it into a shared
internal helper rather than copying it; the sketch above assumes an `Entry`
in scope.

- [ ] **Step 3: Run both to verify they fail**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
```

Expected: `ASingleEntryGroupStillOffersSubmitAll` fails on the assertion;
the others fail to compile.

- [ ] **Step 4: Write the implementation**

In `QueueGrouping.cs`, add `ProjectPath` and `SizeBytes` to
`ProjectQueueGroup` (taking the path from the first member and summing the
bytes in the grouping pass), and change `ShowSubmitAll`:

```csharp
    /// <summary>
    /// Whether the folder row offers a "Submit all" action. Always -- see the
    /// remark.
    /// </summary>
    /// <remarks>
    /// This used to be <c>Count > 1</c>, on the reasoning that a single-entry
    /// group's own row already had a Submit doing exactly the same thing. That
    /// was true of a flat list where the row and the header were on screen
    /// together. Under the folder-first layout the row is a level down, so
    /// hiding this would mean opening a folder to do the thing the folder is
    /// offering. The rule expired with the layout it was written for; the
    /// property stays so callers do not have to know that.
    /// </remarks>
    public bool ShowSubmitAll => true;
```

Update `ProjectQueueGroup`'s class remark, which currently says the type
"deliberately holds counts, not entries" -- still true, and now it holds two
more scalars for the same reason.

Create `QueueNavigation.cs` with the location record and `Resolve`, carrying
the comment that it is a pure function of the location and the groups rather
than a mutation, so a view can call it on every redraw and never hold a
stale location.

- [ ] **Step 5: Run the tests and commit**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
git add windows/src/TraceCommons.Interop/ windows/tests/TraceCommons.Interop.Tests/
git commit -m "Group the queue into folders, with Submit all at every count"
```

---

### Task 4: The folder list and the folder detail

**Files:**
- Modify: `windows/src/TraceCommons.App/MainWindow.xaml` (queue section), `MainWindow.xaml.cs`
- Modify: `windows/src/TraceCommons.App/ViewModels/MainViewModel.cs`, `ViewModels/QueueGroupViewModel.cs`

**Interfaces:**
- Consumes: `QueueLocation`, `QueueNavigation.Resolve`, `ProjectQueueGroup`
  (Task 3); `QueueEntry.ProjectPath`, `.SessionPath` (Task 1).

- [ ] **Step 1: Hold the location on the view model**

In `MainViewModel`, add a `QueueLocation` property defaulting to
`QueueLocation.Root`, re-resolved through `QueueNavigation.Resolve` every
time the queue snapshot changes -- not only when the user navigates. That is
what makes a folder emptying underneath the user return them to the list.

Expose `IsAtQueueRoot` and `OpenFolder`, and raise change notification for
both alongside the existing queue properties.

- [ ] **Step 2: Bind the two levels in XAML**

Split the queue section of `MainWindow.xaml` into two panels whose
visibility is bound to `IsAtQueueRoot`:

- The folder list binds to the `ProjectQueueGroup` collection. Each row puts
  `ProjectLabel` in the card-title style with `ProjectPath` beneath it in the
  meta style, `Count` and `SizeBytes` at the trailing edge, and the three
  actions -- `Submit all`, `Submit all as...`, `Ignore project` -- underneath.
  The row itself is a button that calls `OpenFolder`.
- The detail panel keeps today's card list, filtered to the open folder, with
  a back control bound to a command that sets the location to
  `QueueLocation.Root`, and the folder's label and path as its heading.

Bind `Submit all`'s visibility to `ShowSubmitAll` as it already does; Task 3
changed what that returns, so no binding changes here.

- [ ] **Step 3: Make the folder name the largest text**

On the folder row, the label takes the card-title style and the path the
meta style. Today the project label is the smallest text on its line, beside
an accented `Submit all`, so the line reads as a button with a caption. It
should read as a place with actions.

- [ ] **Step 4: Show where each session ran**

In the queue card template, add a line bound to `SessionPath`, collapsed when
null, in the meta style, trimmed at the head so the tail of the path
survives.

- [ ] **Step 5: Build and test**

```bash
cargo build -p trace-commons-contributor-ffi
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
```

Then build the WinUI app the way CI does (MSBuild from Visual Studio, not
`dotnet build`).

- [ ] **Step 6: Commit**

```bash
git add windows/src/TraceCommons.App/
git commit -m "Show folders first in the queue, with sessions one level in"
```

---

### Task 5: The card opens the preview

**Files:**
- Modify: `windows/src/TraceCommons.App/MainWindow.xaml` (queue card template), `MainWindow.xaml.cs`

- [ ] **Step 1: Make the card a hit target**

Give the card's root container a `Tapped` handler that invokes the same
command `Look inside` uses. The three footer buttons handle their own taps
and mark the event handled, so they keep working -- but WinUI routed events
bubble, so confirm it rather than assuming: if a footer button starts opening
the preview, set `e.Handled = true` in that button's own click handler rather
than removing the card gesture.

Add the comment recording why the button stays:

```csharp
    // A second route to "Look inside", never a replacement for it. The button
    // keeps its emphasis: one-click submit added AVAILABILITY, and accent
    // styling is a RECOMMENDATION. What this adds is that the obvious gesture
    // on a card does the obvious thing.
```

- [ ] **Step 2: Build and check by hand on the dev VM**

Confirm the card body opens the preview and each footer button still does its
own job. Record what you saw in the commit message -- no test covers this.

- [ ] **Step 3: Commit**

```bash
git add windows/src/TraceCommons.App/
git commit -m "Open the preview from anywhere on the card"
```

---

### Task 6: Mark redactions in the transcript

**Files:**
- Create: `windows/src/TraceCommons.Interop/RedactionPlaceholders.cs`
- Modify: `windows/src/TraceCommons.App/ViewModels/PreviewSheetViewModel.cs`, `Controls/PreviewSheet.xaml`
- Test: `windows/tests/TraceCommons.Interop.Tests/RedactionPlaceholdersTests.cs` (create)

**Interfaces:**
- Consumes: the preview transcript string.
- Produces:
  - `public sealed record RedactionPlaceholder(int Start, int Length, string Label, int Ordinal) { public string Display { get; } }`
  - `public static IReadOnlyList<RedactionPlaceholder> Scan(string body)`

- [ ] **Step 1: Write the failing tests**

```csharp
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The redactor leaves a typed placeholder where it removed a value, and
/// those tokens are already in the transcript the ABI hands us -- rendered,
/// until now, as ordinary text. Finding them is what lets the preview say
/// WHERE something was cut, which is more than a category count can.
/// </summary>
public class RedactionPlaceholdersTests
{
    [Fact]
    public void ABodyWithNoPlaceholdersScansToNothing()
    {
        Assert.Empty(RedactionPlaceholders.Scan("just some ordinary text"));
        Assert.Empty(RedactionPlaceholders.Scan(""));
    }

    [Fact]
    public void ASinglePlaceholderIsFound()
    {
        const string body = "ran the build in <PRIVATE_LOCAL_PATH_1> and stopped";
        var found = RedactionPlaceholders.Scan(body);
        Assert.Single(found);
        Assert.Equal("LOCAL_PATH", found[0].Label);
        Assert.Equal(1, found[0].Ordinal);
        Assert.Equal("<PRIVATE_LOCAL_PATH_1>", body.Substring(found[0].Start, found[0].Length));
    }

    [Fact]
    public void TheDisplayNameIsHumanReadable()
        => Assert.Equal(
            "contextual entropy",
            RedactionPlaceholders.Scan("<PRIVATE_CONTEXTUAL_ENTROPY_2>")[0].Display);

    [Fact]
    public void MultiplePlaceholdersAreFoundInOrder()
    {
        var found = RedactionPlaceholders.Scan(
            "<PRIVATE_SECRET_1> then <PRIVATE_LOCAL_PATH_3> then <PRIVATE_SECRET_1>");
        Assert.Equal(new[] { "SECRET", "LOCAL_PATH", "SECRET" },
            System.Linq.Enumerable.ToArray(
                System.Linq.Enumerable.Select(found, p => p.Label)));
        Assert.Equal(new[] { 1, 3, 1 },
            System.Linq.Enumerable.ToArray(
                System.Linq.Enumerable.Select(found, p => p.Ordinal)));
    }

    /// <summary>
    /// The ordinal is the last underscore-delimited run of digits, so a label
    /// that itself ends in a number must not steal it.
    /// </summary>
    [Fact]
    public void ALabelContainingDigitsIsParsedCorrectly()
    {
        var found = RedactionPlaceholders.Scan("<PRIVATE_SHA256_KEY_7>");
        Assert.Equal("SHA256_KEY", found[0].Label);
        Assert.Equal(7, found[0].Ordinal);
    }

    [Fact]
    public void TextThatMerelyLooksLikeAPlaceholderIsIgnored()
    {
        Assert.Empty(RedactionPlaceholders.Scan("<PRIVATE>"));
        Assert.Empty(RedactionPlaceholders.Scan("<PRIVATE_LOCAL_PATH_>"));
        Assert.Empty(RedactionPlaceholders.Scan("<private_local_path_1>"));
        Assert.Empty(RedactionPlaceholders.Scan("PRIVATE_LOCAL_PATH_1"));
    }

    /// <summary>
    /// Offsets index a C# string, which is UTF-16. The ABI reports UTF-8 byte
    /// offsets elsewhere and <c>TcPreview.Search</c> converts them; this scan
    /// runs on the already-converted string, so its offsets are UTF-16 and
    /// must survive text outside the BMP.
    /// </summary>
    [Fact]
    public void OffsetsIndexTheManagedStringIncludingAstralText()
    {
        const string body = "h\U0001F600llo <PRIVATE_SECRET_1> world";
        var found = RedactionPlaceholders.Scan(body);
        Assert.Equal("<PRIVATE_SECRET_1>", body.Substring(found[0].Start, found[0].Length));
    }
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj --filter RedactionPlaceholdersTests
```

Write `RedactionPlaceholders.cs` with the class comment recording that the
redactor substitutes rather than deletes, that the tokens were always in the
transcript, and the caveat that a region with no placeholder is not a region
with nothing sensitive in it -- the detector scans every leaf while the
rewriter reaches only typed fields, so marking makes the app look more
thorough than it is, and the scrubbing caveat belongs beside the marks.

Use a compiled `Regex` with the pattern `<PRIVATE_([A-Z0-9_]*[A-Z0-9])_([0-9]+)>`.
The `[A-Z0-9_]*[A-Z0-9]` shape is what forces the label to end on a
non-underscore so the final `_<digits>` is the ordinal.

- [ ] **Step 3: Mark them in the sheet**

The transcript is displayed as text. Replace it with a `RichTextBlock` (or
inlines on the existing `TextBlock`) built by walking the scan results: plain
runs between placeholders, and a gold-toned run for each placeholder. Keep
the scrubbing caveat visible on the same tab.

- [ ] **Step 4: Run the tests and commit**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
git add windows/src/TraceCommons.Interop/RedactionPlaceholders.cs \
        windows/tests/TraceCommons.Interop.Tests/RedactionPlaceholdersTests.cs \
        windows/src/TraceCommons.App/
git commit -m "Mark each redaction where it happened in the transcript"
```

---

### Task 7: Recent searches stop recording prefixes

This shell has the same defect as macOS. `OnNeedleChanged`
(`Controls/PreviewSheet.xaml.cs:436`) fires on `TextChanged` and calls
`RunSearch()`, which calls `Remember(Needle)` on every hit. Typing `xyz`
records `x`, `xy`, and `xyz`, filling a six-slot strip with prefixes of one
word.

The GTK shell already gets this right by taking the intent as a parameter --
`run_search(needle, remember)` -- and that is the shape to copy.

**Files:**
- Create: `windows/src/TraceCommons.Interop/RecentSearches.cs`
- Modify: `windows/src/TraceCommons.App/ViewModels/PreviewSheetViewModel.cs:530-572, 758-772`
- Modify: `windows/src/TraceCommons.App/Controls/PreviewSheet.xaml.cs:436-445`
- Test: `windows/tests/TraceCommons.Interop.Tests/RecentSearchesTests.cs` (create)

**Interfaces:**
- Produces:
  - `RecentSearches.Remember(string term) -> IReadOnlyList<string>`
  - `RecentSearches.Current -> IReadOnlyList<string>`
  - `RecentSearches.Reset()` (test seam)
  - `PreviewSheetViewModel.RunSearch(bool remember)`

- [ ] **Step 1: Write the failing tests**

```csharp
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A recent-search list is the contributor's list of the things they were
/// afraid of leaking. It stays in memory for that reason, and it must hold
/// what they actually asked -- not every prefix they typed on the way there.
/// </summary>
public class RecentSearchesTests
{
    public RecentSearchesTests() => RecentSearches.Reset();

    [Fact]
    public void AnEmptyListStartsEmpty() => Assert.Empty(RecentSearches.Current);

    [Fact]
    public void ACommittedTermIsRemembered()
        => Assert.Equal(new[] { "acme-corp" }, RecentSearches.Remember("acme-corp"));

    [Fact]
    public void TheMostRecentTermLeads()
    {
        RecentSearches.Remember("first");
        Assert.Equal(new[] { "second", "first" }, RecentSearches.Remember("second"));
    }

    [Fact]
    public void RepeatingATermMovesItToTheFrontWithoutDuplicating()
    {
        RecentSearches.Remember("a");
        RecentSearches.Remember("b");
        Assert.Equal(new[] { "a", "b" }, RecentSearches.Remember("a"));
    }

    [Fact]
    public void TheListIsCappedAtSix()
    {
        foreach (string term in new[] { "1", "2", "3", "4", "5", "6", "7" })
        {
            RecentSearches.Remember(term);
        }
        Assert.Equal(6, RecentSearches.Current.Count);
        Assert.Equal("7", RecentSearches.Current[0]);
        Assert.DoesNotContain("1", RecentSearches.Current);
    }

    [Fact]
    public void AnEmptyOrBlankTermIsNotRemembered()
    {
        RecentSearches.Remember("");
        RecentSearches.Remember("   ");
        Assert.Empty(RecentSearches.Current);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj --filter RecentSearchesTests
```

Expected: `The name 'RecentSearches' does not exist`.

- [ ] **Step 3: Move the list into Interop**

Create `RecentSearches.cs`, carrying over the reasoning from
`PreviewSheetViewModel`'s existing comment at `:89` verbatim -- that the list
is the contributor's own list of things they were afraid of leaking, which is
exactly what makes it a worse thing to write to disk than most of what the
search was checking for. Add trimming, the blank guard, dedupe, the cap at
six, and `Reset()`.

Delete `ProcessRecentSearches` and `Remember` from the view model and have it
call `RecentSearches` instead, refilling its `ObservableCollection` from
`RecentSearches.Current`.

- [ ] **Step 4: Split running from remembering**

```csharp
    /// <summary>Runs the search without recording the term.</summary>
    /// <remarks>
    /// Live search on every keystroke is the good part and stays. Recording
    /// there is what filled the six-slot strip with the prefixes of one word:
    /// typing "xyz" recorded "x", "xy", and "xyz". A recent search is a
    /// question the contributor asked, and they ask it by pressing Enter or
    /// the button -- not by passing through a prefix on the way. The GTK shell
    /// has taken the intent as a parameter from the start; this matches it.
    /// </remarks>
    public void RunSearch() => RunSearch(remember: false);

    public void RunSearch(bool remember)
    {
        // ... existing body, with the Remember call guarded:
        if (remember && _matches.Count > 0)
        {
            Remember(Needle);
        }
    }
```

Point `OnNeedleChanged` at `RunSearch(remember: false)`, and `OnSearchClick`
plus the Enter key at `RunSearch(remember: true)`. If Enter is not currently
bound, bind it -- otherwise the button becomes the only way to record a term.

- [ ] **Step 5: Run the tests and commit**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
git add windows/src/TraceCommons.Interop/RecentSearches.cs \
        windows/tests/TraceCommons.Interop.Tests/RecentSearchesTests.cs \
        windows/src/TraceCommons.App/
git commit -m "Remember a search when it is asked, not on every keystroke"
```

---

### Task 8: Tell "never there" apart from "removed", and give the chip a job

**Files:**
- Modify: `windows/src/TraceCommons.Interop/NativeMethods.cs:166-170` (new P/Invoke)
- Modify: the `TcDaemon` / handle wrapper in `Interop` (new method)
- Create: `windows/src/TraceCommons.Interop/OriginalSearchOutcome.cs`
- Modify: `windows/src/TraceCommons.App/ViewModels/PreviewSheetViewModel.cs`, `MainWindow.xaml` (the chip)
- Test: `windows/tests/TraceCommons.Interop.Tests/OriginalSearchOutcomeTests.cs` (create); extend `NativeRoundTripTests.cs`

**Interfaces:**
- Consumes: `tc_search_original` (plan 1 Task 8).
- Produces:
  - `internal static extern int tc_search_original(IntPtr handle, string entryId, string needle)`
  - `int? SearchOriginal(string entryId, string needle)` on the handle wrapper
  - `public abstract record OriginalSearchOutcome` with `Absent`, `AllRemoved(int)`, `SomeRemain(int Remaining, int Total)`, `Unknown`
  - `public static OriginalSearchOutcome Classify(int remaining, int? original)`
  - `public string Sentence { get; }`, `public bool IsAlarming { get; }`

- [ ] **Step 1: Write the failing outcome tests**

Mirror the macOS and GTK suites exactly -- the same seven cases, including
the two that matter most:

```csharp
    /// <summary>
    /// Reporting "not in this session" because a call failed would be the
    /// single most dangerous wrong answer this tab can give.
    /// </summary>
    [Fact]
    public void AFailedOriginalSearchIsUnknownNotAbsent()
    {
        Assert.Equal(OriginalSearchOutcome.Unknown, OriginalSearchOutcome.Classify(0, null));
        Assert.Equal(
            new OriginalSearchOutcome.SomeRemain(2, 2),
            OriginalSearchOutcome.Classify(2, null));
    }

    [Fact]
    public void AnOriginalCountBelowTheRemainingCountFallsBackToWhatIsCertain()
        => Assert.Equal(
            new OriginalSearchOutcome.SomeRemain(2, 2),
            OriginalSearchOutcome.Classify(2, 1));
```

plus `Absent` for `(0, 0)`, `AllRemoved(3)` for `(0, 3)`, `SomeRemain(2, 5)`
for `(2, 5)`, the four sentences, and `IsAlarming` true only for
`SomeRemain`.

- [ ] **Step 2: Run to verify it fails, then implement**

`Classify` mirrors the other two shells: with no original count, fail toward
what is certain -- the redacted body is in hand, so matches in it are known,
and the absence of a check must never render as a clean result.

- [ ] **Step 3: Add the P/Invoke and the wrapper**

In `NativeMethods.cs`, beside `tc_preview_search`:

```csharp
    /// <summary>
    /// Counts occurrences of <paramref name="needle"/> in an entry's
    /// PRE-redaction session text. Returns the count, or -1 on error.
    /// </summary>
    /// <remarks>
    /// A COUNT, never content -- that is the whole bound of this call, and
    /// the reason it is allowed to read unredacted bytes at all. It takes a
    /// handle and an entry id rather than a preview because a preview lives
    /// as long as its sheet, and an unredacted transcript must not.
    /// </remarks>
    [DllImport(Library, CallingConvention = CallingConvention.Cdecl)]
    internal static extern int tc_search_original(
        IntPtr handle,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string entryId,
        [MarshalAs(UnmanagedType.LPUTF8Str)] string needle);
```

and a `SearchOriginal` method on the handle wrapper returning `null` when the
ABI reports -1.

- [ ] **Step 4: Extend the round-trip test**

`NativeRoundTripTests.cs` links the real cdylib, so it can exercise this for
real. Add a case that opens a daemon with a session containing a plantable
secret, calls `SearchOriginal` for it, and asserts a positive count -- and a
second asserting that a needle nowhere in the session returns 0, not -1.
Follow that file's existing fixture setup rather than building a new one.

- [ ] **Step 5: Wire it into the sheet**

In `RunSearch`, after the redacted-body matches are counted, call
`SearchOriginal` for the same needle and expose
`OriginalSearchOutcome.Classify(...)`. Bind the summary text to
`Sentence` and its tone to `IsAlarming`.

- [ ] **Step 6: Make the nothing-matched chip a control**

Turn the chip into a button that opens the preview on its search tab. Extend
the scrubbing-caveat copy's zero-redaction sentence with the clause pointing
at search, matching the other two shells word for word, and assert it in the
copy tests:

```csharp
    [Fact]
    public void TheNothingMatchedLineOffersANextStep()
        => Assert.Contains("search", ScrubbingCaveat.RowLine(0), System.StringComparison.OrdinalIgnoreCase);
```

- [ ] **Step 7: Run the tests and commit**

```bash
cargo build -p trace-commons-contributor-ffi
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
git add windows/src/ windows/tests/
git commit -m "Say whether a searched value was removed, and give the chip a job"
```

---

### Task 9: Shield state, and history by folder

**Files:**
- Create: `windows/src/TraceCommons.Interop/QueueShieldState.cs`, `windows/src/TraceCommons.Interop/HistoryFolders.cs`
- Modify: `windows/src/TraceCommons.App/MainWindow.xaml` (nav item, history section), `ViewModels/MainViewModel.cs`
- Test: `windows/tests/TraceCommons.Interop.Tests/QueueShieldStateTests.cs`, `HistoryFoldersTests.cs` (create)

**Interfaces:**
- Produces:
  - `public enum QueueShieldState { Clear, Waiting, Attention }` and
    `public static QueueShieldState For(int waiting, int nothingMatched, int trimmed)`
  - `public static IReadOnlyList<HistoryFolder> Group(IReadOnlyList<HistoryRecord> records)`
  - `internal const string UnresolvedPrefix = "label:";`

- [ ] **Step 1: Write the failing tests**

Shield: the five cases from the other two shells -- empty is `Clear`, an
ordinary queue is `Waiting`, a nothing-matched or trimmed session raises
`Attention`, and an empty queue is `Clear` even with stale flags.

History: the four cases -- grouping by `ProjectId`, two projects sharing a
label staying separate, records with no id grouping by label instead, and an
identified and an unidentified record never merging:

```csharp
    /// <summary>
    /// Same label, one resolvable and one not. Claiming they are the same
    /// folder is a guess; two rows is the honest answer.
    /// </summary>
    [Fact]
    public void AnIdentifiedAndAnUnidentifiedRecordDoNotMerge()
    {
        var folders = HistoryFolders.Group(new[]
        {
            Record("1", projectId: "proj_a", label: "api"),
            Record("2", projectId: "", label: "api"),
        });
        Assert.Equal(2, folders.Count);
    }
```

- [ ] **Step 2: Run to verify they fail, then implement**

`QueueShieldState.For` returns `Clear` whenever nothing is waiting, and
otherwise `Attention` if either flag count is non-zero. Carry the comment
recording that the shield is **added to** the numeric count rather than
replacing it: the ask was to swap the count for an icon, and at 149 waiting
sessions the count is the signal a contributor is actually reading.

`HistoryFolders.Group` keys on `ProjectId`, falling back to
`UnresolvedPrefix + ProjectLabel` when the id is empty -- a real id always
starts with `proj_`, so the two key spaces cannot collide. Document why an
unidentified record is never merged into an identified one, and that
pre-normalization records are not backfillable because nothing retained the
key they were minted from.

- [ ] **Step 3: Wire both into the UI**

Nav item: bind a shield glyph's tone to `QueueShieldState`, and **keep the
numeric badge exactly as it is**. Derive `nothingMatched` from entries whose
preview summary has an empty `Redactions` map and `trimmed` from those with
`SubagentsDropped > 0`.

History: the same two-level shape as the queue, with its own `QueueLocation`
on `MainViewModel`, folder paths resolved by matching `ProjectId` against the
loaded project rows, and any group whose key starts with
`HistoryFolders.UnresolvedPrefix` rendering its label alone.

- [ ] **Step 4: Run the tests and commit**

```bash
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
git add windows/src/ windows/tests/
git commit -m "Add the queue shield, and group history by folder"
```

---

### Task 10: Full verification

- [ ] **Step 1: Everything CI runs**

```bash
cargo build -p trace-commons-contributor-ffi
cd windows && dotnet test tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj --configuration Release
```

Then build and package the WinUI app with MSBuild from Visual Studio, as the
`windows contributor app` job does, and confirm exactly one package is
produced. Paste the test summary into the PR body.

- [ ] **Step 2: Confirm the MSIX manifest check still passes**

The same CI job checks the manifest and that the default build is still
unpackaged. This plan changes no packaging, so a failure here means something
unintended moved.

- [ ] **Step 3: Run the app on the dev VM and check what tests cannot see**

Via `win-exec.sh`. Confirm, and report in the PR body:

1. The queue opens on a folder list; folder names are the largest text.
2. Clicking a folder opens its sessions; the back control returns.
3. `Submit all` on a one-session folder works without opening it.
4. Approving a folder's last session returns you to the folder list.
5. Clicking a card body opens the preview; footer buttons still work.
6. Redactions are marked in the transcript.
7. Typing `xyz` in search leaves one recent entry, not three.
8. Searching a value you know was redacted says it was removed.
9. The nothing-matched chip opens search.
10. History is grouped by folder.

- [ ] **Step 4: Open the PR**

```bash
git push -u origin shell-ux-windows
gh pr create --repo zmanian/trace-commons-server \
  --title "Folder-first queue and scrubber transparency, Windows" \
  --body "Implements docs/superpowers/plans/2026-09-03-contributor-shell-ux-windows.md.

Depends on the daemon and FFI foundation PR.

Spec: docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md"
```

---

## Self-Review

**Spec coverage.**

| Spec section | Task |
|---|---|
| §1.1 `project_path` consumed | Task 1 |
| §2.1 folder list, name prominence | Tasks 3, 4 |
| §2.2 folder detail, `session_path` | Task 4 |
| §2.3 `Submit all` at n = 1 | Task 3 (`ShowSubmitAll`) |
| §2.4 card click | Task 5 |
| §3.1 placeholders marked | Task 6 |
| §3.1 distinct counts | Task 2 |
| §3.2 original search | Task 8 |
| §3.3 recent-search prefixes | Task 7 |
| §3.4 nothing-matched affordance | Task 8 Step 6 |
| §4 shield plus count | Task 9 |
| §5 history grouping | Task 9 |

**Placeholder scan:** no TBDs. Several steps say "find it with the grep" or
"match the existing fixture" -- Task 1 Step 1, Task 2 Step 4, Task 8 Step 4,
Task 9 Step 3. Each names what to look for and what to do with it. This plan
quotes less literal code than the macOS and GTK plans on purpose: it is the
shell I have read least, and inventing exact member names I have not
verified would be worse than pointing precisely at where to look.

**Type consistency check.** `RedactionTally.{Line, Total, NothingMatched}`
defined in Task 2, used in Task 2 Step 4. `ProjectQueueGroup.{ProjectPath,
SizeBytes, ShowSubmitAll}` and `QueueLocation` / `QueueNavigation.Resolve`
defined in Task 3, used in Tasks 4 and 9. `RedactionPlaceholders.Scan`
defined and used in Task 6. `RecentSearches.{Remember, Current, Reset}`
defined in Task 7 Step 3, used in Steps 1 and 3.
`OriginalSearchOutcome.Classify(int, int?)` defined in Task 8 Step 2, called
in Step 5; `tc_search_original` declared in Step 3 and called through
`SearchOriginal` in Steps 4 and 5. `QueueShieldState.For` and
`HistoryFolders.Group` defined and used in Task 9.

**Three differences from the other shells, all deliberate.** This shell
reaches `search_original` through the C ABI like macOS rather than the socket
like GTK; it has the prefix bug like macOS and unlike GTK; and its
`ShowSubmitAll` rule is the one place where the retired
single-entry-group rule is not just a comment but an existing, tested
property -- Task 3 Step 1 updates that test rather than deleting it.

**What this plan cannot verify from a Mac.** All of it. The interop tests
need a Windows runner and the app needs MSBuild from Visual Studio. Task 10
Step 3's hand-check on the dev VM is the only thing standing behind six of
the ten tasks' user-visible behavior.
