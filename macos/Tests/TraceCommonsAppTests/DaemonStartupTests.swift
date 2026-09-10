import Foundation
import TCBridge
import XCTest
@testable import TraceCommonsApp

private final class StartupCalls: @unchecked Sendable {
    private let lock = NSLock()
    private var value = 0

    func increment() -> Int {
        lock.lock()
        defer { lock.unlock() }
        value += 1
        return value
    }

    var count: Int {
        lock.lock()
        defer { lock.unlock() }
        return value
    }
}

final class DaemonStartupTests: XCTestCase {
    private let settings = #"{"claude_source":{"mode":"off"},"codex_source":{"mode":"off"}}"#

    private func directory() throws -> URL {
        // Short enough for the actual Unix socket, including on macOS.
        let directory = URL(fileURLWithPath: "/private/tmp/tc-start-\(UUID().uuidString.prefix(8))")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: directory) }
        return directory
    }

    @MainActor
    func testModelStartupYieldsAndSuppressesDuplicatesBeforePublishingSuccess() async throws {
        let directory = try directory()
        let entered = expectation(description: "Constructor entered")
        let completed = expectation(description: "Running state published")
        let responsive = expectation(description: "Main actor remains responsive")
        let release = DispatchSemaphore(value: 0)
        defer { release.signal() }
        let calls = StartupCalls()
        let owner = DaemonStartup(construct: { path, settings in
            XCTAssertFalse(Thread.isMainThread)
            _ = calls.increment()
            entered.fulfill()
            guard release.wait(timeout: .now() + 10) == .success else {
                throw TCDaemon.TCError.daemonGone
            }
            return try TCDaemon(configDir: path, settingsJSON: settings)
        })
        let model = AppModel(daemonStartup: owner)
        defer { model.shutdown() }
        model.startDaemon(at: directory.path, settingsJSON: settings) { state in
            XCTAssertTrue(Thread.isMainThread)
            XCTAssertEqual(state, .running)
            completed.fulfill()
        }
        await fulfillment(of: [entered], timeout: 5)
        XCTAssertTrue(model.isStartingDaemon)
        XCTAssertEqual(model.startup, .starting)
        model.startDaemon(at: directory.path, settingsJSON: settings) { _ in
            XCTFail("Duplicate start reached completion")
        }
        DispatchQueue.main.async { responsive.fulfill() }
        await fulfillment(of: [responsive], timeout: 2)
        XCTAssertEqual(calls.count, 1)
        release.signal()
        await fulfillment(of: [completed], timeout: 10)
        XCTAssertFalse(model.isStartingDaemon)
        XCTAssertEqual(model.startup, .running)
    }

    @MainActor
    func testRootsRefusalCompletesAsynchronouslyAndAllowsSettingsRetry() async throws {
        let directory = try directory()
        let model = AppModel()
        defer { model.shutdown() }
        let refused = expectation(description: "Roots refusal delivered")
        model.startDaemon(at: directory.path, settingsJSON: nil) { state in
            XCTAssertEqual(state, .needsRoots)
            refused.fulfill()
        }
        await fulfillment(of: [refused], timeout: 10)
        XCTAssertFalse(model.isStartingDaemon)
        let started = expectation(description: "Declared roots start the daemon")
        model.startDaemon(at: directory.path, settingsJSON: settings) { state in
            XCTAssertEqual(state, .running)
            started.fulfill()
        }
        await fulfillment(of: [started], timeout: 10)
        XCTAssertFalse(model.isStartingDaemon)
    }

    @MainActor
    func testShutdownRetiresLateSuccessfulStartupWithoutPublishingIt() async throws {
        let directory = try directory()
        let entered = expectation(description: "Constructor blocked")
        let retired = expectation(description: "Late daemon retired")
        let release = DispatchSemaphore(value: 0)
        defer { release.signal() }
        let owner = DaemonStartup(construct: { path, settings in
            entered.fulfill()
            guard release.wait(timeout: .now() + 10) == .success else {
                throw TCDaemon.TCError.daemonGone
            }
            return try TCDaemon(configDir: path, settingsJSON: settings)
        }, shutdown: { daemon in
            XCTAssertFalse(Thread.isMainThread)
            _ = daemon.shutdown()
            retired.fulfill()
        })
        let model = AppModel(daemonStartup: owner)
        model.startDaemon(at: directory.path, settingsJSON: settings) { _ in
            XCTFail("Cancelled startup published a late result")
        }
        await fulfillment(of: [entered], timeout: 5)
        model.shutdown()
        XCTAssertFalse(model.isStartingDaemon)
        release.signal()
        await fulfillment(of: [retired], timeout: 10)
        XCTAssertEqual(model.startup, .starting)
        XCTAssertFalse(FileManager.default.fileExists(atPath: directory.appendingPathComponent("daemon.lock").path))
    }
}
