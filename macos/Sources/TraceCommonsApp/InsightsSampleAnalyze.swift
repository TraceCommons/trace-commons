#if DEBUG
import CryptoKit
import Foundation
import TCBridge

/// Debug builds only: the saved traces the sample week's Analyze tab shows
/// (`InsightsSampleWeek`), shaped as the core sends them and with the IDs
/// and digests its store would check them against. Five saved
/// snapshots in the sample week, Claude Code and Codex, three of them
/// assessed, one with a commit and a test report linked; the saved history
/// summary reduced from them as the core reduces it; one group of two of
/// them, assessed. Sample figures, never a pilot's, and no words: every
/// word on screen is still the core's copy.
///
/// Nothing here is kept. A write to something the sample holds (an
/// assessment, a link, a delete, a group's assessment or members) is
/// answered with that thing as it stands, as a lever write is, so the next
/// read shows the sample unchanged. A write the sample has nothing to
/// answer with (a new group, a comparison task or specification, saving an
/// analyzed file) is refused. Neither reaches the store.
extension InsightsSampleWeek {
    /// The refusal for an operation the sample does not answer. Not a core
    /// code, so the window shows its usual failure line.
    static let refusal = "insights_sample_not_answered"

    /// The operations the sample leaves to the store: the core's copy, the
    /// two Overview drill-downs it read before Analyze was sampled, and the
    /// question cards. Every one is a read.
    static let storeReads: Set<String> = ["copy", "card_inputs", "pattern_sessions", "question_cards"]

    /// The sample's answer body for an Analyze operation; `nil` for one it
    /// does not answer.
    static func analyzeBody(_ operation: InsightsRequest.Operation) throws -> [String: Any]? {
        let type = operation.type
        switch type {
        case "list":
            return ["type": type, "insights": traces.sorted { $0.analyzedAt > $1.analyzedAt }.map(\.insight)]
        case "summary":
            return ["type": type, "summary": summary]
        case "explain", "annotate", "clear_annotation", "link_git", "link_test_report", "unlink_evidence":
            return ["type": type, "insight": try trace(operation.id).insight, "mutation_effects": noEffects]
        case "delete":
            _ = try trace(operation.id)
            return ["type": type, "deleted": false, "mutation_effects": noEffects]
        case "episode_list":
            return ["type": type, "episodes": [["episode": episode, "overlapping_episode_ids": [String]()]]]
        case "episode_explain":
            guard operation.id == episodeID else { throw InsightsError.service("insights_episode_not_found") }
            return ["type": type, "detail": [
                "episode": episode, "members": episodeMembers.map(\.insight), "overlap": [Any](),
                "resolved_at": "2026-10-11T09:00:00Z",
            ]]
        case "episode_annotate", "episode_clear_assessment", "episode_replace_members", "episode_delete":
            guard operation.id == episodeID else { throw InsightsError.service("insights_episode_not_found") }
            return ["type": type, "episode": episode, "mutation_effects": noEffects]
        // No comparison tasks or specifications: the store's would name
        // groups and snapshots the sample does not hold.
        case "comparison_task_list": return ["type": type, "tasks": [Any]()]
        case "comparison_list_specs": return ["type": "comparison_specification_list", "specifications": [Any]()]
        default: return nil
        }
    }

    private static var noEffects: [String: Any] { [
        "invalidated_episode_ids": [String](), "stale_comparison_task_ids": [String](),
        "stale_comparison_tasks": [Any](),
    ] }

    private static func trace(_ id: String?) throws -> SampleTrace {
        guard let trace = traces.first(where: { $0.id == id }) else { throw InsightsError.service("insights_not_found") }
        return trace
    }

    /// A stable 64-digit lowercase hex digest, as the core's IDs are.
    static func digest(_ seed: String) -> String { digest(Data(seed.utf8)) }

    static func digest(_ bytes: Data) -> String {
        SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }

    /// The core's members digest (`episodes::members_digest`): SHA-256 of
    /// `trace-commons-episode-members-v1\0`, the member count as a big-endian
    /// u64, then each member's ID and source digest, in canonical order.
    static func membersDigest(_ members: [SampleTrace]) -> String {
        var bytes = Data("trace-commons-episode-members-v1\0".utf8)
        withUnsafeBytes(of: UInt64(members.count).bigEndian) { bytes.append(contentsOf: $0) }
        for member in members { bytes.append(contentsOf: Data((member.id + member.sourceDigest).utf8)) }
        return digest(bytes)
    }

    // MARK: The saved traces

    /// One metric as the core reports it: `nil` is unknown, with the
    /// coverage it had. The core's store holds a snapshot only if each one
    /// keeps its rules (`validate_local_metrics`), so the sample keeps them
    /// too: tokens and known outcomes are never reported.
    struct SampleMetric {
        let id: String
        let value: Int?
        let observed: Int
        let total: Int
    }

    struct SampleTrace {
        let seed: String
        let source: String
        let model: String
        let analyzedAt: String
        let firstEvent: String
        let lastEvent: String
        let records: Int
        let events: Int
        let toolCalls: Int
        /// Failures and the tool results that said whether they failed.
        let failures: (value: Int, results: Int)?
        let annotation: (category: String, outcome: String, at: String)?
        let linked: Bool

        /// The core's snapshot identity (`snapshot_identity`): the format's
        /// stable ID and the source digest.
        var id: String {
            InsightsSampleWeek.digest((source == "claude_code" ? "ClaudeCode" : "Codex") + ":" + sourceDigest)
        }
        var sourceDigest: String { InsightsSampleWeek.digest("sample-source-" + seed) }

        /// The report's metrics, in the core's order.
        var metrics: [SampleMetric] {
            [
                .init(id: "sessions", value: 1, observed: 1, total: 1),
                .init(id: "events", value: events, observed: events, total: events),
                .init(id: "tool_calls", value: toolCalls, observed: events, total: events),
                .init(id: "tool_failures", value: failures?.value,
                      observed: failures?.results ?? 0, total: failures == nil ? 1 : toolCalls),
                .init(id: "input_tokens", value: nil, observed: 0, total: 1),
                .init(id: "output_tokens", value: nil, observed: 0, total: 1),
                .init(id: "known_outcomes", value: nil, observed: 0, total: 1),
            ]
        }

        var evidence: [String: Any] { ["id": id, "source_digest": sourceDigest] }

        var insight: [String: Any] {
            let claude = source == "claude_code"
            let declarations: [[String: Any]] = stride(from: 2, through: min(records, 40), by: 6).map { index in
                ["model": model, "record_index": index,
                 "kind": claude ? "claude_assistant_message" : "codex_turn_context"]
            }
            return [
                "id": id, "source_format": source, "boundary": "session_proxy", "analyzed_at": analyzedAt,
                "report": [
                    "schema_version": 1, "provider": InsightsSampleWeek.provider, "evidence": [evidence],
                    "metrics": metrics.map { metric in
                        ["id": metric.id, "value": metric.value ?? NSNull(),
                         "coverage": ["observed": metric.observed, "total": metric.total],
                         "evidence_ids": [id]] as [String: Any]
                    },
                ],
                "estimated_cost_usd": NSNull(), "cost_unavailable_reason": "adapter_usage_unavailable",
                "task_category": NSNull(),
                "manual_annotation": annotation.map { annotation in
                    ["category": annotation.category, "outcome": annotation.outcome,
                     "provenance": "user_reported", "recorded_at": annotation.at,
                     "source_digest": sourceDigest] as [String: Any]
                } ?? NSNull(),
                "model_observations": [
                    "schema_version": claude ? 3 : 2, "scope": "declared_metadata_only", "source_format": source,
                    "source_digest": sourceDigest, "coordinates": "jsonl_physical_lines_one_based",
                    "record_count": records, "candidate_records": declarations.count,
                    "valid_declarations": declarations.count, "missing_declarations": 0,
                    "invalid_declarations": 0, "omitted_declarations": 0, "model_labels_omitted": false,
                    "mixed_declared_models": false, "declared_models": [model], "declarations": declarations,
                    "contract": claude ? "claude_assistant_message_v3" : "codex_turn_context_v2",
                ],
                "claude_task_attribution": NSNull(),
                "outcome_links": linked ? outcomeLinks : [Any](),
                "time_evidence": [
                    "schema_version": 1, "scope": "recorded_event_timestamps_only", "source_format": source,
                    "source_digest": sourceDigest, "coordinates": "jsonl_physical_lines_one_based",
                    "total_eligible_records": events, "valid_timestamps": events, "missing_timestamps": 0,
                    "invalid_timestamps": 0,
                    "earliest": ["recorded_at": firstEvent, "event_refs": [["record_index": 1]],
                                 "omitted_event_refs": 0],
                    "latest": ["recorded_at": lastEvent, "event_refs": [["record_index": records]],
                               "omitted_event_refs": 0],
                ],
            ]
        }

        /// The commit the work landed in and the test run against it. Each
        /// link's ID is its evidence's identity, as the core's is
        /// (`identity_digest`).
        private var outcomeLinks: [[String: Any]] {
            let commit = String(InsightsSampleWeek.digest("sample-commit-" + seed).prefix(40))
            let repository = InsightsSampleWeek.digest("sample-repository")
            let report = InsightsSampleWeek.digest("sample-report-" + seed)
            return [
                ["id": InsightsSampleWeek.digest("git-commit:\(repository):\(commit)"),
                 "source_digest": sourceDigest, "linked_at": "2026-10-06T17:04:00Z", "provenance": "user_linked",
                 "evidence": ["type": "git_commit", "evidence": [
                     "repository_path_digest": repository,
                     "object_id": commit,
                     "tree_id": String(InsightsSampleWeek.digest("sample-tree-" + seed).prefix(40)),
                     "parent_ids": [String(InsightsSampleWeek.digest("sample-parent-" + seed).prefix(40))],
                     "inspected_at": "2026-10-06T17:04:00Z", "provenance": "inspected_local_object",
                 ]]],
                ["id": InsightsSampleWeek.digest("test-report:" + report), "source_digest": sourceDigest,
                 "linked_at": "2026-10-06T17:06:00Z", "provenance": "user_linked",
                 "evidence": ["type": "test_report", "evidence": [
                     "schema_version": 1, "runner": "cargo-test", "passed": 212, "failed": 0, "skipped": 3,
                     "observed_at": "2026-10-06T16:58:00Z", "commit_id": commit,
                     "artifact_digest": report,
                     "imported_at": "2026-10-06T17:06:00Z", "provenance": "imported_report",
                 ]]],
            ]
        }
    }

    static var provider: [String: Any] { [
        "id": "trace-commons-local", "version": "1", "rubric_version": "descriptive-counts-v1",
        "execution_mode": "local", "schema_version": 1,
    ] }

    /// Oldest first. The last two are the group.
    static let traces: [SampleTrace] = [
        .init(seed: "1", source: "claude_code", model: "claude-opus-5-5", analyzedAt: "2026-10-06T16:20:00Z",
              firstEvent: "2026-10-06T14:02:00Z", lastEvent: "2026-10-06T16:11:00Z", records: 468,
              events: 412, toolCalls: 96, failures: (4, 96),
              annotation: ("refactor", "accepted", "2026-10-06T17:00:00Z"), linked: true),
        .init(seed: "2", source: "codex", model: "gpt-5-codex", analyzedAt: "2026-10-07T10:05:00Z",
              firstEvent: "2026-10-07T08:41:00Z", lastEvent: "2026-10-07T09:58:00Z", records: 214,
              events: 188, toolCalls: 41, failures: nil,
              annotation: ("tests", "partial", "2026-10-07T11:00:00Z"), linked: false),
        .init(seed: "3", source: "claude_code", model: "claude-sonnet-5-5", analyzedAt: "2026-10-08T14:40:00Z",
              firstEvent: "2026-10-08T13:10:00Z", lastEvent: "2026-10-08T14:32:00Z", records: 301,
              events: 264, toolCalls: 58, failures: (2, 55),
              annotation: nil, linked: false),
        .init(seed: "4", source: "codex", model: "gpt-5-codex", analyzedAt: "2026-10-09T09:15:00Z",
              firstEvent: "2026-10-09T08:20:00Z", lastEvent: "2026-10-09T09:07:00Z", records: 112,
              events: 97, toolCalls: 22, failures: nil,
              annotation: ("debugging", "rejected", "2026-10-09T09:40:00Z"), linked: false),
        .init(seed: "5", source: "claude_code", model: "claude-opus-5-5", analyzedAt: "2026-10-09T17:30:00Z",
              firstEvent: "2026-10-09T15:05:00Z", lastEvent: "2026-10-09T17:21:00Z", records: 602,
              events: 530, toolCalls: 131, failures: (7, 131),
              annotation: nil, linked: false),
    ]

    // MARK: The saved history summary

    /// The core's reducer (`summary::summarize`) over the sample traces, so
    /// every count, sum and evidence ID agrees with the saved list.
    static var summary: [String: Any] {
        let sorted = traces.sorted { $0.id < $1.id }
        let assessed = sorted.filter { $0.annotation != nil }
        let categories = ["refactor", "tests", "docs", "debugging", "other", "unknown"].map { category in
            let ids = assessed.filter { $0.annotation?.category == category }.map(\.id)
            return ["category": category, "snapshots": ids.count, "evidence_snapshot_ids": ids] as [String: Any]
        }
        let outcomes = ["accepted", "partial", "rejected", "unknown"].map { outcome in
            let ids = assessed.filter { $0.annotation?.outcome == outcome }.map(\.id)
            return ["outcome": outcome, "snapshots": ids.count, "evidence_snapshot_ids": ids] as [String: Any]
        }
        let order = ["sessions", "events", "input_tokens", "output_tokens", "tool_calls", "tool_failures",
                     "known_outcomes"]
        let metrics = order.map { id -> [String: Any] in
            let rows = sorted.map { trace in (trace.id, trace.metrics.first { $0.id == id }!) }
            let known = rows.filter { $0.1.value != nil }
            let unit = switch id {
            case "events", "tool_calls": "normalized_events"
            case "tool_failures": "tool_results"
            default: "session_snapshots"
            }
            return [
                "id": id,
                "observed_value_sum": known.isEmpty ? NSNull() : known.reduce(0) { $0 + $1.1.value! } as Any,
                "available_snapshots": known.count, "missing_snapshots": rows.count - known.count,
                "record_coverage": ["observed": rows.reduce(0) { $0 + $1.1.observed },
                                    "total": rows.reduce(0) { $0 + $1.1.total }],
                "coverage_unit": unit, "evidence_snapshot_ids": known.map(\.0),
            ]
        }
        let dates = traces.map(\.analyzedAt).sorted()
        return [
            "schema_version": 1, "scope": "all_saved_selected_session_snapshots",
            "limitations": SavedInsightsSummary.Limitation.allCases.map(\.rawValue),
            "provider": provider, "saved_snapshots": sorted.count,
            "snapshot_analysis_range": ["oldest": dates.first!, "newest": dates.last!],
            "user_reported": [
                "assessed_snapshots": assessed.count, "unassessed_snapshots": sorted.count - assessed.count,
                "categories": categories, "outcomes": outcomes,
            ],
            "metrics": metrics,
            "snapshots": sorted.map { trace in
                ["id": trace.id, "source_format": trace.source, "analyzed_at": trace.analyzedAt,
                 "evidence": [trace.evidence]] as [String: Any]
            },
        ]
    }

    // MARK: The group

    static let episodeID = "5a3c1f2e-8d47-4b6a-9e21-7c0f4d8b3a65"

    /// The last two traces: a Codex attempt and the Claude Code session
    /// that picked the work up the same day.
    static var episodeMembers: [SampleTrace] { traces.suffix(2).sorted { $0.id < $1.id } }

    static var episode: [String: Any] {
        [
            "schema_version": 1, "id": episodeID, "revision": 2, "membership_revision": 1,
            "created_at": "2026-10-09T17:45:00Z", "updated_at": "2026-10-09T17:50:00Z",
            "provenance": "user_selected_whole_snapshots",
            "members": episodeMembers.map { ["snapshot_id": $0.id, "source_digest": $0.sourceDigest] },
            "manual_assessment": [
                "category": "debugging", "outcome": "partial", "provenance": "user_reported",
                "recorded_at": "2026-10-09T17:50:00Z", "membership_revision": 1,
                "members_digest": membersDigest(episodeMembers),
            ],
        ]
    }
}
#endif
