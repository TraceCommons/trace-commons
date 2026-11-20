using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

// The daemon's insights_skills answer, decoded as the Mac decodes it
// (macos/Sources/TCShellCore/DataContract/SkillsModels.swift): a missing
// optional field takes the Mac's default, a missing required one makes the
// whole answer undecodable (null), and the fields the Mac reads with `try?`
// (decorative facts) are unknown when malformed and never fail the answer.

/// <summary>The whole snapshot (<c>DaemonData.SkillsSnapshot</c>).</summary>
public sealed record SkillsSnapshot(
    bool Enabled, bool Readable, bool Refreshing, string? Error,
    SkillsInventory Inventory, SkillsCatalog Catalog,
    IReadOnlyList<string> FollowedPublishers, IReadOnlyList<string> DefaultPublishers,
    IReadOnlyList<SkillsFeedEvent> HomeFeed, bool HomeFeedPartial,
    IReadOnlyList<SkillsUpdate> Updates, IReadOnlyList<SkillsFeedEvent> Feed, bool FeedPartial,
    SkillsUsage Usage, IReadOnlyDictionary<string, string> Copy, SkillsSuggestions Suggestions)
{
    /// <summary>An enabled, readable snapshot with nothing in it (the Mac's <c>.init(enabled: true)</c>).</summary>
    public static SkillsSnapshot Empty(bool enabled = false) => new(
        enabled, true, false, null, SkillsInventory.Empty, SkillsCatalog.Empty,
        Array.Empty<string>(), Array.Empty<string>(), Array.Empty<SkillsFeedEvent>(), false,
        Array.Empty<SkillsUpdate>(), Array.Empty<SkillsFeedEvent>(), false, SkillsUsage.Empty,
        new Dictionary<string, string>(), new SkillsSuggestions(false, false, 0, 0, null));

    public static SkillsSnapshot? Parse(JsonElement o)
    {
        if (o.ValueKind != JsonValueKind.Object) return null;
        try
        {
            return new SkillsSnapshot(
                W.OptBool(o, "enabled") ?? false,
                W.OptBool(o, "readable") ?? true,
                W.OptBool(o, "refreshing") ?? false,
                W.OptString(o, "error"),
                W.Has(o, "inventory") ? SkillsInventory.Parse(o.GetProperty("inventory")) : SkillsInventory.Empty,
                W.Has(o, "catalog") ? SkillsCatalog.Parse(o.GetProperty("catalog")) : SkillsCatalog.Empty,
                W.OptStrings(o, "followed_publishers"),
                W.OptStrings(o, "default_publishers"),
                W.OptList(o, "home_feed", SkillsFeedEvent.Parse),
                W.OptBool(o, "home_feed_partial") ?? false,
                W.OptList(o, "updates", SkillsUpdate.Parse),
                W.OptList(o, "feed", SkillsFeedEvent.Parse),
                W.OptBool(o, "feed_partial") ?? false,
                W.Has(o, "usage") ? SkillsUsage.Parse(o.GetProperty("usage")) : SkillsUsage.Empty,
                W.OptStringMap(o, "copy"),
                W.Lenient(o, "suggestions", SkillsSuggestions.Parse) ?? new SkillsSuggestions(false, false, 0, 0, null));
        }
        catch (SkillsWireException)
        {
            return null;
        }
    }
}

public sealed record SkillsSuggestions(bool Available, bool OptedIn, int Pending, int SentTotal, string? LastSentAt)
{
    internal static SkillsSuggestions Parse(JsonElement o) => new(
        W.OptBool(o, "available") ?? false, W.OptBool(o, "opted_in") ?? false,
        W.OptInt(o, "pending") ?? 0, W.OptInt(o, "sent_total") ?? 0, W.OptString(o, "last_sent_at"));
}

public sealed record SkillsInventory(
    IReadOnlyList<InstalledSkill> Skills, bool Partial, string? ScannedAt,
    IReadOnlyList<SkillsInventoryCoverage> Coverage, IReadOnlyDictionary<string, SkillInstallationDetails> Details)
{
    public static readonly SkillsInventory Empty = new(Array.Empty<InstalledSkill>(), true, null,
        Array.Empty<SkillsInventoryCoverage>(), new Dictionary<string, SkillInstallationDetails>());

    internal static SkillsInventory Parse(JsonElement o)
    {
        W.RequireObject(o);
        var details = new Dictionary<string, SkillInstallationDetails>(StringComparer.Ordinal);
        if (W.Has(o, "details"))
        {
            JsonElement map = o.GetProperty("details");
            if (map.ValueKind != JsonValueKind.Object) throw new SkillsWireException();
            foreach (JsonProperty p in map.EnumerateObject()) details[p.Name] = SkillInstallationDetails.Parse(p.Value);
        }

        return new SkillsInventory(W.OptList(o, "skills", InstalledSkill.Parse), W.OptBool(o, "partial") ?? true,
            W.OptString(o, "scanned_at"), W.OptList(o, "coverage", SkillsInventoryCoverage.Parse), details);
    }
}

public sealed record InstalledSkill(
    string Id, string Name, string Description, string Agent, string Scope, string Location, string Sha256,
    SkillsSource? Source)
{
    internal static InstalledSkill Parse(JsonElement o) => new(
        W.Str(o, "id"), W.Str(o, "name"), W.Str(o, "description"), W.Str(o, "agent"), W.Str(o, "scope"),
        W.Str(o, "location"), W.Str(o, "sha256"), W.Has(o, "source") ? SkillsSource.Parse(o.GetProperty("source")) : null);
}

public sealed record SkillsSource(string Repository, string Path, string? Revision)
{
    internal static SkillsSource Parse(JsonElement o) =>
        new(W.Str(o, "repository"), W.Str(o, "path"), W.OptString(o, "revision"));
}

public sealed record SkillsInventoryCoverage(string Location, string Status)
{
    internal static SkillsInventoryCoverage Parse(JsonElement o) => new(W.Str(o, "location"), W.Str(o, "status"));
}

public sealed record SkillInstallationDetails(IReadOnlyList<string> Aliases, string Availability, string DefinitionStatus)
{
    internal static SkillInstallationDetails Parse(JsonElement o) =>
        new(W.Strings(o, "aliases"), W.Str(o, "availability"), W.Str(o, "definition_status"));
}

public sealed record SkillsCatalog(
    IReadOnlyList<SkillsPublisher> Publishers, IReadOnlyList<CatalogSkill> Skills, string? CheckedAt,
    IReadOnlyList<SkillsPublisherState> PublisherStates, bool Partial, string? Source, int? Revision,
    string? IssuedAt, string? ExpiresAt, int? TotalSkills, IReadOnlyList<SkillsActivePublisher> ActivePublishers)
{
    public static readonly SkillsCatalog Empty = new(Array.Empty<SkillsPublisher>(), Array.Empty<CatalogSkill>(), null,
        Array.Empty<SkillsPublisherState>(), false, null, null, null, null, null, Array.Empty<SkillsActivePublisher>());

    internal static SkillsCatalog Parse(JsonElement o)
    {
        W.RequireObject(o);
        return new SkillsCatalog(
            W.OptList(o, "publishers", SkillsPublisher.Parse), W.OptList(o, "skills", CatalogSkill.Parse),
            W.OptString(o, "checked_at"), W.OptList(o, "publisher_states", SkillsPublisherState.Parse),
            W.OptBool(o, "partial") ?? false, W.OptString(o, "source"), W.OptInt(o, "revision"),
            W.OptString(o, "issued_at"), W.OptString(o, "expires_at"), W.OptInt(o, "total_skills"),
            // Decorative: a malformed list is none and never fails the snapshot.
            W.Lenient(o, "active_publishers", e => W.List(e, SkillsActivePublisher.Parse)) ?? Array.Empty<SkillsActivePublisher>());
    }
}

public sealed record SkillsActivePublisher(string PublisherId, IReadOnlyList<string> SkillIds)
{
    internal static SkillsActivePublisher Parse(JsonElement o) => new(W.Str(o, "publisher_id"), W.Strings(o, "skill_ids"));
}

public sealed record SkillsPublisher(
    string Id, string Name, string Repository, string SkillsRoot, string InstructionsUrl, string? Url, string? SourceKind)
{
    internal static SkillsPublisher Parse(JsonElement o) => new(
        W.Str(o, "id"), W.Str(o, "name"), W.Str(o, "repository"), W.Str(o, "skills_root"), W.Str(o, "instructions_url"),
        W.OptString(o, "url"), W.OptString(o, "source_kind"));
}

public sealed record SkillsPublisherState(string PublisherId, string Status, string? CheckedAt, SkillsRepositorySignals? Signals)
{
    internal static SkillsPublisherState Parse(JsonElement o) => new(
        W.Str(o, "publisher_id"), W.Str(o, "status"), W.OptString(o, "checked_at"),
        W.Lenient(o, "signals", SkillsRepositorySignals.Parse));
}

/// <summary>Public repository signals; every field is unknown (null), never zero, when absent.</summary>
public sealed record SkillsRepositorySignals(
    int? Stars, string? PushedAt, string? RepositoryCheckedAt, int? Commits7d, int? Commits28d, string? ActivityCheckedAt)
{
    internal static SkillsRepositorySignals Parse(JsonElement o)
    {
        W.RequireObject(o);
        return new SkillsRepositorySignals(W.OptInt(o, "stars"), W.OptString(o, "pushed_at"),
            W.OptString(o, "repository_checked_at"), W.OptInt(o, "commits_7d"), W.OptInt(o, "commits_28d"),
            W.OptString(o, "activity_checked_at"));
    }
}

public sealed record CatalogSkill(
    string Id, string PublisherId, string Name, string Description, string Repository, string Path,
    string Revision, string Sha256, string SourceUrl, string InstructionsUrl,
    SkillsLicense? License = null, string? FirstSeenAt = null, string? LastChangedAt = null,
    SkillsChecks? Checks = null, int? ChecksVersion = null)
{
    internal static CatalogSkill Parse(JsonElement o) => new(
        W.Str(o, "id"), W.Str(o, "publisher_id"), W.Str(o, "name"), W.Str(o, "description"),
        W.Str(o, "repository"), W.Str(o, "path"), W.Str(o, "revision"), W.Str(o, "sha256"),
        W.Str(o, "source_url"), W.Str(o, "instructions_url"),
        W.Lenient(o, "license", SkillsLicense.Parse), W.Lenient(o, "first_seen_at", W.AsString),
        W.Lenient(o, "last_changed_at", W.AsString), W.Lenient(o, "checks", SkillsChecks.Parse),
        W.Lenient(o, "checks_version", e => (int?)W.AsInt(e)));
}

public sealed record SkillsLicense(string Status, string? Spdx)
{
    internal static SkillsLicense Parse(JsonElement o) => new(W.Str(o, "status"), W.OptString(o, "spdx"));
}

public sealed record SkillsChecks(
    string Frontmatter, int DocumentBytes, int DirectoryBytes, int DirectoryFiles, SkillsBundledFiles Bundled,
    IReadOnlyList<string> Patterns, bool Inactive)
{
    internal static SkillsChecks Parse(JsonElement o) => new(
        W.Str(o, "frontmatter"), W.Int(o, "document_bytes"), W.Int(o, "directory_bytes"), W.Int(o, "directory_files"),
        W.Req(o, "bundled", SkillsBundledFiles.Parse),
        W.Strings(o, "patterns"), W.Bool(o, "inactive"));
}

public sealed record SkillsBundledFiles(int Scripts, int Executables, int Binaries)
{
    internal static SkillsBundledFiles Parse(JsonElement o) =>
        new(W.Int(o, "scripts"), W.Int(o, "executables"), W.Int(o, "binaries"));
}

public sealed record SkillsUpdate(string InstalledId, string Status, string? CatalogSkillId)
{
    internal static SkillsUpdate Parse(JsonElement o) =>
        new(W.Str(o, "installed_id"), W.Str(o, "status"), W.OptString(o, "catalog_skill_id"));
}

public sealed record SkillsFeedEvent(string Id, string PublisherId, string SkillId, string Kind, string ObservedAt)
{
    internal static SkillsFeedEvent Parse(JsonElement o) => new(
        W.Str(o, "id"), W.Str(o, "publisher_id"), W.Str(o, "skill_id"), W.Str(o, "kind"), W.Str(o, "observed_at"));
}

public sealed record SkillsUsage(
    bool Enabled, string? LastAnalysisAt, string? EarliestEvidence, int SessionsPartial, int SessionsObserved,
    int SessionsUnknown, int? DelegatedSessionsObserved, int? DelegatedSessionsUnknown,
    int? UnclassifiedSessionsObserved, int? UnclassifiedSessionsUnknown, bool Partial,
    IReadOnlyList<SkillsUsageRow> Skills, IReadOnlyList<SkillsUsageCoverage> Coverage,
    IReadOnlyList<SkillsPartialAgent>? PartialAgents)
{
    public static readonly SkillsUsage Empty = new(false, null, null, 0, 0, 0, null, null, null, null, true,
        Array.Empty<SkillsUsageRow>(), Array.Empty<SkillsUsageCoverage>(), null);

    internal static SkillsUsage Parse(JsonElement o)
    {
        W.RequireObject(o);
        return new SkillsUsage(
            W.OptBool(o, "enabled") ?? false, W.OptString(o, "last_analysis_at"), W.OptString(o, "earliest_evidence"),
            W.OptInt(o, "sessions_partial") ?? 0, W.OptInt(o, "sessions_observed") ?? 0, W.OptInt(o, "sessions_unknown") ?? 0,
            W.OptInt(o, "delegated_sessions_observed"), W.OptInt(o, "delegated_sessions_unknown"),
            W.OptInt(o, "unclassified_sessions_observed"), W.OptInt(o, "unclassified_sessions_unknown"),
            W.OptBool(o, "partial") ?? true, W.OptList(o, "skills", SkillsUsageRow.Parse),
            W.OptList(o, "coverage", SkillsUsageCoverage.Parse),
            W.Has(o, "partial_agents") ? W.List(o.GetProperty("partial_agents"), SkillsPartialAgent.Parse) : null);
    }
}

public sealed record SkillsPartialAgent(string Agent, IReadOnlyList<string> Days, int Undated)
{
    internal static SkillsPartialAgent Parse(JsonElement o) =>
        new(W.Str(o, "agent"), W.OptStrings(o, "days"), W.OptInt(o, "undated") ?? 0);
}

public sealed record SkillsUsageCoverage(string Agent, string Status)
{
    internal static SkillsUsageCoverage Parse(JsonElement o) => new(W.Str(o, "agent"), W.Str(o, "status"));
}

public sealed record SkillsUsageWindow(int Days, int Invocations, int Loads, int Sessions)
{
    internal static SkillsUsageWindow Parse(JsonElement o) =>
        new(W.Int(o, "days"), W.Int(o, "invocations"), W.Int(o, "loads"), W.Int(o, "sessions"));
}

public sealed record SkillsUsageDay(string Day, int Invocations, int Loads, int Sessions)
{
    internal static SkillsUsageDay Parse(JsonElement o) =>
        new(W.Str(o, "day"), W.Int(o, "invocations"), W.Int(o, "loads"), W.Int(o, "sessions"));
}

public sealed record SkillsUsageEvidence(string SessionRef, string? ParentSessionRef)
{
    internal static SkillsUsageEvidence Parse(JsonElement o) =>
        new(W.Str(o, "session_ref"), W.OptString(o, "parent_session_ref"));
}

public sealed record SkillsUsageRow(
    string Name, string Agent, int Invocations, int Loads, int Sessions, string? LastSeen,
    IReadOnlyList<string> SessionRefs, IReadOnlyList<SkillsUsageWindow> Windows, IReadOnlyList<SkillsUsageDay> Daily,
    int UndatedEvents, string Population, IReadOnlyList<SkillsUsageEvidence> Evidence, IReadOnlyList<string> ParentSessionRefs)
{
    /// <summary>Agent, population and name: one row's identity (<c>identity</c>).</summary>
    public string Identity => Agent + "\0" + Population + "\0" + Name;

    internal static SkillsUsageRow Parse(JsonElement o) => new(
        W.Str(o, "name"), W.Str(o, "agent"), W.Int(o, "invocations"), W.Int(o, "loads"), W.Int(o, "sessions"),
        W.OptString(o, "last_seen"), W.OptStrings(o, "session_refs"), W.OptList(o, "windows", SkillsUsageWindow.Parse),
        W.OptList(o, "daily", SkillsUsageDay.Parse), W.OptInt(o, "undated_events") ?? 0,
        W.OptString(o, "population") ?? "direct", W.OptList(o, "evidence", SkillsUsageEvidence.Parse),
        W.OptStrings(o, "parent_session_refs"));
}

/// <summary><c>insights_skills</c> with a <c>skill_id</c>: one installed skill's definition.</summary>
public sealed record SkillDefinition(string SkillId, string Status, string? Definition)
{
    public static SkillDefinition? Parse(JsonElement o)
    {
        try { return new SkillDefinition(W.Str(o, "skill_id"), W.Str(o, "status"), W.OptString(o, "definition")); }
        catch (SkillsWireException) { return null; }
    }
}

/// <summary><c>insights_skills</c> with a <c>session_ref</c>: one session's evidence.</summary>
public sealed record SkillEvidence(
    bool Enabled, bool? Readable, SkillEvidenceSession? Session, string? Agent, string? Population,
    SkillEvidenceObservations? SkillUsage)
{
    public static SkillEvidence? Parse(JsonElement o)
    {
        try
        {
            return new SkillEvidence(W.Bool(o, "enabled"), W.OptBool(o, "readable"),
                W.Has(o, "session") ? SkillEvidenceSession.Parse(o.GetProperty("session")) : null,
                W.OptString(o, "agent"), W.OptString(o, "population"),
                W.Has(o, "skill_usage") ? SkillEvidenceObservations.Parse(o.GetProperty("skill_usage")) : null);
        }
        catch (SkillsWireException) { return null; }
    }
}

public sealed record SkillEvidenceSession(string? Source, string? Date, ulong? Turns, ulong? Tokens)
{
    internal static SkillEvidenceSession Parse(JsonElement o)
    {
        W.RequireObject(o);
        return new SkillEvidenceSession(W.OptString(o, "source"), W.OptString(o, "date"), W.OptULong(o, "turns"), W.OptULong(o, "tokens"));
    }
}

public sealed record SkillEvidenceCount(string Name, int Invocations, int Loads)
{
    internal static SkillEvidenceCount Parse(JsonElement o) => new(W.Str(o, "name"), W.Int(o, "invocations"), W.Int(o, "loads"));
}

public sealed record SkillEvidenceObservations(string Coverage, bool Partial, IReadOnlyList<SkillEvidenceCount> Skills)
{
    internal static SkillEvidenceObservations Parse(JsonElement o) =>
        new(W.Str(o, "coverage"), W.Bool(o, "partial"), W.ReqList(o, "skills", SkillEvidenceCount.Parse));
}

internal sealed class SkillsWireException : Exception
{
}

/// <summary>Decoding helpers: Swift's <c>decode</c> (required) and <c>decodeIfPresent</c> (optional; null reads as absent).</summary>
internal static class W
{
    public static bool Has(JsonElement o, string key) =>
        o.ValueKind == JsonValueKind.Object && o.TryGetProperty(key, out JsonElement v) && v.ValueKind != JsonValueKind.Null;

    public static void RequireObject(JsonElement o)
    {
        if (o.ValueKind != JsonValueKind.Object) throw new SkillsWireException();
    }

    private static JsonElement Required(JsonElement o, string key)
    {
        RequireObject(o);
        if (!o.TryGetProperty(key, out JsonElement v) || v.ValueKind == JsonValueKind.Null) throw new SkillsWireException();
        return v;
    }

    public static string AsString(JsonElement v) =>
        v.ValueKind == JsonValueKind.String ? v.GetString()! : throw new SkillsWireException();

    public static int AsInt(JsonElement v) =>
        v.ValueKind == JsonValueKind.Number && v.TryGetInt32(out int i) ? i : throw new SkillsWireException();

    private static bool AsBool(JsonElement v) =>
        v.ValueKind is JsonValueKind.True or JsonValueKind.False ? v.GetBoolean() : throw new SkillsWireException();

    public static string Str(JsonElement o, string key) => AsString(Required(o, key));
    public static int Int(JsonElement o, string key) => AsInt(Required(o, key));
    public static bool Bool(JsonElement o, string key) => AsBool(Required(o, key));
    public static string? OptString(JsonElement o, string key) => Has(o, key) ? AsString(o.GetProperty(key)) : null;
    public static int? OptInt(JsonElement o, string key) => Has(o, key) ? AsInt(o.GetProperty(key)) : null;
    public static bool? OptBool(JsonElement o, string key) => Has(o, key) ? AsBool(o.GetProperty(key)) : null;

    public static ulong? OptULong(JsonElement o, string key)
    {
        if (!Has(o, key)) return null;
        JsonElement v = o.GetProperty(key);
        return v.ValueKind == JsonValueKind.Number && v.TryGetUInt64(out ulong u) ? u : throw new SkillsWireException();
    }

    public static IReadOnlyList<T> List<T>(JsonElement v, Func<JsonElement, T> parse)
    {
        if (v.ValueKind != JsonValueKind.Array) throw new SkillsWireException();
        return v.EnumerateArray().Select(parse).ToArray();
    }

    public static IReadOnlyList<T> OptList<T>(JsonElement o, string key, Func<JsonElement, T> parse) =>
        Has(o, key) ? List(o.GetProperty(key), parse) : Array.Empty<T>();

    public static IReadOnlyList<string> Strings(JsonElement o, string key) => List(Required(o, key), AsString);

    public static IReadOnlyList<T> ReqList<T>(JsonElement o, string key, Func<JsonElement, T> parse) => List(Required(o, key), parse);

    public static T Req<T>(JsonElement o, string key, Func<JsonElement, T> parse) => parse(Required(o, key));

    public static IReadOnlyList<string> OptStrings(JsonElement o, string key) =>
        Has(o, key) ? List(o.GetProperty(key), AsString) : Array.Empty<string>();

    public static IReadOnlyDictionary<string, string> OptStringMap(JsonElement o, string key)
    {
        var map = new Dictionary<string, string>(StringComparer.Ordinal);
        if (!Has(o, key)) return map;
        JsonElement v = o.GetProperty(key);
        if (v.ValueKind != JsonValueKind.Object) throw new SkillsWireException();
        foreach (JsonProperty p in v.EnumerateObject()) map[p.Name] = AsString(p.Value);
        return map;
    }

    /// <summary>Swift's <c>(try? decodeIfPresent(...)) ?? nil</c>: malformed is absent.</summary>
    public static T? Lenient<T>(JsonElement o, string key, Func<JsonElement, T> parse) where T : class
    {
        if (!Has(o, key)) return null;
        try { return parse(o.GetProperty(key)); }
        catch (SkillsWireException) { return null; }
        catch (InvalidOperationException) { return null; }
        catch (KeyNotFoundException) { return null; }
    }

    public static int? Lenient(JsonElement o, string key, Func<JsonElement, int?> parse)
    {
        if (!Has(o, key)) return null;
        try { return parse(o.GetProperty(key)); }
        catch (SkillsWireException) { return null; }
    }
}
