using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The N mark parses to the Mac's own path. Expected values were printed by
/// the throwaway probe from <c>GlassBrandMark.unit</c> (GlassBrandMark.swift:20-27,
/// parsed by SVGPathData.swift:14) at 3372425f4: its CoreGraphics
/// <c>boundingRect</c> and its element counts.
/// </summary>
public class NearMarkPathTests
{
    [Fact]
    public void TheMarkHasTheMacsSteps()
    {
        // probe: elements = close 1, curve 20, line 13, move 1.
        var steps = NearMarkPath.Unit;
        Assert.Equal(1, steps.Count(s => s.Kind == PathStepKind.Move));
        Assert.Equal(13, steps.Count(s => s.Kind == PathStepKind.Line));
        Assert.Equal(20, steps.Count(s => s.Kind == PathStepKind.Cubic));
        Assert.Equal(1, steps.Count(s => s.Kind == PathStepKind.Close));
    }

    [Fact]
    public void TheUnitMarkHasTheMacsBounds()
    {
        // probe: unit.bounds = 2.0501920516835526e-05 2.4334720365004614e-05
        // 0.9999538660049438 0.9898765683174133 (CoreGraphics works in
        // Float on this path, so agreement is to about 1e-5).
        LayoutRect b = NearMarkPath.Bounds(NearMarkPath.Unit);
        Assert.Equal(2.0501920516835526e-05, b.X, 4);
        Assert.Equal(2.4334720365004614e-05, b.Y, 4);
        Assert.Equal(0.9999538660049438, b.Width, 4);
        Assert.Equal(0.9898765683174133, b.Height, 4);
    }

    [Fact]
    public void AnUnsupportedCommandIsRefusedRatherThanDrawnInPart()
    {
        Assert.Throws<System.FormatException>(() => NearMarkPath.Parse("M0,0Q1,1,2,2"));
    }
}
