import XCTest

@testable import TCShellCore

final class SessionRootsTests: XCTestCase {
    private func decode(_ json: String) throws -> [String: Any] {
        let object = try JSONSerialization.jsonObject(with: Data(json.utf8))
        return try XCTUnwrap(object as? [String: Any])
    }

    private func declaration(_ decoded: [String: Any], _ key: String) throws -> [String: String] {
        try XCTUnwrap(decoded[key] as? [String: String], "\(key) must be a declaration object")
    }

    func testWatchingBothCarriesTwoDeclarationsAndNothingElse() throws {
        let roots = SessionRoots(
            claude: .watch(path: "/Users/someone/.claude/projects"),
            codex: .watch(path: "/Users/someone/.codex/sessions")
        )
        let decoded = try decode(try XCTUnwrap(roots.settingsJSON()))

        XCTAssertEqual(Set(decoded.keys), ["claude_source", "codex_source"],
            "the settings validator rejects an unknown top-level key rather than ignoring it")
        XCTAssertEqual(
            try declaration(decoded, "claude_source"),
            ["mode": "watch", "path": "/Users/someone/.claude/projects"]
        )
        XCTAssertEqual(
            try declaration(decoded, "codex_source"),
            ["mode": "watch", "path": "/Users/someone/.codex/sessions"]
        )
    }

    func testDecliningASourceSaysOffRatherThanLeavingItOut() throws {
        // The whole point of the off state. An omitted or null root reads as
        // "never asked", and the daemon answers that by watching the
        // conventional location -- the contributor's real ~/.codex. "I do
        // not use Codex" has to be a thing the file can say.
        let roots = SessionRoots(
            claude: .watch(path: "/Users/someone/.claude/projects"),
            codex: .off
        )
        let decoded = try decode(try XCTUnwrap(roots.settingsJSON()))

        XCTAssertEqual(try declaration(decoded, "codex_source"), ["mode": "off"])
    }

    func testTheLegacyPathSpellingIsNeverEmitted() throws {
        // `claude_root` cannot express off, so a shell that sometimes sends
        // one spelling and sometimes the other has two ways to say the same
        // thing and only one that can say all of it.
        let roots = SessionRoots(claude: .off, codex: .off)
        let json = try XCTUnwrap(roots.settingsJSON())

        XCTAssertFalse(json.contains("claude_root"))
        XCTAssertFalse(json.contains("codex_root"))
    }

    func testDecliningBothIsACompleteDeclarationThatWatchesNothing() throws {
        let roots = SessionRoots(claude: .off, codex: .off)

        XCTAssertTrue(roots.isComplete, "answering no to both is an answer")
        let decoded = try decode(try XCTUnwrap(roots.settingsJSON()))
        XCTAssertEqual(try declaration(decoded, "claude_source"), ["mode": "off"])
        XCTAssertEqual(try declaration(decoded, "codex_source"), ["mode": "off"])
    }

    func testAnUndecidedSourceProducesNoSettingsAtAll() {
        // Nothing is pre-selected, so undecided is the state the screen
        // opens in. Sending it would persist "never asked" and earn the
        // refusal the screen exists to clear.
        XCTAssertNil(SessionRoots(claude: .watch(path: "/a"), codex: .undecided).settingsJSON())
        XCTAssertNil(SessionRoots(claude: .undecided, codex: .off).settingsJSON())
        XCTAssertNil(SessionRoots().settingsJSON())
    }

    func testAWatchWithNoPathIsNotAnAnswer() {
        XCTAssertFalse(SessionRoots(claude: .watch(path: ""), codex: .off).isComplete)
        XCTAssertFalse(SessionRoots(claude: .watch(path: "   "), codex: .off).isComplete)
        XCTAssertNil(SessionRoots(claude: .watch(path: " "), codex: .off).settingsJSON())
    }

    func testAFolderNameContainingQuotesSurvivesTheRoundTrip() throws {
        // The contributor may pick these from a file panel. A folder may
        // legitimately contain a quote or a backslash, and string
        // concatenation would produce settings JSON that either fails to
        // parse or -- worse -- parses as a different path.
        let awkward = #"/Users/someone/He said "hi"\back"#
        let roots = SessionRoots(claude: .watch(path: awkward), codex: .off)
        let decoded = try decode(try XCTUnwrap(roots.settingsJSON()))

        XCTAssertEqual(try declaration(decoded, "claude_source")["path"], awkward)
    }

    func testIsCompleteMatchesWhetherSettingsCanBeBuilt() {
        let cases: [(SessionRoots, Bool)] = [
            (SessionRoots(claude: .watch(path: "/a"), codex: .watch(path: "/b")), true),
            (SessionRoots(claude: .watch(path: "/a"), codex: .off), true),
            (SessionRoots(claude: .off, codex: .off), true),
            (SessionRoots(claude: .watch(path: "/a"), codex: .undecided), false),
            (SessionRoots(), false),
        ]
        for (roots, expected) in cases {
            XCTAssertEqual(roots.isComplete, expected)
            XCTAssertEqual(roots.settingsJSON() != nil, expected,
                "isComplete and settingsJSON must never disagree")
        }
    }

    func testWhatIsSubmittedIsTheSessionStoreNotTheParentConfigDirectory() throws {
        // The screen this replaced offered "Use the standard locations" and
        // filled in ~/.claude and ~/.codex -- the CONFIG directories, not
        // the session stores. `~/.claude` also holds history.jsonl, plugins,
        // skills and each project's memory/, so declaring the parent asks
        // the contributor to agree to one thing and hands the watcher
        // another.
        //
        // The fix is structural: the shell no longer has an opinion about
        // where the stores are. It submits whatever discovery reported, and
        // discovery (source/discovery.rs) is the single place that knows.
        // This test pins the outcome so a "helpful" hardcoded default cannot
        // come back.
        let discovered = """
            [
              {"source":"claude-code","path":"/Users/someone/.claude/projects","exists":true,
               "session_count":953,"most_recent":null,"relocated_by_env":false},
              {"source":"codex","path":"/Users/someone/.codex/sessions","exists":true,
               "session_count":3066,"most_recent":null,"relocated_by_env":false}
            ]
            """
        var roots = SessionRoots()
        for candidate in try SourceCandidate.decodeList(from: discovered) {
            roots.watch(candidate)
        }
        let decoded = try decode(try XCTUnwrap(roots.settingsJSON()))

        let claudePath = try XCTUnwrap(declaration(decoded, "claude_source")["path"])
        let codexPath = try XCTUnwrap(declaration(decoded, "codex_source")["path"])

        XCTAssertTrue(claudePath.hasSuffix("/.claude/projects"), "got \(claudePath)")
        XCTAssertTrue(codexPath.hasSuffix("/.codex/sessions"), "got \(codexPath)")
        XCTAssertFalse(claudePath.hasSuffix("/.claude"), "the config directory is not the store")
        XCTAssertFalse(codexPath.hasSuffix("/.codex"), "the config directory is not the store")
    }

    func testAdoptingACandidateWatchesThePathDiscoveryFound() {
        let candidate = SourceCandidate(
            source: .claudeCode,
            path: "/Users/someone/.claude/projects",
            exists: true,
            sessionCount: 953,
            mostRecent: nil,
            relocatedByEnv: false
        )
        var roots = SessionRoots()
        roots.watch(candidate)

        XCTAssertEqual(roots.claude, .watch(path: "/Users/someone/.claude/projects"))
        XCTAssertEqual(roots.codex, .undecided, "adopting one source must not answer for the other")
    }
}
