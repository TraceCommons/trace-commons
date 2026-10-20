import Foundation
import XCTest
@testable import TraceCommonsApp

/// `InsightsDate.label` used to build two formatters per call; it now parses
/// through the shared `DaemonTimestamp`. Same label for the same text.
final class InsightsDateTests: XCTestCase {
    private static func oldLabel(_ raw: String) -> String {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        let date = formatter.date(from: raw) ?? ISO8601DateFormatter().date(from: raw)
        return date?.formatted(date: .abbreviated, time: .standard) ?? raw
    }

    func test_labelMatchesTheOldOneOverGeneratedInputs() {
        var state: UInt64 = 0x1234_5678_9ABC_DEF1
        func next(_ bound: Int) -> Int {
            state = state &* 6_364_136_223_846_793_005 &+ 1_442_695_040_888_963_407
            return Int((state >> 33) % UInt64(bound))
        }
        let base = ISO8601DateFormatter()
        var inputs = ["", "garbage", "2026-10-05", "2026-10-05T15:04:00", "2026-10-05 15:04:00Z"]
        for _ in 0..<3000 {
            let stamp = String(
                base.string(from: Date(timeIntervalSince1970: Double(1_600_000_000 + next(300_000_000)))).dropLast())
            let digits = next(10)
            let fraction = digits == 0 ? "" : "." + (0..<digits).map { _ in String(next(10)) }.joined()
            inputs.append(stamp + fraction + ["Z", "+00:00", "-07:00", "+05:30"][next(4)])
        }
        for text in inputs {
            XCTAssertEqual(InsightsDate.label(text), Self.oldLabel(text), text)
        }
    }
}
