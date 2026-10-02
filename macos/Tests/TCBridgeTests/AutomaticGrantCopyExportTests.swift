import TCBridge
import TCShellCore
import XCTest

/// An armed folder's disclosure words come from the core, for the
/// disclosure the daemon named, decoded the way the Traces tab decodes them.
final class AutomaticGrantCopyExportTests: XCTestCase {
    func testEachNamedDisclosureDecodesToItsOwnWords() throws {
        for name in ["patterns_only", "model_scrubbed"] {
            let json = try XCTUnwrap(TCCoreCopy.automaticGrantCopyJSON(disclosure: name))
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
            XCTAssertTrue(Set(AutomaticGrantCopy.consumedFields).isSubset(of: object.keys), name)
            let copy = try XCTUnwrap(AutomaticGrantCopy.decode(fromJSON: json))
            XCTAssertEqual(copy.disclosure, name)
            XCTAssertEqual(copy.lines.count, 3)
        }
    }

    /// The patterns-only folder never carries the model-scrub wording.
    func testPatternsOnlyNeverSaysAModelScrubbed() throws {
        let patterns = try XCTUnwrap(AutomaticGrantCopy.decode(
            fromJSON: TCCoreCopy.automaticGrantCopyJSON(disclosure: "patterns_only")))
        let model = try XCTUnwrap(AutomaticGrantCopy.decode(
            fromJSON: TCCoreCopy.automaticGrantCopyJSON(disclosure: "model_scrubbed")))
        XCTAssertNil(patterns.modelScrubbed)
        XCTAssertNotEqual(patterns.lines[0], model.lines[0])
    }

    func testAnUnknownNameHasNoWords() {
        XCTAssertNil(TCCoreCopy.automaticGrantCopyJSON(disclosure: "scrubbed"))
        XCTAssertNil(AutomaticGrantCopy.decode(fromJSON: TCCoreCopy.automaticGrantCopyJSON(disclosure: "")))
    }
}
