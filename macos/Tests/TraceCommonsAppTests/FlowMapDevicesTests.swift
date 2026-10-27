import SwiftUI
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The other devices linked to this account (#1668, #1669) on the Traces
/// map: each a small computer of its own beside this one, joined to the
/// commons by a quiet line, since nothing here says what they send.
@MainActor
final class FlowMapDevicesTests: XCTestCase {
    private func traces() -> FlowMapScene {
        let tree = TracesTree(tools: [.init(kind: .claudeCode, mode: .watch, folders: [])], unplaced: [])
        return FlowMapScene.traces(tree, gate: .init(state: .ready))
    }

    private func device(_ n: Int) -> FlowMapScene.OtherDevice {
        .init(id: "sha256:\(n)", name: "server-\(n)", detail: "Last seen \(n) minutes ago")
    }

    func test_eachOtherDeviceIsACompanionComputerJoinedQuietlyToTheCommons() throws {
        let base = traces()
        let scene = base.addingDevices([device(1), device(2)])
        let devices = scene.nodes.filter { $0.kind == .device }
        XCTAssertEqual(devices.map(\.id), ["device:sha256:1", "device:sha256:2"])
        XCTAssertEqual(devices.map(\.label), ["server-1", "server-2"])
        XCTAssertEqual(devices.map(\.detail), ["Last seen 1 minutes ago", "Last seen 2 minutes ago"])
        let hub = try XCTUnwrap(scene.nodes.first { $0.id == "hub" })
        let library = try XCTUnwrap(scene.nodes.first { $0.id == "library" })
        for node in devices {
            XCTAssertGreaterThan(node.at.x, hub.at.x, "beside this computer, to its right")
            XCTAssertGreaterThan(node.at.y, library.at.y, "under the commons")
            XCTAssertLessThanOrEqual(node.at.x + node.radius, scene.space.width, "inside the map")
            XCTAssertLessThanOrEqual(node.at.y, FlowMapScene.band(scene.space).upperBound)
        }
        // One quiet line from each device to the commons: nothing says what
        // another device sends, so nothing moves or is dashed.
        let added = scene.arcs.dropFirst(base.arcs.count)
        XCTAssertEqual(added.count, 2)
        XCTAssertTrue(added.allSatisfy { $0.style == .quiet })
        XCTAssertTrue(added.allSatisfy { $0.to == library.at })
        XCTAssertEqual(scene.flows, base.flows, "another device never starts the map moving")
        XCTAssertEqual(scene.nodes.filter { $0.kind != .device }, base.nodes, "this computer's nodes are untouched")
    }

    func test_pastFourDevicesTheRestAreCounted() {
        let scene = traces().addingDevices((1...6).map(device))
        XCTAssertEqual(scene.nodes.filter { $0.kind == .device }.count, FlowMapScene.devicesShown)
        XCTAssertEqual(FlowMapScene.devicesShown, 4)
        XCTAssertEqual(scene.overflows.map(\.count), [2])
    }

    func test_noDevicesOrNoCommonsChangesNothing() {
        let base = traces()
        XCTAssertEqual(base.addingDevices([]), base)
        // The Private AI map has no commons to join them to.
        var privateAI = FlowMapScene()
        privateAI.nodes = [.init(id: "destination", kind: .destination(.running), at: .zero, radius: 26,
                                 label: "x", detail: "y")]
        XCTAssertEqual(privateAI.addingDevices([device(1)]), privateAI)
    }

    /// From the Devices section's list: this Mac left out (it is the
    /// computer already drawn), the section's own name for each, and its
    /// last-seen line, a dash when the server has not seen it.
    func test_theOtherDevicesAreTheDevicesListWithoutThisOne() {
        let listed = [
            AccountDevice(deviceKeyId: "sha256:me", label: "laptop", lastSeenAt: "t0", thisDevice: true),
            AccountDevice(deviceKeyId: "sha256:srv", label: "build-box", lastSeenAt: "t1"),
            AccountDevice(deviceKeyId: "sha256:new", label: nil, lastSeenAt: nil),
        ]
        let others = FlowMapScene.otherDevices(
            listed, name: { $0.label ?? "(no name)" }, lastSeen: { $0.lastSeenAt.map { "seen \($0)" } })
        XCTAssertEqual(others, [
            .init(id: "sha256:srv", name: "build-box", detail: "seen t1"),
            .init(id: "sha256:new", name: "(no name)", detail: "—"),
        ])
        XCTAssertEqual(FlowMapScene.otherDevices(nil, name: { _ in "" }, lastSeen: { _ in nil }), [],
                       "an unread list draws no devices, never a guess")
    }

    /// Renders the Traces map with three other devices, in both styles, so
    /// the layout can be looked at (kept under TRACE_COMMONS_SCREENSHOT_DIR
    /// when set, as PrivateAIRenderTests does).
    func test_theMapDrawsTheOtherDevices() async throws {
        let client = SampleDaemonClient(.normalDay)
        let tree = TracesTree.build(
            entries: try await client.listPending(projectId: nil),
            projects: try await client.listProjects().projects,
            settings: try? await client.settings(),
            scansWhenUnset: TracesTreeTests.scansWhenUnset)
        let size = CGSize(width: 420, height: 640)
        let space = FlowMapScene.space(for: size)
        let scene = FlowMapScene.traces(tree, gate: .init(state: .ready), space: space)
            .addingDevices([
                .init(id: "a", name: "build-box", detail: "Last seen 5 minutes ago"),
                .init(id: "b", name: "zaki-air", detail: "Last seen 2 days ago"),
                .init(id: "c", name: "ci-runner-07", detail: "—"),
            ])
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:))
            ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer {
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil {
                try? FileManager.default.removeItem(at: directory)
            }
        }
        for style in [TracesMapStyle.launch, .standard] {
            try render(
                FlowMapView(scene: scene, legend: [.autoUpload, .ask, .ignore], zoomable: false,
                            accessibilityName: "map", mapStyle: style),
                size: size, to: directory.appendingPathComponent("map-devices-\(style).png"))
        }
    }

    private func render<V: View>(_ view: V, size: CGSize, to url: URL) throws {
        _ = NSApplication.shared
        let hosting = NSHostingView(rootView: view.frame(width: size.width, height: size.height)
            .background(Color(nsColor: .windowBackgroundColor)))
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let rep = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: rep)
        let png = try XCTUnwrap(rep.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        try png.write(to: url)
    }

    func test_theTracesMapIsGivenTheDevices() throws {
        let window = try MonitorNavigationTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(".addingDevices(devicesOnMap)"))
        XCTAssertTrue(window.contains("@StateObject private var devices = AccountDevicesModel()"))
        XCTAssertTrue(window.contains("devices.attach(model.payoutLinkDaemonClient)"))
    }
}
