import Foundation
import XCTest
@testable import TCShellCore

/// The anonymous usage counts' names (#1920) are exactly the protocol
/// crate's list, in its order: the daemon refuses any other name, so a
/// Swift case that drifted from the Rust would be dropped on every report
/// without a word.
final class UsageEventTests: XCTestCase {
    /// `crates/trace-commons-protocol/src/usage_analytics.rs`, the one
    /// definition of the list.
    static let rustSource = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()  // TCShellCoreTests
        .deletingLastPathComponent()  // Tests
        .deletingLastPathComponent()  // macos
        .deletingLastPathComponent()  // repo
        .appendingPathComponent("crates/trace-commons-protocol/src/usage_analytics.rs")

    /// The quoted names inside `pub const USAGE_EVENTS: &[&str] = &[ ... ];`.
    static func rustEvents() throws -> [String] {
        let source = try String(contentsOf: rustSource, encoding: .utf8)
        let start = try XCTUnwrap(source.range(of: "pub const USAGE_EVENTS: &[&str] = &["), "the list moved")
        let rest = source[start.upperBound...]
        let end = try XCTUnwrap(rest.range(of: "];"), "the list never closes")
        let body = String(rest[..<end.lowerBound])
        let quoted = try NSRegularExpression(pattern: #""([a-z_]+)""#)
        return quoted.matches(in: body, range: NSRange(body.startIndex..., in: body)).compactMap { match in
            Range(match.range(at: 1), in: body).map { String(body[$0]) }
        }
    }

    func testTheCasesAreTheProtocolListInOrder() throws {
        let rust = try Self.rustEvents()
        XCTAssertEqual(rust.count, 12, "the protocol's list changed size: \(rust)")
        XCTAssertEqual(UsageEvent.allCases.map(\.rawValue), rust)
    }

    /// Sample and preview clients count nothing and send nothing.
    func testASampleClientCountsNothing() async {
        let client = SampleDaemonClient(.normalDay)
        for event in UsageEvent.allCases {
            let recorded = await client.recordUsageEvent(event)
            XCTAssertFalse(recorded, event.rawValue)
        }
    }
}
