import TCBridge
import TCShellCore
import XCTest

/// The first-run table decodes from the real export, so a key renamed in the
/// core fails here rather than as a blank first run.
final class FirstRunCopyExportTests: XCTestCase {
    func testTheExportDecodes() throws {
        let json = try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())
        let copy = try XCTUnwrap(FirstRunCopy.decode(json))
        XCTAssertEqual(copy.frame.quickSetup, "Quick setup")
        XCTAssertEqual(copy.frame.back, "Back")
    }

    /// Folders' sign-in line is the core's existing one, read through the
    /// first-run table rather than a second bridge call.
    func testTheSignInLineIsTheInferenceCopys() throws {
        let copy = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        let inference = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(try XCTUnwrap(TCCoreCopy.inferenceConnectionCopyJSON()).utf8))
                as? [String: Any])
        XCTAssertEqual(copy.folders.signInFailed, inference["sign_in_failed"] as? String)
        XCTAssertNotEqual(copy.folders.lookupUnavailable, copy.join.inviteError)
        XCTAssertNotEqual(copy.uses.completeFailed, copy.uses.sharingRefused)
    }

    func testATableWithAnEmptyStringIsRefused() throws {
        let json = try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())
        let blanked = json.replacingOccurrences(of: "\"Quick setup\"", with: "\"\"")
        XCTAssertNotEqual(blanked, json)
        XCTAssertNil(FirstRunCopy.decode(blanked))
    }
}
