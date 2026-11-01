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

    func test_nativeOpenCodeDatabaseRemainsAnExplicitSettingsCandidate() throws {
        let copy = try XCTUnwrap(TCSourceChecks.settingsCopy())
        let candidate = SourceCandidate(
            source: .opencode, path: "/synthetic/opencode/opencode.db", exists: true,
            sessionCount: 2, mostRecent: nil, relocatedByEnv: false)
        for mode in [nil, "unset"] as [String?] {
            let answer = SourceRowState.answer(
                copy: copy, kind: .opencode, reportedMode: mode,
                choice: .undecided, candidate: candidate)
            XCTAssertEqual(answer.candidatePath, "/synthetic/opencode/opencode.db")
            XCTAssertNotNil(answer.evidence)
            XCTAssertNil(answer.selectedPath)
        }
        let watched = SourceRowState.answer(
            copy: copy, kind: .opencode, reportedMode: nil,
            choice: .watch(path: "/synthetic/opencode/opencode.db"), candidate: candidate)
        XCTAssertEqual(watched.selectedPath, "/synthetic/opencode/opencode.db")
        XCTAssertNil(watched.candidatePath)
        let declined = SourceRowState.answer(
            copy: copy, kind: .opencode, reportedMode: "off",
            choice: .undecided, candidate: candidate)
        XCTAssertEqual(declined.line, TCSourceChecks.checkLine(tool: "opencode", sourceMode: "off"))
        XCTAssertNil(declined.candidatePath)
        XCTAssertNil(declined.selectedPath)
        XCTAssertNil(declined.evidence)
    }

    func test_newOpenCodeFolderSelectionUsesRecognisedNativeDatabase() {
        let native = FolderMatch(
            kind: .source(.opencode), path: "/synthetic/opencode/opencode.db",
            sessionCount: 2, mostRecent: nil)
        XCTAssertEqual(SourceFolderSelection.path(
            for: .opencode, chosenFolder: "/synthetic/opencode", matches: [native]),
            "/synthetic/opencode/opencode.db")
        XCTAssertEqual(SourceFolderSelection.path(
            for: .opencode, chosenFolder: "/synthetic/other", matches: [native]),
            "/synthetic/other")
        for kind in SourceKind.allCases where kind != .opencode {
            XCTAssertEqual(SourceFolderSelection.path(
                for: kind, chosenFolder: "/synthetic/opencode", matches: [native]),
                "/synthetic/opencode")
        }
    }

    func test_newOpenCodeExportFolderSelectionKeepsItsDirectory() {
        let export = FolderMatch(
            kind: .source(.opencode), path: "/synthetic/exports",
            sessionCount: 2, mostRecent: nil)
        let trajectory = FolderMatch(
            kind: .trajectory, path: "/synthetic/exports", sessionCount: 2, mostRecent: nil)
        let native = FolderMatch(
            kind: .source(.opencode), path: "/synthetic/exports/opencode.db",
            sessionCount: 2, mostRecent: nil)
        for matches in [[], [export], [export, trajectory], [native, trajectory]] {
            XCTAssertEqual(SourceFolderSelection.path(
                for: .opencode, chosenFolder: "/synthetic/exports", matches: matches),
                "/synthetic/exports")
        }
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
    @MainActor
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
