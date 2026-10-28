import SwiftUI
import TCDesign
import TCShellCore

/// Where NEAR AI credit went (product owner, 2026-10-28, CodexBar-style),
/// from `insights_week`'s `near_ai_credit`: the NEAR AI ledger's own
/// charge for calls from this Mac, never a list price. Overview shows the
/// week's top tasks (`InsightsCreditCard`); Details breaks the week down by
/// task, tool or project folder (`InsightsCreditTable`). Folder names come
/// from this Mac's own folder list and stay on it (owner decision
/// 2026-10-28, over D7 for this view). A figure with a call that carried no
/// charge is the core's dash, never `$0.00`.
enum InsightsCreditWords {
    enum Dimension: Hashable { case task, tool, folder }

    struct Line: Equatable, Identifiable {
        let id: String
        let label: String
        let amount: String
        let traces: String
    }

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }

    /// Integer USD micros as dollars, as the Models card shows a charge;
    /// `nil` (unknown) is the dash.
    static func amount(_ micros: Int64?, copy: [String: String]) -> String {
        guard let micros, micros >= 0 else { return text("analytics_unavailable", copy) }
        return InsightsModelsWords.money(UInt64(micros))
    }

    /// A group's name: the task's word, the tool's, or the folder's own
    /// label; the unknown group in the words each list already uses.
    static func label(_ key: String?, dimension: Dimension, copy: [String: String]) -> String {
        switch dimension {
        case .task:
            guard let key else { return text("analytics_unknown_label", copy) }
            return text("analytics_task_" + key, copy)
        case .tool:
            guard let key else { return text("analytics_unknown_label", copy) }
            return text("analytics_credit_tool_" + key, copy)
        case .folder:
            return key ?? text("analytics_folder_unknown", copy)
        }
    }

    static func lines(_ groups: [DaemonData.InsightsCreditGroup], dimension: Dimension,
                      copy: [String: String]) -> [Line] {
        groups.map { group in
            Line(id: group.key ?? "\u{0}unknown", label: label(group.key, dimension: dimension, copy: copy),
                 amount: amount(group.usdMicros, copy: copy), traces: String(group.sessions))
        }
    }

    /// The week's tasks by credit, largest first as the daemon orders them.
    static func topTasks(_ credit: DaemonData.InsightsNearAiCredit, copy: [String: String],
                         limit: Int = 5) -> [Line] {
        Array(lines(credit.week.byTask ?? [], dimension: .task, copy: copy).prefix(limit))
    }

    /// Said whenever a call in the window carried no charge.
    static func unknownNote(_ window: DaemonData.InsightsCreditWindow, copy: [String: String]) -> String? {
        window.callsWithoutCharge > 0 ? text("analytics_credit_unknown_note", copy) : nil
    }

    /// No NEAR AI call in the window: a sentence, never a list of zeros.
    static func isEmpty(_ window: DaemonData.InsightsCreditWindow) -> Bool { window.calls == 0 }
}

/// Overview's "Where credit went": the week's top tasks by NEAR AI credit.
struct InsightsCreditCard: View {
    let credit: DaemonData.InsightsNearAiCredit
    let copy: [String: String]

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            InsightsSectionTitle(text("analytics_credit_title"), info: [text("analytics_credit_scope")])
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                if InsightsCreditWords.isEmpty(credit.week) {
                    Text(text("analytics_credit_empty")).insightsNote()
                } else {
                    InsightsCreditRows(lines: InsightsCreditWords.topTasks(credit, copy: copy), traces: false)
                    if let note = InsightsCreditWords.unknownNote(credit.week, copy: copy) {
                        Text(note).insightsCaption()
                    }
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }
}

/// Details' drill-down: the week's NEAR AI credit by task, tool or project
/// folder, with the traces each came from.
struct InsightsCreditTable: View {
    let credit: DaemonData.InsightsNearAiCredit
    let copy: [String: String]
    @State private var dimension: InsightsCreditWords.Dimension = .task

    private func text(_ key: String) -> String { copy[key] ?? "" }

    private var groups: [DaemonData.InsightsCreditGroup] {
        switch dimension {
        case .task: credit.week.byTask ?? []
        case .tool: credit.week.byTool ?? []
        case .folder: credit.week.byFolder ?? []
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            InsightsSectionTitle(text("analytics_credit_title"), info: [text("analytics_credit_scope")])
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                GlassSegmentedTabs(text("analytics_credit_title"), selection: $dimension, segments: [
                    GlassSegment(text("analytics_credit_by_task"), value: InsightsCreditWords.Dimension.task),
                    GlassSegment(text("analytics_card_by_tool"), value: InsightsCreditWords.Dimension.tool),
                    GlassSegment(text("analytics_card_by_project"), value: InsightsCreditWords.Dimension.folder),
                ])
                if InsightsCreditWords.isEmpty(credit.week) {
                    Text(text("analytics_credit_empty")).insightsNote()
                } else {
                    HStack {
                        Spacer(minLength: GlassTokens.Space.s4)
                        Text(text("analytics_credit_spent")).insightsCaption()
                        Text(text("analytics_credit_traces")).insightsCaption()
                            .frame(minWidth: 48, alignment: .trailing)
                    }
                    InsightsCreditRows(lines: InsightsCreditWords.lines(groups, dimension: dimension, copy: copy),
                                       traces: true)
                    if let note = InsightsCreditWords.unknownNote(credit.week, copy: copy) {
                        Text(note).insightsCaption()
                    }
                    if dimension == .folder {
                        Text(text("analytics_credit_folders_local")).insightsCaption()
                    }
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }
}

/// The rows both views draw: a name, its amount, and on Details its traces.
private struct InsightsCreditRows: View {
    let lines: [InsightsCreditWords.Line]
    let traces: Bool

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(lines.enumerated()), id: \.element.id) { index, line in
                GlassTableRow(first: index == 0) {
                    HStack(alignment: .firstTextBaseline) {
                        Text(line.label)
                        Spacer(minLength: GlassTokens.Space.s4)
                        Text(line.amount).insightsMono()
                        if traces {
                            Text(line.traces).insightsMono().frame(minWidth: 48, alignment: .trailing)
                        }
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        }
    }
}
