import Foundation
import XCTest

/// The TCDesign sources, read as text, for the tests that check a rule by
/// reading the code. One scanner, so every source-reading test sees the same
/// files.
enum DesignSources {
    /// Every Swift file under `Sources/TCDesign`, as (file name, text),
    /// sorted by name.
    static func all() throws -> [(String, String)] {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TCDesign")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
            .compactMap { $0 as? URL }
            .filter { $0.pathExtension == "swift" }
        // A scan that finds nothing would pass every rule it checks.
        XCTAssertGreaterThanOrEqual(files.count, 8)
        return try files.map { ($0.lastPathComponent, try String(contentsOf: $0, encoding: .utf8)) }
            .sorted { $0.0 < $1.0 }
    }

    /// Components and styling: everything but the gallery, which is a
    /// development tool with placeholder words.
    static func components() throws -> [(String, String)] {
        try all().filter { !$0.0.hasPrefix("GlassGallery") }
    }
}
