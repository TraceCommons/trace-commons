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
    func testPrivateAIAccountStatesRender() async throws {
        let screenshotDirectory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"]
        let directory = screenshotDirectory.map(URL.init(fileURLWithPath:))
            ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer {
            if screenshotDirectory == nil { try? FileManager.default.removeItem(at: directory) }
        }
        for state in ["absent", "present"] {
            let model = AppModel()
            model.setClientForTesting(DaemonClient(daemon: AccountPreviewDaemon(state: state)))
            model.refreshNearAiCredential()
            for _ in 0..<100 where model.credentialStatus.state != state {
                try await Task.sleep(for: .milliseconds(10))
            }
            XCTAssertEqual(model.credentialStatus.state, state)
            try render(PrivateInferenceContent().environmentObject(model),
                       size: CGSize(width: 760, height: 1100),
                       to: directory.appendingPathComponent("private-ai-\(state).png"))
        }
    }

    @MainActor
    private func render<V: View>(_ view: V, size: CGSize, to url: URL) throws {
        // ImageRenderer cannot draw AppKit-backed TextFields: it emits a
        // yellow placeholder instead. Host the real native hierarchy so this
        // test exercises the same controls the app displays.
        _ = NSApplication.shared
        let content = view.frame(width: size.width, height: size.height, alignment: .topLeading)
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

private final class AccountPreviewDaemon: DaemonCalling {
    let state: String
    init(state: String) { self.state = state }
    func call(_ method: String, params paramsJSON: String) -> String {
        "{\"id\":1,\"result\":{\"state\":\"\(state)\",\"session_state\":\"\(state)\"}}"
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}
