using System;
using System.Collections.Generic;
using System.IO;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// The offer to move a legacy invite identity to the contributor's NEAR AI
/// account, in the Rust's words (<c>tc_legacy_migration_offer_json</c>).
///
/// <para>
/// Every property is filled from the payload and none is written here.
/// </para>
/// </summary>
public sealed record LegacyMigrationOfferCopy
{
    [JsonPropertyName("title")] public string Title { get; init; } = string.Empty;

    [JsonPropertyName("body")] public string Body { get; init; } = string.Empty;

    /// <summary>The button that starts the move.</summary>
    [JsonPropertyName("action")] public string Action { get; init; } = string.Empty;

    /// <summary>Shown while the move runs.</summary>
    [JsonPropertyName("working")] public string Working { get; init; } = string.Empty;

    /// <summary>Asked only after <c>legacy_migration_invite_needed</c>.</summary>
    [JsonPropertyName("invite_prompt")] public string InvitePrompt { get; init; } = string.Empty;

    /// <summary>The move could not be started at all; nothing ran.</summary>
    [JsonPropertyName("start_failed")] public string StartFailed { get; init; } = string.Empty;

    /// <summary>The offer card's dismiss. Not shown in Settings.</summary>
    [JsonPropertyName("dismiss")] public string Dismiss { get; init; } = string.Empty;

    /// <summary>Beside the dismiss: the move is still in Settings.</summary>
    [JsonPropertyName("later")] public string Later { get; init; } = string.Empty;

    /// <summary>While the passkey step is open in the browser.</summary>
    [JsonPropertyName("passkey_waiting")] public string PasskeyWaiting { get; init; } = string.Empty;

    /// <summary>The passkey step ended without finishing.</summary>
    [JsonPropertyName("passkey_failed")] public string PasskeyFailed { get; init; } = string.Empty;

    /// <summary>Stops waiting on the passkey step.</summary>
    [JsonPropertyName("passkey_cancel")] public string PasskeyCancel { get; init; } = string.Empty;

    /// <summary>The accessible name of <see cref="Action"/> on the offer card.</summary>
    [JsonPropertyName("action_name_notice")] public string ActionNameNotice { get; init; } = string.Empty;

    /// <summary>The accessible name of <see cref="Action"/> in Settings.</summary>
    [JsonPropertyName("action_name_settings")] public string ActionNameSettings { get; init; } = string.Empty;

    /// <summary>The payload fields this shell decodes, by wire name.</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "action", "working", "invite_prompt", "start_failed",
        "dismiss", "later", "passkey_waiting", "passkey_failed", "passkey_cancel",
        "action_name_notice", "action_name_settings",
    };
}

/// <summary>
/// The legacy invite migration offer: whether the daemon offers it, its
/// words, the labels the move is refused with, and the local marker for a
/// dismissed card.
/// </summary>
public static class LegacyMigrationOffers
{
    /// <summary>The refusal that means "run the passkey step, then try again".</summary>
    public const string PasskeyNeededLabel = "legacy_migration_no_near_ai_session";

    /// <summary>The refusal that means "ask the contributor for their invite".</summary>
    public const string InviteNeededLabel = "legacy_migration_invite_needed";

    /// <summary>The offer's words from the ABI, or null if they did not arrive whole.</summary>
    public static LegacyMigrationOfferCopy? Copy() =>
        Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_legacy_migration_offer_json()));

    /// <summary>
    /// Decode the offer, or null if it will not parse or a sentence is empty:
    /// shown whole or not at all.
    /// </summary>
    internal static LegacyMigrationOfferCopy? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            LegacyMigrationOfferCopy? copy = JsonSerializer.Deserialize<LegacyMigrationOfferCopy>(json);
            if (copy is null)
            {
                return null;
            }

            foreach (string sentence in new[]
            {
                copy.Title, copy.Body, copy.Action, copy.Working, copy.InvitePrompt, copy.StartFailed,
                copy.Dismiss, copy.Later, copy.PasskeyWaiting, copy.PasskeyFailed, copy.PasskeyCancel,
                copy.ActionNameNotice, copy.ActionNameSettings,
            })
            {
                if (string.IsNullOrEmpty(sentence))
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
    /// <c>status.legacy_invite_migration.offered</c>. False for no field (an
    /// older daemon), and for anything that is not literally true.
    /// </summary>
    public static bool IsOffered(JsonElement? migration) =>
        migration is { ValueKind: JsonValueKind.Object } value
        && value.TryGetProperty("offered", out JsonElement offered)
        && offered.ValueKind == JsonValueKind.True;

    /// <summary>The core's sentence for a refused move, from the IPC error's label.</summary>
    public static string? RefusalLine(string label) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_legacy_migration_refusal_text(label));

    /// <summary>The <c>legacy_invite_migrate</c> body. The invite is sent only when there is one.</summary>
    public static string MigrateParams(string? invite) =>
        string.IsNullOrWhiteSpace(invite)
            ? "{}"
            : JsonSerializer.Serialize(new Dictionary<string, string> { ["invite"] = invite.Trim() });

    /// <summary>
    /// The lowercase hex SHA-256 of the tenant id, or null with no tenant. The
    /// marker for a dismissed card is keyed by this, so a different invite
    /// identity enrolled later is offered the move afresh and the tenant id
    /// itself is never written down.
    /// </summary>
    public static string? TenantDigest(string? tenantId)
    {
        if (string.IsNullOrEmpty(tenantId))
        {
            return null;
        }

        return Convert.ToHexString(SHA256.HashData(Encoding.UTF8.GetBytes(tenantId))).ToLowerInvariant();
    }
}

/// <summary>
/// Which tenants' offer cards were dismissed on this device, by digest.
/// A local marker: no daemon record fits, and the card is this shell's alone.
/// </summary>
/// <remarks>
/// An unreadable file answers "not dismissed". The cost of showing a card
/// again is a click; the cost of hiding one for good is a contributor who
/// never learns the move exists outside Settings.
/// </remarks>
public sealed class LegacyMigrationDismissals
{
    private readonly string _path;

    public LegacyMigrationDismissals(string path)
    {
        _path = path ?? throw new ArgumentNullException(nameof(path));
    }

    public static LegacyMigrationDismissals Default() =>
        new(Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "TraceCommons",
            "legacy-migration-dismissed.json"));

    public bool IsDismissed(string? tenantId) =>
        LegacyMigrationOffers.TenantDigest(tenantId) is { } digest && Read().Contains(digest);

    public void Dismiss(string? tenantId)
    {
        if (LegacyMigrationOffers.TenantDigest(tenantId) is not { } digest)
        {
            return;
        }

        List<string> digests = Read();
        if (digests.Contains(digest))
        {
            return;
        }

        digests.Add(digest);
        try
        {
            string? directory = Path.GetDirectoryName(_path);
            if (!string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }

            File.WriteAllText(_path, JsonSerializer.Serialize(digests));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // The card is put away for this run; only the memory is lost.
        }
    }

    private List<string> Read()
    {
        try
        {
            return File.Exists(_path)
                ? JsonSerializer.Deserialize<List<string>>(File.ReadAllText(_path)) ?? new List<string>()
                : new List<string>();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException)
        {
            return new List<string>();
        }
    }
}
