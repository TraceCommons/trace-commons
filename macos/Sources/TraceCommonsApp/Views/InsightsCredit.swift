import SwiftUI
import TCDesign
import TCShellCore

/// Where NEAR AI credit went (product owner, 2026-10-28): the daemon's
/// `insights_credit` answer, read on open. A failed or refused read, an old
/// daemon, or a ledger that cannot be read draws the unavailable line,
/// never a zero.
@Observable @MainActor
final class InsightsCreditModel {
    private(set) var credit: DaemonData.InsightsCredit?
    private(set) var failed = false

    func load(_ client: (any DaemonDataClient)?) async {
        guard let client else { return }
        do {
            credit = try await client.insightsCredit()
            failed = false
        } catch {
            credit = nil
            failed = true
        }
    }

    /// The answer, only when it could be read and the ledger was.
    var readable: DaemonData.InsightsCredit? {
        guard let credit, credit.enabled, credit.readable == true, credit.ledger == true else { return nil }
        return credit
    }

    /// Whether anything is worth a card: off answers draw none.
    var shown: Bool { failed || credit?.enabled == true }
}

/// The words and figures the credit tables draw, kept pure for tests.
enum InsightsCreditWords {
    enum Dimension { case task, tool, project }

    /// "$18.42"; under a cent, "<$0.01"; zero, "$0.00".
    static func money(_ micros: UInt64) -> String {
        if micros > 0 && micros < 10_000 { return "<$0.01" }
        let dollars = Double(micros) / 1_000_000
        return dollars.formatted(.currency(code: "USD").precision(.fractionLength(2)))
    }

    static func label(_ key: String?, _ dimension: Dimension, copy: [String: String]) -> String {
        func text(_ key: String) -> String { copy[key] ?? "" }
        switch dimension {
        case .task:
            // The rules' "no_edits" is the shared label's Other.
            let word = key == "no_edits" ? "other" : (key ?? "other")
            let label = text("analytics_task_" + word)
            return label.isEmpty ? text("analytics_task_other") : label
        case .tool:
            guard let key else { return text("analytics_unavailable") }
            let label = text("analytics_credit_tool_" + key)
            return label.isEmpty ? key : label
        case .project:
            return key ?? text("analytics_credit_other_project")
        }
    }

    /// The groups of one dimension, with the task groups merged where two
    /// wire words share a label (no_edits and other).
    static func rows(_ groups: [DaemonData.InsightsCredit.CreditGroup], _ dimension: Dimension,
                     copy: [String: String]) -> [(label: String, group: DaemonData.InsightsCredit.CreditGroup)] {
        var order: [String] = []
        var merged: [String: DaemonData.InsightsCredit.CreditGroup] = [:]
        for group in groups {
            let name = label(group.key, dimension, copy: copy)
            if let held = merged[name] {
                merged[name] = .init(key: held.key, usdMicros: held.usdMicros + group.usdMicros,
                                     traces: held.traces + group.traces,
                                     unknownTraces: held.unknownTraces + group.unknownTraces)
            } else {
                order.append(name)
                merged[name] = group
            }
        }
        return order.compactMap { name in merged[name].map { (name, $0) } }
            .sorted { $0.group.usdMicros > $1.group.usdMicros }
    }
}

/// One dimension's table: label, spent, traces, and a share bar.
struct InsightsCreditTable: View {
    let title: String
    let header: String
    let rows: [(label: String, group: DaemonData.InsightsCredit.CreditGroup)]
    let total: UInt64
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Text(title).insightsHeading()
            Grid(alignment: .leading, horizontalSpacing: GlassTokens.Space.s5, verticalSpacing: GlassTokens.Space.s3) {
                GridRow {
                    Text(header).insightsCaption()
                    Text(text("analytics_credit_spent")).insightsCaption().gridColumnAlignment(.trailing)
                    Text(text("analytics_credit_traces")).insightsCaption().gridColumnAlignment(.trailing)
                    Color.clear.frame(width: 1, height: 1)
                }
                Divider()
                ForEach(Array(rows.enumerated()), id: \.offset) { _, row in
                    GridRow {
                        Text(row.label).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                            .lineLimit(1).truncationMode(.middle)
                        Text(InsightsCreditWords.money(row.group.usdMicros)).monospacedDigit()
                        Text(String(row.group.traces)).monospacedDigit()
                        InsightsShareBar(permille: total == 0 ? nil : row.group.usdMicros * 1_000 / total)
                            .frame(minWidth: 60)
                    }
                    .glassType(GlassTokens.TypeScale.label)
                    .accessibilityElement(children: .combine)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// Overview's card: the total, the top tasks, and a way into Details.
struct InsightsCreditCard: View {
    let model: InsightsCreditModel
    let copy: [String: String]
    var showDetails: (() -> Void)?

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        if model.shown {
            VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                InsightsCreditHeader(model: model, copy: copy)
                if let credit = model.readable, let total = credit.total, total.traces > 0 {
                    InsightsCreditTable(
                        title: text("analytics_credit_by_task"), header: text("analytics_credit_task"),
                        rows: Array(InsightsCreditWords.rows(credit.byTask ?? [], .task, copy: copy).prefix(4)),
                        total: total.usdMicros, copy: copy)
                    .insightsCardBody()
                    if let showDetails {
                        Button(text("analytics_credit_details")) { showDetails() }
                            .buttonStyle(GlassButtonStyle(.link))
                    }
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSection()
        }
    }
}

/// The title, the window, the total and the lines that qualify it.
struct InsightsCreditHeader: View {
    let model: InsightsCreditModel
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(text("analytics_credit_title")).insightsTitle()
            if let credit = model.readable, let total = credit.total {
                let days = String(credit.windowDays ?? 30)
                Text(InsightsOverviewWords.fill(text("analytics_credit_window"), ["n": days])).insightsCaption()
                if total.traces == 0 {
                    Text(InsightsOverviewWords.fill(text("analytics_credit_none"), ["n": days])).insightsNote()
                } else {
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s8) {
                        VStack(alignment: .leading, spacing: 0) {
                            Text(text("analytics_credit_spent")).insightsEyebrow()
                            Text(InsightsCreditWords.money(total.usdMicros)).insightsFigure()
                        }
                        VStack(alignment: .leading, spacing: 0) {
                            Text(text("analytics_credit_traces")).insightsEyebrow()
                            Text(String(total.traces)).insightsFigure()
                        }
                    }
                    if total.unknownTraces > 0 {
                        Text(InsightsOverviewWords.fill(text("analytics_credit_unknown"),
                                                        ["n": String(total.unknownTraces)])).insightsCaption()
                    }
                }
            } else {
                Text(text("analytics_credit_unavailable")).insightsCaption()
            }
        }
    }
}

/// Details: where credit went, by task, by tool and by project (product
/// owner, 2026-10-28: in place of Patterns).
struct InsightsDetailsTab: View {
    let model: InsightsCreditModel
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
                VStack(alignment: .leading, spacing: InsightsLayout.headingGap) {
                    InsightsCreditHeader(model: model, copy: copy)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .insightsSection()
                if let credit = model.readable, let total = credit.total, total.traces > 0 {
                    ForEach(Self.tables(credit, copy: copy), id: \.title) { table in
                        InsightsCreditTable(title: table.title, header: table.header, rows: table.rows,
                                            total: total.usdMicros, copy: copy)
                        .insightsCardBody()
                        .insightsSection()
                    }
                }
            }
            .padding(.vertical, GlassTokens.Space.s7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSurface()
        }
    }

    struct Table {
        let title: String
        let header: String
        let rows: [(label: String, group: DaemonData.InsightsCredit.CreditGroup)]
    }

    static func tables(_ credit: DaemonData.InsightsCredit, copy: [String: String]) -> [Table] {
        func text(_ key: String) -> String { copy[key] ?? "" }
        return [
            Table(title: text("analytics_credit_by_task"), header: text("analytics_credit_task"),
                  rows: InsightsCreditWords.rows(credit.byTask ?? [], .task, copy: copy)),
            Table(title: text("analytics_credit_by_tool"), header: text("analytics_credit_tool"),
                  rows: InsightsCreditWords.rows(credit.byTool ?? [], .tool, copy: copy)),
            Table(title: text("analytics_credit_by_project"), header: text("analytics_credit_project"),
                  rows: InsightsCreditWords.rows(credit.byFolder ?? [], .project, copy: copy)),
        ]
    }
}
