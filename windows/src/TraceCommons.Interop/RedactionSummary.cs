using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// One category on the preview's scrubbing panel.
/// </summary>
/// <param name="Family">The label family, e.g. <c>secret</c>.</param>
/// <param name="Display">That family in the words a contributor reads.</param>
/// <param name="Description">
/// What the category IS. The panel's actual value to a reader who has never
/// seen these words.
/// </param>
/// <param name="Occurrences">How many times patterns in this family fired.</param>
/// <param name="Distinct">
/// How many distinct values that covered, or 0 when the daemon reported none.
/// </param>
/// <param name="Detail">
/// The sub-labels that rolled up into this family, e.g.
/// <c>contextual entropy</c>, or for a survivor the schema paths it was found
/// at. Empty for a bare family with no sub-labels.
/// </param>
public sealed record RedactionSummaryRow(
    string Family,
    string Display,
    string Description,
    int Occurrences,
    int Distinct,
    IReadOnlyList<string> Detail)
{
    /// <summary>
    /// The row's figure: <c>185 (12 distinct)</c>, or <c>185</c> when there is
    /// no second number worth printing.
    /// </summary>
    /// <remarks>
    /// The distinct figure is omitted on exactly the terms
    /// <see cref="RedactionLabels.Line"/> omits it, so the panel and the card
    /// cannot come to disagree: absent when the daemon reported none, and
    /// absent when it equals or exceeds the occurrence count, where it either
    /// says the same thing twice or cannot be true.
    /// </remarks>
    public string CountText { get; } =
        Distinct > 0 && Distinct < Occurrences
            ? string.Format(
                CultureInfo.CurrentCulture,
                "{0} ({1} distinct)",
                Occurrences,
                Distinct)
            : Occurrences.ToString(CultureInfo.CurrentCulture);

    /// <summary>The sub-labels on one line, or empty when there are none.</summary>
    public string DetailText { get; } = string.Join(", ", Detail);

    /// <summary>Whether this row has a detail line to draw at all.</summary>
    public bool HasDetail => DetailText.Length > 0;
}

/// <summary>
/// The preview's "what scrubbing removed, and what it left in" panel.
/// </summary>
/// <remarks>
/// <para>
/// Marking placeholders in the transcript answers <i>where</i>. It does not
/// answer "so I can right away see what doesn't go", because collecting the
/// marks means scrolling the whole transcript. This is the surface that
/// answers it: one row per category, what that category is, how many times it
/// fired, and how many distinct values that covered. <b>No matched text,
/// ever</b> -- the value is gone by construction and the row says what KIND of
/// thing left, not what it was.
/// </para>
/// <para>
/// <b>The label vocabulary is open and namespaced, and this is built for
/// that.</b> The redactor emits <c>local_path</c> and <c>secret</c>, but also
/// <c>secret:{pattern_name}</c>, <c>privacy_filter:{label}</c>,
/// <c>tool_sensitive_field:{action}</c> and
/// <c>residual_secret_at:{schema_path}</c> -- the last three generated, so no
/// shell can hold a complete table of them. Hence: group by family; give an
/// unrecognised family a neutral description rather than a guessed one; and
/// never drop one. Hiding a category because this build has no words for it
/// would understate what happened, which is the one direction this panel must
/// not fail in.
/// </para>
/// <para>
/// Sub-labels are safe to render. They are schema-shaped identifiers by
/// construction -- <c>log_residual_secret_locations</c> depends on that same
/// property -- and never contributor strings.
/// </para>
/// <para>
/// <b><c>residual_secret_at</c> goes in the second list, never the first.</b>
/// It counts a secret that was DETECTED AND NOT REMOVED, it arrives in the
/// same map as every genuine removal, and this panel renders under a heading
/// that says "removed". This is the first surface with room to say two
/// different things; see <see cref="RedactionLabels"/> for the defect and the
/// narrower half of the same fix on the card.
/// </para>
/// </remarks>
public static class RedactionSummary
{
    /// <summary>
    /// The two lists the panel draws, what left and what was found and
    /// stayed, as the core grouped, split, ordered and described them
    /// (<c>tc_redaction_summary_json</c>, <c>redaction_summary::summary_copy</c>;
    /// the Mac's <c>RedactionSummary.rows(fromJSON:)</c>, RedactionSummary.swift:52-80).
    /// </summary>
    /// <remarks>
    /// Null when the core's answer is unreadable, or the map is: that is not
    /// "no rows", and the panel then says nothing rather than claim nothing
    /// matched. This shell decides none of the grouping any more; the table of
    /// descriptions it once transcribed is the core's alone.
    /// </remarks>
    public static (IReadOnlyList<RedactionSummaryRow> Removed,
                   IReadOnlyList<RedactionSummaryRow> StillPresent)? Rows(
        IReadOnlyDictionary<string, int>? occurrences,
        IReadOnlyDictionary<string, int>? distinct)
    {
        if (occurrences is null)
        {
            return null;
        }

        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_redaction_summary_json(
            JsonSerializer.Serialize(occurrences),
            distinct is null ? null : JsonSerializer.Serialize(distinct)));
        return Parse(json);
    }

    /// <summary>The core's answer, decoded; null unless both lists parse.</summary>
    public static (IReadOnlyList<RedactionSummaryRow> Removed,
                   IReadOnlyList<RedactionSummaryRow> StillPresent)? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object
                || List(doc.RootElement, "removed") is not { } removed
                || List(doc.RootElement, "still_present") is not { } still)
            {
                return null;
            }

            return (removed, still);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    private static IReadOnlyList<RedactionSummaryRow>? List(JsonElement root, string key)
    {
        if (!root.TryGetProperty(key, out JsonElement array) || array.ValueKind != JsonValueKind.Array)
        {
            return null;
        }

        var rows = new List<RedactionSummaryRow>();
        foreach (JsonElement row in array.EnumerateArray())
        {
            if (SessionDetail.Str(row, "family") is not { } family
                || SessionDetail.Str(row, "display") is not { } display
                || SessionDetail.Str(row, "description") is not { } description
                || !row.TryGetProperty("occurrences", out JsonElement occurrences)
                || !occurrences.TryGetInt32(out int count)
                || !row.TryGetProperty("distinct", out JsonElement distinctElement)
                || !distinctElement.TryGetInt32(out int distinct)
                || !row.TryGetProperty("detail", out JsonElement detail)
                || detail.ValueKind != JsonValueKind.Array)
            {
                return null;
            }

            var labels = new List<string>();
            foreach (JsonElement label in detail.EnumerateArray())
            {
                if (label.ValueKind != JsonValueKind.String)
                {
                    return null;
                }

                labels.Add(label.GetString()!);
            }

            rows.Add(new RedactionSummaryRow(family, display, description, count, distinct, labels));
        }

        return rows;
    }
}
