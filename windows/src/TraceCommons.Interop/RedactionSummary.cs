using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

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
    /// <summary>What a family this build has no words for is described as.</summary>
    /// <remarks>
    /// Neutral, never guessed, and never a reason to hide the row. A missing
    /// entry is expected rather than a bug: the vocabulary is generated.
    /// </remarks>
    public const string UnknownDescription =
        "Removed by a pattern this version has no description for.";

    /// <summary>What a secret found and left in the payload is described as.</summary>
    public const string ResidualDescription =
        "Found by the scan and NOT removed. It is still in what would be sent.";

    private static readonly Dictionary<string, string> Descriptions = new(StringComparer.Ordinal)
    {
        ["local_path"] = "File paths from this machine.",
        ["secret"] =
            "API keys, tokens, private keys, and high-entropy strings found next to "
            + "credential words.",
        ["privacy_filter"] =
            "Names, emails, and other personal details the privacy model found in prose.",
        ["sensitive_field"] =
            "Fields whose name marks them sensitive, like password or authorization.",
        ["tool_sensitive_field"] = "Tool-call arguments whose name marks them sensitive.",
        [RedactionLabels.ResidualPrefix] = ResidualDescription,
    };

    /// <summary>
    /// The two lists the panel draws: what left, and what was found and
    /// stayed.
    /// </summary>
    /// <remarks>
    /// Both ordered by occurrences and then family, so the order is stable
    /// between redraws rather than following whatever order a map iterated
    /// in.
    /// </remarks>
    public static (IReadOnlyList<RedactionSummaryRow> Removed,
                   IReadOnlyList<RedactionSummaryRow> StillPresent) Rows(
        IReadOnlyDictionary<string, int> occurrences,
        IReadOnlyDictionary<string, int> distinct)
    {
        ArgumentNullException.ThrowIfNull(occurrences);
        ArgumentNullException.ThrowIfNull(distinct);

        List<RedactionSummaryRow> rows = occurrences
            .GroupBy(pair => RedactionLabels.Family(pair.Key), StringComparer.Ordinal)
            .Select(family => Row(family.Key, family, distinct))
            .OrderByDescending(row => row.Occurrences)
            .ThenBy(row => row.Family, StringComparer.Ordinal)
            .ToList();

        return (
            rows.Where(row => RedactionLabels.IsRemoval(row.Family)).ToList(),
            rows.Where(row => !RedactionLabels.IsRemoval(row.Family)).ToList());
    }

    /// <summary>
    /// The description for <paramref name="family"/>, falling back to
    /// <see cref="UnknownDescription"/> rather than throwing.
    /// </summary>
    public static string Describe(string family)
    {
        ArgumentNullException.ThrowIfNull(family);
        return Descriptions.TryGetValue(family, out string? description)
            ? description
            : UnknownDescription;
    }

    private static RedactionSummaryRow Row(
        string family,
        IEnumerable<KeyValuePair<string, int>> labels,
        IReadOnlyDictionary<string, int> distinct)
    {
        var members = labels.ToList();

        // The bare family carries no sub-label, so it contributes counts and
        // nothing to the detail line. Sub-labels are rendered in the same
        // words the rest of the surface uses, so "contextual_entropy" and a
        // category name never read as two different kinds of thing.
        IReadOnlyList<string> detail = members
            .Where(pair => pair.Key.Length > family.Length)
            .Select(pair => pair.Key.Substring(family.Length + 1).Replace('_', ' '))
            .OrderBy(sub => sub, StringComparer.Ordinal)
            .ToList();

        return new RedactionSummaryRow(
            family,
            family.Replace('_', ' '),
            Describe(family),
            members.Sum(pair => pair.Value),
            members.Sum(pair => distinct.TryGetValue(pair.Key, out int values) ? values : 0),
            detail);
    }
}
