using System.Collections.Generic;

namespace TraceCommons.Interop.FirstRun;

/// <summary>What the reader's fixed region holds (<c>TermsLoader.State</c>).</summary>
public enum TermsRegion
{
    Loading,
    Failed,
    Loaded,
}

/// <summary>The reader's rows, top to bottom.</summary>
public enum TermsReaderPart
{
    /// <summary>The fixed-height region: the document, or the loading or failed state.</summary>
    Region,

    /// <summary>The document is a draft, not yet in effect.</summary>
    NotInEffect,

    /// <summary>The accept checkbox.</summary>
    Accept,

    /// <summary>"Scroll to the end to accept", while the checkbox is locked.</summary>
    ScrollHint,

    /// <summary>The Terms and Privacy links.</summary>
    Links,
}

/// <summary>
/// The Mac's terms reader (macos/Sources/TraceCommonsApp/Views/Terms/TermsReader.swift),
/// its measures and the rows it draws in each state, so the first run's
/// Terms step keeps the Mac's shape: the region holds its height whatever it
/// shows, so the step does not move as the document arrives.
/// </summary>
public static class TermsReaderLayout
{
    /// <summary>The region's height and the column's widest (<c>TermsReaderLayout</c>, :54-58).</summary>
    public const double Viewport = 260, Width = 420;

    /// <summary>Between rows (s4), under the checkbox (s2), round the document (s8), between its blocks (s6).</summary>
    public const double Spacing = 8, HintSpacing = 4, DocumentPadding = 16, BlockSpacing = 12;

    /// <summary>The region's state from the fetch: loading until it answers, failed with a label.</summary>
    public static TermsRegion Region(bool loaded, string? failureLabel) =>
        loaded ? TermsRegion.Loaded : failureLabel is null ? TermsRegion.Loading : TermsRegion.Failed;

    /// <summary>
    /// The rows (:93-118, :215-240): the region; with the document loaded,
    /// the not-in-effect line for a draft, the checkbox, and the scroll hint
    /// while it is locked; then the links, in every state.
    /// </summary>
    public static IReadOnlyList<TermsReaderPart> Parts(TermsRegion region, bool inEffect, bool unlocked)
    {
        var parts = new List<TermsReaderPart> { TermsReaderPart.Region };
        if (region == TermsRegion.Loaded)
        {
            if (!inEffect)
            {
                parts.Add(TermsReaderPart.NotInEffect);
            }

            parts.Add(TermsReaderPart.Accept);
            if (!unlocked)
            {
                parts.Add(TermsReaderPart.ScrollHint);
            }
        }

        parts.Add(TermsReaderPart.Links);
        return parts;
    }
}
