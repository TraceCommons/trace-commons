import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The edge rail's content (`EdgeRailController`): closed, a short handle
/// on the screen edge; open, the icon rail with the hovered icon's peek
/// beside it. Each peek is a compact summary of one of the app's tabs, in
/// the app's own cards and words (owner, 2026-10-11). Every word is the
/// core's (`MonitorEdgeRailCopy`, and the tables each tab already shows).
struct EdgeRailView: View {
    let state: EdgeRailState
    /// Asks the controller to resize the panel for the new state.
    let onOpenChange: (Bool) -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @EnvironmentObject private var model: AppModel
    /// The pending close: leaving the rail for a moment, as the pointer
    /// does crossing from an icon to its peek, does not close it.
    @State private var closing: Task<Void, Never>?
    /// The peeks' data, read through the app's live client as the
    /// window's is, and only while the rail is open: a closed rail reads
    /// nothing.
    @State private var data: EdgeRailData

    @MainActor
    init(state: EdgeRailState, data: EdgeRailData? = nil, onOpenChange: @escaping (Bool) -> Void) {
        self.state = state
        self._data = State(initialValue: data ?? EdgeRailData())
        self.onOpenChange = onOpenChange
    }

    private var rail: MonitorEdgeRailCopy? { MonitorWords.table?.edgeRail }

    /// What the rail's reads hang on: the window's attachment, and whether
    /// the rail is open.
    private struct Reading: Hashable {
        let attachment: MonitorWindowView.Attachment
        let open: Bool
    }

    var body: some View {
        Group {
            if state.open {
                HStack(alignment: .center, spacing: GlassTokens.Space.edgeRailGap) {
                    EdgeRailPeekCard(peek: state.peek, data: data, close: { setOpen(false) })
                        .frame(width: GlassTokens.Size.edgeRailPeekWidth)
                    icons
                }
                .padding(.trailing, GlassTokens.Space.edgeRailInset)
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
                .transition(reduceMotion ? .opacity : .move(edge: .trailing).combined(with: .opacity))
            } else {
                handle
            }
        }
        .onHover { inside in
            if inside {
                closing?.cancel()
                closing = nil
                if !state.open { setOpen(true) }
            } else if state.open {
                closing = Task { @MainActor in
                    try? await Task.sleep(for: .milliseconds(300))
                    guard !Task.isCancelled else { return }
                    setOpen(false)
                }
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(rail?.railLabel ?? "")
        .task(id: Reading(attachment: MonitorWindowView.Attachment(model), open: state.open)) {
            guard state.open else { return }
            await data.run(MonitorWindowView.sampleClient() ?? model.daemonData,
                           awaiting: MonitorWindowView.Attachment(model).awaiting)
        }
    }

    private func setOpen(_ open: Bool) {
        // The panel grows before the rail draws into it, and shrinks after
        // the rail has gone.
        if open { onOpenChange(true) }
        withAnimation(reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide)) {
            state.open = open
        }
        if !open { onOpenChange(false) }
    }

    /// The closed rail: its handle on the edge (`GlassRailHandle`).
    private var handle: some View {
        GlassRailHandle()
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .trailing)
            .contentShape(Rectangle())
            .accessibilityElement()
            .accessibilityLabel(rail?.handleLabel ?? "")
            .accessibilityAddTraits(.isButton)
            .accessibilityAction { setOpen(true) }
    }

    /// The icon rail, a pane on the desktop: one tile per peek, a rule,
    /// then the app.
    private var icons: some View {
        VStack(spacing: GlassTokens.Space.s1) {
            ForEach(EdgeRailPeek.allCases) { peek in
                EdgeRailTile(
                    icon: peek.icon,
                    label: peek.name(nav: MonitorWords.table?.settingsNav, shell: MonitorWords.table?.shell),
                    selected: state.peek == peek
                ) { state.peek = peek }
            }
            GlassRailRule()
            GlassRailTile(selected: false) {
                setOpen(false)
                OpenMonitor.request(nil)
            } label: {
                Image(nsImage: NSApp.applicationIconImage)
                    .resizable()
                    .frame(width: GlassTokens.Size.edgeRailTile - GlassTokens.Space.s7,
                           height: GlassTokens.Size.edgeRailTile - GlassTokens.Space.s7)
            }
            .help(rail?.openApp ?? "")
            .accessibilityLabel(rail?.openApp ?? "")
        }
        .padding(.vertical, GlassTokens.Space.s3)
        .frame(width: GlassTokens.Size.edgeRailWidth)
        .glassTier(.pane)
    }
}

/// One icon on the rail (`GlassRailTile`). Hovering it shows its peek, as
/// clicking does.
private struct EdgeRailTile: View {
    let icon: EdgeRailIcon
    let label: String
    let selected: Bool
    let select: () -> Void

    var body: some View {
        GlassRailTile(selected: selected, action: select) {
            switch icon {
            case .symbol(let name):
                Image(systemName: name).glassGlyph(17, weight: .regular)
            case .traceMark:
                // PROVISIONAL until the 1c Live Traces artwork lands.
                Image(systemName: "point.3.connected.trianglepath.dotted").glassGlyph(17, weight: .regular)
            }
        }
        .onHover { if $0 { select() } }
        .help(label)
        .accessibilityLabel(label)
    }
}

/// The rail's own reads: the Traces tree and status, the Private AI calls
/// and tools, and Home's status, rollup, credit and history.
@MainActor
final class EdgeRailData {
    let traces: TracesStore
    let inference: InferenceStore
    let home: HomeStore

    /// Stores already loaded may be handed in (the debug screenshots, which
    /// never run the rail's attaching task).
    init(traces: TracesStore? = nil, inference: InferenceStore? = nil, home: HomeStore? = nil) {
        self.traces = traces ?? TracesStore(client: nil)
        self.inference = inference ?? InferenceStore(client: nil)
        self.home = home ?? HomeStore(client: nil)
    }

    /// Attaches the client and follows it until the calling task ends.
    func run(_ client: (any DaemonDataClient)?, awaiting: Bool) async {
        traces.attach(client, awaiting: awaiting)
        inference.attach(client, awaiting: awaiting)
        home.attach(client, awaiting: awaiting)
        async let a: () = traces.run()
        async let b: () = inference.run()
        async let c: () = home.run()
        _ = await (a, b, c)
    }
}

/// The hovered icon's peek: its tab's name, a compact summary of the tab in
/// the tab's own cards, and a link into it. The open rail's height
/// (`edgeRailOpenHeight`) holds the tallest.
private struct EdgeRailPeekCard: View {
    let peek: EdgeRailPeek
    let data: EdgeRailData
    let close: () -> Void

    @EnvironmentObject private var model: AppModel

    private var rail: MonitorEdgeRailCopy? { MonitorWords.table?.edgeRail }
    private var shell: MonitorShellCopy? { MonitorWords.table?.shell }
    private var nav: MonitorSettingsNavCopy? { MonitorWords.table?.settingsNav }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            Text(peek.name(nav: nav, shell: shell))
                .glassType(GlassTokens.TypeScale.heading)
                .foregroundStyle(GlassColor.textPrimary)
                .accessibilityAddTraits(.isHeader)
            content
                .frame(maxWidth: .infinity, alignment: .leading)
            link
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassTier(.pane)
    }

    @ViewBuilder
    private var content: some View {
        switch peek {
        case .home: home
        case .privateAI: privateAI
        case .traces: traces
        }
    }

    // MARK: Home

    /// Home, compact: its status card, its three figures, and its Missions
    /// and History cards as heads only, each opening its page.
    private var home: some View {
        let words = MonitorWords.table?.homeHistory
        let rollup = SummaryFacts.fresh(data.home.rollup, unless: data.home.failures["history_rollup"])
        let credit = SummaryFacts.fresh(data.home.credit, unless: data.home.failures["commons_credit_summary"])
        return VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            Button { open(.traces(entryId: nil)) } label: {
                GlassCard(interactive: true) {
                    HStack(spacing: GlassTokens.Space.s6) {
                        // Home's status card: the waiting line starts under
                        // the status words, after the dot.
                        VStack(alignment: .homeStatusText, spacing: GlassTokens.Space.s1) {
                            HomeStatusLine(store: data.home)
                                .glassType(GlassTokens.TypeScale.bodyStrong)
                            if let line = HomeFormat.waitingLine(
                                data.traces.decisionsOwed, sessions: data.traces.tree.allSessions) {
                                Text(line)
                                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                                    .foregroundStyle(GlassColor.textSecondary)
                                    .alignmentGuide(.homeStatusText) { $0[.leading] }
                            }
                        }
                        Spacer(minLength: 0)
                        GlassChevron()
                    }
                    .foregroundStyle(GlassColor.textPrimary)
                }
            }
            .buttonStyle(GlassPressStyle())
            .accessibilityElement(children: .combine)
            .accessibilityHint(HomeFormat.openTracesWord)
            HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                GlassStatCard(MonitorWords.waiting, value: HomeFormat.count(data.traces.decisionsOwed))
                GlassStatCard(MonitorWords.contributed, value: HomeFormat.count(rollup?.allTime?.accepted))
                GlassStatCard(HomeFormat.creditPendingWord, value: HomeFormat.pendingFigure(
                    rollup?.creditPending, condition: HomeFormat.pendingCondition(credit)))
            }
            .fixedSize(horizontal: false, vertical: true)
            headCard(
                systemImage: HomeOverviewIcons.missions, title: MonitorWords.missions,
                subtitle: words?.missionsSubtitle, promo: GlassPromoTreatment.stored,
                destination: .home(.missionDrafts))
            headCard(
                systemImage: HomeOverviewIcons.history, title: MonitorWords.history,
                subtitle: words?.historyDescription, destination: .home(.history))
        }
    }

    /// A Home card as a head only: icon, title, subtitle and chevron.
    private func headCard(
        systemImage: String, title: String, subtitle: String?, promo: GlassPromoTreatment? = nil,
        destination: MonitorDestination
    ) -> some View {
        Button { open(destination) } label: {
            GlassIconCard(
                systemImage: systemImage, title: title, subtitle: subtitle.map { [$0] } ?? [],
                interactive: true, promo: promo, trailing: { GlassChevron() })
        }
        .buttonStyle(GlassPressStyle())
    }

    // MARK: Private AI

    /// Private AI, compact: the tab's summary (status, credential, tools),
    /// its window's totals and calls by day, and the balance card.
    @ViewBuilder
    private var privateAI: some View {
        if let copy = model.privateInferenceCopy {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                PrivateAIInspectorView(store: data.inference)
                if let page = data.inference.calls, page.readable {
                    let totals = InferenceTabView.totals(
                        page, summary: data.inference.summary, destinations: data.inference.destinations)
                    GlassCard {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                            Text(InferenceTabView.windowLine(page, destinations: data.inference.destinations))
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textTertiary)
                            HStack(spacing: GlassTokens.Space.s3) {
                                GlassLegendCell(MonitorWords.calls, value: totals.calls.map(String.init) ?? "—", status: .shared)
                                GlassLegendCell(InferenceWords.proof(.verified), value: totals.verified.map(String.init) ?? "—", status: .on)
                                GlassLegendCell(MonitorWords.priced, value: totals.priced, status: .kept)
                            }
                            EdgeRailCallsGraph(page: page)
                        }
                    }
                }
                PrivateAIBalanceCard(copy: copy)
            }
        }
    }

    // MARK: Traces

    /// Traces, compact: what is where (waiting here, sent, the watcher with
    /// its pause), the first watched directories as the tree draws them,
    /// and the shared and kept graph.
    private var traces: some View {
        let paused = model.status.paused
        let words = data.traces.words
        let folders = Array(data.traces.tree.folders.filter { $0.mode != .ignore }.prefix(Self.foldersShown))
        return VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            GlassCard {
                VStack(alignment: .leading, spacing: 0) {
                    placeRow(rail?.onThisMac ?? "", status: .kept,
                             line: rail?.onThisMac(count: model.pending.count) ?? "", first: true)
                    placeRow(rail?.inTheLibrary ?? "", status: .shared,
                             line: rail?.inTheLibrary(count: model.history.count) ?? "")
                    HStack(spacing: GlassTokens.Space.s3) {
                        GlassStatusLabel(paused ? shell?.watcherPaused ?? "" : shell?.watcherWatching ?? "",
                                         status: paused ? .off : .on)
                        Spacer(minLength: 0)
                        Button(paused ? MenuBarWords.resume : MenuBarWords.pause) {
                            if paused { model.resume() } else { model.pause(until: nil) }
                        }
                        .buttonStyle(GlassButtonStyle(.glass, small: true))
                    }
                    .padding(.top, GlassTokens.Space.s4)
                    .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
                }
            }
            if let words, !folders.isEmpty {
                VStack(alignment: .leading, spacing: 2) {
                    Text(words.tree.treeTitle)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                        .padding(.bottom, GlassTokens.Space.s2)
                    ForEach(folders) { folder in
                        GlassListRow(
                            depth: .folder, tile: .folder, title: folder.label,
                            sub: TracesTreeView.folderSub(
                                folder, words: words, modeLabels: data.traces.disclosure?.folderModeLabels ?? [:]),
                            onSelect: { open(.traces(entryId: nil)) })
                            // Its own height: the peek's spare room is not the rows'.
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .environment(\.glassTreeRoot, .folder)
            }
            EdgeRailTracesGraph(data: data)
        }
    }

    /// The watched directories a Traces peek lists; the tab has the rest.
    static let foldersShown = 3

    private func placeRow(_ title: String, status: GlassStatus, line: String, first: Bool = false) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            GlassStatusLabel(title, status: status)
                .frame(width: GlassTokens.Size.edgeRailPeekWidth / 3, alignment: .leading)
            Text(line)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.top, first ? 0 : 7)
        .padding(.bottom, 7)
        .overlay(alignment: .top) { if !first { GlassHairline(GlassColor.hairline) } }
    }

    // MARK: Shared

    /// The peek's link: the tab's name.
    private var link: some View {
        let words = rail?.open(section: peek.name(nav: nav, shell: shell))
        return HStack {
            Button { open(peek.destination) } label: {
                HStack(spacing: GlassTokens.Space.s2) {
                    Text(words?.text ?? "")
                    Image(systemName: "arrow.right").glassGlyph(10, weight: .semibold)
                }
            }
            .buttonStyle(GlassButtonStyle(.link))
            .accessibilityLabel(words?.label ?? "")
            Spacer(minLength: 0)
            Text(rail?.nothingSent ?? "")
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
        }
    }

    /// Opens the app at a destination and puts the rail away.
    private func open(_ destination: MonitorDestination) {
        close()
        OpenMonitor.request(destination)
    }
}

/// The Traces graph in a peek: the shared and kept legend over the last
/// days' bars, as the graph under the tree draws them, without its
/// controls.
private struct EdgeRailTracesGraph: View {
    let data: EdgeRailData
    @State private var hovered: String?

    var body: some View {
        let history = TracesGraphFooter.readable(data.home.history, failure: data.home.failures["list_history"])
        let complete = TracesGraphFooter.countable(data.home.history, failure: data.home.failures["list_history"]) != nil
        let buckets = TracesGraphModel.buckets(
            shared: TracesGraphModel.sharedTimes(history ?? [], tool: nil),
            kept: TracesGraphModel.keptTimes(data.traces.tree.allSessions, tool: nil),
            offset: 0, now: .now, range: .days)
        let shown = buckets.first { $0.start.formatted(.iso8601) == hovered }
        let words = MonitorShellWords.graphWords
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(
                    MenuWords.shared,
                    value: TracesGraphFooter.figure(shown?.shared ?? buckets.map(\.shared).reduce(0, +),
                                                    known: history != nil && complete),
                    status: .shared)
                GlassLegendCell(MenuWords.kept, value: String(shown?.kept ?? buckets.map(\.kept).reduce(0, +)), status: .kept)
            }
            GlassBarGraph(buckets.map { TracesGraphFooter.bar($0, words: words) }, scaleFloor: 5, hovered: $hovered)
                .frame(height: EdgeRailGraph.height)
        }
    }
}

/// The Private AI calls in a peek: the calls on the ledger's page, by day,
/// over the same days the Traces graph shows.
private struct EdgeRailCallsGraph: View {
    let page: DaemonData.InferenceCallPage
    @State private var hovered: String?

    var body: some View {
        let buckets = TracesGraphModel.buckets(
            shared: page.calls.map(\.at), kept: [], offset: 0, now: .now, range: .days)
        GlassBarGraph(
            buckets.map { bucket in
                let day = TracesGraphModel.day(bucket.start)
                return GlassBarBucket(
                    id: bucket.start.formatted(.iso8601), label: bucket.label,
                    up: Double(bucket.shared), down: 0,
                    description: "\(day), \(FlowMapScene.pair(MonitorWords.calls, bucket.shared))")
            },
            scaleFloor: 5, hovered: $hovered)
            .frame(height: EdgeRailGraph.height)
    }
}

enum EdgeRailGraph {
    /// A peek's graph height: the bars and their day labels, which draw
    /// below the bars, so the frame holds both.
    static let height: CGFloat = 110
}
