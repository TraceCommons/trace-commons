using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// What a group's "Submit all" offers, and what it says it is leaving behind.
///
/// <para>
/// A group approve means <b>all eligible</b>, never all. The count comes from
/// the daemon's own project row, never recomputed here: the same filter runs
/// server-side inside the approve, and three shells each reimplementing it is
/// what put the hole here in the first place.
/// </para>
/// </summary>
public class GroupContributableCountTests
{
    private static QueueEntry Entry(string id, string project) =>
        new() { EntryId = id, ProjectId = project, ProjectLabel = project, SizeBytes = 10 };

    private static ProjectQueueGroup Group(
        int entries,
        IReadOnlyDictionary<string, int>? counts) =>
        QueueGrouping.ByProject(
            Enumerable.Range(0, entries).Select(i => Entry($"e{i}", "proj")).ToList(),
            counts).Single();

    private static Dictionary<string, int> Counts(int contributable) =>
        new(StringComparer.Ordinal) { ["proj"] = contributable };

    /// <summary>
    /// AN ABSENT <c>contributable_count</c> OFFERS THE WHOLE GROUP.
    /// </summary>
    /// <remarks>
    /// The invited contributor, and the case a wrong reading silently stops
    /// people submitting. Absent is not zero: reading it as zero would take
    /// the group control away from every project they have, and they would
    /// have no way to tell that anything had gone wrong.
    /// </remarks>
    [Fact]
    public void AnAbsentCountOffersTheWholeGroupAndSaysNothingWasWithheld()
    {
        ProjectQueueGroup group = Group(7, null);

        Assert.Null(group.ContributableCount);
        Assert.Equal(7, group.Count);
        Assert.Equal(7, group.OfferedCount);
        Assert.Equal(0, group.Withheld);
        Assert.True(group.ShowSubmitAll);
        Assert.Null(ContributionEligibilitySurface.WithheldLine(group.Withheld));

        // Same answer when a map exists but carries no row for this project.
        ProjectQueueGroup other = Group(
            7, new Dictionary<string, int>(StringComparer.Ordinal) { ["elsewhere"] = 2 });
        Assert.Null(other.ContributableCount);
        Assert.Equal(7, other.OfferedCount);
        Assert.Equal(0, other.Withheld);
        Assert.True(other.ShowSubmitAll);
    }

    /// <summary>
    /// A present count is what the button offers, and the difference is what
    /// the withheld sentence reports.
    /// </summary>
    [Fact]
    public void APresentCountIsOfferedAndTheDifferenceIsWithheld()
    {
        ProjectQueueGroup group = Group(7, Counts(3));

        Assert.Equal(7, group.Count);
        Assert.Equal(3, group.OfferedCount);
        Assert.Equal(4, group.Withheld);
        Assert.True(group.ShowSubmitAll);

        string? line = ContributionEligibilitySurface.WithheldLine(group.Withheld);
        Assert.False(string.IsNullOrWhiteSpace(line));
    }

    /// <summary>
    /// A count of zero REMOVES the group's controls.
    /// </summary>
    /// <remarks>
    /// The same answer as a row, reached for a slightly different reason: the
    /// group now says in words that nothing here can be sent -- the withheld
    /// line -- and an inert button beside that sentence is an inert control
    /// with its own explanation proving it redundant.
    ///
    /// <para>
    /// Reached only when the daemon SENT a zero. An absent count offers
    /// everything, which the test above pins.
    /// </para>
    /// </remarks>
    [Fact]
    public void AZeroCountRemovesTheControls()
    {
        ProjectQueueGroup group = Group(7, Counts(0));

        Assert.Equal(0, group.OfferedCount);
        Assert.False(group.ShowSubmitAll);

        // The row is still there and still says how many it is holding: the
        // group is unoffered, never hidden.
        Assert.Equal(7, group.Count);
        Assert.Equal(7, group.Withheld);
        Assert.False(string.IsNullOrWhiteSpace(
            ContributionEligibilitySurface.WithheldLine(group.Withheld)));
    }

    /// <summary>
    /// AN ABSENT COUNT GOES OVER THE ABI AS A NEGATIVE, NEVER AS ZERO.
    /// </summary>
    /// <remarks>
    /// The trap this surface is most likely to fall into, because
    /// <c>int?</c> to <c>0</c> is the obvious null-coalesce and it is exactly
    /// wrong: zero means the question applies and nothing here can be sent,
    /// so a shell that spelled absence as zero would refuse the control to an
    /// invited contributor whose sessions are all perfectly sendable.
    ///
    /// <para>
    /// Asserted against the raw ABI at several negatives, because the
    /// contract is that ANY negative means absent -- not one sentinel this
    /// shell happens to send.
    /// </para>
    /// </remarks>
    [Fact]
    public void AnAbsentCountIsSpelledNegativeAndNeverZero()
    {
        Assert.Equal(
            ContributionControl.Contribute,
            ContributionEligibilitySurface.GroupControl(7, null));

        foreach (long absent in new long[] { -1, -7, long.MinValue + 1 })
        {
            Assert.Equal(
                ContributionControl.Contribute,
                ContributionEligibilitySurface.FromAbiControl(
                    NativeMethods.tc_contribution_group_control(7, absent)));
        }

        // And zero is emphatically not the same answer.
        Assert.Equal(
            ContributionControl.None,
            ContributionEligibilitySurface.FromAbiControl(
                NativeMethods.tc_contribution_group_control(7, 0)));
        Assert.Equal(
            ContributionControl.None,
            ContributionEligibilitySurface.GroupControl(7, 0));
    }

    /// <summary>
    /// The group control is the ABI's answer, not this shell's.
    /// </summary>
    [Fact]
    public void TheGroupControlComesFromTheAbi()
    {
        foreach ((int pending, int? contributable) in new (int, int?)[]
        {
            (7, null), (7, 0), (7, 3), (7, 7), (0, 0), (0, null), (1, 1),
        })
        {
            Assert.Equal(
                ContributionEligibilitySurface.FromAbiControl(
                    NativeMethods.tc_contribution_group_control(
                        pending, contributable ?? -1)),
                ContributionEligibilitySurface.GroupControl(pending, contributable));

            // A group with no entries is not a group at all -- ByProject
            // returns none -- so the round-trip through ProjectQueueGroup is
            // asserted only where one exists.
            if (pending > 0)
            {
                Assert.Equal(
                    ContributionEligibilitySurface.GroupControl(pending, contributable)
                        == ContributionControl.Contribute,
                    Group(pending, contributable is { } c
                        ? new Dictionary<string, int>(StringComparer.Ordinal) { ["proj"] = c }
                        : null).ShowSubmitAll);
            }
        }
    }

    /// <summary>
    /// <c>excluded_ineligible</c> decodes as absent, not as zero.
    /// </summary>
    /// <remarks>
    /// Absent for a single-entry approve and for an invited contributor,
    /// because zero would read as "nothing was left out" -- a claim about a
    /// filter that did not run. Present-and-zero is a different, meaningful
    /// answer: the filter ran and took everything.
    /// </remarks>
    [Fact]
    public void TheApproveReplyDistinguishesAnAbsentExclusionFromZero()
    {
        ApprovalHold absent = JsonSerializer.Deserialize<ApprovalHold>(
            "{\"approved\":1,\"hold_secs\":5}")!;
        Assert.Null(absent.ExcludedIneligible);

        ApprovalHold ranAndTookEverything = JsonSerializer.Deserialize<ApprovalHold>(
            "{\"approved\":7,\"hold_secs\":5,\"excluded_ineligible\":0}")!;
        Assert.Equal(0, ranAndTookEverything.ExcludedIneligible);
        Assert.NotNull(ranAndTookEverything.ExcludedIneligible);
        // Present-and-zero still draws no line: there is no gap to explain.
        Assert.Null(ContributionEligibilitySurface.WithheldLine(
            ranAndTookEverything.ExcludedIneligible!.Value));

        ApprovalHold leftSomeBehind = JsonSerializer.Deserialize<ApprovalHold>(
            "{\"approved\":3,\"hold_secs\":5,\"excluded_ineligible\":4}")!;
        Assert.Equal(4, leftSomeBehind.ExcludedIneligible);
        Assert.False(string.IsNullOrWhiteSpace(
            ContributionEligibilitySurface.WithheldLine(
                leftSomeBehind.ExcludedIneligible!.Value)));

        // Excluded entries are never in skipped: they were never selected.
        Assert.Empty(leftSomeBehind.Skipped);
    }

    /// <summary>
    /// Every session is still counted in the group, whatever its eligibility.
    /// </summary>
    [Fact]
    public void EverySessionIsStillInTheGroup()
    {
        Assert.Equal(7, Group(7, Counts(1)).Count);
        Assert.Equal(7, Group(7, Counts(0)).Count);
        Assert.Equal(7, Group(7, null).Count);
    }

    /// <summary>
    /// The withheld sentence says nothing for zero and nothing for a
    /// negative.
    /// </summary>
    /// <remarks>
    /// A negative is what a race between the daemon's count and the rows on
    /// screen produces. It must not wrap into a huge number, and it must not
    /// be clamped into a confident "0 withheld" either -- silence is the
    /// honest answer to a number that cannot be right.
    /// </remarks>
    [Fact]
    public void TheWithheldSentenceIsSilentForZeroAndForANegative()
    {
        foreach (long quiet in new long[] { 0, -1, -7, long.MinValue + 1 })
        {
            Assert.Null(ContributionEligibilitySurface.WithheldLine(quiet));
        }

        foreach (long spoken in new long[] { 1, 2, 4, 999 })
        {
            Assert.False(
                string.IsNullOrWhiteSpace(ContributionEligibilitySurface.WithheldLine(spoken)));
        }

        // A race that reads the count after more rows arrived: silent, not a
        // wrapped number.
        ProjectQueueGroup raced = Group(2, Counts(5));
        Assert.Equal(-3, raced.Withheld);
        Assert.Null(ContributionEligibilitySurface.WithheldLine(raced.Withheld));
    }

    /// <summary>
    /// The sentence says HOW MANY and not why.
    /// </summary>
    /// <remarks>
    /// A summary here would stand for up to thirteen different reasons and
    /// say nothing true about any of them. The reason a particular session
    /// cannot be sent is that row's own sentence, one level in.
    /// </remarks>
    [Fact]
    public void TheWithheldSentenceNamesNoReason()
    {
        string line = ContributionEligibilitySurface.WithheldLine(4)!;
        PrivateInferenceCopy copy = PrivateInferenceSurface.Copy()!;

        foreach (string reason in new[]
        {
            copy.EligibilityReasonNoCall,
            copy.EligibilityReasonCaptureOff,
            copy.EligibilityReasonMarkerAbsent,
            copy.EligibilityIneligiblePermanent,
        })
        {
            Assert.NotEqual(reason, line);
            Assert.DoesNotContain(line, reason, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// Both counts decode off the project row, and an absent
    /// <c>contributable_count</c> stays null rather than becoming zero.
    /// </summary>
    [Fact]
    public void TheProjectRowDecodesBothCounts()
    {
        ProjectSetting withCount = JsonSerializer.Deserialize<ProjectSetting>(
            "{\"project_id\":\"p\",\"pending_count\":7,\"contributable_count\":3}")!;
        Assert.Equal(7, withCount.PendingCount);
        Assert.Equal(3, withCount.ContributableCount);

        ProjectSetting withoutCount = JsonSerializer.Deserialize<ProjectSetting>(
            "{\"project_id\":\"p\",\"pending_count\":7}")!;
        Assert.Equal(7, withoutCount.PendingCount);
        Assert.Null(withoutCount.ContributableCount);

        // A real zero is a zero, and stays distinguishable from absent.
        ProjectSetting zero = JsonSerializer.Deserialize<ProjectSetting>(
            "{\"project_id\":\"p\",\"pending_count\":7,\"contributable_count\":0}")!;
        Assert.Equal(0, zero.ContributableCount);
        Assert.NotNull(zero.ContributableCount);
    }
}
