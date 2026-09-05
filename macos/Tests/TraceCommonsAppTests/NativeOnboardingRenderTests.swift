import AppKit
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

final class NativeOnboardingRenderTests: XCTestCase {
    @MainActor
    func testSyntheticFirstContributionAndWitnessConsentRenderWithoutRequestingReview() async throws {
        let copy = try XCTUnwrap(WitnessCopy.decode(fromJSON: TCWitness.copyJSON() ?? "")?.review)
        var confirmed = false
        let directory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"]
            .map(URL.init(fileURLWithPath:))
            ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer {
            if ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"] == nil {
                try? FileManager.default.removeItem(at: directory)
            }
        }
        try render(WitnessReviewConsent(copy: copy) { confirmed = true },
                   size: CGSize(width: 560, height: 390), to: directory.appendingPathComponent("native-witness-consent.png"))
        // Constructing this model does not start a daemon or inspect sessions.
        let model = AppModel()
        try render(QueueContent(previewing: .constant(nil)).environmentObject(model),
                   size: CGSize(width: 860, height: 640), to: directory.appendingPathComponent("native-first-contribution.png"))
        try render(NearAccountConnectView(onEnrolled: {}).environmentObject(model),
                   size: CGSize(width: 680, height: 300), to: directory.appendingPathComponent("native-wallet-connect.png"))
        try render(AdmissionPreparationView(entryID: "synthetic").environmentObject(model),
                   size: CGSize(width: 680, height: 320), to: directory.appendingPathComponent("native-admission-preparation.png"))
        XCTAssertFalse(confirmed)
    }

    @MainActor
    private func render<V: View>(_ view: V, size: CGSize, to url: URL) throws {
        // ImageRenderer cannot draw AppKit-backed TextFields: it emits a
        // yellow placeholder instead. Host the real native hierarchy so this
        // test exercises the same controls the app displays.
        _ = NSApplication.shared
        let content = view.frame(width: size.width, height: size.height)
            .background(Color(nsColor: .windowBackgroundColor))
        let hosting = NSHostingView(rootView: content)
        let bounds = NSRect(origin: .zero, size: size)
        hosting.frame = bounds
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let representation = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: representation)
        let png = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        try png.write(to: url)
    }
}
