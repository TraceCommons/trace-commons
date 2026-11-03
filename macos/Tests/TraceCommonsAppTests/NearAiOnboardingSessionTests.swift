// INTEGRATION: run with the sessionState onboarding changes and their matching FFI dylib.
import AppKit
import Combine
import os
import SwiftUI
import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

private final class OnboardingSessionDaemon: DaemonCalling {
    struct Call: Sendable {
        let method: String
        let params: String
    }

    private enum Phase: Sendable { case legacyKey, waiting, signedIn, storageUnavailable }
    private struct State: Sendable {
        var phase: Phase = .legacyKey
        var recorded: [Call] = []
    }
    private let state = OSAllocatedUnfairLock(initialState: State())
    private let unexpectedCall: XCTestExpectation

    init(unexpectedCall: XCTestExpectation) {
        self.unexpectedCall = unexpectedCall
    }

    var calls: [Call] {
        state.withLock { $0.recorded }
    }

    func completeSignIn() {
        state.withLock { $0.phase = .signedIn }
    }

    func makeStorageUnavailable() {
        state.withLock { $0.phase = .storageUnavailable }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        state.withLock { state in
            state.recorded.append(Call(method: method, params: paramsJSON))
            switch method {
            case "near_ai_credential_status":
                switch state.phase {
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
                state.phase = .waiting
                return #"{"id":1,"result":{"attempt_id":"synthetic-attempt","browser_url":"https://cloud.example.invalid/native-login","status":"waiting_for_browser"}}"#
            case "get_settings", "harness_list", "managed_snapshot":
                // What the key is behind is read again once a sign-in lands.
                // Reads only; the reply is a refusal, so nothing is published.
                return #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"#
            default:
                // Enrollment, wallet requests and consent writes are forbidden in
                // these tests. Record and fail them even if the caller ignores errors.
                unexpectedCall.fulfill()
                return #"{"id":1,"error":{"code":"unavailable","message":"unexpected_test_method"}}"#
            }
        }
    }

    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }
}

final class NearAiOnboardingSessionTests: XCTestCase {
    /// The section offers Sign in with no provider chooser: the daemon's
    /// browser page lists the build's logins (owner, 2026-11-02). Drawing
    /// it starts nothing.
    @MainActor
    func testSettingsDrawsNoChooserAndStartsNoSignIn() async throws {
        let unexpected = expectation(description: "Native controls must not enroll or grant consent")
        unexpected.isInverted = true
        let daemon = OnboardingSessionDaemon(unexpectedCall: unexpected)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        await awaitSession(model, state: "absent") { model.refreshNearAiCredential() }

        // Sign in is the thing to obtain; a legacy inference key must not
        // hide it.
        XCTAssertEqual(
            CredentialSurface.action(CredentialStatus(state: model.credentialStatus.sessionState),
                                     calls: model.credentialCalls),
            .obtain)

        let content = CredentialSection(copy: copy, requiresSession: true)
            .environmentObject(model)
            .padding(24)
            .frame(width: 375)
        let hosting = NSHostingView(rootView: content)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 375, height: 900),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        await Task.yield()
        hosting.layoutSubtreeIfNeeded()

        // Where SwiftUI backs a picker with AppKit (macOS 26), it would be an
        // NSPopUpButton; none is drawn. The source pin in
        // `NearAiSignInNamesNoProviderTests` covers macOS 27, which draws
        // pickers itself.
        XCTAssertTrue(descendants(hosting).compactMap { $0 as? NSPopUpButton }.isEmpty,
                      "Settings draws a provider chooser again")
        await fulfillment(of: [unexpected], timeout: 0.2)
        hosting.layoutSubtreeIfNeeded()
        XCTAssertFalse(daemon.calls.contains { $0.method == "near_ai_credential_start" })
        XCTAssertFalse(model.status.loggedIn)
    }

    @MainActor
    private func descendants(_ view: NSView) -> [NSView] {
        [view] + view.subviews.flatMap { descendants($0) }
    }

    @MainActor
    func testSettingsSignInSendsNoProviderWithoutEnrolling() async throws {
        let unexpected = expectation(description: "Sign-in must not enroll or grant consent")
        unexpected.isInverted = true
        let daemon = OnboardingSessionDaemon(unexpectedCall: unexpected)
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: daemon))
        await awaitSession(model, state: "absent") { model.refreshNearAiCredential() }

        var browserURL: URL?
        await awaitSession(model, state: "obtaining") {
            browserURL = await model.startNearAiCredential()
        }
        XCTAssertEqual(browserURL?.absoluteString, "https://cloud.example.invalid/native-login")
        let starts = daemon.calls.filter { $0.method == "near_ai_credential_start" }
        XCTAssertEqual(starts.count, 1)
        let start = try XCTUnwrap(starts.first)
        let params = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(start.params.utf8)) as? [String: Any])
        XCTAssertTrue(params.isEmpty, "the login is chosen on the daemon's browser page: \(params)")
        XCTAssertFalse(model.status.loggedIn)
        XCTAssertNil(model.status.tenantID)
        XCTAssertFalse(model.isOnboardingComplete)
        await fulfillment(of: [unexpected], timeout: 0.2)
    }

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
        XCTAssertEqual(
            Set(daemon.calls.map(\.method)),
            ["near_ai_credential_start", "near_ai_credential_status", "near_ai_balance",
             "get_settings", "harness_list", "managed_snapshot"],
            "The landed sign-in re-reads what the key is behind, and writes nothing else")
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
