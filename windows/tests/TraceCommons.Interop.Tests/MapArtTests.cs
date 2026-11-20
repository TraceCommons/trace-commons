using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop.Map;
using TraceCommons.Interop.Shell;
using SvgPathData = TraceCommons.Interop.Map.SvgPathData;
using Xunit;
using static TraceCommons.Interop.Tests.FlowMapSceneTests;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Map art (slice W6) and the map's motion against the Mac's own numbers
/// (LibraryStation.swift, LibraryOrb.swift, Indicators.swift:563-885,
/// SVGPathData.swift, GlassLogoPaths.swift, SwiftUI's UnitCurve and Spring;
/// probe at 841d81ba5).
/// </summary>
public class MapArtTests
{
    [Fact]
    public void TheStationIsTheMacsMeshFaceForFace()
    {
        // #2182's pins: 506 faces, 2216 corners; the mark's outline 100 points.
        Assert.Equal(MapProbe.Get("stationFaces").GetInt32(), LibraryStation.Mesh.Count);
        Assert.Equal(MapProbe.Get("stationCorners").GetInt32(), LibraryStation.Mesh.Sum(f => f.Points.Count));
        JsonElement outline = MapProbe.Get("markOutline");
        var ours = LibraryStation.MarkOutline(30);
        Assert.Equal(outline.GetArrayLength(), ours.Count);
        for (int i = 0; i < ours.Count; i++)
        {
            Near(outline[i][0].GetDouble(), ours[i].X, $"outline {i}");
            Near(outline[i][1].GetDouble(), ours[i].Y, $"outline {i}");
        }

        foreach (JsonElement row in MapProbe.Get("stationTurned").EnumerateArray())
        {
            int face = row[0].GetInt32();
            Vec3 q = LibraryStation.Turned(LibraryStation.Mesh[face].Points[0], 0.6);
            Near(row[1].GetDouble(), q.X, $"face {face}");
            Near(row[2].GetDouble(), q.Y, $"face {face}");
            Near(row[3].GetDouble(), q.Z, $"face {face}");
        }

        int k = 0;
        foreach (StationFace face in LibraryStation.Mesh.Where((_, i) => i % 37 == 0))
        {
            JsonElement c = MapProbe.Get("stationColors")[k++];
            Near(c[0].GetDouble(), face.Color.X);
            Near(c[1].GetDouble(), face.Color.Y);
            Near(c[2].GetDouble(), face.Color.Z);
            Assert.Equal(c[3].GetDouble() == 1, face.Mark);
            Assert.Equal(c[4].GetDouble() == 1, face.MarkCap);
        }
    }

    [Fact]
    public void TheStationAndOrbTurnAtTheMacsRateAndRestUnderReduceMotion()
    {
        double[] seconds = { 0, 13, 26, 39.5 };
        JsonElement mac = MapProbe.Get("stationAngle");
        for (int i = 0; i < seconds.Length; i++)
        {
            Near(mac[i].GetDouble(), LibraryStation.Angle(seconds[i], false));
        }

        Near(mac[seconds.Length].GetDouble(), LibraryStation.Angle(5, true));
        // LibraryStation.swift:33-39 (15 a second since #2268), LibraryOrb.swift:24-36.
        Assert.Equal(26, LibraryStation.TurnSeconds);
        Assert.Equal(1.0 / 15, LibraryStation.FrameInterval);
        Assert.Equal(18, LibraryOrb.TurnSeconds);
        Assert.Equal(6, LibraryOrb.Meridians);
        Assert.Equal(1.0 / 15, LibraryOrb.FrameInterval);

        double[] angles = { 0.4, 1.0, 2.5 };
        for (int i = 0; i < angles.Length; i++)
        {
            var widths = LibraryOrb.MeridianWidths(angles[i]);
            for (int m = 0; m < widths.Count; m++)
            {
                Near(MapProbe.Get("orbMeridians")[i][m][0].GetDouble(), widths[m].Width);
                Assert.Equal(MapProbe.Get("orbMeridians")[i][m][1].GetDouble() == 1, widths[m].Near);
            }
        }

        Near(MapProbe.Get("orbRadius").GetDouble(), LibraryOrb.Radius(104, 104));
    }

    [Fact]
    public void TheFieldsMarksDotsLightAndShimmerAreTheMacs()
    {
        var size = new MapSize(440, 614);
        var origins = MapField.Origins(size);
        JsonElement mac = MapProbe.Get("fieldOrigins");
        Assert.Equal(mac[0].GetDouble(), origins.Count);
        for (int i = 0; i < 4; i++)
        {
            Near(mac[1 + 2 * i].GetDouble(), origins[i].X);
            Near(mac[2 + 2 * i].GetDouble(), origins[i].Y);
        }

        Assert.Equal(MapProbe.Get("fieldOriginsNear").GetInt32(), MapField.Origins(size, (60, 60, 280, 280)).Count);
        var dots = MapField.Dots(size);
        JsonElement macDots = MapProbe.Get("fieldDots");
        Assert.Equal(macDots[0].GetDouble(), dots.Count);
        for (int i = 0; i < 4; i++)
        {
            Near(macDots[1 + 2 * i].GetDouble(), dots[i].X);
            Near(macDots[2 + 2 * i].GetDouble(), dots[i].Y);
        }

        double[] distances = { 0, 10, 70, 139, 140, 200 };
        for (int i = 0; i < distances.Length; i++)
        {
            Near(MapProbe.Get("light")[i].GetDouble(), MapField.Light(distances[i]));
        }

        double[] sweeps = { 0, 0.25, 0.5, 0.75, 1, 1.5 };
        for (int i = 0; i < sweeps.Length; i++)
        {
            var band = MapField.ShimmerBand(sweeps[i], size);
            JsonElement m = MapProbe.Get("shimmerBand")[i];
            Near(m[0].GetDouble(), band.Start.X, $"band {sweeps[i]}");
            Near(m[1].GetDouble(), band.Start.Y, $"band {sweeps[i]}");
            Near(m[2].GetDouble(), band.End.X, $"band {sweeps[i]}");
            Near(m[3].GetDouble(), band.End.Y, $"band {sweeps[i]}");
        }
    }

    [Fact]
    public void TheShimmerPassesOneSecondAfterShowingThenEveryFortyAndNeverUnderReduceMotion()
    {
        // Indicators.swift:666-674, 795-806: progress on timingCurve(0.3, 0, 0.15, 1) over 1.1 s.
        JsonElement curve = MapProbe.Get("curveShimmer");
        Assert.Equal(0, MapField.Sweep(0.99, false));
        for (int i = 0; i <= 9; i++)
        {
            // SwiftUI's curve solver works in single precision: within 1e-5.
            Assert.True(Math.Abs(curve[i].GetDouble() - MapField.Sweep(1 + i * 0.11, false)) < 1e-5, $"pass 1 at {i}");
            Assert.True(Math.Abs(curve[i].GetDouble() - MapField.Sweep(41 + i * 0.11, false)) < 1e-5, $"pass 2 at {i}");
        }

        Assert.Equal(0, MapField.Sweep(1 + 1.1, false));
        Assert.Equal(0, MapField.Sweep(1.5, true));
        Assert.Equal(1, MapField.UntilNextPass(0));
        Assert.Equal(0, MapField.UntilNextPass(1.5));
        Near(38.9, MapField.UntilNextPass(2.1));
    }

    [Fact]
    public void EveryLogoParsesToTheMacsPath()
    {
        // SVGPathData.swift over GlassLogoPaths.swift (generated here as MapArt.Logos).
        JsonElement logos = MapProbe.Get("logos");
        Assert.Equal(logos.EnumerateObject().Select(p => p.Name).OrderBy(n => n), MapArt.Logos.Keys.OrderBy(n => n));
        foreach ((string name, LogoArtwork art) in MapArt.Logos)
        {
            var steps = art.Paths.SelectMany(p => SvgPathData.Parse(p) ?? throw new InvalidOperationException(name)).ToList();
            JsonElement elements = logos.GetProperty(name).GetProperty("elements");
            Assert.Equal(elements.GetArrayLength(), steps.Count);
            for (int i = 0; i < steps.Count; i++)
            {
                JsonElement e = elements[i];
                PathStep s = steps[i];
                int type = (int)e[0].GetDouble();
                Assert.True(type == Cg(s.Kind), $"{name} {i}: Mac {type}, Windows {s.Kind}");
                double[] ours = s.Kind switch
                {
                    PathStepKind.Cubic => new[] { s.X1, s.Y1, s.X2, s.Y2, s.X, s.Y },
                    PathStepKind.Close => Array.Empty<double>(),
                    _ => new[] { s.X, s.Y },
                };
                for (int j = 0; j < ours.Length; j++)
                {
                    Near(e[1 + j].GetDouble(), ours[j], $"{name} {i}.{j}");
                }
            }
        }
    }

    private static int Cg(PathStepKind kind) => kind switch
    {
        PathStepKind.Move => 0,
        PathStepKind.Line => 1,
        PathStepKind.Cubic => 3,
        _ => 4,
    };

    [Fact]
    public void TheMapsCurvesAndSpringsAreSwiftUIsOwn()
    {
        // Each sampled at 0, 0.1 ... 1 of its duration (probe: UnitCurve.value(at:)).
        AssertCurve("curveShimmer", MapMotion.ShimmerPass);
        AssertCurve("curveEaseOut", BezierCurve.EaseOut(0.18));
        // The pointer light's fade runs on the token curve (probe "curveToken" is 0.25 s of it).
        AssertCurve("curveToken", BezierCurve.Token(0.25));
        Assert.Equal(0.15, MapMotion.PointerLightFade.Duration);

        // Spring(response:dampingRatio:).value(target: 1, time:) at 0.05 ... 0.8 s.
        var springs = new Dictionary<string, SpringCurve>
        {
            ["0.42/0.8"] = MapMotion.CoinShuffle,
            ["0.36/0.82"] = MapMotion.CoinOpen,
            ["0.3/0.7"] = MapMotion.CoinGrow,
            ["0.34/0.84"] = (SpringCurve)MapMotion.PileOpen[0].Curve,
            ["0.3/0.86"] = (SpringCurve)MapMotion.PileOpen[1].Curve,
            ["0.28/0.9"] = (SpringCurve)MapMotion.PileClose[1].Curve,
            ["0.34/0.86"] = (SpringCurve)MapMotion.PileClose[2].Curve,
            // The transfer cards arrive on the notice stack's own spring (PopoverNotice.swift:238).
            ["0.36/0.86"] = new SpringCurve(NoticeStack.SpringResponse, NoticeStack.SpringDamping),
        };
        double[] times = { 0.05, 0.1, 0.2, 0.3, 0.5, 0.8 };
        foreach ((string key, SpringCurve spring) in springs)
        {
            JsonElement mac = MapProbe.Get("springs").GetProperty(key);
            for (int i = 0; i < times.Length; i++)
            {
                Assert.True(Math.Abs(mac[i].GetDouble() - spring.At(times[i])) < 1e-4, $"{key} at {times[i]}: Mac {mac[i].GetDouble()}, Windows {spring.At(times[i])}");
            }
        }
    }

    private static void AssertCurve(string key, BezierCurve curve)
    {
        JsonElement mac = MapProbe.Get(key);
        for (int i = 0; i <= 10; i++)
        {
            Assert.True(Math.Abs(mac[i].GetDouble() - curve.At(curve.Duration * i / 10)) < 1e-5, $"{key} at {i / 10.0}: Mac {mac[i].GetDouble()}, Windows {curve.At(curve.Duration * i / 10)}");
        }
    }

    [Fact]
    public void TheFlowingDashRunsFromTheArcsStartAtEighteenPointsASecond()
    {
        // FlowMapView.swift:84 phase = seconds * 18; :294 dashPhase: -phase * scale.
        Assert.Equal(-18 * 1.5, MapMotion.DashPhase(1, 1.5, false));
        Assert.True(MapMotion.DashPhase(2, 1, false) < MapMotion.DashPhase(1, 1, false));
        Assert.Equal(0, MapMotion.DashPhase(3, 1, true));
        // Direct2D counts the offset in stroke widths: 1.4 * 1.3 at scale 1.
        Assert.Equal(-18 / (1.4 * 1.3), MapMotion.DashOffsetInWidths(MapMotion.DashPhase(1, 1, false), 1.4 * 1.3), 9);
    }

    [Fact]
    public void TheRestingClockRunsAtThirtyForDashesFifteenForTheLibraryAndNotAtAllOtherwise()
    {
        var cases = new[] { (false, true, true, true), (false, false, true, true), (true, true, true, true), (false, false, true, false), (false, true, false, false) };
        JsonElement mac = MapProbe.Get("resting");
        for (int i = 0; i < cases.Length; i++)
        {
            (double interval, bool paused) = MapMotion.Resting(cases[i].Item1, cases[i].Item2, cases[i].Item3, cases[i].Item4);
            // The probe was recorded with the Mac's library at 20 a second;
            // #2268 lowers it to 15, and Windows follows.
            double expected = Math.Abs(mac[i][0].GetDouble() - 1.0 / 20) < 1e-9 ? LibraryStation.FrameInterval : mac[i][0].GetDouble();
            Near(expected, interval, $"case {i}");
            Assert.Equal(mac[i][1].GetDouble() == 1, paused);
        }

        Assert.False(MapMotion.ClockRuns(true, true, windowVisible: false));
        Assert.False(MapMotion.ClockRuns(true, loaded: false, windowVisible: true));
        Assert.True(MapMotion.ClockRuns(true, true, true));
    }

    [Fact]
    public void ThePileOpensTallThenWideThenNamesAndClosesInReverse()
    {
        // MapToolPile.swift:173-199: 0 / +260 ms / +460 ms opening, 0 / +100 ms / +320 ms closing.
        Assert.Equal(new[] { PilePart.Tall, PilePart.Wide, PilePart.Names }, MapMotion.PileOpen.Select(s => s.Part));
        Assert.Equal(new[] { 0, 0.26, 0.46 }, MapMotion.PileOpen.Select(s => s.Delay));
        Assert.Equal(new[] { PilePart.Names, PilePart.Wide, PilePart.Tall }, MapMotion.PileClose.Select(s => s.Part));
        Assert.Equal(new[] { 0, 0.10, 0.32 }, MapMotion.PileClose.Select(s => s.Delay));
        Assert.Equal(0, MapMotion.PilePartAt(PilePart.Wide, true, 0.2, 0, false));
        Assert.True(MapMotion.PilePartAt(PilePart.Wide, true, 0.4, 0, false) > 0);
        Assert.Equal(1, MapMotion.PilePartAt(PilePart.Names, true, 0, 0, reduceMotion: true));
    }

    [Fact]
    public void TheMapsLayoutStaticsAreTheMacs()
    {
        // FlowMapView.swift:764-804 (FlowMapGeometry), :702-735 (card), :456-469 (labels).
        double[][] fits = { new[] { 440.0, 614, 1 }, new[] { 440.0, 614, 1.5 }, new[] { 900.0, 500, 0.6 } };
        for (int i = 0; i < fits.Length; i++)
        {
            var g = new MapGeometry(new MapSize(fits[i][0], fits[i][1]), fits[i][2]);
            Near(MapProbe.Get("geometry")[i][0].GetDouble(), g.Scale);
            Near(MapProbe.Get("geometry")[i][1].GetDouble(), g.OriginX);
            Near(MapProbe.Get("geometry")[i][2].GetDouble(), g.OriginY);
        }

        var focused = new MapGeometry(new MapSize(440, 614), 1, new MapPoint(290, 380));
        Near(MapProbe.Get("geometry")[3][0].GetDouble(), focused.Scale);
        Near(MapProbe.Get("geometry")[3][1].GetDouble(), focused.OriginX);
        Near(MapProbe.Get("geometry")[3][2].GetDouble(), focused.OriginY);

        double[][] cards = { new[] { 100.0, 32, 240, 440 }, new[] { 380.0, 16, 240, 440 }, new[] { 220.0, 75, 240, 440 }, new[] { 220.0, 75, 240, 300 } };
        for (int i = 0; i < cards.Length; i++)
        {
            Near(MapProbe.Get("cardX")[i].GetDouble(), MapGeometry.CardX(cards[i][0], cards[i][1], cards[i][2], cards[i][3]));
        }

        double[] scales = { 0.5, 1, 1.4 };
        for (int i = 0; i < scales.Length; i++)
        {
            var t = MapGeometry.LabelType(scales[i]);
            Near(MapProbe.Get("labelType")[i][0].GetDouble(), t.LabelSize);
            Near(MapProbe.Get("labelType")[i][1].GetDouble(), t.SublabelSize);
            Near(MapProbe.Get("labelType")[i][2].GetDouble(), t.LabelOffset);
            Near(MapProbe.Get("labelType")[i][3].GetDouble(), t.SublabelOffset);
        }

        Assert.Equal(MapProbe.Get("shortLabel").EnumerateArray().Select(e => e.GetString()),
            new[] { "short", "exactly14chars", "a folder name that is long" }.Select(MapGeometry.ShortLabel));
    }
}
