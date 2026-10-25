import Foundation
import XCTest
import TCBridge
import TCShellCore

final class PrivateAICampaignExportTests: XCTestCase {
    func testCompiledCampaignCrossesTheRealBridgeAndIsUsableOffline() throws {
        let json = try XCTUnwrap(TCCoreCopy.privateAICampaignJSON())
        let campaign = try XCTUnwrap(PrivateAICampaign.decode(from: Data(json.utf8)))
        XCTAssertEqual(campaign.id, "kimi-k3-1-pro")
        XCTAssertEqual(campaign.actionURL.absoluteString, "https://near.ai/models/moonshotai/kimi-k3.1-pro")
        XCTAssertTrue(campaign.body.contains("Incognito"))
        XCTAssertTrue(campaign.body.contains("provider-hosted"))
    }
}
