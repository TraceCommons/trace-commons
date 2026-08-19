import XCTest

@testable import TCShellCore

final class SessionRootsTests: XCTestCase {
    private func decode(_ json: String) throws -> [String: String] {
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
        return try XCTUnwrap(object as? [String: String])
    }

    func testCarriesBothRootsAndNothingElse() throws {
        let roots = SessionRoots(claude: "/Users/someone/.claude", codex: "/Users/someone/.codex")
        let decoded = try decode(try XCTUnwrap(roots.settingsJSON()))

        XCTAssertEqual(
            decoded,
            [
                "claude_root": "/Users/someone/.claude",
                "codex_root": "/Users/someone/.codex",
            ],
            "an unrecognized key is rejected by the settings validator, not ignored, "
                + "so this object must carry exactly the two fields it means"
        )
    }

    func testAFolderNameContainingQuotesSurvivesTheRoundTrip() throws {
        // The contributor picks these from a file panel. A folder may
        // legitimately contain a quote or a backslash, and string
        // concatenation would produce settings JSON that either fails to
        // parse or -- worse -- parses as a different path.
        let awkward = #"/Users/someone/He said "hi"\back"#
        let roots = SessionRoots(claude: awkward, codex: "/Users/someone/.codex")
        let decoded = try decode(try XCTUnwrap(roots.settingsJSON()))

        XCTAssertEqual(decoded["claude_root"], awkward)
    }

    func testAnIncompleteDeclarationProducesNoSettingsAtAll() {
        // Half a declaration is refused by the daemon anyway; producing it
        // here would only turn a clear "you have not finished" into an
        // opaque round trip through the ABI.
        XCTAssertNil(SessionRoots(claude: "/a", codex: "").settingsJSON())
        XCTAssertNil(SessionRoots(claude: "", codex: "/b").settingsJSON())
        XCTAssertNil(SessionRoots(claude: "  ", codex: "/b").settingsJSON())
    }

    func testIsCompleteMatchesWhetherSettingsCanBeBuilt() {
        XCTAssertTrue(SessionRoots(claude: "/a", codex: "/b").isComplete)
        XCTAssertFalse(SessionRoots(claude: "/a", codex: "").isComplete)
        XCTAssertFalse(SessionRoots(claude: "", codex: "").isComplete)
    }
}
