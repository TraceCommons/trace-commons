// INTEGRATION: run with the sessionState onboarding changes and their matching FFI dylib.
import AppKit
import Combine
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

private final class OnboardingSessionDaemon: DaemonCalling {
    struct Call {
        let method: String
        let params: String
    }

    private enum Phase { case legacyKey, waiting, signedIn, storageUnavailable }
    private let lock = NSLock()
    private var phase: Phase = .legacyKey
    private var recorded: [Call] = []
    private let unexpectedCall: XCTestExpectation

    init(unexpectedCall: XCTestExpectation) {
        self.unexpectedCall = unexpectedCall
    }

    var calls: [Call] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    func completeSignIn() {
        lock.lock()
        defer { lock.unlock() }
        phase = .signedIn
    }

    func makeStorageUnavailable() {
        lock.lock()
        defer { lock.unlock() }
        phase = .storageUnavailable
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append(Call(method: method, params: paramsJSON))
        switch method {
        case "near_ai_credential_status":
            switch phase {
            case .legacyKey:
                return #"{"id":1,"result":{"state":"present","session_state":"absent"}}"#
            case .waiting:
                return #"{"id":1,"result":{"state":"obtaining","session_state":"obtaining","attempt_id":"synthetic-attempt","attempt_status":"waiting_for_browser"}}"#
            case .signedIn:
                return #"{"id":1,"result":{"state":"present","session_state":"present"}}"#
            case .storageUnavailable:
                return #"{"id":1,"result":{"state":"storage_unavailable","session_state":"storage_unavailable"}}"#
            }
        case "near_ai_balance":
            return #"{"id":1,"result":{"state":"no_session","remaining_nanos":null,"spend_limit_nanos":null,"total_spent_nanos":null,"scale":9,"observed_at":null}}"#
        case "near_ai_credential_start":
            phase = .waiting
            return #"{"id":1,"result":{"attempt_id":"synthetic-attempt","browser_url":"https://cloud.example.invalid/native-login","status":"waiting_for_browser"}}"#
        default:
            // Enrollment, wallet requests and consent writes are forbidden in
            // these tests. Record and fail them even if the caller ignores errors.
            unexpectedCall.fulfill()
            return #"{"id":1,"error":{"code":"unavailable","message":"unexpected_test_method"}}"#
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

final class NearAiOnboardingSessionTests: XCTestCase {
    @MainActor
    func testLegacyKeyWithoutSessionOffersSignInWithoutEnrolling() async throws {
        let unexpected = expectation(description: "Rendering must not write or enroll")
        unexpected.isInverted = true
        let daemon = OnboardingSessionDaemon(unexpectedCall: unexpected)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        _ = try XCTUnwrap(model.privateInferenceCopy, "The matching FFI copy must be available")

        await awaitSession(model, state: "absent") {
            model.refreshNearAiCredential()
        }
        XCTAssertEqual(model.credentialStatus.state, CredentialSurface.statePresent)
        XCTAssertEqual(CredentialSurface.action(model.credentialStatus, calls: model.credentialCalls), .forget)
        XCTAssertEqual(
            CredentialSurface.action(CredentialStatus(state: model.credentialStatus.sessionState), calls: model.credentialCalls),
            .obtain,
            "A retained inference key cannot replace the Cloud session required to join")

        var enrolled = 0
        try await captureMatrix(model: model, name: "near-ai-legacy-key") { enrolled += 1 }
        await fulfillment(of: [unexpected], timeout: 0.2)
        XCTAssertEqual(enrolled, 0)
        XCTAssertFalse(model.status.loggedIn)
        XCTAssertNil(model.status.tenantID)
        XCTAssertEqual(Set(daemon.calls.map(\.method)), ["near_ai_credential_status", "near_ai_balance"])
    }

    @MainActor
    func testSuccessfulSignInLeavesJoiningAsASeparateAction() async throws {
        let unexpected = expectation(description: "Sign-in must not enroll or grant consent")
        unexpected.isInverted = true
        let daemon = OnboardingSessionDaemon(unexpectedCall: unexpected)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        _ = try XCTUnwrap(model.privateInferenceCopy, "The matching FFI copy must be available")
        await awaitSession(model, state: "absent") {
            model.refreshNearAiCredential()
        }

        var browserURL: URL?
        await awaitSession(model, state: "obtaining") {
            // Exercise the model action without opening a browser or spending.
            browserURL = await model.startNearAiCredential()
        }
        XCTAssertEqual(browserURL?.absoluteString, "https://cloud.example.invalid/native-login")
        XCTAssertEqual(model.credentialAttempt?.attemptID, "synthetic-attempt")
        XCTAssertFalse(model.credentialBusy)

        daemon.completeSignIn()
        await awaitSession(model, state: CredentialSurface.statePresent) {
            model.refreshNearAiCredential()
        }
        XCTAssertEqual(model.credentialStatus.state, CredentialSurface.statePresent)
        XCTAssertNil(model.credentialAttempt, "The completed browser ceremony must stop polling")

        var enrolled = 0
        try await captureMatrix(model: model, name: "near-ai-signed-in") { enrolled += 1 }
        await fulfillment(of: [unexpected], timeout: 0.2)
        XCTAssertEqual(enrolled, 0)
        XCTAssertFalse(model.status.loggedIn, "A Cloud login must not become Commons enrollment")
        XCTAssertNil(model.status.tenantID)
        XCTAssertFalse(model.isOnboardingComplete)
        let starts = daemon.calls.filter { $0.method == "near_ai_credential_start" }
        XCTAssertEqual(starts.count, 1)
        let start = try XCTUnwrap(starts.first)
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(start.params.utf8)) as? [String: Any])
        XCTAssertTrue(params.isEmpty, "Sign-in must not carry enrollment or consent parameters")
        XCTAssertEqual(Set(daemon.calls.map(\.method)), ["near_ai_credential_start", "near_ai_credential_status", "near_ai_balance"])
    }

    @MainActor
    private func awaitSession(_ model: AppModel, state: String, action: () async -> Void) async {
        let published = expectation(description: "Published Cloud session state: \(state)")
        let observation = model.$credentialStatus
            .filter { $0.sessionState == state }
            .prefix(1)
            .sink { _ in published.fulfill() }
        defer { observation.cancel() }
        await action()
        await fulfillment(of: [published], timeout: 3)
        XCTAssertEqual(model.credentialStatus.sessionState, state)
    }

    @MainActor
    func testUnavailableStoreWithholdsSignInAndKeepsRemovalExplicit() async throws {
        let unexpected = expectation(description: "An unavailable store must not start sign-in or enroll")
        unexpected.isInverted = true
        let daemon = OnboardingSessionDaemon(unexpectedCall: unexpected)
        daemon.makeStorageUnavailable()
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        await awaitSession(model, state: "storage_unavailable") {
            model.refreshNearAiCredential()
        }
        XCTAssertEqual(CredentialSurface.action(model.credentialStatus, calls: model.credentialCalls), .forget)
        XCTAssertTrue(CredentialSurface.stateLine(model.credentialStatus, copy: copy, calls: model.credentialCalls).contains("Unlock"))
        var enrolled = 0
        try await captureMatrix(model: model, name: "near-ai-storage-unavailable") { enrolled += 1 }
        await fulfillment(of: [unexpected], timeout: 0.2)
        XCTAssertEqual(enrolled, 0)
        XCTAssertEqual(Set(daemon.calls.map(\.method)), ["near_ai_credential_status", "near_ai_balance"])
    }

    @MainActor
    private func captureMatrix(model: AppModel, name: String, onEnrolled: @escaping () -> Void) async throws {
        let configuredDirectory = ProcessInfo.processInfo.environment["TRACE_COMMONS_SCREENSHOT_DIR"]
            .flatMap { $0.isEmpty ? nil : URL(fileURLWithPath: $0) }
        let directory = configuredDirectory
            ?? FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer {
            if configuredDirectory == nil { try? FileManager.default.removeItem(at: directory) }
        }
        for width in [680, 375] {
            var images: [Data] = []
            for (label, scheme) in [("light", ColorScheme.light), ("dark", ColorScheme.dark)] {
                let image = try await capture(
                    NearAiJoinView(onEnrolled: onEnrolled).environmentObject(model),
                    width: CGFloat(width), scheme: scheme)
                images.append(image)
                try image.write(to: directory.appendingPathComponent("\(name)-\(width)-\(label).png"))
            }
            XCTAssertNotEqual(images[0], images[1], "Both color appearances must reach the native view")
        }
    }

    @MainActor
    private func capture<V: View>(_ view: V, width: CGFloat, scheme: ColorScheme) async throws -> Data {
        // Native hosting draws TextFields correctly. These captures need visual
        // inspection; neither image bytes nor model calls prove button-click wiring.
        _ = NSApplication.shared
        let content = view.padding(24)
            .frame(width: width, alignment: .topLeading)
            .fixedSize(horizontal: false, vertical: true)
            .background(Color(nsColor: .windowBackgroundColor))
            .environment(\.colorScheme, scheme)
        let hosting = NSHostingView(rootView: content)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: width, height: 900),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
        window.contentView = hosting
        defer { window.close() }
        await Task.yield()
        hosting.layoutSubtreeIfNeeded()
        let height = ceil(hosting.fittingSize.height)
        XCTAssertGreaterThan(height, 40, "The sign-in card must contain native content")
        XCTAssertLessThan(height, 2000, "Unexpected layout expansion must not pass as a screenshot")
        let bounds = NSRect(x: 0, y: 0, width: width, height: height)
        window.setContentSize(bounds.size)
        hosting.frame = bounds
        hosting.layoutSubtreeIfNeeded()
        window.displayIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: bounds))
        hosting.cacheDisplay(in: bounds, to: bitmap)
        let image = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        XCTAssertGreaterThan(image.count, 1000)
        return image
    }
}
