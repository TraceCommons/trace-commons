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
