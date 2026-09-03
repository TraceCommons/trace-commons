using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.RegularExpressions;

namespace TraceCommons.Interop;

/// <summary>
/// One typed placeholder the redactor left where it removed a value.
/// </summary>
/// <param name="Start">UTF-16 index into the body.</param>
/// <param name="Length">Length in UTF-16 units.</param>
/// <param name="Label">The category, upper-cased as the token carries it.</param>
/// <param name="Ordinal">
/// Which distinct value this is, within its label. The numbering is per
/// DISTINCT VALUE, so the same path twice carries the same ordinal -- which is
/// what makes the distinct counts on the summary mean anything.
/// </param>
public sealed record RedactionPlaceholder(int Start, int Length, string Label, int Ordinal)
{
    /// <summary>The label in the words a contributor reads, e.g. "local path".</summary>
    public string Display { get; } =
        Label.Replace('_', ' ').ToLower(CultureInfo.InvariantCulture);
}

/// <summary>
/// Finds the typed placeholders in a redacted transcript.
/// </summary>
/// <remarks>
/// <para>
/// <c>DeterministicTraceRedactor</c> does not delete a matched value. It
/// substitutes a typed placeholder from <c>PlaceholderMap::placeholder_for</c>
/// -- <c>&lt;PRIVATE_LOCAL_PATH_1&gt;</c>, <c>&lt;PRIVATE_SECRET_3&gt;</c> --
/// so those tokens were always in the bytes <c>tc_preview_body</c> returns.
/// Reading them back is what lets the preview say WHERE something was cut,
/// which is more than a category count can.
/// </para>
/// <para>
/// <b>A region with no placeholder is not a region with nothing sensitive in
/// it.</b> The detector scans every leaf; the rewriter reaches only typed
/// fields. Marking makes the app look more thorough than it is, and that is
/// exactly the moment the scrubbing caveat earns its place -- it belongs
/// beside the marks, not at the bottom of the screen.
/// </para>
/// <para>
/// This is the LABEL-AWARE scan. <see cref="TranscriptMarkers"/> is the one
/// the transcript is drawn with, and stays: it also covers the
/// <c>[REDACTED:...]</c> family, and the chunker depends on its single pattern
/// so that a marker is never cut in half. The two agree about
/// <c>&lt;PRIVATE_*&gt;</c> tokens by construction -- this pattern is the
/// stricter of the two -- and only this one recovers which category and which
/// distinct value a token stands for.
/// </para>
/// </remarks>
public static class RedactionPlaceholders
{
    /// <summary>
    /// The token shape. <c>[A-Z0-9_]*[A-Z0-9]</c> forces the label to end on a
    /// non-underscore, so the final <c>_&lt;digits&gt;</c> is the ordinal and
    /// a label that itself ends in digits (<c>SHA256_KEY</c>) cannot steal it.
    /// </summary>
    private const string Pattern = "<PRIVATE_([A-Z0-9_]*[A-Z0-9])_([0-9]+)>";

    /// <summary>
    /// A bound on how long the scan may run.
    ///
    /// The pattern has no nested quantifiers so it cannot backtrack
    /// catastrophically, but the input is attacker-adjacent -- it is whatever
    /// was in someone's session -- and a UI thread is a poor place to find out
    /// otherwise. Matches <see cref="TranscriptMarkers"/>'s own bound.
    /// </summary>
    private static readonly Regex Placeholders = new(
        Pattern,
        RegexOptions.CultureInvariant,
        TimeSpan.FromSeconds(2));

    /// <summary>
    /// Every placeholder in <paramref name="body"/>, in order.
    /// </summary>
    /// <remarks>
    /// Offsets index a C# string, which is UTF-16. The ABI reports UTF-8 byte
    /// offsets elsewhere and <see cref="TcPreview.Search"/> converts them;
    /// this scan runs on the already-converted string, so its offsets need no
    /// conversion and survive text outside the BMP.
    /// </remarks>
    public static IReadOnlyList<RedactionPlaceholder> Scan(string body)
    {
        ArgumentNullException.ThrowIfNull(body);

        var found = new List<RedactionPlaceholder>();
        if (body.Length == 0)
        {
            return found;
        }

        try
        {
            foreach (Match match in Placeholders.Matches(body))
            {
                // The ordinal is bounded by the transcript's own placeholder
                // count, but the token is contributor-adjacent text: a run of
                // digits long enough to overflow parses to nothing rather
                // than throwing, and a token that cannot be read is not a
                // placeholder.
                if (!int.TryParse(
                        match.Groups[2].Value,
                        NumberStyles.None,
                        CultureInfo.InvariantCulture,
                        out int ordinal))
                {
                    continue;
                }

                found.Add(new RedactionPlaceholder(
                    match.Index,
                    match.Length,
                    match.Groups[1].Value,
                    ordinal));
            }
        }
        catch (RegexMatchTimeoutException)
        {
            // Report none rather than half of them, the same trade
            // TranscriptMarkers makes: a partial list would understate what
            // the pipeline did, on a surface whose whole job is not to.
            found.Clear();
        }

        return found;
    }
}
