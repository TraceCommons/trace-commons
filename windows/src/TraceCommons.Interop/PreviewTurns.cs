using System;
using System.Collections.Generic;
using System.Globalization;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop;

/// <summary>One turn of the index: the event that opens it and where it sits in the body.</summary>
public sealed record PreviewTurn(int Index, string Role, string? ToolName, int ByteOffset, int ByteLen);

/// <summary>
/// The turn index over a redacted preview body, decoded from
/// <c>tc_preview_turns_json</c>: the Mac's <c>PreviewTurns</c>
/// (TCShellCore/PreviewTurns.swift). An overlay of byte ranges on the body
/// <c>tc_preview_body</c> returned, never text.
/// </summary>
public sealed record PreviewTurns(
    string EntryId,
    string BodyDigest,
    string EnvelopeDigest,
    int TurnCount,
    IReadOnlyList<PreviewTurn> Turns)
{
    /// <summary>
    /// The anchor the index requires: <c>sha256:</c> and lowercase hex over
    /// the exact UTF-8 bytes of the body on screen (<c>LookInside.bodyDigest</c>,
    /// LookInside.swift:13-15).
    /// </summary>
    public static string DigestOf(string body)
    {
        ArgumentNullException.ThrowIfNull(body);
        return "sha256:" + Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(body))).ToLowerInvariant();
    }

    /// <summary>Whether this index is for the body with <paramref name="bodyDigest"/>; one for any other body is not drawn.</summary>
    public bool Indexes(string bodyDigest) => BodyDigest == bodyDigest;

    /// <summary>The index, or null when it will not parse as the Mac decodes it.</summary>
    public static PreviewTurns? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || SessionDetail.Str(root, "entry_id") is not { } entry
                || SessionDetail.Str(root, "body_digest") is not { } body
                || SessionDetail.Str(root, "envelope_digest") is not { } envelope
                || !root.TryGetProperty("turn_count", out JsonElement countElement)
                || !countElement.TryGetInt32(out int count)
                || !root.TryGetProperty("turns", out JsonElement turns)
                || turns.ValueKind != JsonValueKind.Array)
            {
                return null;
            }

            var list = new List<PreviewTurn>();
            foreach (JsonElement turn in turns.EnumerateArray())
            {
                if (!Int(turn, "index", out int index)
                    || SessionDetail.Str(turn, "role") is not { } role
                    || !Int(turn, "byte_offset", out int offset)
                    || !Int(turn, "byte_len", out int length))
                {
                    return null;
                }

                list.Add(new PreviewTurn(index, role, SessionDetail.Str(turn, "tool_name"), offset, length));
            }

            return new PreviewTurns(entry, body, envelope, count, list);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static bool Int(JsonElement item, string key, out int value)
    {
        value = 0;
        return item.ValueKind == JsonValueKind.Object
            && item.TryGetProperty(key, out JsonElement element)
            && element.TryGetInt32(out value);
    }
}

/// <summary>
/// How a turn reads in the Turn index tab (<c>LookInside.turnTitle</c> and
/// <c>turnDetail</c>, LookInside.swift:97-108), in the core's Look inside
/// words (<c>tc_monitor_traces_copy_json</c>, <c>look_inside</c>).
/// </summary>
public static class PreviewTurnFormat
{
    private static readonly Lazy<string?> Words = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_traces_copy_json()));

    /// <summary>A word of the core's <c>look_inside</c> table; empty when absent.</summary>
    public static string Word(string key) => MonitorWords.WordAt(Words.Value, "look_inside", key);

    /// <summary>A Traces word at the table's top level (<c>request_failed</c>); empty when absent.</summary>
    public static string TracesWord(string key) => MonitorWords.WordAt(Words.Value, null, key);

    /// <summary>A turn's heading: its position and its opening event's type.</summary>
    public static string Title(PreviewTurn turn) =>
        $"{(turn.Index + 1).ToString(CultureInfo.InvariantCulture)}. {turn.Role.Replace('_', ' ')}";

    /// <summary>
    /// A turn's detail: the tool it names, or the core's word for an event
    /// that names none, and its byte range in the core's words.
    /// </summary>
    public static string Detail(PreviewTurn turn, Func<string, string> word)
    {
        string bytes = word("turn_bytes")
            .Replace("{start}", turn.ByteOffset.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
            .Replace("{end}", (turn.ByteOffset + turn.ByteLen).ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal);
        return (turn.ToolName ?? word("turn_event")) + " · " + bytes;
    }
}
