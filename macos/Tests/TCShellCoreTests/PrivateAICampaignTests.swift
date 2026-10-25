import Foundation
import XCTest

@testable import TCShellCore

final class PrivateAICampaignTests: XCTestCase {
    private let endpoint = URL(string: "https://assets.example/native/campaign.json")!

    private func data(_ replacements: [String: Any] = [:]) throws -> Data {
        var fields: [String: Any] = [
            "schema_version": 1, "id": "kimi-k3-1-pro", "title": "Kimi K3.1 Pro",
            "body": "Try Kimi with Private AI.", "action_title": "Try Kimi",
            "action_url": "https://near.ai/models/kimi", "symbol": "sparkles",
        ]
        fields.merge(replacements) { _, new in new }
        return try JSONSerialization.data(withJSONObject: fields)
    }

    func testValidManifestAndSiblingRasterArtworkDecode() throws {
        let campaign = try XCTUnwrap(PrivateAICampaign.decode(
            from: data(["image_url": "https://assets.example/native/kimi.png"]), endpoint: endpoint))
        XCTAssertEqual(campaign.schemaVersion, 1)
        XCTAssertEqual(campaign.id, "kimi-k3-1-pro")
        XCTAssertEqual(campaign.actionTitle, "Try Kimi")
        XCTAssertEqual(campaign.actionURL.host, "near.ai")
        XCTAssertEqual(campaign.imageURL?.lastPathComponent, "kimi.png")
    }

    func testUnsupportedSchemaSymbolsAndBlankOrOversizedTextFailClosed() throws {
        for replacement: [String: Any] in [
            ["schema_version": 2], ["symbol": "custom.remote.symbol"], ["title": " \n "],
            ["title": String(repeating: "a", count: 101)], ["body": String(repeating: "a", count: 301)],
            ["action_title": String(repeating: "a", count: 41)], ["id": String(repeating: "a", count: 101)],
            ["body": "visible\u{0000}hidden"], ["title": "<b>Kimi</b>"],
        ] {
            XCTAssertNil(PrivateAICampaign.decode(from: try data(replacement)), "\(replacement.keys)")
        }
        XCTAssertNil(PrivateAICampaign.decode(from: Data("not JSON".utf8)))
    }

    func testActionRequiresHTTPSAndTheExactNearAIOriginWithoutCredentials() throws {
        for url in ["http://near.ai", "https://evilnear.ai", "https://near.ai.example", "https://user@near.ai",
                    "https://near.ai:444", "file:///tmp/campaign", "https://near.ai./models"] {
            XCTAssertNil(PrivateAICampaign.decode(from: try data(["action_url": url])), url)
        }
        XCTAssertNotNil(PrivateAICampaign.decode(from: try data(["action_url": "https://near.ai:443/models"])))
    }

    func testArtworkCannotEscapeTheConfiguredDirectoryOrUseSVG() throws {
        for url in ["https://other.example/native/a.png", "https://assets.example/elsewhere/a.png",
                    "https://assets.example/native/sub/a.png", "https://assets.example/native/../a.png",
                    "https://assets.example/native/%2e%2e/a.png", "https://assets.example/native/%252e%252e/a.png",
                    "https://assets.example/native/a.svg", "http://assets.example/native/a.png",
                    "https://user@assets.example/native/a.png", "https://assets.example:444/native/a.png"] {
            XCTAssertNil(PrivateAICampaign.decode(from: try data(["image_url": url]), endpoint: endpoint), url)
        }
        // Compiled copy has no endpoint; the fetch path supplies and checks it.
        XCTAssertNotNil(PrivateAICampaign.decode(from: try data(["image_url": "https://other.example/a.jpg"])))
    }
}
