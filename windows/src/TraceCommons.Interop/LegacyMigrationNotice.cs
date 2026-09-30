using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The notice the Rust assembled after a legacy invite identity moved to the
/// contributor's NEAR AI account: that their contributions now go under that
/// account, and whether their automatic folders were kept.
///
/// <para>
/// Every property is filled from the payload and none is written here.
/// </para>
/// </summary>
public sealed record LegacyMigrationNotice
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;

    [JsonPropertyName("body")] public string Body { get; init; } = string.Empty;

    /// <summary>Whether automatic folders were kept, already chosen by the Rust.</summary>
    [JsonPropertyName("folders")] public string Folders { get; init; } = string.Empty;

    /// <summary>
    /// The button. It records that the notice was shown
    /// (<c>acknowledge_legacy_invite_migration</c>) and does nothing else.
    /// </summary>
    [JsonPropertyName("acknowledge")] public string Acknowledge { get; init; } = string.Empty;

    /// <summary>The payload fields this shell decodes, by wire name.</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "folders", "acknowledge",
    };
}

/// <summary>
/// <c>status.legacy_invite_migration</c>, turned into its notice by the ABI.
///
/// <para>
/// The <c>notice</c> object goes back to the Rust exactly as the daemon sent
/// it (<c>tc_legacy_migration_notice</c>), which reads <c>folders_kept</c>
/// and returns the words. This shell never reads it to choose a sentence.
/// </para>
/// </summary>
public static class LegacyMigrationNotices
{
    /// <summary>
    /// The notice for <paramref name="migration"/>, worded by
    /// <paramref name="word"/> (the ABI in the app, a fake in tests), or null
    /// when there is nothing to show: no field (an older daemon), no notice,
    /// or one the ABI cannot word.
    /// </summary>
    public static LegacyMigrationNotice? Notice(JsonElement? migration, Func<string, string?> word)
    {
        if (migration is not { ValueKind: JsonValueKind.Object } value
            || !value.TryGetProperty("notice", out JsonElement notice)
            || notice.ValueKind != JsonValueKind.Object)
        {
            return null;
        }

        return Parse(word(notice.GetRawText()));
    }

    /// <summary>The notice for <paramref name="migration"/>, worded by the ABI.</summary>
    public static LegacyMigrationNotice? Notice(JsonElement? migration) => Notice(migration, Word);

    private static string? Word(string noticeJson) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_legacy_migration_notice(noticeJson));

    /// <summary>
    /// Decode the notice, or null if it will not parse or a sentence is
    /// empty: shown whole or not at all.
    /// </summary>
    internal static LegacyMigrationNotice? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            LegacyMigrationNotice? notice = JsonSerializer.Deserialize<LegacyMigrationNotice>(json);
            if (notice is null)
            {
                return null;
            }

            foreach (string sentence in new[] { notice.Title, notice.Body, notice.Folders, notice.Acknowledge })
            {
                if (string.IsNullOrEmpty(sentence))
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
