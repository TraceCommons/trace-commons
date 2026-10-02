import SwiftUI
import XCTest

@testable import TCDesign

/// R4: the component states and rules the spec's "Components" and "Motion"
/// sections set, checked where a reading of the source can check them.
final class ComponentContractTests: XCTestCase {
    // MARK: Pressed

    func test_pressedIsTheFillEightPercentDarker() {
        XCTAssertEqual(GlassPress.darkening, 0.08)
        XCTAssertEqual(GlassPress.brightness(true), -0.08)
        XCTAssertEqual(GlassPress.brightness(false), 0)
        XCTAssertFalse(EnvironmentValues().glassPressed)
    }

    /// A plain-styled button has no pressed state at all. Every button in
    /// the components goes through `GlassButtonStyle` or `GlassPressStyle`.
    func test_noComponentButtonIsLeftWithoutAPressedState() throws {
        for (name, text) in try Self.componentSources() {
            XCTAssertFalse(text.contains(".buttonStyle(.plain)"), "\(name) has a button with no pressed state")
        }
    }

    /// Pressed darkens the fill, not the whole button: a darkened label
    /// loses contrast, and a fade reads as disabled.
    func test_pressedNeverDarkensOrFadesTheWholeButton() throws {
        for (name, text) in try Self.componentSources() {
            XCTAssertFalse(text.contains(".brightness(configuration.isPressed"), "\(name) darkens the whole label")
            XCTAssertFalse(text.contains(".opacity(configuration.isPressed"), "\(name) fades on press")
        }
    }

    // MARK: Wording

    /// Components author no words a person reads or hears: accessibility
    /// labels, values and hints come from the caller (the core's copy).
    func test_noComponentAuthorsAnAccessibilityWord() throws {
        // A string literal in an accessibility call. An interpolation
        // (`"\(count)"`) passes a value through and is not a word.
        let call = try NSRegularExpression(pattern: #"\.accessibility(Label|Value|Hint)\(.*"#)
        let literal = try NSRegularExpression(pattern: #""((?:[^"\\]|\\.)*)""#)
        let interpolation = try NSRegularExpression(pattern: #"\\\([^)]*\)"#)
        for (name, text) in try Self.componentSources() {
            var hits: [String] = []
            for line in text.components(separatedBy: "\n") {
                let whole = NSRange(line.startIndex..., in: line)
                guard let found = call.firstMatch(in: line, range: whole), let span = Range(found.range, in: line) else { continue }
                let tail = String(line[span])
                for match in literal.matches(in: tail, range: NSRange(tail.startIndex..., in: tail)) {
                    let body = String(tail[Range(match.range(at: 1), in: tail)!])
                    let bare = interpolation.stringByReplacingMatches(in: body, range: NSRange(body.startIndex..., in: body), withTemplate: "")
                    if bare.contains(where: \.isLetter) { hits.append(tail) }
                }
            }
            XCTAssertEqual(hits, [], "\(name) authors accessibility wording")
        }
    }

    func test_stepProgressSpeaksOnlyTheCallersStateWords() {
        let values = GlassStepProgress.StateValues(done: "d", current: "c", pending: "p")
        XCTAssertEqual(values.done, "d")
        // Constructible with none: a step then speaks only its label.
        _ = GlassStepProgress(labels: ["a", "b"], current: 0)
    }

    // MARK: Keyboard

    /// A tree row is not its own tab stop: the list holding it is one, and
    /// the arrow keys move its selection.
    func test_aListRowIsNotItsOwnTabStop() throws {
        let row = try XCTUnwrap(try Self.componentSources().first { $0.0 == "ListRow.swift" }?.1)
        XCTAssertFalse(row.contains(".focusable("))
    }

    /// Menus and popovers close on Escape through their caller.
    func test_menusAndPopoversHandleEscape() throws {
        let surfaces = try XCTUnwrap(try Self.componentSources().first { $0.0 == "Surfaces.swift" }?.1)
        XCTAssertEqual(surfaces.components(separatedBy: ".onExitCommand").count - 1, 2)
    }

    // MARK: Motion

    func test_oneCurveAtTheTokenDurations() {
        XCTAssertNil(GlassMotion.fast(true))
        XCTAssertNil(GlassMotion.standard(true))
        XCTAssertNotNil(GlassMotion.fast(false))
        XCTAssertEqual(GlassTokens.Motion.easeX1, 0.2)
        XCTAssertEqual(GlassTokens.Motion.easeY1, 0.8)
        XCTAssertEqual(GlassTokens.Motion.easeX2, 0.2)
        XCTAssertEqual(GlassTokens.Motion.easeY2, 1)
    }

    /// Every animation is `GlassMotion`: one curve, removed under Reduce
    /// Motion. A stock curve skips both.
    func test_noComponentUsesAStockCurve() throws {
        for (name, text) in try Self.sources() where name != "GlassMotion.swift" {
            for curve in [".easeOut", ".easeIn", ".easeInOut", ".spring", ".bouncy", ".snappy", ".smooth"] {
                XCTAssertFalse(text.contains("Animation\(curve)") || text.contains("(\(curve)"), "\(name) uses \(curve)")
            }
        }
    }

    // MARK: Sources

    /// Components and styling: everything but the gallery, which is a
    /// development tool with placeholder words.
    private static func componentSources() throws -> [(String, String)] {
        try sources().filter { !$0.0.hasPrefix("GlassGallery") }
    }

    private static func sources() throws -> [(String, String)] {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TCDesign")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: root, includingPropertiesForKeys: nil))
            .compactMap { $0 as? URL }
            .filter { $0.pathExtension == "swift" }
        return try files.map { ($0.lastPathComponent, try String(contentsOf: $0, encoding: .utf8)) }.sorted { $0.0 < $1.0 }
    }
}
