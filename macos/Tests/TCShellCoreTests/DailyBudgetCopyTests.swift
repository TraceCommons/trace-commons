import XCTest

@testable import TCShellCore

/// The condition: a contributor approved traces, the day's byte budget was
/// spent, and nothing anywhere said so. These pin what the shell now says.
final class DailyBudgetCopyTests: XCTestCase {
    /// A fixed rendering, so the assertions do not depend on the machine's
    /// timezone or locale.
    private func fixedTime(_ date: Date) -> String { "00:00" }

    func testTheDetailStatesHowManyAreWaitingAndWhenTheLimitResets() {
        let text = DailyBudgetCopy.detail(
            blockedEntries: 14,
            resetsAt: Date(timeIntervalSince1970: 1_755_820_800),
            formatter: fixedTime
        )
        XCTAssertEqual(
            text,
            "14 approved traces are waiting. Nothing has been lost -- they go out when the "
                + "limit resets at 00:00."
        )
    }

    func testOneWaitingTraceIsNotDescribedInThePlural() {
        let text = DailyBudgetCopy.detail(
            blockedEntries: 1,
            resetsAt: Date(timeIntervalSince1970: 1_755_820_800),
            formatter: fixedTime
        )
        XCTAssertTrue(text.hasPrefix("1 approved trace is waiting."), text)
    }

    func testWithNoResetTimeTheSentenceStopsRatherThanGuessing() {
        // Never "tomorrow": the counters roll at UTC midnight, which is not
        // tomorrow for most of the world.
        let text = DailyBudgetCopy.detail(blockedEntries: 3, resetsAt: nil, formatter: fixedTime)
        XCTAssertEqual(
            text,
            "3 approved traces are waiting. Nothing has been lost -- they go out when the "
                + "limit resets."
        )
        XCTAssertFalse(text.lowercased().contains("tomorrow"), text)
    }

    func testTheCopyNeverReadsAsAFailure() {
        for text in [
            DailyBudgetCopy.title,
            DailyBudgetCopy.detail(blockedEntries: 14, resetsAt: nil, formatter: fixedTime),
            DailyBudgetCopy.detail(blockedEntries: 0, resetsAt: nil, formatter: fixedTime),
        ] {
            let lower = text.lowercased()
            for word in ["error", "failed", "problem", "wrong"] {
                XCTAssertFalse(lower.contains(word), "\(word) in: \(text)")
            }
        }
    }

    func testTheBudgetDecodesFromTheDaemonsOwnShape() {
        // The state measured on the machine this came from.
        let json = """
            {"bytes_today":204659969,"max_bytes_per_day":209715200,
             "bytes_remaining":5055231,"uploads_today":12,"max_uploads_per_day":50,
             "uploads_remaining":38,"resets_at":"2026-08-22T00:00:00Z",
             "blocked":true,"blocked_entries":14,"blocked_bytes":137283584}
            """
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        let b = try! decoder.decode(DailyBudget.self, from: Data(json.utf8))
        XCTAssertTrue(b.blocked)
        XCTAssertEqual(b.blockedEntries, 14)
        XCTAssertEqual(b.bytesToday, 204_659_969)
        XCTAssertEqual(b.bytesRemaining, 5_055_231)
        XCTAssertEqual(b.uploadsRemaining, 38)
        XCTAssertNotNil(b.resetsAt)
    }

    func testAnUnknownBudgetBlocksNothing() {
        // A daemon that predates the field must not make the window claim a
        // condition it never reported.
        XCTAssertFalse(DailyBudget.unknown.blocked)
        XCTAssertEqual(DailyBudget.unknown.blockedEntries, 0)
    }
}
