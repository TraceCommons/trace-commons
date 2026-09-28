using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The notice the Rust assembled for one armed folder whose arming wording no
/// longer claims a model scrubs its sessions (K5 of the connect-and-forget
/// design): that it is still armed, and what its arming now means.
///
/// <para>
/// Every property is filled from the payload and none is written here.
/// </para>
/// </summary>
public sealed record ArmingRewordedNotice
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;

    [JsonPropertyName("body")] public string Body { get; init; } = string.Empty;

    [JsonPropertyName("now_heading")] public string NowHeading { get; init; } = string.Empty;

    [JsonPropertyName("scope")] public string Scope { get; init; } = string.Empty;

    [JsonPropertyName("limit")] public string Limit { get; init; } = string.Empty;

    [JsonPropertyName("no_review")] public string NoReview { get; init; } = string.Empty;

    /// <summary>Records that the notice was shown, and does nothing else.</summary>
    [JsonPropertyName("acknowledge")] public string Acknowledge { get; init; } = string.Empty;

    /// <summary>"Ask me first", or null when the element names no project.</summary>
    [JsonPropertyName("ask_first_action")] public string? AskFirstAction { get; init; }

    /// <summary>Shown when the daemon refuses the switch; null exactly when the button is.</summary>
    [JsonPropertyName("ask_first_failed")] public string? AskFirstFailed { get; init; }

    /// <summary>The payload fields this shell decodes, by wire name.</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "now_heading", "scope", "limit", "no_review", "acknowledge",
        "ask_first_action", "ask_first_failed",
    };
}

/// <summary>
/// One rewording notice as the window draws it: the id
/// <c>acknowledge_arming_rewordings</c> takes, the notice, and the project
/// "Ask me first" switches.
/// </summary>
public sealed record ArmingRewordingCard(ulong? Id, ArmingRewordedNotice Notice, string? AskFirstProjectId = null)
{
    public bool CanAcknowledge => Id.HasValue;

    /// <summary>Whether to draw "Ask me first": the core offered it and the element names a project.</summary>
    public bool CanAskFirst => AskFirstProjectId is not null && Notice.AskFirstAction is not null;
}

/// <summary>One held folder in the Rust's held notice.</summary>
public sealed record GateHeldProjectNotice
{
    /// <summary>For the "Ask me first" button's <c>set_project_mode</c>. Never shown.</summary>
    [JsonPropertyName("project_id")] public string? ProjectId { get; init; }

    [JsonPropertyName("line")] public string Line { get; init; } = string.Empty;

    [JsonPropertyName("ask_first_action")] public string? AskFirstAction { get; init; }

    [JsonPropertyName("ask_first_failed")] public string? AskFirstFailed { get; init; }

    public bool CanAskFirst => ProjectId is not null && AskFirstAction is not null;
}

/// <summary>
/// The notice the Rust assembled for armed folders the automatic-contribution
/// gate is holding. It is never acknowledged: it goes when the hold does.
/// </summary>
public sealed record GateHeldNotice
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;

    [JsonPropertyName("body")] public string Body { get; init; } = string.Empty;

    [JsonPropertyName("reasons")] public List<string> Reasons { get; init; } = new();

    [JsonPropertyName("release")] public string Release { get; init; } = string.Empty;

    [JsonPropertyName("ask_first")] public string AskFirst { get; init; } = string.Empty;

    [JsonPropertyName("projects")] public List<GateHeldProjectNotice> Projects { get; init; } = new();

    /// <summary>The payload fields this shell decodes, by wire name.</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "reasons", "release", "ask_first", "projects",
    };
}

/// <summary>
/// <c>status.arming_rewordings</c> and <c>status.automatic_contribution_held</c>,
/// turned into notices by the ABI (<c>tc_arming_reworded_notice</c>,
/// <c>tc_gate_held_notice</c>). Each element or object goes back to the Rust
/// exactly as the daemon sent it; this shell never chooses a sentence.
/// </summary>
public static class SwitchOnNotices
{
    /// <summary>The health label the daemon sets while the gate holds armed folders.</summary>
    public const string GateHeldLabel = "automatic-contribution-held";

    /// <summary>
    /// The cards for every rewording, worded by <paramref name="word"/> (the
    /// ABI in the app, a fake in tests). An element the ABI returns nothing
    /// for is left out: that is a caught panic, not a notice it declined.
    /// </summary>
    public static IReadOnlyList<ArmingRewordingCard> RewordingCards(
        IReadOnlyList<JsonElement>? rewordings,
        Func<string, string?> word)
    {
        var cards = new List<ArmingRewordingCard>();
        if (rewordings is null)
        {
            return cards;
        }

        foreach (JsonElement element in rewordings)
        {
            if (ParseRewording(word(element.GetRawText())) is not { } notice)
            {
                continue;
            }

            ulong? id = element.ValueKind == JsonValueKind.Object
                && element.TryGetProperty("id", out JsonElement raw)
                && raw.TryGetUInt64(out ulong value)
                ? value
                : null;
            string? project = notice.AskFirstAction is not null
                && element.ValueKind == JsonValueKind.Object
                && element.TryGetProperty("project_id", out JsonElement rawProject)
                && rawProject.ValueKind == JsonValueKind.String
                && rawProject.GetString() is { Length: > 0 } projectId
                ? projectId
                : null;
            cards.Add(new ArmingRewordingCard(id, notice, project));
        }

        return cards;
    }

    /// <summary>The cards for <paramref name="rewordings"/>, worded by the ABI.</summary>
    public static IReadOnlyList<ArmingRewordingCard> RewordingCards(IReadOnlyList<JsonElement>? rewordings) =>
        RewordingCards(rewordings, WordRewording);

    /// <summary>
    /// The held notice for <paramref name="held"/>, worded by
    /// <paramref name="word"/>, or null when nothing is held (the ABI answers
    /// NULL then) or the object is absent.
    /// </summary>
    public static GateHeldNotice? Held(JsonElement? held, Func<string, string?> word) =>
        held is { ValueKind: JsonValueKind.Object } value ? ParseHeld(word(value.GetRawText())) : null;

    /// <summary>The held notice, worded by the ABI.</summary>
    public static GateHeldNotice? Held(JsonElement? held) => Held(held, WordHeld);

    /// <summary>
    /// The <c>set_project_mode</c> params "Ask me first" sends: the same
    /// request Settings sends to set a project to ask first.
    /// </summary>
    public static string AskFirstParams(string projectId) =>
        JsonSerializer.Serialize(new Dictionary<string, string>
        {
            ["project_id"] = projectId,
            ["mode"] = "notify_only",
        });

    private static string? WordRewording(string wireJson) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_arming_reworded_notice(wireJson));

    private static string? WordHeld(string wireJson) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_gate_held_notice(wireJson));

    private static bool Paired(string? action, string? failed) =>
        (action is null) == (failed is null) && action?.Length != 0 && failed?.Length != 0;

    /// <summary>Decode one rewording notice, or null: shown whole or not at all.</summary>
    internal static ArmingRewordedNotice? ParseRewording(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            ArmingRewordedNotice? notice = JsonSerializer.Deserialize<ArmingRewordedNotice>(json);
            if (notice is null)
            {
                return null;
            }

            string[] sentences =
            {
                notice.Title, notice.Body, notice.NowHeading, notice.Scope, notice.Limit,
                notice.NoReview, notice.Acknowledge,
            };
            if (sentences.Any(string.IsNullOrEmpty) || !Paired(notice.AskFirstAction, notice.AskFirstFailed))
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

    /// <summary>Decode the held notice, or null: shown whole or not at all.</summary>
    internal static GateHeldNotice? ParseHeld(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            GateHeldNotice? notice = JsonSerializer.Deserialize<GateHeldNotice>(json);
            if (notice is null || notice.Reasons.Count == 0)
            {
                return null;
            }

            string[] sentences = { notice.Title, notice.Body, notice.Release, notice.AskFirst };
            if (sentences.Concat(notice.Reasons).Any(string.IsNullOrEmpty))
            {
                return null;
            }

            foreach (GateHeldProjectNotice project in notice.Projects)
            {
                if (string.IsNullOrEmpty(project.Line)
                    || !Paired(project.AskFirstAction, project.AskFirstFailed)
                    || (project.AskFirstAction is null) != (project.ProjectId is null))
                {
                    return null;
                }
            }

            return notice;
        }
        catch (JsonException)
        {
            return null;
        }
    }
}
