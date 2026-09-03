using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// <c>redaction_counts</c> carries two different kinds of fact under one map,
/// and every shell has been rendering both under the heading "Removed by
/// pattern". These tests are the boundary between them.
/// </summary>
public class RedactionLabelsTests
{
    private static Dictionary<string, int> Map(params (string Label, int Count)[] pairs)
    {
        var map = new Dictionary<string, int>();
        foreach ((string label, int count) in pairs)
        {
            map[label] = count;
        }
        return map;
    }

    [Fact]
    public void AFamilyIsTheLabelBeforeItsColon()
    {
        Assert.Equal("secret", RedactionLabels.Family("secret:contextual_entropy"));
        Assert.Equal("local_path", RedactionLabels.Family("local_path"));
        Assert.Equal(
            "residual_secret_at",
            RedactionLabels.Family("residual_secret_at:events.3.correction"));
        Assert.Equal("", RedactionLabels.Family(""));
    }

    [Fact]
    public void AnOrdinaryLabelIsARemoval()
    {
        Assert.True(RedactionLabels.IsRemoval("local_path"));
        Assert.True(RedactionLabels.IsRemoval("secret"));
        Assert.True(RedactionLabels.IsRemoval("secret:pem_private_key"));
        Assert.True(RedactionLabels.IsRemoval("privacy_filter:person_name"));
    }

    /// <summary>
    /// The whole point of the type. <c>residual_secret_at</c> counts a secret
    /// that was DETECTED AND LEFT IN, so counting it as removed states the
    /// exact opposite of what happened.
    /// </summary>
    [Fact]
    public void AResidualSurvivorIsNotARemoval()
        => Assert.False(RedactionLabels.IsRemoval("residual_secret_at:events.correction"));

    [Fact]
    public void RemovedTotalExcludesSurvivors()
    {
        var counts = Map(
            ("local_path", 185),
            ("secret", 3),
            ("residual_secret_at:events.correction", 1));
        Assert.Equal(188, RedactionLabels.RemovedTotal(counts));
        Assert.Equal(2, RedactionLabels.Removals(counts).Count);
        Assert.DoesNotContain(
            "residual_secret_at:events.correction",
            RedactionLabels.Removals(counts).Keys);
    }

    /// <summary>
    /// A session that removed nothing and left a secret in reports zero
    /// removals -- which is what puts the card in the tone that asks somebody
    /// to look.
    /// </summary>
    [Fact]
    public void ASessionWithOnlyASurvivorRemovedNothing()
    {
        var counts = Map(("residual_secret_at:events.x", 1));
        Assert.Equal(0, RedactionLabels.RemovedTotal(counts));
        Assert.Empty(RedactionLabels.Removals(counts));
        Assert.Equal(1, RedactionLabels.SurvivorTotal(counts));
    }

    /// <summary>
    /// Filtering a survivor out of the figure without showing it anywhere
    /// would trade a wrong statement for silence about a secret still in the
    /// payload. These are the accessors that stop that happening.
    /// </summary>
    [Fact]
    public void SurvivorsAreReportedWithTheirSites()
    {
        var counts = Map(
            ("local_path", 3),
            ("residual_secret_at:events.9.correction", 2),
            ("residual_secret_at:events.1.tool_result", 1));
        Assert.Equal(3, RedactionLabels.SurvivorTotal(counts));
        Assert.Equal(
            new[] { "events.1.tool_result", "events.9.correction" },
            RedactionLabels.SurvivorSites(counts));
    }

    [Fact]
    public void ASessionWithNoSurvivorsHasNoLine()
    {
        var counts = Map(("local_path", 3));
        Assert.Equal(0, RedactionLabels.SurvivorTotal(counts));
        Assert.Equal("", RedactionLabels.SurvivorLine(counts));
        Assert.Empty(RedactionLabels.SurvivorSites(counts));
    }

    [Fact]
    public void TheSurvivorLineInflectsAndNamesItsSites()
    {
        Assert.Equal(
            "1 secret found here is still in what would be sent (events.x)",
            RedactionLabels.SurvivorLine(Map(("residual_secret_at:events.x", 1))));
        Assert.StartsWith(
            "2 secrets found here are",
            RedactionLabels.SurvivorLine(Map(
                ("residual_secret_at:events.x", 1),
                ("residual_secret_at:events.y", 1))));
    }

    /// <summary>
    /// The line must say STILL IN, never anything that reads as removal --
    /// stating the opposite is the defect this change exists to fix.
    /// </summary>
    [Fact]
    public void TheLineNeverClaimsTheSecretWasRemoved()
    {
        string line = RedactionLabels.SurvivorLine(
            Map(("residual_secret_at:events.3.correction", 1)));
        Assert.Contains("still in what would be sent", line);
        Assert.DoesNotContain("removed", line.ToLowerInvariant());
    }

    /// <summary>
    /// A bare <c>residual_secret_at</c> with no site still counts. It should
    /// never be minted, but dropping it would be the one failure direction
    /// that matters: silence about a surviving secret.
    /// </summary>
    [Fact]
    public void ASurvivorWithNoSiteIsStillCounted()
    {
        var counts = Map(("residual_secret_at", 1));
        Assert.Equal(1, RedactionLabels.SurvivorTotal(counts));
        Assert.Equal(0, RedactionLabels.RemovedTotal(counts));
        Assert.Empty(RedactionLabels.SurvivorSites(counts));
        Assert.Equal(
            "1 secret found here is still in what would be sent",
            RedactionLabels.SurvivorLine(counts));
    }

    /// <summary>
    /// The card's "removed by pattern" figure carries two different numbers --
    /// how many times a pattern fired, and how many distinct values that was
    /// -- and dropping either one misstates the reach of scrubbing.
    /// </summary>
    [Fact]
    public void AnEmptyTallyIsNothingMatched()
    {
        Assert.Equal(RedactionLabels.NothingMatched, RedactionLabels.Line(Map(), Map()));
        Assert.Equal(0, RedactionLabels.Total(Map()));
    }

    [Fact]
    public void LabelsAreHumanReadable()
        => Assert.Equal("3 local path", RedactionLabels.Line(Map(("local_path", 3)), Map()));

    [Fact]
    public void DistinctCountsAreShownWhenTheyDifferFromOccurrences()
        => Assert.Equal(
            "185 local path (12 distinct)",
            RedactionLabels.Line(Map(("local_path", 185)), Map(("local_path", 12))));

    [Fact]
    public void DistinctIsOmittedWhenEveryOccurrenceIsItsOwnValue()
        // "3 secret (3 distinct)" says the same thing twice.
        => Assert.Equal(
            "3 secret",
            RedactionLabels.Line(Map(("secret", 3)), Map(("secret", 3))));

    [Fact]
    public void DistinctIsOmittedWhenTheDaemonDidNotReportIt()
        => Assert.Equal("3 secret", RedactionLabels.Line(Map(("secret", 3)), Map()));

    [Fact]
    public void ADistinctCountAboveItsOccurrenceCountIsIgnored()
        // Impossible from a correct daemon; "3 secret (9 distinct)" would be
        // worse than saying nothing.
        => Assert.Equal(
            "3 secret",
            RedactionLabels.Line(Map(("secret", 3)), Map(("secret", 9))));

    [Fact]
    public void TheBiggestCountLeadsAndTiesBreakOnLabel()
        => Assert.Equal(
            "185 local path  \u00b7  3 email  \u00b7  3 secret",
            RedactionLabels.Line(
                Map(("secret", 3), ("local_path", 185), ("email", 3)),
                Map()));

    [Fact]
    public void TotalSumsOccurrencesNotDistinct()
        => Assert.Equal(5, RedactionLabels.Total(Map(("a", 2), ("b", 3))));

    /// <summary>
    /// <c>residual_secret_at:*</c> counts a secret that was DETECTED AND NOT
    /// REMOVED, and this line renders under the heading "Removed by pattern",
    /// so including it would state the exact opposite of what happened -- on
    /// the screen where someone is deciding whether to send the thing.
    /// </summary>
    [Fact]
    public void ATallyDoesNotCountASurvivorAsRemoved()
    {
        var counts = Map(("local_path", 3), ("residual_secret_at:events.correction", 1));

        Assert.Equal("3 local path", RedactionLabels.Line(counts, Map()));
        Assert.Equal(3, RedactionLabels.Total(counts));
    }

    /// <summary>A session whose only count is a survivor removed nothing.</summary>
    [Fact]
    public void ATallyOfOnlyASurvivorMatchedNothing()
    {
        var counts = Map(("residual_secret_at:events.x", 1));

        Assert.Equal(RedactionLabels.NothingMatched, RedactionLabels.Line(counts, Map()));
        Assert.Equal(0, RedactionLabels.Total(counts));
    }
}
