import AppKit
import Combine
import SwiftUI
import Vision
import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

private final class RenewalDaemon: DaemonCalling {
    func call(_ method: String, params: String) -> String {
        switch method {
        case "near_ai_credential_status":
            return #"{"id":1,"result":{"state":"present","session_state":"present"}}"#
        case "near_ai_balance":
            return #"{"id":1,"result":{"state":"session_expired"}}"#
        default:
            return #"{"id":1,"error":{"code":"unavailable","message":"synthetic_refusal"}}"#
        }
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

final class CredentialRenewalTests: XCTestCase {
    @MainActor
    func testExpiredSessionRetainsKeyAndOffersSignInWithoutAChooser() async throws {
        let model = AppModel()
        defer { model.shutdown() }
        model.setClientForTesting(DaemonClient(daemon: RenewalDaemon()))
        let loaded = expectation(description: "Expired session and retained inference key")
        let observation = model.$credentialStatus.combineLatest(model.$balanceStatus)
            .filter { $0.state == "present" && $1.state == "session_expired" }
            .first().sink { _ in loaded.fulfill() }
        model.refreshNearAiCredential()
        await fulfillment(of: [loaded], timeout: 3)
        withExtendedLifetime(observation) {}
        let copy = try XCTUnwrap(model.privateInferenceCopy)
        let hosting = NSHostingView(rootView: CredentialSection(copy: copy)
            .environmentObject(model).padding(24).frame(width: 900, height: 1100)
            .background(Color.white).environment(\.colorScheme, .light))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 900, height: 1100),
                              styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = hosting
        defer { window.close() }
        try await Task.sleep(for: .milliseconds(150))
        hosting.layoutSubtreeIfNeeded()
        let bitmap = try XCTUnwrap(hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds))
        hosting.cacheDisplay(in: hosting.bounds, to: bitmap)
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        try VNImageRequestHandler(cgImage: XCTUnwrap(bitmap.cgImage)).perform([request])
        let words = (request.results ?? []).compactMap { $0.topCandidates(1).first?.string }.joined(separator: " ")
        let signIn = try XCTUnwrap(CredentialSurface.actionLabel(.obtain, copy: copy))
        XCTAssertTrue(words.contains(signIn), "Renewing an expired session must offer Sign in: \(words)")
        // The login is chosen on the daemon's browser page (owner,
        // 2026-11-02), so no chooser is drawn here.
        XCTAssertFalse(words.contains(copy.credentialProviderLabel),
                       "Renewal draws a provider chooser again: \(words)")
        XCTAssertEqual(CredentialSurface.action(model.credentialStatus, calls: model.credentialCalls), .forget,
                       "Renewal must preserve the retained inference key")
    }
}
