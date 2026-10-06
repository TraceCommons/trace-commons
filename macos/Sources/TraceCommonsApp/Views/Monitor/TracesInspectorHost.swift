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
/// Summary, never a stale card. A folder shows the Summary until it has an
/// inspector of its own.
struct TracesInspectorHost: View {
    @EnvironmentObject private var model: AppModel
    let traces: TracesStore
    let home: HomeStore
    let selection: MonitorSelection?
    /// The History row selected on History, while it is still listed; nil
    /// elsewhere. Until History's detail moves into the left pane (Task 8
    /// of the #1146 port) its details are drawn here, under the banners and
    /// the prompts like everything else the inspector shows.
    var historyRow: DaemonData.HistoryRow? = nil

    /// What the inspector shows below the prompts.
    private enum Shown {
        case history(DaemonData.HistoryRow)
        case session(DaemonData.QueueEntry)
        case summary
    }

    private var shown: Shown {
        if let historyRow { return .history(historyRow) }
        if let entry = traces.selectedSession(selection) { return .session(entry) }
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
            case .history(let row):
                HistoryDetailInspector(row: row)
            case .session(let entry):
                SessionInspectorView(store: traces, entry: entry)
            case .summary:
                summary
            }
        }
    }

    /// The Summary: the certificates held, why sessions stopped waiting,
    /// and the record as a whole.
    private var summary: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            // An unread queue is not an empty one: before the daemon has
            // answered, nothing (`CertificateSection` says when there are
            // none).
            if model.queueAnswered {
                CertificateSection(entries: model.awaitingDecision)
            }
            NotOfferedGlassDisclosure(counts: model.outcomeCounts)
            HomeSummaryInspector(store: home)
        }
    }
}

/// What the inspector must show has appeared (Ron's `useInspectorDemand`,
/// `monitor-shell.tsx:98-107`): an undo, a selected session, a folder's
/// Submit all in flight, the arming offer, the Private AI offer. Each is a
/// key; a key that was not there before opens the inspector, and a key
/// going away closes nothing.
@MainActor
enum InspectorDemand {
    static func keys(model: AppModel, traces: TracesStore, selection: MonitorSelection?) -> Set<String> {
        var keys: Set<String> = []
        if let undo = model.undo {
            // The approval's time, not its ticking count: a second approval
            // of the same session is a new undo, a tick is not.
            keys.insert("undo:approval:\(undo.entryIDs.joined(separator: ","))@\(undo.approvedAt.timeIntervalSince1970)")
        }
        if let contributed = traces.lastContributed {
            keys.insert("undo:session:\(contributed.entryId)")
        }
        if let folder = traces.lastContributedFolder {
            keys.insert("undo:folder:\(folder.projectId)")
        }
        if let kept = traces.lastKept {
            keys.insert("undo:keep:\(kept)")
        }
        if let entry = traces.selectedSession(selection) {
            keys.insert("review:\(entry.entryId)")
        }
        if let folder = traces.submittingFolder {
            keys.insert("submit:\(folder)")
        }
        if let offer = model.armingOffer {
            keys.insert("arming:\(offer.projectId)")
        }
        if model.showsPrivateInferenceOffer {
            keys.insert("private-ai-offer")
        }
        return keys
    }

    /// Whether `current` holds a key `previous` did not. Never a reason to
    /// close: the inspector closes only on the person's toggle.
    static func opens(previous: Set<String>, current: Set<String>) -> Bool {
        !current.subtracting(previous).isEmpty
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
