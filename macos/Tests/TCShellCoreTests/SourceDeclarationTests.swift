import XCTest

@testable import TCShellCore

/// One answer about one source, as a `set_settings` object -- the shape the
/// Settings screen writes when a contributor changes a watched folder after
/// first run. The roots screen writes all of them at once through
/// `SessionRoots.settingsJSON`; this is the per-row form, and both must
/// spell the keys the same way.
final class SourceDeclarationTests: XCTestCase {
    func testEveryKindHasTheKeyTheDaemonValidates() {
        XCTAssertEqual(SourceKind.claudeCode.settingsKey, "claude_source")
        XCTAssertEqual(SourceKind.codex.settingsKey, "codex_source")
        XCTAssertEqual(SourceKind.geminiCli.settingsKey, "gemini_source")
        XCTAssertEqual(SourceKind.cline.settingsKey, "cline_source")
    }

    func testWatchingDeclaresTheModeAndThePath() throws {
        let params = try XCTUnwrap(
            SourceChoice.watch(path: "/Users/someone/.gemini/tmp").settingsParams(for: .geminiCli)
        )
        XCTAssertEqual(Set(params.keys), ["gemini_source"])
        XCTAssertEqual(
            params["gemini_source"] as? [String: String],
            ["mode": "watch", "path": "/Users/someone/.gemini/tmp"]
        )
    }

    func testDecliningSaysOff() throws {
        let params = try XCTUnwrap(SourceChoice.off.settingsParams(for: .cline))
        XCTAssertEqual(params["cline_source"] as? [String: String], ["mode": "off"])
    }

    func testAnUnfinishedAnswerIsNotSent() {
        // Undecided is the screen's opening state and never a declaration;
        // a blank path is a row still being filled in.
        XCTAssertNil(SourceChoice.undecided.settingsParams(for: .codex))
        XCTAssertNil(SourceChoice.watch(path: "   ").settingsParams(for: .codex))
    }

    func testThePerRowKeysAreTheOnesTheRootsScreenWrites() throws {
        // Two writers of the same file must speak one dialect.
        let roots = SessionRoots(
            claude: .watch(path: "/a"), codex: .off, gemini: .watch(path: "/g"), cline: .off)
        let whole = try JSONSerialization.jsonObject(
            with: Data(try XCTUnwrap(roots.settingsJSON()).utf8)) as? [String: Any]
        XCTAssertEqual(
            Set(try XCTUnwrap(whole).keys),
            Set(SourceKind.allCases.map(\.settingsKey))
        )
    }
}
