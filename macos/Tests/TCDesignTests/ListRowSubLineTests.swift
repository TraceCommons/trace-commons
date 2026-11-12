import Foundation
import XCTest

/// A row's sub-line carries whole sentences -- a size beside a state, such as
/// why a trace is held -- and a sentence cut off at the row's edge is one the
/// person never reads (owner, 2026-11-11). The sub-line wraps, and grows the
/// row, which has a minimum height and no fixed one.
final class ListRowSubLineTests: XCTestCase {
    func testTheSubLineWrapsInsteadOfClipping() throws {
        let source = try Self.listRowSource()
        let start = try XCTUnwrap(source.range(of: "Text(sub)"), "the sub-line is drawn")
        let tail = String(source[start.upperBound...].prefix(400))
        let modifiers = tail.components(separatedBy: "\n").prefix(6).joined(separator: "\n")
        XCTAssertFalse(modifiers.contains(".lineLimit(1)"), "the sub-line is cut to one line:\n\(modifiers)")
        XCTAssertTrue(
            modifiers.contains(".fixedSize(horizontal: false, vertical: true)"),
            "the sub-line must take the height its text needs:\n\(modifiers)")
        XCTAssertTrue(source.contains(".frame(minHeight: GlassTokens.Size.listRow)"), "the row grows with it")
    }

    private static func listRowSource() throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Sources/TCDesign/Components/ListRow.swift")
        return try String(contentsOf: url, encoding: .utf8)
    }
}
