import Foundation
import AppKit
import SwiftUI
import XCTest
import TCShellCore
@testable import TraceCommonsApp

final class ComputeNavigationTests: XCTestCase {
    private func directory() throws -> URL {
        let value = FileManager.default.temporaryDirectory
            .appendingPathComponent("tc-compute-navigation-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: value, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: value) }
        return value
    }

    @MainActor
    func testFreshContributorCanSelectComputeWithoutWatcherOrEnrollment() async throws {
        let root = try directory()
        let trace = AppModel()
        let compute = ComputeModel()
        let navigation = MainWindowNavigation()
        navigation.section = .compute
        await compute.start(configDirectory: root.path)
        XCTAssertTrue(navigation.displaysCompute)
        XCTAssertEqual(trace.startup, .starting)
        XCTAssertFalse(trace.status.loggedIn)
        XCTAssertFalse(trace.isOnboardingComplete)
        XCTAssertFalse(trace.traceNavigationReady)
        XCTAssertEqual(compute.snapshot?.state, "disabled")
        XCTAssertEqual(compute.snapshot?.available, false)
        XCTAssertNotNil(compute.copy)
        await compute.perform(.enable(ramAllowanceGiB: 8))
        XCTAssertEqual(compute.snapshot?.consentGranted, false)
        XCTAssertEqual(trace.startup, .starting)
        XCTAssertEqual(try FileManager.default.contentsOfDirectory(atPath: root.path), ["compute"])
        await compute.close()
    }

    @MainActor
    func testTraceRootsGateDoesNotHideComputeOrAdvanceTraceOnboarding() async throws {
        let root = try directory()
        let trace = AppModel()
        trace.start(configDirectory: root.path)
        XCTAssertEqual(trace.startup, .needsRoots, "fresh launch must refuse watcher before scanning")
        let compute = ComputeModel()
        await compute.start(configDirectory: root.path)
        let navigation = MainWindowNavigation()
        navigation.section = .compute
        XCTAssertTrue(navigation.displaysCompute)
        XCTAssertNotNil(compute.snapshot)
        for section: MainWindowView.Section in [.queue, .history, .settings] {
            navigation.section = section
            XCTAssertFalse(navigation.displaysCompute)
            XCTAssertEqual(trace.startup, .needsRoots)
            XCTAssertFalse(trace.traceNavigationReady)
            XCTAssertFalse(trace.isOnboardingComplete)
        }
        await compute.close()
        trace.shutdown()
    }

    @MainActor
    func testUnavailableSurfaceRendersWithoutTraceModel() async throws {
        let compute = ComputeModel()
        await compute.start(configDirectory: try directory().path)
        let renderer = ImageRenderer(content: ComputeContent(model: compute, allowance: .constant(""))
            .frame(width: 720, height: 520, alignment: .top).background(Color.white))
        let image = try XCTUnwrap(renderer.nsImage)
        XCTAssertEqual(image.size, NSSize(width: 720, height: 520))
        if let output = ProcessInfo.processInfo.environment["TC_COMPUTE_TEST_RENDER_PATH"] {
            let bitmap = try XCTUnwrap(NSBitmapImageRep(data: XCTUnwrap(image.tiffRepresentation)))
            try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: output))
        }
        await compute.close()
    }

    @MainActor
    func testInvalidComputeSettingsKeepNavigationAndSharedCopyAvailable() async throws {
        let root = try directory()
        let settings = root.appendingPathComponent("compute", isDirectory: true)
        try FileManager.default.createDirectory(at: settings, withIntermediateDirectories: true)
        try Data("invalid".utf8).write(to: settings.appendingPathComponent("settings.json"))
        let compute = ComputeModel()
        await compute.start(configDirectory: root.path)
        let navigation = MainWindowNavigation()
        navigation.section = .compute
        XCTAssertTrue(navigation.displaysCompute)
        XCTAssertNotNil(compute.copy)
        XCTAssertNil(compute.snapshot)
        XCTAssertNotNil(compute.failureLabel)
        await compute.close()
    }

    @MainActor
    func testShutdownWithoutWorkerIsSafeAndRetainsIdleController() async throws {
        let compute = ComputeModel()
        await compute.start(configDirectory: try directory().path)
        let first = await compute.shutdown(timeoutMilliseconds: 100)
        let second = await compute.shutdown(timeoutMilliseconds: 100)
        XCTAssertTrue(first)
        XCTAssertTrue(second)
        XCTAssertEqual(compute.snapshot?.workerStopped, true)
        await compute.close()
    }
    @MainActor
    func testFailedOpenCanRetryAfterSettingsAreRepaired() async throws {
        let root = try directory()
        let file = root.appendingPathComponent("compute/settings.json")
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Data("invalid".utf8).write(to: file)
        let compute = ComputeModel()
        await compute.start(configDirectory: root.path)
        XCTAssertNil(compute.snapshot)
        XCTAssertNotNil(compute.failureLabel)
        try FileManager.default.removeItem(at: file)
        await compute.retryOpen()
        XCTAssertNotNil(compute.snapshot)
        XCTAssertNil(compute.failureLabel)
        XCTAssertEqual(compute.snapshot?.consentGranted, false)
        await compute.close()
    }

    func testComputeDirectoryDoesNotInheritDaemonSocketLengthRestriction() throws {
        let path = "/private/tmp/" + String(repeating: "a", count: 120)
        XCTAssertThrowsError(try StateDirectory.resolve(explicit: path, probe: .init { _ in .absent }))
        XCTAssertEqual(try StateDirectory.resolveCompute(explicit: path, probe: .init { _ in .absent }).path, path)
        XCTAssertThrowsError(try StateDirectory.resolveCompute(explicit: path, probe: .init { _ in .file }))
    }

    @MainActor
    func testFailureRecoveryAndSubtitleUseHandleFreeRustCopy() throws {
        let model = ComputeModel()
        let copy = try XCTUnwrap(model.copy)
        XCTAssertFalse(try XCTUnwrap(copy.unavailable).isEmpty)
        XCTAssertFalse(try XCTUnwrap(copy.retry).isEmpty)
        XCTAssertFalse(try XCTUnwrap(copy.subtitle).isEmpty)
        XCTAssertFalse(try XCTUnwrap(copy.quitRefused).isEmpty)
    }

}
