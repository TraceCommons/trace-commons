import Foundation
import Observation
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
    /// The last `status` read; nil when it has not been read or failed. The
    /// badge is its `decisions_owed`, never `queue_depth`.
    private(set) var status: DaemonData.Status?
    /// A review action (R7) in flight, by entry id.
    private(set) var acting: Set<String> = []
    /// The session last kept on this Mac, while its undo is offered.
    private(set) var lastKept: String?
    /// The last review action the core refused, by entry id.
    private(set) var actionError: (entryId: String, error: DaemonDataError)?

    /// The Traces badge: decisions owed, unknown (a dash) when the core did
    /// not say, never a count derived from the queue.
    var decisionsOwed: Int? { status?.decisionsOwed }

    let client: any DaemonDataClient
    private var events: Task<Void, Never>?

    init(client: any DaemonDataClient) {
        self.client = client
    }

    /// Loads once and follows the event stream until `stop()`.
    func start() {
        guard events == nil else { return }
        let stream = client.events()
        events = Task { [weak self] in
            await self?.load()
            for await event in stream {
                switch event {
                case .snapshot, .queueChanged, .statusChanged, .resyncRequired:
                    await self?.load()
                case .digestDue, .previewReady, .inferenceCallAdded, .unknown:
                    break
                }
            }
        }
    }

    func stop() {
        events?.cancel()
        events = nil
    }

    func load() async {
        do {
            async let entries = client.listPending(projectId: nil)
            async let projects = client.listProjects()
            // Settings only decide the tool switches. Unreadable settings
            // leave every tool unset, which draws no switch: never off.
            async let settings = try? client.settings()
            async let status = try? client.status()
            tree = TracesTree.build(entries: try await entries, projects: try await projects.projects, settings: await settings)
            self.status = await status
            phase = .loaded
        } catch let error as DaemonDataError {
            phase = .failed(error)
            status = nil
        } catch {
            phase = .failed(.undecodable(method: "list_pending"))
            status = nil
        }
    }

    // MARK: Review (R7)

    enum ReviewAction: Equatable {
        case contribute, keep, undoKeep, dismiss
    }

    /// One review action on one session, then a reload. Nothing is applied
    /// optimistically: the tree redraws from the core's answer.
    func perform(_ action: ReviewAction, on entryId: String) async {
        guard !acting.contains(entryId) else { return }
        acting.insert(entryId)
        defer { acting.remove(entryId) }
        actionError = nil
        do {
            switch action {
            case .contribute:
                _ = try await client.approve(entryId: entryId)
            case .keep:
                _ = try await client.keep(entryId: entryId)
                lastKept = entryId
            case .undoKeep:
                _ = try await client.undoKeep(entryId: entryId)
                lastKept = nil
            case .dismiss:
                try await client.dismiss(entryId: entryId)
            }
        } catch {
            actionError = (entryId, error as? DaemonDataError ?? .undecodable(method: "\(action)"))
            return
        }
        await load()
    }

    /// The folder switch: on is "Ask me first", off is "Never offer this
    /// one". Turning a folder on never arms it.
    func setFolderOffered(_ folderId: String, _ offered: Bool) async {
        guard !writing.contains(folderId) else { return }
        writing.insert(folderId)
        defer { writing.remove(folderId) }
        do {
            _ = try await client.setProjectMode(projectId: folderId, mode: offered ? .ask : .ignore, includeBacklog: nil)
        } catch {
            // The switch redraws from the core's answer, so a refused write
            // shows the folder as it still is; the error is kept.
            phase = .failed(error as? DaemonDataError ?? .undecodable(method: "set_project_mode"))
            return
        }
        await load()
    }
}
