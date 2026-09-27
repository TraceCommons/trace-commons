import TCShellCore
import XCTest

/// `status.witness_capacity`: approved sessions held because the privacy
/// witness is busy, and the notice the Rust words for them.
final class WitnessCapacityTests: XCTestCase {
    private func decodeCapacity(_ json: String) throws -> WitnessCapacity {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(WitnessCapacity.self, from: Data(json.utf8))
    }

    func testTheStatusObjectDecodes() throws {
        let capacity = try decodeCapacity(
            #"{"waiting_sessions":2,"next_retry_at":"2030-01-01T00:01:00Z"}"#)
        XCTAssertEqual(capacity.waitingSessions, 2)
        XCTAssertTrue(capacity.waiting)
        XCTAssertEqual(capacity.nextRetryAt, ISO8601DateFormatter().date(from: "2030-01-01T00:01:00Z"))
    }

    func testNothingWaitingIsNotWaiting() throws {
        let capacity = try decodeCapacity(#"{"waiting_sessions":0,"next_retry_at":null}"#)
        XCTAssertFalse(capacity.waiting)
        XCTAssertFalse(WitnessCapacity.none.waiting)
    }

    /// Only the count reaches the Rust; it is all the words depend on.
    func testTheWireHandedToTheCoreCarriesTheCount() throws {
        let capacity = WitnessCapacity(waitingSessions: 3)
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(capacity.wireJSON.utf8)) as? [String: Any])
        XCTAssertEqual(object["waiting_sessions"] as? Int, 3)
    }

    private let notice = """
    {"title":"Waiting for the privacy witness","body":"2 approved sessions are waiting.",
     "next_check":"Next try"}
    """

    func testTheNoticeIsTakenWhole() throws {
        let decoded = try XCTUnwrap(WitnessCapacityNotice.decode(fromJSON: notice))
        XCTAssertEqual(decoded.title, "Waiting for the privacy witness")
        XCTAssertEqual(decoded.nextCheck, "Next try")
    }

    func testANoticeMissingASentenceIsNotShownInPart() {
        for key in WitnessCapacityNotice.consumedFields {
            var object = (try? JSONSerialization.jsonObject(with: Data(notice.utf8))) as? [String: Any] ?? [:]
            object[key] = ""
            let data = try! JSONSerialization.data(withJSONObject: object)
            XCTAssertNil(
                WitnessCapacityNotice.decode(fromJSON: String(decoding: data, as: UTF8.self)), key)
        }
        XCTAssertNil(WitnessCapacityNotice.decode(fromJSON: "null"))
    }

    /// The next try is the Rust's label and the daemon's time, or nothing --
    /// never a time this shell made up.
    func testTheNextTryLineIsTheCoresLabelAndTheDaemonsTime() throws {
        let decoded = try XCTUnwrap(WitnessCapacityNotice.decode(fromJSON: notice))
        let at = Date(timeIntervalSince1970: 0)
        XCTAssertEqual(
            decoded.nextRetryLine(for: WitnessCapacity(waitingSessions: 1, nextRetryAt: at)) { _ in "then" },
            "Next try: then")
        XCTAssertNil(decoded.nextRetryLine(for: WitnessCapacity(waitingSessions: 1)) { _ in "then" })
    }
}
