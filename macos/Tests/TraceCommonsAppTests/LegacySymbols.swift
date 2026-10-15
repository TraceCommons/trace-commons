import XCTest

/// What a rebuilt file may no longer name: the legacy palette and every
/// symbol `DesignSystem.swift` defined without a `TC.` prefix. That file and
/// `MainWindowView.swift` (home of `CenteredNotice`) are deleted; this list
/// keeps their names from coming back.
enum LegacySymbols {
    static let banned = [".tcType(", ".tcPrimaryAction(", ".tcCard(", ".tcColumn(", ".tcScreen(",
                         "TCFieldLabel", "TCTag(", "TCSectionHeader", "TCReadGateCheckbox",
                         "TCPrimaryButtonStyle", "QueueGlyph", "MacGlyph", "CenteredNotice(", "CommunityBrand"]

    static func assertClean(_ rel: String, file: StaticString = #filePath, line: UInt = #line) throws {
        let source = try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
        XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression), "\(rel) reads TC.", file: file, line: line)
        for symbol in banned {
            XCTAssertFalse(source.contains(symbol), "\(rel) still uses \(symbol)", file: file, line: line)
        }
    }
}
