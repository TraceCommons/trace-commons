import Foundation

extension DaemonData.InsightsSubmitted {
    /// A believable `insights_submitted` answer for someone who has
    /// submitted nothing yet, to show as clearly labelled sample data: 30
    /// traces over six weeks on three models, with accepted, pending,
    /// quarantined, rejected and withdrawn traces, credit and its outcome
    /// reasons, and the file kinds the daemon counts (source, test, doc,
    /// config). Every aggregate is computed from the traces, so the parts
    /// agree with each other as a real answer's do. Never a real person's
    /// data: the projects, ids and figures are made up.
    public static let sample: DaemonData.InsightsSubmitted = SubmittedSample.build()
}

/// Builds `InsightsSubmitted.sample`.
enum SubmittedSample {
    struct Model {
        let label: String
        let tool: String
        let source: String
        /// Per-session list price, USD micros, before variation.
        let usdMicros: Int64
    }

    static let models = [
        Model(label: "claude-opus-5-5", tool: "claude_code", source: "claude-code", usdMicros: 2_850_000),
        Model(label: "claude-sonnet-5-5", tool: "claude_code", source: "claude-code", usdMicros: 640_000),
        Model(label: "gpt-5.5", tool: "codex", source: "codex", usdMicros: 1_120_000),
    ]
    static let projects = ["trace-commons", "ios-app", "landing-site", "infra"]
    static let tasks = ["feature", "bug_fix", "tests", "docs", "refactor", "setup", "understand"]
    /// The Monday of the newest of the six weeks.
    static let lastMonday = DateComponents(calendar: Calendar(identifier: .iso8601),
                                           timeZone: TimeZone(identifier: "UTC"),
                                           year: 2026, month: 10, day: 26).date!

    static func iso(_ date: Date) -> String {
        let f = ISO8601DateFormatter()
        f.formatOptions = [.withInternetDateTime]
        f.timeZone = TimeZone(identifier: "UTC")
        return f.string(from: date)
    }

    static func day(_ date: Date) -> String {
        String(iso(date).prefix(10))
    }

    /// The thirty traces, newest first.
    static func traces() -> [DaemonData.InsightsSubmittedTrace] {
        (0..<30).map { n in
            // Newest first: about five a week over six weeks.
            let at = lastMonday.addingTimeInterval(TimeInterval(4 * 86_400 + 11 * 3_600 - n * 32 * 3_600))
            let model = models[[0, 1, 0, 2, 1, 0, 2, 1, 0, 1][n % 10]]
            let task = tasks[(n * 3 + n / 7) % tasks.count]
            let status: String
            var final: Double? = nil
            var pending = 0.0
            var reason: String? = nil
            switch n {
            case 0, 1, 3, 6: status = "pending"; pending = [6.0, 4.5, 8.0, 5.0][[0, 1, 3, 6].firstIndex(of: n)!]
            case 9, 17, 24: status = "quarantined"; pending = 3.0
            case 12: status = "withdrawn"
            case 21: status = "rejected"; final = 0; reason = "policy_mismatch"
            case 5, 14, 27: status = "accepted"; final = 0; reason = "duplicate"
            case 10, 19: status = "accepted"; final = 0; reason = "novelty_failed"
            case 16: status = "accepted"; final = 0; reason = "policy_mismatch"
            default: status = "accepted"; final = [5.0, 8.0, 12.0, 6.5, 10.0][n % 5]
            }
            let files: DaemonData.InsightsSubmittedFiles
            switch task {
            case "tests": files = .init(source: 1 + n % 2, test: 2 + n % 3)
            case "docs": files = .init(doc: 2 + n % 3)
            case "setup": files = .init(source: n % 2, config: 2 + n % 2)
            case "understand": files = .init()
            case "refactor": files = .init(source: 4 + n % 5, test: 1)
            default: files = .init(source: 2 + n % 4, test: n % 3, doc: n % 2, config: n % 4 == 0 ? 1 : 0)
            }
            let edits = task == "understand" ? 0 : 3 + (n * 7) % 19
            let undone = edits == 0 ? 0 : (n % 4 == 1 ? 2 : n % 5 == 0 ? 1 : 0)
            let denied = model.tool == "codex" ? nil : (edits > 0 && n % 6 == 2 ? 1 : 0)
            let kept = max(0, edits - undone - (denied ?? 0))
            let usd = model.usdMicros + Int64((n * 37) % 23) * model.usdMicros / 20
            return DaemonData.InsightsSubmittedTrace(
                submissionId: String(format: "5a3b1e00-0000-4000-8000-%012d", 30 - n),
                submittedAt: iso(at), projectLabel: projects[(n + n / 4) % projects.count],
                source: model.source, status: status, creditPointsPending: pending, creditPointsFinal: final,
                creditOutcomeReason: reason, matched: true, tool: model.tool, model: model.label, task: task,
                files: files, edits: edits, editsKept: kept, editsUndone: undone, editsDenied: denied,
                usdMicros: usd)
        }
    }

    struct Sums {
        var submitted = 0, accepted = 0, matched = 0
        var final = 0.0, pending = 0.0

        mutating func add(_ t: DaemonData.InsightsSubmittedTrace) {
            submitted += 1
            if t.status == "accepted" { accepted += 1 }
            if let f = t.creditPointsFinal { final += f } else { pending += t.creditPointsPending }
            if t.matched { matched += 1 }
        }

        func tally(key: String? = nil, weekStart: String? = nil, label: String? = nil) -> DaemonData.InsightsSubmittedTally {
            .init(key: key, weekStart: weekStart, label: label, submitted: submitted, accepted: accepted,
                  creditFinal: final, creditPending: pending, matched: matched)
        }
    }

    static func edits(_ ts: [DaemonData.InsightsSubmittedTrace]) -> DaemonData.InsightsSubmittedEdits {
        .init(editsKnown: Int64(ts.compactMap(\.edits).reduce(0, +)),
              editsKeptKnown: Int64(ts.compactMap(\.editsKept).reduce(0, +)),
              editsUndoneKnown: Int64(ts.compactMap(\.editsUndone).reduce(0, +)),
              editsDeniedKnown: Int64(ts.compactMap(\.editsDenied).reduce(0, +)),
              editsDeniedPartial: ts.contains { $0.editsDenied == nil },
              linesAddedKnown: Int64(ts.compactMap(\.edits).reduce(0, +) * 9),
              linesRemovedKnown: Int64(ts.compactMap(\.edits).reduce(0, +) * 4))
    }

    static func spend(_ ts: [DaemonData.InsightsSubmittedTrace]) -> DaemonData.InsightsSubmittedSpend {
        .init(usdMicrosKnown: ts.compactMap(\.usdMicros).reduce(0, +), pricedSessions: ts.count)
    }

    /// The answer for `all` (newest first) as the daemon would give it for
    /// all time: weeks from the oldest trace's to the newest sample week.
    static func build(_ all: [DaemonData.InsightsSubmittedTrace] = traces()) -> DaemonData.InsightsSubmitted {
        var totals = Sums()
        var byStatus = ["accepted": 0, "pending": 0, "quarantined": 0, "rejected": 0, "withdrawn": 0, "other": 0]
        var reasons: [String: Int] = [:]
        for t in all {
            totals.add(t)
            byStatus[t.status, default: 0] += 1
            if let r = t.creditOutcomeReason { reasons[r, default: 0] += 1 }
        }
        let byModel = models.filter { m in all.contains { $0.model == m.label } }
            .map { m -> DaemonData.InsightsSubmittedModel in
            let ts = all.filter { $0.model == m.label }
            var s = Sums()
            ts.forEach { s.add($0) }
            var taskCounts: [String: Int] = [:]
            ts.compactMap(\.task).forEach { taskCounts[$0, default: 0] += 1 }
            return .init(label: m.label, tool: m.tool, tally: s.tally(label: m.label), spend: spend(ts), edits: edits(ts),
                         tasks: taskCounts)
        }.sorted { $0.tally.submitted > $1.tally.submitted }
        let byTask = ["bug_fix", "feature", "refactor", "tests", "docs", "understand", "review", "setup"].map { key in
            var s = Sums()
            all.filter { $0.task == key }.forEach { s.add($0) }
            return s.tally(key: key)
        }
        let oldest = all.compactMap { ISO8601DateFormatter().date(from: $0.submittedAt) }.min() ?? lastMonday
        let span = max(0, Int((lastMonday.timeIntervalSince(oldest) / (7 * 86_400)).rounded(.up)))
        let byWeek = (0...span).reversed().map { back -> DaemonData.InsightsSubmittedTally in
            let monday = lastMonday.addingTimeInterval(TimeInterval(-back * 7 * 86_400))
            let next = monday.addingTimeInterval(7 * 86_400)
            var s = Sums()
            all.filter { t in
                let at = ISO8601DateFormatter().date(from: t.submittedAt)!
                return at >= monday && at < next
            }.forEach { s.add($0) }
            return s.tally(weekStart: day(monday))
        }
        let byProject = projects.map { label -> DaemonData.InsightsSubmittedTally in
            var s = Sums()
            all.filter { $0.projectLabel == label }.forEach { s.add($0) }
            return s.tally(label: label)
        }.filter { $0.submitted > 0 }.sorted { $0.submitted > $1.submitted }
        let files = all.compactMap(\.files).reduce(DaemonData.InsightsSubmittedFiles()) {
            .init(source: $0.source + $1.source, test: $0.test + $1.test, doc: $0.doc + $1.doc,
                  config: $0.config + $1.config)
        }
        return DaemonData.InsightsSubmitted(
            updatedAt: iso(lastMonday.addingTimeInterval(4 * 86_400 + 12 * 3_600)), weeks: nil,
            windowStart: nil, windowEnd: iso(lastMonday.addingTimeInterval(4 * 86_400 + 12 * 3_600)),
            pricing: .init(tables: []),
            totals: .init(submitted: totals.submitted, accepted: totals.accepted, creditFinal: totals.final,
                          creditPending: totals.pending, matched: totals.matched,
                          unmatched: totals.submitted - totals.matched, byStatus: byStatus,
                          byOutcomeReason: reasons,
                          unmatchedByReason: ["no_queue_entry": 0, "source_not_read": 0, "no_session_row": 0]),
            byModel: byModel, byModelLeftOut: .init(unmatched: 0), byTask: byTask,
            byTaskUnknown: Sums().tally(),
            code: .init(sessions: all.count, sessionsWithFiles: all.count, files: files, edits: edits(all),
                        spend: spend(all)),
            byWeek: byWeek,
            byProject: byProject, leftOut: .init(), tracesTotal: all.count, traces: all)
    }
}
