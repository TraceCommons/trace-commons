import Charts
import SwiftUI
import TCDesign
import TCShellCore

/// Submitted (product owner, 2026-10-29), in place of Benchmarks: every
/// trace the person sent, whatever became of it, from `insights_submitted`.
/// There is no minimum: one trace fills every section with its own
/// figures, and a section is left out only when it would be empty.
struct InsightsSubmittedTab: View {
    let store: InsightsSubmittedStore
    let copy: [String: String]
    /// History's words for statuses and for why a trace earned nothing.
    let labels: PublicRunCopy?
    /// "Add Train consent in History"; `nil` draws no button.
    var openHistory: (() -> Void)?
    /// The get-started card over this tab drawn from the sample (product
    /// owner, 2026-10-29); `nil` for the person's own traces.
    var getStarted: InsightsGetStarted?

    @State private var showsAllTraces = false

    private func text(_ key: String) -> String { copy[key] ?? "" }

    var body: some View {
        InsightsTabPage(getStarted: getStarted) {
            header
            if let answer = store.answer, answer.isAnswered, let totals = answer.totals {
                if totals.submitted == 0 {
                    Text(text("analytics_sub_none_in_period")).insightsNote()
                } else {
                    content(answer)
                }
            } else if store.failed || store.answer != nil {
                Text(text("analytics_unavailable")).insightsNote()
            }
            if store.busy && store.answer == nil { GlassSpinner() }
        }
        .task(id: store.weeks) { await store.load() }
    }

    @ViewBuilder private func content(_ answer: DaemonData.InsightsSubmitted) -> some View {
        InsightsCardFlow(minimum: 170) {
            ForEach(InsightsSubmittedWords.tiles(answer, copy: copy)) { tile in
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(tile.label).insightsEyebrow()
                    Text(tile.value).insightsFigure().lineLimit(1).minimumScaleFactor(0.6)
                    Text(tile.note).insightsCaption().lineLimit(2)
                }
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                .insightsSection()
                .accessibilityElement(children: .combine)
            }
        }
        let reasons = InsightsSubmittedWords.reasons(answer, labels: labels)
        if !reasons.isEmpty { reasonsCard(reasons) }
        if !InsightsSubmittedWords.models(answer).isEmpty { modelsCard(answer) }
        let tasks = InsightsSubmittedWords.taskRows(answer)
        let code = InsightsSubmittedWords.hasCode(answer.code) ? answer.code : nil
        if !tasks.isEmpty || code != nil {
            InsightsCardFlow(minimum: 400) {
                if !tasks.isEmpty { tasksCard(tasks) }
                if let code { codeCard(code) }
            }
        }
        let weeks = answer.byWeek ?? []
        let projects = (answer.byProject ?? []).filter { $0.submitted > 0 }
        if !weeks.isEmpty || !projects.isEmpty {
            InsightsCardFlow(minimum: 400) {
                if !weeks.isEmpty { weeksCard(weeks) }
                if !projects.isEmpty { projectsCard(projects) }
            }
        }
        if let traces = answer.traces, !traces.isEmpty {
            tracesCard(traces, total: answer.tracesTotal ?? traces.count)
        }
    }

    // MARK: Header

    @ViewBuilder private var header: some View {
        @Bindable var store = store
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                if getStarted == nil, let submitted = store.answer?.totals?.submitted, submitted > 0 {
                    Text(InsightsSubmittedWords.intro(submitted, copy: copy)).insightsHeading()
                } else {
                    Text(text("analytics_tab_submitted")).insightsHeading()
                }
                Spacer(minLength: 0)
                GlassSelect(text("analytics_sub_period"), selection: $store.weeks,
                            options: InsightsSubmittedWords.periods(copy: copy).map {
                                GlassPickerOption($0.label, value: $0.weeks)
                            })
                .fixedSize()
            }
            Text(text("analytics_sub_scope")).insightsCaption()
                .fixedSize(horizontal: false, vertical: true)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    // MARK: Sections

    private func card<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(title).insightsHeading().accessibilityAddTraits(.isHeader)
            content()
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .insightsSection()
    }

    private func reasonsCard(_ reasons: [InsightsSubmittedWords.Reason]) -> some View {
        card(text("analytics_sub_reasons_title")) {
            HistoryFilterFlow(spacing: GlassTokens.Space.s3, lineSpacing: GlassTokens.Space.s3) {
                ForEach(reasons) { reason in
                    HStack(spacing: GlassTokens.Space.s3) {
                        Text(reason.label).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                            .foregroundStyle(GlassColor.textPrimary)
                        Text(InsightsSubmittedWords.traces(reason.count, copy: copy)).insightsCaption()
                        if reason.needsConsent, let openHistory {
                            Button(text("analytics_sub_reasons_open_history")) { openHistory() }
                                .buttonStyle(GlassButtonStyle(.link))
                        }
                    }
                    .padding(.horizontal, GlassTokens.Space.s4)
                    .padding(.vertical, GlassTokens.Space.s2)
                    .background(Capsule().fill(GlassTokens.Color.tintNeutral.color))
                    .accessibilityElement(children: .combine)
                }
            }
        }
    }

    private func modelsCard(_ answer: DaemonData.InsightsSubmitted) -> some View {
        let models = InsightsSubmittedWords.models(answer)
        let tools = Set(models.map(\.tool)).count > 1
        let leftOut = InsightsSubmittedWords.modelsLeftOut(answer)
        return card(text("analytics_sub_models_title")) {
            Grid(alignment: .leading, horizontalSpacing: GlassTokens.Space.s6, verticalSpacing: GlassTokens.Space.s3) {
                GridRow {
                    Text(text("analytics_sub_col_model")).insightsCaption()
                    ForEach(["analytics_sub_col_submitted", "analytics_sub_col_accepted", "analytics_sub_col_credit",
                             "analytics_sub_col_spend", "analytics_sub_col_kept"], id: \.self) { key in
                        Text(text(key)).insightsCaption().gridColumnAlignment(.trailing)
                    }
                }
                Divider()
                ForEach(models) { model in
                    GridRow(alignment: .firstTextBaseline) {
                        VStack(alignment: .leading, spacing: 0) {
                            Text(model.label).glassType(GlassTokens.TypeScale.label.weight(.semibold)).lineLimit(1)
                            if tools { Text(InsightsWorkWords.toolName(model.tool, copy: copy)).insightsCaption() }
                        }
                        Text(InsightsSubmittedWords.count(model.tally.submitted)).insightsMono()
                        Text(InsightsSubmittedWords.count(model.tally.accepted)).insightsMono()
                        Text(InsightsSubmittedWords.points(model.tally.creditFinal)).insightsMono()
                        Text(InsightsSubmittedWords.spend(model.spend, copy: copy)).insightsMono()
                        Text(InsightsSubmittedWords.kept(model.edits, copy: copy)).insightsMono()
                    }
                    .accessibilityElement(children: .combine)
                }
            }
            .insightsCardBody()
            if leftOut > 0 {
                Text(InsightsOverviewWords.fill(text("analytics_sub_models_left_out"), ["n": String(leftOut)]))
                    .insightsCaption()
            }
        }
    }

    private func tasksCard(_ rows: [DaemonData.InsightsSubmittedTally]) -> some View {
        let most = max(1, rows.map(\.submitted).max() ?? 1)
        return card(text("analytics_sub_tasks_title")) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                ForEach(rows, id: \.key) { row in
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(InsightsSubmittedWords.taskName(row.key, copy: copy))
                                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                            Spacer(minLength: GlassTokens.Space.s3)
                            Text(InsightsSubmittedWords.points(row.creditFinal)).insightsMono()
                        }
                        bar(submitted: row.submitted, accepted: row.accepted, most: most)
                        Text(InsightsSubmittedWords.acceptedOf(row, copy: copy)).insightsCaption()
                    }
                    .accessibilityElement(children: .combine)
                }
            }
            .insightsCardBody()
        }
    }

    /// Sent as the track's length, accepted filled over it.
    private func bar(submitted: Int, accepted: Int, most: Int) -> some View {
        GeometryReader { proxy in
            let whole = proxy.size.width * CGFloat(submitted) / CGFloat(most)
            ZStack(alignment: .leading) {
                Capsule().fill(GlassTokens.Color.chartGrid.color).frame(width: max(6, whole))
                if accepted > 0 {
                    Capsule().fill(GlassTokens.Color.chartSeries1.color)
                        .frame(width: max(6, whole * CGFloat(accepted) / CGFloat(max(1, submitted))))
                }
            }
        }
        .frame(height: 8)
    }

    private func codeCard(_ code: DaemonData.InsightsSubmittedCode) -> some View {
        let kinds = InsightsSubmittedWords.fileKinds(code.files, copy: copy)
        return card(text("analytics_sub_code_title")) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
                if !kinds.isEmpty {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        HStack(alignment: .firstTextBaseline) {
                            Text(text("analytics_sub_files")).insightsEyebrow()
                            Spacer(minLength: GlassTokens.Space.s3)
                            Text(InsightsSubmittedWords.count(code.files.total)).insightsMono()
                        }
                        GeometryReader { proxy in
                            let gaps = CGFloat(kinds.count - 1) * 2
                            HStack(spacing: 2) {
                                ForEach(kinds) { kind in
                                    Rectangle().fill(kind.color.color)
                                        .frame(width: max(4, (proxy.size.width - gaps) * CGFloat(kind.count)
                                                / CGFloat(max(1, code.files.total))))
                                }
                            }
                            .clipShape(Capsule())
                        }
                        .frame(height: 10)
                        HistoryFilterFlow(spacing: GlassTokens.Space.s5, lineSpacing: GlassTokens.Space.s2) {
                            ForEach(kinds) { kind in
                                HStack(spacing: GlassTokens.Space.s2) {
                                    Circle().fill(kind.color.color).frame(width: 8, height: 8)
                                    Text(kind.label).insightsCaption()
                                    Text(InsightsSubmittedWords.count(kind.count)).insightsMono()
                                }
                                .accessibilityElement(children: .combine)
                            }
                        }
                    }
                }
                if code.edits.editsKnown > 0 {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        Text(text("analytics_sub_edits")).insightsEyebrow()
                        HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
                            figure("analytics_sub_edits_kept", code.edits.editsKeptKnown,
                                   ink: GlassTokens.Color.statusOnText.color)
                            figure("analytics_sub_edits_undone", code.edits.editsUndoneKnown)
                            figure("analytics_sub_edits_denied", code.edits.editsDeniedKnown)
                            Spacer(minLength: 0)
                        }
                    }
                }
            }
            .insightsCardBody()
        }
    }

    private func figure(_ key: String, _ value: Int64, ink: Color = GlassColor.textPrimary) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            Text(value.formatted(.number)).glassType(GlassTokens.TypeScale.bodyStrong).monospacedDigit()
                .foregroundStyle(ink)
            Text(text(key)).insightsCaption()
        }
        .accessibilityElement(children: .combine)
    }

    private func weeksCard(_ weeks: [DaemonData.InsightsSubmittedTally]) -> some View {
        card(text("analytics_sub_weeks_title")) {
            Group {
                // A few weeks: a slim bar each, its Monday under it. More:
                // a time axis, labelled a month apart.
                if weeks.count <= 8 { fewWeeks(weeks) } else { manyWeeks(weeks) }
            }
            .chartForegroundStyleScale([text("analytics_sub_col_submitted"): GlassTokens.Color.chartGrid.color,
                                        text("analytics_sub_col_accepted"): GlassTokens.Color.chartSeries1.color])
            .chartYAxis {
                AxisMarks(position: .leading, values: .automatic(desiredCount: 3)) { _ in
                    AxisGridLine()
                    AxisValueLabel().font(.system(size: 10))
                }
            }
            .chartLegend(position: .top, alignment: .leading)
            .frame(height: 150)
            .insightsPlot()
        }
    }

    private func fewWeeks(_ weeks: [DaemonData.InsightsSubmittedTally]) -> some View {
        let sent = text("analytics_sub_col_submitted"), accepted = text("analytics_sub_col_accepted")
        return Chart {
            ForEach(Array(weeks.enumerated()), id: \.offset) { _, week in
                let label = InsightsSubmittedWords.weekLabel(week.weekStart)
                BarMark(x: .value("week", label), y: .value("traces", week.submitted), width: .fixed(28),
                        stacking: .unstacked)
                    .foregroundStyle(by: .value("kind", sent))
                BarMark(x: .value("week", label), y: .value("traces", week.accepted), width: .fixed(28),
                        stacking: .unstacked)
                    .foregroundStyle(by: .value("kind", accepted))
            }
        }
        .chartXAxis {
            AxisMarks { _ in AxisValueLabel().font(.system(size: 10)) }
        }
    }

    private func manyWeeks(_ weeks: [DaemonData.InsightsSubmittedTally]) -> some View {
        let sent = text("analytics_sub_col_submitted"), accepted = text("analytics_sub_col_accepted")
        return Chart {
            ForEach(Array(weeks.enumerated()), id: \.offset) { _, week in
                if let monday = InsightsSubmittedWords.weekDate(week.weekStart) {
                    BarMark(x: .value("week", monday, unit: .weekOfYear), y: .value("traces", week.submitted),
                            width: .ratio(0.7), stacking: .unstacked)
                        .foregroundStyle(by: .value("kind", sent))
                    BarMark(x: .value("week", monday, unit: .weekOfYear), y: .value("traces", week.accepted),
                            width: .ratio(0.7), stacking: .unstacked)
                        .foregroundStyle(by: .value("kind", accepted))
                }
            }
        }
        .chartXAxis {
            AxisMarks(values: .stride(by: .month)) { _ in
                AxisValueLabel(format: .dateTime.month(.abbreviated).day()).font(.system(size: 10))
            }
        }
        // The daemon's weeks start on Monday; bin them so.
        .environment(\.calendar, InsightsSubmittedWords.mondayCalendar)
    }

    private func projectsCard(_ rows: [DaemonData.InsightsSubmittedTally]) -> some View {
        card(text("analytics_sub_projects_title")) {
            Grid(alignment: .leading, horizontalSpacing: GlassTokens.Space.s6, verticalSpacing: GlassTokens.Space.s3) {
                GridRow {
                    Text(text("analytics_sub_col_project")).insightsCaption()
                    ForEach(["analytics_sub_col_submitted", "analytics_sub_col_accepted", "analytics_sub_col_credit"],
                            id: \.self) { key in
                        Text(text(key)).insightsCaption().gridColumnAlignment(.trailing)
                    }
                }
                Divider()
                ForEach(rows, id: \.label) { row in
                    GridRow(alignment: .firstTextBaseline) {
                        Text(row.label ?? text("analytics_unavailable")).lineLimit(1).truncationMode(.middle)
                        Text(InsightsSubmittedWords.count(row.submitted)).insightsMono()
                        Text(InsightsSubmittedWords.count(row.accepted)).insightsMono()
                        Text(InsightsSubmittedWords.points(row.creditFinal)).insightsMono()
                    }
                    .accessibilityElement(children: .combine)
                }
            }
            .insightsCardBody()
        }
    }

    private func tracesCard(_ traces: [DaemonData.InsightsSubmittedTrace], total: Int) -> some View {
        let shown = showsAllTraces ? traces : Array(traces.prefix(InsightsSubmittedWords.visibleTraces))
        return card(text("analytics_sub_list_title")) {
            VStack(spacing: 0) {
                ForEach(Array(shown.enumerated()), id: \.element.id) { index, trace in
                    GlassTableRow(first: index == 0) { traceRow(trace) }
                }
            }
            HStack(spacing: GlassTokens.Space.s4) {
                if traces.count > InsightsSubmittedWords.visibleTraces {
                    Button(showsAllTraces ? text("analytics_sub_show_fewer")
                           : InsightsOverviewWords.fill(text("analytics_sub_show_all"),
                                                        ["n": InsightsSubmittedWords.count(traces.count)])) {
                        showsAllTraces.toggle()
                    }
                    .buttonStyle(GlassButtonStyle(.link))
                }
                Spacer(minLength: 0)
                if total > traces.count {
                    Text(InsightsOverviewWords.fill(text("analytics_sub_list_capped"), [
                        "k": InsightsSubmittedWords.count(traces.count), "n": InsightsSubmittedWords.count(total),
                    ])).insightsCaption()
                }
            }
        }
    }

    private func traceRow(_ trace: DaemonData.InsightsSubmittedTrace) -> some View {
        let details = [trace.model, trace.task.map { InsightsSubmittedWords.taskName($0, copy: copy) }]
            .compactMap { $0 }.joined(separator: " · ")
        return HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Text(InsightsSubmittedWords.day(trace.submittedAt)).insightsMono()
                .frame(width: 92, alignment: .leading)
            VStack(alignment: .leading, spacing: 0) {
                Text(trace.projectLabel).lineLimit(1).truncationMode(.middle)
                if !details.isEmpty { Text(details).insightsCaption().lineLimit(1) }
            }
            Spacer(minLength: GlassTokens.Space.s3)
            if let status = InsightsSubmittedWords.status(trace.status, labels: labels) {
                GlassTag(status.label, tone: status.tone)
            }
            Text(InsightsSubmittedWords.credit(trace, copy: copy)).insightsMono()
                .frame(minWidth: 72, alignment: .trailing)
        }
        .accessibilityElement(children: .combine)
    }
}
