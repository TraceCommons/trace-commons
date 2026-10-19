import SwiftUI
import TCShellCore

/// Feeds the window's transfer player what the core reported (owner,
/// 2026-10-16 and 2026-10-19): a row History gained is a contribution, the
/// window's last keep a keep, its last dismissal a dismissal. The player
/// is the window's, so the Traces map and the Traces tab show the same
/// trace (`MapTransferPlayer`).
struct MapTransferFeed: ViewModifier {
    @ObservedObject var player: MapTransferPlayer
    /// History's rows; nil while unread.
    let history: [DaemonData.HistoryRow]?
    let traces: TracesStore

    /// The traces the read before this one listed: a kept or dismissed
    /// trace is found there once the reload has dropped it.
    @State private var listedBefore: [DaemonData.QueueEntry] = []
    /// The Traces map's rocket and station, or the laptop and orb
    /// (`TracesMapStyle`, Settings, General).
    @AppStorage(TracesMapStyle.key) private var mapRocket = true

    private var listed: [DaemonData.QueueEntry] { traces.tree.allSessions + listedBefore }

    func body(content: Content) -> some View {
        content
            .onAppear { player.style = mapRocket ? .launch : .standard }
            .onChange(of: mapRocket) { _, rocket in player.style = rocket ? .launch : .standard }
            .onChange(of: history) { old, new in
                player.enqueue(MapTransferDetector.contributions(from: old, to: new))
            }
            .onChange(of: traces.tree) { old, _ in listedBefore = old.allSessions }
            .onChange(of: traces.lastKept) { _, kept in
                if let transfer = MapTransferDetector.kept(kept, in: listed) { player.enqueue([transfer]) }
            }
            .onChange(of: traces.lastDismissed) { _, dismissed in
                if let transfer = MapTransferDetector.dismissed(dismissed, in: listed) { player.announce(transfer) }
            }
            #if DEBUG
            .task { await demo() }
            #endif
    }

    #if DEBUG
    /// A dry-run review aid (owner, 2026-10-16): with
    /// `TRACE_COMMONS_MAP_DEMO=1` the window plays a run every few seconds
    /// from the folders the map draws -- two contributions, a keep, then a
    /// dismissal -- so the motion and the cards can be looked at without
    /// anything leaving this Mac. Nothing is sent, kept or dismissed.
    private func demo() async {
        guard ProcessInfo.processInfo.environment["TRACE_COMMONS_MAP_DEMO"] == "1" else { return }
        var round = 0
        while !Task.isCancelled {
            let tree = traces.tree
            let pairs = FlowMapScene.directories(tree).prefix(FlowMapScene.foldersShown).compactMap { folder in
                FlowMapScene.carriers(of: folder, in: tree).first.map { (folder: folder, tool: $0.id) }
            }
            if !pairs.isEmpty {
                let action: MapTransfer.Action = switch round % 4 {
                case 2: .keep
                case 3: .dismiss
                default: .contribute
                }
                let picked = action == .contribute ? Array(pairs.shuffled().prefix(3)) : [pairs[round % pairs.count]]
                let transfers = picked.map {
                    MapTransfer(id: "demo-\(round)-\($0.folder.id)", folder: $0.folder.id, tool: $0.tool,
                                action: action, at: .now, folderLabel: $0.folder.label)
                }
                if action == .dismiss, let first = transfers.first {
                    player.announce(first)
                } else {
                    player.enqueue(transfers)
                }
                round += 1
            }
            try? await Task.sleep(for: .seconds(12))
        }
    }
    #endif
}
