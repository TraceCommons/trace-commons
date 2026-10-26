import Foundation
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// Analyze & save on a watched trace (Insights PM, 2026-10-21: terminal
/// users never see their trace files): the daemon analyzes and saves the
/// trace its watcher found, and the saved snapshot opens in Analyze. Analyze
/// keeps its own Choose file for when Sessions cannot offer it.
final class InsightsAnalyzeSessionTests: XCTestCase {
    /// The daemon's answer around the core's `analyze` response.
    private static func answer(_ analysis: [String: Any]?, enabled: Bool = true,
                               readable: Bool? = true) throws -> DaemonData.InsightsAnalyzeSessionAnswer {
        var object: [String: Any] = ["enabled": enabled, "feed": "counter_pass"]
        if let readable { object["readable"] = readable }
        if let analysis { object["analysis"] = analysis }
        return try JSONDecoder().decode(DaemonData.InsightsAnalyzeSessionAnswer.self,
                                        from: JSONSerialization.data(withJSONObject: object))
    }

    private static var saved: [String: Any] {
        ["type": "analyze", "insight": WatchedSnapshot.value,
         "mutation_effects": ["invalidated_episode_ids": ["removed-group"]]]
    }

    @MainActor
    private func settle(_ model: InsightsModel) async throws {
        for _ in 0..<500 {
            if !model.busy && !model.episodeBusy { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        XCTFail("Operation did not finish")
    }

    /// Busy from the moment it is asked, then the saved snapshot is the
    /// detail on screen, first in the saved list, and the list, summary and
    /// episodes are read again, as after any save.
    @MainActor
    func testAnalyzeAndSaveShowsTheSavedSnapshotAndReadsTheStoreAgain() async throws {
        let service = SavedStore()
        let daemon = DaemonGate()
        let model = InsightsModel(service: { try await service.call($0) },
                                  sessionAnalyzer: { try await daemon.call($0) })
        XCTAssertTrue(model.canAnalyzeSessions)
        model.open(); try await settle(model)
        let before = await service.operations.count

        let asked = Task { await model.analyzeAndSave(session: "t7") }
        await daemon.waitForCall()
        XCTAssertTrue(model.busy, "busy at once, before the daemon answers")
        XCTAssertEqual(model.analyzingSession, "t7")
        // A second press while it is asked does nothing.
        let again = await model.analyzeAndSave(session: "t8")
        XCTAssertFalse(again)
        await daemon.resolve(.success(try Self.answer(Self.saved)))
        let opened = await asked.value
        try await settle(model)

        XCTAssertTrue(opened)
        let calls = await daemon.calls
        XCTAssertEqual(calls, ["t7"])
        XCTAssertNil(model.analyzingSession)
        XCTAssertNil(model.sessionAnalyzeFailure)
        XCTAssertEqual(model.selected?.id, "watched-snapshot")
        XCTAssertTrue(model.selectedIsSaved, "the daemon saved it: assessment and evidence apply")
        XCTAssertNil(model.selectedFile)
        XCTAssertEqual(model.snapshots.first?.id, "watched-snapshot")
        XCTAssertEqual(model.invalidatedEpisodeIDs, ["removed-group"])
        XCTAssertNotNil(model.summary)
        let operations = await service.operations
        XCTAssertEqual(Array(operations[before...]), ["list", "summary", "episode_list"],
                       "the store is read again; the app itself analyzes nothing")
        model.close()
    }

    /// A failure leaves the detail as it was and names the row with the
    /// core's line: a file that has gone, a row no longer listed, and one
    /// line for anything else.
    @MainActor
    func testAFailureNamesTheRowWithTheCoreLine() async throws {
        let cases: [(Result<DaemonData.InsightsAnalyzeSessionAnswer, Error>, String)] = [
            (.failure(DaemonDataError.daemon(code: "unavailable", message: "insights-session-file-gone")),
             "analytics_analyze_file_gone"),
            (.failure(DaemonDataError.daemon(code: "bad_params", message: "insights-session-unknown")),
             "analytics_analyze_unknown"),
            (.failure(DaemonDataError.daemon(code: "unavailable", message: "insights-session-too-large")),
             "analytics_analyze_too_large"),
            (.failure(DaemonDataError.daemon(code: "unavailable",
                                             message: "insights-session-not-one-session")),
             "analytics_analyze_codex_not_one"),
            (.failure(DaemonDataError.daemon(code: "unknown_method", message: "insights_analyze_session")),
             "analytics_analyze_failed"),
            (.failure(DaemonDataError.unreachable), "analytics_analyze_failed"),
            (.success(try Self.answer(nil, enabled: false, readable: nil)), "analytics_analyze_failed"),
            (.success(try Self.answer(nil, readable: false)), "analytics_analyze_failed"),
            (.success(try Self.answer(["type": "analyze"])), "analytics_analyze_failed"),
        ]
        for (outcome, key) in cases {
            let service = SavedStore()
            let daemon = DaemonGate()
            let model = InsightsModel(service: { try await service.call($0) },
                                      sessionAnalyzer: { try await daemon.call($0) })
            model.open(); try await settle(model)
            let before = await service.operations.count
            let asked = Task { await model.analyzeAndSave(session: "t3") }
            await daemon.waitForCall()
            await daemon.resolve(outcome)
            let opened = await asked.value
            XCTAssertFalse(opened, key)
            XCTAssertFalse(model.busy, key)
            XCTAssertNil(model.analyzingSession, key)
            XCTAssertEqual(model.sessionAnalyzeFailure?.reference, "t3", key)
            XCTAssertEqual(model.sessionAnalyzeFailure?.key, key)
            XCTAssertNil(model.selected, key)
            let after = await service.operations.count
            XCTAssertEqual(after, before, "nothing is read again after a failure: \(key)")
            model.close()
            XCTAssertNil(model.sessionAnalyzeFailure, "closing clears the line")
        }
    }

    /// Closing while the daemon works drops its late answer.
    @MainActor
    func testClosingDropsALateAnswer() async throws {
        let service = SavedStore()
        let daemon = DaemonGate()
        let model = InsightsModel(service: { try await service.call($0) },
                                  sessionAnalyzer: { try await daemon.call($0) })
        model.open(); try await settle(model)
        let asked = Task { await model.analyzeAndSave(session: "t7") }
        await daemon.waitForCall()
        model.close()
        await daemon.resolve(.success(try Self.answer(Self.saved)))
        let opened = await asked.value
        XCTAssertFalse(opened)
        XCTAssertNil(model.selected)
        XCTAssertNil(model.analyzingSession)
    }

    /// Without a daemon there is no action and nothing is asked.
    @MainActor
    func testWithoutADaemonThereIsNoAction() async throws {
        let model = InsightsModel(service: { try await SavedStore().call($0) })
        XCTAssertFalse(model.canAnalyzeSessions)
        model.open(); try await settle(model)
        let opened = await model.analyzeAndSave(session: "t7")
        XCTAssertFalse(opened)
        XCTAssertNil(model.analyzingSession)
        model.close()
    }

    // MARK: Sources

    /// Choose file stays on Analyze whatever Sessions offers: it is the one
    /// way to add a trace with counting off, without a daemon, with a
    /// custom store or in sample data. The line under the title names a
    /// button on screen: Analyze & save only while Sessions offers it.
    @MainActor
    func testAnalyzeKeepsChooseFileAndNamesOnlyButtonsOnScreen() throws {
        let view = try MonitorNavigationTests.text("Views/InsightsView.swift")
        XCTAssertTrue(view.contains("FolderPanel.chooseTraceFile(source: source)"))
        XCTAssertTrue(view.contains("model.text(\"choose_file\")"))
        XCTAssertTrue(view.contains("model.text(\"source\")"))
        XCTAssertTrue(view.contains("model.analyze(file: file, source: source)"))
        XCTAssertTrue(view.contains("model.canAnalyzeSessions && model.weekFeed == .counterPass"))
        XCTAssertEqual(InsightsView.analyzeHeadKey(empty: true, sessions: true), "analytics_analyze_empty")
        XCTAssertEqual(InsightsView.analyzeHeadKey(empty: false, sessions: true), "analytics_analyze_how")
        XCTAssertEqual(InsightsView.analyzeHeadKey(empty: true, sessions: false), "empty")
        XCTAssertNil(InsightsView.analyzeHeadKey(empty: false, sessions: false))
        let copy = try XCTUnwrap(TCInsights.copy())
        XCTAssertTrue(copy["empty", default: ""].contains("Choose a file"))
        XCTAssertFalse(copy["empty", default: ""].contains("Analyze & save"))
    }

    /// The open Sessions row carries the action, which asks the daemon to
    /// save and opens Analyze; only for the standard store.
    @MainActor
    func testTheOpenRowAnalyzesAndSavesThroughTheDaemon() throws {
        let tab = try MonitorNavigationTests.text("Views/InsightsSessionsTab.swift")
        XCTAssertTrue(tab.contains("text(\"analytics_analyze_save\")"))
        XCTAssertTrue(tab.contains("analysis.analyzeAndSave(session: reference)"))
        XCTAssertTrue(tab.contains("\"analytics_analyze_save_a11y\""))
        let view = try MonitorNavigationTests.text("Views/InsightsView.swift")
        XCTAssertTrue(view.contains("client.insightsAnalyzeSession(sessionRef: reference, save: true)"))
        XCTAssertTrue(view.contains("storeSelection.storeDirectory != nil"))
        XCTAssertTrue(view.contains("openAnalysis: { tab = .analyze }"))
        // Analyze appears with the saved snapshot already its detail, so
        // the scroll to it runs on the initial value too.
        XCTAssertTrue(view.contains(".onChange(of: model.selected?.id, initial: true)"))
    }

    /// A row the daemon says it cannot analyze shows why in place of the
    /// action; a daemon that predates the flag offers it on every row. A
    /// row no longer listed under its reference reads the list again, as
    /// the label and drill paths do.
    @MainActor
    func testARowTheDaemonCannotAnalyzeSaysWhyAndAStaleOneReadsTheListAgain() throws {
        let row = { (flag: String) in
            try JSONDecoder().decode(DaemonData.InsightsWatchedSession.self, from: Data(
                #"{"session_ref":"g2.t7","source":"codex","started_at":null,"turns":null,"tokens":null,"sparkline":null,"folder":"f"\#(flag)}"#
                    .utf8))
        }
        XCTAssertFalse(try row(#","analyzable":false"#).canAnalyze)
        XCTAssertTrue(try row(#","analyzable":true"#).canAnalyze)
        XCTAssertTrue(try row("").canAnalyze, "an older daemon's row is offered the action")
        XCTAssertEqual(try row("").sessionRef, "g2.t7", "a reference is kept as sent, never parsed")
        let tab = try MonitorNavigationTests.text("Views/InsightsSessionsTab.swift")
        XCTAssertTrue(tab.contains("if row.canAnalyze {"))
        XCTAssertTrue(tab.contains("text(\"analytics_analyze_not_analyzable\")"))
        XCTAssertTrue(tab.contains("analysis.sessionAnalyzeFailure?.key == InsightsModel.sessionUnknownKey"))
        XCTAssertTrue(tab.contains("model.rowNoLongerListed()"))
    }

    /// Every word the action and Analyze's head show is core copy.
    func testTheWordsAreCoreCopy() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        for key in ["analytics_analyze_save", "analytics_analyze_save_a11y", "analytics_analyze_failed",
                    "analytics_analyze_file_gone", "analytics_analyze_unknown", "analytics_analyze_empty",
                    "analytics_analyze_how", "analytics_analyze_intro", "analytics_analyze_not_analyzable",
                    "analytics_analyze_too_large", "analytics_analyze_codex_not_one"] {
            XCTAssertFalse(copy[key, default: ""].isEmpty, key)
        }
        XCTAssertTrue(copy["analytics_analyze_save_a11y", default: ""].contains("{label}"))
        XCTAssertTrue(copy["analytics_analyze_empty", default: ""].contains("Sessions"))
    }
}

/// The local store's answers the model reads around a save: one saved
/// snapshot in the list once the daemon has saved it.
private actor SavedStore {
    var operations: [String] = []
    private let absentStore = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        let operation = request.operation
        operations.append(operation.type)
        if operation.type == "summary" {
            return try TCInsights.call(.init(storeDirectory: absentStore, operation: .init("summary")))
        }
        var response: [String: Any] = ["type": operation.type]
        switch operation.type {
        case "copy": response["copy"] = ["error": "operation-failed"]
        case "list":
            let saved = operations.dropLast().contains("list") ? [WatchedSnapshot.value] : []
            response["insights"] = saved
        case "episode_list": response["episodes"] = []
        default: throw InsightsError.invalidResponse
        }
        return try JSONDecoder().decode(InsightsResponse.self, from: JSONSerialization.data(withJSONObject: response))
    }
}

/// The snapshot the daemon saved, as the core writes it.
private enum WatchedSnapshot {
    static var value: [String: Any] {
        [
            "id": "watched-snapshot", "source_format": "claude_code", "boundary": "session_proxy",
            "analyzed_at": "2026-10-21T09:00:00Z", "cost_unavailable_reason": "unknown",
            "report": ["schema_version": 1,
                       "provider": ["id": "local", "version": "1", "rubric_version": "1", "execution_mode": "local"],
                       "metrics": [], "evidence": []] as [String: Any],
        ]
    }
}

/// The daemon's `insights_analyze_session`, answered when the test says.
private actor DaemonGate {
    var calls: [String] = []
    private var continuation: CheckedContinuation<DaemonData.InsightsAnalyzeSessionAnswer, Error>?
    func call(_ reference: String) async throws -> DaemonData.InsightsAnalyzeSessionAnswer {
        calls.append(reference)
        return try await withCheckedThrowingContinuation { continuation = $0 }
    }
    func waitForCall() async {
        while continuation == nil { await Task.yield() }
    }
    func resolve(_ outcome: Result<DaemonData.InsightsAnalyzeSessionAnswer, Error>) {
        continuation?.resume(with: outcome)
        continuation = nil
    }
}
