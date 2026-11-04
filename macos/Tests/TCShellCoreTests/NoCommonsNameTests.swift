import Foundation
import XCTest

/// The commons' name stays out of everything this app shows.
///
/// Owner rule, 2026-11-02: "The name Trace Commons should currently not
/// appear anywhere in the app ... not in any user visible place." Most of
/// what this app says comes over the C ABI from the shared Rust copy, which
/// `crates/trace-commons-contributor/tests/no_commons_name_in_copy.rs`
/// guards. This covers what lives here: every Swift literal under
/// `Sources/`, and the recorded daemon samples the previews and tests show.
///
/// Comment lines are skipped. `TraceCommons` type and module names have no
/// space and are not matched.
final class NoCommonsNameTests: XCTestCase {
    static let sources = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent()   // TCShellCoreTests
        .deletingLastPathComponent()   // Tests
        .deletingLastPathComponent()   // macos
        .appendingPathComponent("Sources")

    static func findings(in text: String) -> [(Int, String)] {
        var found: [(Int, String)] = []
        for (index, line) in text.components(separatedBy: "\n").enumerated() {
            let trimmed = line.trimmingCharacters(in: .whitespaces)
            if trimmed.hasPrefix("//") { continue }
            let collapsed = trimmed
                .split(whereSeparator: { $0.isWhitespace })
                .joined(separator: " ")
            if collapsed.lowercased().contains("trace commons") {
                found.append((index + 1, collapsed))
            }
        }
        return found
    }

    func testTheScannerFindsTheNameAndSkipsComments() {
        XCTAssertEqual(Self.findings(in: #"Text("Connected to Trace Commons.")"#).count, 1)
        XCTAssertEqual(Self.findings(in: #""TRACE   COMMONS""#).count, 1)
        XCTAssertEqual(Self.findings(in: "// Trace Commons in a comment").count, 0)
        XCTAssertEqual(Self.findings(in: "import TraceCommonsApp").count, 0)
    }

    func testNothingUnderSourcesNamesTheCommons() throws {
        let walker = try XCTUnwrap(FileManager.default.enumerator(
            at: Self.sources, includingPropertiesForKeys: nil))
        var scanned = 0
        var offenders: [String] = []
        for case let url as URL in walker {
            guard ["swift", "json", "plist", "strings"].contains(url.pathExtension) else { continue }
            let text = try String(contentsOf: url, encoding: .utf8)
            scanned += 1
            let rel = url.path.replacingOccurrences(of: Self.sources.path + "/", with: "")
            for (line, content) in Self.findings(in: text) {
                offenders.append("\(rel):\(line): \(content)")
            }
        }
        XCTAssertGreaterThan(scanned, 100, "scanned only \(scanned) files; Sources/ moved")
        XCTAssertTrue(
            offenders.isEmpty,
            "The commons' name must not appear in anything a user sees (owner rule 2026-11-02):\n"
                + offenders.joined(separator: "\n"))
    }
}
