import Foundation
import Observation
import TCBridge
import TCShellCore

/// The Traces tab's data (R6 of #1173), read through `DaemonDataClient`.
///
/// It reloads on every event that can change the tree. The tree is redrawn
/// only from a successful read: a failed refresh keeps the last tree and
/// says the core is not answering, rather than drawing an empty one.
@MainActor
@Observable
final class TracesStore {
    enum Phase: Equatable {
        case loading
        case loaded
        case failed(DaemonDataError)
    }

    private(set) var phase: Phase = .loading
    private(set) var tree = TracesTree(tools: [], unplaced: [])
    /// The `set_project_mode` write in flight, by folder id.
    private(set) var writing: Set<String> = []

    let client: any DaemonDataClient
    /// The last folder change whose result differed from what the
    /// confirmation promised, in the core's words
    /// (`tc_project_ignore_reconciled_text`).
    private(set) var folderNotice: String?
    /// Each load's number; a load that finishes after a newer one started is
    /// dropped, so an older read never overwrites a newer tree.
    private var generation = 0

    init(client: any DaemonDataClient) {
        self.client = client
    }

    /// The tools the core reads from their usual folder while unset, from
    /// its source copy. Empty if the copy is unavailable: then only tools
    /// with a declaration or something waiting are drawn.
    static let scansWhenUnset: Set<SourceKind> = {
        guard let copy = TCSourceChecks.settingsCopy() else { return [] }
        return Set(SourceKind.allCases.filter { copy.tools[$0.rawValue]?.unsetScansConventional == true })
    }()

    /// A tool row's sub-line: the core's sentence for its declaration, or
    /// its "could not be confirmed" sentence when settings were unreadable.
    static func sourceLine(_ tool: TracesTree.ToolNode) -> String? {
        guard let copy = TCSourceChecks.settingsCopy(), let entry = copy.tools[tool.kind.rawValue] else { return nil }
        guard let wire = tool.mode.wire else { return copy.unavailable }
        return TCSourceChecks.checkLine(tool: entry.key, sourceMode: wire)
    }

    /// Loads, then follows the event stream for as long as the calling task
    /// runs. Call it from a view's `.task`: when the view goes, the task is
    /// cancelled and the stream with it.
    func run() async {
        await load()
        for await event in client.events() {
            if Task.isCancelled { break }
            switch event {
            case .snapshot, .queueChanged, .statusChanged, .resyncRequired:
                await load()
            case .digestDue, .previewReady, .inferenceCallAdded, .unknown:
                break
            }
        }
    }

    func load() async {
        generation += 1
        let mine = generation
        do {
            async let entries = client.listPending(projectId: nil)
            async let projects = client.listProjects()
            // Settings only decide the tool switches. Unreadable settings
            // are unknown: no switch, never off, and the row says so.
            async let settings = try? client.settings()
            let built = TracesTree.build(
                entries: try await entries, projects: try await projects.projects, settings: await settings,
                scansWhenUnset: Self.scansWhenUnset)
            guard mine == generation else { return }
            tree = built
            phase = .loaded
        } catch let error as DaemonDataError {
            phase = .failed(error)
        } catch {
            phase = .failed(.undecodable(method: "list_pending"))
        }
    }

    /// A folder's mode, chosen from its three-way picker after any
    /// confirmation the view showed (arming, or ignoring a folder with
    /// sessions waiting). `promised` is the waiting count that confirmation
    /// named; the core's `purged` is the authority, and a difference is said.
    func setFolderMode(_ folder: TracesTree.FolderNode, _ mode: ProjectMode, promised: Int) async {
        guard !writing.contains(folder.id) else { return }
        writing.insert(folder.id)
        defer { writing.remove(folder.id) }
        folderNotice = nil
        do {
            let result = try await client.setProjectMode(projectId: folder.id, mode: mode, includeBacklog: nil)
            if mode == .ignore {
                folderNotice = TCCoreCopy.projectIgnoreReconciled(
                    project: folder.label, promised: promised, purged: result.purged ?? promised)
            }
        } catch {
            // The picker redraws from the core's answer, so a refused write
            // shows the folder as it still is; the error is kept.
            phase = .failed(error as? DaemonDataError ?? .undecodable(method: "set_project_mode"))
            return
        }
        await load()
    }
}
