import XCTest
@testable import TCShellCore

final class InstallGuidesTests: XCTestCase {
    func testGuidesAreReadBySourceSlugAndOnlyOverHttps() {
        let guides = InstallGuides(json: #"{"claude-code":"https://code.claude.com/docs/en/setup","gemini-cli":"http://example.com","cline":"https://docs.cline.bot/getting-started/installing-cline"}"#)
        XCTAssertEqual(guides.url(for: .claudeCode)?.absoluteString, "https://code.claude.com/docs/en/setup")
        XCTAssertEqual(guides.url(for: .cline)?.host, "docs.cline.bot")
        XCTAssertNil(guides.url(for: .geminiCli))
        XCTAssertNil(guides.url(for: .codex))
        XCTAssertNil(InstallGuides(json: nil).url(for: .codex))
        XCTAssertNil(InstallGuides(json: "not json").url(for: .codex))
    }
}
