using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The build's immutable feature identity, read from
/// <c>tc_feature_profile_json</c> (<c>BuildFeatureProfile</c>,
/// macos/Sources/TCShellCore/FeatureFlags.swift:4-19). Rust owns every
/// default and profile; this only checks that the daemon is the same build.
/// </summary>
public sealed record BuildFeatureProfile(int SchemaVersion, string Environment, string ProfileDigest)
{
    internal static readonly string[] Environments = { "production", "staging", "testing" };

    public static BuildFeatureProfile? Parse(string? json)
    {
        if (json is null) return null;
        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            JsonElement root = document.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("schema_version", out JsonElement version) || version.ValueKind != JsonValueKind.Number
                || !version.TryGetInt32(out int schema)
                || Text(root, "environment") is not { } environment
                || Text(root, "profile_digest") is not { } digest)
            {
                return null;
            }

            return new BuildFeatureProfile(schema, environment, digest);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>The build that asked: from the C ABI, freed here.</summary>
    public static BuildFeatureProfile? FromLibrary() =>
        Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_feature_profile_json()));

    public bool Valid => SchemaVersion == 1 && Environments.Contains(Environment) && FeatureFlagSnapshot.IsDigest(ProfileDigest);

    /// <summary>The daemon's snapshot names this build (<c>agrees(environment:digest:)</c>).</summary>
    public bool Agrees(string? environment, string? digest) =>
        Valid && Environment == environment && ProfileDigest == digest;

    internal static string? Text(JsonElement o, string key) =>
        o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;
}

/// <summary>One flag's decision as the daemon made it.</summary>
public sealed record FeatureFlagDecision(bool Enabled, bool Pending, bool CapabilityBlocked, string Timing);

/// <summary>
/// The daemon's <c>feature_flags_snapshot</c>, never a second evaluator
/// (<c>FeatureFlagSnapshot</c>, FeatureFlags.swift:21-71). A reply that does
/// not decode, or fails the Mac's checks, is no snapshot: every flag off.
/// </summary>
public sealed record FeatureFlagSnapshot(
    string Environment, string ProfileDigest, string InstanceId, string SampledAt, ulong Generation,
    string Source, ulong? Revision, string? ExpiresAt, bool Fresh, IReadOnlyDictionary<string, FeatureFlagDecision> Flags)
{
    private static readonly string[] Sources = { "default", "remote", "cache" };
    private static readonly string[] Timings = { "live", "next_start" };

    public static FeatureFlagSnapshot? Parse(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object) return null;
        if (!root.TryGetProperty("schema_version", out JsonElement version) || version.ValueKind != JsonValueKind.Number
            || !version.TryGetInt32(out int schema) || schema != 1) return null;
        if (BuildFeatureProfile.Text(root, "environment") is not { } environment
            || BuildFeatureProfile.Text(root, "profile_digest") is not { } digest
            || BuildFeatureProfile.Text(root, "instance_id") is not { } instance
            || BuildFeatureProfile.Text(root, "sampled_at") is not { } sampled
            || BuildFeatureProfile.Text(root, "source") is not { } source || !Sources.Contains(source)
            || !root.TryGetProperty("generation", out JsonElement generationValue)
            || generationValue.ValueKind != JsonValueKind.Number || !generationValue.TryGetUInt64(out ulong generation)
            || !root.TryGetProperty("fresh", out JsonElement freshValue)
            || freshValue.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
            || !root.TryGetProperty("flags", out JsonElement flagsValue) || flagsValue.ValueKind != JsonValueKind.Object)
        {
            return null;
        }

        ulong? revision = null;
        if (root.TryGetProperty("revision", out JsonElement revisionValue) && revisionValue.ValueKind != JsonValueKind.Null)
        {
            if (revisionValue.ValueKind != JsonValueKind.Number || !revisionValue.TryGetUInt64(out ulong r)) return null;
            revision = r;
        }

        string? expires = null;
        if (root.TryGetProperty("expires_at", out JsonElement expiresValue) && expiresValue.ValueKind != JsonValueKind.Null)
        {
            if (expiresValue.ValueKind != JsonValueKind.String) return null;
            expires = expiresValue.GetString();
        }

        var flags = new Dictionary<string, FeatureFlagDecision>(StringComparer.Ordinal);
        foreach (JsonProperty flag in flagsValue.EnumerateObject())
        {
            JsonElement d = flag.Value;
            if (d.ValueKind != JsonValueKind.Object || Bool(d, "enabled") is not { } enabled || Bool(d, "pending") is not { } pending
                || Bool(d, "capability_blocked") is not { } blocked
                || BuildFeatureProfile.Text(d, "timing") is not { } timing || !Timings.Contains(timing))
            {
                return null;
            }

            flags[flag.Name] = new FeatureFlagDecision(enabled, pending, blocked, timing);
        }

        // The Mac's guard (FeatureFlags.swift:61-64): a positive generation,
        // a UUID instance, a known environment and a lowercase hex digest.
        if (generation == 0 || !Guid.TryParseExact(instance, "D", out _) || !BuildFeatureProfile.Environments.Contains(environment)
            || !IsDigest(digest))
        {
            return null;
        }

        return new FeatureFlagSnapshot(environment, digest, instance, sampled, generation, source, revision, expires,
            freshValue.GetBoolean(), flags);
    }

    public bool IsEnabled(string id) => Flags.TryGetValue(id, out FeatureFlagDecision? decision) && decision.Enabled;

    /// <summary>
    /// Whether <paramref name="newer"/> may replace this one: the same build,
    /// and either another daemon instance or a later generation
    /// (<c>acceptsSuccessor</c>, FeatureFlags.swift:66-69).
    /// </summary>
    public bool AcceptsSuccessor(FeatureFlagSnapshot newer) =>
        Environment == newer.Environment && ProfileDigest == newer.ProfileDigest
        && (!string.Equals(InstanceId, newer.InstanceId, StringComparison.OrdinalIgnoreCase) || newer.Generation > Generation);

    internal static bool IsDigest(string value) =>
        value.Length == 64 && value.All(c => c is >= '0' and <= '9' or >= 'a' and <= 'f');

    private static bool? Bool(JsonElement o, string key) =>
        o.TryGetProperty(key, out JsonElement v) && v.ValueKind is JsonValueKind.True or JsonValueKind.False ? v.GetBoolean() : null;
}

/// <summary>
/// The window's feature flags, read as the Mac reads them
/// (AppModel+FeatureFlags.swift, DaemonClient+FeatureFlags.swift): asked of
/// the daemon at start, on <c>feature_flags_changed</c>, after a lag or a
/// resync, and after the machine wakes (with <c>feature_flags_refresh</c>
/// first). A failed read, or a snapshot of another build, is none, and
/// with none every flag is off. An older read never lands over a newer one,
/// and a snapshot the current one does not accept as its successor is
/// dropped.
/// </summary>
public sealed class FeatureFlagGate
{
    public const string SnapshotMethod = "feature_flags_snapshot";
    public const string RefreshMethod = "feature_flags_refresh";
    public const string ChangedEvent = "feature_flags_changed";
    public const string InsightsSkills = "insights_skills";

    private readonly Func<string, string, Task<DaemonResponse>>? _call;
    private readonly Func<BuildFeatureProfile?> _profile;
    private long _generation;

    /// <param name="call">The daemon; none in a sample window, where every flag stays off as on the Mac.</param>
    /// <param name="profile">This build's identity; the library's unless a test passes one.</param>
    public FeatureFlagGate(Func<string, string, Task<DaemonResponse>>? call, Func<BuildFeatureProfile?>? profile = null)
    {
        _call = call;
        _profile = profile ?? BuildFeatureProfile.FromLibrary;
    }

    public FeatureFlagSnapshot? Current { get; private set; }

    /// <summary>Raised when <see cref="Current"/> changes, on the caller's thread.</summary>
    public event Action? Changed;

    public bool IsEnabled(string id) => Current?.IsEnabled(id) ?? false;

    /// <summary>Clears the snapshot: the daemon (re)started and nothing is known yet.</summary>
    public void Reset()
    {
        _generation++;
        Set(null);
    }

    public async Task RefreshAsync()
    {
        if (_call is null)
        {
            Set(null);
            return;
        }

        long generation = ++_generation;
        FeatureFlagSnapshot? snapshot = null;
        try
        {
            DaemonResponse response = await _call(SnapshotMethod, "{}").ConfigureAwait(true);
            if (!response.IsError && response.Result is { } result
                && FeatureFlagSnapshot.Parse(result) is { } parsed
                && _profile() is { } build && build.Agrees(parsed.Environment, parsed.ProfileDigest))
            {
                snapshot = parsed;
            }
        }
        catch (Exception)
        {
            snapshot = null;
        }

        if (generation != _generation) return;
        if (snapshot is null)
        {
            Set(null);
            return;
        }

        if (Current is { } old && !old.AcceptsSuccessor(snapshot)) return;
        Set(snapshot);
    }

    /// <summary>After a wake: ask the daemon to refresh, then read (<c>refreshFeatureFlagsAfterWake</c>).</summary>
    public async Task RefreshAfterWakeAsync()
    {
        if (_call is null) return;
        // Not awaited, as on the Mac: the read below runs at once, and the
        // daemon's own change event brings the refreshed answer.
        _ = _call(RefreshMethod, "{}").ContinueWith(t => _ = t.Exception, TaskScheduler.Default);
        await RefreshAsync().ConfigureAwait(true);
    }

    private void Set(FeatureFlagSnapshot? snapshot)
    {
        if (Equals(Current, snapshot)) return;
        Current = snapshot;
        Changed?.Invoke();
    }
}
