using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The notice the Rust assembled for one grant the daemon voided (R6 of the
/// connect-and-forget design): automatic contributing stopped, for which
/// project or for new projects, why, and that it can be turned back on.
///
/// <para>
/// Every property is filled from the payload and none is written here.
/// </para>
/// </summary>
public sealed record GrantVoidNotice
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;

    [JsonPropertyName("body")] public string Body { get; init; } = string.Empty;

    [JsonPropertyName("reasons_heading")] public string ReasonsHeading { get; init; } = string.Empty;

    /// <summary>One sentence per reason, in the daemon's order.</summary>
    [JsonPropertyName("reasons")] public List<string> Reasons { get; init; } = new();

    [JsonPropertyName("rearm")] public string Rearm { get; init; } = string.Empty;

    /// <summary>
    /// The button. It records that the notice was shown and does nothing
    /// else; turning automatic contributing back on is its own act.
    /// </summary>
    [JsonPropertyName("acknowledge")] public string Acknowledge { get; init; } = string.Empty;

    /// <summary>
    /// "Turn back on", or null when the notice has no project to arm: the
    /// automatic grant's, and an unplaced one.
    /// </summary>
    [JsonPropertyName("rearm_action")] public string? RearmAction { get; init; }

    /// <summary>Shown when the daemon refuses the re-arm; null exactly when the button is.</summary>
    [JsonPropertyName("rearm_failed")] public string? RearmFailed { get; init; }

    /// <summary>The payload fields this shell decodes, by wire name.</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "reasons_heading", "reasons", "rearm", "acknowledge",
        "rearm_action", "rearm_failed",
    };
}

/// <summary>
/// One void notice as the window draws it: the id
/// <c>acknowledge_grant_voids</c> takes, and the notice.
/// </summary>
public sealed record GrantVoidCard(ulong? Id, GrantVoidNotice Notice, string? RearmProjectId = null)
{
    public bool CanAcknowledge => Id.HasValue;

    /// <summary>
    /// Whether to draw "Turn back on": the core offered it and the element
    /// names a project. Never for the grant's notice or an unplaced one.
    /// </summary>
    public bool CanRearm => RearmProjectId is not null && Notice.RearmAction is not null;
}

/// <summary>
/// <c>status.grant_voids</c>, turned into notices by the ABI.
///
/// <para>
/// Each element goes back to the Rust exactly as the daemon sent it
/// (<c>tc_grant_void_notice</c>), which reads <c>kind</c> and
/// <c>reasons</c> and returns the words. This shell never reads either to
/// choose a sentence: a branch kept in four shells drifts the way words do.
/// </para>
/// </summary>
public static class GrantVoidNotices
{
    /// <summary>
    /// The cards for every void in <paramref name="voids"/>, worded by
    /// <paramref name="word"/> (the ABI in the app, a fake in tests). An
    /// element the ABI returns nothing for is left out; the ABI words every
    /// element it can read, including one it cannot place, so that is a
    /// caught panic rather than a void it chose not to report.
    /// </summary>
    public static IReadOnlyList<GrantVoidCard> Cards(
        IReadOnlyList<JsonElement>? voids,
        Func<string, string?> word)
    {
        var cards = new List<GrantVoidCard>();
        if (voids is null)
        {
            return cards;
        }

        foreach (JsonElement element in voids)
        {
            if (Parse(word(element.GetRawText())) is not { } notice)
            {
                continue;
            }

            ulong? id = element.ValueKind == JsonValueKind.Object
                && element.TryGetProperty("id", out JsonElement raw)
                && raw.TryGetUInt64(out ulong value)
                ? value
                : null;
            string? project = notice.RearmAction is not null
                && element.ValueKind == JsonValueKind.Object
                && element.TryGetProperty("project_id", out JsonElement rawProject)
                && rawProject.ValueKind == JsonValueKind.String
                && rawProject.GetString() is { Length: > 0 } projectId
                ? projectId
                : null;
            cards.Add(new GrantVoidCard(id, notice, project));
        }

        return cards;
    }

    /// <summary>
    /// The <c>set_project_mode</c> params "Turn back on" sends, or null when
    /// the card has no button. The same request Settings sends to arm a
    /// project, so the daemon applies the same refusals, writes the same
    /// <c>armed-auto-upload</c> row, and clears the notice.
    /// </summary>
    public static string? RearmParams(GrantVoidCard card) =>
        card.CanRearm
            ? JsonSerializer.Serialize(new Dictionary<string, string>
            {
                ["project_id"] = card.RearmProjectId!,
                ["mode"] = "auto_upload",
            })
            : null;

    /// <summary>The cards for <paramref name="voids"/>, worded by the ABI.</summary>
    public static IReadOnlyList<GrantVoidCard> Cards(IReadOnlyList<JsonElement>? voids) =>
        Cards(voids, Word);

    private static string? Word(string wireJson) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_grant_void_notice(wireJson));

    /// <summary>
    /// Decode one notice, or null if it will not parse, a sentence is empty,
    /// or there is no reason. Shown whole or not at all: a notice that says
    /// automatic contributing stopped without saying why is the silent void
    /// this exists to prevent, one layer down.
    /// </summary>
    internal static GrantVoidNotice? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            GrantVoidNotice? notice = JsonSerializer.Deserialize<GrantVoidNotice>(json);
            if (notice is null || notice.Reasons.Count == 0)
            {
                return null;
            }

            foreach (string sentence in new[]
                     {
                         notice.Title, notice.Body, notice.ReasonsHeading, notice.Rearm, notice.Acknowledge,
                     })
            {
                if (string.IsNullOrEmpty(sentence))
                {
                    return null;
                }
            }

            foreach (string reason in notice.Reasons)
            {
                if (string.IsNullOrEmpty(reason))
                {
                    return null;
                }
            }

            // The button and its refusal line travel together, and neither
            // is ever an empty string.
            if ((notice.RearmAction is null) != (notice.RearmFailed is null)
                || notice.RearmAction?.Length == 0
                || notice.RearmFailed?.Length == 0)
            {
                return null;
            }

            return notice;
        }
        catch (JsonException)
        {
            return null;
        }
    }
}
