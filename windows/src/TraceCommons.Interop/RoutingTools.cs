using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// What IronWire answered about one tool, as far as this surface may use it.
/// </summary>
/// <remarks>
/// Deliberately three states and not a boolean. The word this shell printed
/// before was computed from a single switch declaring IronWire in this app,
/// which has no third state and therefore no way to say "nobody has told us".
/// That missing state is the whole defect: a dead proxy and a tool IronWire
/// has never heard of both used to render as a confident verdict.
/// </remarks>
public enum ToolWiring
{
    /// <summary>IronWire listed this tool and said it is pointed at a local address.</summary>
    Wired,

    /// <summary>IronWire listed this tool and said it is not.</summary>
    NotWired,

    /// <summary>
    /// Nothing usable answered, IronWire did not list this tool, or it listed
    /// it as not present on this machine. No verdict is available.
    /// </summary>
    Unknown,
}

/// <summary>The three shapes the daemon's probe can answer in, plus "unreadable".</summary>
public enum RoutingProbeKind
{
    /// <summary>The proxy answered and the credential was accepted.</summary>
    Reachable,

    /// <summary>The file could not be read, or was read and refused.</summary>
    TokenUnreadable,

    /// <summary>Nothing usable answered.</summary>
    Unreachable,

    /// <summary>
    /// An answer this build cannot read, or no answer at all. Claims nothing
    /// about the proxy in either direction.
    /// </summary>
    Unknown,
}

/// <summary>
/// How firmly the daemon's state reads. There is no fault tone, because none
/// of the three states is a fault.
/// </summary>
public enum RoutingTone
{
    /// <summary>Nothing is declared, so nothing is claimed.</summary>
    Neutral,

    /// <summary>Declared, and no answer has arrived yet. Normal, not broken.</summary>
    Held,

    /// <summary>Declared, and answers are arriving.</summary>
    Clear,
}

/// <summary>What the daemon's probe answered, reduced to what may be rendered.</summary>
public sealed record RoutingProbe(RoutingProbeKind Kind, string? TokenPath, ushort? Port)
{
    /// <summary>
    /// Read a <c>probe_routing</c> or <c>probe_routed_tools</c> result.
    ///
    /// Every shape this build cannot read becomes <see cref="RoutingProbeKind.Unknown"/>
    /// rather than an exception or a guess: an answer nothing can parse is
    /// the same fact as no answer, and the surface must render that as no
    /// verdict.
    /// </summary>
    public static RoutingProbe Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return Unreadable;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object
                || !doc.RootElement.TryGetProperty("outcome", out JsonElement outcome)
                || outcome.ValueKind != JsonValueKind.String)
            {
                return Unreadable;
            }

            return outcome.GetString() switch
            {
                "reachable" => new RoutingProbe(RoutingProbeKind.Reachable, null, null),
                "token_unreadable" => new RoutingProbe(
                    RoutingProbeKind.TokenUnreadable,
                    ReadString(doc.RootElement, "token_path"),
                    null),
                "unreachable" => new RoutingProbe(
                    RoutingProbeKind.Unreachable,
                    null,
                    ReadPort(doc.RootElement)),
                _ => Unreadable,
            };
        }
        catch (JsonException)
        {
            return Unreadable;
        }
    }

    private static readonly RoutingProbe Unreadable =
        new(RoutingProbeKind.Unknown, null, null);

    private static string? ReadString(JsonElement root, string name) =>
        root.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    /// <summary>
    /// The port the daemon says it tried, or none.
    ///
    /// A missing port stays missing rather than becoming zero: the Rust
    /// sentence for "no port was tried" names none, and a zero here would
    /// send somebody to check a port that does not exist.
    /// </summary>
    private static ushort? ReadPort(JsonElement root) =>
        root.TryGetProperty("port", out JsonElement value)
        && value.ValueKind == JsonValueKind.Number
        && value.TryGetUInt16(out ushort port)
            ? port
            : null;
}

/// <summary>One row of IronWire's tool list, reduced to what a word may be built on.</summary>
public sealed record RoutedTool(bool Installed, bool Wired);

/// <summary>
/// What IronWire last answered when asked which tools are pointed at it.
/// </summary>
/// <remarks>
/// <see cref="Outcome"/> is what makes a dead proxy stop producing verdicts:
/// on anything but <see cref="RoutingProbeKind.Reachable"/> every tool reads
/// unknown, whatever rows the payload carried and whatever the declaration
/// switch says.
/// </remarks>
public sealed class RoutingEvidence
{
    private readonly IReadOnlyDictionary<string, RoutedTool> _tools;

    /// <summary>
    /// Internal rather than private so the tests can build the one shape
    /// <see cref="Parse"/> will not: rows held beside an outcome that is not
    /// reachable. The guard in <see cref="WiringFor"/> against exactly that is
    /// the surface's last line, and a guard no test can reach is a guard
    /// nobody knows still works.
    /// </summary>
    internal RoutingEvidence(RoutingProbe outcome, IReadOnlyDictionary<string, RoutedTool> tools)
    {
        Outcome = outcome;
        _tools = tools;
    }

    public RoutingProbe Outcome { get; }

    /// <summary>Read a <c>probe_routed_tools</c> result.</summary>
    public static RoutingEvidence Parse(string? json)
    {
        RoutingProbe outcome = RoutingProbe.Parse(json);
        var tools = new Dictionary<string, RoutedTool>(StringComparer.Ordinal);
        if (outcome.Kind != RoutingProbeKind.Reachable || string.IsNullOrWhiteSpace(json))
        {
            return new RoutingEvidence(outcome, tools);
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.TryGetProperty("tools", out JsonElement rows)
                && rows.ValueKind == JsonValueKind.Array)
            {
                foreach (JsonElement row in rows.EnumerateArray())
                {
                    if (row.ValueKind != JsonValueKind.Object
                        || !row.TryGetProperty("id", out JsonElement id)
                        || id.ValueKind != JsonValueKind.String)
                    {
                        continue;
                    }

                    string? name = id.GetString();
                    if (string.IsNullOrEmpty(name))
                    {
                        continue;
                    }

                    tools[name] = new RoutedTool(Flag(row, "installed"), Flag(row, "wired"));
                }
            }
        }
        catch (JsonException)
        {
            // Already Reachable with no rows, which is the right amount of
            // evidence about every tool: none.
        }

        return new RoutingEvidence(outcome, tools);
    }

    /// <summary>
    /// What may be said about one tool, from what IronWire answered about it.
    /// </summary>
    /// <remarks>
    /// The rules, and why each is where it is:
    /// <list type="bullet">
    /// <item><description><b>Nothing answered.</b> Unreachable and
    /// token-unreadable are stable states -- a port nothing is listening on,
    /// a credential that is not there or is refused -- so a word built on
    /// them would keep asserting while the card underneath says nothing
    /// answered. They yield unknown. This is the original defect, in the one
    /// string a person reads.</description></item>
    /// <item><description><b>The daemon's awaiting-rows state is deliberately
    /// not consulted here.</b> A proxy installed this morning legitimately
    /// reports it, and it returns to it whenever a declaration changes, so
    /// letting it downgrade the word would flicker it against a working
    /// install.</description></item>
    /// <item><description><b>Listed but not present.</b> IronWire saying a
    /// tool is not installed, while this app is watching that tool's
    /// sessions, is two detectors disagreeing about one machine. That is not
    /// evidence for a verdict.</description></item>
    /// </list>
    /// </remarks>
    public ToolWiring WiringFor(string id)
    {
        if (Outcome.Kind != RoutingProbeKind.Reachable)
        {
            return ToolWiring.Unknown;
        }

        if (!_tools.TryGetValue(id, out RoutedTool? row))
        {
            return ToolWiring.Unknown;
        }

        if (row.Wired)
        {
            return ToolWiring.Wired;
        }

        return row.Installed ? ToolWiring.NotWired : ToolWiring.Unknown;
    }

    private static bool Flag(JsonElement row, string name) =>
        row.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.True;
}

/// <summary>
/// What the contributor said about each of the three tools this surface
/// names: <c>get_settings</c>'s per-source mode, never the declaration switch.
/// </summary>
public sealed class RoutingModes
{
    public string Claude { get; init; } = string.Empty;

    public string Codex { get; init; } = string.Empty;

    public string Gemini { get; init; } = string.Empty;
}

/// <summary>One tool's name, its one word, and how that word is painted.</summary>
/// <remarks>
/// The tone travels with the word because both are decided by the same
/// shared branch table, from the same two inputs. A view must take it from
/// here and never re-derive it from <see cref="Word"/>: that would be a text
/// comparison against a privacy claim, and "Private" is a substring of the
/// denial that must never come back.
/// </remarks>
public sealed record RoutingToolRow(string Name, string Word, RoutingTone Tone)
{
    /// <summary>
    /// Read as one statement, not as a name and a stray word beside it.
    /// </summary>
    public string AccessibleLabel => Name + ": " + Word;
}

/// <summary>The state sentence, and the stamp under it where there is one.</summary>
public sealed record RoutingStatusLine(string Text, RoutingTone Tone, string? LastChecked);

/// <summary>
/// The routing surface's behaviour: which word each tool gets, what this
/// shell writes, and what it says about the answer.
/// </summary>
/// <remarks>
/// <b>No wording is authored here.</b> Every string a contributor reads comes
/// from <c>crates/trace-commons-contributor/src/routing_copy.rs</c> across the
/// C ABI -- the vocabulary as <see cref="RoutingCopy"/>, the sentences already
/// assembled by <see cref="RoutingSurface"/>. The only string literals in this
/// file are wire values: JSON keys, the daemon's state names, and IronWire's
/// own tool ids. <c>NoWordingIsAuthoredInThisShell</c> reads this file and
/// fails on any other literal.
/// </remarks>
public static class RoutingTools
{
    /// <summary>
    /// IronWire's conventional port, shown in the field so nobody has to know
    /// it.
    /// </summary>
    /// <remarks>
    /// <b>Shown is not declared.</b> Nothing is written until the contributor
    /// turns the switch on: absence means off with no fallback, and a
    /// displayed default that wrote itself would have this window announce a
    /// local service nobody mentioned.
    /// </remarks>
    public const ushort DefaultPort = 8463;

    /// <summary>
    /// The <c>set_settings</c> key. That call refuses an object holding a key
    /// it does not recognise, so a drift here is a silent no-write rather
    /// than an error.
    /// </summary>
    public const string SettingsKey = "ironwire";

    // IronWire's own stable tool ids. `ironwire connect <id>` takes these and
    // its settings response is keyed by them. Gemini CLI has no row upstream
    // at all today -- neither built in nor in the catalogue -- which is why
    // it is named here and expected to be missing rather than left out and
    // quietly defaulted.
    public const string ClaudeId = "claude";
    public const string CodexId = "codex";
    public const string GeminiId = "gemini";

    // The daemon's three routing states, from `status.routing.state`. Wire
    // values, and no longer a branch table: which sentence each reaches is
    // decided in routing_copy.rs and crosses the ABI. They are named here so
    // this shell and its tests can talk about the states the daemon reports
    // without spelling the literal in several files.
    public const string NotDeclared = "not_declared";
    public const string AwaitingRows = "awaiting_rows";
    public const string RowsSeen = "rows_seen";

    /// <summary>
    /// One tool's word, from what the contributor said about that tool's
    /// sessions and what IronWire said about that tool.
    /// </summary>
    /// <remarks>
    /// The declaration switch is <b>not</b> a parameter. It was the only
    /// input before, and that is what let a contributor read the wired word
    /// on the same card as "Nothing answered on port 8463".
    /// </remarks>
    public static string ToolWord(RoutingCopy copy, string sourceMode, ToolWiring wiring)
    {
        ArgumentNullException.ThrowIfNull(copy);

        // NOT A BRANCH TABLE HERE. Which of the four words a tool gets is
        // decided once, in routing_copy.rs, and crosses the ABI. It used to
        // be written out again in this file and again in Swift: three copies
        // of one decision that could drift apart in silence, because every
        // string they returned stayed identical.
        //
        // A word the ABI would not produce falls back to the one that claims
        // nothing, never to a word chosen here.
        return RoutingSurface.ToolWord(sourceMode, wiring) ?? copy.WordUnknown;
    }

    /// <summary>
    /// How that word is painted, from the same two inputs.
    /// </summary>
    /// <remarks>
    /// From <see cref="ToolWiring"/>, never from the rendered word. A bool
    /// meaning "this is the privacy word" invites a text comparison later,
    /// and a text comparison against a privacy claim is the shape that once
    /// let "unreachable" match "reachable" on this same surface.
    /// </remarks>
    public static RoutingTone ToolTone(string sourceMode, ToolWiring wiring) =>
        RoutingSurface.ToolTone(sourceMode, wiring);

    /// <summary>
    /// One row per tool, in the order the surface shows them.
    /// </summary>
    public static IReadOnlyList<RoutingToolRow> Rows(
        RoutingCopy copy,
        RoutingModes modes,
        RoutingEvidence? evidence)
    {
        ArgumentNullException.ThrowIfNull(copy);
        ArgumentNullException.ThrowIfNull(modes);

        return new[]
        {
            Row(copy, copy.ToolClaude, modes.Claude, ClaudeId, evidence),
            Row(copy, copy.ToolCodex, modes.Codex, CodexId, evidence),
            Row(copy, copy.ToolGemini, modes.Gemini, GeminiId, evidence),
        };
    }

    private static RoutingToolRow Row(
        RoutingCopy copy,
        string name,
        string mode,
        string id,
        RoutingEvidence? evidence)
    {
        ToolWiring wiring = evidence?.WiringFor(id) ?? ToolWiring.Unknown;
        return new RoutingToolRow(
            name,
            ToolWord(copy, mode, wiring),
            ToolTone(mode, wiring));
    }

    /// <summary>
    /// One outcome, one sentence.
    /// </summary>
    /// <remarks>
    /// The two interpolating sentences are assembled on the Rust side. A
    /// native call that could not be made falls back to the copy's
    /// "that check couldn't be run" line rather than to a sentence written
    /// here.
    /// </remarks>
    public static string ProbeLine(RoutingCopy copy, RoutingProbe probe)
    {
        ArgumentNullException.ThrowIfNull(copy);
        ArgumentNullException.ThrowIfNull(probe);

        return probe.Kind switch
        {
            RoutingProbeKind.Reachable => copy.ProbeReachable,
            RoutingProbeKind.TokenUnreadable =>
                RoutingSurface.TokenLine(probe.TokenPath) ?? copy.CheckUnavailable,
            RoutingProbeKind.Unreachable =>
                RoutingSurface.UnreachableLine(probe.Port) ?? copy.CheckUnavailable,
            _ => copy.CheckUnavailable,
        };
    }

    /// <summary>
    /// The daemon's three states, in words. A state this build does not know
    /// says what the off state says: it claims nothing.
    /// </summary>
    public static string StateLine(RoutingCopy copy, string state)
    {
        ArgumentNullException.ThrowIfNull(copy);

        // Decided across the ABI, for the reason on ToolWord. A sentence the
        // ABI would not produce falls back to the off line, which is what an
        // unknown state reads as anyway: it claims nothing.
        return RoutingSurface.StateLine(state) ?? copy.StateOff;
    }

    /// <summary>
    /// How firmly a state reads.
    /// </summary>
    /// <remarks>
    /// <see cref="AwaitingRows"/> is <see cref="RoutingTone.Held"/> and never
    /// a fault: a reader built a moment ago starts cold by construction, so
    /// this is the state a contributor sees immediately after touching
    /// anything on this card. Painting it as broken would accuse a working
    /// proxy at exactly that moment.
    /// </remarks>
    public static RoutingTone StateTone(string state) => RoutingSurface.StateTone(state);

    /// <summary>
    /// "Last checked ...", around this shell's own humanised time.
    /// </summary>
    /// <remarks>
    /// The stamp lives in the running daemon and is <b>per-process</b>: it
    /// starts empty again every time that process comes back up. So it is a
    /// "last checked" and never an install date or a "connected since". Null
    /// when there is no stamp, rather than a half-sentence with nothing after
    /// it.
    /// </remarks>
    public static string? LastCheckedLine(DateTimeOffset? at) =>
        LastCheckedLine(at, DateTimeOffset.UtcNow);

    /// <summary>
    /// As <see cref="LastCheckedLine(DateTimeOffset?)"/>, with the clock
    /// injected so the bands are testable.
    /// </summary>
    public static string? LastCheckedLine(DateTimeOffset? at, DateTimeOffset now) =>
        at is null ? null : RoutingSurface.LastChecked(SessionRootsCopy.HumanWhen(at, now));

    /// <summary>
    /// The state sentence, its tone, and the stamp where it says something.
    /// </summary>
    /// <remarks>
    /// The stamp is withheld on the state that has had no answer at all: a
    /// "last checked" beside "reading nothing" would be describing a check
    /// that never happened.
    /// </remarks>
    public static RoutingStatusLine StatusLine(
        RoutingCopy copy,
        string state,
        DateTimeOffset? lastRefreshAt,
        DateTimeOffset now)
    {
        RoutingTone tone = StateTone(state);
        return new RoutingStatusLine(
            StateLine(copy, state),
            tone,
            tone == RoutingTone.Neutral ? null : LastCheckedLine(lastRefreshAt, now));
    }

    /// <summary>
    /// As <see cref="StatusLine(RoutingCopy, string, DateTimeOffset?, DateTimeOffset)"/>,
    /// against the wall clock.
    /// </summary>
    public static RoutingStatusLine StatusLine(
        RoutingCopy copy,
        string state,
        DateTimeOffset? lastRefreshAt) =>
        StatusLine(copy, state, lastRefreshAt, DateTimeOffset.UtcNow);

    /// <summary>
    /// The one-key object <c>set_settings</c> is called with.
    /// </summary>
    /// <remarks>
    /// Off is a JSON null, not an object with a mode: there is no
    /// conventional fallback for a local service, so absence is the off
    /// state. An empty folder box is left out rather than sent as an empty
    /// string, which the daemon refuses outright.
    /// </remarks>
    public static string SerializeDeclaration(bool on, ushort port, string? tokenDir)
    {
        var declaration = new Dictionary<string, object?>(StringComparer.Ordinal);
        if (on)
        {
            declaration["mode"] = "watch";
            declaration["port"] = port;
            string trimmed = (tokenDir ?? string.Empty).Trim();
            if (trimmed.Length > 0)
            {
                declaration["token_dir"] = trimmed;
            }
        }

        return JsonSerializer.Serialize(
            new Dictionary<string, object?>(StringComparer.Ordinal)
            {
                [SettingsKey] = on ? declaration : null,
            });
    }

    /// <summary>What <c>probe_routed_tools</c> is asked. Same rule about the empty box.</summary>
    public static string SerializeProbeParams(ushort port, string? tokenDir)
    {
        var request = new Dictionary<string, object?>(StringComparer.Ordinal)
        {
            ["port"] = port,
        };
        string trimmed = (tokenDir ?? string.Empty).Trim();
        if (trimmed.Length > 0)
        {
            request["token_dir"] = trimmed;
        }

        return JsonSerializer.Serialize(request);
    }
}

/// <summary>
/// <c>get_settings</c>'s <c>ironwire</c> block, as the daemon holds it.
/// </summary>
/// <remarks>
/// Carries the declared folder, never a token: the daemon reads the
/// credential at call time and it never enters settings.
/// </remarks>
public sealed class RoutingDeclarationSnapshot
{
    [JsonPropertyName("mode")]
    public string Mode { get; set; } = string.Empty;

    [JsonPropertyName("port")]
    public ushort? Port { get; set; }

    [JsonPropertyName("token_dir")]
    public string? TokenDir { get; set; }
}

/// <summary>
/// <c>status.routing</c>. Three states and one per-process timestamp; nothing
/// identifying, no port and no path.
/// </summary>
public sealed class RoutingStatusSnapshot
{
    /// <summary>
    /// <c>not_declared</c>, <c>awaiting_rows</c> or <c>rows_seen</c>. Empty
    /// when the daemon did not report the block at all, which reads as the
    /// first.
    /// </summary>
    [JsonPropertyName("state")]
    public string State { get; set; } = string.Empty;

    /// <summary>
    /// When the daemon last got an answer. <b>Per-process</b>: it starts
    /// empty again every time the daemon comes back up, so it is a "last
    /// checked" and never a date this install began.
    /// </summary>
    [JsonPropertyName("last_refresh_at")]
    public DateTimeOffset? LastRefreshAt { get; set; }
}
