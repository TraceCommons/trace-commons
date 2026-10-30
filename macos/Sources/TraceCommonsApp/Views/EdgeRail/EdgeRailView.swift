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
    /// Asks the controller to open or close the rail, with the animation
    /// the rail slides with (`EdgeRailOpening`): it sizes the panel and
    /// changes `state.open`, each in its turn.
    let request: (Bool, Animation?) -> Void

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
    init(state: EdgeRailState, data: EdgeRailData? = nil, request: @escaping (Bool, Animation?) -> Void) {
        self.state = state
        self._data = State(initialValue: data ?? EdgeRailData())
        self.request = request
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
        request(open, reduceMotion ? nil : GlassMotion.curve(GlassTokens.Motion.slide))
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
                // The NEAR AI mark in the brand blue (owner, 2026-10-22), in
                // place of the app icon.
                GlassNearMark()
                    .frame(width: Self.markSide, height: Self.markSide)
            }
            .help(rail?.openApp ?? "")
            .accessibilityLabel(rail?.openApp ?? "")
        }
        .padding(.vertical, GlassTokens.Space.s3)
        .frame(width: GlassTokens.Size.edgeRailWidth)
        .glassTier(.pane)
    }

    /// The N mark's side on the app's tile: the size of the tiles' glyphs.
    static let markSide: CGFloat = 20
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
            case .traceNodes:
                // In the tile's own ink, the size of the tiles' glyphs.
                GlassTraceNodesGlyph(ink: selected ? GlassTokens.Color.textOnSelected.color : GlassColor.textPrimary)
                    .frame(width: EdgeRailView.markSide, height: EdgeRailView.markSide)
            }
        }
        .onHover { if $0 { select() } }
        .help(label)
        .accessibilityLabel(label)
    }
}

/// The rail's own reads: the Traces tree and status, the Private AI calls
/// and tools, and Home's rollup and credit for the Account tiles.
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

/// The hovered icon's peek: its tab's name and a compact summary of the tab
/// in the tab's own cards; no link or line under them (owner, 2026-10-22).
/// The open rail's height (`edgeRailOpenHeight`) holds the tallest.
private struct EdgeRailPeekCard: View {
    let peek: EdgeRailPeek
    let data: EdgeRailData
    let close: () -> Void

    @EnvironmentObject private var model: AppModel

    private var shell: MonitorShellCopy? { MonitorWords.table?.shell }
    private var nav: MonitorSettingsNavCopy? { MonitorWords.table?.settingsNav }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            header
            content
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassTier(.pane)
    }

    /// The peek's title, with Private AI's On or Off right after it, and
    /// the menu-bar popover's buttons in the top right corner (owner,
    /// 2026-10-22): Private AI's Settings, which opens Settings at Private
    /// AI, and Quit; Traces' Watching, Settings and Quit.
    private var header: some View {
        HStack(spacing: GlassTokens.Space.s2) {
            Text(peek.name(nav: nav, shell: shell))
                .glassType(GlassTokens.TypeScale.heading)
                .foregroundStyle(GlassColor.textPrimary)
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)
            if peek == .privateAI, model.privateInferenceCopy != nil {
                PrivateAIStatusLabel()
                    .padding(.leading, GlassTokens.Space.s4)
            }
            Spacer(minLength: GlassTokens.Space.s4)
            if peek == .traces {
                WatchingMenuButton()
            }
            GlassRoundButton(MonitorWords.table?.settingsTitle ?? MenuWords.settings, icon: .glyph(.gear)) {
                open(.settings(EdgeRailSettings.section(peek)))
            }
            GlassRoundButton(MenuWords.quit, systemImage: "power") { NSApp.terminate(nil) }
        }
    }

    @ViewBuilder
    private var content: some View {
        switch peek {
        case .privateAI: privateAI
        case .traces: traces
        }
    }

    // MARK: Private AI

    /// Private AI, compact (owner, 2026-10-22): the NEAR AI tools, one
    /// button each, while signed in and a tool can be started (a button
    /// opens the Private AI tab at that tool's launch dialog, as the
    /// popover's do); Home's Credit and Balance tiles under them; and a
    /// small Private AI map at the foot.
    private var privateAI: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            if let snapshot = model.managedSnapshot, snapshot.nearAiSignedIn, MenuPanelStatus.showsToolButtons(snapshot) {
                HistoryFilterFlow(spacing: GlassTokens.Space.s3) {
                    ForEach(snapshot.signInStartTools, id: \.self) { tool in
                        ManagedToolButton(tool: tool) {
                            model.pendingManagedLaunch = tool
                            open(.inference)
                        }
                    }
                }
                .disabled(model.managedBusy)
            }
            HomeAccountTiles(store: data.home) { open(.inference) }
                // Over the map, so the Balance tile's coins fall over it.
                .zIndex(1)
            if let label = model.privateInferenceCopy?.destination {
                EdgeRailPrivateAIMap(data: data, label: label)
            }
        }
    }

    // MARK: Traces

    /// Traces, compact (owner, 2026-10-22): the watched directories as the
    /// tree draws them, every one, in a list that scrolls, each with its
    /// Submit split button and its "..." menu; then the shared and kept
    /// graph. The card of what is where and the watcher's switch are gone:
    /// Watching is in the header.
    private var traces: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            EdgeRailFolderList(store: data.traces, open: open)
            EdgeRailTracesGraph(data: data)
        }
    }

    // MARK: Shared

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
    /// The Private AI peek's map height.
    static let mapHeight: CGFloat = 190
}

/// Where a peek's Settings button opens Settings: Private AI's own section
/// from its peek (owner, 2026-10-22), General from Traces', as the
/// popover's opens.
enum EdgeRailSettings {
    static func section(_ peek: EdgeRailPeek) -> SettingsSection {
        peek == .privateAI ? .privateAI : .general
    }
}

/// The Traces peek's folder list's height: a row's height, and the list's
/// tallest, seven rows; more scroll.
enum EdgeRailFolderListHeights {
    static let rowPitch: CGFloat = GlassTokens.Size.listRow + 2
    static let maxHeight: CGFloat = 7 * rowPitch

    /// The list's height for `count` rows: all of them up to seven.
    static func height(rows count: Int) -> CGFloat {
        min(CGFloat(count) * rowPitch, maxHeight)
    }

    /// How many rows the list draws: every folder, and the sessions of the
    /// open ones.
    static func rows(_ folders: [TracesTree.FolderNode], expanded: Set<String>) -> Int {
        folders.reduce(0) { $0 + 1 + (expanded.contains($1.id) ? $1.sessions.count : 0) }
    }
}

/// The Private AI peek's map (owner, 2026-10-22): the window's Private AI
/// map (`PrivateAIFlowMap`), small, on a card.
private struct EdgeRailPrivateAIMap: View {
    let data: EdgeRailData
    let label: String
    @EnvironmentObject private var model: AppModel

    var body: some View {
        GeometryReader { proxy in
            PrivateAIFlowMap(
                traces: data.traces, inference: data.inference, label: label,
                sentence: { MonitorWindowView.rowSentence($0, copy: model.privateInferenceCopy, calls: model.harnessCalls) },
                space: FlowMapScene.space(for: proxy.size))
        }
        .frame(height: EdgeRailGraph.mapHeight)
        .glassTier(.card)
    }
}

/// The Traces peek's watched directories (owner, 2026-10-22): every one
/// the tree draws, as its rows draw them, in a list that scrolls, each with
/// its disclosure chevron opening its sessions under it, its Submit split
/// button and its "..." menu, which float over what is below as the tree's
/// do. Submit sends the folder at once, as the tree's does. A session's
/// Review, Submit all as, and a rule change that asks first open the
/// Traces tab, where their reviews and dialogs are: the rail holds none.
struct EdgeRailFolderList: View {
    let store: TracesStore
    let open: (MonitorDestination) -> Void
    @State private var rowMenu: TracesTreeView.RowMenu?
    /// The folders open to show their sessions; none at first.
    @State private var expanded: Set<String> = []

    /// Flat children let the lazy stack defer individual sessions, even
    /// when a folder contains most of the queue.
    static func drawnRows(_ folders: [TracesTree.FolderNode], expanded: Set<String>) -> [TracesTreeView.DrawnRow] {
        folders.flatMap { folder in
            [.folder(folder)] + (expanded.contains(folder.id)
                ? folder.sessions.map { .session($0, off: folder.mode == .ignore) } : [])
        }
    }

    var body: some View {
        let folders = store.tree.folders.filter { $0.mode != .ignore }
        if let words = store.words, !folders.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                Text(words.tree.treeTitle)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                    .accessibilityAddTraits(.isHeader)
                    .padding(.bottom, GlassTokens.Space.s2)
                ScrollView {
                    LazyVStack(spacing: 2) {
                        ForEach(Self.drawnRows(folders, expanded: expanded)) { drawn in
                            switch drawn {
                            case .folder(let folder): row(folder, words: words)
                            case .session(let entry, let off): sessionRow(entry, off: off, words: words)
                            }
                        }
                    }
                    .environment(\.glassTreeRoot, .folder)
                }
                .scrollIndicators(.automatic)
                .frame(height: EdgeRailFolderListHeights.height(rows: EdgeRailFolderListHeights.rows(folders, expanded: expanded)))
                // Over the list, not inside it, so a menu at its last row is
                // not cut off at the list's foot.
                .overlayPreferenceValue(TracesMenuAnchors.self, alignment: .topLeading) { anchors in
                    GeometryReader { space in menus(anchors, in: space, words: words) }
                }
            }
        }
    }

    /// A folder row as the tree's: its Submit, as the split button while
    /// the core's outcome words are read, and its "..." menu when it has a
    /// rule to change or can be ignored.
    @ViewBuilder
    private func row(_ folder: TracesTree.FolderNode, words: MonitorTracesCopy) -> some View {
        #if DEBUG
        let _ = OtherCollectionRowBodies.record("railFolders", folder.id)
        #endif
        let offer = store.groupOffer(folder)
        let submits = offer.offersContribute && store.mayContributeFolder(folder)
        let busy = store.submittingFolder == folder.id
        let switchable = folder.mode != nil && folder.offerableModes.contains(.ignore)
        let menu = (folder.mode != nil && !folder.offerableModes.isEmpty) || (switchable && folder.mode != .ignore)
        let outcome = store.disclosure?.outcome
        let split = submits && outcome != nil
        let title = busy ? words.tree.submitting : TracesTreeView.submitTitle(offer.count, words: words)
        let writing = store.writing.contains(folder.id)
        let submitTip = submits ? TracesTreeView.submitHelp(offer.withheldLine, words: words) : ""
        let floats = rowMenu == .folder(folder.id) || rowMenu == .submit(folder.id)
        GlassListRow(
            depth: .folder, tile: .folder, title: folder.label,
            sub: TracesTreeView.folderSub(folder, words: words, modeLabels: store.disclosure?.folderModeLabels ?? [:]),
            off: folder.mode == .ignore,
            expanded: folder.sessions.isEmpty ? nil : expanded.contains(folder.id),
            submitTitle: submits && !split ? title : nil,
            submitHelp: submitTip,
            accessory: split ? AnyView(
                GlassSplitButton(title, menuLabel: outcome?.submitAllAs ?? "", menuOpen: rowMenu == .submit(folder.id),
                                 onMenu: { rowMenu = TracesTreeView.toggled(.submit(folder.id), current: rowMenu) }) {
                    submit(folder)
                }
                .actionHelp(submitTip)
                .disabled(busy)) : nil,
            expandLabel: folder.label,
            menuLabel: menu ? words.tree.folderMenu : "",
            menuOpen: rowMenu == .folder(folder.id),
            onToggleExpand: { toggle(folder.id) },
            onSelect: { open(.traces(entryId: nil)) },
            onSubmit: submits && !busy ? { submit(folder) } : nil,
            onMenu: menu ? { rowMenu = TracesTreeView.toggled(.folder(folder.id), current: rowMenu) } : nil)
            // Its own height: the list's spare room is not the rows'.
            .fixedSize(horizontal: false, vertical: true)
            .disabled(writing)
            .opacity(writing ? TracesRowBusy.opacity : 1)
            .anchorPreference(key: TracesMenuAnchors.self, value: .bounds) {
                floats ? [TracesTreeView.rowID(.folder(projectID: folder.id)): $0] : [:]
            }
    }

    /// A session under its open folder, as the tree's row draws it: its
    /// tool, when, its state, and Submit, which sends it at once as the
    /// tree's does (owner, 2026-10-22). The row itself opens it in the
    /// Traces tab, where its review is. No "..." menu: Dismiss asks first,
    /// and the rail holds no dialog.
    private func sessionRow(_ entry: DaemonData.QueueEntry, off: Bool, words: MonitorTracesCopy) -> some View {
        #if DEBUG
        OtherCollectionRowBodies.record("railSessions", entry.entryId)
        #endif
        let lines = store.lines(entry)
        return GlassListRow(
            depth: .session, tile: TracesTreeView.sessionTile(entry), title: TracesTreeView.when(entry),
            sub: TracesTreeView.sub(entry, held: words.held, ineligible: lines.ineligible,
                                    attestation: lines.attestation, words: words),
            flag: TracesTreeView.flag(entry, ineligible: lines.ineligible != nil, attestation: lines.tone),
            off: off,
            submitTitle: words.tree.submit,
            // On the pill, which sends it; the row's click opens it.
            submitHelp: words.tree.submitTraceTip,
            onSelect: { open(.traces(entryId: entry.entryId)) },
            onSubmit: store.maySubmitSession(entry, folderIgnored: off)
                ? { Task { await store.submitSession(entry, folderIgnored: off) } } : nil)
            .fixedSize(horizontal: false, vertical: true)
            .disabled(store.acting.contains(entry.entryId))
            .opacity(store.acting.contains(entry.entryId) ? TracesRowBusy.opacity : 1)
    }

    /// Opens a folder to show its sessions, or closes it, at once: nothing
    /// in the rail's view animates a change of its size (#1517).
    private func toggle(_ id: String) {
        expanded.formSymmetricDifference([id])
    }

    /// The open menu, at its row: the folder's rule and Ignore, or Submit
    /// all as.
    @ViewBuilder
    private func menus(_ anchors: [String: Anchor<CGRect>], in space: GeometryProxy, words: MonitorTracesCopy) -> some View {
        if let menu = rowMenu, let id = TracesTreeView.folderID(of: menu),
           let anchor = anchors[TracesTreeView.rowID(.folder(projectID: id))],
           let folder = store.tree.folders.first(where: { $0.id == id }) {
            let rect = space[anchor]
            let ignorable = folder.mode != nil && folder.offerableModes.contains(.ignore) && folder.mode != .ignore
            Group {
                if menu == .folder(id) {
                    TracesFolderMenu(
                        folder: folder, ruleTitle: words.inspector.contributionRule,
                        ignoreLabel: ignorable ? words.tree.ignoreFolder : nil,
                        onRule: { mode in
                            rowMenu = nil
                            change(folder, mode)
                        },
                        onIgnore: {
                            rowMenu = nil
                            change(folder, .ignore)
                        },
                        onDismiss: { rowMenu = nil })
                } else if let outcome = store.disclosure?.outcome {
                    TracesSubmitMenu(
                        label: outcome.submitAllAs, tip: outcome.submitAllAsTooltip,
                        onPick: {
                            rowMenu = nil
                            open(.traces(entryId: nil))
                        },
                        onDismiss: { rowMenu = nil })
                        .offset(TracesSubmitMenu.offset)
                }
            }
            .frame(width: rect.width, height: rect.height, alignment: .topTrailing)
            .offset(x: rect.minX, y: rect.minY)
        }
    }

    /// A rule change: made here when it needs no confirmation, as the
    /// tree's is; one that asks first opens the Traces tab.
    private func change(_ folder: TracesTree.FolderNode, _ mode: ProjectMode) {
        switch TracesTreeView.modeChange(folder, mode) {
        case .noop, .unavailable: return
        case .apply: Task { await store.setFolderMode(folder, mode, promised: folder.sessions.count) }
        case .confirm: open(.traces(entryId: nil))
        }
    }

    private func submit(_ folder: TracesTree.FolderNode) {
        Task { await store.contributeFolder(folder, verdict: nil) }
    }
}
