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
        LightFading: false, SecondsShown: 5, NextDeadlineIn: null, LibraryOnMovingLayer: false);

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
        // dashes at 30 (:283).
        Assert.Equal(1.0 / 60, MapMotion.ClockInterval(Idle with { HasLibrary = true, Flows = true, TransferPlaying = true }));
        Assert.Equal(1.0 / 60, MapMotion.ClockInterval(Idle with { PileMoving = true }));
        Assert.Equal(1.0 / 60, MapMotion.ClockInterval(Idle with { LightFading = true }));
        Assert.Equal(1.0 / 30, MapMotion.ClockInterval(Idle with { HasLibrary = true, Flows = true }));
    }

    /// <summary>
    /// #2268: the station is not redrawn at the dashes' rate, only when its
    /// own 15-a-second frame changes. On Windows one tick source serves both
    /// layers, so they are composed together: the moving layer's clock turns
    /// the station whenever it runs at least that fast, and the station's own
    /// clock runs only while the moving one is slower (no dashes, waiting for
    /// the shimmer). A second clock beside the dashes' composed the window
    /// about 12 more times a second and cost CPU (PR #2278).
    /// </summary>
    [Fact]
    public void TheStationTurnsAtItsOwnRateOnOneTickSource()
    {
        // At rest the moving layer's clock runs for the dashes only.
        Assert.Equal(36, MapMotion.ClockInterval(Idle with { HasLibrary = true })!.Value, 9);
        Assert.Equal(36, MapMotion.ClockInterval(Idle with { HasLibrary = true, Flows = true, Healthy = false })!.Value, 9);
        Assert.Equal(1.0 / 30, MapMotion.ClockInterval(Idle with { HasLibrary = true, Flows = true }));

        // The station's own clock runs only while the moving one is slower than 15 a second.
        Assert.Equal(1.0 / 15, MapMotion.LibraryInterval(Idle with { HasLibrary = true }));
        Assert.Equal(1.0 / 15, MapMotion.LibraryInterval(Idle with { HasLibrary = true, Flows = true, Healthy = false }));
        Assert.Null(MapMotion.LibraryInterval(Idle with { HasLibrary = true, Flows = true }));
        Assert.Null(MapMotion.LibraryInterval(Idle with { HasLibrary = true, SecondsShown = 1.5 }));
        Assert.Null(MapMotion.LibraryInterval(Idle with { HasLibrary = true, LightFading = true }));
        Assert.Null(MapMotion.LibraryInterval(Idle));

        // On the moving layer, the moving clock turns it and its own stops.
        MapClockInputs onMoving = Idle with { HasLibrary = true, LibraryOnMovingLayer = true };
        Assert.Equal(1.0 / 15, MapMotion.ClockInterval(onMoving));
        Assert.Equal(1.0 / 30, MapMotion.ClockInterval(onMoving with { Flows = true }));
        Assert.Null(MapMotion.LibraryInterval(onMoving));

        // It stops where every map clock stops.
        MapClockInputs turning = Idle with { HasLibrary = true };
        Assert.Null(MapMotion.LibraryInterval(turning with { WindowVisible = false }));
        Assert.Null(MapMotion.LibraryInterval(turning with { Loaded = false }));
        Assert.Null(MapMotion.LibraryInterval(turning with { PaneShown = false }));
        Assert.Null(MapMotion.LibraryInterval(turning with { AnimationsEnabled = false }));
    }

    /// <summary>The station's frame of the turn: 15 a second, 390 a turn, the same a turn later.</summary>
    [Fact]
    public void TheStationsFrameIsFifteenASecondAroundTheTurn()
    {
        double turn = LibraryStation.TurnSeconds;
        Assert.Equal(390, MapMotion.LibraryFrames(turn));
        Assert.Equal(0, MapMotion.LibraryFrameIndex(0, turn));
        Assert.Equal(0, MapMotion.LibraryFrameIndex(1.0 / 15 - 1e-9, turn));
        Assert.Equal(1, MapMotion.LibraryFrameIndex(1.0 / 15 + 1e-9, turn));
        Assert.Equal(15, MapMotion.LibraryFrameIndex(1.0 + 1e-9, turn));
        Assert.Equal(389, MapMotion.LibraryFrameIndex(turn - 1e-6, turn));
        foreach (double t in new[] { 3.31, 17.03, 7e8 + 0.52, -2.23 })
        {
            int index = MapMotion.LibraryFrameIndex(t, turn);
            Assert.InRange(index, 0, 389);
            Assert.Equal(index, MapMotion.LibraryFrameIndex(t + turn, turn));
        }

        // Fifteen changes in a second, at whatever rate it is asked.
        int changes = 0, last = MapMotion.LibraryFrameIndex(100, turn);
        for (int tick = 1; tick <= 210; tick++)
        {
            int index = MapMotion.LibraryFrameIndex(100 + tick / 210.0 * 1.0, turn);
            changes += index != last ? 1 : 0;
            last = index;
        }

        Assert.InRange(changes, 15, 16);
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
    [InlineData(true, 1.0 / 15, 1.0 / 60, true)]
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
}
