import AppKit
import SwiftUI
import TCBridge
import XCTest
@testable import TraceCommonsApp

/// The sample week's Analyze tab (`InsightsSampleAnalyze.swift`, what
/// `TRACE_COMMONS_INSIGHTS_SAMPLE=1` shows in a dev build): every answer
/// decodes and passes the checks the core's answers pass, the summary
/// agrees with the saved list, Analyze shows them through the real models,
/// and no write reaches the store.
final class InsightsSampleAnalyzeTests: XCTestCase {
    private typealias Operation = InsightsRequest.Operation

    private func answer(_ operation: Operation) throws -> InsightsResponse {
        let response = try XCTUnwrap(try InsightsSampleWeek.response(operation), operation.type)
        let expected = operation.type == "comparison_list_specs" ? "comparison_specification_list" : operation.type
        XCTAssertEqual(response.type, expected)
        return response
    }

    private var ids: [String] { InsightsSampleWeek.traces.map(\.id) }

    func test_everyReadDecodesAndValidates() throws {
        let list = try XCTUnwrap(answer(.init("list")).insights)
        XCTAssertEqual(list.count, 5)
        XCTAssertEqual(list.map(\.analyzed_at), list.map(\.analyzed_at).sorted(by: >), "newest first, as the core lists")
        XCTAssertEqual(Set(list.map(\.source_format)), ["claude_code", "codex"])
        for insight in list {
            XCTAssertTrue(Self.isDigest(insight.id))
            XCTAssertTrue(insight.analyzed_at.hasPrefix("2026-10-0"), "in the sample week")
            XCTAssertGreaterThanOrEqual(String(insight.analyzed_at.prefix(10)), InsightsSampleWeek.weekStart)
            XCTAssertEqual(insight.report.metrics.count, 7)
            XCTAssertEqual(insight.model_observations?.declared_models.count, 1)
            XCTAssertNotEqual(insight.model_observations?.contract, .unsupported)
            XCTAssertNotNil(insight.time_evidence?.earliest)
            let explained = try XCTUnwrap(answer(.init("explain", id: insight.id)).insight)
            XCTAssertEqual(explained.id, insight.id)
        }
        let assessed = list.filter { $0.manual_annotation != nil }
        XCTAssertEqual(assessed.count, 3)
        XCTAssertTrue(assessed.allSatisfy { $0.manual_annotation?.provenance == "user_reported" })
        let linked = try XCTUnwrap(list.first { !($0.outcome_links ?? []).isEmpty })
        XCTAssertNotNil(linked.manual_annotation)
        let kinds = (linked.outcome_links ?? []).map { link -> String in
            switch link.evidence {
            case .gitCommit(let commit): XCTAssertEqual(commit.object_id.count, 40); return "git"
            case .testReport(let report):
                XCTAssertEqual(report.failed, 0); XCTAssertNotNil(report.commit_id); return "test"
            }
        }
        XCTAssertEqual(kinds.sorted(), ["git", "test"])
        XCTAssertEqual(Set(list.compactMap { $0.model_observations?.declared_models.first }),
                       ["claude-opus-5-5", "claude-sonnet-5-5", "gpt-5-codex"])

        let episodes = try XCTUnwrap(answer(.init("episode_list")).episodes)
        XCTAssertEqual(episodes.count, 1)
        try episodes.forEach { try $0.episode.validateSupportedSchema() }
        let episode = episodes[0].episode
        XCTAssertEqual(episode.members.count, 2)
        XCTAssertNotNil(episode.manual_assessment)
        let detail = try XCTUnwrap(answer(.init("episode_explain", id: episode.id)).detail)
        try detail.validateSupportedSchema(expectedID: episode.id)
        XCTAssertTrue(Set(detail.members.map(\.id)).isSubset(of: Set(list.map(\.id))))

        XCTAssertEqual(try answer(.init("comparison_task_list")).tasks?.count, 0)
        XCTAssertEqual(try answer(.init("comparison_list_specs")).specifications?.count, 0)
    }

    /// The summary is the core's reduction of the saved list: counts,
    /// categories, outcomes, metric sums and their evidence agree with it.
    func test_theSummaryAgreesWithTheSavedList() throws {
        let list = try XCTUnwrap(answer(.init("list")).insights)
        let summary = try XCTUnwrap(answer(.init("summary")).summary)
        try summary.validateSupportedSchema()
        XCTAssertEqual(summary.saved_snapshots, UInt64(list.count))
        XCTAssertEqual(Set(summary.snapshots.map(\.id)), Set(list.map(\.id)))
        XCTAssertEqual(summary.snapshot_analysis_range?.oldest, list.map(\.analyzed_at).min())
        XCTAssertEqual(summary.snapshot_analysis_range?.newest, list.map(\.analyzed_at).max())
        let annotations = list.compactMap { insight in insight.manual_annotation.map { (insight.id, $0) } }
        XCTAssertEqual(summary.user_reported.assessed_snapshots, UInt64(annotations.count))
        XCTAssertEqual(summary.user_reported.unassessed_snapshots, UInt64(list.count - annotations.count))
        for category in summary.user_reported.categories {
            XCTAssertEqual(Set(category.evidence_snapshot_ids),
                           Set(annotations.filter { $0.1.category == category.category }.map(\.0)))
            XCTAssertEqual(category.snapshots, UInt64(category.evidence_snapshot_ids.count))
        }
        for outcome in summary.user_reported.outcomes {
            XCTAssertEqual(Set(outcome.evidence_snapshot_ids),
                           Set(annotations.filter { $0.1.outcome == outcome.outcome }.map(\.0)))
        }
        XCTAssertEqual(summary.metrics.count, 7)
        for metric in summary.metrics {
            let rows = list.map { insight in (insight.id, insight.report.metrics.first { $0.id == metric.id }!) }
            let known = rows.filter { $0.1.value != nil }
            XCTAssertEqual(metric.available_snapshots, UInt64(known.count), metric.id)
            XCTAssertEqual(metric.missing_snapshots, UInt64(rows.count - known.count), metric.id)
            XCTAssertEqual(metric.observed_value_sum, known.isEmpty ? nil : known.reduce(0) { $0 + $1.1.value! })
            XCTAssertEqual(metric.record_coverage.total, rows.reduce(0) { $0 + $1.1.coverage.total })
            XCTAssertEqual(metric.record_coverage.observed, rows.reduce(0) { $0 + $1.1.coverage.observed })
            XCTAssertEqual(Set(metric.evidence_snapshot_ids), Set(known.map(\.0)))
        }
        let partly = summary.metrics.filter { $0.available_snapshots > 0 && $0.missing_snapshots > 0 }
        XCTAssertFalse(partly.isEmpty, "some metrics are missing for some snapshots")
    }

    /// Every sample metric keeps the rules the core's store holds a saved
    /// snapshot to (`validate_local_metrics`): one session; every event
    /// observed; tokens and known outcomes never reported; tool calls over
    /// every event; tool failures over at most every event.
    func test_everySampleMetricKeepsTheCoreRules() throws {
        let list = try XCTUnwrap(answer(.init("list")).insights)
        for insight in list {
            let metrics = Dictionary(uniqueKeysWithValues: insight.report.metrics.map { ($0.id, $0) })
            XCTAssertEqual(metrics.count, 7)
            for metric in insight.report.metrics { XCTAssertEqual(metric.evidence_ids, [insight.id], metric.id) }
            let sessions = try XCTUnwrap(metrics["sessions"])
            XCTAssertEqual([sessions.value, sessions.coverage.observed, sessions.coverage.total], [1, 1, 1])
            let events = try XCTUnwrap(metrics["events"]?.value)
            XCTAssertGreaterThan(events, 0)
            XCTAssertEqual(metrics["events"]?.coverage.observed, events)
            XCTAssertEqual(metrics["events"]?.coverage.total, events)
            for id in ["input_tokens", "output_tokens", "known_outcomes"] {
                let metric = try XCTUnwrap(metrics[id])
                XCTAssertNil(metric.value, id)
                XCTAssertEqual([metric.coverage.observed, metric.coverage.total], [0, 1], id)
            }
            let calls = try XCTUnwrap(metrics["tool_calls"])
            XCTAssertEqual(calls.coverage.total, events)
            XCTAssertLessThanOrEqual(calls.value ?? 0, calls.coverage.observed)
            let failures = try XCTUnwrap(metrics["tool_failures"])
            XCTAssertLessThanOrEqual(failures.coverage.total, events)
            XCTAssertLessThanOrEqual(failures.value ?? 0, failures.coverage.observed)
        }
        XCTAssertTrue(list.contains { $0.report.metrics.first { $0.id == "tool_failures" }?.value == nil },
                      "tool failures are unknown for some")
        XCTAssertTrue(list.contains { insight in
            let metrics = insight.report.metrics
            return metrics.first { $0.id == "tool_calls" }!.value! < metrics.first { $0.id == "events" }!.value!
        })
    }

    /// The core holds the sample as its own: written into a store as the
    /// core writes one, every snapshot, link and the group pass its load-time
    /// checks (snapshot and link identities, metric rules, members digest),
    /// and the core's list, summary, snapshots and group are the sample's.
    func test_theCoreHoldsTheSampleAsItsOwnStore() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: false,
                                                attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(at: directory) }
        let index: [String: Any] = [
            "version": 13, "aliases": [String: Any](),
            "reports": Dictionary(uniqueKeysWithValues: InsightsSampleWeek.traces.map { ($0.id, $0.insight) }),
            "episodes": [InsightsSampleWeek.episodeID: InsightsSampleWeek.episode],
        ]
        let path = directory.appendingPathComponent("index.json").path
        XCTAssertTrue(FileManager.default.createFile(atPath: path, contents: try JSONSerialization.data(
            withJSONObject: index), attributes: [.posixPermissions: 0o600]))
        func core(_ operation: Operation) throws -> InsightsResponse {
            try TCInsights.call(.init(storeDirectory: directory.path, operation: operation))
        }

        let list = try XCTUnwrap(core(.init("list")).insights)
        XCTAssertEqual(list.map(\.id).sorted(), ids.sorted(), "the core withholds none of the sample")
        for id in ids {
            let held = try XCTUnwrap(core(.init("explain", id: id)).insight)
            let sample = try XCTUnwrap(answer(.init("explain", id: id)).insight)
            XCTAssertEqual(String(describing: held), String(describing: sample))
        }
        let summary = try XCTUnwrap(core(.init("summary")).summary)
        XCTAssertEqual(String(describing: summary), String(describing: try XCTUnwrap(answer(.init("summary")).summary)))

        let episodes = try XCTUnwrap(core(.init("episode_list")).episodes)
        XCTAssertEqual(episodes.map(\.episode.id), [InsightsSampleWeek.episodeID], "the core withholds no group")
        let detail = try XCTUnwrap(core(.init("episode_explain", id: InsightsSampleWeek.episodeID)).detail)
        let sampleDetail = try XCTUnwrap(answer(.init("episode_explain", id: InsightsSampleWeek.episodeID)).detail)
        XCTAssertEqual(String(describing: detail.episode), String(describing: sampleDetail.episode))
        XCTAssertEqual(detail.members.map(\.id), sampleDetail.members.map(\.id))
    }

    /// Each answer's words are the core's: every source, category, outcome,
    /// limitation and metric the sample names has a copy entry.
    func test_everyWordTheSampleNamesIsInTheCoreCopy() throws {
        let copy = try XCTUnwrap(TCInsights.copy())
        let list = try XCTUnwrap(answer(.init("list")).insights)
        let summary = try XCTUnwrap(answer(.init("summary")).summary)
        var keys = Set(list.map(\.source_format))
        for insight in list {
            keys.formUnion(insight.report.metrics.map { "metric_" + $0.id })
            if let annotation = insight.manual_annotation {
                keys.formUnion(["category_" + annotation.category, "outcome_" + annotation.outcome])
            }
        }
        keys.formUnion(summary.limitations.map { "summary_limitation_" + $0.rawValue })
        keys.formUnion(summary.metrics.map { "summary_unit_" + $0.coverage_unit.rawValue })
        for key in keys { XCTAssertNotNil(copy[key], key) }
    }

    /// Writes to what the sample holds are answered with it unchanged.
    func test_writesAreAnsweredUnchanged() throws {
        let id = ids[0]
        let before = try XCTUnwrap(answer(.init("explain", id: id)).insight)
        for operation: Operation in [
            .init("annotate", id: id, category: "docs", outcome: "rejected"), .init("clear_annotation", id: id),
            .init("link_git", id: id, repository: "/tmp/repository", commit: String(repeating: "a", count: 40)),
            .init("link_test_report", file: "/tmp/report.json", id: id),
            .init("unlink_evidence", id: id, evidenceID: "x"),
        ] {
            let insight = try XCTUnwrap(answer(operation).insight)
            XCTAssertEqual(insight.id, id)
            XCTAssertEqual(insight.manual_annotation?.category, before.manual_annotation?.category)
            XCTAssertEqual(insight.outcome_links?.count, before.outcome_links?.count)
        }
        XCTAssertEqual(try answer(.init("delete", id: id)).deleted, false)
        XCTAssertEqual(try answer(.init("list")).insights?.count, 5)
        let episodeID = InsightsSampleWeek.episodeID
        for operation: Operation in [
            .init("episode_annotate", id: episodeID, category: "docs", outcome: "accepted", expectedRevision: 2),
            .init("episode_clear_assessment", id: episodeID, expectedRevision: 2),
            .init("episode_replace_members", id: episodeID, snapshotIDs: [id], expectedRevision: 2),
            .init("episode_delete", id: episodeID, expectedRevision: 2),
        ] {
            let episode = try XCTUnwrap(answer(operation).episode)
            try episode.validateSupportedSchema()
            XCTAssertEqual(episode.revision, 2)
            XCTAssertEqual(episode.manual_assessment?.category, "debugging")
        }
    }

    /// No operation the window can send reaches the store except the
    /// sample's three reads; everything the sample does not answer is
    /// refused.
    func test_noSampleWriteReachesTheStore() async throws {
        XCTAssertEqual(InsightsSampleWeek.storeReads, ["copy", "card_inputs", "pattern_sessions"])
        let store = StoreRecorder()
        let service = InsightsSampleWeek.service(fallback: { request in try await store.call(request) })
        let id = ids[0], episodeID = InsightsSampleWeek.episodeID
        let writes: [Operation] = [
            .init("analyze", source: "claude_code", file: "/tmp/trace.jsonl", save: true),
            .init("analyze", source: "claude_code", file: "/tmp/trace.jsonl", save: false),
            .init("annotate", id: id, category: "docs", outcome: "accepted"), .init("clear_annotation", id: id),
            .init("delete", id: id), .init("delete", id: "unknown"),
            .init("link_git", id: id, repository: "/tmp/repository", commit: String(repeating: "a", count: 40)),
            .init("link_test_report", file: "/tmp/report.json", id: id),
            .init("unlink_evidence", id: id, evidenceID: "x"),
            .init("episode_create", snapshotIDs: [id]),
            .init("episode_annotate", id: episodeID, category: "docs", outcome: "accepted", expectedRevision: 2),
            .init("episode_clear_assessment", id: episodeID, expectedRevision: 2),
            .init("episode_replace_members", id: episodeID, snapshotIDs: [id], expectedRevision: 2),
            .init("episode_delete", id: episodeID, expectedRevision: 2),
            .init("episode_delete", id: UUID().uuidString.lowercased(), expectedRevision: 1),
            .init("comparison_task_create", episodeIDs: [episodeID]),
            .init("comparison_task_set_outcome", id: "t", outcome: "accepted", expectedRevision: 1),
            .init("comparison_task_delete", id: "t", expectedRevision: 1),
            .init("comparison_save_spec"), .init("comparison_preview_spec"),
            .init("question_cards", snapshotIDs: [id], questions: InsightQuestion.allCases),
            .init("goal_set"), .init("lever_feedback"), .init("lever_card", enabled: false),
        ]
        for operation in writes { _ = try? await service(.init(operation: operation)) }
        let reached = await store.operations
        XCTAssertEqual(reached, [], "no write reaches the store")
        do {
            _ = try await service(.init(operation: .init("episode_create", snapshotIDs: [id])))
            XCTFail("a new group is refused")
        } catch { XCTAssertEqual(error as? InsightsError, .service(InsightsSampleWeek.refusal)) }
    }

    /// Analyze through the real models: the saved list, the summary, a
    /// snapshot's detail and the group, and the tab drawn at the pane width.
    /// Set `TC_INSIGHTS_RENDER_DIR` to keep the PNG.
    @MainActor
    func test_analyzeInSampleModeListsTheSnapshotsAndShowsTheSummary() async throws {
        let store = StoreRecorder()
        let service = InsightsSampleWeek.service(fallback: { request in try await store.call(request) })
        let model = InsightsModel(service: service)
        model.open()
        try await settle { !model.busy && !model.episodeBusy && model.summary != nil }
        XCTAssertNil(model.error)
        XCTAssertEqual(model.snapshots.map(\.id).sorted(), ids.sorted())
        XCTAssertEqual(model.summary?.saved_snapshots, 5)
        XCTAssertEqual(model.summary?.user_reported.assessed_snapshots, 3)
        XCTAssertEqual(model.episodes.map(\.id), [InsightsSampleWeek.episodeID])
        XCTAssertFalse(model.copy.isEmpty, "the words are the core's")

        let linked = try XCTUnwrap(InsightsSampleWeek.traces.first(where: \.linked)).id
        model.explain(linked)
        try await settle { !model.busy }
        XCTAssertEqual(model.selected?.id, linked)
        XCTAssertTrue(model.selectedIsSaved)
        XCTAssertEqual(model.selected?.outcome_links?.count, 2)

        model.annotate(category: "docs", outcome: "rejected")
        try await settle { !model.busy && model.summary != nil }
        XCTAssertNil(model.error)
        XCTAssertEqual(model.selected?.manual_annotation?.category, "refactor", "kept nowhere")
        XCTAssertEqual(model.summary?.saved_snapshots, 5)

        model.openEpisode(InsightsSampleWeek.episodeID)
        try await settle { !model.episodeBusy }
        XCTAssertEqual(model.episodeDetail?.members.count, 2)
        XCTAssertNil(model.episodeError)

        let reached = await store.operations
        XCTAssertTrue(Set(reached).isSubset(of: InsightsSampleWeek.storeReads), "\(reached)")

        let copy = model.copy
        let view = InsightsView(storeSelection: .standard, storeCopy: copy, model: model,
                                comparisonModel: ComparisonTasksModel(service: service),
                                specificationModel: ComparisonSpecificationsModel(service: service),
                                initialTab: .analyze)
        let png = try InsightsPaneWidthRenderTests.render(AnyView(view), height: 3600, dark: false)
        XCTAssertGreaterThan(png.count, 20_000)
        if let dir = ProcessInfo.processInfo.environment["TC_INSIGHTS_RENDER_DIR"] {
            try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent("insights-analyze-sample.png"))
        }
    }

    @MainActor
    private func settle(_ done: () -> Bool) async throws {
        for _ in 0 ..< 300 where !done() { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertTrue(done(), "the model did not settle")
    }

    private static func isDigest(_ value: String) -> Bool {
        value.count == 64 && value.allSatisfy { "0123456789abcdef".contains($0) }
    }
}

/// The store behind the sample: records what reaches it, and answers the
/// core's copy from the real core over an empty temporary store.
private actor StoreRecorder {
    private(set) var operations: [String] = []
    private let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path
    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        operations.append(request.operation.type)
        return try TCInsights.call(.init(storeDirectory: directory, operation: request.operation))
    }
}
