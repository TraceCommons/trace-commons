import TCBridge
import TCShellCore
import XCTest

/// The Contribution mode pill's words (#1208) cross the ABI whole, and this
/// shell decodes exactly the fields the core exports.
final class ContributionModeCopyExportTests: XCTestCase {
    func testTheExportCarriesExactlyTheFieldsThisShellDecodes() throws {
        let json = try XCTUnwrap(TCCoreCopy.contributionModeCopyJSON())
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
        XCTAssertEqual(object.keys.sorted(), ContributionModeCopy.consumedFields.sorted())
        let copy = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: json))
        XCTAssertEqual(copy.choices.map(\.mode), ["notify_only", "auto_upload", "ignore"])
    }

    /// Each choice's confirmation crosses whole; Automatic's carries
    /// the arming disclosure, and is not given without a configuration
    /// directory to read it for.
    func testEveryChoiceHasItsCoreConfirmation() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("tc-override-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let pill = try XCTUnwrap(ContributionModeCopy.decode(fromJSON: TCCoreCopy.contributionModeCopyJSON()))
        for choice in pill.choices {
            let json = try XCTUnwrap(TCCoreCopy.contributionOverrideConfirmJSON(mode: choice.mode, configDir: dir.path))
            let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
            XCTAssertEqual(object.keys.sorted(), ContributionOverrideConfirmCopy.consumedFields.sorted())
            let copy = try XCTUnwrap(ContributionOverrideConfirmCopy.decode(fromJSON: json), choice.mode)
            XCTAssertEqual(copy.mode, choice.mode)
            XCTAssertEqual(copy.arming != nil, choice.mode == "auto_upload", choice.mode)
        }
        XCTAssertNil(TCCoreCopy.contributionOverrideConfirmJSON(mode: "auto_upload", configDir: nil))
        XCTAssertNotNil(TCCoreCopy.contributionOverrideConfirmJSON(mode: "ignore", configDir: nil))
    }

    func testARefusalHasACoreLine() throws {
        let terms = try XCTUnwrap(TCCoreCopy.contributionOverrideRefusalLine(label: "arming-terms-unavailable"))
        let other = try XCTUnwrap(TCCoreCopy.contributionOverrideRefusalLine(label: "policy-write-failed"))
        XCTAssertFalse(terms.isEmpty)
        XCTAssertNotEqual(terms, other)
        XCTAssertEqual(TCCoreCopy.contributionOverrideRefusalLine(label: "unknown-label"), other)
    }
}
