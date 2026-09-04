using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop;

/// <summary>
/// Reading the daemon's redaction count map, which does not mean what its
/// heading says it means.
/// </summary>
/// <remarks>
/// <para>
/// <c>DeterministicTraceRedactor</c> sets <c>redaction_counts</c> to the WHOLE
/// redaction report. Most of that report is what you would expect -- one entry
/// per pattern that fired, counting values it took out -- but it also carries
/// <c>residual_secret_at:{path}</c>, which <c>note_residual_secret_location</c>
/// increments when a secret was <b>detected and NOT removed</b>: a credential
/// inside a correction the contributor wrote, which is preserved on purpose, or
/// a field the typed redaction traversal never visits, which is a real gap.
/// </para>
/// <para>
/// Every shell renders that map under the heading "Removed by pattern", so a
/// session carrying a surviving secret has been reporting it as a thing that
/// was taken out -- the exact opposite of what happened, on the one screen
/// where somebody is deciding whether to send it.
/// </para>
/// <para>
/// Both halves of the fix matter and neither is optional: a survivor must not
/// be counted as a removal, and a survivor must still be SHOWN. Filtering one
/// out of the figure and saying nothing else would trade a wrong statement for
/// silence about a secret that is still in the payload, which on a consent
/// surface is not an improvement.
/// </para>
/// </remarks>
public static class RedactionLabels
{
    /// <summary>The label family marking a secret found and left in place.</summary>
    public const string ResidualPrefix = "residual_secret_at";

    /// <summary>The part of a label before its first <c>:</c>.</summary>
    /// <remarks>
    /// The count vocabulary is namespaced and OPEN -- <c>secret:{pattern_name}</c>,
    /// <c>privacy_filter:{label}</c> and <c>tool_sensitive_field:{action}</c> are
    /// generated at redaction time -- so nothing here may assume a closed set of
    /// labels. Families are the only stable thing to reason about.
    /// </remarks>
    public static string Family(string label)
    {
        if (label is null)
        {
            return string.Empty;
        }
        int colon = label.IndexOf(':', StringComparison.Ordinal);
        return colon < 0 ? label : label.Substring(0, colon);
    }

    /// <summary>Whether a label counts something that actually left the payload.</summary>
    public static bool IsRemoval(string label)
        => !string.Equals(Family(label), ResidualPrefix, StringComparison.Ordinal);

    /// <summary>The counts for things that were genuinely removed.</summary>
    public static IReadOnlyDictionary<string, int> Removals(
        IReadOnlyDictionary<string, int> counts)
        => counts
            .Where(pair => IsRemoval(pair.Key))
            .ToDictionary(pair => pair.Key, pair => pair.Value, StringComparer.Ordinal);

    /// <summary>Total occurrences removed. Never includes survivors.</summary>
    public static int RemovedTotal(IReadOnlyDictionary<string, int> counts)
        => counts.Where(pair => IsRemoval(pair.Key)).Sum(pair => pair.Value);

    /// <summary>How many places a secret was found and left in what would be
    /// sent. Sites, not secrets: one site can hold more than one value.</summary>
    public static int SurvivorTotal(IReadOnlyDictionary<string, int> counts)
        => counts.Where(pair => !IsRemoval(pair.Key)).Sum(pair => pair.Value);

    /// <summary>
    /// Where secrets were found and left in the payload, ordered for a stable
    /// rendering.
    /// </summary>
    /// <remarks>
    /// The sites are schema-shaped identifiers -- <c>events.3.correction</c>,
    /// not a filesystem path and not transcript text. The redactor guarantees
    /// that where these labels are minted, and it is what makes them safe to
    /// show.
    /// </remarks>
    public static IReadOnlyList<string> SurvivorSites(IReadOnlyDictionary<string, int> counts)
        => counts
            .Where(pair => !IsRemoval(pair.Key))
            .Select(pair => pair.Key.StartsWith(ResidualPrefix + ":", StringComparison.Ordinal)
                ? pair.Key.Substring(ResidualPrefix.Length + 1)
                : string.Empty)
            .Where(site => site.Length > 0)
            .OrderBy(site => site, StringComparer.Ordinal)
            .ToList();

    /// <summary>
    /// The line shown when a session carries survivors, in the attention tone.
    /// Empty when there are none.
    /// </summary>
    /// <remarks>
    /// Never names a number of secrets. The count is of detection SITES, and
    /// one site can hold more than one value, so "2 secrets" would understate
    /// what survived. The plural says "found in N places" instead, which is
    /// what the number actually counts; the singular drops it entirely.
    /// </remarks>
    public static string SurvivorLine(IReadOnlyDictionary<string, int> counts)
    {
        int total = SurvivorTotal(counts);
        if (total == 0)
        {
            return string.Empty;
        }
        string head = total == 1
            ? "A secret found here is still in what would be sent"
            : $"Secrets found in {total} places are still in what would be sent";
        IReadOnlyList<string> sites = SurvivorSites(counts);
        return sites.Count == 0 ? head : $"{head} ({string.Join(", ", sites)})";
    }
}
