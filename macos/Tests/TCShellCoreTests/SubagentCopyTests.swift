import XCTest

@testable import TCShellCore

/// The condition: a very large Claude Code conversation had its largest
/// delegated transcripts left out to fit the source's byte budget, and no
/// client said so -- a contributor consenting to a conversation nobody told
/// them was trimmed. The contract calls surfacing that a `must`. These pin
/// what the card now says.
final class SubagentCopyTests: XCTestCase {
    /// Never a "0 dropped" row: a line that is always present is a line
    /// nobody reads, and the one case that matters would be lost in it.
    func testACardCoveringNothingDelegatedSaysNothingAtAll() {
        XCTAssertNil(SubagentCopy.line(count: 0, dropped: 0))
    }

    func testTheExtentLineCountsInWordsAPersonCanRead() {
        XCTAssertEqual(
            SubagentCopy.line(count: 1, dropped: 0),
            "Includes 1 delegated subagent transcript."
        )
        XCTAssertEqual(
            SubagentCopy.line(count: 42, dropped: 0),
            "Includes 42 delegated subagent transcripts."
        )
    }

    /// The contract's one `must`. Every shape with a drop in it says so, says
    /// what survived, and says neither in a word that reads as a fault --
    /// trimming is a normal consequence of a very large session.
    func testADroppedTranscriptIsAlwaysStated() {
        for (kept, dropped) in [(0, 1), (0, 7), (3, 1), (42, 3)] {
            guard let line = SubagentCopy.line(count: kept, dropped: dropped) else {
                return XCTFail("a drop is never silent: \(kept)/\(dropped)")
            }
            XCTAssertTrue(
                line.contains("\(dropped)") || (dropped == 1 && line.contains("largest")),
                "the count of what was left out has to appear: \(line)"
            )
            XCTAssertTrue(
                line.contains("the conversation itself is complete"),
                "a trimmed card must say what survived: \(line)"
            )
            for alarming in ["error", "failed", "corrupt", "incomplete", "lost", "missing"] {
                XCTAssertFalse(
                    line.lowercased().contains(alarming),
                    "\(alarming) makes a normal trim read as a fault: \(line)"
                )
            }
        }
    }

    /// "The 1 largest" is a bug; one dropped transcript is "the largest".
    func testOneDroppedTranscriptIsNotDescribedInThePlural() {
        XCTAssertEqual(
            SubagentCopy.line(count: 42, dropped: 1),
            "Includes 42 delegated subagent transcripts. The largest was left out to keep this "
                + "session within its size limit; the conversation itself is complete."
        )
        XCTAssertEqual(
            SubagentCopy.line(count: 42, dropped: 3),
            "Includes 42 delegated subagent transcripts. The 3 largest were left out to keep "
                + "this session within its size limit; the conversation itself is complete."
        )
    }

    /// Everything delegated was dropped: there is no kept count to open with,
    /// so the sentence starts from what was left out rather than claiming to
    /// include nothing.
    func testEverythingDroppedStartsFromWhatWasLeftOut() {
        XCTAssertEqual(
            SubagentCopy.line(count: 0, dropped: 1),
            "1 delegated subagent transcript was left out to keep this session within its size "
                + "limit; the conversation itself is complete."
        )
        XCTAssertEqual(
            SubagentCopy.line(count: 0, dropped: 2),
            "2 delegated subagent transcripts were left out to keep this session within its "
                + "size limit; the conversation itself is complete."
        )
    }
}
