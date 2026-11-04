import Foundation
import XCTest
import TCBridge
import TCShellCore

final class PrivateAICampaignExportTests: XCTestCase {
    func testCompiledCampaignCrossesTheRealBridgeAndIsUsableOffline() throws {
        let json = try XCTUnwrap(TCCoreCopy.privateAICampaignJSON())
        let campaign = try XCTUnwrap(PrivateAICampaign.decode(from: Data(json.utf8)))
        XCTAssertEqual(campaign.id, "near-ai-red-cybersecurity-models")
        XCTAssertEqual(campaign.actionURL.absoluteString, "https://near.ai/red")
        XCTAssertTrue(campaign.body.contains("secure enclaves"))
    }
}
