import XCTest

/// What a rebuilt file may no longer name: the legacy palette and every
/// `DesignSystem.swift` symbol without a `TC.` prefix, so Phase 4 can
/// delete that file. (`CenteredNotice` is `MainWindowView`'s, deleted with
/// it in Phase 4.)
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
