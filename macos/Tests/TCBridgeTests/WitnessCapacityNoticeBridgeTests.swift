import TCBridge
import TCShellCore
import XCTest

/// The witness capacity notice through the real dylib: the count goes in,
/// the Rust's notice comes out, and this shell decodes all of it.
final class WitnessCapacityNoticeBridgeTests: XCTestCase {
    func testWaitingSessionsBecomeTheRustsNotice() throws {
        let capacity = WitnessCapacity(waitingSessions: 2)
        let json = try XCTUnwrap(TCConsentCopy.witnessCapacityNoticeJSON(forCapacity: capacity.wireJSON))
        let notice = try XCTUnwrap(WitnessCapacityNotice.decode(fromJSON: json))
        XCTAssertTrue(notice.body.hasPrefix("2 "), notice.body)
        XCTAssertFalse(notice.title.isEmpty)
    }

    /// Nothing waiting is nothing to say, decided by the ABI.
    func testNothingWaitingGetsNoNotice() {
        XCTAssertNil(TCConsentCopy.witnessCapacityNoticeJSON(forCapacity: WitnessCapacity.none.wireJSON))
    }

    /// The exported field set is exactly what this shell decodes, so a
    /// sentence added in Rust cannot be silently dropped here.
    func testTheExportedFieldsAreExactlyTheOnesThisShellConsumes() throws {
        let json = try XCTUnwrap(
            TCConsentCopy.witnessCapacityNoticeJSON(forCapacity: WitnessCapacity(waitingSessions: 1).wireJSON))
        let object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), WitnessCapacityNotice.consumedFields.sorted())
    }
}
