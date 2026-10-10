import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The inspector on Home, Traces and History (Ron's #1146 `WaitingPage`):
/// the selection's inspector, and nothing above it. The health banners and
/// the prompts (the undos, the offers, the first-contribution note) are
/// drawn above the Traces tree on Traces, an accepted difference from #1146
/// (owner, 2026-10-07), and above this host by the window on Home and
/// History (`MonitorWindowView.promptsInInspector`), so the card's
/// Contribute is never offered without its Undo or the core's health.
///
/// The selection is read only through `TracesStore.selectedSession`, so a
/// session that has gone (uploaded, expired, dismissed elsewhere) is the
/// Summary, never a stale card. A folder is read only through
/// `TracesStore.selectedFolder` and shows Ron's Folder inspector
/// (`FolderInspector`); one that has gone is the Summary too. On History,
/// with no row open, it keeps the Traces selection's card; an opened
/// History row is the window's own inspector arm (`HistoryInspectorPane`,
/// V8), never this host.
struct TracesInspectorHost: View {
    @EnvironmentObject private var model: AppModel
    let traces: TracesStore
    let home: HomeStore
    let selection: MonitorSelection?

    /// What the inspector shows.
    private enum Shown {
        case session(DaemonData.QueueEntry)
        case folder(TracesTree.FolderNode)
        case summary
    }

    private var shown: Shown {
        if let entry = traces.selectedSession(selection) { return .session(entry) }
        if let folder = traces.selectedFolder(selection) { return .folder(folder) }
        return .summary
    }

    var body: some View {
        switch shown {
        case .session(let entry):
            SessionReviewCard(store: traces, entry: entry)
        case .folder(let folder):
            FolderInspector(store: traces, folder: folder, history: history)
        case .summary:
            summary
        }
    }

    /// History as of its last good read, for a folder's shared count: the
    /// store keeps the last good page on a failure, which is not current.
    private var history: [DaemonData.HistoryRow]? {
        SummaryFacts.fresh(home.history, unless: home.failures["list_history"])
    }

    /// The Summary (Ron's `SummaryInspector`): what is waiting, what went,
    /// the safeguards, the certificates held and why sessions stopped
    /// waiting.
    private var summary: some View {
        SummaryInspector(
            traces: traces, home: home, awaitingDecision: model.awaitingDecision,
            queueAnswered: model.queueAnswered, outcomeCounts: model.outcomeCounts)
    }
}

/// The top of the inspector on Home and History, above the host
/// (`MonitorWindowView.promptsInInspector`): the core's health banners,
/// then the prompts, as on Traces they head the tree. Those tabs still
/// offer the Traces selection's Contribute, so its Undo, the offers and a
/// core that is down are said beside it.
struct InspectorPromptsHeader: View {
    @EnvironmentObject private var model: AppModel
    let traces: TracesStore

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            // Why approved sessions are not moving, or that the core is not
            // answering, first: an unread status is never drawn as healthy.
            ForEach(TracesHealth.banners(
                phase: traces.phase, status: traces.status, words: traces.words, coreDown: TracesHealth.coreDownLine,
                maxQueueEntries: model.daemonSettings?.maxQueueEntries)
            ) { GlassHealthBanner(banner: $0) }
            InspectorPrompts(store: traces)
        }
    }
}

/// Ron's `InspectorHeader` (`waiting-page.tsx:60-76`): a large tile, the
/// title at 17/700 on one line, and a tertiary sub line. Every inspector on
/// Traces opens with it: the Summary, a folder and a session.
struct InspectorHeader: View {
    var tile: GlassToolTile.Kind? = nil
    let title: String
    let sub: String?

    var body: some View {
        HStack(spacing: GlassTokens.Space.s5) {
            if let tile { GlassToolTile(tile, large: true) }
            VStack(alignment: .leading, spacing: 0) {
                Text(title)
                    .glassType(GlassTokens.TypeScale.heading.weight(.bold))
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                if let sub, !sub.isEmpty {
                    Text(sub)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

/// Ron's inspector `Section` (`waiting-page.tsx:78-97`): an expander
/// heading over bare content, no card, 6 between them and the content
/// inset 4. A collapsible one opens and closes; the others always show
/// their content under an open heading that is not a control.
struct InspectorSection<Content: View>: View {
    let title: String
    let collapsible: Bool
    let content: Content
    @State private var open = true

    init(_ title: String, collapsible: Bool = false, @ViewBuilder content: () -> Content) {
        self.title = title
        self.collapsible = collapsible
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            if collapsible {
                GlassExpander(title, isOpen: $open)
            } else {
                GlassExpander(title, isOpen: .constant(true))
                    .allowsHitTesting(false)
                    .accessibilityRemoveTraits(.isButton)
                    .accessibilityAddTraits(.isHeader)
            }
            if open || !collapsible {
                content
                    .padding(.horizontal, GlassTokens.Space.s2)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
    }
}

/// What the inspector must show has appeared (Ron's `useInspectorDemand`,
/// `monitor-shell.tsx:98-107`), read from the app and the Traces store:
/// the shell's keys (`InspectorDemand.keys(approvalUndo:...)`: an undo, a
/// folder's Submit all in flight) and two of the port's. A key that was not
/// there before opens the inspector, and a key going away closes nothing.
/// The offers (`offerKeys`) are a demand only off the Traces tab, where the
/// inspector draws the prompts; beside the tree they are already shown.
extension InspectorDemand {
    @MainActor
    static func keys(model: AppModel, traces: TracesStore, selection: MonitorSelection?) -> Set<String> {
        var keys = keys(
            approvalUndo: model.undo?.entryIDs, contributed: traces.lastContributed?.entryId, kept: traces.lastKept,
            contributedFolder: traces.lastContributedFolder?.projectId, submittingFolder: traces.submittingFolder)
        if let undo = model.undo {
            // The approval's time, not its ticking count: a second approval
            // of the same session is a new undo, a tick is not.
            keys.insert("undo:approval-at:\(undo.approvedAt.timeIntervalSince1970)")
        }
        // A selected session: its card lives in the inspector. A folder is
        // not a demand.
        if let entry = traces.selectedSession(selection) {
            keys.insert("review:\(entry.entryId)")
        }
        return keys
    }

    /// The offers' keys (Ron's `offer:` demands), which the window adds
    /// only where the inspector draws the prompts
    /// (`MonitorWindowView.promptsInInspector`): beside the Traces tree an
    /// offer is already on screen.
    @MainActor
    static func offerKeys(model: AppModel) -> Set<String> {
        var keys: Set<String> = []
        if model.showsPrivateInferenceOffer { keys.insert("offer:private-ai") }
        if let project = model.armingOffer?.projectId { keys.insert("offer:arming:" + project) }
        return keys
    }

    /// Whether the keys there when the window appears open the inspector:
    /// any at all, as Ron's effect runs on mount with nothing seen before.
    /// A closed inspector restored from the last session, or seeded closed
    /// by a narrow first layout, does not hide an undo made while the
    /// Monitor was closed.
    static func opensOnAppear(keys: Set<String>) -> Bool {
        opens(previous: [], current: keys)
    }
}
