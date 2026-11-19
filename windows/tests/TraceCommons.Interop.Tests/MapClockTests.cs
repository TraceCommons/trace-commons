using System;
using System.IO;
using System.Linq;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The map's one clock and its pile motion, the decisions the Win2D map
/// runs on (spec "Animation coverage": each row's own frame rate; checklist
/// P1: nothing ticks hidden, minimised or with its pane closed, and reduce
/// motion stops it).
/// </summary>
public class MapClockTests
{
    private static MapClockInputs Idle => new(
        AnimationsEnabled: true, Loaded: true, WindowVisible: true, PaneShown: true,
        HasLibrary: false, Flows: false, Healthy: true, TransferPlaying: false, PileMoving: false,
        LightFading: false, SecondsShown: 5, NextDeadlineIn: null);

    [Fact]
    public void NothingTicksHiddenMinimisedUnloadedWithItsPaneClosedOrUnderReduceMotion()
    {
        MapClockInputs busy = Idle with { HasLibrary = true, Flows = true, TransferPlaying = true };
        Assert.NotNull(MapMotion.ClockInterval(busy));
        Assert.Null(MapMotion.ClockInterval(busy with { WindowVisible = false }));
        Assert.Null(MapMotion.ClockInterval(busy with { Loaded = false }));
        Assert.Null(MapMotion.ClockInterval(busy with { PaneShown = false }));
        Assert.Null(MapMotion.ClockInterval(busy with { AnimationsEnabled = false }));
    }

    [Fact]
    public void EachPartRunsAtTheMacsOwnRate()
    {
        // Transfers and the pile at 60 (FlowMapView.swift:107, :140); the
        // dashes at 30 (:283); the station and orb alone at 20
        // (LibraryStation.swift:39).
        Assert.Equal(1.0 / 60, MapMotion.ClockInterval(Idle with { HasLibrary = true, Flows = true, TransferPlaying = true }));
        Assert.Equal(1.0 / 60, MapMotion.ClockInterval(Idle with { PileMoving = true }));
        Assert.Equal(1.0 / 60, MapMotion.ClockInterval(Idle with { LightFading = true }));
        Assert.Equal(1.0 / 30, MapMotion.ClockInterval(Idle with { HasLibrary = true, Flows = true }));
        Assert.Equal(1.0 / 20, MapMotion.ClockInterval(Idle with { HasLibrary = true }));
        // Paused or stale dashes do not move (FlowMapView.swift:278-285).
        Assert.Equal(1.0 / 20, MapMotion.ClockInterval(Idle with { HasLibrary = true, Flows = true, Healthy = false }));
    }

    [Fact]
    public void WithNothingMovingItWaitsForTheNextShimmerOrCard()
    {
        // The shimmer passes 1 s after showing and every 40 s (Indicators.swift:666-667).
        Assert.Equal(1.0 / 60, MapMotion.ClockInterval(Idle with { SecondsShown = 1.5 }));
        Assert.Equal(36, MapMotion.ClockInterval(Idle with { SecondsShown = 5 })!.Value, 9);
        Assert.Equal(2, MapMotion.ClockInterval(Idle with { SecondsShown = 5, NextDeadlineIn = 2 }));
    }

    [Fact]
    public void ThePileStagesTallWideThenNamesAndRetargetsMidway()
    {
        var pile = new ToolPileState();
        pile.Hover(true, 10);
        (double tall, double wide, double names) = pile.Stages(10.2, reduceMotion: false);
        Assert.True(tall > 0.5, $"tall {tall}");
        Assert.Equal(0, wide);
        Assert.Equal(0, names);
        (_, wide, names) = pile.Stages(10.4, false);
        Assert.True(wide > 0, $"wide {wide}");
        Assert.Equal(0, names);
        (_, _, names) = pile.Stages(10.6, false);
        Assert.True(names > 0 && names < 1, $"names {names}");
        Assert.True(pile.Moving(10.6));

        // Leaving: the names go first, the card stays tall until +320 ms.
        pile.Hover(false, 11);
        (tall, _, names) = pile.Stages(11.1, false);
        Assert.True(names < 1);
        Assert.True(tall > 0.99, $"tall {tall}");
        Assert.False(pile.Moving(13));
    }

    [Fact]
    public void UnderReduceMotionThePileAndCoinsJumpToTheirEnds()
    {
        var pile = new ToolPileState();
        pile.Hover(true, 0);
        Assert.Equal((1.0, 0.0, 0.0), pile.Stages(0, reduceMotion: true));
        var coin = pile.Coin("tool:codex", 0, open: false, grown: false, now: 0, reduceMotion: true);
        coin = pile.Coin("tool:codex", 2, open: true, grown: true, now: 0.01, reduceMotion: true);
        Assert.Equal((2.0, 1.0, MapMotion.CoinGrown), coin);
    }

    [Theory]
    [InlineData(true, 32.5, 1.0 / 30, true)]   // waiting for a shimmer, the arcs want 30 a second
    [InlineData(true, 1.0 / 20, 1.0 / 60, true)]
    [InlineData(true, 1.0 / 60, 1.0 / 30, true)]
    [InlineData(false, 32.5, 1.0 / 30, true)]  // stopped: start it
    [InlineData(true, 1.0 / 30, 1.0 / 30, false)] // already on the wanted beat: leave its phase alone
    public void AChangedIntervalStartsTheClockAgain(bool running, double current, double wanted, bool restarts)
    {
        Assert.Equal(restarts, MapMotion.ClockRestarts(running, current, wanted));
    }

    [Fact]
    public void ACoinGrowsOnTheMacsSpringAsATraceArrives()
    {
        var pile = new ToolPileState();
        pile.Coin("tool:codex", 0, false, grown: false, now: 0, reduceMotion: false);
        pile.Coin("tool:codex", 0, false, grown: true, now: 1, reduceMotion: false);
        double scale = pile.Coin("tool:codex", 0, false, grown: true, now: 1.1, reduceMotion: false).Scale;
        // 1 + 0.3 * spring(0.3, 0.7) at 0.1 s (probe "springs").
        double mac = MapProbe.Get("springs").GetProperty("0.3/0.7")[1].GetDouble();
        Assert.True(Math.Abs(1 + 0.3 * mac - scale) < 1e-4, $"Mac {1 + 0.3 * mac}, Windows {scale}");
    }

    [Fact]
    public void EverySampleSetAnswersEveryReadTheMapMakes()
    {
        // Checklist F1: each method the map sends is recorded in every set
        // the sample client serves, or refused by coreDown alone.
        string root = Path.Combine(AppContext.BaseDirectory, "RecordedSamples");
        foreach (string set in SampleDaemon.Sets.Where(s => s != SampleDaemon.CoreDown))
        {
            var sample = new SampleDaemon(set, root);
            foreach (string method in MapFeed.Methods)
            {
                DaemonResponse response = DaemonResponse.Parse(sample.Call(method));
                Assert.False(response.IsError, $"{set} refused {method}");
            }
        }
    }

    [Fact]
    public void TheStillLayerRedrawsOnlyWhenSomethingInItChanged()
    {
        // Review finding 3 (#2215): the Private AI view has no tool pile, so
        // "the pile moved between the layers" must never be read from it, or
        // the field, nodes and text are redrawn at the arc rate.
        Assert.False(MapMotion.StillRedraws(fieldMoves: false, fieldMoved: false, hasPile: false, pileMoving: false, pileInStill: false));
        Assert.False(MapMotion.StillRedraws(fieldMoves: false, fieldMoved: false, hasPile: false, pileMoving: true, pileInStill: true));

        // A pile moves into the moving layer as it starts, and back as it rests.
        Assert.True(MapMotion.StillRedraws(fieldMoves: false, fieldMoved: false, hasPile: true, pileMoving: true, pileInStill: true));
        Assert.True(MapMotion.StillRedraws(fieldMoves: false, fieldMoved: false, hasPile: true, pileMoving: false, pileInStill: false));
        Assert.False(MapMotion.StillRedraws(fieldMoves: false, fieldMoved: false, hasPile: true, pileMoving: false, pileInStill: true));
        Assert.False(MapMotion.StillRedraws(fieldMoves: false, fieldMoved: false, hasPile: true, pileMoving: true, pileInStill: false));

        // A shimmer pass, and the frame after it ends, redraw the field.
        Assert.True(MapMotion.StillRedraws(fieldMoves: true, fieldMoved: false, hasPile: false, pileMoving: false, pileInStill: false));
        Assert.True(MapMotion.StillRedraws(fieldMoves: false, fieldMoved: true, hasPile: false, pileMoving: false, pileInStill: false));
    }

    [Fact]
    public void TheMapsTickAsksTheStillLayerDecision()
    {
        string source = ShellSource("TraceCommons.App/Controls/TracesMapView.cs");
        Assert.Contains("MapMotion.StillRedraws(", source, StringComparison.Ordinal);
        Assert.DoesNotContain("pileMoving == _pileInStill", source, StringComparison.Ordinal);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
