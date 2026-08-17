using System;
using System.Collections.Generic;
using System.Text;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// One open preview: the redacted transcript body plus its summary.
///
/// This is the C ABI's single deliberate content exemption. Everything else
/// the library returns is a fixed label; <see cref="Body"/> carries real
/// post-redaction trace text, because a contributor cannot consent to sending
/// something they cannot see. That exemption is bounded and it does not travel:
/// this string must never reach a log line, an audit entry, a history record,
/// notification text, or a receipt.
///
/// Every accessor is read once at construction rather than on demand. The
/// native strings are borrowed and valid only until tc_preview_free, so
/// copying them up front means a disposed preview cannot hand out a dangling
/// pointer no matter how the UI holds it.
/// </summary>
public sealed class TcPreview : IDisposable
{
    private IntPtr _preview;
    private readonly object _gate = new();

    internal TcPreview(IntPtr preview)
    {
        _preview = preview;

        // Copied eagerly, while the pointer is known live. NULL from either
        // accessor means the ABI rejected the pointer, which cannot happen for
        // a preview we just opened -- but an empty string is a safer default
        // than a null the UI would have to special-case.
        Body = NativeMethods.BorrowedString(NativeMethods.tc_preview_body(preview)) ?? string.Empty;
        SummaryJson =
            NativeMethods.BorrowedString(NativeMethods.tc_preview_summary_json(preview)) ?? "{}";
    }

    /// <summary>
    /// The redacted transcript, already copied into managed memory. See the
    /// type doc: this is trace content and must not be logged.
    /// </summary>
    public string Body { get; }

    /// <summary>Counts, sizes, and the opening prompt, as raw JSON.</summary>
    public string SummaryJson { get; }

    /// <summary>
    /// Searches the redacted body for <paramref name="needle"/> and returns
    /// match positions as UTF-16 indices into <see cref="Body"/>.
    ///
    /// THE OFFSET CONVERSION IS THE WHOLE POINT OF THIS METHOD. The ABI
    /// reports UTF-8 BYTE offsets. A C# string is UTF-16, so those numbers do
    /// not index it: any non-ASCII character earlier in the transcript shifts
    /// every subsequent match, and an emoji shifts it differently again
    /// (4 UTF-8 bytes, 2 UTF-16 units). Handing the raw values to a text
    /// highlighter would silently mis-highlight exactly the traces most likely
    /// to contain something a contributor wants to find before consenting.
    ///
    /// Returns an empty list for an empty needle, matching the ABI's own
    /// "an empty needle matches nothing".
    /// </summary>
    /// <exception cref="TcException">
    /// The preview pointer was rejected, or the match count would overflow a
    /// 32-bit count -- which the ABI reports as an error rather than silently
    /// truncating.
    /// </exception>
    public IReadOnlyList<int> Search(string needle)
    {
        ArgumentNullException.ThrowIfNull(needle);

        IntPtr preview;
        lock (_gate)
        {
            if (_preview == IntPtr.Zero)
            {
                throw new ObjectDisposedException(nameof(TcPreview));
            }

            preview = _preview;
        }

        int count = NativeMethods.tc_preview_search(preview, needle, out IntPtr matchesPtr);
        if (count < 0)
        {
            // On error the ABI sets matchesJson to NULL, so there is nothing
            // to free here.
            string? label = NativeMethods.BorrowedString(NativeMethods.tc_last_error());
            throw new TcException($"tc_preview_search failed: {label ?? "unknown error"}");
        }

        string? json = NativeMethods.TakeOwnedString(matchesPtr);
        if (string.IsNullOrEmpty(json))
        {
            return Array.Empty<int>();
        }

        int[] byteOffsets = JsonSerializer.Deserialize<int[]>(json) ?? Array.Empty<int>();
        return ConvertUtf8OffsetsToUtf16(Body, byteOffsets);
    }

    /// <summary>
    /// Maps UTF-8 byte offsets into <paramref name="text"/> to UTF-16 indices.
    ///
    /// Walks the string once, keeping a running byte position, rather than
    /// re-encoding a prefix per offset -- a transcript is large and the match
    /// list can be long, so the naive version is quadratic on exactly the
    /// inputs that matter.
    ///
    /// Offsets are sorted first because the walk is single-pass and forward
    /// only; the ABI documents matches as left-to-right, but sorting makes the
    /// method correct for any input rather than correct-by-assumption. An
    /// offset past the end of the text is dropped rather than clamped: a match
    /// that cannot be located is not a match at position zero.
    /// </summary>
    internal static IReadOnlyList<int> ConvertUtf8OffsetsToUtf16(string text, int[] byteOffsets)
    {
        if (byteOffsets.Length == 0)
        {
            return Array.Empty<int>();
        }

        int[] sorted = (int[])byteOffsets.Clone();
        Array.Sort(sorted);

        var results = new List<int>(sorted.Length);
        int utf16Index = 0;
        int byteIndex = 0;
        int cursor = 0;

        while (cursor < sorted.Length)
        {
            if (utf16Index >= text.Length)
            {
                // Past the last character. Only an offset pointing exactly at
                // the end position still resolves; anything beyond it is past
                // the end of the text and is dropped by falling out of the
                // loop.
                while (cursor < sorted.Length && sorted[cursor] <= byteIndex)
                {
                    results.Add(utf16Index);
                    cursor++;
                }

                break;
            }

            // Surrogate pairs advance two UTF-16 units and four UTF-8 bytes.
            bool isSurrogatePair = char.IsHighSurrogate(text[utf16Index])
                                   && utf16Index + 1 < text.Length
                                   && char.IsLowSurrogate(text[utf16Index + 1]);

            int units = isSurrogatePair ? 2 : 1;
            int charBytes = Encoding.UTF8.GetByteCount(text.AsSpan(utf16Index, units));

            // Claim every offset falling anywhere WITHIN this character before
            // advancing past it. Testing the range here, rather than testing
            // `<= byteIndex` on the next iteration, is what makes an offset
            // that lands mid-sequence resolve to the start of the character
            // containing it instead of to the one after it.
            while (cursor < sorted.Length && sorted[cursor] < byteIndex + charBytes)
            {
                results.Add(utf16Index);
                cursor++;
            }

            byteIndex += charBytes;
            utf16Index += units;
        }

        // Any offsets left over point past the end of the text; see the doc.
        return results;
    }

    /// <summary>
    /// Frees the preview. Idempotent.
    ///
    /// The ABI's wrong-pointer check is keyed on the pointer VALUE, not on
    /// shared ownership, so it cannot make a concurrent free safe. The lock
    /// here is what actually prevents a double free from this type; the
    /// caller's obligation is not to be inside <see cref="Search"/> on another
    /// thread while disposing.
    /// </summary>
    public void Dispose()
    {
        IntPtr preview;
        lock (_gate)
        {
            preview = _preview;
            _preview = IntPtr.Zero;
        }

        if (preview != IntPtr.Zero)
        {
            NativeMethods.tc_preview_free(preview);
        }
    }
}
