import SwiftUI
import TCDesign
import TCShellCore

/// Details' drill-down (product owner, 2026-10-28): the week on screen by
/// project folder, with Claude Code and Codex usage (traces and tokens,
/// from the counter pass's rows) beside NEAR AI spend (the NEAR AI
/// ledger's own charge, from `near_ai_credit`, never a list price); and
/// NEAR AI spend by task or by tool. Folder names come from this Mac's own
/// folder list and stay on it (owner decision 2026-10-28, over D7 for this
/// view). An unknown figure is the core's dash, never zero.
enum InsightsCreditWords {
    enum Dimension: Hashable { case folder, task, tool }

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

    /// No NEAR AI call in the window: a sentence, never a list of zeros.
    static func isEmpty(_ window: DaemonData.InsightsCreditWindow) -> Bool { window.calls == 0 }
}

/// By project: one row per folder, Claude Code and Codex usage beside
/// NEAR AI spend. Kept pure for tests.
enum InsightsProjectRows {
    struct Usage: Equatable {
        /// Traces counted this week.
        let traces: Int
        /// Their tokens; `nil` when any trace's count is unknown.
        let tokens: Int64?
    }

    struct Row: Equatable, Identifiable {
        let id: String
        let label: String
        let claude: Usage?
        let codex: Usage?
        /// The NEAR AI amount, or the dash when there was no call or a call
        /// had no charge.
        let nearAi: String
        /// The known NEAR AI amount, for the order.
        let nearAiMicros: Int64?
    }

    /// Every folder with a trace counted this week or NEAR AI spend this
    /// week. Largest NEAR AI spend first, then most tokens; the unknown
    /// folder last.
    static func rows(_ week: DaemonData.InsightsWeek, copy: [String: String]) -> [Row] {
        var usage: [String?: (claude: Usage?, codex: Usage?)] = [:]
        func add(_ current: Usage?, tokens: Int64?) -> Usage {
            let traces = (current?.traces ?? 0) + 1
            let known: Int64? = current == nil ? 0 : current?.tokens
            return Usage(traces: traces, tokens: known.flatMap { sum in tokens.map { sum + $0 } })
        }
        for session in week.rollup?.sessions ?? [] {
            var entry = usage[session.folder] ?? (nil, nil)
            switch session.source {
            case "claude_code": entry.claude = add(entry.claude, tokens: session.tokens)
            case "codex": entry.codex = add(entry.codex, tokens: session.tokens)
            default: continue
            }
            usage[session.folder] = entry
        }
        let spend = Dictionary((week.nearAiCredit?.week.byFolder ?? []).map { ($0.key, $0) },
                               uniquingKeysWith: { first, _ in first })
        let folders = Set(usage.keys).union(spend.keys)
        let unknown = copy["analytics_unavailable"] ?? ""
        let rows = folders.map { folder -> Row in
            let group = spend[folder]
            return Row(id: folder ?? "\u{0}unknown",
                       label: InsightsCreditWords.label(folder, dimension: .folder, copy: copy),
                       claude: usage[folder]?.claude, codex: usage[folder]?.codex,
                       nearAi: group.map { InsightsCreditWords.amount($0.usdMicros, copy: copy) } ?? unknown,
                       nearAiMicros: group?.usdMicros)
        }
        func tokens(_ row: Row) -> Int64 { (row.claude?.tokens ?? 0) + (row.codex?.tokens ?? 0) }
        return rows.sorted { a, b in
            let (aUnknown, bUnknown) = (a.id.hasPrefix("\u{0}"), b.id.hasPrefix("\u{0}"))
            if aUnknown != bUnknown { return bUnknown }
            if (a.nearAiMicros ?? -1) != (b.nearAiMicros ?? -1) { return (a.nearAiMicros ?? -1) > (b.nearAiMicros ?? -1) }
            if tokens(a) != tokens(b) { return tokens(a) > tokens(b) }
            return a.label < b.label
        }
    }

    /// A tool's cell: its tokens, or the dash.
    static func tokens(_ usage: Usage?, copy: [String: String]) -> String {
        guard let usage else { return copy["analytics_unavailable"] ?? "" }
        return InsightsOverviewWords.figure(usage.tokens.map { UInt64(max(0, $0)) }, copy: copy)
    }

    /// A tool's trace count under its tokens, "1 trace" or "{n} traces";
    /// `nil` without traces.
    static func traces(_ usage: Usage?, copy: [String: String]) -> String? {
        usage.map { usage in
            let key = usage.traces == 1 ? "analytics_projects_traces_one" : "analytics_projects_traces"
            return InsightsOverviewWords.fill(copy[key] ?? "", ["n": String(usage.traces)])
        }
    }
}

/// Details' drill-down: By project (the default, and what each Overview
/// usage card's "By project" opens), By task and By tool.
struct InsightsDetailsTable: View {
    let week: DaemonData.InsightsWeek
    let copy: [String: String]
    @Binding var dimension: InsightsCreditWords.Dimension

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            InsightsSectionTitle(text("analytics_credit_title"), info: [text("analytics_credit_scope")])
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                GlassSegmentedTabs(text("analytics_credit_title"), selection: $dimension, segments: [
                    GlassSegment(text("analytics_card_by_project"), value: InsightsCreditWords.Dimension.folder),
                    GlassSegment(text("analytics_credit_by_task"), value: InsightsCreditWords.Dimension.task),
                    GlassSegment(text("analytics_card_by_tool"), value: InsightsCreditWords.Dimension.tool),
                ])
                switch dimension {
                case .folder: projects
                case .task, .tool: nearAi
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// One table: a row per folder, Claude Code and Codex tokens with their
    /// traces, and NEAR AI spend.
    @ViewBuilder private var projects: some View {
        let rows = InsightsProjectRows.rows(week, copy: copy)
        if rows.isEmpty {
            Text(text("analytics_unavailable")).insightsNote()
        } else {
            Grid(alignment: .leading, horizontalSpacing: GlassTokens.Space.s4, verticalSpacing: GlassTokens.Space.s3) {
                GridRow {
                    Text(text("analytics_projects_project")).insightsCaption()
                    Text(text("analytics_credit_tool_claude_code")).insightsCaption().gridColumnAlignment(.trailing)
                    Text(text("analytics_credit_tool_codex")).insightsCaption().gridColumnAlignment(.trailing)
                    Text(text("analytics_usage_near_ai")).insightsCaption().gridColumnAlignment(.trailing)
                }
                Divider()
                ForEach(rows) { row in
                    GridRow(alignment: .firstTextBaseline) {
                        Text(row.label).lineLimit(2)
                        usage(row.claude)
                        usage(row.codex)
                        Text(row.nearAi).insightsMono()
                    }
                    .accessibilityElement(children: .combine)
                }
            }
        }
    }

    private func usage(_ usage: InsightsProjectRows.Usage?) -> some View {
        VStack(alignment: .trailing, spacing: 0) {
            Text(InsightsProjectRows.tokens(usage, copy: copy)).insightsMono()
            if let traces = InsightsProjectRows.traces(usage, copy: copy) {
                Text(traces).insightsCaption()
            }
        }
    }

    /// NEAR AI spend by task or by tool, with the traces it came from.
    @ViewBuilder private var nearAi: some View {
        if let credit = week.nearAiCredit, !InsightsCreditWords.isEmpty(credit.week) {
            let groups = (dimension == .task ? credit.week.byTask : credit.week.byTool) ?? []
            HStack {
                Spacer(minLength: GlassTokens.Space.s4)
                Text(text("analytics_credit_spent")).insightsCaption()
                Text(text("analytics_credit_traces")).insightsCaption()
                    .frame(minWidth: 48, alignment: .trailing)
            }
            VStack(spacing: 0) {
                ForEach(Array(InsightsCreditWords.lines(groups, dimension: dimension, copy: copy).enumerated()),
                        id: \.element.id) { index, line in
                    GlassTableRow(first: index == 0) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(line.label)
                            Spacer(minLength: GlassTokens.Space.s4)
                            Text(line.amount).insightsMono()
                            Text(line.traces).insightsMono().frame(minWidth: 48, alignment: .trailing)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
        } else {
            // No NEAR AI call this week: the dash, as an empty By project.
            Text(text("analytics_unavailable")).insightsNote()
        }
    }
}
