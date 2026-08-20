import Foundation

/// How much of a redacted body the transcript tab lays out at once.
///
/// The tab used to hand its whole body to one text run. That held while a
/// trace was a pilot trace: 169 KB, laid out in a few milliseconds. It does
/// not hold for a real Claude Code session. A 17.5 MB body typeset as a
/// single attributed string pins the main thread inside CoreText and takes
/// roughly 3 GB of glyph storage to do it, and every relayout -- a resize,
/// a scroll that changes the width -- pays it again. The window stops
/// answering. That is not a slow render; it is an unusable app, and the
/// only input needed to reach it is a session a heavy user would call
/// ordinary.
///
/// So the tab lays out a bounded slice and says so. What it must never do
/// is imply the slice is the whole thing: this tab's promise is "exactly
/// what would be sent", the approval covers every byte, and a view that
/// quietly shows the first fraction of a body while the button beneath it
/// approves all of it would make that promise false. `notice` is therefore
/// not decoration -- it is the sentence that keeps the tab honest, and it
/// states both what is on screen and that approval still covers the rest.
///
/// Three shells render this and they must agree, for the same reason the
/// submit toast must: the Linux copy is
/// `crates/trace-commons-contributor-gtk/src/transcript_budget.rs` and the
/// Windows copy is
/// `windows/src/TraceCommons.Interop/TranscriptBudget.cs`. All three assert
/// the same worked examples.
public enum TranscriptBudget {
    /// The slice size, in bytes of UTF-8.
    ///
    /// Chosen to be far above any plausible "read the first screenful"
    /// need and far below where single-run text layout gets slow: 256 KB is
    /// about 3,000 lines of transcript, and lays out in well under a frame.
    public static let limitBytes = 256 * 1024

    /// A body clamped to the budget.
    public struct Clamped: Equatable, Sendable {
        /// The text to lay out. Never longer than `limitBytes` in UTF-8.
        public let shown: String
        /// UTF-8 bytes of the original body.
        public let totalBytes: Int
        /// UTF-8 bytes not laid out. Zero when the whole body fits.
        public let withheldBytes: Int

        /// True when the body did not fit and `notice` must be shown.
        public var isClamped: Bool { withheldBytes > 0 }
    }

    /// Clamps `text` to the budget, cutting at a line boundary so the slice
    /// never ends mid-line -- and, because a line boundary is always a
    /// scalar boundary, never mid-character either.
    ///
    /// A body with no newline inside the budget (minified JSON on one line,
    /// for instance) still has to be cut somewhere, so the cut backs off to
    /// the nearest UTF-8 scalar boundary instead. Cutting mid-scalar would
    /// put a replacement character on screen inside a tab whose entire job
    /// is showing bytes faithfully.
    public static func clamp(_ text: String) -> Clamped {
        let total = text.utf8.count
        guard total > limitBytes else {
            return Clamped(shown: text, totalBytes: total, withheldBytes: 0)
        }

        let bytes = Array(text.utf8)
        var cut = limitBytes

        // Prefer the last newline in the slice: a whole number of lines is
        // what a person expects to see, and it keeps the cut off the middle
        // of a redaction marker often enough to matter.
        if let newline = bytes[..<cut].lastIndex(of: 0x0A) {
            cut = newline + 1
        } else {
            // No newline to cut on. Back off at most three bytes to leave a
            // continuation byte (0b10xxxxxx) behind us rather than in front.
            while cut > 0, bytes[cut] & 0xC0 == 0x80 {
                cut -= 1
            }
        }

        let shown = String(decoding: bytes[..<cut], as: UTF8.self)
        return Clamped(shown: shown, totalBytes: total, withheldBytes: total - cut)
    }

    /// The sentence shown above a clamped body.
    ///
    /// Says what is displayed, says what is not, and says that approval is
    /// unaffected -- in that order, because the reader's first question is
    /// "am I seeing all of it" and their second is "does that change what I
    /// am about to agree to".
    public static func notice(_ clamped: Clamped) -> String {
        guard clamped.isClamped else { return "" }
        let shownBytes = clamped.totalBytes - clamped.withheldBytes
        return """
            Showing the first \(bytes(shownBytes)) of \(bytes(clamped.totalBytes)). \
            The rest is not displayed here. Approving still covers the whole body.
            """
    }

    /// Byte counts in the shell's usual units. Kept here rather than taken
    /// from a view helper so the three shells format the notice identically.
    static func bytes(_ count: Int) -> String {
        let kb = 1024.0
        let mb = kb * 1024.0
        let value = Double(count)
        if value >= mb {
            return String(format: "%.1f MB", value / mb)
        }
        if value >= kb {
            return String(format: "%.0f KB", value / kb)
        }
        return "\(count) B"
    }
}
