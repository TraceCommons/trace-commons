import XCTest

@testable import TCShellCore

final class SourceCandidateTests: XCTestCase {
    /// Exactly what `tc_discover_sources` returned on the machine this was
    /// written on, pasted rather than paraphrased. The nine-digit fractional
    /// second is the detail worth pinning: it is what the Rust side actually
    /// emits, and a date parser that only tolerates three silently drops
    /// every timestamp to nil, which shows up as "no sessions" on a store
    /// holding three thousand.
    private let realOutput = """
        [
          {
            "source": "claude-code",
            "path": "/Users/someone/.claude/projects",
            "exists": true,
            "session_count": 953,
            "most_recent": "2026-08-19T12:28:34.412518838Z",
            "relocated_by_env": false
          },
          {
            "source": "codex",
            "path": "/Users/someone/.codex/sessions",
            "exists": true,
            "session_count": 3066,
            "most_recent": "2026-08-19T06:24:03.081319881Z",
            "relocated_by_env": false
          }
        ]
        """

    func testDecodesWhatTheAbiActuallyReturns() throws {
        let candidates = try SourceCandidate.decodeList(from: realOutput)

        XCTAssertEqual(candidates.count, 2)
        XCTAssertEqual(candidates[0].source, .claudeCode)
        XCTAssertEqual(candidates[0].path, "/Users/someone/.claude/projects")
        XCTAssertTrue(candidates[0].exists)
        XCTAssertEqual(candidates[0].sessionCount, 953)
        XCTAssertFalse(candidates[0].relocatedByEnv)
        XCTAssertEqual(candidates[1].source, .codex)
        XCTAssertEqual(candidates[1].sessionCount, 3066)
    }

    func testANineDigitFractionalSecondStillParses() throws {
        let candidates = try SourceCandidate.decodeList(from: realOutput)
        let recent = try XCTUnwrap(
            candidates[0].mostRecent,
            "a timestamp the ABI really emits must not decode to nil"
        )
        // 2026-08-19T12:28:34Z
        XCTAssertEqual(recent.timeIntervalSince1970, 1_787_142_514, accuracy: 1)
    }

    func testAStoreThatIsNotThereSaysSoRatherThanShowingZero() {
        let missing = SourceCandidate(
            source: .codex,
            path: "/Users/someone/.codex/sessions",
            exists: false,
            sessionCount: 0,
            mostRecent: nil,
            relocatedByEnv: false
        )
        // "0 sessions" and "that folder is not here" are materially
        // different answers to "may I watch this", and a contributor
        // deciding between them deserves the difference.
        XCTAssertEqual(missing.evidence(now: Date()), "Not found on this machine")
    }

    func testAnEmptyStoreIsDistinctFromAMissingOne() {
        let empty = SourceCandidate(
            source: .codex,
            path: "/Users/someone/.codex/sessions",
            exists: true,
            sessionCount: 0,
            mostRecent: nil,
            relocatedByEnv: false
        )
        XCTAssertEqual(empty.evidence(now: Date()), "Found, but holding no sessions yet")
    }

    func testEvidenceCountsSessionsAndSaysHowRecent() {
        let now = Date(timeIntervalSince1970: 1_787_142_514)
        let candidate = SourceCandidate(
            source: .claudeCode,
            path: "/Users/someone/.claude/projects",
            exists: true,
            sessionCount: 953,
            mostRecent: now.addingTimeInterval(-2 * 60 * 60),
            relocatedByEnv: false
        )
        XCTAssertEqual(candidate.evidence(now: now), "953 sessions, most recent 2 hours ago")
    }

    func testASingleSessionIsNotPluralised() {
        let now = Date(timeIntervalSince1970: 1_787_142_514)
        let candidate = SourceCandidate(
            source: .codex,
            path: "/p",
            exists: true,
            sessionCount: 1,
            mostRecent: now.addingTimeInterval(-90),
            relocatedByEnv: false
        )
        XCTAssertEqual(candidate.evidence(now: now), "1 session, most recent just now")
    }

    func testRelocationIsSurfacedSoAnUnusualPathHasAnExplanation() {
        let candidate = SourceCandidate(
            source: .claudeCode,
            path: "/elsewhere/projects",
            exists: true,
            sessionCount: 4,
            mostRecent: nil,
            relocatedByEnv: true
        )
        // Without this the screen shows a path the contributor did not
        // expect and offers no reason for it.
        XCTAssertTrue(candidate.evidence(now: Date()).contains("moved here by an environment variable"))
    }

    func testDisplayNamesAreTheProductNamesNotTheAdapterSlugs() {
        XCTAssertEqual(SourceKind.claudeCode.displayName, "Claude Code")
        XCTAssertEqual(SourceKind.codex.displayName, "Codex")
    }

    func testAnUnknownSourceSlugIsIgnoredRatherThanCrashingTheScreen() throws {
        // A future adapter this build has never heard of must not take the
        // roots screen down with it; the screen would then be unreachable
        // and the daemon unstartable.
        let json = """
            [{"source":"something-new","path":"/p","exists":true,
              "session_count":1,"most_recent":null,"relocated_by_env":false}]
            """
        XCTAssertEqual(try SourceCandidate.decodeList(from: json).count, 0)
    }
}
