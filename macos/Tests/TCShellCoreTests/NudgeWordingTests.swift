import XCTest

/// The nudges' words are the daemon's: `status.nudge.text`, `mark_text`,
/// `reengage_due` and `digest_due` carry every sentence, and the core's
/// tables (`tc_nudge_copy_json`, `tc_nudge_entry_tags_json`) every fixed
/// word. No Swift source under `macos/Sources` may write one of them.
///
/// A source line holding one of these fragments is a failure unless it is
/// a comment. That catches a literal on any code line, a multi-line string
/// included, which is what composing a nudge sentence here would take. The
/// recorded replies (`RecordedSamples`, JSON) are the daemon's own words and
/// are not Swift, so they are not read.
final class NudgeWordingTests: XCTestCase {
    /// Fragments of the core's nudge sentences and fixed words
    /// (`nudge_copy.rs`), each specific enough that no other surface says it.
    static let fragments = [
        "idle for", "have been idle", "has been idle", "sessions from", "session from",
        "credit is now final", "held for privacy review", "Verdicts are in", "previewed session",
        "unpurposed trace", "fit a mission", "fits a mission", "Fits a mission",
        "Estimate: about", "Estimated credit", "Higher estimate", "Typical estimate", "Lower estimate",
        "Suggested first", "Oldest first", "Showing idle sessions", "Show suggestions",
        "Notifications from", "sessions are judged", "Tell me when", "Turn on", "No thanks",
        "Not now", "See history", "Last week:", "nothing is waiting",
    ]

    private static func sources() -> URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // TCShellCoreTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Sources")
    }

    func testNoSwiftSourceWritesANudgeSentence() throws {
        let base = Self.sources()
        let enumerator = try XCTUnwrap(FileManager.default.enumerator(at: base, includingPropertiesForKeys: nil))
        var scanned = 0
        var failures: [String] = []
        for case let url as URL in enumerator where url.pathExtension == "swift" {
            scanned += 1
            let text = try String(contentsOf: url, encoding: .utf8)
            for (index, line) in text.components(separatedBy: "\n").enumerated() {
                let trimmed = line.trimmingCharacters(in: .whitespaces)
                if trimmed.hasPrefix("//") || trimmed.hasPrefix("*") || trimmed.hasPrefix("/*") { continue }
                for fragment in Self.fragments where line.contains(fragment) {
                    let path = url.path.replacingOccurrences(of: base.path + "/", with: "")
                    failures.append("\(path):\(index + 1): \"\(fragment)\"")
                }
            }
        }
        // A scan that read nothing would pass over nothing.
        XCTAssertGreaterThanOrEqual(scanned, 232, "only \(scanned) Swift sources were scanned")
        XCTAssertTrue(
            failures.isEmpty,
            "A nudge's words are the daemon's or the core's, never this shell's:\n"
                + failures.joined(separator: "\n"))
    }

    /// The guard itself: a literal on a code line is caught, a comment is not.
    func testTheGuardCatchesALiteralAndPassesAComment() {
        let literal = #"let title = "\(n) sessions from \(tool) have been idle""#
        let comment = "    /// The idle card: \"sessions from\" is the daemon's."
        XCTAssertTrue(Self.fragments.contains { literal.contains($0) })
        XCTAssertTrue(comment.trimmingCharacters(in: .whitespaces).hasPrefix("//"))
    }
}
