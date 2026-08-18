using System;
using System.Collections.Generic;
using System.Text.RegularExpressions;

namespace TraceCommons.Interop;

/// <summary>One run of the transcript, and whether it is a redaction marker.</summary>
/// <param name="Start">UTF-16 index into the transcript.</param>
/// <param name="Length">Length in UTF-16 units.</param>
/// <param name="IsMarker">True for a marker the redaction pipeline left behind.</param>
public readonly record struct TranscriptRun(int Start, int Length, bool IsMarker);

/// <summary>
/// Splits a redacted transcript into plain runs and marker runs, so a shell
/// can draw the markers as chips.
///
/// Redactions stay VISIBLE as chips rather than becoming deletions, which is
/// the point: a hole tells a contributor nothing, a chip tells them the
/// pipeline was standing right there. The macOS sheet does this with an
/// AttributedString and the Linux one with text tags; this returns the runs
/// and lets the shell draw them, so the scan itself is testable off Windows.
/// </summary>
public static class TranscriptMarkers
{
    /// <summary>
    /// Both marker families the pipeline emits, including the
    /// <c>[REDACTED:aws_secret_key]</c> form that carries a category label.
    /// Kept identical to the macOS sheet's pattern -- three shells disagreeing
    /// about what a redaction looks like is three different pictures of the
    /// same bytes.
    /// </summary>
    private const string Pattern = @"<PRIVATE_[A-Za-z0-9_]+>|\[REDACTED[^\]]*\]";

    /// <summary>
    /// A bound on how long the scan may run.
    ///
    /// The pattern has no nested quantifiers so it cannot backtrack
    /// catastrophically, but the input is attacker-adjacent -- it is whatever
    /// was in someone's session -- and a UI thread is a poor place to find out
    /// otherwise.
    /// </summary>
    private static readonly Regex Markers = new(
        Pattern,
        RegexOptions.CultureInvariant,
        TimeSpan.FromSeconds(2));

    /// <summary>
    /// Returns the transcript as consecutive runs covering it exactly once,
    /// in order. An empty transcript yields no runs.
    /// </summary>
    /// <remarks>
    /// Runs rather than a single "here are the markers" list because the
    /// caller has to emit the plain text between them too, and having this
    /// method produce both halves is what guarantees no byte is dropped
    /// between two chips -- dropping one would silently show the contributor
    /// less than the approval covers.
    /// </remarks>
    public static IReadOnlyList<TranscriptRun> Split(string transcript)
    {
        ArgumentNullException.ThrowIfNull(transcript);

        var runs = new List<TranscriptRun>();
        if (transcript.Length == 0)
        {
            return runs;
        }

        int cursor = 0;

        try
        {
            foreach (Match match in Markers.Matches(transcript))
            {
                if (match.Index > cursor)
                {
                    runs.Add(new TranscriptRun(cursor, match.Index - cursor, false));
                }

                runs.Add(new TranscriptRun(match.Index, match.Length, true));
                cursor = match.Index + match.Length;
            }
        }
        catch (RegexMatchTimeoutException)
        {
            // Fall through with whatever was found so far discarded: showing
            // the whole body as plain text is worse than showing it as chips,
            // and it is very much better than showing nothing. The bytes are
            // identical either way, which is what the approval covers.
            runs.Clear();
            cursor = 0;
        }

        if (cursor < transcript.Length)
        {
            runs.Add(new TranscriptRun(cursor, transcript.Length - cursor, false));
        }

        return runs;
    }
}
