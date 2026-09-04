using System;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The routing surface's fixed words, read from the Rust rather than kept
/// here.
///
/// Every property is filled from the payload and none has a default: a word
/// this shell invented would be a word the Linux and macOS shells do not
/// print, and <see cref="WordPrivate"/> is a privacy claim, so inventing one
/// is inventing a claim.
///
/// Exactly one word claims privacy and none denies it. Do not derive a "not
/// private" label from any of the others: "Private" is a substring of "Not
/// private", and a surface carrying both is one <c>Contains</c> away from
/// showing the wrong verdict.
/// </summary>
public sealed record RoutingCopy
{
    [JsonPropertyName("tools_heading")] public string ToolsHeading { get; init; } = "";

    /// <summary>The one word on this surface that claims privacy.</summary>
    [JsonPropertyName("word_private")] public string WordPrivate { get; init; } = "";

    /// <summary>
    /// The not-wired word. Deliberately not "Not private" -- see the type
    /// summary.
    /// </summary>
    [JsonPropertyName("word_direct")] public string WordDirect { get; init; } = "";

    [JsonPropertyName("word_unknown")] public string WordUnknown { get; init; } = "";
    [JsonPropertyName("word_not_used")] public string WordNotUsed { get; init; } = "";
    [JsonPropertyName("tool_claude")] public string ToolClaude { get; init; } = "";
    [JsonPropertyName("tool_codex")] public string ToolCodex { get; init; } = "";
    [JsonPropertyName("tool_gemini")] public string ToolGemini { get; init; } = "";
    [JsonPropertyName("tool_cline")] public string ToolCline { get; init; } = "";
    [JsonPropertyName("intro")] public string Intro { get; init; } = "";
    [JsonPropertyName("toggle")] public string Toggle { get; init; } = "";
    [JsonPropertyName("applies_at_once")] public string AppliesAtOnce { get; init; } = "";

    /// <summary>The one action offered when discovery already found the port.</summary>
    [JsonPropertyName("connect")] public string Connect { get; init; } = "";

    /// <summary>The disclosure the port and folder sit behind once it has.</summary>
    [JsonPropertyName("override_title")] public string OverrideTitle { get; init; } = "";

    [JsonPropertyName("look_again")] public string LookAgain { get; init; } = "";
    [JsonPropertyName("port_title")] public string PortTitle { get; init; } = "";
    [JsonPropertyName("port_note")] public string PortNote { get; init; } = "";
    [JsonPropertyName("folder_title")] public string FolderTitle { get; init; } = "";

    /// <summary>
    /// The macOS folder control's action. This shell keeps a text box -- the
    /// permission rule that makes a chooser necessary there does not hold
    /// here -- so it arrives in the payload and is not rendered.
    /// </summary>
    [JsonPropertyName("choose_folder")] public string ChooseFolder { get; init; } = "";
    [JsonPropertyName("folder_note")] public string FolderNote { get; init; } = "";
    [JsonPropertyName("apply")] public string Apply { get; init; } = "";
    [JsonPropertyName("checking")] public string Checking { get; init; } = "";
    [JsonPropertyName("check_unavailable")] public string CheckUnavailable { get; init; } = "";
    [JsonPropertyName("probe_reachable")] public string ProbeReachable { get; init; } = "";
    [JsonPropertyName("state_off")] public string StateOff { get; init; } = "";
    [JsonPropertyName("state_waiting")] public string StateWaiting { get; init; } = "";
    [JsonPropertyName("state_reading")] public string StateReading { get; init; } = "";

    /// <summary>
    /// Declared, and no reader could be built. Carried because the payload
    /// carries it: the three shells read the same set, and a field this one
    /// dropped would be a sentence it could not show.
    /// </summary>
    [JsonPropertyName("state_token_unreadable")]
    public string StateTokenUnreadable { get; init; } = "";

    /// <summary>The four words, in the order the surface uses them.</summary>
    public string[] Words => new[] { WordPrivate, WordDirect, WordUnknown, WordNotUsed };
}
