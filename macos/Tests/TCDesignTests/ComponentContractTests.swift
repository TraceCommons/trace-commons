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
        // A control with no fill of its own darkens what is behind its label
        // by the same 8%, as a wash under the label.
        XCTAssertEqual(GlassPress.washOpacity(true), 0.08)
        XCTAssertEqual(GlassPress.washOpacity(false), 0)
    }

    /// The tab label and the toolbar glyph are labels: pressed darkens a fill
    /// or a wash behind them, never the label itself.
    func test_pressedNeverDarkensATabLabelOrAToolbarGlyph() throws {
        let sources = Dictionary(uniqueKeysWithValues: try Self.componentSources())
        let navigation = try XCTUnwrap(sources["Navigation.swift"])
        let tabs = try XCTUnwrap(navigation.range(of: "struct GlassSegmentedTabs")
            .map { String(navigation[$0.lowerBound...].prefix(4000)) })
        XCTAssertFalse(tabs.contains("}\n                    .glassPressedFill()"), "the tab label darkens")
        XCTAssertTrue(tabs.contains(".glassPressedWash("), "the tab has no pressed wash")
        let ruleTabs = try XCTUnwrap(navigation.range(of: "struct GlassRuleTabs")
            .map { String(navigation[$0.lowerBound...].prefix(5000)) })
        XCTAssertFalse(ruleTabs.contains(".glassPressedFill()"), "the rule tab label darkens")
        XCTAssertTrue(ruleTabs.contains(".glassPressedWash("), "the rule tab has no pressed wash")
        let controls = try XCTUnwrap(sources["Controls.swift"])
        let toolbar = try XCTUnwrap(controls.range(of: "struct GlassToolbarButton")
            .map { String(controls[$0.lowerBound...].prefix(1500)) })
        XCTAssertFalse(toolbar.contains(".glassPressedFill()"), "the toolbar glyph darkens")
        XCTAssertTrue(toolbar.contains(".glassPressedWash("), "the toolbar glyph has no pressed wash")
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

    /// Each step's accessibility value is the caller's word for where it
    /// stands, and with no words a step speaks only its label.
    @MainActor
    func test_stepProgressSpeaksOnlyTheCallersStateWords() {
        let values = GlassStepProgress.StateValues(done: "d", current: "c", pending: "p")
        let progress = GlassStepProgress(labels: ["a", "b", "c"], current: 1, stateValues: values)
        XCTAssertEqual((0..<3).map(progress.value(at:)), ["d", "c", "p"])

        let wordless = GlassStepProgress(labels: ["a", "b", "c"], current: 1)
        XCTAssertEqual((0..<3).map(wordless.value(at:)), ["", "", ""])
    }

    // MARK: Keyboard

    /// A tree row is not its own tab stop: the list holding it is one, and
    /// the arrow keys move its selection.
    ///
    /// R6 CONTRACT: no TCDesign list provides that yet. Keyboard reach for a
    /// row therefore rests on the screen that holds the rows (R6's Traces
    /// tree), which must give the list focus, arrow-key selection and
    /// Return/Space. Do not ship a screen of rows without it.
    ///
    /// The one `.focusable` the row may carry is its pill's opt-out, which
    /// only ever takes a stop away (roving focus: the selected row's pill is
    /// the only one); nothing in the row may make itself a stop.
    func test_aListRowIsNotItsOwnTabStop() throws {
        let row = try XCTUnwrap(try Self.componentSources().first { $0.0 == "ListRow.swift" }?.1)
        let calls = row.components(separatedBy: ".focusable(").dropFirst().map { $0.prefix { $0 != ")" } }
        XCTAssertEqual(calls, ["submitFocusable"])
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

    private static func componentSources() throws -> [(String, String)] {
        try DesignSources.components()
    }

    private static func sources() throws -> [(String, String)] {
        try DesignSources.all()
    }
}
