using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// What the account section shows while this app's account needs attention
/// (<c>status.account_state</c> is <c>needs_attention</c>): its account record
/// cannot be read, or the address it was set up with is refused. Signing in
/// again does not fix it; setting up again does.
///
/// <para>
/// Every property is filled from the payload and none is written here. A
/// status in the account section, never a banner or notification.
/// </para>
/// </summary>
public sealed record AccountAttention
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;

    /// <summary>The reason's sentence, or null for a reason this build does not know.</summary>
    [JsonPropertyName("body")] public string? Body { get; init; }

    /// <summary>Present only while a NEAR AI login is stored.</summary>
    [JsonPropertyName("private_ai")] public string? PrivateAi { get; init; }

    /// <summary>
    /// The button: <c>unenroll</c>, then the first run.
    /// </summary>
    [JsonPropertyName("action")] public string Action { get; init; } = string.Empty;

    /// <summary>The payload fields this shell decodes, by wire name.</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "private_ai", "action",
    };
}

/// <summary>
/// <c>status</c>, turned into the account section's attention block by the
/// ABI (<c>tc_account_attention_copy_json</c>), which reads
/// <c>account_state</c>, <c>account_state_reason</c> and
/// <c>near_ai_login_unconfirmed</c> and returns the words. This shell never
/// reads them to choose a sentence.
/// </summary>
public static class AccountAttentions
{
    /// <summary>
    /// The block for <paramref name="statusJson"/>, worded by
    /// <paramref name="word"/> (the ABI in the app, a fake in tests), or null
    /// when there is nothing to show: no status, an account that is fine, a
    /// daemon too old to say, or a payload the ABI cannot word.
    /// </summary>
    public static AccountAttention? For(string? statusJson, Func<string, string?> word)
    {
        if (string.IsNullOrWhiteSpace(statusJson))
        {
            return null;
        }

        return Parse(word(statusJson));
    }

    /// <summary>The block for <paramref name="statusJson"/>, worded by the ABI.</summary>
    public static AccountAttention? For(string? statusJson) => For(statusJson, Word);

    private static string? Word(string statusJson) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_account_attention_copy_json(statusJson));

    /// <summary>
    /// Decode the block, or null if it will not parse or its title or button
    /// is empty: shown whole or not at all. A null body or Private AI line
    /// is the Rust's choice, not a gap.
    /// </summary>
    internal static AccountAttention? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            AccountAttention? attention = JsonSerializer.Deserialize<AccountAttention>(json);
            if (attention is null
                || string.IsNullOrEmpty(attention.Title)
                || string.IsNullOrEmpty(attention.Action))
            {
                return null;
            }

            return attention;
        }
        catch (JsonException)
        {
            return null;
        }
    }
}
