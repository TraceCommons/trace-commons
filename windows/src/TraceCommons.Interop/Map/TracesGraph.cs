using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop.Map;

/// <summary>
/// The graph footer's three ranges (<c>TracesGraphModel.Range</c>,
/// MonitorShell.swift:80-111): 24 hours in 2-hour buckets, 11 days, and
/// 42 days. Zooming out widens the range.
/// </summary>
public enum GraphRange
{
    Hours = 0,
    Days = 1,
    Weeks = 2,
}

/// <summary>One bucket: when it starts, what was shared and kept in it, and the label under its bar.</summary>
public sealed record GraphBucket(DateTimeOffset Start, int Shared, int Kept, string Label)
{
    /// <summary>The bucket's identity, as the Mac's <c>start.formatted(.iso8601)</c>.</summary>
    public string Id => Start.UtcDateTime.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture);
}

/// <summary>
/// Shared over kept, per period (<c>TracesGraphModel</c>, MonitorShell.swift:70-186):
/// shared counts traces that left this computer (History, by submission
/// time), kept counts traces still here (the queue, by discovery time).
/// Counts, not bytes.
/// </summary>
public static class TracesGraphModel
{
    /// <summary>The window, in days, at the default range (:73).</summary>
    public const int Days = 11;

    /// <summary>How many buckets a range has (:86-92).</summary>
    public static int Count(GraphRange range) => range switch
    {
        GraphRange.Hours => 12,
        GraphRange.Days => Days,
        _ => 42,
    };

    /// <summary>A bucket's width in hours: 2, or a day (:95-97).</summary>
    public static int StepHours(GraphRange range) => range == GraphRange.Hours ? 2 : 24;

    /// <summary>The window's span as the range pill says it: 24 (hours), 11 or 42 (days) (:101).</summary>
    public static int Span(GraphRange range) => range == GraphRange.Hours ? Count(range) * 2 : Count(range);

    /// <summary>One step wider, or null at the widest (:106).</summary>
    public static GraphRange? ZoomedOut(GraphRange range) => range == GraphRange.Weeks ? null : range + 1;

    /// <summary>One step narrower, or null at the narrowest (:108).</summary>
    public static GraphRange? ZoomedIn(GraphRange range) => range == GraphRange.Hours ? null : range - 1;

    /// <summary>When each contribution left, for one tool or all of them (:125-129).</summary>
    public static IReadOnlyList<DateTimeOffset> SharedTimes(IReadOnlyList<MapHistoryRow> rows, string? tool) =>
        rows.Where(r => MapContributions.Statuses.Contains(r.Status ?? string.Empty))
            .Where(r => tool is null || (MapTools.Known(r.Source) && r.Source == tool))
            .Select(r => r.SubmittedAt)
            .OfType<DateTimeOffset>()
            .ToList();

    /// <summary>When each trace still waiting was found, for one tool or all (:132-134).</summary>
    public static IReadOnlyList<DateTimeOffset> KeptTimes(IReadOnlyList<MapEntry> sessions, string? tool) =>
        sessions.Where(s => tool is null || TracesTree.MajorityTool(new[] { s }) == tool)
            .Select(s => s.DiscoveredAt)
            .OfType<DateTimeOffset>()
            .ToList();

    /// <summary>
    /// The range's buckets ending in the current one (this hour's 2-hour
    /// bucket, or today), <paramref name="offset"/> whole windows back (0 is
    /// now, -1 the window before it), in <paramref name="zone"/>'s calendar
    /// (:139-160).
    /// </summary>
    public static IReadOnlyList<GraphBucket> Buckets(
        IReadOnlyList<DateTimeOffset> shared, IReadOnlyList<DateTimeOffset> kept, int offset, DateTimeOffset now,
        GraphRange range, TimeZoneInfo zone, CultureInfo culture)
    {
        int count = Count(range);
        var buckets = new List<GraphBucket>(count);
        DateTime local = TimeZoneInfo.ConvertTime(now, zone).DateTime;
        for (int index = 0; index < count; index++)
        {
            int back = count - 1 - index - offset * count;
            DateTimeOffset start, end;
            if (range == GraphRange.Hours)
            {
                // The start of the hour, then whole hours of elapsed time
                // (Calendar.date(byAdding: .hour)), whatever the zone's clock does.
                DateTimeOffset anchor = Instant(new DateTime(local.Year, local.Month, local.Day, local.Hour, 0, 0), zone);
                start = anchor.AddHours(-back * 2);
                end = start.AddHours(2);
            }
            else
            {
                // Days on the zone's calendar: each starts at its own local midnight.
                DateTime day = local.Date.AddDays(-back);
                start = Instant(day, zone);
                end = Instant(day.AddDays(1), zone);
            }

            bool Inside(DateTimeOffset date) => date >= start && date < end;
            buckets.Add(new GraphBucket(
                start, shared.Count(Inside), kept.Count(Inside), Label(range, start, index, zone, culture)));
        }

        return buckets;
    }

    /// <summary>
    /// Every third 2-hour bucket's hour; every day's short weekday at 11
    /// days; every sixth day's at 42 (:164-173).
    /// </summary>
    public static string Label(GraphRange range, DateTimeOffset start, int index, TimeZoneInfo zone, CultureInfo culture) => range switch
    {
        GraphRange.Hours => index % 3 == 0
            ? TimeZoneInfo.ConvertTime(start, zone).Hour.ToString(CultureInfo.InvariantCulture) + ":00"
            : string.Empty,
        GraphRange.Days => Weekday(start, zone, culture),
        _ => index % 6 == 0 ? Weekday(start, zone, culture) : string.Empty,
    };

    /// <summary>"Sat": the short weekday (:176-180).</summary>
    public static string Weekday(DateTimeOffset date, TimeZoneInfo zone, CultureInfo culture) =>
        culture.DateTimeFormat.GetAbbreviatedDayName(TimeZoneInfo.ConvertTime(date, zone).DayOfWeek);

    /// <summary>"Monday, October 5": a hovered bar's day (:181-185).</summary>
    public static string Day(DateTimeOffset date, TimeZoneInfo zone, CultureInfo culture)
    {
        DateTime local = TimeZoneInfo.ConvertTime(date, zone).DateTime;
        return culture.DateTimeFormat.GetDayName(local.DayOfWeek) + ", "
            + culture.DateTimeFormat.GetMonthName(local.Month) + " "
            + local.Day.ToString(culture);
    }

    /// <summary>A count, or a dash while History has not been read (MonitorShell.swift:315-317).</summary>
    public static string Figure(int count, bool known) => known ? count.ToString(CultureInfo.InvariantCulture) : "—";

    private static DateTimeOffset Instant(DateTime wall, TimeZoneInfo zone)
    {
        DateTime unspecified = DateTime.SpecifyKind(wall, DateTimeKind.Unspecified);
        // A wall time a clock change skips is moved past the gap, as Foundation does.
        while (zone.IsInvalidTime(unspecified))
        {
            unspecified = unspecified.AddMinutes(30);
        }

        TimeSpan offset = zone.IsAmbiguousTime(unspecified)
            ? zone.GetAmbiguousTimeOffsets(unspecified).Max()
            : zone.GetUtcOffset(unspecified);
        return new DateTimeOffset(unspecified, offset);
    }
}

/// <summary>
/// The core's words for the graph footer (<c>MonitorTracesGraphCopy</c>,
/// MonitorGraphMapCopy.swift; the <c>traces_graph</c> table of
/// <c>tc_monitor_screens_copy_json</c>), with the legend's and the
/// period pills' words from the same table. Nothing here is this shell's.
/// </summary>
public sealed record TracesGraphWords(
    string ZoomOut, string ZoomIn, string JumpToNow, string ZoomOutGlyph, string ZoomInGlyph, string JumpToNowGlyph,
    string LastHours, string LastDays, string HoursBackOne, string HoursBack, string DaysBackOne, string DaysBack,
    string Bar, string Shared, string Kept, string Previous, string Next, string Focus,
    string FocusNeedsSelection, string FocusWholeMap, string FocusTool)
{
    /// <summary>The core's table; empty words when it could not be read.</summary>
    public static TracesGraphWords Core => new(
        G("zoom_out"), G("zoom_in"), G("jump_to_now"), G("zoom_out_glyph"), G("zoom_in_glyph"), G("jump_to_now_glyph"),
        G("last_hours"), G("last_days"), G("hours_back_one"), G("hours_back"), G("days_back_one"), G("days_back"),
        G("bar"), MonitorShellCopy.Top("shared"), MonitorShellCopy.Top("kept"), MonitorShellCopy.Top("previous"),
        MonitorShellCopy.Top("next"), MonitorShellCopy.Top("focus"),
        MonitorShellCopy.Shell("focus_needs_selection"), MonitorShellCopy.Shell("focus_whole_map"), MonitorShellCopy.Shell("focus_tool"));

    private static string G(string key) => MonitorShellCopy.Section("traces_graph", key);

    /// <summary>
    /// The range pill when no bar is hovered: the window's span at now, or
    /// how many whole windows back it is (MonitorGraphMapCopy.swift:41-52).
    /// </summary>
    public string Range(int span, bool hours, int back)
    {
        string word = back <= 0 ? (hours ? LastHours : LastDays)
            : back == 1 ? (hours ? HoursBackOne : DaysBackOne)
            : (hours ? HoursBack : DaysBack);
        return word.Replace(hours ? "{hours}" : "{days}", span.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
            .Replace("{count}", back.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal);
    }

    /// <summary>One bar's text equivalent: its day, then shared and kept (:55-58).</summary>
    public string BarText(string day, int shared, int kept) =>
        Bar.Replace("{label}", day, StringComparison.Ordinal)
            .Replace("{count}", shared.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
            .Replace("{total}", kept.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal);

    /// <summary>The binoculars' tip (MonitorTracesCopy.swift:769-772).</summary>
    public string FocusTip(string? tool, bool focused) =>
        tool is null ? FocusNeedsSelection : focused ? FocusWholeMap : FocusTool.Replace("{tool}", tool, StringComparison.Ordinal);
}

/// <summary>How a new window of bars comes in.</summary>
public enum GraphSlide
{
    /// <summary>A zoom: the bars swap in place with a fade.</summary>
    None,

    /// <summary>The new bars move in from the trailing edge (a step to the previous period).</summary>
    FromTrailing,

    /// <summary>The new bars move in from the leading edge (a step to the next period, or back to now).</summary>
    FromLeading,
}

/// <summary>
/// The tapered dot matrix's numbers (<c>GlassBarGraph</c>, Indicators.swift:358-506).
/// </summary>
public static class BarGraph
{
    /// <summary>Dots in each half of a column, from the axis out (:441).</summary>
    public const int Rows = 7;

    /// <summary>The space between the matrix and the column's top and foot (:445).</summary>
    public const double MatrixEnd = 6;

    /// <summary>A column's height (:425).</summary>
    public const double ColumnHeight = 96;

    /// <summary>A label's minimum height (:438).</summary>
    public const double LabelMinHeight = 12;

    /// <summary>The axis rule's ink alpha (:505).</summary>
    public const double AxisAlpha = 0.14;

    /// <summary>A column more than 14 buckets wide draws one dot across (:409).</summary>
    public static bool Thin(int count) => count > 14;

    /// <summary>Dots across a column (:416).</summary>
    public static int Columns(bool thin) => thin ? 1 : 3;

    /// <summary>A dot's radius (:417).</summary>
    public static double Radius(bool thin) => thin ? 1.5 : 1.9;

    /// <summary>The hover wash's corner: 3, or the control radius (:423).</summary>
    public static double WashRadius(bool thin, double controlRadius) => thin ? 3 : controlRadius;

    /// <summary>The wash under the pointer, an ink alpha (:423).</summary>
    public static double WashAlpha(bool hovered) => hovered ? 0.07 : 0;

    /// <summary>An unlit dot, an ink alpha (:421).</summary>
    public static double UnlitAlpha(bool hovered) => hovered ? 0.2 : 0.09;

    /// <summary>The label's ink token: primary under the pointer, else statusOff (:402-404).</summary>
    public static string LabelInk(bool hovered) => hovered ? "textPrimary" : "statusOff";

    /// <summary>
    /// The label's ink as this shell draws it. The Mac's statusOff measured
    /// 2.88:1 under the light footer's columns on the box (round 2), under
    /// the 4.5:1 every drawn word must hold (checklist D1); textTertiary is
    /// the quiet text ink that holds it. Under the pointer, the Mac's.
    /// </summary>
    public static string LabelTextInk(bool hovered) => hovered ? LabelInk(true) : "textTertiary";

    /// <summary>The gap between columns and the matrix's inset inside each (:396-399).</summary>
    public static (double Between, double Inset) Gaps(bool thin, bool compact = false) =>
        thin ? (2, 1) : compact ? (2, 2) : (6, 5);

    /// <summary>
    /// Each column's left edge and width across <paramref name="width"/>:
    /// equal shares of the row, the gap between them (:410-413, each
    /// column's label frame taking <c>maxWidth: .infinity</c>).
    /// </summary>
    public static IReadOnlyList<(double X, double Width)> ColumnFrames(double width, int count)
    {
        var frames = new List<(double X, double Width)>(count);
        if (count <= 0)
        {
            return frames;
        }

        double between = Gaps(Thin(count)).Between;
        double each = Math.Max(0, (width - between * (count - 1)) / count);
        for (int i = 0; i < count; i++)
        {
            frames.Add((i * (each + between), each));
        }

        return frames;
    }

    /// <summary>The column under <paramref name="x"/>, or -1 between and outside columns' reach.</summary>
    public static int ColumnAt(double x, double width, int count)
    {
        IReadOnlyList<(double X, double Width)> frames = ColumnFrames(width, count);
        for (int i = 0; i < frames.Count; i++)
        {
            if (x >= frames[i].X && x < frames[i].X + frames[i].Width)
            {
                return i;
            }
        }

        return -1;
    }

    /// <summary>Values scale to the largest bucket, never below the floor (:407).</summary>
    public static double Maximum(IEnumerable<(double Up, double Down)> buckets, double scaleFloor) =>
        Math.Max(scaleFloor, buckets.Select(b => Math.Max(b.Up, b.Down)).DefaultIfEmpty(0).Max());

    /// <summary>Where a label sits under its column: -1 leading, 0 centred, 1 trailing (:451-456).</summary>
    public static int LabelAlignment(int index, int count, bool thin) =>
        !thin ? 0 : index == 0 ? -1 : index == count - 1 ? 1 : 0;

    /// <summary>
    /// Each half's dots from the axis out: lit in proportion, brighter toward
    /// the tip, the last carrying the remainder; a count of one lights a dot
    /// (:464-475).
    /// </summary>
    public static double[] DotAlphas(double value, double maximum, int rows = Rows)
    {
        var alphas = new double[rows];
        if (!(value > 0) || !(maximum > 0))
        {
            return alphas;
        }

        double lit = Math.Max(1, Math.Min(rows, value / maximum * rows));
        double tip = Math.Ceiling(lit);
        for (int row = 0; row < rows; row++)
        {
            double fill = Math.Min(1, Math.Max(0, lit - row));
            if (fill <= 0)
            {
                continue;
            }

            double taper = 0.35 + 0.65 * (row + 1) / tip;
            alphas[row] = taper * (fill < 1 ? Math.Max(0.35, fill) : 1);
        }

        return alphas;
    }

    /// <summary>
    /// One column's dot centres, shared above the axis and kept below, and
    /// the axis rule (x, y, width, height) (:480-506).
    /// </summary>
    public static BarMatrix Matrix(double width, double height, bool thin)
    {
        int columns = Columns(thin);
        double inset = Gaps(thin).Inset;
        double half = height / 2;
        double pitch = (half - 1 - MatrixEnd) / Rows;
        double across = Math.Min(pitch, (width - inset * 2) / columns);
        double left = (width - across * columns) / 2 + across / 2;
        var up = new List<(double X, double Y)>();
        var down = new List<(double X, double Y)>();
        for (int column = 0; column < columns; column++)
        {
            double x = left + column * across;
            for (int row = 0; row < Rows; row++)
            {
                double offset = pitch * (row + 0.5) + 0.5;
                up.Add((x, half - offset));
                down.Add((x, half + offset));
            }
        }

        return new BarMatrix(pitch, across, left, Radius(thin), up, down, (0, half - 0.5, width, 1));
    }
}

/// <summary>A column's dots: index i is column i / Rows, row i % Rows from the axis out.</summary>
public sealed record BarMatrix(
    double Pitch, double Across, double Left, double Radius,
    IReadOnlyList<(double X, double Y)> Up, IReadOnlyList<(double X, double Y)> Down,
    (double X, double Y, double Width, double Height) Axis);

/// <summary>One bar as drawn: its id, label, shared up and kept down, and what is read for it.</summary>
public sealed record GraphBar(string Id, string Label, double Up, double Down, string Description, DateTimeOffset Start);

/// <summary>What the footer shows for one state (TracesGraphFooter.body, MonitorShell.swift:216-277).</summary>
public sealed record GraphFace(
    IReadOnlyList<GraphBar> Bars, string SharedFigure, string KeptFigure, string RangeLabel,
    string FocusTip, string FocusInk, bool CanFocus, bool FocusSelected,
    bool CanZoomOut, bool CanZoomIn, bool CanStepNext, bool CanJump);

/// <summary>
/// The graph footer's state and every decision it draws from
/// (<c>TracesGraphFooter</c>, MonitorShell.swift:188-346): the range, how
/// many windows back, the hovered bar, and which way the next bars come in.
/// The window's control only draws <see cref="Face"/> and plays
/// <see cref="Change"/>.
/// </summary>
public sealed class TracesGraphState
{
    /// <summary>The footer's fixed height, rule included (:204).</summary>
    public const double Height = 206;

    /// <summary>The bars' scale floor (:228).</summary>
    public const double ScaleFloor = 5;

    /// <summary>The footer's period step: 0.45 s on (0.4, 0, 0.2, 1) (:293-295).</summary>
    public static readonly BezierCurve PeriodSlide = new(0.4, 0, 0.2, 1, 0.45);

    /// <summary>A zoom swaps the bars in place with a fade, on GlassMotion.fast (:281-288).</summary>
    public static readonly BezierCurve ZoomFade = BezierCurve.Token(MonitorMotion.Fast);

    /// <summary>
    /// The footer opening and closing: height 0 to 206 with a move from the
    /// bottom and a fade, on the token curve over Motion.slide 0.25 s
    /// (MonitorWindowView.swift:1034, which the toggle's own easeInOut at
    /// :943 sits under).
    /// </summary>
    public static readonly BezierCurve Footer = BezierCurve.Token(0.25);

    /// <summary>The graph is the Traces tab's (MonitorWindowView.swift:679).</summary>
    public static bool Available(MonitorTab tab) => tab == MonitorTab.Traces;

    /// <summary>The toggle reads pressed only where the graph can show (:940-944).</summary>
    public static bool Shown(bool wanted, MonitorTab tab) => wanted && Available(tab);

    public GraphRange Range { get; private set; } = GraphRange.Days;

    /// <summary>Whole windows back: 0 is now, -1 the window before it.</summary>
    public int Offset { get; private set; }

    /// <summary>The hovered bar's id, or null.</summary>
    public string? Hovered { get; set; }

    /// <summary>How the bars drawn last came in.</summary>
    public GraphSlide Slide { get; private set; } = GraphSlide.None;

    /// <summary>The previous period: its bars move in from the trailing edge (:239).</summary>
    public void Previous() => Step(-1, GraphSlide.FromTrailing);

    /// <summary>The next period, never past now: from the leading edge (:263).</summary>
    public void Next() => Step(1, GraphSlide.FromLeading);

    /// <summary>Back to now, from the leading edge (:265-268).</summary>
    public void JumpToNow()
    {
        Slide = GraphSlide.FromLeading;
        Offset = 0;
    }

    /// <summary>One step wider: back to now, swapped in place (:282-289).</summary>
    public void ZoomOut() => Zoom(TracesGraphModel.ZoomedOut(Range));

    /// <summary>One step narrower: back to now, swapped in place.</summary>
    public void ZoomIn() => Zoom(TracesGraphModel.ZoomedIn(Range));

    /// <summary>
    /// The curve the last change plays on: a period step slides, a zoom
    /// fades; none under Reduce Motion (:293-295, :285).
    /// </summary>
    public BezierCurve? Change(bool reduceMotion) =>
        reduceMotion ? null : Slide == GraphSlide.None ? ZoomFade : PeriodSlide;

    private void Step(int by, GraphSlide from)
    {
        Slide = from;
        Offset = Math.Min(0, Offset + by);
    }

    private void Zoom(GraphRange? to)
    {
        if (to is not { } range)
        {
            return;
        }

        Slide = GraphSlide.None;
        Hovered = null;
        Range = range;
        Offset = 0;
    }

    /// <summary>
    /// What the footer draws. <paramref name="history"/> is the last History
    /// page from a read that did not fail (null while unread or failed); the
    /// legend's shared figure is a dash unless it is a whole page
    /// (<paramref name="complete"/>). <paramref name="tool"/> is the
    /// selection's tool: the graph counts only it, and the binoculars can
    /// focus the map on it (:194-201, :216-277, :297-317).
    /// </summary>
    public GraphFace Face(
        IReadOnlyList<MapHistoryRow>? history, bool complete, IReadOnlyList<MapEntry> sessions, string? tool,
        bool focus, DateTimeOffset now, TimeZoneInfo zone, CultureInfo culture, TracesGraphWords words)
    {
        IReadOnlyList<GraphBucket> buckets = TracesGraphModel.Buckets(
            TracesGraphModel.SharedTimes(history ?? Array.Empty<MapHistoryRow>(), tool),
            TracesGraphModel.KeptTimes(sessions, tool),
            Offset, now, Range, zone, culture);
        GraphBucket? shown = buckets.FirstOrDefault(b => b.Id == Hovered);
        var bars = buckets.Select(b => new GraphBar(
            b.Id, b.Label, b.Shared, b.Kept,
            words.BarText(TracesGraphModel.Day(b.Start, zone, culture), b.Shared, b.Kept), b.Start)).ToList();
        string range = shown is { } hovered
            ? TracesGraphModel.Day(hovered.Start, zone, culture)
            : words.Range(TracesGraphModel.Span(Range), Range == GraphRange.Hours, -Offset);
        bool canFocus = tool is not null;
        string? toolName = tool is null ? null : MapTools.DisplayName(tool);
        return new GraphFace(
            bars,
            TracesGraphModel.Figure(shown?.Shared ?? buckets.Sum(b => b.Shared), history is not null && complete),
            (shown?.Kept ?? buckets.Sum(b => b.Kept)).ToString(CultureInfo.InvariantCulture),
            range,
            words.FocusTip(toolName, focus && canFocus),
            !canFocus ? "graphFocusOff" : focus ? "graphFocusOn" : "graphFocusIdle",
            canFocus,
            focus && canFocus,
            TracesGraphModel.ZoomedOut(Range) is not null,
            TracesGraphModel.ZoomedIn(Range) is not null,
            Offset != 0,
            Offset != 0);
    }

    /// <summary>
    /// The history the bars are drawn from (<c>readable</c>, :299-303) and
    /// whether the legend may count it whole (<c>countable</c>, :305-310):
    /// a page from a read that did not fail, and a whole page only under the
    /// History limit.
    /// </summary>
    public static (IReadOnlyList<MapHistoryRow>? Rows, bool Complete) Readable(IReadOnlyList<MapHistoryRow>? history) =>
        (history, history is not null && history.Count < MapContributions.HistoryLimit);
}
