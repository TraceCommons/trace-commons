import XCTest
import TCBridge
import TCLocalization
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

    private func reviewCopy() throws -> [String: Any] {
        let json = try XCTUnwrap(TCWitness.copyJSON())
        let copy = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        return try XCTUnwrap(copy["review"] as? [String: Any])
    }

    func testABusyViewCarriesItsSentenceAndTryAgainLine() throws {
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
        let copy = try reviewCopy()
        XCTAssertEqual(DaemonClient.refusalSentence(from: failure), copy["failed_busy"] as? String)
        let label = try XCTUnwrap(copy["busy_retry_at"] as? String)
        XCTAssertEqual(
            DaemonClient.busyRetryLine(from: failure, format: utc),
            Localization.fill(Localization.current.text("app.witness.retry_time"), [
                "label": label, "time": "2030-01-01T00:01:00Z"
            ])
        )
    }

    /// A refusal or unreadable time gets no retry line. The core selects
    /// the Busy label even when an older daemon omitted that prose field.
    func testAnythingElseCarriesNoTryAgainLine() {
        let busy: [String: Any] = [
            "state": "Busy", "message": "m",
            "retry_at": "2030-01-01T00:01:00Z", "retry_label": "Try again after",
        ]
        var refused = busy; refused["state"] = "Refused"
        var unreadable = busy; unreadable["retry_at"] = "soon"
        var untimed = busy; untimed.removeValue(forKey: "retry_at")
        for view in [refused, unreadable, untimed] {
            let failure = DaemonClient.failure(code: "unavailable", message: "x", view: view)
            XCTAssertNil(DaemonClient.busyRetryLine(from: failure, format: utc), "\(view)")
        }
        let noView = DaemonClient.failure(code: "unavailable", message: "x", view: nil)
        XCTAssertNil(DaemonClient.busyRetryLine(from: noView, format: utc))
    }

    func testUnknownStatesKeepRawFactsAndBusyLabelsAreCoreSelected() throws {
        let unknown: [String: Any] = [
            "state": "FromFuture", "message": "raw {label}", "retry_label": "raw {time}",
            "retry_at": "2030-01-01T00:01:00Z"
        ]
        let future = DaemonClient.failure(code: "unavailable", message: "future_label", view: unknown)
        XCTAssertEqual(future.viewMessage, "raw {label}")
        XCTAssertEqual(future.viewState, "FromFuture")
        XCTAssertEqual(future.retryLabel, "raw {time}")
        XCTAssertNil(DaemonClient.busyRetryLine(from: future, format: utc))
        let busy = DaemonClient.failure(code: "unavailable", message: "witness_saturated", view: [
            "state": "Busy", "message": "not canonical", "retry_at": "2030-01-01T00:01:00Z"
        ])
        XCTAssertEqual(busy.retryLabel, try reviewCopy()["busy_retry_at"] as? String)
        XCTAssertEqual(busy.viewMessage, try reviewCopy()["failed_busy"] as? String)
        XCTAssertEqual(busy.retryAt.map(utc), "2030-01-01T00:01:00Z")
    }
}
