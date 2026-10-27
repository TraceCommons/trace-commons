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
        // Directly below this computer (owner, 2026-10-27), in one column,
        // clear of its name, and inside the band.
        var above = hub.at.y + FlowMapScene.deviceClearance - 1
        for node in devices {
            XCTAssertEqual(node.at.x, hub.at.x, "directly below this computer")
            XCTAssertGreaterThan(node.at.y, above, "each below the one before, clear of this computer's name")
            XCTAssertLessThanOrEqual(node.at.y, FlowMapScene.band(scene.space).upperBound)
            above = node.at.y
        }
        // One quiet line from each device to the commons: nothing says what
        // another device sends, so nothing moves or is dashed.
        let added = scene.arcs.dropFirst(base.arcs.count)
        XCTAssertEqual(added.count, 2)
        XCTAssertTrue(added.allSatisfy { $0.style == .quiet })
        XCTAssertTrue(added.allSatisfy { $0.to == library.at })
        // Bowed out to the right of this computer, so no line runs through
        // it on the way up.
        XCTAssertTrue(added.allSatisfy { min($0.control1.x, $0.control2.x) > hub.at.x + FlowMapScene.padHalfWidth })
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
            listed, name: { $0.label ?? "(no name)" }, lastSeen: { $0.lastSeenAt.map { "seen \($0)" } },
            traces: { "\($0) traces" })
        XCTAssertEqual(others, [
            .init(id: "sha256:srv", name: "build-box", detail: "seen t1"),
            .init(id: "sha256:new", name: "(no name)", detail: "—"),
        ])
        XCTAssertEqual(FlowMapScene.otherDevices(nil, name: { _ in "" }, lastSeen: { _ in nil }, traces: { _ in "" }), [],
                       "an unread list draws no devices, never a guess")
    }

    /// A device that sent a trace within the hour carries traces now: its
    /// line moves, as this computer's does while it sends. One that sent
    /// earlier, or that the server has not counted, stays quiet; a count
    /// the server did not send is never drawn as nothing sent.
    func test_aDeviceThatContributedWithinTheHourMoves() throws {
        let now = Date(timeIntervalSince1970: 1_800_000_000)
        let base = traces()
        let scene = base.addingDevices([
            .init(id: "a", name: "a", detail: "", contributed: 3, lastContributedAt: now.addingTimeInterval(-600)),
            .init(id: "b", name: "b", detail: "", contributed: 9, lastContributedAt: now.addingTimeInterval(-7200)),
            .init(id: "c", name: "c", detail: "", contributed: nil, lastContributedAt: nil),
            .init(id: "d", name: "d", detail: "", contributed: 0, lastContributedAt: nil),
        ], now: now)
        let added = Array(scene.arcs.dropFirst(base.arcs.count))
        XCTAssertEqual(added.map(\.style), [.flowing, .quiet, .quiet, .quiet])
        XCTAssertTrue(scene.flows)
        XCTAssertEqual(FlowMapScene.deviceActiveWindow, 3600)
        // Something a device sent stands in the commons: the commons is lit.
        guard case .library(let active) = try XCTUnwrap(scene.nodes.first { $0.id == "library" }).kind else {
            return XCTFail("no library")
        }
        XCTAssertTrue(active)
    }

    func test_theCommonsIsNotLitByDevicesThatSentNothing() throws {
        let tree = TracesTree(tools: [.init(kind: .claudeCode, mode: .watch, folders: [])], unplaced: [])
        let base = FlowMapScene.traces(tree, gate: .init(state: .ready), contributed: .init(total: 0))
        let scene = base.addingDevices([
            .init(id: "c", name: "c", detail: "", contributed: nil, lastContributedAt: nil),
            .init(id: "d", name: "d", detail: "", contributed: 0, lastContributedAt: nil),
        ])
        XCTAssertEqual(scene.nodes.first { $0.id == "library" }?.kind, .library(active: false))
    }

    /// The card says how many of the device's traces stand, in the core's
    /// own counted words, under its last-seen line; nothing when the server
    /// did not say.
    func test_theCardSaysHowManyTracesTheDeviceContributed() {
        let listed = [
            AccountDevice(deviceKeyId: "sha256:a", label: "a", lastSeenAt: "t1",
                          contributedTraces: 12, lastContributedAt: "2027-01-15T08:00:00Z"),
            AccountDevice(deviceKeyId: "sha256:b", label: "b", lastSeenAt: "t1"),
        ]
        let others = FlowMapScene.otherDevices(
            listed, name: { $0.label ?? "" }, lastSeen: { $0.lastSeenAt.map { "seen \($0)" } },
            traces: { "\($0) traces" })
        XCTAssertEqual(others.map(\.detail), ["seen t1\n12 traces", "seen t1"])
        // The core's own counted words, singular included.
        XCTAssertEqual(FlowMapScene.words?.tracesCounted(1), "1 trace")
        XCTAssertEqual(FlowMapScene.words?.tracesCounted(12), "12 traces")
        XCTAssertEqual(others.map(\.contributed), [12, nil])
        XCTAssertEqual(others[0].lastContributedAt, ISO8601DateFormatter().date(from: "2027-01-15T08:00:00Z"))
        XCTAssertNil(others[1].lastContributedAt)
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
                .init(id: "a", name: "build-box", detail: "Last seen 5 minutes ago\n12 traces",
                      contributed: 12, lastContributedAt: Date().addingTimeInterval(-300)),
                .init(id: "b", name: "zaki-air", detail: "Last seen 2 days ago\n3 traces",
                      contributed: 3, lastContributedAt: Date().addingTimeInterval(-172_800)),
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
