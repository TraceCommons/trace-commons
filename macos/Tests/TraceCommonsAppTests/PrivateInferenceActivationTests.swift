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
        let firstRun = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        for section: MainWindowView.Section in [.queue, .history, .settings] {
            navigation.section = section
            let words = try recognizedWords(await render(model: model, navigation: navigation))
            XCTAssertTrue(words.contains(firstRun.join.lookUp), "\(section) must retain the first run's Join")
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
        let firstRun = try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
        // The first run's Folders step, every row unanswered, and nothing
        // started by showing it. Read by a row's question, which only Folders
        // asks: the eyebrow and the footer's links are too faint to read
        // back, so Continue is not pressed here and its disabled rule is
        // `FoldersScreenTests.test_continueIsDisabledUntilEveryRowIsAnswered`.
        let question = firstRun.folders.watchQuestion.replacingOccurrences(of: "{tool}", with: "Codex")
        let folders = try await settledWords(hosting) { $0.contains(question) }
        XCTAssertTrue(folders.contains(question),
                      "Private AI must ask the first run's Folders")
        XCTAssertEqual(model.startup, .needsRoots, "showing Folders starts nothing")
        XCTAssertFalse(model.isStartingDaemon)
        // Each row's answer is a `GlassPicker` menu, which a synthesised
        // click cannot open; the answers go in through the call Continue
        // makes (`FirstRunDaemon.startDaemon`), with every tool declined.
        var declined = SessionRoots()
        for kind in SourceKind.allCases { declined[kind] = .off }
        let ready = expectation(description: "Real daemon reports capture and credential state")
        let observation = model.$daemonSettings.combineLatest(model.$credentialStatus, model.$status)
            .filter { settings, credential, status in
                settings != nil && !credential.state.isEmpty && !status.schemaVersion.isEmpty
            }.first().sink { _ in ready.fulfill() }
        let started = await model.startDaemon(settingsJSON: try XCTUnwrap(declined.settingsJSON()))
        XCTAssertTrue(started)
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
        // The Folders step's list grows the borderless window past the
        // height it was made at, and Vision misses most of the glass
        // text on a page that tall; Private AI is read at the original
        // size, where every line of it fits.
        window.setFrame(NSRect(x: 0, y: 0, width: 1000, height: 1600), display: true)
        hosting.frame = NSRect(x: 0, y: 0, width: 1000, height: 1600)
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        let action = CredentialSurface.action(model.credentialStatus, calls: model.credentialCalls)
        XCTAssertEqual(action, .obtain)
        let label = try XCTUnwrap(CredentialSurface.actionLabel(action, copy: copy))
        let words = try await settledWords(hosting) { $0.contains(label) }
        XCTAssertTrue(words.contains(label), "The fresh profile must have an actionable Cloud sign-in")
    }

    /// The window's words once `ready` holds, reading again for up to two
    /// seconds: a slower runner draws the page a frame or two after the
    /// state it reflects. The last read comes back either way, for the
    /// caller's assertion.
    @MainActor
    private func settledWords(_ hosting: NSHostingView<AnyView>,
                              until ready: (String) -> Bool) async throws -> String {
        var words = ""
        for _ in 0..<8 {
            words = try recognizedWords(await snapshot(hosting))
            if ready(words) { break }
            try await Task.sleep(nanoseconds: 250_000_000)
        }
        return words
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
