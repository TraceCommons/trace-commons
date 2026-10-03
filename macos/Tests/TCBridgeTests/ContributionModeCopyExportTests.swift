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
}
