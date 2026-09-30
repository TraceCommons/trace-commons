// INTEGRATION: exercises the reviewed session-to-publication flow through the
// real Swift client and AppModel, with a synthetic daemon response only.

import AppKit
import SwiftUI
import TCBridge
import XCTest
@testable import TraceCommonsApp

private final class SessionPublicationDaemon: DaemonCalling, @unchecked Sendable {
    struct Call {
        let method: String
        let params: String
    }

    private let lock = NSLock()
    private var recorded: [Call] = []
    private var failures: Set<String> = []
    private var credentialWarnings: Set<String> = []
    private var publicationVersion = 0
    private var ownerScopeSHA256 = "sha256:owner-a"

    var calls: [Call] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func setFailure(_ method: String, enabled: Bool = true) {
        lock.lock()
        defer { lock.unlock() }
        if enabled {
            failures.insert(method)
        } else {
            failures.remove(method)
        }
    }

    func setCredentialWarning(_ method: String, enabled: Bool = true) {
        lock.lock()
        defer { lock.unlock() }
        if enabled {
            credentialWarnings.insert(method)
        } else {
            credentialWarnings.remove(method)
        }
    }

    func setOwnerScopeSHA256(_ value: String) {
        lock.lock()
        defer { lock.unlock() }
        ownerScopeSHA256 = value
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append(Call(method: method, params: paramsJSON))
        if failures.contains(method) {
            let message: String
            switch method {
            case "history_detail": message = "session-detail-not-found"
            case "publish_public_run": message = "public-run-conflict"
            case "unpublish_public_run": message = "public-run-unpublish-failed"
            default: message = "synthetic-test-failure"
            }
            return #"{"id":1,"error":{"code":"unavailable","message":"\#(message)"}}"#
        }
        switch method {
        case "history_detail":
            return #"{"id":1,"result":{"owner_scope_sha256":"\#(ownerScopeSHA256)","task":"Repair the generated client without editing its checked-in output.","task_success":"partial","user_feedback":"correction","human_correction":"Use the bounded retry path.","evidence":[{"event_id":"11111111-1111-4111-8111-111111111111","kind":"tool_result","excerpt":"The bounded retry completed on the second attempt."}],"contribution_status":"accepted","permitted_uses":["debugging","evaluation"],"contributed_version":"trace-contribution/1","consent_policy_version":"consent/1","redaction_pipeline_version":"redaction/3","publication":null,"publication_version":\#(publicationVersion),"retained_source_slug":"source-workflow"}}"#
        case "publish_public_run":
            publicationVersion += 1
            let warning = credentialWarnings.contains(method)
                ? #", "credential_warning":"commons_credential_storage_unavailable""#
                : ""
            return #"{"id":1,"result":{"slug":"bounded-retry","title":"Bounded retry recovery","outcome_summary":"The workflow recovered after one limited retry.","workflow":"Retry once with the corrected bounded input.","reuse_permission":"cc_by_4_0","evidence":[{"excerpt":"The bounded retry completed on the second attempt."}],"task_success":"partial","contributed_version":"trace-contribution/1","version":1,"published_at":"2026-09-10T12:00:00Z","variations":[]\#(warning)}}"#
        case "unpublish_public_run":
            publicationVersion += 1
            let warning = credentialWarnings.contains(method)
                ? #", "credential_warning":"commons_credential_storage_unavailable""#
                : ""
            return #"{"id":1,"result":{"unpublished":true,"expected_publication_version":\#(publicationVersion)\#(warning)}}"#
        default:
            return #"{"id":1,"error":{"code":"unavailable","message":"unexpected-test-method"}}"#
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

final class SessionPublicationTests: XCTestCase {
    private var daemon: SessionPublicationDaemon!
    private var client: DaemonClient!

    override func setUp() {
        super.setUp()
        daemon = SessionPublicationDaemon()
        client = DaemonClient(daemon: daemon)
    }

    func testClientDecodesSessionDetailAndSendsOnlyTheReviewedDraft() throws {
        let detail = try client.sessionDetail(submissionID: record.submissionID)
        XCTAssertEqual(
            detail.task,
            "Repair the generated client without editing its checked-in output."
        )
        XCTAssertEqual(detail.taskSuccess, "partial")
        XCTAssertEqual(detail.ownerScopeSHA256, "sha256:owner-a")
        let copy = try XCTUnwrap(PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? ""))
        XCTAssertEqual(copy.taskOutcomeLabel(for: detail.taskSuccess), "Partly completed")
        XCTAssertEqual(copy.feedbackLabel(for: detail.userFeedback), "Feedback: correction supplied")
        XCTAssertEqual(detail.humanCorrection, "Use the bounded retry path.")
        XCTAssertEqual(detail.evidence.map(\.eventID), ["11111111-1111-4111-8111-111111111111"])
        XCTAssertEqual(detail.evidence.map(\.kind), [.toolResult])
        XCTAssertEqual(detail.evidence.map { copy.evidenceKindLabel(for: $0.kind) }, ["Tool result"])
        XCTAssertEqual(detail.contributionStatus, "accepted")
        XCTAssertEqual(detail.permittedUses, ["debugging", "evaluation"])
        XCTAssertEqual(detail.consentPolicyVersion, "consent/1")
        XCTAssertNil(detail.publication)
        XCTAssertEqual(detail.publicationVersion, 0)
        XCTAssertEqual(detail.retainedSourceSlug, "source-workflow")
        let taskSuccess = try XCTUnwrap(detail.taskSuccess)

        let page = try client.publishPublicRun(
            submissionID: record.submissionID,
            draft: draft,
            taskSuccess: taskSuccess,
            contributedVersion: detail.contributedVersion,
            expectedPublicationVersion: detail.publicationVersion
        )
        XCTAssertEqual(page.slug, "bounded-retry")
        XCTAssertEqual(page.reusePermission, .ccBy40)

        let publish = try XCTUnwrap(daemon.calls.last)
        XCTAssertEqual(publish.method, "publish_public_run")
        let params = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(publish.params.utf8)) as? [String: Any]
        )
        XCTAssertEqual(
            Set(params.keys),
            ["draft", "submission_id", "task_success", "contributed_version", "expected_publication_version"]
        )
        XCTAssertEqual(params["submission_id"] as? String, record.submissionID)
        XCTAssertEqual(params["task_success"] as? String, "partial")
        XCTAssertEqual(params["contributed_version"] as? String, "trace-contribution/1")
        XCTAssertEqual(params["expected_publication_version"] as? Int, 0)
        let sentDraft = try XCTUnwrap(params["draft"] as? [String: Any])
        XCTAssertEqual(
            Set(sentDraft.keys),
            ["title", "outcome_summary", "correction_excerpt", "workflow", "reuse_permission", "evidence"]
        )
        XCTAssertEqual(sentDraft["workflow"] as? String, draft.workflow)
        XCTAssertNil(sentDraft["source_slug"], "An absent source must stay absent")
        XCTAssertNil(params["approval_sha256"], "The daemon derives approval from this exact draft")
    }

    func testSessionDetailDecodesRecordsFromAnOlderServerWithoutNewFields() throws {
        let json = #"{"task_success":"success","task_outcome":"Completed","user_feedback":"none","evidence":[],"contributed_version":"trace-contribution/1","consent_policy_version":"consent/1","redaction_pipeline_version":"redaction/3","publication":null,"publication_version":0}"#
        let detail = try JSONDecoder().decode(SessionDetail.self, from: Data(json.utf8))

        XCTAssertNil(detail.task)
        XCTAssertNil(detail.contentUnavailable)
        XCTAssertNil(detail.contributionStatus)
        XCTAssertNil(detail.permittedUses)
        XCTAssertFalse(detail.accepted)
    }

    func testStatusOnlySessionDetailDoesNotFabricateAnOutcome() throws {
        let json = #"{"content_unavailable":true,"task":null,"contribution_status":"received","permitted_uses":["debugging"],"evidence":[],"contributed_version":"trace-contribution/1","consent_policy_version":"consent/1","redaction_pipeline_version":"redaction/3","publication":null,"publication_version":0}"#
        let detail = try JSONDecoder().decode(SessionDetail.self, from: Data(json.utf8))

        XCTAssertEqual(detail.contentUnavailable, true)
        XCTAssertNil(detail.taskSuccess)
        let copy = try XCTUnwrap(PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? ""))
        XCTAssertNil(copy.taskOutcomeLabel(for: detail.taskSuccess))
        XCTAssertNil(detail.userFeedback)
        XCTAssertNil(detail.humanCorrection)
        XCTAssertTrue(detail.evidence.isEmpty)
        XCTAssertEqual(detail.contributionStatus, "received")
    }

    func testUnknownEvidenceKindRemainsInspectable() throws {
        let json = #"{"event_id":"11111111-1111-4111-8111-111111111111","kind":"future_event","excerpt":"Bounded evidence."}"#
        let evidence = try JSONDecoder().decode(
            SessionEvidenceCandidate.self,
            from: Data(json.utf8)
        )

        XCTAssertEqual(evidence.kind, .unknown)
        let copy = try XCTUnwrap(PublicRunCopy.decode(fromJSON: TCPublicRun.copyJSON() ?? ""))
        XCTAssertEqual(copy.evidenceKindLabel(for: evidence.kind), "Evidence")
        XCTAssertEqual(evidence.excerpt, "Bounded evidence.")
    }

    func testSharedCopyLabelsEveryContributionStateAndPermittedUse() throws {
        let json = try XCTUnwrap(TCPublicRun.copyJSON())
        let copy = try XCTUnwrap(PublicRunCopy.decode(fromJSON: json))

        XCTAssertEqual(copy.contributionStatusChoices.count, 10)
        XCTAssertEqual(copy.contributionStatusLabel(for: "submitted"), "Submitted")
        XCTAssertEqual(copy.contributionStatusLabel(for: "accepted"), "Accepted into the commons")
        XCTAssertEqual(copy.contributionStatusLabel(for: "future_state"), copy.unrecognizedValue)
        XCTAssertEqual(copy.permittedUseChoices.count, 6)
        XCTAssertEqual(copy.permittedUseLabel(for: "model_training"), "Model training")
        XCTAssertEqual(copy.permittedUseLabel(for: "future_use"), copy.unrecognizedValue)
        XCTAssertFalse(copy.permittedUsesUnavailable.isEmpty)
    }

    func testServerPreAcceptanceStatesUseTheNonDistributedWithdrawalCopy() {
        for status in ["received", "quarantined", "awaiting_pii_backstop", "rejected"] {
            let confirmation = WithdrawalCopy.confirmation(for: .init(status: status))
            XCTAssertNil(confirmation.ambiguity, status)
            XCTAssertEqual(confirmation.bodies, [WithdrawalCopy.canonicalNotDistributed], status)
        }
    }

    func testEveryTerminalContributionStateSuppressesWithdrawal() {
        for status in ["withdrawn", "revoked", "purged", "expired"] {
            XCTAssertTrue(ContributionStatusPresentation.isTerminal(status), status)
        }
        for status in ["submitted", "received", "quarantined", "accepted", "rejected"] {
            XCTAssertFalse(ContributionStatusPresentation.isTerminal(status), status)
        }
        XCTAssertFalse(ContributionStatusPresentation.isTerminal(nil))
    }

    func testEditorUsesTheSharedRustValidatorAndNormalizedDraft() throws {
        let input = PublicRunEditorInput(
            title: "  Bounded retry recovery  ",
            outcomeSummary: "  The workflow recovered.  ",
            correctionExcerpt: nil,
            workflow: "  Retry once with the corrected bounded input.  ",
            reusePermission: .ccBy40,
            evidence: draft.evidence,
            source: "  https://tracecommons.ai/runs/source-workflow  "
        )
        let inputJSON = try XCTUnwrap(
            String(data: JSONEncoder().encode(input), encoding: .utf8)
        )
        let resultJSON = try XCTUnwrap(TCPublicRun.validateEditorJSON(inputJSON))
        let result = try JSONDecoder().decode(
            PublicRunEditorValidation.self,
            from: Data(resultJSON.utf8)
        )

        XCTAssertNil(result.error)
        XCTAssertEqual(result.draft?.title, "Bounded retry recovery")
        XCTAssertEqual(result.draft?.outcomeSummary, "The workflow recovered.")
        XCTAssertEqual(result.draft?.workflow, "Retry once with the corrected bounded input.")
        XCTAssertEqual(result.draft?.sourceSlug, "source-workflow")

        let missingPermission = PublicRunEditorInput(
            title: "Bounded retry recovery",
            outcomeSummary: "The workflow recovered.",
            correctionExcerpt: nil,
            workflow: "Retry once with the corrected bounded input.",
            reusePermission: nil,
            evidence: draft.evidence,
            source: ""
        )
        let invalidJSON = try XCTUnwrap(
            String(data: JSONEncoder().encode(missingPermission), encoding: .utf8)
        )
        let invalidResultJSON = try XCTUnwrap(TCPublicRun.validateEditorJSON(invalidJSON))
        let invalidResult = try JSONDecoder().decode(
            PublicRunEditorValidation.self,
            from: Data(invalidResultJSON.utf8)
        )
        XCTAssertNil(invalidResult.draft)
        XCTAssertNotNil(invalidResult.error)
    }

    @MainActor
    func testModelPublishesAndUnpublishesWithoutChangingContributionState() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        model.loadSessionDetail(record)
        try await waitUntil { model.sessionDetails[self.record.submissionID] != nil }

        model.publishPublicRun(record, draft: draft)
        try await waitUntil {
            model.sessionDetails[self.record.submissionID]?.publication?.slug == "bounded-retry"
        }
        XCTAssertEqual(record.status, "accepted")
        XCTAssertFalse(model.publicRunWorking.contains(record.submissionID))

        model.unpublishPublicRun(record)
        try await waitUntil {
            model.sessionDetails[self.record.submissionID]?.publication == nil
                && !model.publicRunWorking.contains(self.record.submissionID)
        }
        XCTAssertEqual(record.status, "accepted")
        XCTAssertEqual(
            daemon.calls.map(\.method),
            ["history_detail", "publish_public_run", "unpublish_public_run"]
        )
        XCTAssertEqual(model.sessionDetails[record.submissionID]?.publicationVersion, 2)
    }

    @MainActor
    func testModelClearsWorkingStateAndPreservesPublicationAcrossFailures() async throws {
        daemon.setFailure("history_detail")
        let missingModel = AppModel()
        missingModel.setClientForTesting(client)
        missingModel.loadSessionDetail(record)
        try await waitUntil {
            !missingModel.loadingSessionDetails.contains(self.record.submissionID)
        }
        XCTAssertNil(missingModel.sessionDetails[record.submissionID])
        XCTAssertEqual(
            missingModel.sessionDetailErrors[record.submissionID],
            TCPublicRun.sessionDetailErrorLine(label: "session-detail-not-found")
        )

        daemon.setFailure("history_detail", enabled: false)
        let model = AppModel()
        model.setClientForTesting(client)
        model.loadSessionDetail(record)
        try await waitUntil { model.sessionDetails[self.record.submissionID] != nil }

        daemon.setFailure("publish_public_run")
        model.publishPublicRun(record, draft: draft)
        try await waitUntil { !model.publicRunWorking.contains(self.record.submissionID) }
        XCTAssertNil(model.sessionDetails[record.submissionID]?.publication)
        XCTAssertEqual(
            model.publicRunErrors[record.submissionID],
            TCPublicRun.publicationErrorLine(label: "public-run-conflict")
        )

        daemon.setFailure("publish_public_run", enabled: false)
        model.publishPublicRun(record, draft: draft)
        try await waitUntil {
            model.sessionDetails[self.record.submissionID]?.publication?.slug == "bounded-retry"
        }
        daemon.setFailure("unpublish_public_run")
        model.unpublishPublicRun(record)
        try await waitUntil { !model.publicRunWorking.contains(self.record.submissionID) }
        XCTAssertEqual(
            model.sessionDetails[record.submissionID]?.publication?.slug,
            "bounded-retry"
        )
        XCTAssertEqual(
            model.publicRunErrors[record.submissionID],
            TCPublicRun.publicationErrorLine(label: "public-run-unpublish-failed")
        )
    }

    @MainActor
    func testCommittedMutationsRemainVisibleWhenCredentialPersistenceWarns() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        model.loadSessionDetail(record)
        try await waitUntil { model.sessionDetails[self.record.submissionID] != nil }

        daemon.setCredentialWarning("publish_public_run")
        model.publishPublicRun(record, draft: draft)
        try await waitUntil {
            model.sessionDetails[self.record.submissionID]?.publication?.slug == "bounded-retry"
        }
        XCTAssertEqual(
            model.publicRunErrors[record.submissionID],
            TCPublicRun.publicationErrorLine(label: "commons_credential_storage_unavailable")
        )

        daemon.setCredentialWarning("publish_public_run", enabled: false)
        daemon.setCredentialWarning("unpublish_public_run")
        model.unpublishPublicRun(record)
        try await waitUntil {
            model.sessionDetails[self.record.submissionID]?.publication == nil
                && !model.publicRunWorking.contains(self.record.submissionID)
        }
        XCTAssertEqual(model.sessionDetails[record.submissionID]?.publicationVersion, 2)
        XCTAssertEqual(
            model.publicRunErrors[record.submissionID],
            TCPublicRun.publicationErrorLine(label: "commons_credential_storage_unavailable")
        )
    }

    @MainActor
    func testRefreshFailureClearsCachedSessionDetail() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        model.loadSessionDetail(record)
        try await waitUntil { model.sessionDetails[self.record.submissionID] != nil }

        daemon.setFailure("history_detail")
        model.loadSessionDetail(record)
        try await waitUntil {
            !model.loadingSessionDetails.contains(self.record.submissionID)
                && model.sessionDetailErrors[self.record.submissionID] != nil
        }
        XCTAssertNil(model.sessionDetails[record.submissionID])
    }

    @MainActor
    func testAccountChangeClearsSessionAndSkillCaches() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        model.loadSessionDetail(record)
        try await waitUntil { model.sessionDetails[self.record.submissionID] != nil }
        XCTAssertTrue(model.skillLearningStore.beginInstallStatusLoading(for: record.submissionID))
        XCTAssertTrue(
            model.skillLearningStore.finishInstallStatusLoading(
                for: record.submissionID,
                installed: nil
            )
        )
        XCTAssertEqual(model.skillLearningStore.retainedSessionCount, 1)

        daemon.setOwnerScopeSHA256("sha256:owner-b")
        model.loadSessionDetail(record)
        try await waitUntil {
            model.sessionDetails[self.record.submissionID]?.ownerScopeSHA256 == "sha256:owner-b"
        }

        XCTAssertEqual(model.skillLearningStore.retainedSessionCount, 0)
        XCTAssertEqual(model.sessionDetails.count, 1)
    }

    @MainActor
    func testSessionDetailRendersAtMobileAndDesktopWidths() async throws {
        let model = AppModel()
        model.setClientForTesting(client)
        model.loadSessionDetail(record)
        try await waitUntil { model.sessionDetails[self.record.submissionID] != nil }

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
                    SessionDetailView(record: record, onBack: {}).environmentObject(model),
                    width: CGFloat(width),
                    scheme: scheme
                )
                try image.write(
                    to: directory.appendingPathComponent("session-publication-\(width)-\(label).png")
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
                XCTFail("Timed out waiting for publication state")
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
        // Attaching the window can emit app activation. SessionDetailView
        // deliberately revalidates account-owned content on that event, so
        // wait for the synthetic daemon round trip before measuring it.
        try await Task.sleep(for: .milliseconds(100))
        hosting.layoutSubtreeIfNeeded()
        let height = ceil(hosting.fittingSize.height)
        XCTAssertGreaterThan(height, 300)
        XCTAssertLessThan(height, 3_000)
        let bounds = NSRect(x: 0, y: 0, width: width, height: height)
        window.setContentSize(bounds.size)
        hosting.frame = bounds
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }

    private var draft: PublicRunDraftInput {
        PublicRunDraftInput(
            title: "Bounded retry recovery",
            outcomeSummary: "The workflow recovered after one limited retry.",
            correctionExcerpt: "Use the bounded retry path.",
            workflow: "Retry once with the corrected bounded input.",
            reusePermission: .ccBy40,
            evidence: [
                PublicRunEvidenceDraft(
                    eventID: "11111111-1111-4111-8111-111111111111",
                    excerpt: "The bounded retry completed on the second attempt."
                )
            ],
            sourceSlug: nil
        )
    }

    private var record: HistoryRecord {
        HistoryRecord(
            submissionID: "22222222-2222-4222-8222-222222222222",
            submittedAt: Date(timeIntervalSince1970: 1_789_000_000),
            projectID: "project-test",
            projectLabel: "Synthetic project",
            source: "claude_code",
            status: "accepted",
            consentScopes: ["code_content"],
            creditPointsPending: 0,
            creditPointsFinal: 1,
            explanations: [],
            lastRefreshedAt: nil
        )
    }
}
