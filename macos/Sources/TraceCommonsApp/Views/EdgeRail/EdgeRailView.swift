import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The edge rail's content (`EdgeRailController`): closed, a short handle
/// on the screen edge; open, the icon rail with the hovered icon's peek
/// beside it. Every word is the core's (`MonitorEdgeRailCopy`, and the
/// tables each peek already shows elsewhere in the app).
struct EdgeRailView: View {
    let state: EdgeRailState
    /// Asks the controller to resize the panel for the new state.
    let onOpenChange: (Bool) -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @EnvironmentObject private var model: AppModel
    /// The pending close: leaving the rail for a moment, as the pointer
    /// does crossing from an icon to its peek, does not close it.
    @State private var closing: Task<Void, Never>?
    /// The maps' and graphs' data, read through the app's live client as
    /// the window's are, and only while the rail is open: a closed rail
    /// reads nothing.
    @State private var data = EdgeRailData()

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
                    systemImage: peek.symbol,
                    label: peek.name(rail, nav: MonitorWords.table?.settingsNav, shell: MonitorWords.table?.shell),
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
    let systemImage: String
    let label: String
    let selected: Bool
    let select: () -> Void

    var body: some View {
        GlassRailTile(selected: selected, action: select) {
            Image(systemName: systemImage).glassGlyph(17, weight: .regular)
        }
        .onHover { if $0 { select() } }
        .help(label)
        .accessibilityLabel(label)
    }
}

/// The rail's own reads for the maps and graphs: the Traces tree and
/// status, the Private AI calls and tools, and the contribution history.
@MainActor
final class EdgeRailData {
    let traces = TracesStore(client: nil)
    let inference = InferenceStore(client: nil)
    let home = HomeStore(client: nil)

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

/// The hovered icon's peek: a short summary of its part of the app, at
/// most one action, and a link into the app. The open rail's height
/// (`edgeRailOpenHeight`) holds the tallest, Traces with its map and graph.
private struct EdgeRailPeekCard: View {
    let peek: EdgeRailPeek
    let data: EdgeRailData
    let close: () -> Void

    @EnvironmentObject private var model: AppModel
    @Environment(ComputeModel.self) private var compute

    private var rail: MonitorEdgeRailCopy? { MonitorWords.table?.edgeRail }
    private var shell: MonitorShellCopy? { MonitorWords.table?.shell }
    private var nav: MonitorSettingsNavCopy? { MonitorWords.table?.settingsNav }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            Text(peek == .traces ? rail?.privacyTitle ?? "" : peek.name(rail, nav: nav, shell: shell))
                .glassType(GlassTokens.TypeScale.heading)
                .foregroundStyle(GlassColor.textPrimary)
                .accessibilityAddTraits(.isHeader)
            content
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.bottom, GlassTokens.Space.s3)
            link
        }
        .padding(GlassTokens.Space.s9)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassTier(.pane)
    }

    @ViewBuilder
    private var content: some View {
        switch peek {
        case .overview: overview
        case .privateAI: privateAI
        case .traces: traces
        case .compute: computeSummary
        }
    }

    // MARK: Overview

    /// What Waiting, Tools and Balance showed as three icons, as one page
    /// (owner, 2026-10-11), each part under its own name.
    private var overview: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            part(rail?.waiting ?? "") { waiting }
            part(nav?.tools ?? "") { tools }
            if model.privateInferenceCopy != nil {
                part(rail?.balance ?? "") { balance }
            }
        }
    }

    /// One part of the Overview: its name as an eyebrow over its content.
    private func part<Content: View>(_ name: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Text(name)
                .glassType(GlassTokens.TypeScale.eyebrow)
                .foregroundStyle(GlassColor.textTertiary)
                .accessibilityAddTraits(.isHeader)
            content()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var waiting: some View {
        let count = model.awaitingDecision.count
        return HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Text(shell?.waiting(count, secondLook: 0) ?? "")
                .glassType(GlassTokens.TypeScale.title)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
            if count > 0 {
                Button(rail?.review ?? "") { open(.traces(entryId: nil)) }
                    .buttonStyle(GlassButtonStyle(.primary, small: true))
            }
        }
    }

    @ViewBuilder
    private var tools: some View {
        let rows = model.harnesses.harnesses
        if rows.isEmpty {
            caption(rail?.toolsEmpty ?? "")
        } else if let copy = model.privateInferenceCopy {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(rows) { row in
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                        Text(row.name)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                        Spacer(minLength: 0)
                        Text(row.connected ? copy.harnessCaptionConnected : copy.harnessCaptionNotConnected)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .lineLimit(1)
                    }
                    .padding(.vertical, 7)
                    .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
                }
            }
        }
    }

    @ViewBuilder
    private var balance: some View {
        if let copy = model.privateInferenceCopy {
            BalanceRow(copy: copy)
        }
    }

    // MARK: Private AI

    /// The connection's state, the Private AI map, and the calls by day.
    @ViewBuilder
    private var privateAI: some View {
        if let copy = model.privateInferenceCopy {
            let state = model.privateInferenceState
            let calls = model.privateInferenceCalls
            VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
                GlassStatusLabel(
                    PrivateInferenceSurface.stateLine(state, copy: copy, calls: calls),
                    status: PrivateInferenceIndicator.status(PrivateInferenceSurface.tone(state, calls: calls)))
                    .fixedSize(horizontal: false, vertical: true)
                EdgeRailMiniMap { privateAIMap(copy) }
                EdgeRailCallsGraph(page: data.inference.calls)
            }
        }
    }

    @ViewBuilder
    private func privateAIMap(_ copy: PrivateInferenceCopy) -> some View {
        // The one tool list, as the window's map draws it.
        let harnesses = PrivateAIInspectorView.liveHarnesses(
            model.harnesses, read: data.inference.harnesses, failure: data.inference.failures["harness_list"])
        if harnesses != .none {
            FlowMapView(
                scene: .privateAI(
                    harnesses, destinationLabel: copy.destination,
                    privateAI: data.traces.destinations?.privateAi ?? data.traces.status?.privateInferenceState?.state,
                    sentence: { MonitorWindowView.rowSentence($0, copy: copy, calls: model.harnessCalls) },
                    state: { HarnessSurface.state($0, calls: model.harnessCalls) }),
                legend: [], zoomable: false, accessibilityName: copy.destination)
        } else {
            Color.clear
        }
    }

    // MARK: Traces

    /// What is where right now: what waits on this Mac, what was sent, and
    /// the watcher with its pause; then the Traces map and the shared and
    /// kept graph.
    private var traces: some View {
        let paused = model.status.paused
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
            VStack(alignment: .leading, spacing: 0) {
                placeRow(rail?.onThisMac ?? "", status: .kept,
                         line: rail?.onThisMac(count: model.pending.count) ?? "")
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
            EdgeRailMiniMap { tracesMap }
            EdgeRailTracesGraph(data: data)
        }
    }

    private var tracesMap: some View {
        let store = data.traces
        var failure: DaemonDataError?
        if case .failed(let error) = store.phase { failure = error }
        let state = ScreenState.resolve(
            failure: failure, loaded: store.phase != .loading,
            paused: store.status?.paused, known: store.status != nil)
        let scene = FlowMapScene.traces(
            store.tree,
            gate: .init(state: state, status: store.status, destinations: store.destinations),
            contributed: .init(history: data.home.history, failure: data.home.failures["list_history"]))
        return FlowMapView(
            scene: scene, legend: [], zoomable: false,
            accessibilityName: FlowMapScene.words?.mapLabel ?? shell?.tabTraces ?? "", state: state)
    }

    private func placeRow(_ title: String, status: GlassStatus, line: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            GlassStatusLabel(title, status: status)
                .frame(width: GlassTokens.Size.edgeRailPeekWidth / 3, alignment: .leading)
            Text(line)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(.vertical, 7)
        .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
    }

    // MARK: Compute

    @ViewBuilder
    private var computeSummary: some View {
        if let snapshot = compute.snapshot {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                Text(snapshot.title)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                caption(snapshot.detail)
            }
        } else {
            caption(rail?.computeUnreported ?? "")
        }
    }

    // MARK: Shared

    /// The peek's link: the section's name, or the Home tab's for the
    /// Overview.
    private var link: some View {
        let words = rail?.open(section: peek.linkSection(rail, nav: nav, shell: shell))
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

    private func caption(_ sentence: String) -> some View {
        Text(sentence)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// Opens the app at the peek's destination and puts the rail away.
    private func open(_ destination: MonitorDestination) {
        close()
        OpenMonitor.request(destination)
    }
}

/// A small map in a peek: the window's map field and edge, at a fixed
/// height, not zoomable.
private struct EdgeRailMiniMap<Map: View>: View {
    @ViewBuilder let map: () -> Map

    static var height: CGFloat { 180 }

    var body: some View {
        GlassPane(padding: 0, isContent: true, edge: GlassTokens.Shadow.mapEdge) {
            GlassMapStage(alignment: .topTrailing) { map() }
        }
        .frame(height: Self.height)
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
/// over the same days the Traces graph shows. A page that is not the whole
/// window draws its bars, but its total is a dash.
private struct EdgeRailCallsGraph: View {
    let page: DaemonData.InferenceCallPage?
    @State private var hovered: String?

    var body: some View {
        let calls = page?.readable == true ? page?.calls ?? [] : []
        let buckets = TracesGraphModel.buckets(
            shared: calls.map(\.at), kept: [], offset: 0, now: .now, range: .days)
        let shown = buckets.first { $0.start.formatted(.iso8601) == hovered }
        let whole = page?.readable == true && page?.nextCursor == nil
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(
                    MonitorWords.calls,
                    value: TracesGraphFooter.figure(shown?.shared ?? buckets.map(\.shared).reduce(0, +), known: whole),
                    status: .shared)
            }
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
}

enum EdgeRailGraph {
    /// A peek's graph height: the bars and their day labels, which draw
    /// below the bars, so the frame holds both.
    static let height: CGFloat = 112
}
