import SwiftUI
import TCDesign
import TCShellCore

/// Benchmarks (product owner, 2026-10-29): like openrouter.ai/benchmarks,
/// but from the person's own traces. One card per kind of work, each with
/// the model that leads on quality (edits kept), value (cost per trace at
/// API prices) and speed (time per trace), the runner-up under it, and one
/// observation from those traces. Models are compared only where each has
/// enough traces; otherwise the card says so.
@MainActor
@Observable
final class InsightsBenchmarksModel {
    typealias Reader = @Sendable (Int) async throws -> DaemonData.InsightsBenchmarks

    private let reader: Reader?
    private(set) var answer: DaemonData.InsightsBenchmarks?
    private(set) var failed = false
    private(set) var busy = false
    /// 4 or 13 weeks.
    var weeks = 4

    /// Whether a read has finished at least once, or there is nothing to
    /// read: until then the get-started card is not drawn.
    private(set) var settled = false

    init(reader: Reader?) { self.reader = reader }

    /// A fixed answer and no reader: the sample under the get-started card.
    init(sample: DaemonData.InsightsBenchmarks) {
        reader = nil; answer = sample; settled = true
    }

    /// Whether this daemon can answer at all.
    var available: Bool { reader != nil }

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

/// The Benchmarks figures, kept pure for tests.
enum InsightsBenchmarkWords {
    struct Pick: Equatable, Identifiable {
        let id: String
        let title: String
        /// What the label measures, behind its "i".
        let info: String
        let model: String
        let figure: String
        let next: String
    }

    /// One model's own figures on a kind of work, shown from its first
    /// trace (product owner, 2026-10-29); a figure the daemon could not
    /// make is the dash.
    struct ModelRow: Equatable, Identifiable {
        let id: String
        let model: String
        let traces: String
        let kept: String
        let cost: String
        let time: String
    }

    static func rows(_ category: DaemonData.InsightsBenchmarkCategory, copy: [String: String]) -> [ModelRow] {
        let dash = text("analytics_unavailable", copy)
        return category.models
            .sorted { $0.sessions != $1.sessions ? $0.sessions > $1.sessions : $0.label < $1.label }
            .map { model in
                ModelRow(id: model.label + "/" + model.tool, model: model.label,
                         traces: traces(model.sessions, copy: copy),
                         kept: model.keptPermille.map { String(Int((Double($0) / 10).rounded())) + "%" } ?? dash,
                         cost: model.usdMicrosMedian.map { InsightsCreditWords.amount($0, copy: copy) } ?? dash,
                         time: model.activeSecsMedian.map { duration($0) } ?? dash)
            }
    }

    /// One model's work mix: each kind of work it was used for, with its
    /// share of that model's traces, most first (product owner, 2026-10-29:
    /// with one model, show what share went to what task).
    struct Mix: Equatable, Identifiable {
        struct Share: Equatable, Identifiable {
            let id: String
            let task: String
            let fraction: Double
            let label: String
        }
        let id: String
        let model: String
        let traces: String
        let shares: [Share]
    }

    static func mix(_ answer: DaemonData.InsightsBenchmarks, copy: [String: String]) -> [Mix] {
        var byModel: [String: (label: String, tasks: [(key: String, sessions: Int)])] = [:]
        for category in ordered(answer) {
            for model in category.models where model.sessions > 0 {
                let id = model.label + "/" + model.tool
                byModel[id, default: (model.label, [])].tasks.append((category.key, model.sessions))
            }
        }
        return byModel.sorted { lhs, rhs in
            let l = lhs.value.tasks.reduce(0) { $0 + $1.sessions }, r = rhs.value.tasks.reduce(0) { $0 + $1.sessions }
            return l != r ? l > r : lhs.value.label < rhs.value.label
        }
        .map { id, entry in
            let total = entry.tasks.reduce(0) { $0 + $1.sessions }
            let shares = entry.tasks
                .sorted { $0.sessions != $1.sessions ? $0.sessions > $1.sessions : $0.key < $1.key }
                .map { task -> Mix.Share in
                    let fraction = Double(task.sessions) / Double(max(total, 1))
                    return Mix.Share(id: task.key, task: name(task.key, copy: copy), fraction: fraction,
                                     label: InsightsOverviewWords.fill(text("analytics_bench_mix_share", copy), [
                                         "p": String(Int((fraction * 100).rounded())),
                                         "n": traces(task.sessions, copy: copy),
                                     ]))
                }
            return Mix(id: id, model: entry.label, traces: traces(total, copy: copy), shares: shares)
        }
    }

    static let categories = ["bug_fix", "feature", "refactor", "tests", "docs", "understand", "review", "setup"]

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }

    static func name(_ key: String, copy: [String: String]) -> String { text("analytics_bench_cat_" + key, copy) }

    static func traces(_ count: Int, copy: [String: String]) -> String {
        InsightsOverviewWords.fill(text("analytics_bench_traces", copy), ["n": String(count)])
    }

    /// "18 min", "1h 5m".
    static func duration(_ seconds: Int64) -> String {
        let formatter = DateComponentsFormatter()
        formatter.unitsStyle = .abbreviated
        formatter.maximumUnitCount = 2
        formatter.allowedUnits = [.hour, .minute]
        formatter.zeroFormattingBehavior = .dropAll
        return formatter.string(from: TimeInterval(max(60, seconds))) ?? ""
    }

    static func figure(_ kind: String, _ value: Int64, copy: [String: String]) -> String {
        switch kind {
        case "quality":
            return InsightsOverviewWords.fill(text("analytics_bench_kept", copy),
                                              ["p": String(Int((Double(value) / 10).rounded()))])
        case "value":
            return InsightsOverviewWords.fill(text("analytics_bench_per_trace", copy),
                                              ["x": InsightsCreditWords.amount(value, copy: copy)])
        default:
            return duration(value)
        }
    }

    /// The traces actually compared: those on a kind of work, which is the
    /// window's total less everything in `left_out` (automated, no model,
    /// routed, ...). Not the window total (owner, 2026-10-29).
    static func compared(_ answer: DaemonData.InsightsBenchmarks) -> Int {
        (answer.categories ?? []).reduce(0) { $0 + $1.sessions }
    }

    /// Quality, value and speed, each present only when two models qualify.
    static func picks(_ category: DaemonData.InsightsBenchmarkCategory, copy: [String: String]) -> [Pick] {
        [("quality", category.quality), ("value", category.value), ("speed", category.speed)]
            .compactMap { kind, pick in
                guard let pick else { return nil }
                let next = InsightsOverviewWords.fill(text("analytics_bench_next", copy), [
                    "label": pick.next.label, "x": figure(kind, pick.next.figure, copy: copy),
                ])
                return Pick(id: kind, title: text("analytics_bench_" + kind, copy),
                            info: text("analytics_bench_" + kind + "_detail", copy), model: pick.leader.label,
                            figure: figure(kind, pick.leader.figure, copy: copy), next: next)
            }
    }

    /// The card's one observation, in the core's words; `nil` for a kind
    /// this build has no words for.
    static func tip(_ tip: DaemonData.InsightsBenchmarkTip?, copy: [String: String]) -> String? {
        guard let tip else { return nil }
        func percent(_ permille: Int?) -> String { String(Int((Double(permille ?? 0) / 10).rounded())) }
        switch tip.kind {
        case "test_first":
            return InsightsOverviewWords.fill(text("analytics_bench_tip_test_first", copy), [
                "p": percent(tip.fasterPermille), "k": String(tip.sessionsWithout ?? 0), "n": String(tip.sessions ?? 0),
            ])
        case "cheaper_same_quality":
            return InsightsOverviewWords.fill(text("analytics_bench_tip_cheaper", copy), [
                "first": tip.cheap ?? "", "second": tip.expensive ?? "", "p": percent(tip.costRatioPermille),
            ])
        case "fewer_undos":
            let ratio = Double(tip.ratioPermille ?? 0) / 1_000
            return InsightsOverviewWords.fill(text("analytics_bench_tip_fewer_undos", copy), [
                "first": tip.better ?? "", "second": tip.worse ?? "",
                "x": ratio.formatted(.number.precision(.fractionLength(0...1))),
            ])
        default:
            return nil
        }
    }

    /// The categories in the order the tab shows them, whatever order they came in.
    static func ordered(_ answer: DaemonData.InsightsBenchmarks) -> [DaemonData.InsightsBenchmarkCategory] {
        let byKey = Dictionary((answer.categories ?? []).map { ($0.key, $0) }, uniquingKeysWith: { first, _ in first })
        return categories.compactMap { byKey[$0] }
    }
}

struct InsightsBenchmarksTab: View {
    let model: InsightsBenchmarksModel
    let copy: [String: String]
    /// The get-started card over this tab drawn from the sample (product
    /// owner, 2026-10-29); `nil` for the person's own benchmarks.
    var getStarted: InsightsGetStarted?

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        InsightsTabPage(getStarted: getStarted) {
            header
            if let answer = model.answer, answer.enabled, answer.readable != false {
                // A kind of work with no trace has no card.
                let categories = InsightsBenchmarkWords.ordered(answer).filter { $0.sessions > 0 }
                if categories.isEmpty {
                    Text(text("analytics_bench_not_enough")).insightsNote()
                } else {
                    mixCard(answer)
                    InsightsCardFlow(minimum: 420) {
                        ForEach(categories) { category in
                            card(category, minimum: answer.minSessions ?? 5)
                        }
                    }
                }
            } else if model.failed || model.answer != nil || !model.available {
                Text(text("analytics_unavailable")).insightsNote()
            }
            if model.busy { GlassSpinner() }
        }
        .task(id: model.weeks) { await model.load() }
    }

    @ViewBuilder private var header: some View {
        @Bindable var model = model
        HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            if getStarted == nil, let answer = model.answer {
                Text(InsightsOverviewWords.fill(text("analytics_bench_intro"),
                                                ["n": String(InsightsBenchmarkWords.compared(answer))]))
                    .insightsHeading()
            }
            Spacer(minLength: 0)
            GlassSelect(text("analytics_tab_benchmarks"), selection: $model.weeks, options: [4, 13].map {
                GlassPickerOption(InsightsOverviewWords.fill(text("analytics_bench_weeks"), ["n": String($0)]),
                                  value: $0)
            })
            .fixedSize()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    private func card(_ category: DaemonData.InsightsBenchmarkCategory, minimum: Int) -> some View {
        let picks = InsightsBenchmarkWords.picks(category, copy: copy)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .firstTextBaseline) {
                Text(InsightsBenchmarkWords.name(category.key, copy: copy)).insightsHeading()
                Spacer(minLength: GlassTokens.Space.s3)
                Text(InsightsBenchmarkWords.traces(category.sessions, copy: copy)).insightsCaption()
            }
            if !picks.isEmpty {
                // One row per metric: its label and "i", the leading model
                // (the runner-up under it), the figure on the right.
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    ForEach(picks) { pick in
                        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                            HStack(spacing: GlassTokens.Space.s2) {
                                Text(pick.title).insightsEyebrow().lineLimit(1)
                                InsightsInfo(pick.title, [pick.info])
                            }
                            .frame(width: 84, alignment: .leading)
                            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                                Text(pick.model).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                                    .lineLimit(2).fixedSize(horizontal: false, vertical: true)
                                Text(pick.next).insightsCaption()
                            }
                            Spacer(minLength: GlassTokens.Space.s3)
                            Text(pick.figure).insightsMono()
                        }
                    }
                }
            }
            ownFigures(category)
            if picks.isEmpty {
                Text(InsightsOverviewWords.fill(text("analytics_bench_compare_later"), ["n": String(minimum)]))
                    .insightsCaption()
            }
            if let tip = InsightsBenchmarkWords.tip(category.tip, copy: copy) {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
                    Image(systemName: "lightbulb").glassGlyph(12, weight: .semibold).accessibilityHidden(true)
                    Text(tip).insightsNote().fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .insightsSection()
    }

    /// What share of each model's traces went to each kind of work: the
    /// one-model view (owner, 2026-10-29).
    @ViewBuilder private func mixCard(_ answer: DaemonData.InsightsBenchmarks) -> some View {
        let mixes = InsightsBenchmarkWords.mix(answer, copy: copy)
        if !mixes.isEmpty {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                Text(text("analytics_bench_mix_title")).insightsHeading()
                ForEach(mixes) { mix in
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(mix.model).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                            Spacer(minLength: GlassTokens.Space.s3)
                            Text(mix.traces).insightsCaption()
                        }
                        ForEach(mix.shares) { share in
                            HStack(spacing: GlassTokens.Space.s3) {
                                Text(share.task).insightsCaption()
                                    .frame(width: 170, alignment: .leading)
                                GeometryReader { proxy in
                                    Capsule().fill(GlassTokens.Color.chartGrid.color)
                                        .overlay(alignment: .leading) {
                                            Capsule().fill(GlassTokens.Color.chartSeries1.color)
                                                .frame(width: max(4, proxy.size.width * share.fraction))
                                        }
                                }
                                .frame(height: 8)
                                Text(share.label).insightsMono()
                                    .frame(width: 110, alignment: .trailing)
                            }
                            .accessibilityElement(children: .combine)
                        }
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSection()
        }
    }

    /// Every model used on this kind of work, with its own figures: edits
    /// kept, cost per trace and time per trace (typical, the median).
    @ViewBuilder private func ownFigures(_ category: DaemonData.InsightsBenchmarkCategory) -> some View {
        let rows = InsightsBenchmarkWords.rows(category, copy: copy)
        if !rows.isEmpty {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(text("analytics_bench_yours")).insightsEyebrow()
                Grid(alignment: .leading, horizontalSpacing: GlassTokens.Space.s4,
                     verticalSpacing: GlassTokens.Space.s2) {
                    GridRow {
                        Color.clear.gridCellUnsizedAxes([.horizontal, .vertical])
                        ForEach(["kept", "cost", "time"], id: \.self) { column in
                            Text(text("analytics_bench_col_" + column)).insightsCaption()
                                .frame(maxWidth: .infinity, alignment: .trailing)
                                .gridColumnAlignment(.trailing)
                        }
                    }
                    ForEach(rows) { row in
                        GridRow {
                            VStack(alignment: .leading, spacing: 0) {
                                Text(row.model).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                                    .lineLimit(1).truncationMode(.middle)
                                Text(row.traces).insightsCaption()
                            }
                            .frame(maxWidth: .infinity, alignment: .leading)
                            ForEach([row.kept, row.cost, row.time], id: \.self) { figure in
                                Text(figure).insightsMono().frame(maxWidth: .infinity, alignment: .trailing)
                            }
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        }
    }
}
