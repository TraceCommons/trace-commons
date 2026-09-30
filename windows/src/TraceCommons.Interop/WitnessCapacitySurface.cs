using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The notice for approved sessions held because the privacy witness is
/// busy, as the Rust assembled it (<c>tc_witness_capacity_notice</c>).
/// Every property is filled from the payload and none is written in C#.
/// </summary>
public sealed record WitnessCapacityNotice
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;

    /// <summary>
    /// Why the sessions wait, that nothing is sent meanwhile, and that
    /// nothing was lost. Counted by the Rust.
    /// </summary>
    [JsonPropertyName("body")] public string Body { get; init; } = string.Empty;

    /// <summary>The label beside the next retry time, rendered here in local time.</summary>
    [JsonPropertyName("next_check")] public string NextCheck { get; init; } = string.Empty;

    /// <summary>
    /// The payload fields this shell decodes, by wire name. Compared against
    /// the live export by the interop tests, so a field added in Rust fails
    /// here until somebody decides what this shell does with it.
    /// </summary>
    public static IReadOnlyList<string> ConsumedFields { get; } = new[] { "title", "body", "next_check" };
}

/// <summary>
/// The witness capacity notice, across the C ABI.
/// </summary>
/// <remarks>
/// Nothing in this file is a word. The count goes to the Rust, which words
/// and pluralises the notice; this shell decodes it whole or not at all.
/// </remarks>
public static class WitnessCapacitySurface
{
    /// <summary>
    /// The notice for this status's waiting sessions, or null when nothing is
    /// waiting, the call failed, or the payload will not parse.
    /// </summary>
    public static WitnessCapacityNotice? Notice(WitnessCapacity? capacity)
    {
        if (capacity is null || !capacity.Waiting)
        {
            return null;
        }

        return Parse(NativeMethods.TakeOwnedString(
            NativeMethods.tc_witness_capacity_notice(capacity.WireJson)));
    }

    /// <summary>
    /// The payload half of <see cref="Notice"/>, split out so it is testable
    /// without the cdylib.
    /// </summary>
    internal static WitnessCapacityNotice? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            WitnessCapacityNotice? notice = JsonSerializer.Deserialize<WitnessCapacityNotice>(json);
            if (notice is null
                || string.IsNullOrEmpty(notice.Title)
                || string.IsNullOrEmpty(notice.Body)
                || string.IsNullOrEmpty(notice.NextCheck))
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
