import Charts
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// Sessions (drill-in). While the daemon's watched-folder counting (feed T)
/// shows, a feed of every watched trace, newest first, drawn lazily and read
/// a page at a time as its last rows appear (owner request 2026-10-20): each
/// row its folder, time, harness, turns, tokens and a sparkline of tokens per
/// turn, and a row expands its drill beneath it (one at a time). Otherwise a
/// saved snapshot (feed S), picked as before; never both (owner decision
/// 2026-10-20). Every word is the core's analytics copy and every figure the
/// core's. In a drill, input sent each turn is stacked cache read, cache
/// write and uncached on the chart surface, with a dashed line at the
/// long-context threshold and the core's lettered markers drawn at their
/// turns. An unknown turn is a gap, never a zero bar. A Codex session draws
/// no chart and reads "not recorded" (owner decision D11, open). No what-if
/// card is drawn (owner decision D2, open), and no project name is shown
/// for a saved snapshot (owner decision D7, open).
///
/// Laid out for the main pane (direction B, 2026-10-15): the chart takes
/// the pane's width and the markers are listed under it. Hovering a turn,
/// or pressing a marker, reads that turn out in the legend.
struct InsightsSessionsTab: View {
    let model: InsightsSessionsModel
    /// Saved snapshots, newest first, for the session picker under feed S.
    /// Each is shown by its label, never its ID.
    let snapshots: [LocalInsight]
    let copy: [String: String]
    /// Why watched-folder counting is not showing (the window's week, or
    /// this tab's own list), as Overview says it; `nil` otherwise.
    var notice: String? = nil
    /// Analyze & save on the open watched row (Insights PM, 2026-10-21):
    /// the model that asks the daemon and shows the snapshot; `nil` draws
    /// no action (no daemon, or a custom store).
    var analysis: InsightsModel? = nil
    /// Opens Analyze once the saved snapshot is its detail.
    var openAnalysis: () -> Void = {}
    /// The turn being read; `nil` reads the whole session.
    @State private var readTurn: Int?
    /// The feed row the arrow keys are on (Full Keyboard Access).
    @State private var keyRow: String?
    /// The feed row at the top of the scroll: when the list is read again
    /// and replaced, the scroll stays on it rather than jumping.
    @State private var anchorRow: String?

    private func text(_ key: String) -> String { copy[key] ?? "" }
    private func figure(_ value: UInt64?) -> String { InsightsOverviewWords.figure(value, copy: copy) }

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                Group {
                    if let watched = model.watched {
                        feed(watched)
                    } else {
                        saved
                    }
                }
                .padding(.vertical, GlassTokens.Space.s7)
                .frame(maxWidth: .infinity, alignment: .leading)
                .insightsSurface()
            }
            .scrollPosition(id: $anchorRow, anchor: .top)
            .onChange(of: keyRow) { _, row in
                guard let row else { return }
                withAnimation(GlassMotion.fast(GlassMotion.systemReducesMotion)) { proxy.scrollTo(row) }
            }
        }
    }

    // MARK: Feed S

    /// One saved snapshot, picked from the saved list (feed S, as before).
    private var saved: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
            let choices = InsightsSessionsWords.pickerChoices(snapshots, copy: copy)
            if !choices.isEmpty {
                GlassSelect(text("analytics_drill_session"), selection: Binding(
                    get: { model.selected ?? "" },
                    set: { model.select($0); readTurn = nil }
                ), options: choices.map { GlassPickerOption($0.title, value: $0.value) })
            }
            if let drill = model.drill {
                drillContent(drill)
            } else if model.failed || choices.isEmpty {
                Text(text("analytics_unavailable")).insightsError()
            }
            if model.busy { GlassSpinner() }
        }
    }

    // MARK: Feed T

    /// Every watched trace read so far, newest first, drawn as it scrolls
    /// in; the last rows appearing read the next page.
    private func feed(_ rows: [DaemonData.InsightsWatchedSession]) -> some View {
        LazyVStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            ForEach(rows) { row in
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    feedRow(row)
                    if model.selected == row.sessionRef { expanded(row) }
                }
                .id(row.sessionRef)
                .onAppear {
                    if row.sessionRef == rows.last?.sessionRef { model.loadMore() }
                }
            }
            feedFoot(empty: rows.isEmpty)
        }
        .scrollTargetLayout()
        // Full Keyboard Access: focus the feed, then up and down move
        // between rows and Return opens or closes the row.
        .focusable()
        .focusEffectDisabled()
        .onMoveCommand { direction in
            let refs = rows.map(\.sessionRef)
            switch direction {
            case .up: keyRow = InsightsSessionsWords.moved(keyRow, by: -1, in: refs)
            case .down: keyRow = InsightsSessionsWords.moved(keyRow, by: 1, in: refs)
            default: break
            }
        }
        .onKeyPress(.return) {
            guard let keyRow, rows.contains(where: { $0.sessionRef == keyRow }) else { return .ignored }
            readTurn = nil
            model.toggle(keyRow)
            return .handled
        }
    }

    /// One watched trace: its harness tile, its label and tokens, and its
    /// sparkline, then its own labels' controls. VoiceOver reads the trace
    /// as one button, the sparkline in words; each control after it names
    /// the trace it labels.
    private func feedRow(_ row: DaemonData.InsightsWatchedSession) -> some View {
        let open = model.selected == row.sessionRef
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Button {
                readTurn = nil
                keyRow = row.sessionRef
                model.toggle(row.sessionRef)
            } label: {
                HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
                    InsightsHarnessTile(source: row.source)
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                        Text(InsightsSessionsWords.watchedLabel(row, copy: copy))
                            .insightsNote()
                            .foregroundStyle(GlassColor.textPrimary)
                            .lineLimit(1)
                        Text(InsightsSessionsWords.rowTokens(row, copy: copy)).insightsCaption()
                    }
                    Spacer(minLength: GlassTokens.Space.s4)
                    if let sparkline = row.sparkline {
                        InsightsSparkline(points: sparkline.points)
                            .frame(width: InsightsSparkline.width, height: InsightsSparkline.height)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(GlassPressStyle())
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(InsightsSessionsWords.rowAccessibilityLabel(row, copy: copy))
            .accessibilityAddTraits(open ? [.isButton, .isSelected] : .isButton)
            if model.canLabel { labelControls(row) }
        }
        .padding(.vertical, GlassTokens.Space.s3)
        .padding(.horizontal, GlassTokens.Space.s4)
        .insightsRowCard()
        .overlay {
            if keyRow == row.sessionRef {
                RoundedRectangle(cornerRadius: GlassTokens.Radius.cardQuiet)
                    .strokeBorder(GlassTokens.Color.selection.color, lineWidth: 2)
            }
        }
    }

    /// A row's own labels (Insights PM request, 2026-10-21): its task,
    /// pre-filled from the inferred one and marked so until the person
    /// takes it (one click on Confirm) or picks another, and its outcome,
    /// unset until they say. Kept on this Mac only. A label that could not
    /// be saved shows as it was, with the core's line under it.
    private func labelControls(_ row: DaemonData.InsightsWatchedSession) -> some View {
        let reference = row.sessionRef
        let shown = InsightsSessionsWords.shownTask(row)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            HStack(spacing: GlassTokens.Space.s3) {
                InsightsLabelMenu(
                    title: InsightsSessionsWords.taskTitle(row, copy: copy),
                    name: InsightsSessionsWords.labelAccessibility("analytics_label_task_a11y", row, copy: copy),
                    choices: InsightsSessionsWords.taskChoices(copy: copy),
                    chosen: row.userTask,
                    inferred: shown?.inferred ?? false,
                    unset: text("analytics_task_use_inferred")
                ) { model.setTask(reference, to: $0) }
                if InsightsSessionsWords.confirmableTask(row) != nil {
                    Button(text("analytics_task_confirm")) { model.confirmTask(reference) }
                        .buttonStyle(GlassButtonStyle(.link))
                        .accessibilityLabel(InsightsSessionsWords.labelAccessibility(
                            "analytics_task_confirm_a11y", row, copy: copy))
                }
                InsightsLabelMenu(
                    title: InsightsSessionsWords.outcomeTitle(row, copy: copy),
                    name: InsightsSessionsWords.labelAccessibility("analytics_label_outcome_a11y", row, copy: copy),
                    choices: InsightsSessionsWords.outcomeChoices(copy: copy),
                    chosen: row.userOutcome,
                    inferred: false,
                    unset: text("analytics_outcome_clear")
                ) { model.setOutcome(reference, to: $0) }
                Spacer(minLength: 0)
            }
            if model.labelFailed.contains(reference) {
                Text(text("analytics_label_save_failed")).insightsError()
            }
        }
        // Lined up under the label, past the harness tile.
        .padding(.leading, GlassTokens.Size.toolTile + GlassTokens.Space.s4)
    }

    /// The open row's Analyze & save, then its drill beneath it: the drill
    /// once read, the unavailable line if it could not be, a spinner
    /// meanwhile.
    @ViewBuilder
    private func expanded(_ row: DaemonData.InsightsWatchedSession) -> some View {
        if let analysis { analyzeAction(row, analysis) }
        if let drill = model.drill, drill.session_ref == row.sessionRef {
            drillContent(drill)
        } else if model.failed {
            Text(text("analytics_unavailable")).insightsError()
        } else {
            GlassSpinner()
        }
    }

    /// The open row's one action: the daemon analyzes the trace it found
    /// and saves the snapshot, which then opens in Analyze. A spinner while
    /// it is asked, and the core's line if it could not be.
    private func analyzeAction(_ row: DaemonData.InsightsWatchedSession, _ analysis: InsightsModel) -> some View {
        let reference = row.sessionRef
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            HStack(spacing: GlassTokens.Space.s3) {
                Button(text("analytics_analyze_save")) {
                    Task { if await analysis.analyzeAndSave(session: reference) { openAnalysis() } }
                }
                .buttonStyle(GlassButtonStyle(.secondary, small: true))
                .disabled(analysis.busy)
                .accessibilityLabel(InsightsSessionsWords.labelAccessibility(
                    "analytics_analyze_save_a11y", row, copy: copy))
                if analysis.analyzingSession == reference { GlassSpinner() }
                Spacer(minLength: 0)
            }
            if let failure = analysis.sessionAnalyzeFailure, failure.reference == reference {
                Text(text(failure.key)).insightsError()
            }
        }
        // Lined up under the label, past the harness tile, as the labels are.
        .padding(.leading, GlassTokens.Size.toolTile + GlassTokens.Space.s4)
    }

    /// The foot of the feed: the loading row while the next page is read,
    /// the unavailable line and a retry control if it could not be, and nothing at the end.
    @ViewBuilder
    private func feedFoot(empty: Bool) -> some View {
        if model.loadingMore {
            HStack(spacing: GlassTokens.Space.s3) {
                GlassSpinner()
                Text(text("analytics_sessions_loading_more")).insightsCaption()
            }
            .accessibilityElement(children: .combine)
        } else if model.pageFailed {
            HStack(spacing: GlassTokens.Space.s3) {
                Text(text("analytics_sessions_page_failed")).insightsError()
                Button(text("analytics_sessions_retry")) { model.retry() }
            }
        } else if empty {
            if model.busy { GlassSpinner() } else { Text(text("analytics_unavailable")).insightsError() }
        }
    }

    // MARK: A drill

    /// One session turn by turn: its header, then the chart and its marker
    /// cards, or the line that says why there is no chart.
    @ViewBuilder
    private func drillContent(_ drill: InsightsSessionDrill) -> some View {
        VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
            header(drill)
            if let line = InsightsSessionsWords.unavailableLine(drill, copy: copy) {
                Text(line).insightsNote()
            } else {
                chart(drill)
                LazyVGrid(columns: InsightsLayout.columns, alignment: .leading,
                          spacing: GlassTokens.Space.cardGap) {
                    ForEach(drill.markers) { marker in
                        markerCard(marker, threshold: drill.long_context_threshold)
                    }
                }
            }
        }
    }

    private func header(_ drill: InsightsSessionDrill) -> some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
            InsightsHarnessTile(source: drill.source ?? "")
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                // Which traces are counted, behind the header's "i".
                InsightsSectionTitle(InsightsSessionsWords.header(drill, copy: copy),
                                     info: InsightsSessionsWords.feedLines(drill.feed, notice: notice, copy: copy))
                let reasons = InsightsSessionsWords.headerReasons(drill, copy: copy)
                if !reasons.isEmpty {
                    Text(reasons.joined(separator: " \u{b7} "))
                        .insightsCaption()
                }
            }
        }
    }

    /// The stacked segments in stack order: cache read, cache write,
    /// uncached, from the axis up.
    private func stacked(_ drill: InsightsSessionDrill) -> [InsightsSessionsWords.Segment] {
        let rank = Dictionary(uniqueKeysWithValues: InsightsSeries.stack.enumerated().map { (text($1.key), $0) })
        return InsightsSessionsWords.segments(drill, copy: copy)
            .sorted { ($0.turn, rank[$0.series] ?? 0) < ($1.turn, rank[$1.series] ?? 0) }
    }

    /// The legend's figure for one series: the turn being read, or the
    /// session's known turns together. An unknown turn reads the dash.
    private func legendValue(_ series: InsightsSeries, drill: InsightsSessionDrill) -> String {
        let turns = drill.series ?? []
        func value(_ turn: InsightsDrillTurn) -> UInt64? {
            guard let read = turn.cache_read, let uncached = turn.uncached, let write = turn.cache_write else { return nil }
            switch series {
            case .cacheRead: return UInt64(read)
            case .uncached: return UInt64(uncached)
            case .cacheWrite: return write
            case .output: return nil
            }
        }
        if let readTurn {
            return figure(turns.first { Int($0.ordinal) == readTurn }.flatMap(value))
        }
        return figure(turns.compactMap(value).reduce(0, +))
    }

    /// The series the per-turn chart stacks, in palette order.
    static let turnSeries: [InsightsSeries] = [.cacheRead, .uncached, .cacheWrite]

    /// The per-turn chart's card: its heading, legend, plot and end turns.
    /// Built from small pieces: as one expression it is more than the Xcode
    /// 26 type checker will finish.
    private func chart(_ drill: InsightsSessionDrill) -> some View {
        let turns = (drill.series ?? []).map(\.ordinal)
        let first = turns.min() ?? 1
        let last = turns.max() ?? 1
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            chartHeading
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                legend(drill)
                plot(drill, first: first, last: last)
                HStack {
                    Text(InsightsSessionsWords.turnLabel(first, copy: copy)).insightsTick()
                    Spacer()
                    Text(InsightsSessionsWords.turnLabel(last, copy: copy)).insightsTick()
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    private var chartHeading: some View {
        HStack(alignment: .firstTextBaseline) {
            Text(text("analytics_input_each_turn")).insightsHeading()
            Spacer(minLength: GlassTokens.Space.s4)
            if let readTurn {
                Text(InsightsSessionsWords.turnLabel(UInt32(readTurn), copy: copy)).insightsCaption()
            }
        }
    }

    private func legend(_ drill: InsightsSessionDrill) -> some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: GlassTokens.Space.s3)],
                  spacing: GlassTokens.Space.s3) {
            ForEach(Self.turnSeries, id: \.self) { item in
                GlassLegendCell(text(item.key), value: legendValue(item, drill: drill), series: item.color)
            }
        }
    }

    /// The top of the value axis: room above the tallest bar and the
    /// threshold for the marker letters.
    private func plotTop(_ drill: InsightsSessionDrill) -> Double {
        let tallest: UInt64 = (drill.series ?? []).compactMap { turn -> UInt64? in
            guard let read = turn.cache_read, let uncached = turn.uncached, let write = turn.cache_write else { return nil }
            return UInt64(read) + UInt64(uncached) + write
        }.max() ?? 0
        return max(Double(max(tallest, drill.long_context_threshold)) * 1.18, 1)
    }

    /// The hovered turn as the chart's selection, held to the session's turns.
    private func selection(first: UInt32, last: UInt32) -> Binding<Double?> {
        Binding(
            get: { readTurn.map(Double.init) },
            set: { value in readTurn = value.map { min(max(Int($0.rounded()), Int(first)), Int(last)) } })
    }

    private func plot(_ drill: InsightsSessionDrill, first: UInt32, last: UInt32) -> some View {
        let series = Self.turnSeries
        let domain: ClosedRange<Double> = (Double(first) - 1)...(Double(max(last, first)) + 1)
        return Chart {
            washMark
            barMarks(drill)
            thresholdMark(drill)
            markerMarks(drill)
        }
        .chartForegroundStyleScale(domain: series.map { text($0.key) }, range: series.map { $0.color.color })
        .chartLegend(.hidden)
        .chartXSelection(value: selection(first: first, last: last))
        .chartXAxis(.hidden)
        .chartXScale(domain: domain)
        .chartYScale(domain: 0...plotTop(drill))
        .chartYAxis { valueAxis }
        .frame(height: 200)
        .insightsPlot()
        .accessibilityHidden(true)
    }

    /// The turn being read, one turn wide. The axis is Double so the wash
    /// spans exactly the turn's band.
    @ChartContentBuilder
    private var washMark: some ChartContent {
        if let readTurn {
            RectangleMark(xStart: .value(text("analytics_turn"), Double(readTurn) - 0.5),
                          xEnd: .value(text("analytics_turn"), Double(readTurn) + 0.5))
                .foregroundStyle(GlassColor.ink(0.08))
        }
    }

    private func barMarks(_ drill: InsightsSessionDrill) -> some ChartContent {
        ForEach(stacked(drill)) { segment in
            BarMark(x: .value(text("analytics_turn"), Double(segment.turn)),
                    y: .value(text("analytics_card_tokens"), segment.tokens))
            .foregroundStyle(by: .value(text("analytics_card_tokens"), segment.series))
        }
    }

    private func thresholdMark(_ drill: InsightsSessionDrill) -> some ChartContent {
        RuleMark(y: .value(text("analytics_pattern_long_context"), drill.long_context_threshold))
            .lineStyle(StrokeStyle(lineWidth: 1.5, dash: [4, 3]))
            .foregroundStyle(GlassTokens.Color.statusAsk.color)
            .annotation(position: .top, alignment: .trailing, spacing: GlassTokens.Space.s1) {
                Text(figure(drill.long_context_threshold)).insightsTick(current: true)
            }
    }

    private func markerMarks(_ drill: InsightsSessionDrill) -> some ChartContent {
        ForEach(drill.markers) { marker in
            let read = readTurn == Int(marker.turn_ordinal)
            RuleMark(x: .value(text("analytics_turn"), Double(marker.turn_ordinal)))
                .lineStyle(StrokeStyle(lineWidth: 1))
                .foregroundStyle(read ? GlassColor.textPrimary : GlassColor.textTertiary)
                .annotation(position: .top, spacing: 0) {
                    InsightsMarkerBadge(letter: marker.letter)
                }
        }
    }

    private var valueAxis: some AxisContent {
        AxisMarks(position: .leading, values: .automatic(desiredCount: 4)) { value in
            AxisGridLine().foregroundStyle(GlassTokens.Color.chartGrid.color)
            AxisValueLabel {
                if let tokens = value.as(Double.self) {
                    Text(figure(UInt64(max(0, tokens)))).insightsTick()
                }
            }
        }
    }

    /// The derivation a marker's lines end with ("Inferred from token
    /// counts."), which goes behind the title's "i"; `nil` when the marker
    /// carries none.
    static let derivationBases: Set<String> = ["inferred_from_counters", "from_counters", "from_tool_calls"]

    /// A marker: its letter, the core's lines with the derivation behind an
    /// "i", and its turn, which reads that turn on the chart (a second
    /// press reads the session). The card itself is not a button, so the
    /// "i" is never a control inside a control.
    private func markerCard(_ marker: InsightsDrillMarker, threshold: UInt64) -> some View {
        var lines = InsightsSessionsWords.markerLines(marker, threshold: threshold, copy: copy)
        let derivation = Self.derivationBases.contains(marker.basis) && lines.count > 1 ? lines.removeLast() : nil
        let turn = Int(marker.turn_ordinal)
        return HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
            InsightsMarkerBadge(letter: marker.letter)
            VStack(alignment: .leading, spacing: InsightsLayout.lineGap) {
                if let title = lines.first {
                    InsightsGroupHeading(title, info: derivation.map { [$0] } ?? [])
                }
                ForEach(Array(lines.dropFirst().enumerated()), id: \.offset) { _, line in
                    Text(line).insightsNote().fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: GlassTokens.Space.s4)
            Button(InsightsSessionsWords.turnLabel(marker.turn_ordinal, copy: copy)) {
                readTurn = readTurn == turn ? nil : turn
            }
            .buttonStyle(GlassButtonStyle(.link))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }
}

/// A row's label control: a native menu dressed as a compact glass pill, in
/// the feed row's caption type on the control tier, with the system's check
/// beside the chosen word and, while one is chosen, the item that unsets
/// it. `inferred` draws the words in the secondary ink, so a value the
/// rules inferred never reads as one the person set. Every word is the
/// caller's, from the core's copy.
struct InsightsLabelMenu: View {
    let title: String
    /// What VoiceOver calls the control, naming its row.
    let name: String
    let choices: [(value: String, title: String)]
    /// The person's own value; `nil` while unset.
    let chosen: String?
    let inferred: Bool
    /// The item that unsets the value, shown while one is chosen.
    let unset: String
    let choose: (String?) -> Void

    var body: some View {
        Menu {
            ForEach(choices, id: \.value) { choice in
                Toggle(choice.title, isOn: Binding(
                    get: { chosen == choice.value },
                    set: { if $0 { choose(choice.value) } }))
            }
            if chosen != nil {
                Divider()
                Button(unset) { choose(nil) }
            }
        } label: {
            HStack(spacing: GlassTokens.Space.s1) {
                Text(title).lineLimit(1)
                GlassGlyphView(.chevronDown)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            .glassType(GlassTokens.TypeScale.caption.weight(.semibold))
            .foregroundStyle(inferred ? GlassColor.textSecondary : GlassColor.textPrimary)
            .padding(.horizontal, GlassTokens.Space.s3)
            .frame(minHeight: GlassTokens.Size.submitPill)
            .glassSurface(.control, hover: GlassTokens.Color.controlHover)
        }
        // As `GlassSelect`: a button-style menu keeps the pill as drawn,
        // and `GlassPressStyle` darkens it while pressed.
        .menuStyle(.button)
        .buttonStyle(GlassPressStyle())
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel(name)
        .accessibilityValue(title)
    }
}

/// A marker's letter in a solid round badge, on the chart and on its card.
struct InsightsMarkerBadge: View {
    let letter: String

    static let side: CGFloat = 16

    var body: some View {
        Text(letter)
            .glassType(GlassTokens.TypeScale.micro)
            .foregroundStyle(GlassTokens.Color.paneOpaque.color)
            .frame(width: Self.side, height: Self.side)
            .background(Circle().fill(GlassColor.textPrimary))
    }
}

/// A row's sparkline: tokens per turn as one line over a soft fill, scaled
/// to the row's largest point, in the first chart series token (light and
/// dark). Drawn on a Canvas: no layout per point. Hidden from VoiceOver; the
/// row's label says it in words.
struct InsightsSparkline: View {
    let points: [UInt64]

    static let width: CGFloat = 96
    static let height: CGFloat = 24

    /// Each point's place in a `size` box, oldest on the left, 0 at the
    /// bottom and the largest at the top. One point is a level line.
    static func positions(_ points: [UInt64], in size: CGSize) -> [CGPoint] {
        guard !points.isEmpty else { return [] }
        let top = Double(max(points.max() ?? 0, 1))
        let step = points.count > 1 ? size.width / CGFloat(points.count - 1) : 0
        return points.enumerated().map { index, value in
            CGPoint(x: points.count > 1 ? CGFloat(index) * step : size.width / 2,
                    y: size.height - CGFloat(Double(value) / top) * size.height)
        }
    }

    var body: some View {
        Canvas { context, size in
            let inset: CGFloat = 1.5
            let box = CGSize(width: size.width, height: max(size.height - inset * 2, 0))
            let at = Self.positions(points, in: box).map { CGPoint(x: $0.x, y: $0.y + inset) }
            guard let first = at.first else { return }
            let series = GlassTokens.Color.chartSeries1.color
            if at.count == 1 {
                var level = Path()
                level.move(to: CGPoint(x: 0, y: first.y)); level.addLine(to: CGPoint(x: size.width, y: first.y))
                context.stroke(level, with: .color(series), lineWidth: 1.5)
                return
            }
            var line = Path()
            line.addLines(at)
            var area = line
            area.addLine(to: CGPoint(x: at[at.count - 1].x, y: size.height))
            area.addLine(to: CGPoint(x: first.x, y: size.height))
            area.closeSubpath()
            context.fill(area, with: .color(series.opacity(0.18)))
            context.stroke(line, with: .color(series),
                           style: StrokeStyle(lineWidth: 1.5, lineCap: .round, lineJoin: .round))
        }
        .accessibilityHidden(true)
    }
}
