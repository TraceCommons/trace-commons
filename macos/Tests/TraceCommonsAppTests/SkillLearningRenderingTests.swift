// INTEGRATION: renders every tested-skill stage at the native app's compact
// and desktop widths in both intentional color schemes.

import AppKit
import SwiftUI
import TCBridge
import XCTest

@testable import TraceCommonsApp

final class SkillLearningRenderingTests: XCTestCase {
    private var daemon: SkillLearningDaemon!
    private var client: DaemonClient!

    override func setUp() {
        super.setUp()
        daemon = SkillLearningDaemon()
        client = DaemonClient(daemon: daemon)
    }

    @MainActor
    func testEverySkillStageRendersAtMobileAndDesktopWidths() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        let copy = try XCTUnwrap(model.skillLearningCopy)

        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model) != nil }
        try await assertStageRenders("candidate", model: model, copy: copy)

        model.reviewSkill(
            from: record,
            draft: try XCTUnwrap(candidate(in: model)).draft
        )
        try await waitUntil { self.review(in: model) != nil }
        try await assertStageRenders("review", model: model, copy: copy)

        model.testSkill(from: record)
        try await waitUntil { self.evaluation(in: model) != nil }
        try await assertStageRenders("results", model: model, copy: copy)

        model.reviewSkillInstall(from: record)
        try await waitUntil { self.installPlan(in: model) != nil }
        try await assertStageRenders("install-preview", model: model, copy: copy)

        model.installSkill(from: record)
        try await waitUntil { self.installedSkill(in: model) != nil }
        try await assertStageRenders("installed", model: model, copy: copy)
    }

    @MainActor
    func testInstallCommitReplyLossRecoversReceiptAndSupportsRollback() async throws {
        let model = AppModel()
        model.setClientForTesting(client)

        model.learnSkill(from: record)
        try await waitUntil { self.candidate(in: model) != nil }
        model.reviewSkill(
            from: record,
            draft: try XCTUnwrap(candidate(in: model)).draft
        )
        try await waitUntil { self.review(in: model) != nil }
        model.testSkill(from: record)
        try await waitUntil { self.evaluation(in: model) != nil }
        model.reviewSkillInstall(from: record)
        try await waitUntil { self.installPlan(in: model) != nil }

        let statusCallsBeforeCommit = statusCallSubmissionIDs().count
        daemon.setInstallCommitReplyFailure("skill-install-write-failed")
        model.installSkill(from: record)
        try await waitUntil {
            self.installedSkill(in: model) != nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }

        let recovered = try XCTUnwrap(installedSkill(in: model))
        XCTAssertNil(model.skillLearningState(for: record.submissionID).failure)
        XCTAssertEqual(statusCallSubmissionIDs().count, statusCallsBeforeCommit + 1)
        XCTAssertEqual(
            daemon.calls.suffix(2).map(\.method),
            ["skill_install_commit", "skill_install_status"]
        )

        model.rollbackSkill(from: record)
        try await waitUntil {
            self.installedSkill(in: model) == nil
                && !model.skillLearningState(for: self.record.submissionID).isWorking
        }
        XCTAssertEqual(
            daemon.calls.last?.method,
            "skill_install_rollback"
        )
        XCTAssertTrue(daemon.calls.last?.params.contains(recovered.installID) == true)
    }

    @MainActor
    func testAcceptedSessionFirstOpenRecoversInstallAfterRestartPerSession() async throws {
        let first = record
        let second = record(submissionID: "88888888-8888-4888-8888-888888888888")
        daemon.seedInstalledSkill(
            name: "repair-generated-artifacts",
            sourceSubmissionID: first.submissionID
        )

        let restartedModel = AppModel()
        restartedModel.setClientForTesting(client)
        let copy = try XCTUnwrap(restartedModel.skillLearningCopy)

        let firstWindow = host(
            SkillLearningView(record: first, copy: copy).environmentObject(restartedModel)
        )
        defer { firstWindow.close() }
        try await waitUntil {
            self.installedSkill(in: restartedModel, id: first.submissionID) != nil
        }

        let secondWindow = host(
            SkillLearningView(record: second, copy: copy).environmentObject(restartedModel)
        )
        defer { secondWindow.close() }
        try await waitUntil {
            !restartedModel.skillLearningState(for: second.submissionID).isWorking
                && self.statusCallSubmissionIDs().contains(second.submissionID)
        }
        try await Task.sleep(for: .milliseconds(100))

        XCTAssertEqual(
            installedSkill(in: restartedModel, id: first.submissionID)?.name,
            "repair-generated-artifacts"
        )
        guard case .idle = restartedModel.skillLearningState(for: second.submissionID).phase else {
            return XCTFail("A different accepted session must remain idle")
        }
        XCTAssertEqual(
            statusCallSubmissionIDs(),
            [first.submissionID, second.submissionID]
        )
        XCTAssertFalse(daemon.calls.contains { $0.method == "skill_candidate" })
    }

    func testSkillReviewControlsExposeLabelsAndMinimumTargets() throws {
        let source = try skillLearningViewSource()

        assertControl(
            in: source,
            from: #"TextField("", text: $draft.name)"#,
            to: "fieldLabel(",
            label: ".accessibilityLabel(copy.name)",
            minimumHeight: 44
        )
        assertControl(
            in: source,
            from: "TextEditor(text: $draft.description)",
            to: "fieldLabel(",
            label: ".accessibilityLabel(copy.applicability)",
            minimumHeight: 96
        )
        assertControl(
            in: source,
            from: "TextEditor(text: $draft.procedure)",
            to: "DisclosureGroup(copy.sourceEvidence)",
            label: ".accessibilityLabel(copy.procedure)",
            minimumHeight: 240
        )
        assertControl(
            in: source,
            from: "DisclosureGroup(copy.sourceEvidence)",
            to: "evaluationContract(candidate)",
            label: ".accessibilityLabel(copy.sourceEvidence)",
            minimumHeight: 44
        )
        assertControl(
            in: source,
            from: "DisclosureGroup(copy.inspectRuns)",
            to: "private struct SkillTrialRow",
            label: ".accessibilityLabel(copy.inspectRuns)",
            minimumHeight: 44
        )
        assertControl(
            in: source,
            from: "Link(copy.openFixtureSource, destination: trial.sourceURL)",
            to: "ForEach(trial.failureReasons",
            label: #".accessibilityLabel("\(copy.openFixtureSource): \(trial.taskID)")"#,
            minimumHeight: 44
        )
    }

    @MainActor
    private func assertStageRenders(
        _ stage: String,
        model: AppModel,
        copy: SkillLearningCopy
    ) async throws {
        let configuredDirectory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"]
            .flatMap { $0.isEmpty ? nil : URL(fileURLWithPath: $0) }
        let directory = configuredDirectory
            ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer {
            if configuredDirectory == nil { try? FileManager.default.removeItem(at: directory) }
        }

        for width in [375, 760] {
            var appearances: [Data] = []
            for (label, scheme) in [("light", ColorScheme.light), ("dark", ColorScheme.dark)] {
                let image = try await capture(
                    SkillLearningView(record: record, copy: copy).environmentObject(model),
                    width: CGFloat(width),
                    scheme: scheme
                )
                try image.write(
                    to: directory.appendingPathComponent(
                        "skill-" + stage + "-" + String(width) + "-" + label + ".png"
                    )
                )
                XCTAssertGreaterThan(image.count, 1_000)
                appearances.append(image)
            }
            XCTAssertNotEqual(appearances[0], appearances[1])
        }
    }

    @MainActor
    private func waitUntil(
        timeout: Duration = .seconds(3),
        condition: @escaping @MainActor () -> Bool
    ) async throws {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: timeout)
        while !condition() {
            if clock.now >= deadline {
                XCTFail("Timed out waiting for tested-skill state")
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    @MainActor
    private func capture<V: View>(
        _ view: V,
        width: CGFloat,
        scheme: ColorScheme
    ) async throws -> Data {
        _ = NSApplication.shared
        let content = view
            .padding(24)
            .frame(width: width, alignment: .topLeading)
            .fixedSize(horizontal: false, vertical: true)
            .background(Color(nsColor: .windowBackgroundColor))
            .environment(\.colorScheme, scheme)
        let hosting = NSHostingView(rootView: content)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: 1_600),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.contentView = hosting
        defer { window.close() }
        await Task.yield()
        hosting.layoutSubtreeIfNeeded()
        let height = ceil(hosting.fittingSize.height)
        XCTAssertGreaterThan(height, 300)
        XCTAssertLessThan(height, 4_500)
        let bounds = NSRect(x: 0, y: 0, width: width, height: height)
        window.setContentSize(bounds.size)
        hosting.frame = bounds
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }

    @MainActor
    private func host<V: View>(_ view: V) -> NSWindow {
        _ = NSApplication.shared
        let hosting = NSHostingView(
            rootView: view
                .frame(width: 760, alignment: .topLeading)
                .environment(\.colorScheme, .dark)
        )
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 760, height: 1_000),
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        window.orderFrontRegardless()
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        return window
    }

    @MainActor
    private func candidate(in model: AppModel) -> SkillCandidate? {
        model.skillLearningState(for: record.submissionID).phase.candidate
    }

    @MainActor
    private func review(in model: AppModel) -> SkillReview? {
        guard case .reviewed(_, let review) = model
            .skillLearningState(for: record.submissionID).phase
        else { return nil }
        return review
    }

    @MainActor
    private func evaluation(in model: AppModel) -> SkillEvaluationReport? {
        guard case .evaluated(_, _, let report) = model
            .skillLearningState(for: record.submissionID).phase
        else { return nil }
        return report
    }

    @MainActor
    private func installPlan(in model: AppModel) -> SkillInstallPlan? {
        guard case .planned(_, _, _, let plan) = model
            .skillLearningState(for: record.submissionID).phase
        else { return nil }
        return plan
    }

    @MainActor
    private func installedSkill(in model: AppModel, id: String? = nil) -> InstalledSkill? {
        model.skillLearningState(for: id ?? record.submissionID).installedSkill
    }

    private func statusCallSubmissionIDs() -> [String] {
        daemon.calls.compactMap { call in
            guard call.method == "skill_install_status",
                  let data = call.params.data(using: .utf8),
                  let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            else { return nil }
            return object["source_submission_id"] as? String
        }
    }

    private func skillLearningViewSource() throws -> String {
        let macOSDirectory = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
        let sourceURL = macOSDirectory
            .appendingPathComponent("Sources")
            .appendingPathComponent("TraceCommonsApp")
            .appendingPathComponent("Views")
            .appendingPathComponent("SkillLearningView.swift")
        return try String(contentsOf: sourceURL, encoding: .utf8)
    }

    private func assertControl(
        in source: String,
        from start: String,
        to end: String,
        label: String,
        minimumHeight: Int,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        guard let startRange = source.range(of: start),
              let endRange = source.range(
                of: end,
                range: startRange.upperBound..<source.endIndex
              )
        else {
            return XCTFail("Missing control source region: \(start)", file: file, line: line)
        }
        let region = String(source[startRange.lowerBound..<endRange.lowerBound])
        XCTAssertTrue(region.contains(label), "Missing label for \(start)", file: file, line: line)
        XCTAssertTrue(
            region.contains(".frame(minHeight: \(minimumHeight)"),
            "Missing \(minimumHeight)pt target for \(start)",
            file: file,
            line: line
        )
    }

    private var record: HistoryRecord {
        record(submissionID: "22222222-2222-4222-8222-222222222222")
    }

    private func record(submissionID: String) -> HistoryRecord {
        HistoryRecord(
            submissionID: submissionID,
            submittedAt: Date(timeIntervalSince1970: 1_789_000_000),
            projectID: "project-test",
            projectLabel: "Synthetic project",
            source: "codex",
            status: "accepted",
            consentScopes: ["code_content"],
            creditPointsPending: 0,
            creditPointsFinal: 1,
            explanations: [],
            lastRefreshedAt: nil
        )
    }
}
