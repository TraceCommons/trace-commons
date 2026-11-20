using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop.Map;

/// <summary>
/// One read of everything the Traces tree and the two map views are drawn
/// from, each read on its own: a failed read is null and draws as unread,
/// never as empty (TracesStore.swift:700-730, InferenceStore.swift:210-225,
/// MonitorWindowView.swift:1053-1085).
/// </summary>
public sealed record MapSnapshot(
    IReadOnlyList<MapEntry>? Entries,
    IReadOnlyList<MapProject>? Projects,
    IReadOnlyDictionary<string, string?>? SourceModes,
    MapStatus? Status,
    MapDestinations? Destinations,
    IReadOnlyList<MapHistoryRow>? History,
    IReadOnlyList<HarnessRow>? Harnesses)
{
    /// <summary>
    /// The Traces tree. The Mac reads the queue and the projects together and
    /// shows nothing current when either fails (TracesStore.swift:711-717);
    /// settings alone may be unread, which draws each tool as unknown.
    /// </summary>
    public TracesTree Tree(IReadOnlySet<string> scansWhenUnset, bool showsIgnored = false) =>
        Entries is null || Projects is null
            ? TracesTree.Empty
            : TracesTree.Build(Entries, Projects, SourceModes, scansWhenUnset, showsIgnored);

    /// <summary>Whether the tree's reads came back (the Mac's <c>ScreenState.ready</c>).</summary>
    public bool Ready => Entries is not null && Projects is not null;

    public CommonsGate Gate => new(Ready, Status, Destinations);

    public MapContributions Contributions => MapContributions.From(History);
}

/// <summary>Reads a <see cref="MapSnapshot"/> over <c>tc_call</c>.</summary>
public static class MapFeed
{
    /// <summary>The reads, by name: each one a fake or the sample client must answer or refuse.</summary>
    public static readonly IReadOnlyList<string> Methods = new[]
    {
        DaemonProtocol.Methods.ListPending,
        DaemonProtocol.Methods.ListProjects,
        DaemonProtocol.Methods.GetSettings,
        DaemonProtocol.Methods.Status,
        ToolDestinations,
        DaemonProtocol.Methods.ListHistory,
        DaemonProtocol.Methods.HarnessList,
    };

    /// <summary><c>tool_destinations</c>: where each tool's traces and calls go.</summary>
    public const string ToolDestinations = "tool_destinations";

    /// <summary>
    /// Reads everything through <paramref name="call"/> (method, params JSON
    /// to a response frame), each read independently.
    /// </summary>
    public static Task<MapSnapshot> ReadAsync(Func<string, string, Task<string>> call)
    {
        ArgumentNullException.ThrowIfNull(call);
        return ReadAsync(async (method, parameters) => DaemonResponse.Parse(await call(method, parameters).ConfigureAwait(false)));
    }

    /// <summary>The same, over a call that answers a parsed response (the app's <c>DaemonHost.CallAsync</c>).</summary>
    public static async Task<MapSnapshot> ReadAsync(Func<string, string, Task<DaemonResponse>> call)
    {
        ArgumentNullException.ThrowIfNull(call);
        async Task<string?> Result(string method, string parameters = "{}")
        {
            DaemonResponse response;
            try
            {
                response = await call(method, parameters).ConfigureAwait(false);
            }
            catch (Exception e) when (e is not OutOfMemoryException)
            {
                return null;
            }

            return response.IsError || response.Result is not { } result ? null : result.GetRawText();
        }

        Task<string?> pending = Result(DaemonProtocol.Methods.ListPending);
        Task<string?> projects = Result(DaemonProtocol.Methods.ListProjects);
        Task<string?> settings = Result(DaemonProtocol.Methods.GetSettings);
        Task<string?> status = Result(DaemonProtocol.Methods.Status);
        Task<string?> destinations = Result(ToolDestinations);
        Task<string?> history = Result(DaemonProtocol.Methods.ListHistory, "{\"limit\":" + MapContributions.HistoryLimit + "}");
        Task<string?> harnesses = Result(DaemonProtocol.Methods.HarnessList);
        await Task.WhenAll(pending, projects, settings, status, destinations, history, harnesses).ConfigureAwait(false);
        string? harnessJson = harnesses.Result;
        return new MapSnapshot(
            MapReads.Pending(pending.Result),
            MapReads.Projects(projects.Result),
            MapReads.SourceModes(settings.Result),
            MapReads.Status(status.Result),
            MapReads.Destinations(destinations.Result),
            MapReads.History(history.Result),
            harnessJson is null ? null : HarnessSurface.ParseListing(harnessJson).Harnesses);
    }

    /// <summary>
    /// The tools the core reads from their usual folder while unset
    /// (<c>unset_scans_conventional</c> in <c>tc_source_settings_copy</c>,
    /// TracesStore.swift:638-641). Empty when the copy is unavailable.
    /// </summary>
    public static IReadOnlySet<string> ScansWhenUnset(string? sourceSettingsCopy)
    {
        var tools = new HashSet<string>(StringComparer.Ordinal);
        if (string.IsNullOrEmpty(sourceSettingsCopy))
        {
            return tools;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(sourceSettingsCopy);
            if (doc.RootElement.ValueKind == JsonValueKind.Object
                && doc.RootElement.TryGetProperty("tools", out JsonElement all) && all.ValueKind == JsonValueKind.Object)
            {
                foreach (string kind in MapTools.All)
                {
                    if (all.TryGetProperty(kind, out JsonElement tool) && tool.ValueKind == JsonValueKind.Object
                        && tool.TryGetProperty("unset_scans_conventional", out JsonElement scans) && scans.ValueKind == JsonValueKind.True)
                    {
                        tools.Add(kind);
                    }
                }
            }
        }
        catch (JsonException)
        {
            tools.Clear();
        }

        return tools;
    }

    private static readonly Lazy<IReadOnlySet<string>> CoreScans =
        new(() => ScansWhenUnset(NativeMethods.TakeOwnedString(NativeMethods.tc_source_settings_copy())));

    /// <summary><see cref="ScansWhenUnset(string?)"/> from the core, read once.</summary>
    public static IReadOnlySet<string> CoreScansWhenUnset => CoreScans.Value;
}
