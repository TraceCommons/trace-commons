import Foundation
import XCTest
@testable import TraceCommonsApp

#if DEBUG
final class CaptureSuiteTests: XCTestCase {
    private func plist(_ name: String) -> URL {
        CaptureSuite.preferencesDirectory.appendingPathComponent(name + ".plist")
    }

    func testRunLeavesNoPlistBehind() {
        var name = ""
        CaptureSuite.run { defaults, suiteName in
            name = suiteName
            defaults.set(true, forKey: "monitor.showsMap")
            defaults.synchronize()
            XCTAssertTrue(suiteName.hasPrefix(CaptureSuite.prefix))
        }
        XCTAssertFalse(name.isEmpty)
        // cfprefsd flushes late; the file must stay gone, not just be gone once.
        Thread.sleep(forTimeInterval: 0.5)
        XCTAssertFalse(FileManager.default.fileExists(atPath: plist(name).path),
                       "a capture suite's plist must be deleted, removePersistentDomain alone leaves it")
    }

    func testSweepRemovesOnlyStaleCaptureSuites() throws {
        let dir = CaptureSuite.preferencesDirectory
        let stale = dir.appendingPathComponent(CaptureSuite.prefix + "stale-test-\(UUID().uuidString).plist")
        let fresh = dir.appendingPathComponent(CaptureSuite.prefix + "fresh-test-\(UUID().uuidString).plist")
        let other = dir.appendingPathComponent("not-a-capture-\(UUID().uuidString).plist")
        for url in [stale, fresh, other] { try Data("x".utf8).write(to: url) }
        defer { for url in [stale, fresh, other] { try? FileManager.default.removeItem(at: url) } }
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -7200)], ofItemAtPath: stale.path)
        try FileManager.default.setAttributes([.modificationDate: Date(timeIntervalSinceNow: -7200)], ofItemAtPath: other.path)

        CaptureSuite.sweepStale(olderThan: 3600)

        XCTAssertFalse(FileManager.default.fileExists(atPath: stale.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: fresh.path), "a run in progress keeps its suite")
        XCTAssertTrue(FileManager.default.fileExists(atPath: other.path), "only capture suites are touched")
    }
}
#endif
