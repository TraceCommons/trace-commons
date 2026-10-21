import Foundation
import Observation
import TCShellCore

/// The Patterns "Models" card's answers from the daemon
/// (`insights_model_answers`, spec 2026-10-20 sections 7 and 8). Held only
/// while the daemon answered enabled and readable with its rows. Counting
/// off, an unreadable store, an older daemon that refuses the method, no
/// daemon and any failed read all leave it `nil`, and the card is not drawn:
/// a card that cannot be used is hidden, never greyed.
@Observable @MainActor
final class InsightsModelAnswersModel {
    typealias Reader = @Sendable () async throws -> DaemonData.InsightsModelAnswers

    private let reader: Reader?
    private(set) var task: Task<Void, Never>?
    /// Bumped by every read and by `close()`. A read whose generation is no
    /// longer current lands nothing: daemon reads run concurrently and can
    /// finish out of order, and cancelling the task does not stop one.
    private var generation = 0
    private(set) var answers: DaemonData.InsightsModelAnswers?

    init(reader: Reader?) { self.reader = reader }

    /// Read the answers again; on any failure the card goes away rather
    /// than keeping figures it can no longer stand behind. Only the latest
    /// read assigns.
    func load() async {
        generation += 1
        let token = generation
        guard let reader else { answers = nil; return }
        let read: DaemonData.InsightsModelAnswers?
        do {
            read = try await reader()
        } catch {
            read = nil
        }
        guard generation == token, !Task.isCancelled else { return }
        answers = read.flatMap { $0.showsAnswers && !InsightsModelsWords.rows($0).isEmpty ? $0 : nil }
    }

    @discardableResult
    func open() -> Task<Void, Never> {
        task?.cancel()
        let read = Task<Void, Never> { [weak self] in await self?.load() }
        task = read
        return read
    }

    /// Leaving Insights drops the figures, so reopening never draws the old
    /// ones, and any read still out lands nothing.
    func close() {
        task?.cancel(); task = nil
        generation += 1
        answers = nil
    }
}

/// The core's Models words, filled. Nothing here composes a sentence: each
/// line is one `analytics_models_*` template whose holes take the daemon's
/// figures, and the scope is said on every row, so a screenshot keeps it.
enum InsightsModelsWords {
    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }
    private static let fill = InsightsOverviewWords.fill

    static let rejectionQuestions: Set<String> = [
        "bug_fix_rejected", "refactor_rejected", "tests_rejected", "rejected_across_tasks",
    ]

    /// The questions this build has words for, in the daemon's order.
    static let questions = ["bug_fix_rejected", "refactor_rejected", "tests_rejected",
                            "docs_cost", "rejected_across_tasks", "finish_time"]
    static let states: Set<String> = ["answered", "no_clear_difference", "not_priced", "not_enough_sessions"]
    static let editModes: Set<String> = ["prompting", "not_prompting"]

    /// The words a rejection basis is drawn with: the suffix on the
    /// question's title key and the figure's key. A basis that is not here,
    /// such as a judged one, is not drawn at all; adding one is an entry
    /// here plus its title, rate and `analytics_models_basis_` keys.
    static let bases: [String: (title: String, rate: String)] = [
        "rejected": ("", "analytics_models_rate_rejected"),
        "undone": ("_undone", "analytics_models_rate_undone"),
    ]

    private static func comparesTwo(_ row: DaemonData.InsightsModelAnswerRow) -> Bool {
        row.state == "answered" || row.state == "no_clear_difference"
    }

    /// Whether this build can say what the row means. A question, state,
    /// basis or approval mode a later daemon adds is left off the card
    /// rather than drawn as another state, under a blank heading, or in
    /// the rejected wording; so is a comparison missing one of its models.
    static func isDrawable(_ row: DaemonData.InsightsModelAnswerRow) -> Bool {
        guard questions.contains(row.question), states.contains(row.state) else { return false }
        guard comparesTwo(row) else { return true }
        guard row.leader != nil, row.next != nil else { return false }
        if rejectionQuestions.contains(row.question) {
            return row.basis.map { bases[$0] != nil } ?? false
        }
        if row.question == "finish_time" {
            return row.editMode.map(editModes.contains) ?? false
        }
        return true
    }

    /// The rows the card draws, in the daemon's order.
    static func rows(_ answers: DaemonData.InsightsModelAnswers) -> [DaemonData.InsightsModelAnswerRow] {
        (answers.rows ?? []).filter(isDrawable)
    }

    /// The question as a scoped fact. A rejection question on the undone
    /// basis says undone edits in the question itself.
    static func title(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> String {
        let suffix = rejectionQuestions.contains(row.question) ? row.basis.flatMap { bases[$0]?.title } ?? "" : ""
        return text("analytics_models_" + row.question + suffix, copy)
    }

    /// Every row's scope: in your traces, labels as recorded, task type
    /// inferred.
    static func scope(copy: [String: String]) -> String { text("analytics_models_scope", copy) }

    /// The answer's window, the 13 weeks ending when it was built.
    static func window(_ answers: DaemonData.InsightsModelAnswers, copy: [String: String]) -> String? {
        guard let start = answers.windowStart, let end = answers.windowEnd else { return nil }
        return fill(text("analytics_models_window", copy),
                    ["range": InsightsOverviewWords.weekRange(start: String(start.prefix(10)),
                                                              end: String(end.prefix(10)))])
    }

    /// USD from integer micros, in the user's locale.
    static func money(_ micros: UInt64) -> String {
        (Double(micros) / 1_000_000).formatted(.currency(code: "USD").precision(.fractionLength(2 ... 4)))
    }

    /// A median active time, hours, minutes and seconds, two units at most.
    static func duration(_ seconds: UInt64) -> String {
        Duration.seconds(Int64(clamping: seconds))
            .formatted(.units(allowed: [.hours, .minutes, .seconds], width: .abbreviated, maximumUnitCount: 2))
    }

    /// A known rate as a percent. Whole percent, except that a rate that is
    /// not zero never rounds to 0 and one that is not all never rounds to
    /// 100: those show a tenth of a percent.
    static func rate(_ permille: UInt64) -> String {
        if (1 ... 4).contains(permille) || (995 ... 999).contains(permille) {
            return (Double(permille) / 10).formatted(.number.precision(.fractionLength(1)))
        }
        return InsightsOverviewWords.percent(permille)
    }

    /// Interrupts per trace, to a tenth, and the template that agrees with
    /// it: the singular only when it reads as exactly 1. A rate below a
    /// twentieth is shown to the thousandth, so it never prints as 0.
    static func interrupts(_ permille: Int, copy: [String: String]) -> String {
        let tenths = (Double(permille) / 100).rounded() / 10
        let shown: String
        if permille > 0, tenths == 0 {
            shown = (Double(permille) / 1000).formatted(.number.precision(.fractionLength(0 ... 3)))
        } else {
            shown = tenths.formatted(.number.precision(.fractionLength(0 ... 1)))
        }
        let key = tenths == 1 ? "analytics_models_interrupts_one" : "analytics_models_interrupts"
        return fill(text(key, copy), ["x": shown])
    }

    /// A model's figure in the row's words; the dash for a kind this build
    /// does not know, never a zero.
    static func figure(_ value: DaemonData.InsightsModelFigureValue, basis: String?,
                       copy: [String: String]) -> String {
        switch value.kind {
        case "rate_permille":
            guard let permille = value.value, let key = basis.flatMap({ bases[$0]?.rate }) else { break }
            return fill(text(key, copy), ["p": rate(permille)])
        case "usd_micros":
            guard let micros = value.value else { break }
            return fill(text("analytics_models_cost", copy), ["x": money(micros)])
        case "finish_time":
            guard let seconds = value.medianActiveSecs, let turns = value.medianUserTurns else { break }
            return fill(text("analytics_models_finish", copy), ["span": duration(seconds), "q": String(turns)])
        default:
            break
        }
        return text("analytics_unavailable", copy)
    }

    /// One model's line: its label, figure and traces, then its interrupts
    /// beside it when known (never added to the figure, never shown as 0
    /// when unknown).
    static func modelLine(_ model: DaemonData.InsightsModelFigure, key: String, basis: String?,
                          copy: [String: String]) -> String {
        let line = fill(text(key, copy), [
            "label": model.label,
            "x": figure(model.figure, basis: basis, copy: copy),
            "n": String(model.sessions),
        ])
        guard let permille = model.interruptsPerSessionPermille else { return line }
        return line + " \u{b7} " + interrupts(permille, copy: copy)
    }

    /// Each model's traces, for a row that does not compare.
    private static func counts(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> [String] {
        row.sessionsByModel.map { count in
            let key = count.sessions == 1 ? "analytics_models_count_one" : "analytics_models_count_other"
            return fill(text(key, copy), ["label": count.label, "n": String(count.sessions)])
        }
    }

    /// The row's lines under its question: the leader and the next model;
    /// "No clear difference" and both figures, neither leading; or the
    /// state with each model's traces.
    static func lines(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> [String] {
        switch row.state {
        case "answered":
            if let leader = row.leader, let next = row.next {
                return [modelLine(leader, key: "analytics_models_leader", basis: row.basis, copy: copy),
                        modelLine(next, key: "analytics_models_next", basis: row.basis, copy: copy)]
            }
        case "no_clear_difference":
            if let leader = row.leader, let next = row.next {
                return [text("analytics_models_no_clear_difference", copy),
                        modelLine(leader, key: "analytics_models_leader", basis: row.basis, copy: copy),
                        modelLine(next, key: "analytics_models_leader", basis: row.basis, copy: copy)]
            }
        case "not_priced":
            return [text("analytics_models_not_priced", copy), text("analytics_models_not_priced_line", copy)]
                + counts(row, copy: copy)
        default:
            break
        }
        return [text("analytics_models_not_enough", copy)] + counts(row, copy: copy)
    }

    /// What the figures beside it rest on, shown only beside figures: the
    /// rejection basis, or the approval mode a time is compared within.
    static func basisLine(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> String? {
        guard comparesTwo(row) else { return nil }
        if rejectionQuestions.contains(row.question), let basis = row.basis, bases[basis] != nil {
            return copy["analytics_models_basis_" + basis]
        }
        if row.question == "finish_time", let mode = row.editMode, editModes.contains(mode) {
            return copy["analytics_models_mode_" + mode]
        }
        return nil
    }

    /// Behind the row's "i": how its figures are measured, the floors and
    /// the noise test, then each reason that left a trace out, with its
    /// count. A reason that left none out is not listed.
    static func info(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> [String] {
        let measured: [String]
        if rejectionQuestions.contains(row.question) {
            measured = ["analytics_models_info_rejected", "analytics_models_info_codex",
                        "analytics_models_info_interrupts"]
        } else if row.question == "docs_cost" {
            measured = ["analytics_models_info_cost"]
        } else {
            measured = ["analytics_models_info_time"]
        }
        let shared = ["analytics_models_info_floor", "analytics_models_info_majority", "analytics_models_info_noise"]
        let leftOut = row.leftOut.counts.filter { $0.count > 0 }.map { reason in
            fill(text("analytics_models_left_out_" + reason.reason, copy), ["n": String(reason.count)])
        }
        return (measured + shared).map { text($0, copy) } + leftOut
    }

    /// Behind the card title's "i": where the traces come from, that labels
    /// are as recorded, and that task types are inferred.
    static func cardInfo(copy: [String: String]) -> [String] {
        ["analytics_models_info_scope", "analytics_model_labels_note", "analytics_models_info_task"]
            .map { text($0, copy) }
    }

    /// The row as VoiceOver reads it: the question, its lines, its basis
    /// and its scope, as one element. A part that already ends a sentence
    /// is not given a second full stop, and an empty part is skipped.
    static func accessibilityLabel(_ row: DaemonData.InsightsModelAnswerRow, copy: [String: String]) -> String {
        let parts = ([title(row, copy: copy)] + lines(row, copy: copy) + [basisLine(row, copy: copy), scope(copy: copy)]
            .compactMap { $0 })
            .filter { !$0.isEmpty }
        return parts.enumerated().reduce(into: "") { spoken, part in
            if part.offset > 0 {
                spoken += [".", "?", "!"].contains(spoken.last.map(String.init) ?? "") ? " " : ". "
            }
            spoken += part.element
        }
    }
}
