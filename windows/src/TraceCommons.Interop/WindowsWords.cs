using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The Windows app's own words where it cannot say what the Mac says
/// (<c>preview_copy::windows_words_copy</c>): Settings' credit-arrival switch
/// names the taskbar and tray, not the Dock and menu bar. Owner-approved
/// 2026-11-19. Empty when the core could not produce them, so the card is
/// left out rather than showing words typed here.
/// </summary>
public static class WindowsWords
{
    private static readonly Lazy<IReadOnlyDictionary<string, string>> Words =
        new(() => Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_windows_words_copy_json())));

    /// <summary>Settings' credit-arrival switch.</summary>
    public static string CreditIconAnimationToggle => Word("credit_icon_animation_toggle");

    /// <summary>The line under that switch.</summary>
    public static string CreditIconAnimationCaption => Word("credit_icon_animation_caption");

    private static string Word(string key) => Words.Value.TryGetValue(key, out string? word) ? word : string.Empty;

    /// <summary>The table's string fields; nothing for anything but a JSON object.</summary>
    internal static IReadOnlyDictionary<string, string> Parse(string? json)
    {
        var words = new Dictionary<string, string>(StringComparer.Ordinal);
        if (string.IsNullOrEmpty(json))
        {
            return words;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object)
            {
                return words;
            }

            foreach (JsonProperty field in doc.RootElement.EnumerateObject())
            {
                if (field.Value.ValueKind == JsonValueKind.String && field.Value.GetString() is { } word)
                {
                    words[field.Name] = word;
                }
            }
        }
        catch (JsonException)
        {
            words.Clear();
        }

        return words;
    }
}
