// INTEGRATION: drives AppModel.loadSessionDetail through the real Swift
// client against a synthetic daemon whose answers the test releases one at a
// time, so the order reads land in is the test's, not the scheduler's.

import Foundation
import TCBridge
import XCTest
@testable import TraceCommonsApp

/// Holds each `history_detail` answer until the test releases it, and
/// answers each submission id with the owner scope the test gave it.
private final class GatedDetailDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var gates: [String: DispatchSemaphore] = [:]
    private var scopes: [String: String] = [:]

    func hold(_ id: String, scope: String) {
        lock.lock()
        defer { lock.unlock() }
        gates[id] = DispatchSemaphore(value: 0)
        scopes[id] = scope
    }

    func release(_ id: String) {
        lock.lock()
        let gate = gates[id]
        lock.unlock()
        gate?.signal()
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        guard method == "history_detail",
            let object = try? JSONSerialization.jsonObject(with: Data(paramsJSON.utf8)) as? [String: Any],
            let id = object["submission_id"] as? String
        else {
            return #"{"id":1,"error":{"code":"unavailable","message":"unexpected-test-method"}}"#
        }
        lock.lock()
        let gate = gates[id]
        let scope = scopes[id] ?? "sha256:owner-a"
        lock.unlock()
        gate?.wait()
        return #"{"id":1,"result":{"owner_scope_sha256":"\#(scope)","task":"Synthetic task.","task_success":"partial","user_feedback":"correction","human_correction":"Synthetic correction.","evidence":[],"contribution_status":"accepted","permitted_uses":["debugging"],"contributed_version":"trace-contribution/1","consent_policy_version":"consent/1","redaction_pipeline_version":"redaction/3","publication":null,"publication_version":0,"retained_source_slug":null}}"#
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

final class SessionDetailReadOrderTests: XCTestCase {
    private let first = "33333333-3333-4333-8333-333333333333"
    private let second = "44444444-4444-4444-8444-444444444444"

    /// Two rows read at once -- the main window's History and the Monitor's,
    /// or two rows opened in quick succession -- both land. A read for one
    /// row is never dropped because another row's started after it, which
    /// would leave the first with no detail, no reading line and no error.
    @MainActor
    func test_readsForTwoRowsBothLandWhicheverAnswersFirst() async throws {
        let daemon = GatedDetailDaemon()
        daemon.hold(first, scope: "sha256:owner-a")
        daemon.hold(second, scope: "sha256:owner-a")
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))

        model.loadSessionDetail(record(first))
        model.loadSessionDetail(record(second))
        daemon.release(second)
        try await waitUntil { model.sessionDetails[self.second] != nil }
        daemon.release(first)
        try await waitUntil { !model.loadingSessionDetails.contains(self.first) }

        XCTAssertNotNil(model.sessionDetails[first], "the earlier row's read was dropped")
        XCTAssertNotNil(model.sessionDetails[second])
        XCTAssertNil(model.sessionDetailErrors[first])
    }

    /// #869's guarantee, kept: a read made under an earlier account that
    /// answers after a newer account's read never clears the newer content.
    /// It is not shown either; the row says its read could not be made, with
    /// Retry, rather than drawing nothing.
    @MainActor
    func test_aLateAnswerFromAnEarlierAccountNeverClearsTheNewerOne() async throws {
        let daemon = GatedDetailDaemon()
        daemon.hold(first, scope: "sha256:owner-a")
        daemon.hold(second, scope: "sha256:owner-b")
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))

        model.loadSessionDetail(record(first))
        model.loadSessionDetail(record(second))
        daemon.release(second)
        try await waitUntil { model.sessionDetails[self.second] != nil }
        daemon.release(first)
        try await waitUntil { !model.loadingSessionDetails.contains(self.first) }

        XCTAssertEqual(model.sessionDetails[second]?.ownerScopeSHA256, "sha256:owner-b")
        XCTAssertNil(model.sessionDetails[first])
        XCTAssertEqual(
            model.sessionDetailErrors[first], TCPublicRun.sessionDetailErrorLine(label: "session-owner-changed"))
    }

    private func record(_ id: String) -> HistoryRecord {
        HistoryRecord(
            submissionID: id, submittedAt: Date(timeIntervalSince1970: 1_789_000_000), projectID: "project-test",
            projectLabel: "Synthetic project", source: "claude_code", status: "accepted",
            consentScopes: ["code_content"], creditPointsPending: 0, creditPointsFinal: 1,
            explanations: [], lastRefreshedAt: nil)
    }

    private func waitUntil(
        timeout: Duration = .seconds(3),
        condition: @escaping @MainActor () -> Bool
    ) async throws {
        let clock = ContinuousClock()
        let deadline = clock.now.advanced(by: timeout)
        while await !condition() {
            if clock.now >= deadline {
                XCTFail("Timed out waiting for the detail reads")
                return
            }
            try await Task.sleep(for: .milliseconds(20))
        }
    }
}
