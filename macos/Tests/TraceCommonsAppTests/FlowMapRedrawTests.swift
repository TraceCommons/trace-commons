import AppKit
import SwiftUI
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// What the Traces map draws again, counted rather than timed (owner,
/// 2026-11-20: measure, then fix). A tick of its timelines redraws the
/// arcs and the library and nothing else; a folder's Automatic switch
/// redraws its own row and the map once, never the whole tree. Loose on
/// purpose: they fail on a map that draws everything each frame, or a
/// press that rebuilds every row, never on a draw or two more than today.
@MainActor
final class FlowMapRedrawTests: XCTestCase {
    typealias Perf = TracesScalePerfTests

    private func drain(_ hosts: [Perf.Host], _ seconds: Double) {
        let end = Date().addingTimeInterval(seconds)
        while Date() < end {
            RunLoop.main.run(mode: .default, before: Date().addingTimeInterval(0.01))
            hosts.forEach { $0.draw() }
        }
    }

    /// The map as `MonitorMapPane` builds it for the Traces tab
    /// (MonitorWindowView.swift, `mapContent`): the scene made in the body
    /// from the store's tree, phase, status and destinations, so a change
    /// to any of them reaches the map as the window's does.
    struct WindowMap: View {
        let store: TracesStore
        var body: some View {
            GeometryReader { proxy in
                let space = FlowMapScene.space(for: proxy.size)
                let scene = FlowMapScene.traces(
                    store.tree,
                    gate: .init(state: state, status: store.status, destinations: store.destinations),
                    contributed: .init(history: nil, failure: nil), selectedTool: nil, revealed: [:], space: space)
                FlowMapView(scene: scene, legend: [.autoUpload, .ask, .ignore], zoomable: true, accessibilityName: "map",
                            state: state)
            }
        }

        private var state: ScreenState {
            var failure: DaemonDataError?
            if case .failed(let error) = store.phase { failure = error }
            return ScreenState.resolve(
                failure: failure, loaded: store.phase != .loading,
                paused: store.status?.paused, known: store.status != nil)
        }
    }

    /// The recorded armed-folder day: live, healthy flows and the library,
    /// so both timelines run. Over three seconds the dashes draw at about
    /// 30 a second and the library at about 15; the map's body, its still
    /// layer (nodes, names, logos) and its node controls are not built
    /// again at all.
    func test_aTickRedrawsOnlyTheArcsAndTheLibrary() async throws {
        let client = SampleDaemonClient(.armedFolder)
        let store = TracesStore(client: client)
        await store.load()
        let host = Perf.Host(WindowMap(store: store))
        defer { host.close() }
        host.draw()
        FlowMapBodies.reset()
        drain([host], 0.5)
        // The timelines of a window never put on screen ticked on a
        // developer Mac but not on the CI runner (2026-11-20): with no tick
        // at all there is nothing to count, so the test says so instead of
        // passing on zeros.
        if FlowMapBodies.arcs == 0 && FlowMapBodies.library == 0 {
            throw XCTSkip("the map's timelines did not tick in this environment")
        }
        let scene = FlowMapScene.traces(
            store.tree, gate: .init(state: .ready, status: store.status, destinations: store.destinations))
        XCTAssertTrue(scene.flows, "the armed-folder day has a live flow")

        FlowMapBodies.reset()
        drain([host], 3)
        print("COUNT tick maps=\(FlowMapBodies.maps) arcs=\(FlowMapBodies.arcs) library=\(FlowMapBodies.library) still=\(FlowMapBodies.still) targets=\(FlowMapBodies.targets)")
        // The positive control: both timelines ran.
        XCTAssertGreaterThan(FlowMapBodies.arcs, 30, "the dashes did not run")
        XCTAssertGreaterThan(FlowMapBodies.library, 15, "the library did not turn")
        XCTAssertLessThan(FlowMapBodies.library, FlowMapBodies.arcs, "the library ran at the dashes' rate")
        XCTAssertEqual(FlowMapBodies.maps, 0, "a tick rebuilt the map")
        XCTAssertEqual(FlowMapBodies.still, 0, "a tick redrew the nodes, names and logos")
        XCTAssertEqual(FlowMapBodies.targets, 0, "a tick rebuilt the node controls")
    }

    /// A folder's Automatic switch at the owner's scale (231 folders): the
    /// press (`TracesStore.setFolderMode`, then its reload) rebuilds the
    /// folder's own row, not the tree, and the map, whose scene it changes,
    /// a few times, not once per folder.
    func test_anAutomaticSwitchRebuildsItsRowAndTheMapOnce() async throws {
        let client = try Perf.client()
        let store = TracesStore(client: client)
        await store.load()
        XCTAssertEqual(store.tree.folders.count, 231)
        let tree = Perf.Host(Perf.Hosted(store: store, box: Perf.SelectionBox(), model: AppModel()))
        let map = Perf.Host(WindowMap(store: store))
        defer {
            tree.close()
            map.close()
        }
        tree.draw()
        map.draw()
        drain([tree, map], 0.4)

        var rows: [Int] = []
        var maps: [Int] = []
        var stills: [Int] = []
        // The daemon's projects, each press's folder left Automatic.
        var object = try XCTUnwrap(
            JSONSerialization.jsonObject(with: Data(try Perf.replies().projects.utf8)) as? [String: Any])
        for index in [1, 2, 3] {
            let folder = store.tree.folders[index]
            XCTAssertEqual(folder.mode, .ask)
            var projects = try XCTUnwrap(object["projects"] as? [[String: Any]])
            for i in projects.indices where projects[i]["project_id"] as? String == folder.id {
                projects[i]["mode"] = "auto_upload"
                projects[i]["folder_mode"] = "auto_upload"
                projects[i]["automatic_disclosure"] = "standard"
            }
            object["projects"] = projects
            client.replaceReply(
                "list_projects", with: String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self))

            TracesRowBodies.reset()
            FlowMapBodies.reset()
            await store.setFolderMode(folder, .autoUpload, promised: folder.sessions.count)
            drain([tree, map], 0.4)
            XCTAssertEqual(store.tree.folders.first { $0.id == folder.id }?.mode, .autoUpload)
            print("COUNT switch rows=\(TracesRowBodies.total) built=\(TracesRowBodies.built.sorted()) maps=\(FlowMapBodies.maps) still=\(FlowMapBodies.still) targets=\(FlowMapBodies.targets)")
            XCTAssertEqual(
                TracesRowBodies.built, [TracesTreeView.rowID(.folder(projectID: folder.id))],
                "a switch redrew another folder's row, or not its own")
            rows.append(TracesRowBodies.total)
            maps.append(FlowMapBodies.maps)
            stills.append(FlowMapBodies.still)
        }
        XCTAssertLessThanOrEqual(rows.max() ?? 0, 6, "a switch rebuilt rows it did not change: \(rows)")
        XCTAssertGreaterThanOrEqual(maps.min() ?? 0, 1, "the map did not follow the switch: \(maps)")
        XCTAssertLessThanOrEqual(maps.max() ?? 0, 4, "a switch rebuilt the map again and again: \(maps)")
        XCTAssertLessThanOrEqual(stills.max() ?? 0, 4, "a switch redrew the map's nodes again and again: \(stills)")
    }
}
