using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop.Map;
using Xunit;
using static TraceCommons.Interop.Tests.FlowMapSceneTests;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Transfers (slice W4) against the Mac's <c>MapTransferRun</c>,
/// <c>MapTransfer.batch</c> and <c>MapTransferDrawing</c>
/// (MapTransfers.swift, MapToolPile.swift at 841d81ba5, probe "runs",
/// "climb", "pulse", "rise", "rim", "dock", "smoke", "rocket").
/// </summary>
public class MapTransferTests
{
    private static readonly MapTransfer[] Three =
    {
        new("a", "p1", MapTools.ClaudeCode, TransferAction.Contribute),
        new("b", "p2", MapTools.Codex, TransferAction.Contribute),
        new("c", "p1", MapTools.ClaudeCode, TransferAction.Contribute),
    };

    [Fact]
    public void ThePhaseLengthsAreTheMacs()
    {
        // MapTransfers.swift:112-125.
        Assert.Equal(1.05, MapTransferRun.Step);
        Assert.Equal(0.70, MapTransferRun.Inbound);
        Assert.Equal(0.45, MapTransferRun.Orbit);
        Assert.Equal(0.70, MapTransferRun.Outbound);
        Assert.Equal(0.5, MapTransferRun.Settle);
        Assert.Equal(2.6, MapTransferRun.Flight);
        Assert.Equal(1.3, MapTransferRun.Linger);
        Assert.Equal(1.6, MapTransferRun.KeptLinger);
        Assert.Equal(0.9, MapTransferRun.Pulse);
        Assert.Equal(1.1, MapTransferRun.Ascent);
        // MapTransfers.swift:262-266; PopoverNotice.swift:202 draws 3.
        Assert.Equal(5, MapTransferPlayer.Hold);
        Assert.Equal(6, MapTransferPlayer.StackLimit);
    }

    [Theory]
    [InlineData("launch")]
    [InlineData("standard")]
    [InlineData("keep")]
    public void EveryMomentOfARunIsTheMacs(string name)
    {
        JsonElement mac = MapProbe.Get("runs").GetProperty(name);
        TracesMapStyle style = name == "standard" ? TracesMapStyle.Standard : TracesMapStyle.Launch;
        var transfers = name == "keep" ? Three.Select(t => t with { Action = TransferAction.Keep }).ToArray() : Three;
        var run = new MapTransferRun(transfers, 0, style);
        Near(mac.GetProperty("allArrived").GetDouble(), run.AllArrived);
        Near(mac.GetProperty("liftOff").GetDouble(), run.LiftOff);
        Near(mac.GetProperty("end").GetDouble(), run.End);
        Assert.Equal(mac.GetProperty("rises").GetBoolean(), run.Rises);
        JsonElement times = mac.GetProperty("times");
        string[] pile = { "tool:codex", "tool:claude-code", "tool:cline" };
        for (int i = 0; i < times.GetArrayLength(); i++)
        {
            double t = times[i].GetDouble();
            for (int k = 0; k < transfers.Length; k++)
            {
                AssertLeg(mac.GetProperty("legs")[i][k].GetString()!, run.LegOf(k, t), $"leg {k} at {t}");
            }

            Assert.Equal(mac.GetProperty("arrived")[i].GetInt32(), run.ArrivedBy(t));
            Near(mac.GetProperty("flight")[i].GetDouble(), run.FlightAt(t) ?? -1, $"flight at {t}");
            Assert.Equal(mac.GetProperty("order")[i].EnumerateArray().Select(e => e.GetString()), run.Order(pile, t));
            Assert.Equal(mac.GetProperty("current")[i].GetString(), run.Current(t)?.Id ?? string.Empty);
            Assert.Equal(mac.GetProperty("moving")[i].GetBoolean(), run.Moving(t));
            var orbits = mac.GetProperty("orbits")[i].EnumerateObject().ToDictionary(p => p.Name, p => p.Value.GetDouble());
            IReadOnlyDictionary<string, double> ours = run.Orbits(t);
            Assert.Equal(orbits.Keys.OrderBy(k => k), ours.Keys.OrderBy(k => k));
            foreach ((string key, double value) in orbits)
            {
                Near(value, ours[key], key);
            }
        }
    }

    /// <summary>A leg against the probe's spelling of it (Swift's enum description, "inbound(0.5)").</summary>
    private static void AssertLeg(string mac, Leg leg, string where)
    {
        int open = mac.IndexOf('(');
        string kind = open < 0 ? mac : mac[..open];
        Assert.True(kind == leg.Kind.ToString().ToLowerInvariant(), $"{where}: Mac {mac}, Windows {leg}");
        if (open >= 0)
        {
            Near(double.Parse(mac[(open + 1)..^1], System.Globalization.CultureInfo.InvariantCulture), leg.Value, where);
        }
    }

    [Fact]
    public void TheClimbEaseAndBatchAreTheMacs()
    {
        JsonElement climb = MapProbe.Get("climb");
        for (int i = 0; i < climb.GetArrayLength(); i++)
        {
            Near(climb[i].GetDouble(), MapTransferRun.Climb(i * 0.05), $"climb {i}");
        }

        JsonElement ease = MapProbe.Get("ease3");
        for (int i = 0; i < ease.GetArrayLength(); i++)
        {
            Near(ease[i].GetDouble(), TransferDrawing.Ease(i * 0.1), $"ease {i}");
        }

        MapTransfer batch = MapTransfer.Batch(Three)!;
        JsonElement mac = MapProbe.Get("batch");
        Assert.Equal(mac[0].GetString(), batch.Id);
        Assert.Equal(mac[1].GetInt32(), batch.Count);
        Assert.Equal(mac[2].GetBoolean(), batch.MixedFolders);
        Assert.Equal(mac[3].GetBoolean(), batch.MixedTools);
    }

    [Fact]
    public void ThePulseRiseRimDockSmokeAndRocketAreTheMacs()
    {
        JsonElement pulse = MapProbe.Get("pulse");
        for (int i = 0; i < pulse.GetArrayLength(); i++)
        {
            (double r, double a) = TransferDrawing.PulseAt(i * 0.1);
            Near(pulse[i][0].GetDouble(), r, "pulse r");
            Near(pulse[i][1].GetDouble(), a, "pulse alpha");
        }

        double[] rises = { 0, 0.1, 0.5, 0.9, 1 };
        for (int i = 0; i < rises.Length; i++)
        {
            var orbs = TransferDrawing.Rise(new MapPoint(400, 344), new MapPoint(400, 160), rises[i]);
            for (int k = 0; k < orbs.Count; k++)
            {
                JsonElement m = MapProbe.Get("rise")[i][k];
                Near(m[2].GetDouble(), orbs[k].At.Y, "rise y");
                Near(m[3].GetDouble(), orbs[k].Opacity, "rise opacity");
                Near(m[4].GetDouble(), orbs[k].Size, "rise size");
            }
        }

        // Counter-clockwise from the base: up the right side first (MapToolPile.swift:276-292).
        double[] rims = { 0, 0.25, 0.5, 0.75 };
        for (int i = 0; i < rims.Length; i++)
        {
            MapPoint p = TransferDrawing.RimLight(new MapPoint(290, 380), rims[i]);
            Near(MapProbe.Get("rim")[i][0].GetDouble(), p.X, "rim x");
            Near(MapProbe.Get("rim")[i][1].GetDouble(), p.Y, "rim y");
        }

        double[] docks = { -0.1, 0, 0.55, 1.09, 1.1 };
        for (int i = 0; i < docks.Length; i++)
        {
            JsonElement m = MapProbe.Get("dock")[i];
            var ring = TransferDrawing.Dock(docks[i]);
            Assert.Equal(m.GetArrayLength() == 0, ring is null);
            if (ring is { } d)
            {
                Near(m[0].GetDouble(), d.Radius);
                Near(m[1].GetDouble(), d.Alpha);
            }
        }

        double[] smokes = { 0, 0.1, 0.5, 1 };
        for (int i = 0; i < smokes.Length; i++)
        {
            var puffs = TransferDrawing.Smoke(smokes[i]);
            JsonElement m = MapProbe.Get("smoke")[i];
            Assert.Equal(m.GetArrayLength(), puffs.Count);
            for (int k = 0; k < puffs.Count; k++)
            {
                Near(m[k][0].GetDouble(), puffs[k].Offset);
                Near(m[k][1].GetDouble(), puffs[k].Radius);
                Near(m[k][2].GetDouble(), puffs[k].Alpha);
            }
        }

        double?[] flights = { null, 0.1, 0.3, 0.6, 0.999, 1 };
        for (int i = 0; i < flights.Length; i++)
        {
            var rocket = TransferDrawing.Rocket(new MapPoint(417.6, 353), new MapPoint(417.6, 129.2), flights[i], 0.37);
            JsonElement m = MapProbe.Get("rocket")[i];
            Near(m[0].GetDouble(), rocket.At.X, $"rocket x {i}");
            Near(m[1].GetDouble(), rocket.At.Y, $"rocket y {i}");
            Near(m[2].GetDouble(), rocket.Angle, $"rocket angle {i}");
            Near(m[3].GetDouble(), rocket.Burn ?? -1, $"rocket burn {i}");
        }
    }

    [Fact]
    public void ThePlayerPlaysOneRunAtATimeAndHoldsEachCardFiveSecondsPastItsEnd()
    {
        var player = new MapTransferPlayer();
        player.Enqueue(Three.Take(2), 10);
        MapTransferRun first = player.Run!;
        Assert.Equal(2, first.Transfers.Count);
        Assert.Equal(10, first.Start);
        Assert.Equal("batch:a", Assert.Single(player.Cards).Id);

        // What arrives while a run plays waits for the next run.
        player.Enqueue(new[] { new MapTransfer("k", "p3", MapTools.Codex, TransferAction.Keep) }, 11);
        Assert.Same(first, player.Run);

        // A replayed id is never played again.
        player.Enqueue(Three.Take(1), 11.5);
        Assert.Same(first, player.Run);

        player.Tick(10 + first.End - 0.01);
        Assert.Same(first, player.Run);
        player.Tick(10 + first.End);
        Assert.Equal("k", player.Run!.Transfers[0].Id);
        Assert.Equal(new[] { "batch:a", "k" }, player.Cards.Select(c => c.Id));

        // The batch card leaves Hold seconds after its run ended.
        player.Tick(10 + first.End + MapTransferPlayer.Hold - 0.01);
        Assert.Contains(player.Cards, c => c.Id == "batch:a");
        player.Tick(10 + first.End + MapTransferPlayer.Hold);
        Assert.DoesNotContain(player.Cards, c => c.Id == "batch:a");
    }

    [Fact]
    public void ADismissalOnlyShowsItsCardAndTheStackKeepsSix()
    {
        var player = new MapTransferPlayer();
        player.Enqueue(new[] { new MapTransfer("d0", "p", MapTools.Codex, TransferAction.Dismiss) }, 0);
        Assert.Null(player.Run);
        Assert.Empty(player.Cards);
        for (int i = 0; i < 8; i++)
        {
            player.Announce(new MapTransfer("d" + i, "p", MapTools.Codex, TransferAction.Dismiss), i);
        }

        Assert.Null(player.Run);
        Assert.Equal(new[] { "d2", "d3", "d4", "d5", "d6", "d7" }, player.Cards.Select(c => c.Id));
        player.Close("d5");
        Assert.Equal(new[] { "d2", "d3", "d4", "d6", "d7" }, player.Cards.Select(c => c.Id));
        Assert.Equal(7, player.NextDeadline);
    }

    [Fact]
    public void AContributionIsFoundWhereItLandsInHistory()
    {
        var old = new List<MapHistoryRow> { new("s1", DateTimeOffset.Parse("2026-11-01T10:00:00Z"), "p1", "api", MapTools.Codex, "submitted") };
        var fresh = old.Concat(new[]
        {
            new MapHistoryRow("s3", DateTimeOffset.Parse("2026-11-01T12:00:00Z"), "p2", "web", MapTools.ClaudeCode, "submitted"),
            new MapHistoryRow("s2", DateTimeOffset.Parse("2026-11-01T11:00:00Z"), "p1", "api", MapTools.Codex, "submitted"),
            new MapHistoryRow("s0", DateTimeOffset.Parse("2026-10-30T10:00:00Z"), "p1", "api", MapTools.Codex, "submitted"),
        }).ToList();
        // MapTransfers.swift:58-72: new, sent after everything seen, oldest first.
        Assert.Equal(new[] { "s2", "s3" }, MapTransferDetector.Contributions(old, fresh).Select(t => t.Id));
        Assert.Empty(MapTransferDetector.Contributions(null, fresh));
    }
}
