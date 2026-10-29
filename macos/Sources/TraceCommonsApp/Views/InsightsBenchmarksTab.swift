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

    init(reader: Reader?) { self.reader = reader }

    /// Whether this daemon can answer at all.
    var available: Bool { reader != nil }

    func load() async {
        guard let reader else { return }
        busy = true
        defer { busy = false }
        do {
            answer = try await reader(weeks)
            failed = false
        } catch {
            answer = nil
            failed = true
        }
    }
}

/// The Benchmarks figures, kept pure for tests.
enum InsightsBenchmarkWords {
    struct Pick: Equatable, Identifiable {
        let id: String
        let title: String
        let model: String
        let figure: String
        let next: String
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

    /// Quality, value and speed, each present only when two models qualify.
    static func picks(_ category: DaemonData.InsightsBenchmarkCategory, copy: [String: String]) -> [Pick] {
        [("quality", category.quality), ("value", category.value), ("speed", category.speed)]
            .compactMap { kind, pick in
                guard let pick else { return nil }
                let next = InsightsOverviewWords.fill(text("analytics_bench_next", copy), [
                    "label": pick.next.label, "x": figure(kind, pick.next.figure, copy: copy),
                ])
                return Pick(id: kind, title: text("analytics_bench_" + kind, copy), model: pick.leader.label,
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
        case "rereads":
            return InsightsOverviewWords.fill(text("analytics_bench_tip_rereads", copy), [
                "k": String(tip.sessionsWith ?? 0), "n": String(tip.sessions ?? 0),
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

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
                header
                if let answer = model.answer, answer.enabled, answer.readable != false {
                    let categories = InsightsBenchmarkWords.ordered(answer)
                    if categories.allSatisfy({ $0.sessions == 0 }) {
                        Text(text("analytics_bench_empty")).insightsNote()
                    } else {
                        LazyVGrid(columns: [GridItem(.adaptive(minimum: 320), spacing: GlassTokens.Space.cardGap,
                                                     alignment: .top)],
                                  alignment: .leading, spacing: GlassTokens.Space.cardGap) {
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
            .padding(.vertical, GlassTokens.Space.s7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSurface()
        }
        .task(id: model.weeks) { await model.load() }
    }

    @ViewBuilder private var header: some View {
        @Bindable var model = model
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                if let sessions = model.answer?.sessions {
                    Text(InsightsOverviewWords.fill(text("analytics_bench_intro"), ["n": String(sessions)]))
                        .insightsHeading()
                }
                Spacer(minLength: 0)
                GlassSelect(text("analytics_tab_benchmarks"), selection: $model.weeks, options: [4, 13].map {
                    GlassPickerOption(InsightsOverviewWords.fill(text("analytics_bench_weeks"), ["n": String($0)]),
                                      value: $0)
                })
                .fixedSize()
            }
            HStack(spacing: GlassTokens.Space.s5) {
                ForEach(["quality", "value", "speed"], id: \.self) { kind in
                    Text(text("analytics_bench_" + kind) + " · " + text("analytics_bench_" + kind + "_detail"))
                        .insightsCaption()
                }
            }
            Text(InsightsOverviewWords.fill(text("analytics_bench_min"),
                                            ["n": String(model.answer?.minSessions ?? 5)])).insightsCaption()
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
            if picks.isEmpty {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(text("analytics_bench_not_enough")).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    Text(InsightsOverviewWords.fill(text("analytics_bench_not_enough_detail"), ["n": String(minimum)]))
                        .insightsCaption()
                }
            } else {
                HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                    ForEach(picks) { pick in
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                            Text(pick.title).insightsEyebrow()
                            Text(pick.model).glassType(GlassTokens.TypeScale.label.weight(.semibold)).lineLimit(1)
                            Text(pick.figure).insightsMono()
                            Text(pick.next).insightsCaption()
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityElement(children: .combine)
                    }
                }
            }
            if let tip = InsightsBenchmarkWords.tip(category.tip, copy: copy) {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
                    Image(systemName: "lightbulb").glassGlyph(12, weight: .semibold).accessibilityHidden(true)
                    Text(tip).insightsNote().fixedSize(horizontal: false, vertical: true)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .insightsCard()
    }
}
