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

    /// The one sentence each route gets for the Contribution rule card,
    /// approved by the owner on 2026-10-12, word for word.
    func testEachRouteHasItsApprovedOneSentence() throws {
        let patterns = try XCTUnwrap(AutomaticGrantCopy.decode(
            fromJSON: TCCoreCopy.automaticGrantCopyJSON(disclosure: "patterns_only")))
        let model = try XCTUnwrap(AutomaticGrantCopy.decode(
            fromJSON: TCCoreCopy.automaticGrantCopyJSON(disclosure: "model_scrubbed")))
        XCTAssertEqual(
            patterns.summary,
            "Near.ai removes keys, emails, phone numbers and file paths, but names and anything written in plain text can still go out, and no one reviews traces before they're sent.")
        XCTAssertEqual(
            model.summary,
            "Near.ai removes keys, emails, phone numbers and file paths, and a privacy model catches most names, but some personal details can still go out, and no one reviews traces before they're sent.")
    }

    func testAnUnknownNameHasNoWords() {
        XCTAssertNil(TCCoreCopy.automaticGrantCopyJSON(disclosure: "scrubbed"))
        XCTAssertNil(AutomaticGrantCopy.decode(fromJSON: TCCoreCopy.automaticGrantCopyJSON(disclosure: "")))
    }
}
