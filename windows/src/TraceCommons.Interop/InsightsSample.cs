using System;
using System.Threading;
using System.Threading.Tasks;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The sample Insights draws when it has no trace of the person's own, and
/// what the debug sample sets open on (<c>InsightsSample</c>,
/// macos/Sources/TraceCommonsApp/InsightsSample.swift). Sample figures, never
/// a pilot's, and never mixed with the person's own. Every number here is
/// the Mac's, copied from that file.
/// </summary>
public static class InsightsSample
{
    private const string Opus = "claude-opus-5-5", Sonnet = "claude-sonnet-5-5", Haiku = "claude-haiku-5-5";
    private const string Codex = "gpt-5-codex", Claude = "claude_code", CodexTool = "codex";

    private static BenchmarkPick Pick((string Label, string Tool, long Figure) leader, (string Label, string Tool, long Figure) next) =>
        new(new BenchmarkEntry(leader.Label, leader.Tool, leader.Figure), new BenchmarkEntry(next.Label, next.Tool, next.Figure));

    private static BenchmarkCategory Category(string key, int sessions, BenchmarkPick quality, BenchmarkPick value,
        BenchmarkPick speed, BenchmarkTip? tip = null) =>
        new(key, sessions, Array.Empty<BenchmarkModel>(), quality, value, speed, tip);

    /// <summary>Four weeks of sample benchmarks: five kinds of work, two models compared on each (InsightsSample.swift:68-104).</summary>
    public static readonly InsightsBenchmarks Benchmarks = new(
        Enabled: true, Readable: true, Weeks: 4, Population: null, Sessions: 107, MinSessions: 5,
        Categories: new[]
        {
            Category("bug_fix", 34,
                Pick((Opus, Claude, 930), (Codex, CodexTool, 880)),
                Pick((Sonnet, Claude, 410_000), (Opus, Claude, 1_840_000)),
                Pick((Codex, CodexTool, 840), (Opus, Claude, 1_260)),
                new BenchmarkTip("test_first", FasterPermille: 350, SessionsWithout: 12, Sessions: 34)),
            Category("feature", 28,
                Pick((Opus, Claude, 910), (Sonnet, Claude, 860)),
                Pick((Sonnet, Claude, 620_000), (Opus, Claude, 2_450_000)),
                Pick((Sonnet, Claude, 1_500), (Opus, Claude, 2_100)),
                new BenchmarkTip("cheaper_same_quality", Cheap: Sonnet, Expensive: Opus, CostRatioPermille: 250)),
            Category("refactor", 19,
                Pick((Codex, CodexTool, 940), (Opus, Claude, 900)),
                Pick((Codex, CodexTool, 780_000), (Opus, Claude, 1_960_000)),
                Pick((Opus, Claude, 1_080), (Codex, CodexTool, 1_320)),
                new BenchmarkTip("fewer_undos", Better: Codex, Worse: Opus, RatioPermille: 2_300)),
            Category("tests", 17,
                Pick((Sonnet, Claude, 950), (Opus, Claude, 930)),
                Pick((Sonnet, Claude, 290_000), (Opus, Claude, 1_100_000)),
                Pick((Sonnet, Claude, 660), (Opus, Claude, 900)),
                new BenchmarkTip("test_first", FasterPermille: 220, SessionsWithout: 5, Sessions: 17)),
            Category("docs", 9,
                Pick((Sonnet, Claude, 970), (Haiku, Claude, 950)),
                Pick((Haiku, Claude, 60_000), (Sonnet, Claude, 210_000)),
                Pick((Haiku, Claude, 240), (Sonnet, Claude, 420))),
        });

    /// <summary>The refusal every sample read answers with (InsightsSampleWeek.swift:34).</summary>
    public const string Refusal = "insights_sample_not_answered";
}

/// <summary>
/// The Insights service a sample launch uses: it refuses every request, so
/// nothing a sample window shows is read from this machine's store, and
/// nothing is written (<c>InsightsSampleModels</c>' <c>refuse</c> service,
/// InsightsSample.swift:219-222). It never reaches <c>tc_insights_call</c>.
/// </summary>
public sealed class SampleInsights : ILocalInsights
{
    public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken) =>
        Task.FromException<JsonElement>(new InvalidOperationException(InsightsSample.Refusal));
}
