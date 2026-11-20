using System;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Private AI tab's ledger (spec W19). Every expected value was printed
/// by a Swift probe that compiled the Mac's own <c>InferenceTabView</c>
/// statics (InferenceViews.swift:184-234, 433-451, 487-496) against
/// TCShellCore's <c>DaemonData</c>, over the recorded samples.
/// </summary>
public sealed class InferenceLedgerTests
{
    private static readonly string Root = Path.Combine(AppContext.BaseDirectory, "RecordedSamples");

    private static JsonElement Read(string set, string method) =>
        JsonDocument.Parse(File.ReadAllText(Path.Combine(Root, set, method + ".json"))).RootElement.Clone();

    [Theory]
    [InlineData("normalDay", null, 1L, "—", 5L, 1L, "—")]
    [InlineData("busyQueue", null, 1L, "—", 5L, 1L, "—")]
    [InlineData("empty", null, 0L, "$0.00", 0L, 0L, "$0.00")]
    public void TotalsAreTheMacsOverTheSamples(
        string set, long? calls, long? verified, string priced, long? callsNoDest, long? verifiedNoDest, string pricedNoDest)
    {
        InferenceCallPage page = InferenceLedger.ParsePage(Read(set, "inference_calls"))!;
        DestinationCallCounts destinations = InferenceLedger.ParseDestinations(Read(set, "tool_destinations"))!;
        Assert.Equal(new LedgerTotals(calls, verified, priced), InferenceLedger.Totals(page, null, destinations));
        Assert.Equal(new LedgerTotals(callsNoDest, verifiedNoDest, pricedNoDest), InferenceLedger.Totals(page, null, null));
    }

    [Fact]
    public void EachCallIsPricedAsTheMacPricesIt()
    {
        InferenceCallPage page = InferenceLedger.ParsePage(Read("normalDay", "inference_calls"))!;
        Assert.Equal(
            new[] { "$0.0084", "$0.0123", "—", "$0.0051", "$0.0049" },
            page.Calls.Select(c => InferenceLedger.Priced(new[] { c.Cost })).ToArray());
    }

    [Fact]
    public void TheSummaryAnswersVerifiedAndPricedAndOnePageNeverStandsForTheWindow()
    {
        InferenceModelSummary summary = InferenceLedger.ParseSummary(JsonDocument.Parse(
            """{"readable":true,"window_hours":24,"models":[{"model":"a","calls":3,"priced_micros":20700,"proof_counts":{"failed":1,"verified":2,"zzz":4,"gateway_only":1}},{"model":"b","calls":2,"priced_micros":5100,"proof_counts":{"verified":1}}]}""").RootElement)!;
        InferenceCallPage cursor = InferenceLedger.ParsePage(JsonDocument.Parse("""{"readable":true,"calls":[],"next_cursor":"c"}""").RootElement)!;
        Assert.Equal(new LedgerTotals(5, 3, "$0.0258"), InferenceLedger.Totals(cursor, summary, null));
        Assert.Equal(new LedgerTotals(null, null, "—"), InferenceLedger.Totals(cursor, null, null));
        Assert.Equal(
            new[] { "verified=2", "gateway_only=1", "failed=1", "zzz=4" },
            InferenceLedger.SortedProofs(summary.Models[0].ProofCounts).Select(p => $"{p.Label}={p.Count}").ToArray());
    }

    // .currency(code: "USD").precision(.fractionLength(2 ... 4)), en_US,
    // printed by the probe (InferenceViews.swift:445-451).
    [Theory]
    [InlineData(0L, "$0.00")]
    [InlineData(8400L, "$0.0084")]
    [InlineData(1_234_567L, "$1.2346")]
    [InlineData(50_000_000L, "$50.00")]
    [InlineData(1L, "$0.00")]
    [InlineData(99L, "$0.0001")]
    [InlineData(50L, "$0.00")]
    [InlineData(150L, "$0.0002")]
    [InlineData(250L, "$0.0002")]
    [InlineData(350L, "$0.0004")]
    [InlineData(105_000L, "$0.105")]
    [InlineData(1_005_000L, "$1.005")]
    [InlineData(123_456_789_000L, "$123,456.789")]
    public void MoneyIsTheMacs(long micros, string expected) => Assert.Equal(expected, InferenceLedger.Money(micros));

    // .number.notation(.compactName), en_US (InsightsModel.swift:242-245).
    [Theory]
    [InlineData(0UL, "0")]
    [InlineData(950UL, "950")]
    [InlineData(999UL, "999")]
    [InlineData(1000UL, "1K")]
    [InlineData(1150UL, "1.2K")]
    [InlineData(1200UL, "1.2K")]
    [InlineData(1250UL, "1.2K")]
    [InlineData(1350UL, "1.4K")]
    [InlineData(2048UL, "2K")]
    [InlineData(9949UL, "9.9K")]
    [InlineData(9950UL, "10K")]
    [InlineData(12500UL, "12K")]
    [InlineData(13500UL, "14K")]
    [InlineData(18000UL, "18K")]
    [InlineData(99_949UL, "100K")]
    [InlineData(123_456UL, "123K")]
    [InlineData(125_000UL, "125K")]
    [InlineData(999_999UL, "1M")]
    [InlineData(1_500_000UL, "1.5M")]
    [InlineData(12_345_678_901UL, "12B")]
    [InlineData(1_000_000_000_000UL, "1T")]
    [InlineData(1_250_000_000_000_000UL, "1250T")]
    public void CompactFiguresAreTheMacs(ulong value, string expected) =>
        Assert.Equal(expected, InferenceLedger.CompactFigure(value));

    // A label this shell does not know is the dash and neutral, never a
    // verdict (InferenceViews.swift:478-485, 548-552).
    [Fact]
    public void AnUnknownProofLabelIsTheDashAndNeutral()
    {
        Assert.Equal("—", InferenceLedger.ProofWord("audited", key => "word:" + key));
        Assert.Equal("word:proof_gateway_only", InferenceLedger.ProofWord("gateway_only", key => "word:" + key));
        Assert.Equal(LedgerTone.Neutral, InferenceLedger.Tone("audited"));
        Assert.Equal(LedgerTone.On, InferenceLedger.Tone("verified"));
        Assert.Equal(LedgerTone.Ask, InferenceLedger.Tone("gateway_only"));
        Assert.Equal(LedgerTone.Ask, InferenceLedger.Tone("pending"));
        Assert.Equal(LedgerTone.Failed, InferenceLedger.Tone("failed"));
        Assert.Equal(LedgerTone.Outside, InferenceLedger.Tone("outside"));
        Assert.Equal(LedgerTone.Neutral, InferenceLedger.Tone("unattested"));
        // An unknown label is not proof (DaemonDataModels.swift:1720).
        Assert.Equal(ProofLabel.Unrecorded, new InferenceCall(1, default, "", "", "", "", null, "audited", null).ProofLabel);
    }

    // The tool's name, its raw id, or the dash (InferenceViews.swift:427-431).
    [Fact]
    public void AToolIsNamedRawOrDashed()
    {
        Func<string, string?> known = id => id == "claude-code" ? "Claude Code" : null;
        Assert.Equal("Claude Code", InferenceLedger.ToolName("claude-code", known));
        Assert.Equal("newtool", InferenceLedger.ToolName("newtool", known));
        Assert.Equal("—", InferenceLedger.ToolName("unknown", known));
    }

    // Each counter as reported, never added; the dash for unknown
    // (InferenceViews.swift:499-527), over the core's own Insights words.
    [Fact]
    public void TheTokenLineIsEachCounterInTheCoresWords()
    {
        var copy = LocalInsights.CoreCopy();
        string Word(string key) => copy.TryGetValue(key, out string? w) ? w : "";
        InferenceCallPage page = InferenceLedger.ParsePage(Read("normalDay", "inference_calls"))!;
        string first = InferenceLedger.TokenLine(page.Calls[0].Tokens!, Word);
        Assert.Equal($"{Word("metric_input_tokens")} 1.2K · {Word("analytics_series_cache_read")} 18K · {Word("analytics_series_cache_write")} 0 · {Word("metric_output_tokens")} 350", first);
        string unknown = InferenceLedger.TokenLine(page.Calls[2].Tokens!, Word);
        Assert.Equal(4, unknown.Split(" · ").Count(part => part.EndsWith(" " + Word("analytics_unavailable"), StringComparison.Ordinal)));
        Assert.DoesNotContain(" 0", unknown);
        Assert.Contains("Unknown", InferenceLedger.TokenAccessibilityLine(page.Calls[2].Tokens!, Word, "Unknown"));
    }

    // The window line is the core's, or the dash when none was reported
    // (MonitorScreensCopy.swift:366-369).
    [Fact]
    public void TheWindowLineIsTheCoresOrTheDash()
    {
        string template = MonitorShellCopy.Top("window_last_hours");
        Assert.Contains("{hours}", template);
        Assert.Equal(template.Replace("{hours}", "24"), InferenceLedger.WindowLine(24, template));
        Assert.Equal("—", InferenceLedger.WindowLine(null, template));
    }

    [Fact]
    public void ACallTimeIsTheReadersShortTime()
    {
        string time = InferenceLedger.CallTime(
            DateTimeOffset.Parse("2026-09-30T09:04:11+00:00", CultureInfo.InvariantCulture), TimeZoneInfo.Utc, CultureInfo.GetCultureInfo("en-US"));
        Assert.StartsWith("9:04", time);
        Assert.EndsWith("AM", time);
    }

    private const string Zero = "\"proof\":{\"verified\":0,\"gateway_only\":0,\"unattested\":0,\"pending\":0,\"unavailable\":0,\"failed\":0,\"outside\":0,\"unrecorded\":0}";

    private static string Network(bool enabled, string routed, string outside, string unknown) =>
        "{\"readable\":true,\"window_hours\":24,\"summary\":{\"enabled\":" + (enabled ? "true" : "false")
        + ",\"receipts\":true,\"since\":\"2026-09-30T00:00:00Z\",\"groups\":[],\"routed\":{" + routed + "},\"outside\":{" + outside + "},\"unknown\":{" + unknown + "}}}";

    // InferenceTabView.networkHeaderTotals over HomePrivateAI.networkTotals
    // (InferenceViews.swift:162-180, HomeViews.swift:775-791), probe-printed.
    [Fact]
    public void TheNetworkHeaderIsTheMacs()
    {
        static LedgerTotals Header(string json) =>
            InferenceLedger.NetworkHeaderTotals(InferenceLedger.ParseNetworkSummary(JsonDocument.Parse(json).RootElement));
        string verified3 = Zero.Replace("\"verified\":0", "\"verified\":3");
        Assert.Equal(new LedgerTotals(5, 3, "$0.0124"), Header(Network(true,
            "\"calls\":4,\"priced_calls\":4,\"cost_usd\":0.0123," + verified3,
            "\"calls\":1,\"priced_calls\":1,\"cost_usd\":0.00005," + Zero,
            "\"calls\":0,\"priced_calls\":0,\"cost_usd\":0," + Zero)));
        // A route with a call left unpriced, or no pricing coverage: the dash.
        Assert.Equal(new LedgerTotals(5, 3, "\u2014"), Header(Network(true,
            "\"calls\":4,\"priced_calls\":3,\"cost_usd\":0.0123," + verified3,
            "\"calls\":1,\"priced_calls\":1,\"cost_usd\":0.5," + Zero,
            "\"calls\":0,\"cost_usd\":0," + Zero)));
        // One route with a call left unpriced, every other priced: the dash.
        Assert.Equal(new LedgerTotals(5, 0, "\u2014"), Header(Network(true,
            "\"calls\":4,\"priced_calls\":3,\"cost_usd\":0.0123," + Zero,
            "\"calls\":1,\"priced_calls\":1,\"cost_usd\":0.5," + Zero,
            "\"calls\":0,\"priced_calls\":0,\"cost_usd\":0," + Zero)));
        // Capture off is not an observed empty window.
        Assert.Equal(new LedgerTotals(null, null, "\u2014"), Header(Network(false,
            "\"calls\":4,\"priced_calls\":4,\"cost_usd\":1.5," + verified3,
            "\"calls\":0,\"priced_calls\":0,\"cost_usd\":0," + Zero,
            "\"calls\":0,\"priced_calls\":0,\"cost_usd\":0," + Zero)));
        Assert.Equal(new LedgerTotals(3, 0, "$12.3459"), Header(Network(true,
            "\"calls\":2,\"priced_calls\":2,\"cost_usd\":12.34567," + Zero,
            "\"calls\":0,\"priced_calls\":0,\"cost_usd\":0," + Zero,
            "\"calls\":1,\"priced_calls\":1,\"cost_usd\":0.00025," + Zero)));
        Assert.Equal(new LedgerTotals(null, null, "\u2014"), Header("{\"readable\":false,\"window_hours\":null,\"summary\":null}"));
    }

    // The Mac rounds the shortest decimal that reads back as the double,
    // half to even (`formattedUSD`, printed by Swift's `.currency` style on
    // macOS: $0.0124, $0.0012, $2.6744). Each of these is a midpoint only in
    // that decimal; the double's binary value sits just off it, on one side
    // or the other, so a conversion that keeps the binary value rounds them
    // the other way.
    [Theory]
    [InlineData(0.0123, 0.00005, "$0.0124")]
    [InlineData(0.00125, 0.0, "$0.0012")]
    [InlineData(2.67435, 0.0, "$2.6744")]
    public void TheHeaderRoundsTheAmountTheMacDoes(double routed, double outside, string priced)
    {
        string cost(double usd) => usd.ToString("R", System.Globalization.CultureInfo.InvariantCulture);
        LedgerTotals totals = InferenceLedger.NetworkHeaderTotals(InferenceLedger.ParseNetworkSummary(JsonDocument.Parse(Network(true,
            "\"calls\":1,\"priced_calls\":1,\"cost_usd\":" + cost(routed) + "," + Zero,
            "\"calls\":1,\"priced_calls\":1,\"cost_usd\":" + cost(outside) + "," + Zero,
            "\"calls\":0,\"priced_calls\":0,\"cost_usd\":0," + Zero)).RootElement));
        Assert.Equal(priced, totals.Priced);
    }

    // Without a network summary the header draws priced alone; calls and
    // verified are dashes (InferenceViews.swift:129-135).
    [Fact]
    public void WithoutTheNetworkSummaryTheHeaderDrawsPricedAlone()
    {
        InferenceCallPage page = InferenceLedger.ParsePage(Read("empty", "inference_calls"))!;
        Assert.Equal(new LedgerTotals(null, null, "$0.00"), InferenceLedger.PageHeaderTotals(page, null, null));
    }
}
