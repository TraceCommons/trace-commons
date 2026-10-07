import CryptoKit
import Foundation

/// The arithmetic behind Look inside (Ron's `PreviewInspector`, #1241):
/// which review actions it offers, how much of the body a page shows, where
/// a turn separator falls, and how a turn-index row reads.
///
/// Every word here is the core's (`MonitorLookInsideCopy`); this file only
/// fills holes and joins pieces.
public enum LookInside {
    /// The anchor `tc_preview_turns_json` requires: `sha256:` and lowercase
    /// hex over the exact UTF-8 bytes of the body on screen.
    public static func bodyDigest(_ body: String) -> String {
        "sha256:" + SHA256.hash(data: Data(body.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    // MARK: - Native review

    /// Ron's `canWitnessReview`: the daemon supports a review, the witness
    /// is pinned, and the session does not already hold a certificate.
    public static func offersWitnessReview(supported: Bool, witnessPinned: Bool, holdsCertificate: Bool) -> Bool {
        supported && witnessPinned && !holdsCertificate
    }

    /// Whether the session holds a certificate now, not when the sheet
    /// opened: the live queue row says so, or the body on screen was
    /// reopened under a witness digest after a review succeeded (Ron
    /// re-reads `hasCertificate` from the refreshed entry).
    public static func holdsCertificate(liveRow: Bool, envelopeDigest: String?) -> Bool {
        liveRow || envelopeDigest?.hasPrefix("witness-sha256:") == true
    }

    /// Ron's `NativeReviewActions` is drawn when it has something to offer,
    /// and never for a session that already holds a certificate.
    public static func showsNativeReview(admissionOffered: Bool, offersWitness: Bool, holdsCertificate: Bool) -> Bool {
        !holdsCertificate && (admissionOffered || offersWitness)
    }

    // MARK: - Paging

    /// How many of the document's chunks `pages` pages show: whole chunks,
    /// until each page holds at least `pageBytes`.
    public static func shownChunks(_ document: TranscriptDocument, pages: Int, pageBytes: Int) -> Int {
        let budget = max(1, pages) * max(1, pageBytes)
        var bytes = 0
        var count = 0
        for chunk in document.chunks {
            if bytes >= budget { break }
            bytes += chunk.byteCount
            count += 1
        }
        return count
    }

    /// The bytes of the body not yet shown.
    public static func remainingBytes(_ document: TranscriptDocument, shownChunks: Int) -> Int {
        let shown = document.chunks.prefix(max(0, shownChunks)).reduce(0) { $0 + $1.byteCount }
        return max(0, document.totalBytes - shown)
    }

    // MARK: - Turns

    /// A piece of one chunk: from the chunk's start, or from a turn that
    /// opens inside it, to the next turn or the chunk's end.
    public struct Segment: Equatable, Sendable {
        /// The turn that opens this segment, or nil for the chunk's lead-in.
        public let turn: PreviewTurns.Turn?
        /// Absolute UTF-8 byte range in the body.
        public let byteRange: Range<Int>
    }

    /// The chunk cut at every turn that opens strictly inside it or at its
    /// first byte. Turn offsets fall between events, never inside a
    /// redaction marker, so each cut is on a character boundary.
    public static func segments(of chunk: TranscriptDocument.Chunk, turns: [PreviewTurns.Turn]) -> [Segment] {
        let range = chunk.byteRange
        let opening = turns
            .filter { range.contains($0.byteOffset) }
            .sorted { $0.byteOffset < $1.byteOffset }
        var segments: [Segment] = []
        var cursor = range.lowerBound
        var current: PreviewTurns.Turn?
        for turn in opening {
            if turn.byteOffset > cursor {
                segments.append(Segment(turn: current, byteRange: cursor..<turn.byteOffset))
            }
            cursor = turn.byteOffset
            current = turn
        }
        if cursor < range.upperBound || segments.isEmpty {
            segments.append(Segment(turn: current, byteRange: cursor..<range.upperBound))
        }
        return segments
    }

    /// A turn's heading: its position and its opening event's type.
    public static func turnTitle(_ turn: PreviewTurns.Turn) -> String {
        "\(turn.index + 1). " + turn.role.replacingOccurrences(of: "_", with: " ")
    }

    /// A turn's detail: the tool it names (or the core's word for an event
    /// that names none) and its byte range, in the core's words.
    public static func turnDetail(_ turn: PreviewTurns.Turn, words: MonitorLookInsideCopy) -> String {
        let bytes = FirstRunCopy.fill(words.turnBytes, [
            "start": "\(turn.byteOffset)", "end": "\(turn.byteOffset + turn.byteLen)",
        ])
        return (turn.toolName ?? words.turnEvent) + " · " + bytes
    }

    // MARK: - Original search

    /// Ron's count line for a search of the original session.
    public static func originalMatches(_ count: Int, words: MonitorLookInsideCopy) -> String {
        count == 1 ? words.originalMatchOne : FirstRunCopy.fill(words.originalMatches, ["count": "\(count)"])
    }
}
