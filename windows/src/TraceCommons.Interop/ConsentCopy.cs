using System;
using System.Collections.Generic;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The consent surface's fixed sentences, read from the Rust rather than
/// kept here.
///
/// <para>
/// Every property is filled from the payload and none has a default worth
/// rendering: a sentence this shell invented would be a sentence the Linux
/// and macOS shells do not print, and <see cref="GateStatement"/> is the
/// claim a contributor reads immediately above an irreversible button, so
/// inventing one is inventing a claim.
/// </para>
/// </summary>
public sealed record ConsentCopy
{
    /// <summary>
    /// The claim that replaced the acknowledgement checkbox. Both halves of
    /// what the tick used to assert: scrubbing is pattern-based and may have
    /// missed something, and nothing here can tell whether anyone looked.
    /// </summary>
    [JsonPropertyName("gate_statement")] public string GateStatement { get; init; } = string.Empty;

    /// <summary>The tooltip on an armed Contribute.</summary>
    [JsonPropertyName("ready_help")] public string ReadyHelp { get; init; } = string.Empty;

    /// <summary>
    /// The tooltip on a Contribute with nothing to bind to. Never chosen
    /// here: <see cref="ConsentSurface.GateHelp"/> asks the ABI which of the
    /// two applies, because a branch kept in three shells drifts the same
    /// way words do.
    /// </summary>
    [JsonPropertyName("not_pinned_help")] public string NotPinnedHelp { get; init; } = string.Empty;

    /// <summary>
    /// What runs on every automatically contributed session, for the Flow 1
    /// grant screen. Not rendered yet; decoded so this shell has it before
    /// that screen exists, rather than writing its own.
    /// </summary>
    [JsonPropertyName("auto_scrub_scope")] public string AutoScrubScope { get; init; } = string.Empty;

    /// <summary>The limit of both halves of the scrubbing.</summary>
    [JsonPropertyName("auto_scrub_limit")] public string AutoScrubLimit { get; init; } = string.Empty;

    /// <summary>
    /// That no one reviews a session before it is sent. The sentence the
    /// product will most want to soften; this shell never rewrites it.
    /// </summary>
    [JsonPropertyName("auto_no_review")] public string AutoNoReview { get; init; } = string.Empty;

    /// <summary>Every sentence, for the refuse-on-any-empty-field check.</summary>
    public string[] Sentences => new[]
    {
        GateStatement, ReadyHelp, NotPinnedHelp, AutoScrubScope, AutoScrubLimit, AutoNoReview,
    };

    /// <summary>
    /// The payload fields this shell decodes, by wire name.
    ///
    /// <para>
    /// Compared against the live export by
    /// <c>TheExportedFieldsAreExactlyTheOnesThisShellConsumes</c>. A field
    /// added in Rust and not added here is a sentence the other two shells
    /// show and this one does not, and no round-trip test can see that.
    /// </para>
    /// </summary>
    public static IReadOnlyList<string> ConsumedFields { get; } =
        new[]
        {
            "gate_statement", "ready_help", "not_pinned_help",
            "auto_scrub_scope", "auto_scrub_limit", "auto_no_review",
        };
}
