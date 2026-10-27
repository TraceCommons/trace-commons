import SwiftUI
import TCBridge
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
        // No line joins another device to the commons (owner, 2026-10-27):
        // the devices are drawn, and the map's lines stay this computer's.
        XCTAssertEqual(scene.arcs, base.arcs)
        XCTAssertGreaterThan(devices[0].at.y, library.at.y)
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

    /// What another device has in the commons lights the commons, as this
    /// computer's contributions do, but draws no line (owner, 2026-10-27).
    func test_aDeviceWithTracesInTheCommonsLightsItWithoutALine() throws {
        let base = traces()
        let scene = base.addingDevices([
            .init(id: "a", name: "a", detail: "", contributed: 3),
            .init(id: "c", name: "c", detail: "", contributed: nil),
        ])
        XCTAssertEqual(scene.arcs, base.arcs)
        guard case .library(let active) = try XCTUnwrap(scene.nodes.first { $0.id == "library" }).kind else {
            return XCTFail("no library")
        }
        XCTAssertTrue(active)
    }

    func test_theCommonsIsNotLitByDevicesThatSentNothing() throws {
        let tree = TracesTree(tools: [.init(kind: .claudeCode, mode: .watch, folders: [])], unplaced: [])
        let base = FlowMapScene.traces(tree, gate: .init(state: .ready), contributed: .init(total: 0))
        let scene = base.addingDevices([
            .init(id: "c", name: "c", detail: "", contributed: nil),
            .init(id: "d", name: "d", detail: "", contributed: 0),
        ])
        XCTAssertEqual(scene.nodes.first { $0.id == "library" }?.kind, .library(active: false))
    }

    /// The card says how many of the device's traces stand, in the core's
    /// own counted words, under its last-seen line; nothing when the server
    /// did not say.
    func test_theCardSaysHowManyTracesTheDeviceContributed() {
        let listed = [
            AccountDevice(deviceKeyId: "sha256:a", label: "a", lastSeenAt: "t1", contributedTraces: 12),
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
    }

    /// The + under the column (owner, 2026-10-27): below the last device,
    /// in this computer's column, or just under this computer when there
    /// are none; only when the map offers linking, and never on a map with
    /// no commons.
    func test_thePlusSitsUnderTheColumnWhenLinkingIsOffered() throws {
        let base = traces()
        let hub = try XCTUnwrap(base.nodes.first { $0.id == "hub" })
        XCTAssertNil(base.addingDevices([device(1)]).addDevice, "not offered, not drawn")

        let none = base.addingDevices([], offersLink: true)
        let alone = try XCTUnwrap(none.addDevice)
        XCTAssertEqual(alone.x, hub.at.x)
        XCTAssertGreaterThan(alone.y, hub.at.y + FlowMapScene.deviceClearance - 1, "clear of this computer's name")

        let two = base.addingDevices([device(1), device(2)], offersLink: true)
        let plus = try XCTUnwrap(two.addDevice)
        let last = try XCTUnwrap(two.nodes.last { $0.kind == .device })
        XCTAssertEqual(plus.x, hub.at.x)
        XCTAssertGreaterThan(plus.y, last.at.y, "under the last device")
        XCTAssertLessThanOrEqual(plus.y, two.space.height)

        let six = base.addingDevices((1...6).map(device), offersLink: true)
        let overflow = try XCTUnwrap(six.overflows.first)
        XCTAssertGreaterThan(try XCTUnwrap(six.addDevice).y, overflow.at.y, "under the +N count too")

        var privateAI = FlowMapScene()
        privateAI.nodes = [.init(id: "destination", kind: .destination(.running), at: .zero, radius: 26,
                                 label: "x", detail: "y")]
        XCTAssertNil(privateAI.addingDevices([], offersLink: true).addDevice)
    }

    /// The + hands one request to the Devices section, which starts linking
    /// a server once and only once.
    func test_aLinkRequestIsTakenOnce() {
        let request = AccountDeviceLinkRequest()
        XCTAssertFalse(request.take())
        request.ask()
        XCTAssertTrue(request.take())
        XCTAssertFalse(request.take(), "a second look starts nothing")
    }

    /// Kristi's #1681 review: with Settings closed, the + opens a fresh
    /// Devices section whose model has no client yet. A request taken then
    /// would be spent on a link that cannot start, so the model takes it
    /// only once attached, and then starts linking.
    func test_aLinkRequestWaitsForTheModelToBeAttached() async throws {
        let request = AccountDeviceLinkRequest()
        request.ask()
        let model = AccountDevicesModel()
        let early = await model.linkIfRequested(request)
        XCTAssertFalse(early, "no client: nothing starts")
        XCTAssertTrue(request.pending, "and the request is kept for when it can")

        let daemon = LinkDaemon()
        model.attach(DaemonClient(daemon: daemon))
        let started = Task { await model.linkIfRequested(request) }
        let deadline = Date().addingTimeInterval(3)
        while !daemon.methods.contains("account_device_code_mint"), Date() < deadline {
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        started.cancel()
        XCTAssertTrue(daemon.methods.contains("account_device_code_mint"), "linking started: \(daemon.methods)")
        XCTAssertFalse(request.pending, "taken once")
    }

    /// Kristi's #1681 review: in a wide pane the column is short, and the
    /// + must stay inside the band, above the legend and zoom row.
    func test_thePlusStaysInsideTheBandInAWidePane() throws {
        let tree = TracesTree(tools: [.init(kind: .claudeCode, mode: .watch, folders: [])], unplaced: [])
        let space = FlowMapScene.space(for: CGSize(width: 1700, height: 1000))
        let scene = FlowMapScene.traces(tree, gate: .init(state: .ready), space: space)
            .addingDevices((1...4).map(device), offersLink: true)
        let band = FlowMapScene.band(scene.space)
        let plus = try XCTUnwrap(scene.addDevice)
        XCTAssertLessThanOrEqual(plus.y, band.upperBound, "clear of the legend and zoom row")
        let last = try XCTUnwrap(scene.nodes.last { $0.kind == .device })
        XCTAssertGreaterThan(plus.y, last.at.y, "still under the last device")
    }

    func test_thePlusOpensTheDevicesSectionAndStartsLinking() throws {
        let window = try MonitorNavigationTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains(".addingDevices(devicesOnMap, offersLink: devices.devices != nil && devices.surface != nil)"))
        XCTAssertTrue(window.contains("AccountDeviceLinkRequest.shared.ask()"))
        XCTAssertTrue(window.contains("OpenMonitor.request(.settings(.connection))"))
        let view = try MonitorNavigationTests.text("Views/AccountDevicesView.swift")
        XCTAssertTrue(view.contains("await model.linkIfRequested()"))
        let map = try MonitorNavigationTests.text("Views/Monitor/FlowMapView.swift")
        XCTAssertTrue(map.contains("if let at = scene.addDevice, let onAddDevice {"))
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
                      contributed: 12),
                .init(id: "b", name: "zaki-air", detail: "Last seen 2 days ago\n3 traces",
                      contributed: 3),
                .init(id: "c", name: "ci-runner-07", detail: "—"),
            ], offersLink: true)
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
                            accessibilityName: "map", mapStyle: style, onAddDevice: {}, addDeviceLabel: "Link a server"),
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
        XCTAssertTrue(window.contains(".addingDevices(devicesOnMap, offersLink:"))
        XCTAssertTrue(window.contains("@StateObject private var devices = AccountDevicesModel()"))
        XCTAssertTrue(window.contains("devices.attach(model.payoutLinkDaemonClient)"))
    }
}

/// Answers the Devices section's calls: a code when one is minted, used at
/// once when asked about, and an empty list.
private final class LinkDaemon: DaemonCalling, @unchecked Sendable {
    private let lock = NSLock()
    private var seen: [String] = []
    var methods: [String] { lock.withLock { seen } }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.withLock { seen.append(method) }
        switch method {
        case "account_device_code_mint":
            return #"{"id":1,"result":{"code_id":"c1","code":"tcd1_dA.s","expires_at":"2099-01-01T00:00:00Z"}}"#
        case "account_device_code_status":
            return #"{"id":1,"result":{"state":"redeemed","device_key_id":"sha256:cc"}}"#
        default:
            return #"{"id":1,"result":{"devices":[]}}"#
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
