#if DEBUG
import XCTest
import TCBridge
@testable import TCShellCore
@testable import TraceCommonsApp

/// Ron's summary inspector (#1146 `SummaryInspector`, Task 4 of the #1241
/// port): what the inspector shows with nothing selected. Its counts are
/// the core's, a missing one is a dash, and pending credit follows D6.
@MainActor
final class SummaryInspectorTests: XCTestCase {
    /// A recorded reply, decoded after `edit` is applied to its JSON object.
    private func recorded<T: Decodable>(
        _ type: T.Type, _ method: String, edit: (inout [String: Any]) -> Void = { _ in }
    ) throws -> T {
        let reply = try XCTUnwrap(SampleDaemonData.reply(method, in: .normalDay))
        var object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        edit(&object)
        return try DaemonDataDecoding.decoder().decode(T.self, from: JSONSerialization.data(withJSONObject: object))
    }

    private static func source(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Pending credit appears only beside the commons' statement of what it
    /// waits on (D6); without that statement it is a dash, never a figure.
    func test_pendingCreditFollowsD6() throws {
        let rollup = try recorded(DaemonData.HistoryRollup.self, "history_rollup") { $0["credit_pending"] = 12.5 }
        let known = try recorded(DaemonData.CommonsCreditSummary.self, "commons_credit_summary") {
            $0["posture_state"] = "known"
            $0["commons_settlement"] = "pending_review"
            $0["commons_settlement_explanation"] = "Scored when the commons reviews it."
        }
        let stated = SummaryFacts.pending(rollup: rollup, credit: known)
        XCTAssertEqual(stated.value, HomeFormat.points(12.5))
        XCTAssertEqual(stated.condition, "Scored when the commons reviews it.")

        // The recorded posture is unknown: no condition, so no figure.
        let unknown = try recorded(DaemonData.CommonsCreditSummary.self, "commons_credit_summary")
        XCTAssertNil(HomeFormat.pendingCondition(unknown))
        let unstated = SummaryFacts.pending(rollup: rollup, credit: unknown)
        XCTAssertEqual(unstated.value, "—")
        XCTAssertNil(unstated.condition)
        XCTAssertEqual(SummaryFacts.pending(rollup: rollup, credit: nil).value, "—")
        // A condition with no rollup is still no figure.
        XCTAssertEqual(SummaryFacts.pending(rollup: nil, credit: known).value, "—")

        // The view draws pending credit only through that rule, and says
        // the commons' condition beside it.
        let view = try Self.source("Views/Monitor/SummaryInspector.swift")
        XCTAssertTrue(view.contains("SummaryFacts.pending(rollup: rollup, credit: credit)"))
        XCTAssertEqual(view.components(separatedBy: "creditPending").count - 1, 1,
                       "the pending figure is read in one place, the D6 rule")
        XCTAssertTrue(view.contains("MonitorWords.creditNotCurrency"))
    }

    /// A count the core did not give is a dash, never a zero and never a
    /// number derived from something else (`queue_depth`, the tree).
    func test_aMissingCountIsADash() throws {
        let status = try recorded(DaemonData.Status.self, "status")
        XCTAssertEqual(SummaryFacts.waiting(status), "3")
        let unsaid = try recorded(DaemonData.Status.self, "status") { $0["decisions_owed"] = NSNull() }
        XCTAssertNotNil(unsaid.queueDepth)
        XCTAssertEqual(SummaryFacts.waiting(unsaid), "—", "never queue_depth")
        XCTAssertEqual(SummaryFacts.waiting(nil), "—")

        // Before the tree is read, nothing waiting is known: a dash, not 0.
        XCTAssertEqual(SummaryFacts.secondLook(loaded: false, sessions: []), "—")
        XCTAssertEqual(SummaryFacts.secondLook(loaded: true, sessions: []), "0")
        let held = try recordedEntry(["second_look": ["nothing-matched"]])
        let clear = try recordedEntry(["second_look": []])
        XCTAssertEqual(SummaryFacts.secondLook(loaded: true, sessions: [held, clear]), "1")
        XCTAssertEqual(SummaryFacts.kept(loaded: false, sessions: [held]), "—")
        XCTAssertEqual(SummaryFacts.kept(loaded: true, sessions: [held, clear]), "2")

        // Contributed is Ron's `isContributed`: accepted, waiting to be
        // scored and held for privacy review, all time (1 + 2 + 1 here),
        // the same set the statistics count; any part unsaid is a dash.
        XCTAssertEqual(SummaryFacts.contributed(nil), "—")
        let rollup = try recorded(DaemonData.HistoryRollup.self, "history_rollup")
        XCTAssertEqual(SummaryFacts.contributed(rollup), "4")
        for part in ["accepted", "submitted", "quarantined"] {
            let unsaid = try recorded(DaemonData.HistoryRollup.self, "history_rollup") {
                var all = $0["all_time"] as? [String: Any] ?? [:]
                all[part] = NSNull()
                $0["all_time"] = all
            }
            XCTAssertEqual(SummaryFacts.contributed(unsaid), "—", part)
        }
        let submittedOnly = try recorded(DaemonData.HistoryRollup.self, "history_rollup") {
            $0["all_time"] = ["submitted": 5, "accepted": 0, "quarantined": 0, "withdrawn": 0, "other": 0]
        }
        XCTAssertEqual(SummaryFacts.contributed(submittedOnly), "5", "waiting to be scored is contributed")

        // No budget is no line (Ron's guard); a budget missing a count says
        // a dash for it.
        let unbudgeted = try recorded(DaemonData.Status.self, "status") { $0["daily_budget"] = NSNull() }
        XCTAssertNil(SummaryFacts.uploads(unbudgeted))
        XCTAssertNil(SummaryFacts.uploads(nil))
        let budgeted = try XCTUnwrap(SummaryFacts.uploads(status))
        XCTAssertEqual(budgeted.count, "4")
        XCTAssertEqual(budgeted.max, "50")
        let partial = try recorded(DaemonData.Status.self, "status") {
            var budget = $0["daily_budget"] as? [String: Any] ?? [:]
            budget["uploads_today"] = NSNull()
            $0["daily_budget"] = budget
        }
        XCTAssertEqual(SummaryFacts.uploads(partial)?.count, "—")

        // The header's tool count is said only when the core said which
        // tools it reads; an unread one is left out, never "— of —".
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let unread = SummaryFacts.subline(words: words, destinations: nil, loaded: true, folders: 2, sessions: 1)
        XCTAssertFalse(unread.contains("—"))
        XCTAssertFalse(unread.contains("{"))
        let destinations = try recorded(DaemonData.ToolDestinations.self, "tool_destinations")
        let read = SummaryFacts.subline(words: words, destinations: destinations, loaded: true, folders: 2, sessions: 1)
        XCTAssertTrue(read.hasPrefix(FirstRunCopy.fill(words.summaryPanel.toolsWatched, [
            "count": String(destinations.watchedCount), "total": String(destinations.tools.count),
        ])))
        XCTAssertTrue(read.hasSuffix(words.counts.sessionsWaitingOne))
        // An unread tree counts nothing.
        let loading = SummaryFacts.subline(words: words, destinations: nil, loaded: false, folders: 0, sessions: 0)
        XCTAssertEqual(loading, "")
    }

    /// `count` copies of the recording's first history row, each with its
    /// own id and `fields` applied.
    private func historyRows(_ count: Int, _ fields: [String: Any]) throws -> [DaemonData.HistoryRow] {
        let reply = try XCTUnwrap(SampleDaemonData.reply("list_history", in: .normalDay))
        let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(reply.utf8)) as? [String: Any])
        let first = try XCTUnwrap((object["history"] as? [[String: Any]])?.first)
        return try (0..<count).map { index in
            var row = first
            row.merge(fields) { _, new in new }
            row["submission_id"] = "row-\(index)"
            row["session_hash"] = "sha256:row-\(index)"
            return try DaemonDataDecoding.decoder().decode(
                DaemonData.HistoryRow.self, from: JSONSerialization.data(withJSONObject: row))
        }
    }

    /// The per-project and per-tool contributed counts come from one page
    /// of history. A page the daemon capped is not a whole count: it is a
    /// dash, as an unread history is, and ranking falls back to waiting.
    func test_aCappedHistoryCountsNoContributed() throws {
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let counts = words.counts
        let fields: [String: Any] = ["project_id": "p1", "source": "claude-code", "status": "accepted"]
        let session = try recordedEntry(["project_id": "p1", "source": "claude-code", "declared_source": "claude-code"])
        let folder = TracesTree.FolderNode(id: "p1", label: "api", mode: nil, sessions: [session])
        let contributed = { (n: String) in FirstRunCopy.fill(counts.contributedCount, ["count": n]) }

        let whole = try historyRows(HomeStore.historyLimit - 1, fields)
        let project = try XCTUnwrap(SummaryFacts.topProjects(folders: [folder], history: whole, counts: counts).first)
        XCTAssertTrue(project.sub.contains(contributed(String(HomeStore.historyLimit - 1))), project.sub)
        let tool = try XCTUnwrap(SummaryFacts.topTools(sessions: [session], history: whole, counts: counts).first)
        XCTAssertTrue(tool.sub.contains(contributed(String(HomeStore.historyLimit - 1))), tool.sub)

        let capped = try historyRows(HomeStore.historyLimit, fields)
        let cappedProject = try XCTUnwrap(
            SummaryFacts.topProjects(folders: [folder], history: capped, counts: counts).first)
        XCTAssertTrue(cappedProject.sub.contains(contributed("—")), cappedProject.sub)
        XCTAssertFalse(cappedProject.sub.contains(contributed(String(HomeStore.historyLimit))))
        let cappedTool = try XCTUnwrap(SummaryFacts.topTools(sessions: [session], history: capped, counts: counts).first)
        XCTAssertTrue(cappedTool.sub.contains(contributed("—")), cappedTool.sub)

        // A capped page ranks on waiting alone: a tool with only capped
        // history and nothing waiting is not a top tool.
        let elsewhere = try historyRows(HomeStore.historyLimit, ["source": "codex", "status": "accepted"])
        let ranked = SummaryFacts.topTools(sessions: [session], history: elsewhere, counts: counts)
        XCTAssertEqual(ranked.map(\.id), [session.declaredSource ?? session.source])
    }

    /// A `witness_capacity` the daemon reported but this build cannot read
    /// is never "none waiting": the core's unreadable line is drawn among
    /// the safeguards above every inspector (Ron's `QueueStatusPanel`).
    func test_anUnreadableWitnessCapacityIsSaid() throws {
        let copy = try XCTUnwrap(MonitorWords.table?.safeguards)
        let fine = try recorded(DaemonData.Status.self, "status")
        XCTAssertFalse(TracesStore.safeguards(fine).contains { $0.title == copy.capacityUnreadable })

        let noCount = try recorded(DaemonData.Status.self, "status") {
            $0["witness_capacity"] = ["next_retry_at": NSNull()]
        }
        XCTAssertTrue(TracesStore.safeguards(noCount).contains { $0.title == copy.capacityUnreadable })
        let negative = try recorded(DaemonData.Status.self, "status") {
            $0["witness_capacity"] = ["waiting_sessions": -1, "next_retry_at": NSNull()]
        }
        XCTAssertTrue(TracesStore.safeguards(negative).contains { $0.title == copy.capacityUnreadable })
        let none = try recorded(DaemonData.Status.self, "status") {
            $0["witness_capacity"] = ["waiting_sessions": 0, "next_retry_at": NSNull()]
        }
        XCTAssertFalse(TracesStore.safeguards(none).contains { $0.title == copy.capacityUnreadable })

        // The unreadable line says what the daemon's witness-saturated
        // label would, so that label's generic line steps aside for it
        // (Ron's `saturatedShownByNotice`): one capacity line, not two.
        let saturated = try recorded(DaemonData.Status.self, "status") {
            $0["witness_capacity"] = ["next_retry_at": NSNull()]
            $0["health"] = ["last_error_label": "witness-saturated", "since": NSNull()]
        }
        let lines = TracesStore.safeguards(saturated)
        XCTAssertTrue(lines.contains { $0.title == copy.capacityUnreadable })
        let generic = HealthCopy.core(label: "witness-saturated", maxQueueEntries: nil)
        XCTAssertFalse(lines.contains { $0.title == generic.title }, "the generic saturated line steps aside")

        // Without the screens table's unreadable line, the capacity is
        // still said, in the core's witness-saturated line: never the
        // silence that reads as "none waiting". Said once, not twice.
        let fallback = TracesStore.safeguards(noCount, capacityUnreadable: nil)
        XCTAssertEqual(fallback.filter { $0.title == generic.title }.count, 1, "\(fallback.map(\.title))")
        let fallbackSaturated = TracesStore.safeguards(saturated, capacityUnreadable: nil)
        XCTAssertEqual(fallbackSaturated.filter { $0.title == generic.title }.count, 1)
        XCTAssertTrue(TracesStore.safeguards(none, capacityUnreadable: nil).isEmpty)
    }

    /// HomeStore keeps the last good history, credit and rollup when a read fails.
    /// The Summary does not show them as current: a failed `list_history`
    /// counts no contributed (a dash), and a failed
    /// `commons_credit_summary` states no condition, so pending is a dash.
    func test_aFailedReadIsNotShownAsCurrent() throws {
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let counts = words.counts
        let fields: [String: Any] = ["project_id": "p1", "source": "claude-code", "status": "accepted"]
        let session = try recordedEntry(["project_id": "p1", "source": "claude-code", "declared_source": "claude-code"])
        let folder = TracesTree.FolderNode(id: "p1", label: "api", mode: nil, sessions: [session])
        let contributed = { (n: String) in FirstRunCopy.fill(counts.contributedCount, ["count": n]) }
        let rows = try historyRows(3, fields)

        XCTAssertEqual(SummaryFacts.fresh(rows, unless: nil)?.count, 3)
        let stale = SummaryFacts.fresh(rows, unless: .unreachable)
        XCTAssertNil(stale)
        let project = try XCTUnwrap(SummaryFacts.topProjects(folders: [folder], history: stale, counts: counts).first)
        XCTAssertTrue(project.sub.contains(contributed("—")), project.sub)
        let tool = try XCTUnwrap(SummaryFacts.topTools(sessions: [session], history: stale, counts: counts).first)
        XCTAssertTrue(tool.sub.contains(contributed("—")), tool.sub)

        let rollup = try recorded(DaemonData.HistoryRollup.self, "history_rollup") { $0["credit_pending"] = 12.5 }
        let known = try recorded(DaemonData.CommonsCreditSummary.self, "commons_credit_summary") {
            $0["posture_state"] = "known"
            $0["commons_settlement"] = "pending_review"
            $0["commons_settlement_explanation"] = "Scored when the commons reviews it."
        }
        XCTAssertEqual(
            SummaryFacts.pending(rollup: rollup, credit: SummaryFacts.fresh(known, unless: nil)).value,
            HomeFormat.points(12.5))
        let failed = SummaryFacts.pending(rollup: rollup, credit: SummaryFacts.fresh(known, unless: .unreachable))
        XCTAssertEqual(failed.value, "—")
        XCTAssertNil(failed.condition)
        // A failed history_rollup is not shown as current either: no
        // contributed count and no pending figure, even beside a fresh
        // condition (D6).
        let staleRollup = SummaryFacts.fresh(rollup, unless: .unreachable)
        XCTAssertNil(staleRollup)
        XCTAssertEqual(SummaryFacts.pending(rollup: staleRollup, credit: known).value, "—")
        XCTAssertEqual(SummaryFacts.contributed(staleRollup), "—")

        // The view reads history, credit and the rollup only through that rule.
        let view = try Self.source("Views/Monitor/SummaryInspector.swift")
        XCTAssertTrue(view.contains("SummaryFacts.fresh(home.history, unless: home.failures[\"list_history\"])"))
        XCTAssertTrue(view.contains(
            "SummaryFacts.fresh(home.credit, unless: home.failures[\"commons_credit_summary\"])"))
        XCTAssertFalse(view.contains("history: home.history"), "no stale history reaches the statistics")
        XCTAssertFalse(view.contains("credit: home.credit"), "no stale credit reaches pending")
        XCTAssertTrue(view.contains(
            "SummaryFacts.fresh(home.rollup, unless: home.failures[\"history_rollup\"])"))
        XCTAssertEqual(
            view.components(separatedBy: "home.rollup").count - 1, 1,
            "the rollup is read once, through fresh(...)")
    }

    /// The legend says its words or nothing: without the screens table it
    /// is not drawn, as Decisions is not, never a bare number.
    func test_theLegendNeedsItsWords() throws {
        let view = try Self.source("Views/Monitor/SummaryInspector.swift")
        XCTAssertFalse(view.contains("MonitorWords.table?.shared ?? \"\""))
        XCTAssertFalse(view.contains("MonitorWords.table?.kept ?? \"\""))
        let guarded = try XCTUnwrap(view.range(of: "if let table = MonitorWords.table {"))
        let legend = try XCTUnwrap(view.range(of: "GlassLegendCell("))
        XCTAssertLessThan(guarded.lowerBound, legend.lowerBound)
        XCTAssertTrue(view.contains("table.shared, value:"))
        XCTAssertTrue(view.contains("table.kept, value:"))
        // The rollup's failure notice too: its words, or no notice.
        XCTAssertFalse(view.contains("MonitorWords.table?.line(for: failure) ?? \"\""))
        XCTAssertTrue(view.contains(
            "if let failure = home.failures[\"history_rollup\"], let table = MonitorWords.table {"))
        XCTAssertTrue(view.contains("GlassNotice(tone: .outside, title: table.line(for: failure))"))
    }

    /// The daily limit row is Ron's: uploads and megabytes left, and the
    /// sessions the limit holds, in the core's words; not a repeat of the
    /// Decisions line's uploads today.
    func test_theDailyLimitRowSaysWhatIsLeft() throws {
        let copy = try XCTUnwrap(MonitorWords.table?.safeguards)
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        let status = try recorded(DaemonData.Status.self, "status") {
            $0["daily_budget"] = [
                "bytes_today": 0, "max_bytes_per_day": 0, "bytes_remaining": 3 * 1024 * 1024,
                "uploads_today": 4, "max_uploads_per_day": 50, "uploads_remaining": 46,
                "resets_at": NSNull(), "blocked": true, "blocked_entries": 2, "blocked_bytes": 10,
            ]
        }
        let rows = SummaryFacts.safeguardRows(status, copy: copy)
        let limit = try XCTUnwrap(rows.first { $0.label == copy.dailyLimit })
        XCTAssertEqual(limit.value, FirstRunCopy.fill(copy.remaining, ["uploads": "46", "megabytes": "3"]))
        XCTAssertEqual(SummaryFacts.heldByLimit(status.dailyBudget, copy: copy),
                       FirstRunCopy.fill(copy.heldByLimit, ["count": "2"]))
        XCTAssertNotEqual(limit.value, FirstRunCopy.fill(words.summaryPanel.uploadsToday, ["count": "4", "max": "50"]))

        // A part unsaid is a dash; nothing held, or a limit not reached,
        // says no held line.
        let partial = try recorded(DaemonData.Status.self, "status") {
            $0["daily_budget"] = ["uploads_remaining": 46, "blocked": false, "blocked_entries": 0]
        }
        let partialRow = try XCTUnwrap(
            SummaryFacts.safeguardRows(partial, copy: copy).first { $0.label == copy.dailyLimit })
        XCTAssertEqual(partialRow.value, FirstRunCopy.fill(copy.remaining, ["uploads": "46", "megabytes": "—"]))
        XCTAssertNil(SummaryFacts.heldByLimit(partial.dailyBudget, copy: copy))
        let one = try recorded(DaemonData.Status.self, "status") {
            $0["daily_budget"] = ["blocked": true, "blocked_entries": 1]
        }
        XCTAssertEqual(SummaryFacts.heldByLimit(one.dailyBudget, copy: copy), copy.heldByLimitOne)
    }

    /// The certificates held and why sessions stopped waiting are the
    /// Summary's, in the core's words; the host draws the Summary only
    /// through `SummaryInspector`, under its banners and prompts.
    func test_certificatesAndNoLongerWaitingLiveHere() throws {
        let view = try Self.source("Views/Monitor/SummaryInspector.swift")
        let certificates = try XCTUnwrap(view.range(of: "CertificateSection(entries: awaitingDecision)"))
        let outcomes = try XCTUnwrap(view.range(of: "NotOfferedGlassDisclosure(counts: outcomeCounts, words: words?.summaryPanel)"))
        XCTAssertLessThan(certificates.lowerBound, outcomes.lowerBound, "Ron's order: certificates, then outcomes")
        // An unread queue is not an empty one.
        XCTAssertTrue(view.contains("if queueAnswered {"))

        let host = try Self.source("Views/Monitor/TracesInspectorHost.swift")
        XCTAssertTrue(host.contains("SummaryInspector("))
        XCTAssertFalse(host.contains("CertificateSection("), "the Summary draws the certificates")
        XCTAssertFalse(host.contains("NotOfferedGlassDisclosure("), "the Summary draws the outcomes")
        XCTAssertFalse(host.contains("HomeSummaryInspector("), "Ron's Summary replaces Home's")

        // The disclosure says the core's summary words when it has them.
        let words = try XCTUnwrap(MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON()))
        XCTAssertEqual(NotOfferedGlassDisclosure.title(4, words: words.summaryPanel),
                       FirstRunCopy.fill(words.summaryPanel.noLongerWaiting, ["count": "4"]))
        XCTAssertEqual(NotOfferedGlassDisclosure.scope(words: words.summaryPanel), words.summaryPanel.noLongerWaitingScope)
    }
}
#endif
