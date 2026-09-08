using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The half of this surface a contributor actually looks at.
///
/// <para>
/// <c>TraceCommons.App</c> is a WinUI project and cannot be built, let alone
/// referenced, on the macOS and Linux machines this suite runs on -- so
/// without these, nothing anywhere checks that the decisions in
/// <see cref="ContributionEligibilitySurface"/> ever reach a screen. Asserted
/// about the sources, the same way the routing and credential surfaces are:
/// a view that re-derived a send control from a state string would pass every
/// behavioural test in <see cref="ContributionEligibilityTests"/>.
/// </para>
/// </summary>
public class EligibilityShellWiringTests
{
    /// <summary>
    /// The row asks the shared crate for its eligibility rather than reading
    /// the wire field itself.
    /// </summary>
    [Fact]
    public void TheRowAsksTheSharedCrate()
    {
        string source = ShellSource("TraceCommons.App/ViewModels/QueueEntryViewModel.cs");
        Assert.Contains("ContributionEligibilitySurface.Decide(", source, StringComparison.Ordinal);

        // Never the raw wire strings. A row that compared the state itself
        // would be a fourth place the branch table lives, and it would go on
        // agreeing with an old one after the Rust changed an arm.
        foreach (string state in new[]
        {
            "\"eligible\"", "\"ineligible_permanent\"", "\"ineligible_configuration\"",
        })
        {
            Assert.DoesNotContain(state, source, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// EVERY SESSION IS SHOWN. The queue draws no eligibility condition on
    /// anything that decides whether a row exists.
    /// </summary>
    /// <remarks>
    /// The rule this whole surface turns on. Hiding a contributor's own work
    /// is its own dishonesty and makes this app look as though it had not
    /// noticed files they know it can see, so the only thing eligibility may
    /// govern is the send control -- checked by name below.
    /// </remarks>
    [Fact]
    public void EligibilityGatesTheSendControlAndNothingElse()
    {
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");

        // The one place CanContribute is allowed to appear: the Submit
        // button's own visibility.
        Assert.Single(Regex.Matches(markup, @"\{x:Bind CanContribute[^}]*\}"));
        Assert.Contains(
            "Visibility=\"{x:Bind CanContribute, Mode=OneWay}\"",
            markup,
            StringComparison.Ordinal);

        // Nothing eligibility-derived reaches the ItemsSource, the repeater or
        // any row-level Visibility other than the three sentence arms.
        foreach (string forbidden in new[]
        {
            "ItemsSource=\"{x:Bind CanContribute",
            "ItemsSource=\"{x:Bind HasEligibilityText",
            "ItemsSource=\"{x:Bind EligibilityIsPlain",
        })
        {
            Assert.DoesNotContain(forbidden, markup, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The sentence and its reason are bound, never typed, and every arm of
    /// the tone is drawn.
    /// </summary>
    /// <remarks>
    /// All three arms, because a row that carries no send control has to say
    /// why. A tone with no arm would leave that row silent, which is the one
    /// outcome forbidden here -- see
    /// <see cref="QueueEntryViewModel"/>'s <c>EligibilityIsPlain</c>, which is
    /// the complement of the other two rather than a fourth named tone.
    /// </remarks>
    [Fact]
    public void TheRowDrawsTheSharedSentences()
    {
        string markup = ShellSource("TraceCommons.App/MainWindow.xaml");
        foreach (string bound in new[]
        {
            "Text=\"{x:Bind EligibilityText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind EligibilityIsPlain, Mode=OneWay}\"",
            "Visibility=\"{x:Bind EligibilityIsClear, Mode=OneWay}\"",
            "Visibility=\"{x:Bind EligibilityIsAttention, Mode=OneWay}\"",
            "Text=\"{x:Bind EligibilityReasonText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind HasEligibilityReason, Mode=OneWay}\"",
        })
        {
            Assert.Contains(bound, markup, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The preview sheet's Contribute is held by the same rule.
    /// </summary>
    /// <remarks>
    /// "Look inside" is a second route to the send. A button the row withheld
    /// would otherwise be waiting one click behind it, which is the same
    /// defect with an extra step in front of it.
    /// </remarks>
    [Fact]
    public void TheSheetsContributeIsHeldByTheSameRule()
    {
        string source = ShellSource("TraceCommons.App/ViewModels/PreviewSheetViewModel.cs");
        Assert.Matches(
            new Regex(@"public bool CanContribute =>[^;]*Entry\.CanContribute", RegexOptions.Singleline),
            source);

        string markup = ShellSource("TraceCommons.App/Controls/PreviewSheet.xaml");
        foreach (string bound in new[]
        {
            "Text=\"{x:Bind ViewModel.EligibilityText, Mode=OneWay}\"",
            "Text=\"{x:Bind ViewModel.EligibilityReasonText, Mode=OneWay}\"",
            "Visibility=\"{x:Bind ViewModel.HasEligibilityText, Mode=OneWay}\"",
        })
        {
            Assert.Contains(bound, markup, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// Both send paths re-check the gate AT THE ACTION, not only in markup.
    /// </summary>
    /// <remarks>
    /// The GTK shell had a correction-refusal path that re-armed Contribute
    /// by setting the widget directly, undoing a gate that had been applied
    /// once at draw time. WinUI cannot reproduce that shape -- <c>x:Bind</c>
    /// OneWay pulls from the getter, so every re-raise re-evaluates
    /// <c>Entry.CanContribute</c> rather than pushing a stale value -- but a
    /// send reached by any route OTHER than the button is the same defect,
    /// and a rule that lives in one markup attribute is one caller away from
    /// being bypassed. Both entry points assert it themselves.
    /// </remarks>
    [Fact]
    public void BothSendPathsRecheckTheGateAtTheAction()
    {
        Assert.Matches(
            new Regex(
                @"public async Task SubmitEntryAsync\(QueueEntryViewModel entry\)\s*\{"
                + @"(?:(?!ClearUndo|CallAsync).)*?if \(!entry\.CanContribute\)\s*\{\s*return;",
                RegexOptions.Singleline),
            ShellSource("TraceCommons.App/ViewModels/MainViewModel.cs"));

        Assert.Matches(
            new Regex(
                @"public async Task ContributeAsync\(\)\s*\{"
                + @"(?:(?!CallAsync).)*?if \(!CanContribute\)\s*\{\s*return;",
                RegexOptions.Singleline),
            ShellSource("TraceCommons.App/ViewModels/PreviewSheetViewModel.cs"));
    }

    /// <summary>
    /// Nothing sets a send control's state imperatively.
    /// </summary>
    /// <remarks>
    /// The direct check for the GTK shape. A control whose enabled or visible
    /// state is assigned in code has escaped its binding, and the assignment
    /// is free to disagree with the gate. Neither send control is even NAMED
    /// in the markup, so nothing in code-behind can address one; this asserts
    /// that stays true.
    /// </remarks>
    [Fact]
    public void NoSendControlIsAddressableFromCode()
    {
        foreach (string markupPath in new[]
        {
            "TraceCommons.App/MainWindow.xaml",
            "TraceCommons.App/Controls/PreviewSheet.xaml",
        })
        {
            string markup = ShellSource(markupPath);
            foreach (Match button in Regex.Matches(
                markup, @"<Button\b[^>]*Content=""(?:Submit|Contribute)""[^>]*>", RegexOptions.Singleline))
            {
                Assert.DoesNotContain("x:Name", button.Value, StringComparison.Ordinal);
            }
        }
    }

    /// <summary>
    /// THE SHEET'S GATE READS THE LIVE QUEUE, NOT THE COPY IT OPENED WITH.
    /// </summary>
    /// <remarks>
    /// The macOS shell held its entry as a value captured when the sheet
    /// opened, computed the gate from it once, and never re-consulted the
    /// queue -- so a session downgraded by a submit-time write-back went on
    /// offering Contribute for as long as the sheet stayed up. No write undid
    /// the gate; the gate read a copy that had stopped being true.
    ///
    /// <para>
    /// Windows had the same shape: <c>MainViewModel.ReplacePending</c> clears
    /// and refills rather than diffing, so every row object is replaced and
    /// an open sheet keeps one nobody updates.
    /// </para>
    /// </remarks>
    [Fact]
    public void TheSheetsGateReadsTheLiveEntry()
    {
        string source = ShellSource("TraceCommons.App/ViewModels/PreviewSheetViewModel.cs");

        Assert.Matches(
            new Regex(
                @"public bool CanContribute =>[^;]*LiveEntry\.CanContribute",
                RegexOptions.Singleline),
            source);

        // Every sentence beside the button too: a gate that closed while its
        // explanation went on saying the old thing is its own dishonesty.
        foreach (string live in new[]
        {
            "HasEligibilityText => LiveEntry.HasEligibilityText",
            "EligibilityText => LiveEntry.EligibilityText",
            "HasEligibilityReason => LiveEntry.HasEligibilityReason",
            "EligibilityReasonText => LiveEntry.EligibilityReasonText",
        })
        {
            Assert.Contains(live, source, StringComparison.Ordinal);
        }

        // And none of them reads the pinned copy any more.
        foreach (string pinned in new[]
        {
            "&& Entry.CanContribute",
            "=> Entry.HasEligibilityText",
            "=> Entry.EligibilityText",
            "=> Entry.HasEligibilityReason",
            "=> Entry.EligibilityReasonText",
        })
        {
            Assert.DoesNotContain(pinned, source, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The resolution falls back to the pinned copy, and reads the queue in
    /// BOTH directions.
    /// </summary>
    /// <remarks>
    /// Not "assume the worst": an entry the daemon upgrades becomes offerable
    /// without closing the sheet, and an entry that has left the queue is not
    /// evidence that it became ineligible. The rule is read the queue, and
    /// fall back to what this sheet was handed when there is no queue to
    /// read.
    /// </remarks>
    [Fact]
    public void TheResolutionFallsBackToThePinnedCopy()
    {
        Assert.Matches(
            new Regex(
                @"private QueueEntryViewModel LiveEntry =>\s*"
                + @"\(_liveEntry is null \? null : _liveEntry\(Entry\.EntryId\)\) \?\? Entry;",
                RegexOptions.Singleline),
            ShellSource("TraceCommons.App/ViewModels/PreviewSheetViewModel.cs"));

        // The queue side returns null rather than a substitute when the entry
        // is gone, which is what makes that fallback reachable.
        Assert.Matches(
            new Regex(
                @"public QueueEntryViewModel\? LiveEntry\(string entryId\) =>\s*"
                + @"_rowsByEntryId\.TryGetValue\(entryId, out QueueEntryViewModel\? row\) \? row : null;",
                RegexOptions.Singleline),
            ShellSource("TraceCommons.App/ViewModels/MainViewModel.cs"));
    }

    /// <summary>
    /// What the sheet APPROVES still goes through the pinned entry.
    /// </summary>
    /// <remarks>
    /// The entry id is identical either way, so this changes nothing a
    /// contributor can observe -- which is the point. Reading the live copy
    /// is for the gate and its sentences; the thing being sent must not move
    /// under somebody mid-read.
    /// </remarks>
    [Fact]
    public void WhatIsApprovedStillGoesThroughThePinnedEntry()
    {
        string source = ShellSource("TraceCommons.App/ViewModels/PreviewSheetViewModel.cs");
        Assert.Contains(
            "SubmitParams.ForEntry(Entry.EntryId, SelectedVerdict, CorrectionToSend)",
            source,
            StringComparison.Ordinal);
        Assert.DoesNotContain("LiveEntry.EntryId", source, StringComparison.Ordinal);
        Assert.DoesNotContain("SubmitParams.ForEntry(LiveEntry", source, StringComparison.Ordinal);
    }

    /// <summary>
    /// The sheet is TOLD when the queue changes.
    /// </summary>
    /// <remarks>
    /// Resolving live is only half the fix. These properties are pull-bound,
    /// so without a raise the footer keeps drawing the answer it last read.
    /// <c>Pending</c> is cleared and refilled on every refresh, so its
    /// CollectionChanged is the queue's own signal that the rows were
    /// replaced -- and the handler is removed on close, so a sheet that has
    /// gone cannot be kept alive by it.
    /// </remarks>
    [Fact]
    public void TheSheetIsToldWhenTheQueueChanges()
    {
        string source = ShellSource("TraceCommons.App/MainWindow.xaml.cs");
        Assert.Contains(
            "new PreviewWindow(_host, entry, ViewModel.LiveEntry)",
            source,
            StringComparison.Ordinal);
        Assert.Contains(
            "ViewModel.Pending.CollectionChanged += OnQueueChanged;",
            source,
            StringComparison.Ordinal);
        Assert.Contains(
            "ViewModel.Pending.CollectionChanged -= OnQueueChanged;",
            source,
            StringComparison.Ordinal);

        Assert.Matches(
            new Regex(
                @"public void QueueChanged\(\)\s*\{(?:(?!\}).)*Raise\(nameof\(CanContribute\)\)",
                RegexOptions.Singleline),
            ShellSource("TraceCommons.App/ViewModels/PreviewSheetViewModel.cs"));
    }

    /// <summary>
    /// The live-entry resolver is REQUIRED at every layer it passes through.
    /// </summary>
    /// <remarks>
    /// An optional parameter whose omission restores a bug is a trap for
    /// whoever adds the next construction site: they get no test failure and
    /// no warning, just a sheet that quietly stops re-reading the queue. A
    /// required parameter fails at compile time at the forgotten call site,
    /// which is the earliest failure available here -- and it is the only one
    /// available, because none of these three types can be constructed in
    /// this suite at all.
    ///
    /// <para>
    /// The type stays nullable so a caller with genuinely no queue to resolve
    /// against can pass <c>liveEntry: null</c> and say why. What must not
    /// exist is a DEFAULT, which would make forgetting indistinguishable from
    /// choosing.
    /// </para>
    /// </remarks>
    [Fact]
    public void TheLiveEntryResolverIsRequiredAtEveryLayer()
    {
        foreach (string path in new[]
        {
            "TraceCommons.App/ViewModels/PreviewSheetViewModel.cs",
            "TraceCommons.App/Controls/PreviewSheet.xaml.cs",
            "TraceCommons.App/PreviewWindow.cs",
        })
        {
            string source = ShellSource(path);

            Assert.Contains(
                "Func<string, QueueEntryViewModel?>? liveEntry)",
                source,
                StringComparison.Ordinal);

            Assert.DoesNotContain(
                "liveEntry = null",
                source,
                StringComparison.Ordinal);
        }

        // And the one site that constructs the chain supplies it, so nothing
        // is relying on a default that no longer exists.
        Assert.Contains(
            "new PreviewWindow(_host, entry, ViewModel.LiveEntry)",
            ShellSource("TraceCommons.App/MainWindow.xaml.cs"),
            StringComparison.Ordinal);
        Assert.Contains(
            "new PreviewSheet(host, entry, liveEntry)",
            ShellSource("TraceCommons.App/PreviewWindow.cs"),
            StringComparison.Ordinal);
        Assert.Contains(
            "new PreviewSheetViewModel(host, entry, liveEntry)",
            ShellSource("TraceCommons.App/Controls/PreviewSheet.xaml.cs"),
            StringComparison.Ordinal);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
