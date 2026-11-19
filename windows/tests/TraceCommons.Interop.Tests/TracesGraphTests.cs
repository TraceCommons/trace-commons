using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Traces graph footer (W3, moved from W2 2026-11-19) against the Mac's
/// own numbers: <c>Fixtures/Map/mac-graph-probe.json</c>, printed by a
/// throwaway swiftc probe of TracesGraphModel (MonitorShell.swift:76-186),
/// GlassBarGraph (Indicators.swift:396-506) and MonitorTracesGraphCopy
/// (MonitorGraphMapCopy.swift:41-58); its <c>_source</c> says what it
/// compiled. No expectation here is computed by the C# under test.
/// </summary>
public class TracesGraphTests
{
    private const double Tolerance = 1e-9;

    private static readonly CultureInfo EnUs = CultureInfo.GetCultureInfo("en-US");

    private static readonly Lazy<JsonElement> Root = new(() =>
    {
        using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "Fixtures", "Map", "mac-graph-probe.json")));
        return doc.RootElement.Clone();
    });

    private static JsonElement Mac(string key) => Root.Value.GetProperty(key);

    private static DateTimeOffset Date(JsonElement e) => DateTimeOffset.Parse(e.GetString()!, CultureInfo.InvariantCulture);

    private static IReadOnlyList<DateTimeOffset> Dates(string key) => Mac(key).EnumerateArray().Select(Date).ToList();

    public static IEnumerable<object[]> BucketSets() =>
        from zone in new[] { "UTC", "America/Los_Angeles", "Asia/Kolkata" }
        from range in new[] { 0, 1, 2 }
        from offset in new[] { 0, -1, -2, -5 }
        select new object[] { zone, range, offset };

    [Theory]
    [MemberData(nameof(BucketSets))]
    public void TheBucketsAreTheMacs(string zone, int range, int offset)
    {
        // MonitorShell.swift:139-185, in three zones (Los Angeles' clocks
        // went back on 2026-11-01, inside every window here).
        TimeZoneInfo tz = TimeZoneInfo.FindSystemTimeZoneById(zone);
        IReadOnlyList<GraphBucket> buckets = TracesGraphModel.Buckets(
            Dates("shared"), Dates("kept"), offset, Date(Mac("now")), (GraphRange)range, tz, EnUs);
        JsonElement mac = Mac("buckets").GetProperty($"{zone}|{range}|{offset}");
        Assert.Equal(mac.GetArrayLength(), buckets.Count);
        for (int i = 0; i < buckets.Count; i++)
        {
            JsonElement m = mac[i];
            string at = $"{zone} range {range} offset {offset} #{i}";
            Assert.True(Date(m.GetProperty("start")) == buckets[i].Start, $"{at} start: Mac {m.GetProperty("start")}, Windows {buckets[i].Start:o}");
            Assert.Equal(m.GetProperty("start").GetString(), buckets[i].Id);
            Assert.True(m.GetProperty("shared").GetInt32() == buckets[i].Shared, $"{at} shared: Mac {m.GetProperty("shared")}, Windows {buckets[i].Shared}");
            Assert.True(m.GetProperty("kept").GetInt32() == buckets[i].Kept, $"{at} kept: Mac {m.GetProperty("kept")}, Windows {buckets[i].Kept}");
            Assert.True(m.GetProperty("label").GetString() == buckets[i].Label, $"{at} label: Mac '{m.GetProperty("label")}', Windows '{buckets[i].Label}'");
            Assert.Equal(m.GetProperty("day").GetString(), TracesGraphModel.Day(buckets[i].Start, tz, EnUs));
        }
    }

    [Fact]
    public void TheRangesAreTheMacs()
    {
        // MonitorShell.swift:80-111.
        JsonElement mac = Mac("ranges");
        foreach (GraphRange range in Enum.GetValues<GraphRange>())
        {
            JsonElement m = mac[(int)range];
            Assert.Equal(m.GetProperty("count").GetInt32(), TracesGraphModel.Count(range));
            Assert.Equal(m.GetProperty("span").GetInt32(), TracesGraphModel.Span(range));
            Assert.Equal(m.GetProperty("stepHours").GetInt32(), TracesGraphModel.StepHours(range));
            Assert.Equal(m.GetProperty("out").ValueKind == JsonValueKind.Null ? null : (GraphRange?)m.GetProperty("out").GetInt32(), TracesGraphModel.ZoomedOut(range));
            Assert.Equal(m.GetProperty("in").ValueKind == JsonValueKind.Null ? null : (GraphRange?)m.GetProperty("in").GetInt32(), TracesGraphModel.ZoomedIn(range));
        }
    }

    [Fact]
    public void TheDotsAreTheMacs()
    {
        // Indicators.swift:464-475.
        foreach (JsonProperty pair in Mac("dotAlphas").EnumerateObject())
        {
            string[] parts = pair.Name.Split('/');
            double[] alphas = BarGraph.DotAlphas(double.Parse(parts[0], CultureInfo.InvariantCulture), double.Parse(parts[1], CultureInfo.InvariantCulture));
            double[] mac = pair.Value.EnumerateArray().Select(e => e.GetDouble()).ToArray();
            Assert.Equal(mac.Length, alphas.Length);
            for (int i = 0; i < mac.Length; i++)
            {
                Assert.True(Math.Abs(mac[i] - alphas[i]) < Tolerance, $"{pair.Name} row {i}: Mac {mac[i]}, Windows {alphas[i]}");
            }
        }
    }

    [Theory]
    [InlineData("full_40x96", 40, 96, false)]
    [InlineData("full_18x96", 18, 96, false)]
    [InlineData("thin_9x96", 9, 96, true)]
    public void TheMatrixIsTheMacs(string key, double width, double height, bool thin)
    {
        // drawMatrix's arithmetic, Indicators.swift:480-506, with the radius
        // and inset the bars pass (:416-418, :396-399).
        JsonElement mac = Mac("matrix").GetProperty(key);
        BarMatrix m = BarGraph.Matrix(width, height, thin);
        Assert.True(Math.Abs(mac.GetProperty("pitch").GetDouble() - m.Pitch) < Tolerance, $"pitch: Mac {mac.GetProperty("pitch")}, Windows {m.Pitch}");
        Assert.True(Math.Abs(mac.GetProperty("across").GetDouble() - m.Across) < Tolerance, $"across: Mac {mac.GetProperty("across")}, Windows {m.Across}");
        Assert.True(Math.Abs(mac.GetProperty("left").GetDouble() - m.Left) < Tolerance, $"left: Mac {mac.GetProperty("left")}, Windows {m.Left}");
        foreach ((string half, IReadOnlyList<(double X, double Y)> dots) in new[] { ("up", m.Up), ("down", m.Down) })
        {
            JsonElement points = mac.GetProperty(half);
            Assert.Equal(points.GetArrayLength(), dots.Count);
            for (int i = 0; i < dots.Count; i++)
            {
                Assert.True(Math.Abs(points[i][0].GetDouble() - dots[i].X) < Tolerance && Math.Abs(points[i][1].GetDouble() - dots[i].Y) < Tolerance,
                    $"{half} #{i}: Mac ({points[i][0]}, {points[i][1]}), Windows ({dots[i].X}, {dots[i].Y})");
            }
        }

        JsonElement axis = mac.GetProperty("axis");
        Assert.Equal(new[] { axis[0].GetDouble(), axis[1].GetDouble(), axis[2].GetDouble(), axis[3].GetDouble() },
            new[] { m.Axis.X, m.Axis.Y, m.Axis.Width, m.Axis.Height });
        Assert.Equal(thin ? 1.5 : 1.9, m.Radius);
    }

    [Fact]
    public void TheGapsAndLabelsAreTheMacs()
    {
        // Indicators.swift:396-399, :451-456; a column past 14 buckets is
        // thin (:409); 96 tall (:425); axis ink 0.14 (:505).
        Assert.Equal(Mac("gaps").GetProperty("thin").EnumerateArray().Select(e => e.GetDouble()), new[] { BarGraph.Gaps(true).Between, BarGraph.Gaps(true).Inset });
        Assert.Equal(Mac("gaps").GetProperty("full").EnumerateArray().Select(e => e.GetDouble()), new[] { BarGraph.Gaps(false).Between, BarGraph.Gaps(false).Inset });
        string[] mac = Mac("labelAlignment").EnumerateArray().Select(e => e.GetString()!).ToArray();
        for (int i = 0; i < 42; i++)
        {
            Assert.Equal(mac[i], Name(BarGraph.LabelAlignment(i, 42, thin: true)));
        }

        Assert.Equal(mac[42], Name(BarGraph.LabelAlignment(0, 11, thin: false)));
        Assert.False(BarGraph.Thin(11));
        Assert.False(BarGraph.Thin(14));
        Assert.True(BarGraph.Thin(42));
        Assert.Equal(96, BarGraph.ColumnHeight);
        Assert.Equal(0.14, BarGraph.AxisAlpha);
        Assert.Equal(7, BarGraph.Rows);
        Assert.Equal(6, BarGraph.MatrixEnd);

        static string Name(int alignment) => alignment switch { -1 => "leading", 1 => "trailing", _ => "center" };
    }

    [Fact]
    public void TheColumnsShareTheRowEqually()
    {
        // Indicators.swift:410-413: an HStack of columns `spacing:
        // gaps.between` apart, each as wide as the others (its label frame
        // takes maxWidth .infinity). 11 columns 6 apart over 400: (400 - 60) / 11.
        IReadOnlyList<(double X, double Width)> days = BarGraph.ColumnFrames(400, 11);
        Assert.Equal(11, days.Count);
        Assert.Equal(340.0 / 11, days[0].Width, 9);
        Assert.Equal(0, days[0].X);
        Assert.Equal(340.0 / 11 + 6, days[1].X, 9);
        Assert.Equal(400, days[10].X + days[10].Width, 9);
        // 42 thin columns 2 apart: (400 - 82) / 42.
        Assert.Equal(318.0 / 42, BarGraph.ColumnFrames(400, 42)[0].Width, 9);
        Assert.Equal(0, BarGraph.ColumnAt(1, 400, 11));
        Assert.Equal(-1, BarGraph.ColumnAt(340.0 / 11 + 3, 400, 11));
        Assert.Equal(10, BarGraph.ColumnAt(399, 400, 11));
    }

    [Fact]
    public void TheRangePillAndBarsSayTheCoresWords()
    {
        // MonitorGraphMapCopy.swift:41-58 over the core's own table.
        TracesGraphWords words = TracesGraphWords.Core;
        foreach (JsonProperty pair in Mac("rangeWords").EnumerateObject())
        {
            string[] parts = pair.Name.Split('|');
            Assert.Equal(pair.Value.GetString(), words.Range(int.Parse(parts[0], CultureInfo.InvariantCulture), parts[1] == "true", int.Parse(parts[2], CultureInfo.InvariantCulture)));
        }

        Assert.Equal(Mac("barWords").GetString(), words.BarText("Monday, November 2", 3, 0));
    }

    [Fact]
    public void ThePeriodPillsSlideAndTheZoomSwapsInPlace()
    {
        // MonitorShell.swift:239 (previous from .trailing), :263 (next from
        // .leading), :265-268 (jump from .leading), :280-289 (zoom: no
        // slide, hover cleared, back to now), :272-277 (offset never past 0).
        var state = new TracesGraphState();
        Assert.Equal(GraphRange.Days, state.Range);
        state.Next();
        Assert.Equal(0, state.Offset);
        state.Previous();
        state.Previous();
        Assert.Equal(-2, state.Offset);
        Assert.Equal(GraphSlide.FromTrailing, state.Slide);
        Assert.Same(TracesGraphState.PeriodSlide, state.Change(reduceMotion: false));
        state.Next();
        Assert.Equal(-1, state.Offset);
        Assert.Equal(GraphSlide.FromLeading, state.Slide);
        state.Previous();
        state.JumpToNow();
        Assert.Equal(0, state.Offset);
        Assert.Equal(GraphSlide.FromLeading, state.Slide);

        state.Previous();
        state.Hovered = "x";
        state.ZoomOut();
        Assert.Equal(GraphRange.Weeks, state.Range);
        Assert.Equal(0, state.Offset);
        Assert.Null(state.Hovered);
        Assert.Equal(GraphSlide.None, state.Slide);
        Assert.Same(TracesGraphState.ZoomFade, state.Change(reduceMotion: false));
        Assert.Null(state.Change(reduceMotion: true));
        state.ZoomOut();
        Assert.Equal(GraphRange.Weeks, state.Range);
        state.ZoomIn();
        state.ZoomIn();
        state.ZoomIn();
        Assert.Equal(GraphRange.Hours, state.Range);
    }

    [Fact]
    public void TheMotionsAreTheMacs()
    {
        // The period slide is timingCurve(0.4, 0, 0.2, 1) over 0.45 s
        // (MonitorShell.swift:293-295; probe "curveGraph" samples that
        // curve); the zoom is GlassMotion.fast on the token curve (:285);
        // the footer opens on the token curve over 0.25 s
        // (MonitorWindowView.swift:1034; probe "curveToken" is 0.25 s of it),
        // 206 tall (:418).
        AssertCurve("curveGraph", TracesGraphState.PeriodSlide);
        Assert.Equal(0.45, TracesGraphState.PeriodSlide.Duration);
        AssertCurve("curveToken", TracesGraphState.Footer);
        Assert.Equal(0.25, TracesGraphState.Footer.Duration);
        Assert.Equal(0.15, TracesGraphState.ZoomFade.Duration);
        Assert.Equal(BezierCurve.Token(1) with { Seconds = 0.15 }, TracesGraphState.ZoomFade);
        Assert.Equal(206, TracesGraphState.Height);
        Assert.Equal(5, TracesGraphState.ScaleFloor);
    }

    private static void AssertCurve(string key, BezierCurve curve)
    {
        JsonElement mac = MapProbe.Get(key);
        for (int i = 0; i <= 10; i++)
        {
            double t = curve.Duration * i / 10;
            Assert.True(Math.Abs(mac[i].GetDouble() - curve.At(t)) < 1e-4, $"{key} at {i / 10.0}: Mac {mac[i].GetDouble()}, Windows {curve.At(t)}");
        }
    }

    [Fact]
    public void TheGraphIsTheTracesTabs()
    {
        // MonitorWindowView.swift:679, :940-944.
        Assert.True(TracesGraphState.Available(MonitorTab.Traces));
        Assert.False(TracesGraphState.Available(MonitorTab.Home));
        Assert.False(TracesGraphState.Available(MonitorTab.Inference));
        Assert.False(TracesGraphState.Available(MonitorTab.Insights));
        Assert.True(TracesGraphState.Shown(true, MonitorTab.Traces));
        Assert.False(TracesGraphState.Shown(true, MonitorTab.Home));
        Assert.False(TracesGraphState.Shown(false, MonitorTab.Traces));
    }

    private static MapHistoryRow Row(string id, DateTimeOffset at, string? source, string? status) => new(id, at, "p", "P", source, status);

    private static MapEntry Entry(string id, DateTimeOffset found, string source, string? declared = null) =>
        new(id, source, declared, "p", "P", null, found);

    [Fact]
    public void TheFaceCountsOnlyTheSelectedToolAndDashesAnUnreadShare()
    {
        // MonitorShell.swift:122 (contributed statuses), :125-134 (one
        // tool's times), :219-230 (legend totals, hovered bucket),
        // :315-317 (a dash for an unread or partial History), :329-333 (bar
        // words), :337-345 (the pill), :313-314 (focus ink) and
        // MonitorTracesCopy.swift:769-772 (the tip).
        TracesGraphWords words = TracesGraphWords.Core;
        TimeZoneInfo utc = TimeZoneInfo.Utc;
        var now = new DateTimeOffset(2026, 11, 2, 10, 37, 0, TimeSpan.Zero);
        var history = new[]
        {
            Row("a", now.AddHours(-1), "codex", "accepted"),
            Row("b", now.AddHours(-2), "codex", "submitted"),
            Row("c", now.AddDays(-1), "claude-code", "quarantined"),
            Row("d", now.AddDays(-1), "codex", "withdrawn"),
            Row("e", now.AddDays(-30), "codex", "accepted"),
        };
        var sessions = new[]
        {
            Entry("s1", now.AddHours(-3), "codex"),
            Entry("s2", now.AddDays(-2), "claude-code"),
            Entry("s3", now.AddDays(-2), "other", declared: "codex"),
        };

        var state = new TracesGraphState();
        GraphFace all = state.Face(history, complete: true, sessions, tool: null, focus: false, now, utc, EnUs, words);
        Assert.Equal(11, all.Bars.Count);
        Assert.Equal("3", all.SharedFigure);
        Assert.Equal("3", all.KeptFigure);
        Assert.Equal(words.Range(11, false, 0), all.RangeLabel);
        Assert.Equal(words.FocusNeedsSelection, all.FocusTip);
        Assert.Equal("graphFocusOff", all.FocusInk);
        Assert.False(all.CanFocus);
        Assert.False(all.CanStepNext);
        Assert.False(all.CanJump);
        Assert.True(all.CanZoomOut);
        Assert.True(all.CanZoomIn);
        GraphBar today = all.Bars[^1];
        Assert.Equal(2, today.Up);
        Assert.Equal(1, today.Down);
        Assert.Equal(words.BarText("Monday, November 2", 2, 1), today.Description);

        GraphFace codex = state.Face(history, complete: true, sessions, tool: "codex", focus: false, now, utc, EnUs, words);
        Assert.Equal("2", codex.SharedFigure);
        Assert.Equal("2", codex.KeptFigure);
        Assert.Equal(words.FocusTip("Codex", false), codex.FocusTip);
        Assert.Equal("graphFocusIdle", codex.FocusInk);
        GraphFace focused = state.Face(history, complete: true, sessions, tool: "codex", focus: true, now, utc, EnUs, words);
        Assert.Equal(words.FocusWholeMap, focused.FocusTip);
        Assert.Equal("graphFocusOn", focused.FocusInk);
        Assert.True(focused.FocusSelected);

        // Unread or a capped page: the shared figure is a dash, never a part count.
        Assert.Equal("—", state.Face(null, complete: false, sessions, null, false, now, utc, EnUs, words).SharedFigure);
        Assert.Equal("—", state.Face(history, complete: false, sessions, null, false, now, utc, EnUs, words).SharedFigure);

        // A hovered bar: its own counts and its day in the pill.
        state.Hovered = today.Id;
        GraphFace hovered = state.Face(history, complete: true, sessions, null, false, now, utc, EnUs, words);
        Assert.Equal("2", hovered.SharedFigure);
        Assert.Equal("1", hovered.KeptFigure);
        Assert.Equal("Monday, November 2", hovered.RangeLabel);

        state.Previous();
        Assert.True(state.Face(history, true, sessions, null, false, now, utc, EnUs, words).CanStepNext);
    }

    [Fact]
    public void TheLegendCountsOnlyAWholePage()
    {
        // MonitorShell.swift:297-310: readable is the last page of a read
        // that did not fail; countable only under HomeStore.historyLimit (100).
        var row = new MapHistoryRow("x", DateTimeOffset.UnixEpoch, null, null, null, "accepted");
        Assert.Equal((null, false), TracesGraphState.Readable(null));
        Assert.True(TracesGraphState.Readable(Enumerable.Repeat(row, 99).ToList()).Complete);
        (IReadOnlyList<MapHistoryRow>? rows, bool complete) = TracesGraphState.Readable(Enumerable.Repeat(row, 100).ToList());
        Assert.NotNull(rows);
        Assert.False(complete);
    }
}
