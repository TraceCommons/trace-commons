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
    enum Dimension: Hashable { case folder, task, tool, model }

    struct Line: Equatable, Identifiable {
        let id: String
        let label: String
        let amount: String
        let traces: String
    }

    private static func text(_ key: String, _ copy: [String: String]) -> String { copy[key] ?? "" }

    /// Integer USD micros as dollars; `nil` (unknown) is the dash.
    static func amount(_ micros: Int64?, copy: [String: String]) -> String {
        guard let micros, micros >= 0 else { return text("analytics_unavailable", copy) }
        return money(UInt64(micros))
    }

    /// Integer USD micros as dollars, as the app's other prices are
    /// (`InferenceViews.money`): two to four decimals, so "$0.50", "$1.00"
    /// and "$0.0412". Under a hundredth of a cent, one significant digit,
    /// so a charge is never shown as zero.
    static func money(_ micros: UInt64) -> String {
        let dollars = Decimal(micros) / 1_000_000
        let style = Decimal.FormatStyle.Currency(code: "USD")
        return micros > 0 && micros < 100
            ? dollars.formatted(style.precision(.significantDigits(1)))
            : dollars.formatted(style.precision(.fractionLength(2...4)))
    }

    /// A token figure with its unit, "1.2M tokens"; the dash when unknown.
    static func tokens(_ tokens: Int64?, copy: [String: String]) -> String {
        guard let tokens, tokens >= 0 else { return text("analytics_unavailable", copy) }
        return InsightsOverviewWords.fill(text("analytics_usage_tokens", copy),
                                          ["t": InsightsOverviewWords.figure(UInt64(tokens), copy: copy)])
    }

    /// A figure some of whose calls carried none (product owner,
    /// 2026-10-28): the exact one when known; else the known part as a
    /// minimum, "at least $5.21"; the dash only when nothing is known.
    static func atLeast(exact: Int64?, known: Int64?, partial: Bool?, copy: [String: String],
                        format: (Int64?) -> String) -> String {
        if let exact { return format(exact) }
        guard let known, partial == true else { return text("analytics_unavailable", copy) }
        return InsightsOverviewWords.fill(text("analytics_usage_at_least", copy), ["x": format(known)])
    }

    /// A window's or group's spend, "at least" when partly known.
    static func spend(_ exact: Int64?, known: Int64?, partial: Bool?, copy: [String: String]) -> String {
        atLeast(exact: exact, known: known, partial: partial, copy: copy) { amount($0, copy: copy) }
    }

    /// A window's or group's tokens, "at least" when partly known.
    static func tokens(_ exact: Int64?, known: Int64?, partial: Bool?, copy: [String: String]) -> String {
        atLeast(exact: exact, known: known, partial: partial, copy: copy) { tokens($0, copy: copy) }
    }

    /// A NEAR AI window's spend and tokens together, "$4.12 · 1.2M tokens".
    static func spendAndTokens(_ window: DaemonData.InsightsCreditWindow, copy: [String: String]) -> String {
        spend(window.usdMicros, known: window.usdMicrosKnown, partial: window.usdPartial, copy: copy) + " · "
            + tokens(window.tokens, known: window.tokensKnown, partial: window.tokensPartial, copy: copy)
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
        case .model:
            return key ?? text("analytics_unknown_label", copy)
        }
    }

    static func lines(_ groups: [DaemonData.InsightsCreditGroup], dimension: Dimension,
                      copy: [String: String]) -> [Line] {
        groups.map { group in
            Line(id: group.key ?? "\u{0}unknown", label: label(group.key, dimension: dimension, copy: copy),
                 amount: spend(group.usdMicros, known: group.usdMicrosKnown, partial: group.usdPartial, copy: copy),
                 traces: String(group.sessions))
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
        /// NEAR AI's tokens under its amount, "1.2M tokens"; `nil` with no
        /// NEAR AI call in the folder.
        let nearAiTokens: String?
        /// The known NEAR AI amount, for the order.
        let nearAiMicros: Int64?
        /// Claude and Codex at API prices (product owner, 2026-10-29);
        /// `nil` from a daemon that predates `work`.
        var spend: String? = nil
        var spendMicros: Int64 = 0
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
        let work = Dictionary((week.work?.byFolder ?? []).map { ($0.key, $0) },
                              uniquingKeysWith: { first, _ in first })
        let folders = Set(usage.keys).union(spend.keys).union(work.keys)
        let unknown = copy["analytics_unavailable"] ?? ""
        let rows = folders.map { folder -> Row in
            let group = spend[folder]
            let totals = work[folder]
            return Row(id: folder ?? "\u{0}unknown",
                       label: InsightsCreditWords.label(folder, dimension: .folder, copy: copy),
                       claude: usage[folder]?.claude, codex: usage[folder]?.codex,
                       nearAi: group.map {
                           InsightsCreditWords.spend($0.usdMicros, known: $0.usdMicrosKnown, partial: $0.usdPartial,
                                                     copy: copy)
                       } ?? unknown,
                       nearAiTokens: group.map {
                           InsightsCreditWords.tokens($0.tokens, known: $0.tokensKnown, partial: $0.tokensPartial,
                                                      copy: copy)
                       },
                       nearAiMicros: group.flatMap { $0.usdMicros ?? $0.usdMicrosKnown },
                       spend: week.work.map { _ in InsightsWorkWords.spend(totals, copy: copy) },
                       spendMicros: totals?.usdMicrosKnown ?? 0)
        }
        func tokens(_ row: Row) -> Int64 { (row.claude?.tokens ?? 0) + (row.codex?.tokens ?? 0) }
        return rows.sorted { a, b in
            let (aUnknown, bUnknown) = (a.id.hasPrefix("\u{0}"), b.id.hasPrefix("\u{0}"))
            if aUnknown != bUnknown { return bUnknown }
            let (aSpend, bSpend) = (a.spendMicros + (a.nearAiMicros ?? 0), b.spendMicros + (b.nearAiMicros ?? 0))
            if aSpend != bSpend { return aSpend > bSpend }
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
                ] + (week.work == nil ? [] : [
                    GlassSegment(text("analytics_card_by_model"), value: InsightsCreditWords.Dimension.model),
                ]))
                switch dimension {
                case .folder: projects
                case .task, .tool: nearAi
                case .model: models
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
                    if week.work != nil {
                        Text(text("analytics_column_spend")).insightsCaption().gridColumnAlignment(.trailing)
                    }
                    Text(text("analytics_credit_tool_claude_code")).insightsCaption().gridColumnAlignment(.trailing)
                    Text(text("analytics_credit_tool_codex")).insightsCaption().gridColumnAlignment(.trailing)
                    Text(text("analytics_usage_near_ai")).insightsCaption().gridColumnAlignment(.trailing)
                }
                Divider()
                ForEach(rows) { row in
                    GridRow(alignment: .firstTextBaseline) {
                        Text(row.label).lineLimit(2)
                        if let spend = row.spend { Text(spend).insightsMono() }
                        usage(row.claude)
                        usage(row.codex)
                        VStack(alignment: .trailing, spacing: 0) {
                            Text(row.nearAi).insightsMono()
                            if let tokens = row.nearAiTokens { Text(tokens).insightsCaption() }
                        }
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

    /// Claude and Codex spend by model at API prices, with its tokens.
    @ViewBuilder private var models: some View {
        if let work = week.work, !work.byModel.isEmpty {
            VStack(spacing: 0) {
                ForEach(Array(work.byModel.enumerated()), id: \.offset) { index, model in
                    GlassTableRow(first: index == 0) {
                        HStack(alignment: .firstTextBaseline) {
                            VStack(alignment: .leading, spacing: 0) {
                                Text(model.key)
                                Text(InsightsWorkWords.toolName(model.tool, copy: copy)).insightsCaption()
                            }
                            Spacer(minLength: GlassTokens.Space.s4)
                            Text(InsightsCreditWords.tokens(model.tokens, copy: copy)).insightsCaption()
                            Text(InsightsWorkWords.dollars(model.usdMicrosKnown))
                                .insightsMono().frame(minWidth: 72, alignment: .trailing)
                        }
                        .accessibilityElement(children: .combine)
                    }
                }
            }
            if work.week.cacheSavingsUsdMicrosKnown > 0 {
                InsightsUsageRow(label: text("analytics_cache_savings"),
                                 value: InsightsOverviewWords.fill(text("analytics_cache_savings_value"), [
                                     "x": InsightsCreditWords.amount(work.week.cacheSavingsUsdMicrosKnown, copy: copy),
                                 ]))
            }
        } else {
            Text(text("analytics_unavailable")).insightsNote()
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
