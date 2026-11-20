using System;
using System.IO;
using System.Text.Json;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Mac's numbers for the map slices (W3, W4, W6), printed by a throwaway
/// SwiftPM probe that compiles macos/Sources at 841d81ba5; the fixture's
/// <c>_source</c> key records what it compiled. Every expectation the map
/// tests read comes from here, never from the C# under test.
/// </summary>
internal static class MapProbe
{
    private static readonly Lazy<JsonElement> Root = new(() =>
    {
        using JsonDocument doc = JsonDocument.Parse(File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "Fixtures", "Map", "mac-map-probe.json")));
        return doc.RootElement.Clone();
    });

    public static JsonElement Get(string key) => Root.Value.GetProperty(key);

    public static JsonElement Set(string set) => Get("sets").GetProperty(set);
}
