import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class GlassSourceRowTests: XCTestCase {
    private func candidate(_ kind: SourceKind) -> SourceCandidate {
        SourceCandidate(source: kind, path: "/tmp/x", exists: true, sessionCount: 3, mostRecent: nil, relocatedByEnv: false)
    }

    /// A reported MODE is authoritative: the candidate's evidence never
    /// replaces the core's sentence for a watched or declined source.
    func test_aReportedModeOutranksTheCandidate() throws {
        let copy = try XCTUnwrap(TCSourceChecks.settingsCopy())
        let answer = SourceRowState.answer(
            copy: copy, kind: .codex, reportedMode: "watch", choice: .undecided, candidate: candidate(.codex))
        XCTAssertEqual(answer.line, TCSourceChecks.checkLine(tool: copy.tools["codex"]!.key, sourceMode: "watch"))
        XCTAssertNil(answer.candidatePath)
        XCTAssertNil(answer.evidence)
    }

    /// Undecided with no mode shows the candidate, and with no candidate the
    /// core's no-candidate sentence, never a path this shell invented.
    func test_undecidedShowsTheCandidateOrTheCoresNoCandidateLine() throws {
        let copy = try XCTUnwrap(TCSourceChecks.settingsCopy())
        let with = SourceRowState.answer(
            copy: copy, kind: .geminiCli, reportedMode: nil, choice: .undecided, candidate: candidate(.geminiCli))
        XCTAssertEqual(with.candidatePath, "/tmp/x")
        XCTAssertNotNil(with.evidence)
        let none = SourceRowState.answer(
            copy: copy, kind: .geminiCli, reportedMode: nil, choice: .undecided, candidate: nil)
        XCTAssertNil(none.candidatePath)
        let expected = copy.tools["gemini-cli"]?.explanation == nil ? copy.noCandidate : nil
        XCTAssertEqual(none.line, expected)
    }

    /// A watched choice with an empty path says a folder is selected and
    /// shows no path.
    func test_watchWithoutAPathShowsNoPath() throws {
        let copy = try XCTUnwrap(TCSourceChecks.settingsCopy())
        let answer = SourceRowState.answer(
            copy: copy, kind: .cline, reportedMode: nil, choice: .watch(path: ""), candidate: nil)
        XCTAssertEqual(answer.line, copy.selectedFolder)
        XCTAssertNil(answer.selectedPath)
    }

    /// The row keeps the legacy initialiser so the Folders step and Settings
    /// share it unchanged.
    func test_theRowKeepsTheLegacySignature() throws {
        let source = try String(
            contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent("Views/Settings/GlassSourceRow.swift"),
            encoding: .utf8)
        for label in ["kind:", "candidate:", "choice:", "reportedMode:", "onWatchCandidate:", "onChoose:", "onDecline:"] {
            XCTAssertTrue(source.contains(label), "GlassSourceRow lacks \(label)")
        }
    }
}
