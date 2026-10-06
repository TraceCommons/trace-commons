import AppKit
import SwiftUI
import Vision
import XCTest
import TCShellCore
@testable import TraceCommonsApp

/// Private AI sign-in is reachable before Commons enrollment (R-38), as the
/// legacy window allowed: the Inference request opens the Monitor while
/// onboarding is required, the strip shows Inference alone, and its
/// inspector draws the sign-in. Drawing it completes no onboarding. Home
/// and Traces still open first run.
final class InferenceDuringOnboardingTests: XCTestCase {
    @MainActor
    func testSignInRendersOnInferenceWhileOnboardingIsRequired() throws {
        let model = AppModel()
        model.setStartupForTesting(.running)
        XCTAssertTrue(model.requiresOnboarding)
        XCTAssertEqual(LaunchRouting.opening(.inference, startup: model.startup, requiresOnboarding: model.requiresOnboarding, onboardingKnown: true).window, .monitor)
        XCTAssertEqual(MonitorWindowView.Tab.shown(requiresOnboarding: model.requiresOnboarding), [.inference])
        let shown = MonitorWindowView.shownTab(.home, requiresOnboarding: model.requiresOnboarding)
        XCTAssertEqual(shown, .inference, "a restored Home is not drawn during onboarding")

        let copy = try XCTUnwrap(model.privateInferenceCopy)
        let image = try render(
            PrivateAIInspectorView(store: InferenceStore(client: nil), destinationLabel: copy.destination)
                .environmentObject(model)
                .environment(\.colorScheme, .light))
        let words = try recognizedWords(image)
        XCTAssertTrue(words.contains(copy.credentialWhat),
                      "Private AI must show sign-in before Commons enrollment")

        // Drawing it finished nothing.
        XCTAssertFalse(model.status.loggedIn)
        XCTAssertFalse(model.isOnboardingComplete)
        XCTAssertTrue(model.requiresOnboarding)
    }

    @MainActor
    func testHomeAndTracesStillOpenFirstRun() {
        for destination: MonitorDestination? in [nil, .home(.overview), .home(.history), .traces(entryId: nil)] {
            XCTAssertEqual(LaunchRouting.opening(destination, startup: .running, requiresOnboarding: true, onboardingKnown: true).window, .firstRun,
                           String(describing: destination))
        }
        // The Tab commands follow the same routing: Cmd-2 reaches Inference.
        XCTAssertEqual(LaunchRouting.opening(MonitorCommands.destination(.inference), startup: .running, requiresOnboarding: true, onboardingKnown: true).window,
                       .monitor)
        XCTAssertEqual(LaunchRouting.opening(MonitorCommands.destination(.home), startup: .running, requiresOnboarding: true, onboardingKnown: true).window,
                       .firstRun)
        XCTAssertEqual(LaunchRouting.opening(MonitorCommands.destination(.traces), startup: .running, requiresOnboarding: true, onboardingKnown: true).window,
                       .firstRun)
    }

    @MainActor
    private func render<V: View>(_ view: V) throws -> Data {
        _ = NSApplication.shared
        let bounds = NSRect(x: 0, y: 0, width: 640, height: 1100)
        let hosting = NSHostingView(rootView: view.frame(width: bounds.width, height: bounds.height, alignment: .top)
            .background(Color.white))
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        hosting.frame = bounds
        defer { window.close() }
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }

    private func recognizedWords(_ image: Data) throws -> String {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        try VNImageRequestHandler(data: image).perform([request])
        // Vision reads the sans-serif capital I in AI as a lowercase l.
        return (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: " ").replacingOccurrences(of: " Al ", with: " AI ")
    }
}
