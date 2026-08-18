using System;
using System.Collections.Generic;

namespace TraceCommons.Interop;

/// <summary>
/// Turns search hits into the short excerpts the Search tab shows.
///
/// Search is the highest-value affordance in the product: someone under an NDA
/// types a client name and gets certainty in five seconds without reading 148
/// turns. A bare match count does not give them that -- "3 matches" with no
/// visible context is the one answer this tab must never leave them holding.
/// </summary>
public static class SearchContexts
{
    /// <summary>Characters of context kept either side of a hit.</summary>
    private const int Window = 120;

    /// <summary>
    /// How many excerpts are built at most. A common word in a large
    /// transcript matches thousands of times, and a list that long answers
    /// nothing while costing everything to lay out. The count shown to the
    /// contributor is always the full one.
    /// </summary>
    public const int MaxExcerpts = 20;

    /// <summary>
    /// Builds one excerpt per hit, in order, each with an ellipsis on the
    /// sides that were cut.
    /// </summary>
    /// <param name="body">
    /// The redacted transcript. This is trace content: the excerpts are for
    /// display only and must never be logged.
    /// </param>
    /// <param name="needle">The term searched for.</param>
    /// <param name="offsets">
    /// UTF-16 indices, as <see cref="TcPreview.Search"/> returns them. Raw
    /// ABI byte offsets would land in the wrong place; the conversion has
    /// already happened by the time they reach here.
    /// </param>
    public static IReadOnlyList<string> Build(
        string body,
        string needle,
        IReadOnlyList<int> offsets)
    {
        ArgumentNullException.ThrowIfNull(body);
        ArgumentNullException.ThrowIfNull(needle);
        ArgumentNullException.ThrowIfNull(offsets);

        var excerpts = new List<string>();
        if (needle.Length == 0)
        {
            return excerpts;
        }

        for (int i = 0; i < offsets.Count && excerpts.Count < MaxExcerpts; i++)
        {
            int offset = offsets[i];
            if (offset < 0 || offset > body.Length)
            {
                continue;
            }

            int start = SafeStart(body, Math.Max(0, offset - Window));
            int end = SafeEnd(body, Math.Min(body.Length, offset + needle.Length + Window));
            if (start >= end)
            {
                continue;
            }

            // Newlines collapse to spaces so an excerpt stays one line: these
            // are cut at arbitrary points, and a fragment of indented code
            // wrapped over six lines is harder to scan than the same text in
            // a row.
            string text = body[start..end].Replace('\n', ' ').Replace('\r', ' ');

            excerpts.Add(
                (start > 0 ? "…" : string.Empty)
                + text
                + (end < body.Length ? "…" : string.Empty));
        }

        return excerpts;
    }

    /// <summary>
    /// Nudges a cut point off the second half of a surrogate pair.
    ///
    /// A window boundary lands wherever the arithmetic puts it, and half a
    /// surrogate pair renders as a replacement character -- in an excerpt
    /// whose whole job is to show someone exactly what is in their transcript.
    /// </summary>
    private static int SafeStart(string body, int index) =>
        index > 0 && index < body.Length && char.IsLowSurrogate(body[index])
            ? index - 1
            : index;

    private static int SafeEnd(string body, int index) =>
        index > 0 && index < body.Length && char.IsLowSurrogate(body[index])
            ? index + 1
            : index;
}
