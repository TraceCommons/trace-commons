using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The stage model's clock rules (checklist O1, P1): every change asks for a
/// redraw whatever the clock does; nothing asks for frames under reduce
/// motion; the Mac's once-only plays (FirstRunStageMemory,
/// FirstRunStage.swift:548-578) do not replay on Back.
/// </summary>
public sealed class FirstRunStageModelTests
{
    private static FirstRunStageModel Model(bool reduce)
    {
        var m = new FirstRunStageModel();
        m.SetReduceMotion(reduce);
        m.SetRows(new[] { ("codex", true), ("claude-code", true), ("cline", false) });
        return m;
    }

    [Fact]
    public void WithAnimationsOffEveryChangeStillRedrawsAndNothingTicks()
    {
        FirstRunStageModel m = Model(reduce: true);
        int redraws = 0;
        m.Changed += () => redraws++;
        m.SetStep(FirstRunStep.Tools, 10);
        m.SetWatched(new[] { "codex" }, 10.1);
        m.SetSharing(SharingPath.Automatic, 10.2);
        Assert.Equal(3, redraws);
        Assert.Null(m.Interval(10.3));
        // The scan is over at once, and the actors are at their places.
        Assert.True(m.ToolsScanned);
        StageSnapshot s = m.Snapshot(10.3);
        Assert.Equal(StageActors.Hub(FirstRunStep.Tools).Side, s.Hub.Side);
        Assert.Equal(1, m.BeamOpacity("codex", 10.1));
    }

    [Fact]
    public void TheRingIsInRingOrderAndTheScanPlaysOnce()
    {
        FirstRunStageModel m = Model(reduce: false);
        Assert.Equal(new[] { "claude-code", "codex", "cline" }, m.Rows.Select(r => r.Source));
        m.SetStep(FirstRunStep.Tools, 0);
        Assert.Equal(0, m.Interval(0.5));
        m.Tick(3.3);
        Assert.True(m.ToolsScanned);
        m.Tick(4.3);
        Assert.Null(m.ScanStartedAt);
        Assert.Equal(1.0 / 30, m.Interval(10));
        m.SetStep(FirstRunStep.PrivateAI, 11);
        m.SetStep(FirstRunStep.Tools, 20);
        Assert.Null(m.ScanStartedAt);
        Assert.Equal(1.0 / 30, m.Interval(25));
    }

    [Fact]
    public void ActorsMoveFromWhereTheyWereOnTheSpatialSpring()
    {
        FirstRunStageModel m = Model(reduce: false);
        m.SetStep(FirstRunStep.Tools, 0);
        m.Tick(3.3);
        m.Tick(4.3);
        m.SetStep(FirstRunStep.PrivateAI, 10);
        StageFrame from = StageActors.Hub(FirstRunStep.Tools), to = StageActors.Hub(FirstRunStep.PrivateAI);
        StageFrame mid = m.Snapshot(10.3).Hub;
        // spatial(0.3) = 0.634402070 (Spring(duration: 0.9, bounce: 0.04), probe).
        Assert.Equal(from.Side + (to.Side - from.Side) * 0.634402070, mid.Side, 5);
        Assert.Equal(0, m.Interval(10.3));
        Assert.Equal(to, m.Snapshot(12).Hub);
    }
}
