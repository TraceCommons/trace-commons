using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// What the contributor said about one agent's session store.
///
/// Three states, not two. <see cref="Undecided"/> is the state every source
/// starts in and is NOT a synonym for "do not watch": the daemon reads an
/// undeclared source as the conventional per-user location, i.e. the
/// contributor's real <c>~/.claude</c> or <c>~/.codex</c>. So the answer a
/// privacy-conscious contributor is most likely to give -- "I don't use this
/// one" -- is exactly the answer that must be sent as a real declaration
/// rather than left unsent.
///
/// Mirrors <c>SourceDeclaration</c> in
/// <c>crates/trace-commons-contributor/src/daemon/settings.rs</c>, which owns
/// the serialized shape.
/// </summary>
public enum SourceDecisionKind
{
    /// <summary>Not answered yet. Never sent; never a fallback.</summary>
    Undecided,

    /// <summary>Watch a folder the contributor named.</summary>
    Watch,

    /// <summary>The contributor does not use this agent. Nothing is watched.</summary>
    Off,
}

/// <summary>
/// One source's answer, and the folder it names when there is one.
/// </summary>
public readonly struct SourceDecision : IEquatable<SourceDecision>
{
    private SourceDecision(SourceDecisionKind kind, string path)
    {
        Kind = kind;
        Path = path;
    }

    /// <summary>The starting state for every source on the roots screen.</summary>
    public static SourceDecision Undecided { get; } =
        new(SourceDecisionKind.Undecided, string.Empty);

    /// <summary>"I don't use this agent", as an answer the daemon obeys.</summary>
    public static SourceDecision Off { get; } = new(SourceDecisionKind.Off, string.Empty);

    /// <summary>
    /// Watch <paramref name="path"/>. A blank path is not a declaration, so
    /// it collapses to <see cref="Undecided"/> rather than producing a
    /// settings object the daemon would reject.
    /// </summary>
    public static SourceDecision Watch(string path)
    {
        string trimmed = (path ?? string.Empty).Trim();
        return trimmed.Length == 0
            ? Undecided
            : new SourceDecision(SourceDecisionKind.Watch, trimmed);
    }

    public SourceDecisionKind Kind { get; }

    /// <summary>The folder to watch, or empty for every other kind.</summary>
    public string Path { get; }

    /// <summary>Whether this source has been answered at all.</summary>
    public bool IsDecided => Kind != SourceDecisionKind.Undecided;

    public bool Equals(SourceDecision other) =>
        Kind == other.Kind && string.Equals(Path, other.Path, StringComparison.Ordinal);

    public override bool Equals(object? obj) => obj is SourceDecision other && Equals(other);

    public override int GetHashCode() => HashCode.Combine(Kind, Path);

    public static bool operator ==(SourceDecision left, SourceDecision right) => left.Equals(right);

    public static bool operator !=(SourceDecision left, SourceDecision right) => !left.Equals(right);
}

/// <summary>
/// One candidate session store as the ABI described it.
///
/// Deserialized from <c>tc_discover_sources</c>. Every field exists to make
/// the consent prompt specific: a path with 946 sessions and activity two
/// hours ago is a materially different thing to agree to than a directory
/// that is not there.
/// </summary>
public sealed class SourceCandidate
{
    /// <summary><c>claude-code</c> or <c>codex</c>.</summary>
    [JsonPropertyName("source")]
    public string Source { get; set; } = string.Empty;

    /// <summary>Where this store would be watched.</summary>
    [JsonPropertyName("path")]
    public string Path { get; set; } = string.Empty;

    /// <summary>Whether that directory is there right now.</summary>
    [JsonPropertyName("exists")]
    public bool Exists { get; set; }

    /// <summary>Session files found, counted recursively.</summary>
    [JsonPropertyName("session_count")]
    public ulong SessionCount { get; set; }

    /// <summary>Most recent session-file mtime, or null.</summary>
    [JsonPropertyName("most_recent")]
    public DateTimeOffset? MostRecent { get; set; }

    /// <summary>
    /// Whether <c>CLAUDE_CONFIG_DIR</c> or <c>CODEX_HOME</c> moved this store,
    /// so the screen can say why the path is not the usual one.
    /// </summary>
    [JsonPropertyName("relocated_by_env")]
    public bool RelocatedByEnv { get; set; }
}

/// <summary>
/// Reads the machine's candidate session stores through the ABI.
///
/// Parsing is separated from the native call so it can be tested on a machine
/// with no native library, which is the same reason <see cref="ReadGate"/> and
/// <see cref="WithdrawCopy"/> live in this assembly.
/// </summary>
public static class SourceDiscovery
{
    /// <summary>The <c>source</c> value for Claude Code's store.</summary>
    public const string ClaudeCode = "claude-code";

    /// <summary>The <c>source</c> value for Codex's store.</summary>
    public const string Codex = "codex";

    /// <summary>The <c>source</c> value for the Gemini CLI's store.</summary>
    public const string GeminiCli = "gemini-cli";

    /// <summary>The <c>source</c> value for Cline's store.</summary>
    public const string Cline = "cline";

    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNameCaseInsensitive = false,
    };

    /// <summary>
    /// Probes this machine. Returns an empty list rather than throwing when
    /// the ABI reports a panic: a roots screen that cannot describe the
    /// standard locations must still let the contributor name a folder by
    /// hand, and a discovery failure is not a reason to refuse them that.
    /// </summary>
    public static IReadOnlyList<SourceCandidate> ProbeThisMachine()
    {
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_discover_sources());
        return json is null ? Array.Empty<SourceCandidate>() : Parse(json);
    }

    /// <summary>
    /// Parses a <c>tc_discover_sources</c> payload. Malformed input yields an
    /// empty list, for the reason given on <see cref="ProbeThisMachine"/>.
    /// </summary>
    public static IReadOnlyList<SourceCandidate> Parse(string json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return Array.Empty<SourceCandidate>();
        }

        try
        {
            return JsonSerializer.Deserialize<List<SourceCandidate>>(json, Options)
                   ?? (IReadOnlyList<SourceCandidate>)Array.Empty<SourceCandidate>();
        }
        catch (JsonException)
        {
            return Array.Empty<SourceCandidate>();
        }
    }

    /// <summary>The candidate for <paramref name="source"/>, or null.</summary>
    public static SourceCandidate? For(
        IReadOnlyList<SourceCandidate> candidates,
        string source)
    {
        ArgumentNullException.ThrowIfNull(candidates);

        foreach (SourceCandidate candidate in candidates)
        {
            if (string.Equals(candidate.Source, source, StringComparison.Ordinal))
            {
                return candidate;
            }
        }

        return null;
    }
}

/// <summary>
/// The two answers the roots screen collects, and the settings object that
/// declares them to the daemon.
///
/// BOTH, always. <c>daemon::settings::roots_declared</c> owns that rule and
/// the C ABI enforces it; this type only refuses to send something it already
/// knows will be refused, so an unfinished screen reads as unfinished instead
/// of as an error from across the boundary.
///
/// The Linux and macOS shells collect the same two answers. The serialized
/// shape belongs to
/// <c>crates/trace-commons-contributor/src/daemon/settings.rs</c>, not to any
/// shell.
/// </summary>
public sealed class SessionRootsDeclaration
{
    /// <summary>What the contributor said about Claude Code's sessions.</summary>
    public SourceDecision Claude { get; set; } = SourceDecision.Undecided;

    /// <summary>What the contributor said about Codex's sessions.</summary>
    public SourceDecision Codex { get; set; } = SourceDecision.Undecided;

    /// <summary>What the contributor said about the Gemini CLI's sessions.</summary>
    public SourceDecision Gemini { get; set; } = SourceDecision.Undecided;

    /// <summary>What the contributor said about Cline's sessions.</summary>
    public SourceDecision Cline { get; set; } = SourceDecision.Undecided;

    /// <summary>
    /// Whether Claude Code and Codex have been answered. Continue stays
    /// disabled until this is true -- an unanswered source is not "no".
    ///
    /// Gemini and Cline are deliberately excluded. This mirrors
    /// <c>daemon::settings::roots_declared</c>, the rule that actually gates
    /// the daemon starting, which stays two-conjunct: an absent Gemini or
    /// Cline declaration constructs no adapter, so nothing is read unasked.
    /// Requiring them here would refuse to start for every contributor
    /// upgrading from a build that never asked them, over a store the daemon
    /// will not touch either way. Both are still offered and still recorded
    /// when answered; they just cannot block.
    /// </summary>
    public bool IsComplete => Claude.IsDecided && Codex.IsDecided;

    /// <summary>
    /// The <c>settings_json</c> argument for the settings-bearing daemon
    /// start, or null when either source is unanswered.
    ///
    /// Serialized, never concatenated. On Windows these paths are full of
    /// backslashes and may contain quotes, and hand-built JSON would corrupt
    /// the first store whose folder had either. It carries only recognized
    /// keys, because the settings validator rejects an unknown top-level key
    /// rather than ignoring it.
    /// </summary>
    public string? SettingsJson()
    {
        if (!IsComplete)
        {
            return null;
        }

        // Typed all the way down rather than Dictionary<string, object>: an
        // object-valued map leaves the serializer to decide from the runtime
        // type, and the shape of this payload is the one thing here that must
        // not depend on a serializer's polymorphism rules.
        var payload = new Dictionary<string, Dictionary<string, string>>(StringComparer.Ordinal)
        {
            ["claude_source"] = Describe(Claude),
            ["codex_source"] = Describe(Codex),
        };

        // Only when answered. Absent is the tri-state's "never asked", which
        // the contributor library reads as "construct no adapter"; sending
        // "off" for an unanswered row would record a refusal nobody made.
        if (Gemini.IsDecided)
        {
            payload["gemini_source"] = Describe(Gemini);
        }

        if (Cline.IsDecided)
        {
            payload["cline_source"] = Describe(Cline);
        }

        return JsonSerializer.Serialize(payload);
    }

    private static Dictionary<string, string> Describe(SourceDecision decision) =>
        decision.Kind == SourceDecisionKind.Off
            ? new Dictionary<string, string>(StringComparer.Ordinal) { ["mode"] = "off" }
            : new Dictionary<string, string>(StringComparer.Ordinal)
            {
                ["mode"] = "watch",
                ["path"] = decision.Path,
            };
}
