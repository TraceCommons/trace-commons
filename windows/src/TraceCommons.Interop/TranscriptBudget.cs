using System;
using System.Globalization;
using System.Text;

namespace TraceCommons.Interop;

/// <summary>
/// How much of a redacted body the transcript tab lays out at once.
///
/// The tab used to hand its whole body to one text run. That held while a
/// trace was a pilot trace: 169 KB, laid out in a few milliseconds. It does
/// not hold for a real Claude Code session. A 17.5 MB body typeset as a
/// single <c>RichTextBlock</c> pins the UI thread inside the text engine and
/// takes roughly 3 GB of glyph storage to do it, and every relayout -- a
/// resize, a scroll that changes the width -- pays it again. The window
/// stops answering. That is not a slow render; it is an unusable app, and
/// the only input needed to reach it is a session a heavy user would call
/// ordinary.
///
/// So the tab lays out a bounded slice and says so. What it must never do
/// is imply the slice is the whole thing: this tab's promise is "exactly
/// what would be sent", the approval covers every byte, and a view that
/// quietly shows the first fraction of a body while the button beneath it
/// approves all of it would make that promise false. <see cref="Notice"/> is
/// therefore not decoration -- it is the sentence that keeps the tab
/// honest, and it states both what is on screen and that approval still
/// covers the rest.
///
/// Three shells render this and they must agree, for the same reason the
/// submit toast must: the macOS copy is
/// <c>macos/Sources/TCShellCore/TranscriptBudget.swift</c> and the Linux
/// copy is <c>crates/trace-commons-contributor-gtk/src/transcript_budget.rs</c>.
/// All three assert the same worked examples.
///
/// This lives in the interop assembly rather than in the App project for the
/// same reason <see cref="SubmitToast"/> does: it is exercised by tests on a
/// machine that cannot build WinUI at all.
/// </summary>
public static class TranscriptBudget
{
    /// <summary>
    /// The slice size, in bytes of UTF-8.
    ///
    /// Chosen to be far above any plausible "read the first screenful" need
    /// and far below where single-run text layout gets slow: 256 KB is about
    /// 3,000 lines of transcript, and lays out in well under a frame.
    /// </summary>
    public const int LimitBytes = 256 * 1024;

    /// <summary>A body clamped to the budget.</summary>
    public sealed class Clamped
    {
        internal Clamped(string shown, int totalBytes, int withheldBytes)
        {
            Shown = shown;
            TotalBytes = totalBytes;
            WithheldBytes = withheldBytes;
        }

        /// <summary>The text to lay out. Never longer than <see cref="LimitBytes"/> in UTF-8.</summary>
        public string Shown { get; }

        /// <summary>UTF-8 bytes of the original body.</summary>
        public int TotalBytes { get; }

        /// <summary>UTF-8 bytes not laid out. Zero when the whole body fits.</summary>
        public int WithheldBytes { get; }

        /// <summary>True when the body did not fit and <see cref="Notice"/> must be shown.</summary>
        public bool IsClamped => WithheldBytes > 0;
    }

    /// <summary>
    /// Clamps <paramref name="text"/> to the budget, cutting at a line
    /// boundary so the slice never ends mid-line -- and, because a line
    /// boundary is always a character boundary, never mid-character either.
    ///
    /// A body with no newline inside the budget (minified JSON on one line,
    /// for instance) still has to be cut somewhere, so the cut backs off to
    /// the nearest UTF-8 byte boundary instead. Cutting mid-sequence would
    /// put a replacement character on screen inside a tab whose entire job
    /// is showing bytes faithfully. Note that a .NET <see cref="string"/> is
    /// UTF-16: the budget is defined in UTF-8 bytes, so the cut is computed
    /// over the UTF-8 encoding and only decoded back to UTF-16 once the cut
    /// point is known to sit on a scalar boundary, which is what keeps a
    /// four-byte character's surrogate pair from being split too.
    /// </summary>
    public static Clamped Clamp(string text)
    {
        ArgumentNullException.ThrowIfNull(text);

        byte[] bytes = Encoding.UTF8.GetBytes(text);
        int total = bytes.Length;

        if (total <= LimitBytes)
        {
            return new Clamped(text, total, 0);
        }

        int cut = LimitBytes;

        // Prefer the last newline in the slice: a whole number of lines is
        // what a person expects to see, and it keeps the cut off the middle
        // of a redaction marker often enough to matter.
        int newline = LastIndexOfNewline(bytes, cut);
        if (newline >= 0)
        {
            cut = newline + 1;
        }
        else
        {
            // No newline to cut on. Back off until we are not sitting on a
            // UTF-8 continuation byte (0b10xxxxxx), so the cut lands after a
            // whole scalar rather than inside one.
            while (cut > 0 && (bytes[cut] & 0xC0) == 0x80)
            {
                cut -= 1;
            }
        }

        string shown = Encoding.UTF8.GetString(bytes, 0, cut);
        return new Clamped(shown, total, total - cut);
    }

    private static int LastIndexOfNewline(byte[] bytes, int upperBoundExclusive)
    {
        for (int i = upperBoundExclusive - 1; i >= 0; i--)
        {
            if (bytes[i] == 0x0A)
            {
                return i;
            }
        }

        return -1;
    }

    /// <summary>
    /// The sentence shown above a clamped body.
    ///
    /// Says what is displayed, says what is not, and says that approval is
    /// unaffected -- in that order, because the reader's first question is
    /// "am I seeing all of it" and their second is "does that change what I
    /// am about to agree to".
    /// </summary>
    public static string Notice(Clamped clamped)
    {
        ArgumentNullException.ThrowIfNull(clamped);

        if (!clamped.IsClamped)
        {
            return string.Empty;
        }

        int shownBytes = clamped.TotalBytes - clamped.WithheldBytes;
        return string.Format(
            CultureInfo.InvariantCulture,
            "Showing the first {0} of {1}. The rest is not displayed here. Approving still covers the whole body.",
            Bytes(shownBytes),
            Bytes(clamped.TotalBytes));
    }

    /// <summary>
    /// Byte counts in the shell's usual units. Kept here rather than taken
    /// from a view helper so the three shells format the notice identically.
    /// </summary>
    internal static string Bytes(int count)
    {
        const double Kb = 1024.0;
        const double Mb = Kb * 1024.0;
        double value = count;

        if (value >= Mb)
        {
            return string.Format(CultureInfo.InvariantCulture, "{0:F1} MB", value / Mb);
        }

        if (value >= Kb)
        {
            return string.Format(CultureInfo.InvariantCulture, "{0:F0} KB", value / Kb);
        }

        return string.Format(CultureInfo.InvariantCulture, "{0} B", count);
    }
}
