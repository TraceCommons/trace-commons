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
/// <param name="Label">
/// The category the token names, or empty when the token names none. Only
/// <c>&lt;PRIVATE_*_n&gt;</c> and <c>[REDACTED:{label}]</c> carry one;
/// <c>[REDACTED]</c> and <c>&lt;REDACTED_PRIVATE_KEY&gt;</c> can say that
/// something left and not what.
/// </param>
/// <param name="Ordinal">
/// Which distinct value this is, within its label, or null for a token that
/// carries no index. The numbering is per DISTINCT VALUE, so the same path
/// twice carries the same ordinal, which is what makes the summary's distinct
/// counts mean anything.
///
/// <b>Null, never zero.</b> Only <c>apply_placeholder_regex</c> mints a
/// number, and it is called for exactly two labels; faking a zero for the
/// other three shapes would put a value in the field that the redactor never
/// assigned.
/// </param>
public sealed record RedactionPlaceholder(int Start, int Length, string Label, int? Ordinal)
{
    /// <summary>
    /// The label in the words a contributor reads, e.g. "local path", or
    /// empty for a token that names no category.
    /// </summary>
    public string Display { get; } =
        Label.Replace('_', ' ').ToLower(CultureInfo.InvariantCulture);

    /// <summary>Whether this mark can say WHAT left, rather than only that something did.</summary>
    public bool HasLabel => Display.Length > 0;
}

/// <summary>
/// Finds the typed placeholders in a redacted transcript.
/// </summary>
/// <remarks>
/// <para>
/// <c>DeterministicTraceRedactor</c> does not delete a matched value, it
/// substitutes a token, and those tokens were always in the bytes
/// <c>tc_preview_body</c> returns. Reading them back is what lets the preview
/// say WHERE something was cut, which is more than a category count can.
/// </para>
/// <para>
/// <b>Four token shapes, and the difference matters.</b> Only
/// <c>apply_placeholder_regex</c> mints the numbered
/// <c>&lt;PRIVATE_LABEL_n&gt;</c> form, and it is called for exactly two
/// labels: <c>local_path</c> and <c>private_email</c>. Everything else gets
/// one of three fixed tokens, none of them numbered: <c>[REDACTED]</c> for
/// secrets and sensitive fields, <c>&lt;REDACTED_PRIVATE_KEY&gt;</c> for a PEM
/// key, and <c>[REDACTED:{label}]</c> for tool arguments and privacy-filter
/// findings. A scan that recognised only the numbered form would mark every
/// path and NO SECRET, while the summary panel beside it reported those
/// secrets as removed. Only the labelled shapes can name their own category;
/// the other two say that something left and not what, and
/// <see cref="RedactionPlaceholder.HasLabel"/> is how a caller tells.
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
/// the transcript is drawn with, and stays, because the chunker depends on its
/// single pattern so that a marker is never cut in half. The two cover the
/// same four shapes; only this one recovers which category and which distinct
/// value a token stands for.
/// </para>
/// </remarks>
public static class RedactionPlaceholders
{
    /// <summary>
    /// Every token shape the redactor emits.
    /// </summary>
    /// <remarks>
    /// In the numbered arm, <c>[A-Z0-9_]*[A-Z0-9]</c> forces the label to end
    /// on a non-underscore, so the final <c>_&lt;digits&gt;</c> is the ordinal
    /// and a label that itself ends in digits (<c>SHA256_KEY</c>) cannot steal
    /// it.
    ///
    /// <c>&lt;REDACTED_PRIVATE_KEY&gt;</c> is listed before the numbered arm
    /// only for readability; it cannot match that arm anyway, having no index.
    ///
    /// The <c>[REDACTED...]</c> arm excludes newlines as well as <c>]</c>.
    /// Without that, one unclosed bracket anywhere in a body would let a
    /// "marker" run to the end of the file. It is the same shape
    /// <see cref="TranscriptMarkers"/> uses, deliberately.
    /// </remarks>
    private const string Pattern =
        @"<REDACTED_PRIVATE_KEY>|<PRIVATE_([A-Z0-9_]*[A-Z0-9])_([0-9]+)>|\[REDACTED(?::([^\]\n]*))?\]";

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
                found.Add(new RedactionPlaceholder(
                    match.Index,
                    match.Length,
                    LabelOf(match),
                    OrdinalOf(match)));
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

    /// <summary>
    /// The category a token names: the numbered form's own label, or the one
    /// carried after the colon in <c>[REDACTED:{label}]</c>. Empty for the two
    /// shapes that name none.
    /// </summary>
    private static string LabelOf(Match match) =>
        match.Groups[1].Success ? match.Groups[1].Value
        : match.Groups[3].Success ? match.Groups[3].Value
        : string.Empty;

    /// <summary>
    /// The token's index, or null for a shape that carries none.
    /// </summary>
    /// <remarks>
    /// Null rather than zero, and null rather than throwing: the token is
    /// contributor-adjacent text, so a run of digits long enough to overflow
    /// reads as "no index" rather than taking down the scan of a transcript
    /// somebody is about to consent to.
    /// </remarks>
    private static int? OrdinalOf(Match match) =>
        match.Groups[2].Success
        && int.TryParse(
            match.Groups[2].Value,
            NumberStyles.None,
            CultureInfo.InvariantCulture,
            out int ordinal)
            ? ordinal
            : null;
}
