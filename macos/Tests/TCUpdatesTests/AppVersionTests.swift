import Foundation
import XCTest

@testable import TCUpdates

/// Locates `tests/fixtures/update-conformance/` from this source file's own
/// path. The fixtures live outside the SwiftPM package directory because the
/// Rust updater reads the same files; copying them into a resource bundle
/// would produce two files that drift.
enum ConformanceFixtures {
    static var directory: URL {
        // .../macos/Tests/TCUpdatesTests/AppVersionTests.swift
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()   // TCUpdatesTests
            .deletingLastPathComponent()   // Tests
            .deletingLastPathComponent()   // macos
            .deletingLastPathComponent()   // <repo root>
            .appendingPathComponent("tests/fixtures/update-conformance")
    }

    static func json(_ name: String) throws -> [String: Any] {
        let url = directory.appendingPathComponent(name)
        let data = try Data(contentsOf: url)
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw NSError(
                domain: "ConformanceFixtures", code: 1,
                userInfo: [NSLocalizedDescriptionKey: "\(name) is not a JSON object"]
            )
        }
        return object
    }
}

final class AppVersionTests: XCTestCase {
    func testFixtureComparisonsAllHold() throws {
        let fixture = try ConformanceFixtures.json("version-comparison.json")
        let cases = try XCTUnwrap(fixture["comparisons"] as? [[String: Any]])
        XCTAssertGreaterThanOrEqual(cases.count, 10, "the fixture lost cases")

        for entry in cases {
            let current = try XCTUnwrap(entry["current"] as? String)
            let offered = try XCTUnwrap(entry["offered"] as? String)
            let expected = try XCTUnwrap(entry["newer"] as? Bool)
            let actual = try VersionComparison.isNewer(current: current, offered: offered)
            XCTAssertEqual(
                actual, expected,
                "isNewer(current: \(current), offered: \(offered)) should be \(expected)"
            )
        }
    }

    func testFixtureMalformedVersionsAreRefusedNotGuessed() throws {
        let fixture = try ConformanceFixtures.json("version-comparison.json")
        let cases = try XCTUnwrap(fixture["malformed"] as? [[String: Any]])
        XCTAssertGreaterThanOrEqual(cases.count, 8, "the fixture lost cases")

        for entry in cases {
            let current = try XCTUnwrap(entry["current"] as? String)
            let offered = try XCTUnwrap(entry["offered"] as? String)
            XCTAssertThrowsError(
                try VersionComparison.isNewer(current: current, offered: offered),
                "isNewer(current: \(current), offered: \(offered)) must throw, not guess"
            ) { error in
                XCTAssertTrue(error is VersionError, "wrong error type: \(error)")
            }
        }
    }

    func testEqualVersionIsNotNewerSoAReplayedManifestInstallsNothing() throws {
        XCTAssertFalse(try VersionComparison.isNewer(current: "1.4.2", offered: "1.4.2"))
    }

    func testOlderVersionIsNotNewerSoAReplayedOldManifestCannotDowngrade() throws {
        XCTAssertFalse(try VersionComparison.isNewer(current: "1.4.2", offered: "1.4.1"))
    }

    func testAppVersionParsesAndRoundTrips() throws {
        let version = try XCTUnwrap(AppVersion("12.34.56"))
        XCTAssertEqual(version.major, 12)
        XCTAssertEqual(version.minor, 34)
        XCTAssertEqual(version.patch, 56)
        XCTAssertEqual(version.description, "12.34.56")
    }

    func testAppVersionRejectsNonNumericComponents() {
        XCTAssertNil(AppVersion("1.2.beta"))
        XCTAssertNil(AppVersion("1.2"))
        XCTAssertNil(AppVersion("1.2.3.4"))
        XCTAssertNil(AppVersion("v1.2.3"))
        XCTAssertNil(AppVersion(""))
    }

    func testAppVersionOrders() throws {
        let older = try XCTUnwrap(AppVersion("1.9.0"))
        let newer = try XCTUnwrap(AppVersion("1.10.0"))
        XCTAssertLessThan(older, newer)
        XCTAssertEqual(try XCTUnwrap(AppVersion("1.2.3")), try XCTUnwrap(AppVersion("1.2.3")))
    }
}
