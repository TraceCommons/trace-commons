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
        XCTAssertFalse(confirmed)
    }

    @MainActor
    private func render<V: View>(_ view: V, size: CGSize, to url: URL) throws {
        let renderer = ImageRenderer(content: view.frame(width: size.width, height: size.height)
            .background(Color(nsColor: .windowBackgroundColor)))
        renderer.scale = 2
        let image = try XCTUnwrap(renderer.nsImage)
        let tiff = try XCTUnwrap(image.tiffRepresentation)
        let representation = try XCTUnwrap(NSBitmapImageRep(data: tiff))
        let png = try XCTUnwrap(representation.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(png.count, 1000)
        try png.write(to: url)
    }
}
