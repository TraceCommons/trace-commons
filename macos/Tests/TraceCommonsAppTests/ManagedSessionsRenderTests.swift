import AppKit
import SwiftUI
import TCShellCore
import XCTest
@testable import TraceCommonsApp

final class ManagedSessionsRenderTests: XCTestCase {
    @MainActor
    func testSavedAccountsAndCliSessionRenderWithoutStartingANativeTool() throws {
        let repository = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        let model = AppModel()
        model.managedSnapshot = try JSONDecoder().decode(ManagedSnapshot.self, from: Data(contentsOf: repository.appendingPathComponent("tests/fixtures/managed/snapshot.json")))
        XCTAssertEqual(model.managedSnapshot?.sessions.first?.accountLabel, "Personal")
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"].map(URL.init(fileURLWithPath:)) ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil { try? FileManager.default.removeItem(at: directory) } }
        try render(ManagedSessionsSection().environmentObject(model), size: CGSize(width: 720, height: 480), to: directory.appendingPathComponent("managed-sessions.png"))
        try render(ManagedAccountSheet(onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 360), to: directory.appendingPathComponent("managed-add-account.png"))
        try render(ManagedLaunchSheet(onClose: {}).environmentObject(model), size: CGSize(width: 580, height: 300), to: directory.appendingPathComponent("managed-launch.png"))
        let account = try XCTUnwrap(model.managedSnapshot?.accounts.first)
        try render(ManagedRenameSheet(account: account, onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 240), to: directory.appendingPathComponent("managed-rename.png"))
        try render(ManagedRemoveSheet(account: account, onClose: {}).environmentObject(model), size: CGSize(width: 520, height: 240), to: directory.appendingPathComponent("managed-remove.png"))
    }

    @MainActor
    private func render<V: View>(_ view: V, size: CGSize, to url: URL) throws {
        _ = NSApplication.shared
        let hosting = NSHostingView(rootView: view.padding(20).frame(width: size.width, height: size.height).background(Color(nsColor: .windowBackgroundColor)))
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
}
