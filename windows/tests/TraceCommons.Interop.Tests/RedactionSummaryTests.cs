using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The panel that answers "so I can right away see what doesn't go". It is
/// also the first surface with room to say two different things, which is
/// where the <c>residual_secret_at</c> defect stops being stated backwards.
/// </summary>
public class RedactionSummaryTests
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
    public void AnEmptyMapProducesNoRows()
    {
        (IReadOnlyList<RedactionSummaryRow> removed,
         IReadOnlyList<RedactionSummaryRow> still) = RedactionSummary.Rows(Map(), Map());

        Assert.Empty(removed);
        Assert.Empty(still);
    }

    [Fact]
    public void OneFamilyBecomesOneRow()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) = RedactionSummary.Rows(
            Map(("local_path", 185)),
            Map(("local_path", 12)));

        RedactionSummaryRow row = Assert.Single(removed);
        Assert.Equal("local_path", row.Family);
        Assert.Equal("local path", row.Display);
        Assert.Equal(185, row.Occurrences);
        Assert.Equal(12, row.Distinct);
        Assert.NotEmpty(row.Description);
        Assert.NotEqual(RedactionSummary.UnknownDescription, row.Description);
    }

    /// <summary>
    /// A session that tripped nine different secret patterns is one row
    /// summing them, with the sub-labels on a detail line, not nine rows.
    /// </summary>
    [Fact]
    public void SubLabelsCollapseIntoTheirFamily()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) = RedactionSummary.Rows(
            Map(("secret:contextual_entropy", 3), ("secret:pem_private_key", 2), ("secret", 1)),
            Map(("secret:contextual_entropy", 3), ("secret:pem_private_key", 1), ("secret", 1)));

        RedactionSummaryRow row = Assert.Single(removed);
        Assert.Equal("secret", row.Family);
        Assert.Equal(6, row.Occurrences);
        Assert.Equal(5, row.Distinct);
        Assert.Equal(new[] { "contextual entropy", "pem private key" }, row.Detail);
    }

    /// <summary>
    /// A secret DETECTED AND NOT REMOVED. Putting it in Removed would state
    /// the exact opposite of what happened.
    /// </summary>
    [Fact]
    public void AResidualSurvivorIsReportedAsStillPresent()
    {
        (IReadOnlyList<RedactionSummaryRow> removed,
         IReadOnlyList<RedactionSummaryRow> still) = RedactionSummary.Rows(
            Map(("local_path", 3), ("residual_secret_at:events.correction", 1)),
            Map());

        Assert.Equal(new[] { "local_path" }, removed.Select(r => r.Family));
        Assert.Equal(new[] { "residual_secret_at" }, still.Select(r => r.Family));
        Assert.Equal(new[] { "events.correction" }, still[0].Detail);
    }

    /// <summary>
    /// Hiding a category this build has no words for would understate what
    /// happened, which is the one direction this panel must not fail in.
    /// </summary>
    [Fact]
    public void AnUnknownFamilyIsKeptWithANeutralDescription()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) =
            RedactionSummary.Rows(Map(("some_future_family:thing", 2)), Map());

        RedactionSummaryRow row = Assert.Single(removed);
        Assert.Equal("some_future_family", row.Family);
        Assert.Equal(RedactionSummary.UnknownDescription, row.Description);
        Assert.Equal(2, row.Occurrences);
    }

    [Fact]
    public void RowsAreOrderedByOccurrencesThenFamily()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) = RedactionSummary.Rows(
            Map(("secret", 3), ("local_path", 185), ("email", 3)),
            Map());

        Assert.Equal(new[] { "local_path", "email", "secret" }, removed.Select(r => r.Family));
    }

    [Fact]
    public void ARowCarriesNoMatchedText()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) =
            RedactionSummary.Rows(Map(("local_path", 3)), Map());

        Assert.Empty(Assert.Single(removed).Detail);
    }

    /// <summary>
    /// The daemon reports no distinct counts at all against an older build.
    /// Zero means "no figure to show", never "zero distinct values", and the
    /// panel has to be able to tell those apart.
    /// </summary>
    [Fact]
    public void ARowFromAnOlderDaemonReportsNoDistinctFigure()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) =
            RedactionSummary.Rows(Map(("local_path", 185)), Map());

        Assert.Equal(0, Assert.Single(removed).Distinct);
    }

    /// <summary>
    /// The row's figure omits the distinct count on exactly the terms the
    /// card's line omits it, so the two surfaces cannot disagree about one
    /// session.
    /// </summary>
    [Fact]
    public void TheFigureShowsADistinctCountOnlyWhenItSaysSomethingNew()
    {
        (IReadOnlyList<RedactionSummaryRow> differing, _) =
            RedactionSummary.Rows(Map(("local_path", 185)), Map(("local_path", 12)));
        (IReadOnlyList<RedactionSummaryRow> equal, _) =
            RedactionSummary.Rows(Map(("secret", 3)), Map(("secret", 3)));
        (IReadOnlyList<RedactionSummaryRow> unreported, _) =
            RedactionSummary.Rows(Map(("secret", 3)), Map());

        Assert.Equal("185 (12 distinct)", differing[0].CountText);
        Assert.Equal("3", equal[0].CountText);
        Assert.Equal("3", unreported[0].CountText);
    }

    [Fact]
    public void ARowWithNoSubLabelsDrawsNoDetailLine()
    {
        (IReadOnlyList<RedactionSummaryRow> bare, _) =
            RedactionSummary.Rows(Map(("local_path", 3)), Map());
        (IReadOnlyList<RedactionSummaryRow> subLabelled, _) =
            RedactionSummary.Rows(Map(("secret:pem_private_key", 1)), Map());

        Assert.False(bare[0].HasDetail);
        Assert.Equal("", bare[0].DetailText);
        Assert.True(subLabelled[0].HasDetail);
        Assert.Equal("pem private key", subLabelled[0].DetailText);
    }
}
