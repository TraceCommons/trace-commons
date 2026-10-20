import Foundation
import XCTest
@testable import TCShellCore

/// Follows the `DaemonTimestamp` finding: a formatter costs about 0.33 ms to
/// build, so the ones called per row are built once. Each test compares the
/// shared version with the per-call one it replaced.
final class SharedFormatterTests: XCTestCase {
    /// `SourceCandidate.Wire.parseTimestamp` as it was, two formatters per call.
    private static func oldParseTimestamp(_ raw: String) -> Date? {
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]

        if let date = fractional.date(from: raw) ?? plain.date(from: raw) {
            return date
        }
        guard let dot = raw.firstIndex(of: "."),
            let zone = raw[dot...].firstIndex(where: { $0 == "Z" || $0 == "+" || $0 == "-" })
        else { return nil }
        let digits = raw[raw.index(after: dot)..<zone].prefix(3)
        let truncated = raw[..<dot] + "." + digits + raw[zone...]
        return fractional.date(from: String(truncated))
    }

    private static let shapes = [
        "2026-08-19T12:28:34.412518838Z", "2026-08-19T12:28:34.412Z", "2026-08-19T12:28:34Z",
        "2026-08-19T12:28:34+00:00", "2026-08-19T12:28:34.412518838+00:00",
        "2026-08-19T12:28:34.5-07:00", "2026-08-19T12:28:34.123456+05:30",
        "2026-08-19T12:28:34.Z", "2026-08-19T12:28:34.", "2026-08-19T12:28:34.1234567890123Z",
        // Not timestamps.
        "2026-08-19 12:28:34Z", "2026-08-19", "2026-08-19T12:28:34", "garbage", "", ".", "Z", "1.2.3",
    ]

    private struct Lcg {
        var state: UInt64 = 0x9E37_79B9_7F4A_7C15
        mutating func next(_ bound: Int) -> Int {
            state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return Int((state >> 33) % UInt64(bound))
        }
    }

    func test_parseTimestampMatchesTheOldOneOnEveryShape() {
        for text in Self.shapes {
            XCTAssertEqual(SourceCandidate.Wire.parseTimestamp(text), Self.oldParseTimestamp(text), text)
        }
        XCTAssertNotNil(SourceCandidate.Wire.parseTimestamp("2026-08-19T12:28:34.412518838Z"))
        XCTAssertNil(SourceCandidate.Wire.parseTimestamp("garbage"))
    }

    func test_parseTimestampMatchesTheOldOneOverGeneratedTimestamps() {
        var rng = Lcg()
        let base = ISO8601DateFormatter()
        for _ in 0..<3000 {
            let stamp = String(
                base.string(from: Date(timeIntervalSince1970: Double(1_600_000_000 + rng.next(300_000_000))))
                    .dropLast())
            let digits = rng.next(13)
            let fraction = digits == 0 ? "" : "." + (0..<digits).map { _ in String(rng.next(10)) }.joined()
            var text = stamp + fraction + ["Z", "+00:00", "-07:00", "+05:30"][rng.next(4)]
            // A few corrupted ones, which must be refused the same way.
            if rng.next(10) == 0 {
                text = String(text.dropLast(rng.next(4) + 1))
            }
            XCTAssertEqual(SourceCandidate.Wire.parseTimestamp(text), Self.oldParseTimestamp(text), text)
        }
    }

    func test_formatBytesMatchesAFreshFormatter() {
        var rng = Lcg()
        var counts = [0, 1, 999, 1000, 1024, 1_000_000, Int.max / 4]
        for _ in 0..<3000 { counts.append(rng.next(1 << 40) >> rng.next(30)) }
        for count in counts {
            let fresh = ByteCountFormatter()
            fresh.countStyle = .file
            fresh.allowedUnits = [.useKB, .useMB, .useGB]
            XCTAssertEqual(Format.bytes(count), fresh.string(fromByteCount: Int64(count)), "\(count)")
        }
    }

    func test_formatWhenMatchesAFreshFormatterAtAFixedNow() {
        var rng = Lcg()
        let now = Date(timeIntervalSince1970: 1_790_000_000)
        for _ in 0..<3000 {
            let offset = Double(rng.next(400_000_000) - 200_000_000) + Double(rng.next(1000)) / 1000
            let date = now.addingTimeInterval(offset)
            let fresh = RelativeDateTimeFormatter()
            fresh.unitsStyle = .full
            XCTAssertEqual(
                Format.when(date, relativeTo: now), fresh.localizedString(for: date, relativeTo: now), "\(offset)")
        }
    }

    /// `when(_:)` still reads the clock on each call.
    func test_formatWhenIsRelativeToNowOnEachCall() {
        let fresh = RelativeDateTimeFormatter()
        fresh.unitsStyle = .full
        let date = Date().addingTimeInterval(-3 * 86_400)
        XCTAssertEqual(Format.when(date), fresh.localizedString(for: date, relativeTo: Date()))
        XCTAssertEqual(Format.when(Date().addingTimeInterval(-3600)), "1 hour ago")
        XCTAssertEqual(Format.when(Date().addingTimeInterval(-7200)), "2 hours ago")
    }

    /// Counts constructions, not time: thousands of calls build nothing new.
    func test_manyCallsBuildNoNewFormatter() {
        _ = Format.bytes(1)
        _ = Format.when(Date())
        let before = Format.formattersBuilt
        for i in 0..<5000 {
            _ = Format.bytes(i * 1000)
            _ = Format.when(Date(timeIntervalSinceNow: Double(-i)))
            _ = SourceCandidate.Wire.parseTimestamp("2026-08-19T12:28:34.412518838Z")
        }
        XCTAssertEqual(Format.formattersBuilt, before)
        XCTAssertEqual(before, 2)
    }

    func test_sharedFormatsAreSafeAcrossThreads() {
        let now = Date(timeIntervalSince1970: 1_790_000_000)
        let expectedWhen = Format.when(now.addingTimeInterval(-90_000), relativeTo: now)
        let expectedBytes = Format.bytes(5_000_000)
        DispatchQueue.concurrentPerform(iterations: 200) { _ in
            XCTAssertEqual(Format.when(now.addingTimeInterval(-90_000), relativeTo: now), expectedWhen)
            XCTAssertEqual(Format.bytes(5_000_000), expectedBytes)
            XCTAssertNotNil(SourceCandidate.Wire.parseTimestamp("2026-08-19T12:28:34.412518838Z"))
        }
    }

    /// No per-call construction is left at the sites this change covers.
    func test_noPerCallFormatterRemainsAtTheCoveredSites() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources")
        for rel in ["TCShellCore/SourceCandidate.swift", "TraceCommonsApp/Views/InsightsView.swift"] {
            let source = try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
            XCTAssertFalse(source.contains("ISO8601DateFormatter()"), "\(rel) builds a formatter per call")
        }
    }
}
