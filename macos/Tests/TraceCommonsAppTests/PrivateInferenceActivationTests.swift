import AppKit
import Combine
import SwiftUI
import Vision
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

final class PrivateInferenceActivationTests: XCTestCase {
    @MainActor
    func testPrivateAIRendersSignInBeforeCommonsEnrollment() async throws {
        let model = AppModel()
        model.setStartupForTesting(.running)
        let navigation = MainWindowNavigation()
        navigation.section = .privateInference
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        let image = try await render(model: model, navigation: navigation)
        let words = try recognizedWords(image)
        XCTAssertTrue(words.contains(copy.credentialWhat),
                      "Private AI must show Cloud sign-in before Commons enrollment")
        XCTAssertFalse(model.status.loggedIn)
        XCTAssertFalse(model.isOnboardingComplete)
        XCTAssertTrue(model.requiresOnboarding, "Private AI must not complete Commons onboarding")
    }

    @MainActor
    func testCommonsDestinationsStillRequireEnrollment() async throws {
        let model = AppModel()
        model.setStartupForTesting(.running)
        let navigation = MainWindowNavigation()
        for section: MainWindowView.Section in [.queue, .history, .settings] {
            navigation.section = section
            let words = try recognizedWords(await render(model: model, navigation: navigation))
            XCTAssertTrue(words.contains("GET STARTED"), "\(section) must retain the Commons welcome")
            XCTAssertFalse(model.status.loggedIn)
            XCTAssertFalse(model.isOnboardingComplete)
        }
    }

    @MainActor
    func testFreshPrivateAIRequiresCaptureChoicesAndTransitionsAfterContinue() async throws {
        let root = URL(fileURLWithPath: "/private/tmp/tc-activation-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let model = AppModel()
        defer { model.shutdown() }
        let refused = expectation(description: "Fresh profile requires capture choices")
        let startup = model.$startup.filter { $0 == .needsRoots }.first().sink { _ in refused.fulfill() }
        model.start(configDirectory: root.path)
        await fulfillment(of: [refused], timeout: 10)
        withExtendedLifetime(startup) {}
        let navigation = MainWindowNavigation()
        navigation.section = .privateInference
        let (window, hosting) = makeWindow(model: model, navigation: navigation, height: 1600)
        defer { window.close() }
        window.orderFront(nil)
        try await press("Continue", window: window, hosting: hosting)
        XCTAssertEqual(model.startup, .needsRoots, "Unanswered capture choices must block Continue")
        XCTAssertFalse(model.isStartingDaemon)
        let sourceCopy = try XCTUnwrap(TCSourceChecks.settingsCopy())
        for kind in SourceKind.allCases {
            let label = try XCTUnwrap(sourceCopy.tools[kind.rawValue]?.decline)
            try await press(label, window: window, hosting: hosting)
        }
        let ready = expectation(description: "Real daemon reports capture and credential state")
        let observation = model.$daemonSettings.combineLatest(model.$credentialStatus, model.$status)
            .filter { settings, credential, status in
                settings != nil && !credential.state.isEmpty && !status.schemaVersion.isEmpty
            }.first().sink { _ in ready.fulfill() }
        try await press("Continue", window: window, hosting: hosting)
        await fulfillment(of: [ready], timeout: 10)
        withExtendedLifetime(observation) {}
        XCTAssertEqual(model.startup, .running)
        let settings = try XCTUnwrap(model.daemonSettings)
        XCTAssertEqual(settings.claudeSourceMode, "off")
        XCTAssertEqual(settings.codexSourceMode, "off")
        XCTAssertEqual(settings.geminiSourceMode, "off")
        XCTAssertEqual(settings.clineSourceMode, "off")
        XCTAssertEqual(settings.opencodeSourceMode, "off")
        XCTAssertFalse(settings.privateInferenceOn, "Opening setup must not expose a listener")
        XCTAssertFalse(settings.inferenceEvidenceEnabled)
        XCTAssertFalse(model.status.loggedIn)
        XCTAssertTrue(model.status.consentScopes.isEmpty)
        XCTAssertFalse(model.isOnboardingComplete)
        let words = try recognizedWords(await snapshot(hosting))
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        let action = CredentialSurface.action(model.credentialStatus, calls: model.credentialCalls)
        XCTAssertEqual(action, .obtain)
        let label = try XCTUnwrap(CredentialSurface.actionLabel(action, copy: copy))
        XCTAssertTrue(words.contains(label), "The fresh profile must have an actionable Cloud sign-in")
    }

    private func recognizedWords(_ image: Data) throws -> String {
        // Vision reads the sans-serif capital I in AI as a lowercase l.
        try observations(image).compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: " ").replacingOccurrences(of: " Al ", with: " AI ")
    }

    private func observations(_ image: Data, labels: [String] = []) throws -> [VNRecognizedTextObservation] {
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.customWords = labels
        try VNImageRequestHandler(data: image).perform([request])
        return request.results ?? []
    }

    @MainActor
    private func makeWindow(model: AppModel, navigation: MainWindowNavigation, height: CGFloat = 900)
        -> (NSWindow, NSHostingView<AnyView>) {
        _ = NSApplication.shared
        let bounds = NSRect(x: 0, y: 0, width: 1000, height: height)
        let hosting = NSHostingView(rootView: AnyView(MainWindowView(navigation: navigation)
            .environmentObject(model).environment(ComputeModel())
            .environment(\.colorScheme, .light)))
        let window = NSWindow(contentRect: bounds, styleMask: [.borderless],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        hosting.frame = bounds
        return (window, hosting)
    }

    @MainActor
    private func snapshot(_ hosting: NSHostingView<AnyView>) async throws -> Data {
        try await Task.sleep(nanoseconds: 50_000_000)
        hosting.layoutSubtreeIfNeeded()
        hosting.window?.displayIfNeeded()
        let bounds = hosting.bounds
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        return try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
    }

    @MainActor
    private func press(_ label: String, window: NSWindow, hosting: NSHostingView<AnyView>) async throws {
        let detected = try observations(await snapshot(hosting), labels: [label])
        let matches = try detected.compactMap { observation -> CGRect? in
            for text in observation.topCandidates(5) {
                let normalized = text.string.replacingOccurrences(of: "’", with: "'")
                if let match = normalized.range(of: label, options: .caseInsensitive),
                   let range = Range(NSRange(match, in: normalized), in: text.string) {
                    return try text.boundingBox(for: range)?.boundingBox
                }
            }
            return nil
        }
        XCTAssertEqual(matches.count, 1, "The native control must have one visible label: \(label)")
        let rect = try XCTUnwrap(matches.first)
        let point = NSPoint(x: rect.midX * hosting.bounds.width,
                            y: (hosting.isFlipped ? 1 - rect.midY : rect.midY) * hosting.bounds.height)
        XCTAssertNotNil(hosting.hitTest(point))
        let location = hosting.convert(point, to: nil)
        for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
            let event = try XCTUnwrap(NSEvent.mouseEvent(with: type, location: location,
                modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                windowNumber: window.windowNumber, context: nil, eventNumber: 1,
                clickCount: 1, pressure: 1))
            window.sendEvent(event)
            await Task.yield()
        }
    }

    @MainActor
    private func render(model: AppModel, navigation: MainWindowNavigation) async throws -> Data {
        let (window, hosting) = makeWindow(model: model, navigation: navigation)
        defer { window.close() }
        let image = try await snapshot(hosting)
        if let directory = ProcessInfo.processInfo.environment["TC_ACTIVATION_SCREENSHOT_DIR"] {
            let root = URL(fileURLWithPath: directory)
            try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
            try image.write(to: root.appendingPathComponent("\(navigation.section.rawValue)-\(model.credentialStatus.state.isEmpty ? "unreported" : "fresh").png"))
        }
        return image
    }
}
