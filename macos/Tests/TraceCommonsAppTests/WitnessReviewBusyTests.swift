import XCTest
@testable import TraceCommonsApp

/// A review a person asked for that met a busy witness is its own outcome:
/// the daemon's busy sentence plus when to try again, read from the
/// response's `view` -- not a refusal. Asserted on the static reader, so it
/// needs no live socket.
final class WitnessReviewBusyTests: XCTestCase {
    private let utc: (Date) -> String = { date in
        let formatter = ISO8601DateFormatter()
        formatter.timeZone = TimeZone(identifier: "UTC")
        return formatter.string(from: date)
    }

    func testABusyViewCarriesItsSentenceAndTryAgainLine() {
        let failure = DaemonClient.failure(
            code: "unavailable",
            message: "witness_saturated",
            view: [
                "state": "Busy",
                "message": "The review could not go ahead yet.",
                "retry_at": "2030-01-01T00:01:00Z",
                "retry_label": "Try again after",
            ]
        )
        XCTAssertEqual(DaemonClient.refusalSentence(from: failure), "The review could not go ahead yet.")
        XCTAssertEqual(
            DaemonClient.busyRetryLine(from: failure, format: utc),
            "Try again after: 2030-01-01T00:01:00Z"
        )
    }

    /// A refusal, an unreadable time, or no label: no line, never a guess.
    func testAnythingElseCarriesNoTryAgainLine() {
        let busy: [String: Any] = [
            "state": "Busy", "message": "m",
            "retry_at": "2030-01-01T00:01:00Z", "retry_label": "Try again after",
        ]
        var refused = busy; refused["state"] = "Refused"
        var unreadable = busy; unreadable["retry_at"] = "soon"
        var unlabelled = busy; unlabelled.removeValue(forKey: "retry_label")
        for view in [refused, unreadable, unlabelled] {
            let failure = DaemonClient.failure(code: "unavailable", message: "x", view: view)
            XCTAssertNil(DaemonClient.busyRetryLine(from: failure, format: utc), "\(view)")
        }
        let noView = DaemonClient.failure(code: "unavailable", message: "x", view: nil)
        XCTAssertNil(DaemonClient.busyRetryLine(from: noView, format: utc))
    }
}
