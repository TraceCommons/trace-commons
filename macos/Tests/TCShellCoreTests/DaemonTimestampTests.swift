import Foundation
import XCTest
@testable import TCShellCore

/// Every timestamp the daemon sends goes through one parser, whose two
/// formatters are built once rather than once or twice per timestamp: a
/// `list_pending` of 180 entries spent about 180 ms building formatters.
final class DaemonTimestampTests: XCTestCase {
    /// The two formatters each timestamp used to build, for comparison.
    private static func reference(_ text: String) -> Date? {
        let withFraction = ISO8601DateFormatter()
        withFraction.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = withFraction.date(from: text) { return date }
        let plain = ISO8601DateFormatter()
        plain.formatOptions = [.withInternetDateTime]
        return plain.date(from: text)
    }

    static let shapes = [
        "2026-10-05T15:04:00Z", "2026-10-05T15:04:00.123Z", "2026-10-05T15:04:00.123456789Z",
        "2026-10-05T15:04:00+00:00", "2026-10-05T15:04:00.123456+00:00", "2026-10-05T15:04:00+05:30",
        "2026-10-05T15:04:00.5-07:00",
        // Not timestamps: each is refused, as before.
        "2026-10-05 15:04:00Z", "2026-10-05", "2026-10-05T15:04:00", "garbage", "",
    ]

    func test_everyShapeParsesAsTheFormattersDid() {
        for text in Self.shapes {
            XCTAssertEqual(DaemonTimestamp.parse(text), Self.reference(text), text)
            XCTAssertEqual(DaemonDataDecoding.parseDate(text), Self.reference(text), text)
        }
        XCTAssertNotNil(DaemonTimestamp.parse("2026-10-05T15:04:00.123456+00:00"))
        XCTAssertNil(DaemonTimestamp.parse("garbage"))
    }

    /// Many generated timestamps, every fraction length and offset, parse
    /// to the very same `Date` the per-timestamp formatters gave.
    func test_generatedTimestampsParseAsTheFormattersDid() {
        var state: UInt64 = 0x9E37_79B9_7F4A_7C15
        func next(_ bound: Int) -> Int {
            state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return Int((state >> 33) % UInt64(bound))
        }
        let base = ISO8601DateFormatter()
        for _ in 0..<2000 {
            let stamp = String(base.string(from: Date(timeIntervalSince1970: Double(1_600_000_000 + next(300_000_000))))
                .dropLast())
            let digits = next(10)
            let fraction = digits == 0 ? "" : "." + (0..<digits).map { _ in String(next(10)) }.joined()
            let text = stamp + fraction + ["Z", "+00:00", "-07:00", "+05:30"][next(4)]
            XCTAssertEqual(DaemonTimestamp.parse(text), Self.reference(text), text)
        }
    }

    /// Decoders on several threads at once share the one parser.
    func test_theParserIsSharedAcrossThreads() {
        let texts = Self.shapes
        DispatchQueue.concurrentPerform(iterations: 64) { i in
            let text = texts[i % texts.count]
            XCTAssertEqual(DaemonTimestamp.parse(text), Self.reference(text), text)
        }
    }

    /// No call site builds its own formatter per timestamp any more.
    func test_noDecoderBuildsAFormatterPerTimestamp() throws {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources")
        for rel in ["TCShellCore/BalanceSurface.swift", "TCShellCore/HarnessSurface.swift", "TraceCommonsApp/Models.swift"] {
            let source = try String(contentsOf: root.appendingPathComponent(rel), encoding: .utf8)
            XCTAssertFalse(source.contains("ISO8601DateFormatter()"), "\(rel) builds a formatter per call")
            XCTAssertTrue(source.contains("DaemonTimestamp.parse(text)"), rel)
        }
        // The parser's own two, built once.
        let models = try String(
            contentsOf: root.appendingPathComponent("TCShellCore/DataContract/DaemonDataModels.swift"), encoding: .utf8)
        XCTAssertEqual(models.components(separatedBy: "ISO8601DateFormatter()").count - 1, 2)
        XCTAssertTrue(models.contains("private static let formatters = Formatters()"))
    }
}
