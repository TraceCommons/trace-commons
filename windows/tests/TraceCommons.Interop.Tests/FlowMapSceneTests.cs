using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop.Map;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Traces tree and both map views, built the way the window builds them
/// (<see cref="MapFeed.ReadAsync"/> over the recorded sample client, the
/// core's own words and scan list), against the Mac's scenes for the same
/// recorded sets (FlowMapScene.swift, TracesTree.swift at 841d81ba5, printed
/// by the probe). Positions to 1e-6.
/// </summary>
public class FlowMapSceneTests
{
    private const double Tolerance = 1e-6;

    /// <summary>The sets a running core answers; coreDown is tested on its own.</summary>
    public static IEnumerable<object[]> AnsweringSets() =>
        new[] { "empty", "normalDay", "busyQueue", "unknownCounts", "heldSessions", "armedFolder" }.Select(s => new object[] { s });

    private static readonly IReadOnlySet<string> ProbeScans = new HashSet<string> { MapTools.ClaudeCode, MapTools.Codex };

    internal static async Task<MapSnapshot> Read(string set)
    {
        var sample = new SampleDaemon(set, Path.Combine(AppContext.BaseDirectory, "RecordedSamples"));
        return await MapFeed.ReadAsync((method, parameters) => Task.FromResult(sample.Call(method, parameters)));
    }

    /// <summary>The probe's fixed harness verdicts (see the fixture's _source).</summary>
    internal static HarnessState ProbeState(HarnessRow row) => row.Id switch
    {
        "claude" or "claude-code" => HarnessState.Answering,
        "codex" => HarnessState.ActivityShared,
        "gemini" or "gemini-cli" => HarnessState.Unknown,
        _ => row.Installed ? HarnessState.ConnectedNoCalls : HarnessState.NotConnected,
    };

    [Fact]
    public void TheCoreScansClaudeCodeAndCodexWhenUnsetAsTheProbeAssumed()
    {
        // TracesStore.swift:638-641 reads this from the core; the probe ran
        // with the same two tools, so the scenes below test the same input.
        Assert.Equal(ProbeScans.OrderBy(s => s), MapFeed.CoreScansWhenUnset.OrderBy(s => s));
    }

    [Theory]
    [MemberData(nameof(AnsweringSets))]
    public async Task TheTreeIsTheMacs(string set)
    {
        MapSnapshot snapshot = await Read(set);
        JsonElement mac = MapProbe.Set(set);
        TracesTree tree = snapshot.Tree(MapFeed.CoreScansWhenUnset);
        JsonElement folders = mac.GetProperty("tree").GetProperty("folders");
        Assert.Equal(folders.GetArrayLength(), tree.Folders.Count);
        for (int i = 0; i < tree.Folders.Count; i++)
        {
            JsonElement f = folders[i];
            Assert.Equal(f.GetProperty("id").GetString(), tree.Folders[i].Id);
            Assert.Equal(f.GetProperty("label").GetString(), tree.Folders[i].Label);
            Assert.Equal(f.GetProperty("sessions").EnumerateArray().Select(e => e.GetString()), tree.Folders[i].Sessions.Select(s => s.EntryId));
        }

        JsonElement tools = mac.GetProperty("tree").GetProperty("tools");
        Assert.Equal(tools.EnumerateArray().Select(t => t.GetProperty("id").GetString()), tree.Tools.Select(t => t.Id));
        Assert.Equal(tools.EnumerateArray().Select(t => t.GetProperty("waiting").GetInt32()), tree.Tools.Select(t => t.Waiting));
        Assert.Equal(tools.EnumerateArray().Select(t => t.GetProperty("mode").GetString()), tree.Tools.Select(t => Mac(t.Mode)));
        Assert.Equal(mac.GetProperty("tree").GetProperty("unplaced").EnumerateArray().Select(e => e.GetString()), tree.Unplaced.Select(f => f.Id));
        Assert.Equal(mac.GetProperty("tree").GetProperty("hiddenIgnored").GetInt32(), tree.HiddenIgnored);
        Assert.Equal(
            mac.GetProperty("treeShowsIgnored").EnumerateArray().Select(e => e.GetString()),
            snapshot.Tree(MapFeed.CoreScansWhenUnset, showsIgnored: true).Folders.Select(f => f.Id));
        Assert.Equal(
            mac.GetProperty("treeKeepsOrder").EnumerateArray().Select(e => e.GetString()),
            TracesTree.Build(snapshot.Entries!, snapshot.Projects!, snapshot.SourceModes, MapFeed.CoreScansWhenUnset, keepsOrder: true).Folders.Select(f => f.Id));
        Assert.Equal(
            mac.GetProperty("treeMatching_a").EnumerateArray().Select(e => e[0].GetString() + ":" + e[1].GetInt32()),
            tree.Matching("a").Folders.Select(f => f.Id + ":" + f.Sessions.Count));
    }

    [Theory]
    [MemberData(nameof(AnsweringSets))]
    public async Task TheGateAndTheCountsAreTheMacs(string set)
    {
        MapSnapshot snapshot = await Read(set);
        JsonElement mac = MapProbe.Set(set);
        Assert.Equal(mac.GetProperty("gateOpen").GetBoolean(), snapshot.Gate.Open);
        JsonElement total = mac.GetProperty("contributed");
        Assert.Equal(total.ValueKind == JsonValueKind.Null ? null : total.GetInt32(), snapshot.Contributions.Total);
    }

    [Theory]
    [MemberData(nameof(AnsweringSets))]
    public async Task TheTracesMapIsTheMacs(string set)
    {
        MapSnapshot snapshot = await Read(set);
        JsonElement mac = MapProbe.Set(set);
        TracesTree tree = snapshot.Tree(MapFeed.CoreScansWhenUnset);
        FlowMapScene scene = FlowMapScene.Traces(tree, snapshot.Gate, FlowMapCopy.Table, snapshot.Contributions);
        AssertScene(mac.GetProperty("traces"), scene);

        // The pane-shaped space the window uses for a 440 by 614 map.
        FlowMapScene pane = FlowMapScene.Traces(tree, snapshot.Gate, FlowMapCopy.Table, snapshot.Contributions,
            space: FlowMapScene.SpaceFor(new MapSize(440, 614)));
        AssertScene(mac.GetProperty("tracesPane"), pane);

        if (mac.TryGetProperty("tracesSelected", out JsonElement selected))
        {
            AssertScene(selected, FlowMapScene.Traces(tree, snapshot.Gate, FlowMapCopy.Table, snapshot.Contributions, selectedTool: tree.Tools[0].Id));
        }

        var devices = new[] { new MapOtherDevice("d1", "Desk", "seen"), new MapOtherDevice("d2", "Lap", "seen", 3) };
        AssertScene(mac.GetProperty("devices"), scene.AddingDevices(devices, offersLink: true));
    }

    [Theory]
    [MemberData(nameof(AnsweringSets))]
    public async Task ThePrivateAIMapIsTheMacs(string set)
    {
        MapSnapshot snapshot = await Read(set);
        FlowMapScene scene = FlowMapScene.PrivateAI(
            snapshot.Harnesses, "DEST", snapshot.Destinations?.PrivateAi ?? snapshot.Status?.PrivateInferenceState,
            row => "s:" + row.Id, ProbeState, FlowMapCopy.Table, snapshot.Destinations)
            .AddingManagedRoutes(snapshot.Destinations, FlowMapCopy.Table);
        AssertScene(MapProbe.Set(set).GetProperty("privateAI"), scene);
    }

    [Fact]
    public void TheGateClosesWhereverTheMacsDoes()
    {
        // FlowMapScene.swift:348-355 over the recorded normalDay status and
        // routes with one field changed each (probe "gates"): held traces,
        // a pause, a sign-out, a voided grant, a missing field, a route
        // with nowhere to send, and a screen that is not ready.
        JsonElement gates = MapProbe.Get("gates");
        string routes = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "RecordedSamples", "normalDay", "tool_destinations.json"));
        foreach (JsonProperty status in MapProbe.Get("gateStatus").EnumerateObject())
        {
            var gate = new CommonsGate(true, MapReads.Status(status.Value.GetString()), MapReads.Destinations(routes));
            Assert.True(gates.GetProperty(status.Name).GetBoolean() == gate.Open, status.Name);
        }

        string recorded = MapProbe.Get("gateStatus").GetProperty("asRecorded").GetString()!;
        foreach (JsonProperty route in MapProbe.Get("gateRoutes").EnumerateObject())
        {
            var gate = new CommonsGate(true, MapReads.Status(recorded), MapReads.Destinations(route.Value.GetString()));
            Assert.True(gates.GetProperty("route_" + route.Name).GetBoolean() == gate.Open, route.Name);
        }

        Assert.Equal(gates.GetProperty("notReady").GetBoolean(),
            new CommonsGate(false, MapReads.Status(recorded), MapReads.Destinations(routes)).Open);
    }

    [Fact]
    public async Task ACoreThatIsDownDrawsNothingAsCurrent()
    {
        // TracesStore.swift:21 and :736-741: a failed first read leaves the
        // empty tree, no status and no destinations; nothing flows.
        MapSnapshot snapshot = await Read(SampleDaemon.CoreDown);
        Assert.False(snapshot.Ready);
        Assert.Empty(snapshot.Tree(MapFeed.CoreScansWhenUnset).Folders);
        Assert.False(snapshot.Gate.Open);
        Assert.Null(snapshot.Contributions.Total);
        Assert.False(FlowMapScene.PrivateAI(snapshot.Harnesses, "DEST", null, _ => null, ProbeState, FlowMapCopy.Table).DrawsMap);
    }

    [Fact]
    public async Task AColumnPastTenFoldersEndsInItsPillAndAClickRevealsSix()
    {
        // Synthetic queue of 25 folders (probe "synthetic*"): FlowMapScene.swift:247-254, :412-416, :448-453.
        var sample = new SampleDaemon("normalDay", Path.Combine(AppContext.BaseDirectory, "RecordedSamples"));
        MapSnapshot recorded = await MapFeed.ReadAsync((m, p) => Task.FromResult(sample.Call(m, p)));
        var entries = Enumerable.Range(0, 25).Select(i => new MapEntry(
            "e" + i, i % 3 == 0 ? MapTools.Codex : MapTools.ClaudeCode, null, "p" + i, $"Folder number {i} long name",
            DateTimeOffset.Parse($"2026-11-0{1 + (i % 9)}T1{i % 10}:00:00Z", System.Globalization.CultureInfo.InvariantCulture),
            DateTimeOffset.Parse($"2026-11-0{1 + (i % 9)}T10:00:00Z", System.Globalization.CultureInfo.InvariantCulture))).ToList();
        TracesTree tree = TracesTree.Build(entries, Array.Empty<MapProject>(), recorded.SourceModes, ProbeScans);
        Assert.Equal(MapProbe.Get("syntheticTreeOrder").EnumerateArray().Select(e => e.GetString()), tree.Folders.Select(f => f.Id));
        var gate = new CommonsGate(true, recorded.Status, recorded.Destinations);
        AssertScene(MapProbe.Get("synthetic"), FlowMapScene.Traces(tree, gate, FlowMapCopy.Table));
        AssertScene(MapProbe.Get("syntheticRevealed"), FlowMapScene.Traces(tree, gate, FlowMapCopy.Table,
            revealed: new Dictionary<string, int> { ["folders"] = 12 }));
        AssertScene(MapProbe.Get("syntheticRevealedAll"), FlowMapScene.Traces(tree, gate, FlowMapCopy.Table,
            revealed: new Dictionary<string, int> { ["folders"] = 18 }));
    }

    [Fact]
    public void TheLayoutHelpersAreTheMacs()
    {
        double[][] panes = { new[] { 440.0, 614 }, new[] { 300.0, 900 }, new[] { 1200.0, 400 }, new[] { 0.0, 0 } };
        JsonElement spaces = MapProbe.Get("space");
        for (int i = 0; i < panes.Length; i++)
        {
            MapSize s = FlowMapScene.SpaceFor(new MapSize(panes[i][0], panes[i][1]));
            Near(spaces[i][0].GetDouble(), s.Width);
            Near(spaces[i][1].GetDouble(), s.Height);
        }

        Assert.Equal(
            MapProbe.Get("initials").EnumerateArray().Select(e => e.GetString()),
            new[] { "Cursor", "GitHub Copilot CLI", "", "a", "x-y", "Claude Code" }.Select(FlowMapScene.Initials));
        var slots = FlowMapScene.StackDown(new[] { 1.0, 1, 1, 0.6 }, (100, 600), 34, 22);
        JsonElement mac = MapProbe.Get("stackFn");
        for (int i = 0; i < slots.Count; i++)
        {
            Near(mac[i][0].GetDouble(), slots[i].Y);
            Near(mac[i][1].GetDouble(), slots[i].Room);
        }

        Assert.Equal(MapProbe.Get("moving").EnumerateArray().Select(e => e.GetString()),
            TracesTree.Moving("c", "a", new[] { "a", "b", "c", "d" }, new[] { "x", "a" }));
    }

    internal static string Mac(SourceMode mode) => mode switch
    {
        SourceMode.Watch => "watch",
        SourceMode.Off => "off",
        SourceMode.Unset => "unset",
        _ => "unknown",
    };

    private static string MacMode(string? mode) => mode switch
    {
        MapModes.Ask => "ask",
        MapModes.AutoUpload => "autoUpload",
        MapModes.Ignore => "ignore",
        _ => "nil",
    };

    /// <summary>The probe's <c>kindTag</c> for a node.</summary>
    internal static string KindTag(MapNode n) => n.Kind switch
    {
        MapNodeKind.Hub => "hub",
        MapNodeKind.Library => "library:" + (n.Active ? "true" : "false"),
        MapNodeKind.Tool => $"tool:{n.Tool!.Initials}:{(n.Off ? "true" : "false")}",
        MapNodeKind.Folder => "folder:" + MacMode(n.Mode),
        MapNodeKind.Harness => $"harness:{n.Tool?.Initials ?? "nil"}:{(n.Installed ? "true" : "false")}",
        MapNodeKind.Destination => "destination:" + n.Lamp.ToString().ToLowerInvariant(),
        _ => "device",
    };

    internal static string Style(MapArcStyle style) => style.ToString().ToLowerInvariant();

    internal static void AssertScene(JsonElement mac, FlowMapScene scene)
    {
        Near(mac.GetProperty("space")[0].GetDouble(), scene.Space.Width);
        Near(mac.GetProperty("space")[1].GetDouble(), scene.Space.Height);
        Assert.Equal(mac.GetProperty("flows").GetBoolean(), scene.Flows);
        JsonElement nodes = mac.GetProperty("nodes");
        Assert.Equal(nodes.EnumerateArray().Select(n => n.GetProperty("id").GetString()), scene.Nodes.Select(n => n.Id));
        for (int i = 0; i < scene.Nodes.Count; i++)
        {
            JsonElement m = nodes[i];
            MapNode n = scene.Nodes[i];
            string where = n.Id;
            Assert.True(m.GetProperty("kind").GetString() == KindTag(n), $"{where}: kind {m.GetProperty("kind")} vs {KindTag(n)}");
            Near(m.GetProperty("at")[0].GetDouble(), n.At.X, where);
            Near(m.GetProperty("at")[1].GetDouble(), n.At.Y, where);
            Near(m.GetProperty("r").GetDouble(), n.Radius, where);
            Near(m.GetProperty("dim").GetDouble(), n.Dim, where);
            Assert.Equal(m.GetProperty("label").GetString(), n.Label);
            Assert.Equal(m.GetProperty("detail").GetString(), n.Detail);
            Assert.Equal(Str(m.GetProperty("title")), n.Title);
            Assert.Equal(Str(m.GetProperty("sublabel")), n.Sublabel);
            Assert.Equal(m.GetProperty("dashed").GetBoolean(), n.Dashed);
            Assert.Equal(m.GetProperty("ringed").GetBoolean(), n.Ringed);
            JsonElement waiting = m.GetProperty("waiting");
            Assert.Equal(waiting.ValueKind == JsonValueKind.Null ? null : waiting.GetInt32(), n.Waiting);
        }

        JsonElement arcs = mac.GetProperty("arcs");
        Assert.Equal(arcs.GetArrayLength(), scene.Arcs.Count);
        for (int i = 0; i < scene.Arcs.Count; i++)
        {
            JsonElement m = arcs[i];
            MapArc a = scene.Arcs[i];
            Assert.Equal(m.GetProperty("style").GetString(), Style(a.Style));
            Near(m.GetProperty("dim").GetDouble(), a.Dim);
            foreach ((string key, MapPoint p) in new[] { ("from", a.From), ("c1", a.Control1), ("c2", a.Control2), ("to", a.To) })
            {
                Near(m.GetProperty(key)[0].GetDouble(), p.X, $"arc {i} {key}");
                Near(m.GetProperty(key)[1].GetDouble(), p.Y, $"arc {i} {key}");
            }
        }

        JsonElement reveals = mac.GetProperty("reveals");
        Assert.Equal(reveals.GetArrayLength(), scene.Reveals.Count);
        for (int i = 0; i < scene.Reveals.Count; i++)
        {
            Assert.Equal(reveals[i].GetProperty("hidden").GetInt32(), scene.Reveals[i].Hidden);
            Assert.Equal(reveals[i].GetProperty("next").GetInt32(), scene.Reveals[i].Next);
            Near(reveals[i].GetProperty("at")[0].GetDouble(), scene.Reveals[i].At.X);
            Near(reveals[i].GetProperty("at")[1].GetDouble(), scene.Reveals[i].At.Y);
        }

        JsonElement overflows = mac.GetProperty("overflows");
        Assert.Equal(overflows.GetArrayLength(), scene.Overflows.Count);
        for (int i = 0; i < scene.Overflows.Count; i++)
        {
            Assert.Equal(overflows[i].GetProperty("count").GetInt32(), scene.Overflows[i].Count);
            Near(overflows[i].GetProperty("at")[1].GetDouble(), scene.Overflows[i].At.Y);
        }

        JsonElement stack = mac.GetProperty("stack");
        if (stack.ValueKind == JsonValueKind.Null)
        {
            Assert.Null(scene.Stack);
        }
        else
        {
            Assert.NotNull(scene.Stack);
            Assert.Equal(stack.GetProperty("tools").EnumerateArray().Select(e => e.GetString()), scene.Stack!.Tools);
            Near(stack.GetProperty("closed").GetDouble(), scene.Stack.Height(false));
            Near(stack.GetProperty("open").GetDouble(), scene.Stack.Height(true));
            for (int i = 0; i < scene.Stack.Tools.Count; i++)
            {
                Near(stack.GetProperty("coinsOpen")[i][1].GetDouble(), scene.Stack.CoinCenter(i, true).Y);
                Near(stack.GetProperty("coins")[i][1].GetDouble(), scene.Stack.CoinCenter(i, false).Y);
            }
        }

        JsonElement add = mac.GetProperty("addDevice");
        if (add.ValueKind == JsonValueKind.Null)
        {
            Assert.Null(scene.AddDevice);
        }
        else
        {
            Near(add[1].GetDouble(), scene.AddDevice!.Value.Y);
        }
    }

    private static string? Str(JsonElement e) => e.ValueKind == JsonValueKind.Null ? null : e.GetString();

    internal static void Near(double mac, double ours, string where = "")
    {
        Assert.True(Math.Abs(mac - ours) <= Tolerance, $"{where}: Mac {mac:R}, Windows {ours:R}");
    }
}
