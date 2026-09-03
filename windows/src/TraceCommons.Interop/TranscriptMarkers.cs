using System;
using System.Collections.Generic;
using System.Text;
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
    /// Every marker shape the pipeline emits, including the
    /// <c>[REDACTED:aws_secret_key]</c> form that carries a category label.
    /// Three shells disagreeing about what a redaction looks like is three
    /// different pictures of the same bytes.
    ///
    /// The redactor emits four shapes, and this arm exists because one of
    /// them reached neither of the others. <c>apply_placeholder_regex</c>
    /// mints the numbered <c>&lt;PRIVATE_LABEL_n&gt;</c> form for exactly two
    /// labels, <c>local_path</c> and <c>private_email</c>. Everything else
    /// gets a fixed token: <c>[REDACTED]</c> for secrets and sensitive
    /// fields, <c>[REDACTED:{label}]</c> for tool arguments and
    /// privacy-filter findings, and <c>&lt;REDACTED_PRIVATE_KEY&gt;</c> for a
    /// PEM key. That last one starts neither <c>&lt;PRIVATE_</c> nor a
    /// bracket, so both other arms missed it and a private key the redactor
    /// HAD removed drew as ordinary transcript text. Secrets are the category
    /// a contributor most wants to see marked, and this was the shape that
    /// was not.
    ///
    /// The <c>[REDACTED...]</c> arm excludes newlines as well as <c>]</c>.
    /// Without that, one unclosed bracket anywhere in a body would let a
    /// "marker" run to the end of the file, and the chunker -- which uses
    /// this same pattern to avoid cutting through a marker -- would then
    /// refuse to cut anywhere inside it.
    /// </summary>
    private const string Pattern =
        @"<REDACTED_PRIVATE_KEY>|<PRIVATE_[A-Za-z0-9_]+>|\[REDACTED[^\]\n]*\]";

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

    /// <summary>
    /// Marker spans in <paramref name="text"/> as UTF-8 byte offsets from
    /// its start, in order.
    /// </summary>
    /// <remarks>
    /// The chunker works in UTF-8 bytes because the budget is defined in
    /// them, while <see cref="Regex"/> works in UTF-16 units. This is the
    /// one place that conversion happens, and it is here rather than in the
    /// chunker so that the chunker and the chipping cannot drift apart about
    /// what a marker is: a chunker protecting a different set of markers
    /// than the view highlights would split exactly the ones the view cares
    /// about.
    /// </remarks>
    public static IReadOnlyList<(int Start, int End)> ByteSpans(string text)
    {
        ArgumentNullException.ThrowIfNull(text);

        var spans = new List<(int, int)>();
        if (text.Length == 0)
        {
            return spans;
        }

        try
        {
            int cursor = 0;
            int bytesBefore = 0;
            foreach (Match match in Markers.Matches(text))
            {
                bytesBefore += Encoding.UTF8.GetByteCount(
                    text.AsSpan(cursor, match.Index - cursor));
                int length = Encoding.UTF8.GetByteCount(
                    text.AsSpan(match.Index, match.Length));
                spans.Add((bytesBefore, bytesBefore + length));
                bytesBefore += length;
                cursor = match.Index + match.Length;
            }
        }
        catch (RegexMatchTimeoutException)
        {
            // Same trade as Split: report no markers rather than half of
            // them. A chunk boundary that lands inside a marker is a
            // rendering blemish; a scan that never returns is a hung window.
            spans.Clear();
        }

        return spans;
    }
}
