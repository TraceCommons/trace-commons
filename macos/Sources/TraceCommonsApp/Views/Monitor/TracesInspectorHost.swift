#if DEBUG
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The inspector on Home, Traces and History (Ron's #1146: the Monitor
/// mounts `WaitingPrompts` and then `WaitingPage` in the inspector for every
/// view but Inference, which gets `WaitingPrompts` and its own inspector). Top to bottom: the core's health banners, always,
/// whatever is selected; the prompts (`InspectorPrompts`); then the
/// selection's inspector.
///
/// The selection is read only through `TracesStore.selectedSession`, so a
/// session that has gone (uploaded, expired, dismissed elsewhere) is the
/// Summary, never a stale card. A folder is read only through
/// `TracesStore.selectedFolder` and shows Ron's Folder inspector
/// (`FolderInspector`); one that has gone is the Summary too. On History it
/// keeps the Traces selection's card: History's opened row is drawn in
/// History's left pane, never here.
struct TracesInspectorHost: View {
    @EnvironmentObject private var model: AppModel
    let traces: TracesStore
    let home: HomeStore
    let selection: MonitorSelection?

    /// What the inspector shows below the prompts.
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
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            // Why approved sessions are not moving, or that the core is not
            // answering, above everything the inspector shows. An unread
            // status is never drawn as healthy.
            ForEach(TracesHealth.banners(
                phase: traces.phase, status: traces.status, words: traces.words, coreDown: TracesHealth.coreDownLine,
                maxQueueEntries: model.daemonSettings?.maxQueueEntries)
            ) { GlassHealthBanner(banner: $0) }
            InspectorPrompts(store: traces)
            switch shown {
            case .session(let entry):
                SessionReviewCard(store: traces, entry: entry)
            case .folder(let folder):
                FolderInspector(store: traces, folder: folder)
            case .summary:
                summary
            }
        }
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

/// What the inspector must show has appeared (Ron's `useInspectorDemand`,
/// `monitor-shell.tsx:98-107`), read from the app and the Traces store:
/// the shell's keys (`InspectorDemand.keys(approvalUndo:...)`: an undo, a
/// folder's Submit all in flight, the arming offer, the Private AI offer)
/// and two of the port's. A key that was not there before opens the
/// inspector, and a key going away closes nothing.
extension InspectorDemand {
    @MainActor
    static func keys(model: AppModel, traces: TracesStore, selection: MonitorSelection?) -> Set<String> {
        var keys = keys(
            approvalUndo: model.undo?.entryIDs, contributed: traces.lastContributed?.entryId, kept: traces.lastKept,
            contributedFolder: traces.lastContributedFolder?.projectId, submittingFolder: traces.submittingFolder,
            privateAIOffer: model.showsPrivateInferenceOffer, armingOffer: model.armingOffer?.projectId)
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

    /// Whether the keys there when the window appears open the inspector:
    /// any at all, as Ron's effect runs on mount with nothing seen before.
    /// A closed inspector restored from the last session, or seeded closed
    /// by a narrow first layout, does not hide an undo made while the
    /// Monitor was closed.
    static func opensOnAppear(keys: Set<String>) -> Bool {
        opens(previous: [], current: keys)
    }
}
#endif
