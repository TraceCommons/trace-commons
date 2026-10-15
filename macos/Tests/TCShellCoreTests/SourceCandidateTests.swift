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
        // "0 traces" and "that folder is not here" are materially
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
        XCTAssertEqual(empty.evidence(now: Date()), "Found, but holding no traces yet")
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
        XCTAssertEqual(candidate.evidence(now: now), "953 traces, most recent 2 hours ago")
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
        XCTAssertEqual(candidate.evidence(now: now), "1 trace, most recent just now")
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

    func testAnswersAtDecodesWhenPresentAndTheDaemonMayOmitIt() throws {
        let withVendor = """
            [{"source":"claude-code","path":"/p","exists":true,
              "session_count":1,"most_recent":null,"relocated_by_env":false,
              "answers_at":"Anthropic"}]
            """
        let decoded = try SourceCandidate.decodeList(from: withVendor)
        XCTAssertEqual(decoded[0].answersAt, "Anthropic")

        // A payload from a daemon built before this field existed must still
        // decode, with the field simply absent rather than the row dropped.
        // Inline on purpose: a shared fixture could later gain the field and
        // leave this assertion testing nothing.
        let olderDaemon = """
            [{"source":"claude-code","path":"/p","exists":true,
              "session_count":1,"most_recent":null,"relocated_by_env":false}]
            """
        XCTAssertFalse(olderDaemon.contains("answers_at"))
        let candidates = try SourceCandidate.decodeList(from: olderDaemon)
        XCTAssertEqual(candidates.count, 1)
        XCTAssertNil(candidates[0].answersAt)
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

    /// What `tc_describe_folder` returns for a flat folder of `.json`
    /// exports: an OpenCode row and a trajectory row, both naming the picked
    /// folder. Both survive, so the shell has two matches to ask between
    /// rather than one it would take as certain.
    func testADescribedFlatJsonFolderKeepsBothRows() throws {
        let json = """
            [{"source":"opencode","path":"/Users/someone/exports","exists":true,
              "session_count":2,"most_recent":"2026-10-04T09:00:00.123456789Z",
              "relocated_by_env":false,"answers_at":null},
             {"source":"trajectory","path":"/Users/someone/exports","exists":true,
              "session_count":2,"most_recent":"2026-10-04T09:00:00.123456789Z",
              "relocated_by_env":false,"answers_at":null}]
            """
        let matches = try FolderMatch.decodeList(from: json)
        XCTAssertEqual(matches.map(\.kind), [.source(.opencode), .trajectory])
        XCTAssertEqual(matches.map(\.path), ["/Users/someone/exports", "/Users/someone/exports"])
        XCTAssertEqual(matches.map(\.sessionCount), [2, 2])
        XCTAssertTrue(matches.allSatisfy { $0.mostRecent != nil })
        XCTAssertEqual(matches[0].candidate?.source, .opencode)
        XCTAssertNil(matches[1].candidate)
    }

    /// A trajectory-only export is one trajectory match, not no match: a
    /// declared Letta trajectory folder is a recognised kind.
    func testADescribedJsonlFolderIsOneTrajectoryRow() throws {
        let json = """
            [{"source":"trajectory","path":"/Users/someone/runs","exists":true,
              "session_count":3,"most_recent":null,
              "relocated_by_env":false,"answers_at":null}]
            """
        let matches = try FolderMatch.decodeList(from: json)
        XCTAssertEqual(matches.map(\.kind), [.trajectory])
        XCTAssertEqual(matches.first?.sessionCount, 3)
    }

    /// A slug this build has never heard of is kept as a match it cannot
    /// name, not dropped: dropping it would turn two matches into one and
    /// the shell would stop asking.
    func testADescribedFolderKeepsAnUnknownKind() throws {
        let json = """
            [{"source":"codex","path":"/p","exists":true,"session_count":1,
              "most_recent":null,"relocated_by_env":false,"answers_at":"OpenAI"},
             {"source":"some-future-tool","path":"/p","exists":true,"session_count":1,
              "most_recent":null,"relocated_by_env":false,"answers_at":null}]
            """
        let matches = try FolderMatch.decodeList(from: json)
        XCTAssertEqual(matches.map(\.kind), [.source(.codex), .unrecognised("some-future-tool")])
        XCTAssertEqual(matches[0].answersAt, "OpenAI")
    }

    /// A folder that matches no layout is an empty array, and decodes to no
    /// matches rather than failing.
    func testAnUnrecognisedFolderDecodesToNoMatches() throws {
        XCTAssertEqual(try FolderMatch.decodeList(from: "[]"), [])
    }
}
