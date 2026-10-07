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

    /// Kristi b#10: a declared folder of exported traces has a row of its
    /// own while the daemon reports one watched or turned off: the core's
    /// line for that mode, and an off switch only while it is watched. With
    /// none declared there is no row. The off switch writes exactly the
    /// `trajectory_source` off declaration.
    func test_aTrajectoryFolderHasARowWithAnOffSwitch() throws {
        let copy = try XCTUnwrap(TCSourceChecks.settingsCopy()?.trajectory)
        let watched = try XCTUnwrap(TrajectoryRowState.answer(copy: copy, mode: "watch"))
        XCTAssertEqual(watched.line, copy.watching)
        XCTAssertTrue(watched.canTurnOff)
        let off = try XCTUnwrap(TrajectoryRowState.answer(copy: copy, mode: "off"))
        XCTAssertEqual(off.line, copy.off)
        XCTAssertFalse(off.canTurnOff)
        XCTAssertNil(TrajectoryRowState.answer(copy: copy, mode: "unset"))
        XCTAssertNil(TrajectoryRowState.answer(copy: copy, mode: nil))
        XCTAssertTrue(copy.explanation.contains("wait for you"))
        let params = TrajectoryRowState.offParams
        XCTAssertEqual(params.count, 1)
        XCTAssertEqual(
            (params[SessionRoots.trajectorySettingsKey] as? [String: String]), ["mode": "off"])
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
    /// share it unchanged: a signature change fails to compile here.
    func test_theRowKeepsTheLegacySignature() {
        let withMode = GlassSourceRow(
            kind: .codex, candidate: nil, choice: .undecided, reportedMode: "unset",
            onWatchCandidate: { _ in }, onChoose: { _ in }, onDecline: {})
        let withoutMode = GlassSourceRow(
            kind: .codex, candidate: nil, choice: .undecided,
            onWatchCandidate: { _ in }, onChoose: { _ in }, onDecline: {})
        XCTAssertEqual(withMode.kind, withoutMode.kind)
        XCTAssertEqual(withMode.reportedMode, "unset")
        XCTAssertNil(withoutMode.reportedMode)
    }
}
