using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// Each consent scope's title, keyed by wire name, read from the disclosure
/// bundle's <c>consent_scope_titles</c>
/// (<c>tc_contributor_disclosure_copy_json</c>).
/// </summary>
/// <remarks>
/// For a surface that names a scope without a <c>consent_options</c> answer
/// in hand -- the preview sheet's permission rows. Onboarding and Settings
/// read <c>ConsentOption.Title</c>, the same words from the daemon.
/// No shell keeps its own table of them (owner ruling, 2026-10-06): a scope
/// this build has no title for is null from <see cref="Title"/>, and
/// <see cref="PermissionTitles(IEnumerable{string})"/> names it by its wire name.
/// </remarks>
public static class ConsentScopeTitles
{
    private static readonly Lazy<IReadOnlyDictionary<string, string>> Titles = new(Read);

    /// <summary>The core's title for <paramref name="wireName"/>, or null.</summary>
    public static string? Title(string wireName) =>
        Titles.Value.TryGetValue(wireName, out string? title) ? title : null;

    /// <summary>
    /// One permission row per scope this upload asks for, named by the core's
    /// title. A scope with no title, or every scope when the bundle could not
    /// be read, shows its wire name, as GTK does: it is held, so it is never
    /// hidden, and a list shorter than the grant would understate it.
    /// </summary>
    public static IReadOnlyList<string> PermissionTitles(IEnumerable<string> scopes) =>
        PermissionTitles(scopes, Titles.Value);

    internal static IReadOnlyList<string> PermissionTitles(
        IEnumerable<string> scopes, IReadOnlyDictionary<string, string> titles)
    {
        var rows = new List<string>();
        foreach (string scope in scopes)
        {
            rows.Add(titles.TryGetValue(scope, out string? title) ? title : scope);
        }

        return rows;
    }

    private static IReadOnlyDictionary<string, string> Read()
    {
        var titles = new Dictionary<string, string>(StringComparer.Ordinal);
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_contributor_disclosure_copy_json());
        if (string.IsNullOrEmpty(json))
        {
            return titles;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind == JsonValueKind.Object
                && doc.RootElement.TryGetProperty("consent_scope_titles", out JsonElement map)
                && map.ValueKind == JsonValueKind.Object)
            {
                foreach (JsonProperty scope in map.EnumerateObject())
                {
                    if (scope.Value.ValueKind == JsonValueKind.String
                        && scope.Value.GetString() is { Length: > 0 } title)
                    {
                        titles[scope.Name] = title;
                    }
                }
            }
        }
        catch (JsonException)
        {
            titles.Clear();
        }

        return titles;
    }
}
