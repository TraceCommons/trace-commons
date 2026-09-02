using System;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The routing surface's wording, across the C ABI.
///
/// Nothing in this file is a word. The vocabulary crosses as JSON and the
/// sentences cross already assembled, so this shell never fills in a template
/// and there is no fourth place the wording can drift to.
/// </summary>
public static class RoutingSurface
{
    /// <summary>
    /// Every fixed string on the surface, or null when the call failed or the
    /// payload will not parse.
    ///
    /// Null, never a partly-filled record: a screen rendering an empty string
    /// beside a tool name is worse than one rendering nothing, and a screen
    /// rendering a C#-authored word is worse than both. The caller decides
    /// what to show when the words are not available.
    /// </summary>
    public static RoutingCopy? Copy() => Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_routing_copy()));

    /// <summary>
    /// The payload half of <see cref="Copy"/>, split out so it is testable
    /// without the cdylib. The native call is a one-liner; this is where the
    /// behaviour that can actually be wrong lives.
    /// </summary>
    internal static RoutingCopy? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            RoutingCopy? copy = JsonSerializer.Deserialize<RoutingCopy>(json);
            if (copy is null)
            {
                return null;
            }

            // A field the Rust stopped exporting would deserialise to the
            // empty string and render as a blank. Refuse the whole payload
            // instead: a missing word here is a missing verdict.
            foreach (string word in copy.Words)
            {
                if (string.IsNullOrEmpty(word))
                {
                    return null;
                }
            }

            return copy;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// One tool's word, decided by the shared branch table rather than by
    /// this shell.
    ///
    /// Null when the ABI would produce no word, which is what a caught panic
    /// or an unreadable source mode looks like from here. The caller decides
    /// what to show; this method never invents one.
    /// </summary>
    public static string? ToolWord(string? sourceMode, ToolWiring wiring) =>
        NativeMethods.TakeOwnedString(
            NativeMethods.tc_routing_tool_word(sourceMode, (int)wiring));

    /// <summary>
    /// How that word is painted, from the same two inputs.
    ///
    /// Never null and never a failure: anything the ABI cannot read answers
    /// the neutral tone, which claims nothing. A styling call that could fail
    /// would leave this shell choosing a tone for itself, which is exactly
    /// what crossing the boundary is meant to stop.
    /// </summary>
    public static RoutingTone ToolTone(string? sourceMode, ToolWiring wiring) =>
        NativeMethods.tc_routing_tool_tone(sourceMode, (int)wiring) == AbiToneClear
            ? RoutingTone.Clear
            : RoutingTone.Neutral;

    /// <summary>
    /// <c>TC_TOOL_TONE_CLEAR</c>. Spelled out rather than cast from
    /// <see cref="RoutingTone"/>, whose Clear is 2 because it carries a third
    /// value the tool words never take. The two numberings are unrelated and
    /// a cast between them would be silently wrong.
    /// </summary>
    private const int AbiToneClear = 1;

    /// <summary>
    /// The daemon's routing state, in words, decided by the shared branch
    /// table. Null only when the ABI produced nothing.
    /// </summary>
    public static string? StateLine(string? state) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_routing_state_line(state));

    /// <summary>
    /// "That file could not be used", assembled on the Rust side.
    /// <paramref name="tokenPath"/> is null when nothing resolved at all,
    /// which is a different sentence and not an error.
    /// </summary>
    public static string? TokenLine(string? tokenPath) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_routing_token_line(tokenPath));

    /// <summary>
    /// "Nothing answered", assembled on the Rust side. A null port is "no port
    /// was tried"; the sentence for that names none rather than naming port 0.
    /// </summary>
    public static string? UnreachableLine(ushort? port) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_routing_unreachable_line(port ?? 0));

    /// <summary>
    /// "Last checked ...", assembled on the Rust side around this shell's own
    /// humanised time -- the one part of this surface each shell renders for
    /// itself, because it is a rendering of a timestamp and not wording about
    /// routing.
    ///
    /// Returns null rather than a half-sentence for an empty time.
    /// </summary>
    public static string? LastChecked(string when) =>
        string.IsNullOrEmpty(when)
            ? null
            : NativeMethods.TakeOwnedString(NativeMethods.tc_routing_last_checked(when));
}
