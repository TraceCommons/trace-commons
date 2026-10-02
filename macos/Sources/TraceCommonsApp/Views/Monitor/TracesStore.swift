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
            tree = TracesTree.build(entries: try await entries, projects: try await projects.projects, settings: await settings)
            phase = .loaded
        } catch let error as DaemonDataError {
            phase = .failed(error)
        } catch {
            phase = .failed(.undecodable(method: "list_pending"))
        }
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
