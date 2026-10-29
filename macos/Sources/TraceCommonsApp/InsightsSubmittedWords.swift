import Foundation
import TCDesign
import TCShellCore

/// Submitted (product owner, 2026-10-29: "I need to analyse submitted
/// work, not even admitted work, all the traces submitted"): the daemon's
/// `insights_submitted` answer for the period picked, read once per pick.
@MainActor
@Observable
final class InsightsSubmittedStore {
    typealias Reader = @Sendable (Int?) async throws -> DaemonData.InsightsSubmitted

    private let reader: Reader?
    private(set) var answer: DaemonData.InsightsSubmitted?
    private(set) var failed = false
    private(set) var busy = false
    /// 4 or 13 weeks, or `nil` for all time (the default).
    var weeks: Int?
    /// Whether a read has finished at least once, or there is nothing to
    /// read: until then the get-started card is not drawn.
    private(set) var settled = false
    /// Traces sent over all time, from the last all-time answer: the tab
    /// shows the get-started card only when it is 0, never because a
    /// shorter period holds none.
    private(set) var everSubmitted: Int?

    init(reader: Reader?) { self.reader = reader }

    /// A fixed answer and no reader: the sample under the get-started card.
    init(sample: DaemonData.InsightsSubmitted) {
        reader = nil; answer = sample; settled = true; everSubmitted = sample.totals?.submitted
    }

    /// The latest request; an earlier reply that lands after it is dropped.
    private var generation = 0

    func load() async {
        guard let reader else { settled = true; return }
        generation += 1
        let mine = generation, asked = weeks
        busy = true
        do {
            let reply = try await reader(asked)
            guard mine == generation else { return }
            answer = reply
            if asked == nil, reply.isAnswered { everSubmitted = reply.totals?.submitted ?? 0 }
            failed = false
            settled = true
        } catch is CancellationError {
            // Leaving the tab or picking another period is not a failure.
        } catch {
            guard mine == generation else { return }
            answer = nil
            failed = true
            settled = true
        }
        if mine == generation { busy = false }
    }
}

/// Submitted's figures and words, kept pure for tests. Every word is the
/// core's: the analytics table's, and History's own for statuses and for
/// why a trace earned nothing (`PublicRunCopy`).
enum InsightsSubmittedWords {
    /// The kinds of work, in the daemon's order.
    static let tasks = ["bug_fix", "feature", "refactor", "tests", "docs", "understand", "review", "setup"]

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static func fill(_ key: String, _ copy: [String: String], _ values: [String: String]) -> String {
        InsightsOverviewWords.fill(text(key, copy), values)
    }

    /// Whether Submitted has nothing of the person's own: no trace sent,
    /// ever, counting off, or no daemon to ask. A failed or unreadable
    /// read is not "nothing sent": the tab says it is unavailable.
    @MainActor static func isEmpty(_ store: InsightsSubmittedStore) -> Bool {
        guard store.settled else { return false }
        if let ever = store.everSubmitted { return ever == 0 }
        guard let answer = store.answer else { return !store.failed }
        if !answer.enabled { return true }
        guard answer.isAnswered else { return false }
        return answer.weeks == nil && (answer.totals?.submitted ?? 0) == 0
    }

    /// A kind of work's name.
    static func taskName(_ key: String?, copy: [String: String]) -> String {
        guard let key, tasks.contains(key) else { return text("analytics_sub_task_unknown", copy) }
        return text("analytics_bench_cat_" + key, copy)
    }

    /// Credit in points, as History shows it.
    static func points(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(1)))
    }

    static func count(_ value: Int) -> String { value.formatted(.number) }

    /// "1 trace", "{n} traces".
    static func traces(_ count: Int, copy: [String: String]) -> String {
        count == 1 ? text("analytics_sub_traces_one", copy)
            : fill("analytics_sub_traces", copy, ["n": self.count(count)])
    }

    static func intro(_ submitted: Int, copy: [String: String]) -> String {
        submitted == 1 ? text("analytics_sub_intro_one", copy)
            : fill("analytics_sub_intro", copy, ["n": count(submitted)])
    }

    /// Whole percent, never 0% for a share above nothing nor 100% short of all.
    static func percent(_ part: Int, of whole: Int) -> Int {
        guard whole > 0, part > 0 else { return 0 }
        let value = Int((Double(part) * 100 / Double(whole)).rounded())
        return part < whole ? min(99, max(1, value)) : 100
    }

    struct Tile: Equatable, Identifiable {
        let id: String
        let label: String
        let value: String
        let note: String
    }

    /// Submitted, Accepted with its rate, Credit received with pending, and
    /// the top earning model (the dash while no model has earned).
    static func tiles(_ answer: DaemonData.InsightsSubmitted, copy: [String: String]) -> [Tile] {
        guard let totals = answer.totals else { return [] }
        let top = topModel(answer)
        return [
            Tile(id: "submitted", label: text("analytics_sub_tile_submitted", copy), value: count(totals.submitted),
                 note: text("analytics_sub_tile_submitted_note", copy)),
            Tile(id: "accepted", label: text("analytics_sub_tile_accepted", copy), value: count(totals.accepted),
                 note: fill("analytics_sub_tile_accepted_rate", copy,
                            ["p": String(percent(totals.accepted, of: totals.submitted))])),
            Tile(id: "credit", label: text("analytics_sub_tile_credit", copy), value: points(totals.creditFinal),
                 note: fill("analytics_sub_tile_credit_pending", copy, ["n": points(totals.creditPending)])),
            Tile(id: "model", label: text("analytics_sub_tile_top_model", copy),
                 value: top?.label ?? text("analytics_unavailable", copy),
                 note: top.map { fill("analytics_sub_tile_top_model_note", copy, ["n": points($0.tally.creditFinal)]) }
                     ?? text("analytics_sub_tile_top_model_none", copy)),
        ]
    }

    /// The model with the most credit received; `nil` while none has any.
    static func topModel(_ answer: DaemonData.InsightsSubmitted) -> DaemonData.InsightsSubmittedModel? {
        models(answer).first { $0.tally.creditFinal > 0 }
    }

    /// By model: most credit received first, then most sent, then by name.
    static func models(_ answer: DaemonData.InsightsSubmitted) -> [DaemonData.InsightsSubmittedModel] {
        (answer.byModel ?? []).sorted { a, b in
            if a.tally.creditFinal != b.tally.creditFinal { return a.tally.creditFinal > b.tally.creditFinal }
            if a.tally.submitted != b.tally.submitted { return a.tally.submitted > b.tally.submitted }
            return a.label < b.label
        }
    }

    /// Spend at API prices, the dash when no part of it is known.
    static func spend(_ spend: DaemonData.InsightsSubmittedSpend, copy: [String: String]) -> String {
        guard spend.pricedSessions > 0 || spend.usdMicrosKnown > 0 else { return text("analytics_unavailable", copy) }
        return InsightsWorkWords.dollars(spend.usdMicrosKnown)
    }

    /// "12 of 14" edits kept, the dash with no edit known.
    static func kept(_ edits: DaemonData.InsightsSubmittedEdits, copy: [String: String]) -> String {
        guard edits.editsKnown > 0 else { return text("analytics_unavailable", copy) }
        return fill("analytics_sub_kept_of", copy, ["k": edits.editsKeptKnown.formatted(.number),
                                                    "n": edits.editsKnown.formatted(.number)])
    }

    /// The traces By model leaves out: unmatched, of mixed models, or with
    /// none on record.
    static func modelsLeftOut(_ answer: DaemonData.InsightsSubmitted) -> Int {
        guard let out = answer.byModelLeftOut else { return 0 }
        return out.unmatched + out.mixedModels + out.noModel
    }

    struct Reason: Equatable, Identifiable {
        let id: String
        let label: String
        let count: Int
        /// "Needs Train consent": History is where it is added.
        var needsConsent: Bool { id == "policy_mismatch" }
    }

    /// Why traces earned nothing, most first, in History's words. With no
    /// History words decoded the card has nothing to say and is left out.
    static func reasons(_ answer: DaemonData.InsightsSubmitted, labels: PublicRunCopy?) -> [Reason] {
        guard let labels else { return [] }
        return (answer.totals?.byOutcomeReason ?? [:])
            .filter { $0.value > 0 }
            .map { Reason(id: $0.key, label: labels.creditOutcomeLabel(for: $0.key), count: $0.value) }
            .sorted { $0.count != $1.count ? $0.count > $1.count : $0.label < $1.label }
    }

    /// The kinds of work with a trace sent, in the daemon's order, the
    /// unknown kind last.
    static func taskRows(_ answer: DaemonData.InsightsSubmitted) -> [DaemonData.InsightsSubmittedTally] {
        var rows = (answer.byTask ?? []).filter { $0.submitted > 0 }
        if let unknown = answer.byTaskUnknown, unknown.submitted > 0 {
            var row = unknown
            row.key = nil
            rows.append(row)
        }
        return rows
    }

    /// "3 of 5 accepted".
    static func acceptedOf(_ tally: DaemonData.InsightsSubmittedTally, copy: [String: String]) -> String {
        fill("analytics_sub_accepted_of", copy, ["k": count(tally.accepted), "n": count(tally.submitted)])
    }

    struct FileKind: Equatable, Identifiable {
        let id: String
        let label: String
        let count: Int
        let color: GlassRGBA
    }

    /// The file kinds with a file in them.
    static func fileKinds(_ files: DaemonData.InsightsSubmittedFiles, copy: [String: String]) -> [FileKind] {
        [("source", files.source, GlassTokens.Color.chartSeries1),
         ("test", files.test, GlassTokens.Color.chartSeries2),
         ("doc", files.doc, GlassTokens.Color.chartSeries3),
         ("config", files.config, GlassTokens.Color.chartSeries4)]
            .filter { $0.1 > 0 }
            .map { FileKind(id: $0.0, label: text("analytics_sub_files_" + $0.0, copy), count: $0.1, color: $0.2) }
    }

    /// Whether "Code in your traces" has anything: a file or an edit.
    static func hasCode(_ code: DaemonData.InsightsSubmittedCode?) -> Bool {
        guard let code else { return false }
        return code.files.total > 0 || code.edits.editsKnown > 0
    }

    /// The local calendar with weeks from Monday, as `by_week` counts them.
    static var mondayCalendar: Calendar {
        var calendar = Calendar.current
        calendar.firstWeekday = 2
        return calendar
    }

    /// A week's axis label, "Oct 19", from its Monday.
    static func weekLabel(_ weekStart: String?) -> String {
        weekDate(weekStart)?.formatted(.dateTime.month(.abbreviated).day()) ?? (weekStart ?? "")
    }

    /// A week's Monday (`YYYY-MM-DD`, local) at local noon, for the chart.
    static func weekDate(_ weekStart: String?) -> Date? {
        guard let weekStart else { return nil }
        let parts = weekStart.split(separator: "-").compactMap { Int($0) }
        guard parts.count == 3 else { return nil }
        return Calendar.current.date(from: DateComponents(year: parts[0], month: parts[1], day: parts[2], hour: 12))
    }

    /// A trace's day, "Oct 30", with the year when it is not this year's.
    static func day(_ submittedAt: String, now: Date = Date()) -> String {
        guard let date = try? Date(submittedAt, strategy: .iso8601) else { return "" }
        let calendar = Calendar.current
        let style = calendar.component(.year, from: date) == calendar.component(.year, from: now)
            ? Date.FormatStyle().month(.abbreviated).day()
            : Date.FormatStyle().month(.abbreviated).day().year()
        return date.formatted(style)
    }

    /// History's status for a submitted trace: the daemon's `pending` is the
    /// server's `submitted`; `other` reads History's unavailable word.
    static func historyStatus(_ status: String) -> String {
        switch status {
        case "pending": "submitted"
        case "other": ""
        default: status
        }
    }

    static func status(_ status: String, labels: PublicRunCopy?) -> (label: String, tone: GlassTag.Tone)? {
        let history = historyStatus(status)
        guard let label = HomeFormat.historyStatusLabel(copy: labels, history) else { return nil }
        return (label, HomeFormat.tone(history))
    }

    /// A trace's credit: its final figure, its pending figure, or nothing.
    static func credit(_ trace: DaemonData.InsightsSubmittedTrace, copy: [String: String]) -> String {
        if let final = trace.creditPointsFinal { return points(final) }
        if trace.creditPointsPending > 0 {
            return fill("analytics_sub_row_pending", copy, ["n": points(trace.creditPointsPending)])
        }
        return text("analytics_unavailable", copy)
    }

    /// The period picker's choices: all time, then 13 and 4 weeks.
    static func periods(copy: [String: String]) -> [(label: String, weeks: Int?)] {
        [(text("analytics_sub_period_all", copy), nil)]
            + [13, 4].map { (fill("analytics_sub_period_weeks", copy, ["n": String($0)]), $0) }
    }

    /// The rows "Your traces" shows before "Show all".
    static let visibleTraces = 8
}
