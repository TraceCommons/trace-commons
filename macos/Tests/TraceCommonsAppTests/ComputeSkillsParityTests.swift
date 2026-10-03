import XCTest
@testable import TraceCommonsApp

final class ComputeSkillsParityTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Zero and non-numbers are refused; the daemon refuses an allowance of
    /// nothing and a typed word is not an allowance.
    func test_theAllowanceParsesOnlyAPositiveInteger() {
        XCTAssertNil(ComputeAllowance.parse(""))
        XCTAssertNil(ComputeAllowance.parse("0"))
        XCTAssertNil(ComputeAllowance.parse("8 GiB"))
        XCTAssertEqual(ComputeAllowance.parse("8"), 8)
    }

    func test_computeKeepsEveryControlAndCopySource() throws {
        let source = try Self.text("Views/ComputeView.swift")
        for needle in ["ComputeContent(model:", "snapshot.copy.introduction", "snapshot.copy.allowanceLabel",
                       "snapshot.copy.allowanceDetail", "snapshot.copy.resume", "snapshot.copy.pause",
                       "snapshot.copy.disable", "snapshot.copy.enable", "snapshot.canEnable", "snapshot.canResume",
                       "snapshot.canPause", "snapshot.available", "snapshot.consentGranted", "model.controlsBusy",
                       "model.quitWasRefused", "copy.quitRefused", "copy.unavailable", "copy.retry", "model.failureLabel",
                       ".enable(ramAllowanceGiB:", ".perform(.resume)", ".perform(.pause)", ".perform(.disable)",
                       "model.retryOpen()", "ComputeAllowance.parse(", "GlassTextField("] {
            XCTAssertTrue(source.contains(needle), "ComputeView.swift lacks \(needle)")
        }
        XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression))
    }

    /// The inspector finds the legacy record by the row's submission id, and
    /// finds nothing (so draws no Skills panel) for a row it does not hold.
    func test_theInspectorResolvesTheRecordBySubmissionId() {
        let record = HistoryRecord(
            submissionID: "s-1", submittedAt: Date(timeIntervalSince1970: 1_789_000_000), projectID: "p-1",
            projectLabel: "payments-api", source: "codex", status: "accepted", consentScopes: [],
            creditPointsPending: 0, creditPointsFinal: nil, explanations: [], lastRefreshedAt: nil)
        XCTAssertEqual(HistorySelection.record(for: "s-1", in: [record])?.submissionID, "s-1")
        XCTAssertNil(HistorySelection.record(for: "s-2", in: [record]))
        XCTAssertNil(HistorySelection.record(for: "", in: [record]))
    }

    func test_skillsLivesInTheHistoryInspector() throws {
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        XCTAssertTrue(inspector.contains("SkillLearningView(record: record, copy: copy)"))
        XCTAssertTrue(inspector.contains("model.skillLearningCopy"))
        // A new row is a new panel: the draft and the install status are per
        // record, and the inspector pane outlives a change of selection.
        XCTAssertTrue(inspector.contains(".id(record.submissionID)"))
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("HistoryDetailInspector(row: row)"))
        XCTAssertTrue(window.contains("@SceneStorage(\"monitor.selectedHistory\") private var selectedHistory = \"\""))
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("HistoryPage(store: store, statusLabel: statusLabel, selection: $selection,"))
        XCTAssertTrue(home.contains("selection = row.submissionId"))
    }

    func test_skillLearningKeepsEveryCallAndAuthorsNothing() throws {
        let source = try Self.text("Views/SkillLearningView.swift")
        for needle in ["model.skillLearningState(for:", "model.learnSkill(from:", "model.reviewSkill(from:",
                       "model.testSkill(from:", "model.editSkill(from:", "model.reviewSkillInstall(from:",
                       "model.installSkill(from:", "model.rollbackSkill(from:", "model.ensureLocalInstalledSkillStatus(for:",
                       "TCSkillLearning.validateDraftJSON(", "TCSkillLearning.errorLine(label:", "copy.rollbackDisclosure",
                       "copy.evaluationDisclosure", "GlassTextField(", "GlassEyebrowCard(copy.heading)",
                       "GlassExpander(copy.sourceEvidence,", "GlassExpander(copy.inspectRuns,",
                       "GlassExpander(copy.modelOutput,", "GlassNotice(tone: .outside)", ".task(id: draft)",
                       ".milliseconds(150)", "private struct SkillReviewPreview: View",
                       "private struct SkillEvaluationResults: View", "private struct SkillTrialRow: View",
                       "private struct SkillInstallPreview: View", "private struct InstalledSkillPanel: View"] {
            XCTAssertTrue(source.contains(needle), "SkillLearningView.swift lacks \(needle)")
        }
        XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression))
        XCTAssertFalse(source.contains("DisclosureGroup("))
        // A failure is never undismissable; the word is the core's, with the
        // banner's as the fallback by reference.
        XCTAssertTrue(source.contains(
            "Button(Self.dismissLabel ?? ActionMessageBanner.dismissWord) { dismissedFailure = message }"))
        XCTAssertTrue(source.contains("MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.dismiss"))
    }
}
