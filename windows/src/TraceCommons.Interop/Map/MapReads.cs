using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop.Map;

/// <summary>
/// The daemon reads the Traces tree and the map are built from, decoded to
/// just the fields they use, with the Mac's optionality: a field the reply
/// leaves out is null, never a default that would read as an answer
/// (<c>DaemonData</c> in macos/Sources/TCShellCore/DataContract/DaemonDataModels.swift).
/// Every decoder answers null for anything but the shape it expects, so a
/// failed or malformed read is unread, not empty.
/// </summary>
public static class MapReads
{
    /// <summary><c>list_pending</c>'s rows.</summary>
    public static IReadOnlyList<MapEntry>? Pending(string? json) =>
        Array(json, "pending", e => new MapEntry(
            Str(e, "entry_id") ?? string.Empty,
            Str(e, "source") ?? string.Empty,
            Str(e, "declared_source"),
            Str(e, "project_id") ?? string.Empty,
            Str(e, "project_label") ?? string.Empty,
            Date(e, "started_at"),
            Date(e, "discovered_at"),
            Str(e, "title")));

    /// <summary><c>list_projects</c>'s rows (ProjectRow.swift:105-225).</summary>
    public static IReadOnlyList<MapProject>? Projects(string? json) =>
        Array(json, "projects", e =>
        {
            List<string>? offerable = null;
            if (e.TryGetProperty("offerable_modes", out JsonElement modes) && modes.ValueKind == JsonValueKind.Array)
            {
                offerable = new List<string>();
                foreach (JsonElement m in modes.EnumerateArray())
                {
                    if (m.ValueKind == JsonValueKind.String && m.GetString() is { } mode)
                    {
                        offerable.Add(mode);
                    }
                }
            }

            return new MapProject(
                Str(e, "project_id") ?? string.Empty,
                Str(e, "project_label") ?? string.Empty,
                Str(e, "mode"),
                Bool(e, "is_unresolved_bucket") ?? false,
                Int(e, "pending_count"),
                Int(e, "contributable_count"),
                Str(e, "project_path"),
                Str(e, "automatic_disclosure"),
                offerable);
        });

    /// <summary>
    /// <c>get_settings</c>'s <c>*_source_mode</c> keys, by the tool's adapter
    /// name (TracesTree.swift:323-333: the keys name the tool, not the
    /// adapter). Null when the reply is not an object.
    /// </summary>
    public static IReadOnlyDictionary<string, string?>? SourceModes(string? json)
    {
        JsonElement? root = Root(json);
        if (root is not { ValueKind: JsonValueKind.Object } r)
        {
            return null;
        }

        var modes = new Dictionary<string, string?>(StringComparer.Ordinal);
        foreach ((string kind, string key) in SettingsKeys)
        {
            modes[kind] = Str(r, key + "_source_mode");
        }

        return modes;
    }

    /// <summary>The settings key each tool's source mode is under (TracesTree.swift:323-333).</summary>
    public static readonly IReadOnlyList<(string Kind, string Key)> SettingsKeys = new[]
    {
        (MapTools.ClaudeCode, "claude"),
        (MapTools.Codex, "codex"),
        (MapTools.GeminiCli, "gemini"),
        (MapTools.Cline, "cline"),
        (MapTools.OpenCode, "opencode"),
    };

    /// <summary><c>status</c>'s fields the commons gate and the Private AI lamp read.</summary>
    public static MapStatus? Status(string? json)
    {
        JsonElement? root = Root(json);
        if (root is not { ValueKind: JsonValueKind.Object } r)
        {
            return null;
        }

        int? held = null;
        if (r.TryGetProperty("automatic_contribution_held", out JsonElement h) && h.ValueKind == JsonValueKind.Object)
        {
            held = Int(h, "held_sessions");
        }

        int? voids = null;
        if (r.TryGetProperty("grant_voids", out JsonElement v) && v.ValueKind == JsonValueKind.Array)
        {
            voids = v.GetArrayLength();
        }

        string? privateState = null;
        if (r.TryGetProperty("private_inference_state", out JsonElement p) && p.ValueKind == JsonValueKind.Object)
        {
            privateState = Str(p, "state");
        }

        return new MapStatus(Bool(r, "paused"), Bool(r, "logged_in"), held, voids, privateState);
    }

    /// <summary><c>tool_destinations</c> (DaemonDataModels.swift:1528-1640).</summary>
    public static MapDestinations? Destinations(string? json)
    {
        JsonElement? root = Root(json);
        if (root is not { ValueKind: JsonValueKind.Object } r
            || !r.TryGetProperty("tools", out JsonElement tools) || tools.ValueKind != JsonValueKind.Array)
        {
            return null;
        }

        var rows = new List<MapToolDestination>();
        foreach (JsonElement t in tools.EnumerateArray())
        {
            if (t.ValueKind != JsonValueKind.Object)
            {
                continue;
            }

            string? watch = null;
            var to = new List<string>();
            if (t.TryGetProperty("sessions", out JsonElement s) && s.ValueKind == JsonValueKind.Object)
            {
                watch = Str(s, "watch");
                if (s.TryGetProperty("to", out JsonElement dest) && dest.ValueKind == JsonValueKind.Array)
                {
                    foreach (JsonElement d in dest.EnumerateArray())
                    {
                        if (d.GetString() is { } where)
                        {
                            to.Add(where);
                        }
                    }
                }
            }

            int? sessions = null;
            if (t.TryGetProperty("counts", out JsonElement c) && c.ValueKind == JsonValueKind.Object)
            {
                sessions = Int(c, "sessions");
            }

            rows.Add(new MapToolDestination(Str(t, "tool") ?? string.Empty, Str(t, "name"), watch, to, sessions, Bool(t, "installed")));
        }

        List<MapManagedSession>? managed = null;
        if (r.TryGetProperty("managed_sessions", out JsonElement m) && m.ValueKind == JsonValueKind.Array)
        {
            managed = new List<MapManagedSession>();
            foreach (JsonElement e in m.EnumerateArray())
            {
                if (e.ValueKind == JsonValueKind.Object)
                {
                    managed.Add(new MapManagedSession(
                        Str(e, "id") ?? string.Empty, Str(e, "tool") ?? string.Empty, Str(e, "provider") ?? string.Empty,
                        Str(e, "model"), Str(e, "collection_state") ?? string.Empty, Str(e, "state") ?? string.Empty));
                }
            }
        }

        Dictionary<string, string>? copy = null;
        if (r.TryGetProperty("managed_copy", out JsonElement mc) && mc.ValueKind == JsonValueKind.Object)
        {
            copy = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (JsonProperty field in mc.EnumerateObject())
            {
                if (field.Value.GetString() is { } word)
                {
                    copy[field.Name] = word;
                }
            }
        }

        return new MapDestinations(Str(r, "private_ai"), Str(r, "sessions_route"), rows, managed, copy);
    }

    /// <summary><c>list_history</c>'s rows.</summary>
    public static IReadOnlyList<MapHistoryRow>? History(string? json) =>
        Array(json, "history", e => new MapHistoryRow(
            Str(e, "submission_id") ?? string.Empty,
            Date(e, "submitted_at"),
            Str(e, "project_id"),
            Str(e, "project_label"),
            Str(e, "source"),
            Str(e, "status")));

    private static IReadOnlyList<T>? Array<T>(string? json, string key, Func<JsonElement, T> row)
    {
        JsonElement? root = Root(json);
        if (root is not { ValueKind: JsonValueKind.Object } r
            || !r.TryGetProperty(key, out JsonElement list) || list.ValueKind != JsonValueKind.Array)
        {
            return null;
        }

        var rows = new List<T>();
        foreach (JsonElement e in list.EnumerateArray())
        {
            if (e.ValueKind == JsonValueKind.Object)
            {
                rows.Add(row(e));
            }
        }

        return rows;
    }

    private static JsonElement? Root(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            return doc.RootElement.Clone();
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static string? Str(JsonElement e, string key) =>
        e.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    private static bool? Bool(JsonElement e, string key) =>
        e.TryGetProperty(key, out JsonElement v) && v.ValueKind is JsonValueKind.True or JsonValueKind.False ? v.GetBoolean() : null;

    private static int? Int(JsonElement e, string key) =>
        e.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.Number && v.TryGetInt32(out int n) ? n : null;

    private static DateTimeOffset? Date(JsonElement e, string key) =>
        Str(e, key) is { } text
        && DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.AssumeUniversal, out DateTimeOffset at)
            ? at
            : null;
}

/// <summary>One waiting trace, as the tree reads it.</summary>
public sealed record MapEntry(
    string EntryId, string Source, string? DeclaredSource, string ProjectId, string ProjectLabel,
    DateTimeOffset? StartedAt, DateTimeOffset? DiscoveredAt, string? Title = null);

/// <summary>One <c>list_projects</c> row, as the tree reads it.</summary>
public sealed record MapProject(
    string ProjectId, string ProjectLabel, string? Mode, bool IsUnresolvedBucket, int? PendingCount,
    int? ContributableCount, string? ProjectPath, string? AutomaticDisclosure, IReadOnlyList<string>? DaemonOfferableModes)
{
    /// <summary>ProjectRow.swift:272-274: the bucket's shared name, never its slug.</summary>
    public string DisplayLabel => IsUnresolvedBucket ? UnresolvedBucketCopy.Label : ProjectLabel;

    /// <summary>ProjectRow.swift:261-265.</summary>
    public IReadOnlyList<string> OfferableModes
    {
        get
        {
            string[] supported = IsUnresolvedBucket
                ? new[] { MapModes.Ask, MapModes.Ignore }
                : new[] { MapModes.Ask, MapModes.AutoUpload, MapModes.Ignore };
            if (DaemonOfferableModes is null)
            {
                return supported;
            }

            var offered = new List<string>();
            foreach (string mode in supported)
            {
                if (DaemonOfferableModes.Contains(mode))
                {
                    offered.Add(mode);
                }
            }

            return offered;
        }
    }

    /// <summary>The mode as the Mac decodes it (ProjectMode.swift:10-17): an unknown value is no mode.</summary>
    public string? KnownMode => MapModes.Known(Mode);
}

/// <summary>The folder modes on the wire (ProjectRow.swift:10-17).</summary>
public static class MapModes
{
    public const string Ask = "notify_only";
    public const string AutoUpload = "auto_upload";
    public const string Ignore = "ignore";

    /// <summary>A wire mode the Mac decodes, or null.</summary>
    public static string? Known(string? mode) => mode is Ask or AutoUpload or Ignore ? mode : null;
}

/// <summary>The status fields the map reads; null for each one the reply left out.</summary>
public sealed record MapStatus(bool? Paused, bool? LoggedIn, int? HeldSessions, int? GrantVoids, string? PrivateInferenceState);

/// <summary>One <c>tool_destinations</c> tool row.</summary>
public sealed record MapToolDestination(
    string Tool, string? Name, string? Watch, IReadOnlyList<string> SessionsTo, int? SessionCount, bool? Installed);

/// <summary>One managed session route (DaemonDataModels.swift:1561-1580).</summary>
public sealed record MapManagedSession(string Id, string Tool, string Provider, string? Model, string CollectionState, string State)
{
    /// <summary>The tool's adapter name: <c>claude</c> is Claude Code's.</summary>
    public string Source => Tool == "claude" ? MapTools.ClaudeCode : Tool;

    /// <summary>
    /// DaemonDataModels.swift:1574-1579: provider, model, state and collection
    /// in the core's words, the missing ones left out, joined by a spaced dot.
    /// </summary>
    public string Detail(IReadOnlyDictionary<string, string> copy)
    {
        var parts = new List<string>();
        if (copy.TryGetValue("provider_" + Provider, out string? provider))
        {
            parts.Add(provider);
        }

        if (Model is not null)
        {
            parts.Add(Model);
        }

        if (copy.TryGetValue(ManagedStateKey(State), out string? state))
        {
            parts.Add(state);
        }

        if (copy.TryGetValue("collection_" + CollectionState, out string? collection))
        {
            parts.Add(collection);
        }

        return string.Join(" · ", parts);
    }

    /// <summary>
    /// The copy key for a managed session's state (ManagedSessionsSurface.swift:307-313):
    /// a state this shell does not know is said as unknown.
    /// </summary>
    public static string ManagedStateKey(string state) =>
        "state_" + (state is "starting" or "running" or "exited" or "failed" or "unknown" ? state : "unknown");
}

/// <summary><c>tool_destinations</c>, as the map reads it.</summary>
public sealed record MapDestinations(
    string? PrivateAi, string? SessionsRoute, IReadOnlyList<MapToolDestination> Tools,
    IReadOnlyList<MapManagedSession>? ManagedSessions, IReadOnlyDictionary<string, string>? ManagedCopy);

/// <summary>One History row, as the map reads it.</summary>
public sealed record MapHistoryRow(
    string SubmissionId, DateTimeOffset? SubmittedAt, string? ProjectId, string? ProjectLabel, string? Source, string? Status);
