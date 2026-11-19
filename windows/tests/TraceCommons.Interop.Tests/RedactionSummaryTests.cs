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
         IReadOnlyList<RedactionSummaryRow> still) = RedactionSummary.Rows(Map(), Map())!.Value;

        Assert.Empty(removed);
        Assert.Empty(still);
    }

    [Fact]
    public void OneFamilyBecomesOneRow()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) = RedactionSummary.Rows(
            Map(("local_path", 185)),
            Map(("local_path", 12)))!.Value;

        RedactionSummaryRow row = Assert.Single(removed);
        Assert.Equal("local_path", row.Family);
        Assert.Equal("local path", row.Display);
        Assert.Equal(185, row.Occurrences);
        Assert.Equal(12, row.Distinct);
        Assert.NotEmpty(row.Description);
        Assert.NotEqual(UnknownFamilyDescription(), row.Description);
    }

    /// <summary>
    /// A session that tripped nine different secret patterns is one row
    /// summing them, with the sub-labels on a detail line, not nine rows.
    /// </summary>
    /// <remarks>
    /// No distinct counts here, and that is not an omission. They come from
    /// the placeholder map, which is filled only for <c>local_path</c>,
    /// <c>private_email</c> and the #1362 tool-output labels. A secret will
    /// never carry a distinct figure, so a
    /// fixture asserting one would be testing a daemon that does not exist.
    /// </remarks>
    [Fact]
    public void SubLabelsCollapseIntoTheirFamily()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) = RedactionSummary.Rows(
            Map(("secret:contextual_entropy", 3), ("secret:pem_private_key", 2), ("secret", 1)),
            Map())!.Value;

        RedactionSummaryRow row = Assert.Single(removed);
        Assert.Equal("secret", row.Family);
        Assert.Equal(6, row.Occurrences);
        Assert.Equal(0, row.Distinct);
        Assert.Equal("6", row.CountText);
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
            Map())!.Value;

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
            RedactionSummary.Rows(Map(("some_future_family:thing", 2)), Map())!.Value;

        RedactionSummaryRow row = Assert.Single(removed);
        Assert.Equal("some_future_family", row.Family);
        Assert.Equal(UnknownFamilyDescription(), row.Description);
        Assert.NotEqual(
            RedactionSummary.Rows(Map(("local_path", 1)), Map())!.Value.Removed[0].Description,
            row.Description);
        Assert.Equal(2, row.Occurrences);
    }

    [Fact]
    public void RowsAreOrderedByOccurrencesThenFamily()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) = RedactionSummary.Rows(
            Map(("secret", 3), ("local_path", 185), ("email", 3)),
            Map())!.Value;

        Assert.Equal(new[] { "local_path", "email", "secret" }, removed.Select(r => r.Family));
    }

    [Fact]
    public void ARowCarriesNoMatchedText()
    {
        (IReadOnlyList<RedactionSummaryRow> removed, _) =
            RedactionSummary.Rows(Map(("local_path", 3)), Map())!.Value;

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
            RedactionSummary.Rows(Map(("local_path", 185)), Map())!.Value;

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
            RedactionSummary.Rows(Map(("local_path", 185)), Map(("local_path", 12)))!.Value;
        (IReadOnlyList<RedactionSummaryRow> equal, _) =
            RedactionSummary.Rows(Map(("local_path", 3)), Map(("local_path", 3)))!.Value;
        (IReadOnlyList<RedactionSummaryRow> unreported, _) =
            RedactionSummary.Rows(Map(("secret", 3)), Map())!.Value;

        Assert.Equal("185 (12 distinct)", differing[0].CountText);
        Assert.Equal("3", equal[0].CountText);
        Assert.Equal("3", unreported[0].CountText);
    }

    [Fact]
    public void ARowWithNoSubLabelsDrawsNoDetailLine()
    {
        (IReadOnlyList<RedactionSummaryRow> bare, _) =
            RedactionSummary.Rows(Map(("local_path", 3)), Map())!.Value;
        (IReadOnlyList<RedactionSummaryRow> subLabelled, _) =
            RedactionSummary.Rows(Map(("secret:pem_private_key", 1)), Map())!.Value;

        Assert.False(bare[0].HasDetail);
        Assert.Equal("", bare[0].DetailText);
        Assert.True(subLabelled[0].HasDetail);
        Assert.Equal("pem private key", subLabelled[0].DetailText);
    }

    /// <summary>The core's neutral description for a family it has no words for.</summary>
    private static string UnknownFamilyDescription() =>
        RedactionSummary.Rows(Map(("another_future_family", 1)), Map())!.Value.Removed[0].Description;

    /// <summary>
    /// The rows are the core's (<c>tc_redaction_summary_json</c>), as the Mac
    /// draws them (RedactionSummary.swift:52-80): every field, in the core's
    /// order, against the export's own answer for the same maps.
    /// </summary>
    [Fact]
    public void TheRowsAreTheCoresRowsFieldForField()
    {
        string occurrences = "{\"local_path\":185,\"secret:aws\":2,\"residual_secret_at:a.b\":1}";
        string distinct = "{\"local_path\":12}";
        string core = NativeMethods.TakeOwnedString(NativeMethods.tc_redaction_summary_json(occurrences, distinct))!;
        using System.Text.Json.JsonDocument doc = System.Text.Json.JsonDocument.Parse(core);
        (IReadOnlyList<RedactionSummaryRow> removed, IReadOnlyList<RedactionSummaryRow> still) = RedactionSummary.Rows(
            Map(("local_path", 185), ("secret:aws", 2), ("residual_secret_at:a.b", 1)),
            Map(("local_path", 12)))!.Value;
        foreach ((string list, IReadOnlyList<RedactionSummaryRow> rows) in new[] { ("removed", removed), ("still_present", still) })
        {
            var expected = doc.RootElement.GetProperty(list).EnumerateArray().ToList();
            Assert.Equal(expected.Count, rows.Count);
            for (int i = 0; i < rows.Count; i++)
            {
                Assert.Equal(expected[i].GetProperty("family").GetString(), rows[i].Family);
                Assert.Equal(expected[i].GetProperty("display").GetString(), rows[i].Display);
                Assert.Equal(expected[i].GetProperty("description").GetString(), rows[i].Description);
                Assert.Equal(expected[i].GetProperty("occurrences").GetInt32(), rows[i].Occurrences);
                Assert.Equal(expected[i].GetProperty("distinct").GetInt32(), rows[i].Distinct);
                Assert.Equal(expected[i].GetProperty("detail").EnumerateArray().Select(d => d.GetString()), rows[i].Detail);
            }
        }
    }

    /// <summary>
    /// An unreadable map is not "no rows": the panel must not claim nothing
    /// matched because it could not read what did (RedactionSummaryTests.swift:36-42).
    /// </summary>
    [Fact]
    public void AnUnreadableAnswerIsNullNotEmpty()
    {
        Assert.Null(RedactionSummary.Rows(null, Map()));
    }
}
