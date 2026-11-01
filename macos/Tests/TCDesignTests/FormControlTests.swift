import AppKit
import SwiftUI
import XCTest

@testable import TCDesign

/// The form controls: text field, text area, select, radio group, check row.
@MainActor
final class FormControlTests: XCTestCase {
    private func size<V: View>(_ view: V) -> CGSize {
        NSHostingView(rootView: view.fixedSize()).fittingSize
    }

    func test_anInvalidEntryRingsTheFieldInTheOutsideRed() {
        XCTAssertEqual(GlassTextField.ring(invalid: true), GlassTokens.Color.statusOutside)
        XCTAssertNil(GlassTextField.ring(invalid: false))
    }

    /// The prompt is the placeholder ink, in the text field and the text
    /// area alike.
    func test_thePromptIsThePlaceholderInk() {
        XCTAssertEqual(GlassTextField.promptInk, GlassTokens.Color.placeholder)
    }

    /// A prompt is text, so it clears 4.5:1 on the field fill over every
    /// ground a field can sit on, in both appearances (owner ruling,
    /// 2026-10-07: hold the WCAG floors). #1146's 30% was 2.60 in dark and
    /// 2.03 in light.
    func test_thePromptClearsTextContrastOnTheFieldOverEveryGround() {
        typealias L = LightContrastGroundsTests
        let c = GlassTokens.Color.self
        let scene = L.solid(c.sceneBase.dark)
        let pane = L.solid(c.paneOpaque.dark)
        let darkGrounds: [(String, L.RGB)] = [
            ("sceneBase", scene),
            ("paneOpaque", pane),
            ("paneBase", L.over(c.paneBase.dark, scene)),
            ("well on pane", L.over(c.wellFill.dark, pane)),
            ("well on scene", L.over(c.wellFill.dark, scene)),
            ("popover", L.over(c.popoverFill.dark, scene)),
            ("menu", L.over(c.menuFill.dark, scene)),
            ("node card", L.over(c.nodeCardFill.dark, L.solid(c.mapFieldOuter.dark))),
        ]
        let ink = GlassTextField.promptInk
        for (dark, grounds) in [(true, darkGrounds), (false, L.lightGrounds)] {
            for (name, ground) in grounds {
                let field = L.over(dark ? c.fieldFill.dark : c.fieldFill.light, ground)
                let prompt = L.over(dark ? ink.dark : ink.light, field)
                let ratio = L.contrast(prompt, field)
                XCTAssertGreaterThanOrEqual(ratio, 4.5, "\(dark ? "dark" : "light") prompt on \(name): \(ratio)")
            }
        }
    }

    /// A secure field and a plain one lay out alike.
    func test_aSecureFieldIsTheSameSizeAsAPlainOne() {
        let plain = size(GlassTextField("Port", text: .constant("x")).frame(width: 200))
        let secure = size(GlassTextField("Port", text: .constant("x"), secure: true).frame(width: 200))
        XCTAssertEqual(plain.height, secure.height, accuracy: 0.5)
        XCTAssertGreaterThanOrEqual(plain.height, GlassTokens.Size.controlLarge)
        let bare = size(GlassTextField("Port", text: .constant("x"), showsLabel: false).frame(width: 200))
        XCTAssertLessThan(bare.height, plain.height)
    }

    func test_aTextAreaIsAtLeastItsMinimumHeight() {
        let area = size(GlassTextArea("Notes", text: .constant(""), showsLabel: false).frame(width: 240))
        XCTAssertGreaterThanOrEqual(area.height, GlassTokens.Size.textAreaMinHeight)
    }

    func test_theSelectShowsItsSelection() {
        let options = [GlassPickerOption("A", value: 1), GlassPickerOption("B", value: 2)]
        XCTAssertEqual(GlassSelect<Int>.current(2, in: options)?.title, "B")
        XCTAssertNil(GlassSelect<Int>.current(3, in: options))
    }

    /// A dot is never the only signal: an option's detail is spoken with its
    /// title, as the picker's accessibility value.
    func test_aPickerOptionsDetailIsSpokenWithItsTitle() {
        let plain = GlassPickerOption("A", value: 1)
        let detailed = GlassPickerOption("B", value: 2, dot: .on, detail: "Heading")
        XCTAssertEqual(detailed.detail, "Heading")
        XCTAssertEqual(GlassPicker<Int>.spoken(plain), "A")
        XCTAssertEqual(GlassPicker<Int>.spoken(detailed), "B, Heading")
    }

    /// The open menu shows an option's tier too, not only the closed pill
    /// (#1797 review): its dot is a colour image that is not a template,
    /// which a menu would draw in its own monochrome, at the pill's dot
    /// size; an option with no heading of its own carries its detail as the
    /// item's subtitle.
    func test_aPickerMenuItemDrawsItsDotInColourAndItsDetail() throws {
        for status in [GlassStatus.on, .ask, .outside] {
            let image = GlassPickerDot.image(status)
            XCTAssertFalse(image.isTemplate)
            XCTAssertEqual(image.size, NSSize(width: GlassTokens.Size.dot, height: GlassTokens.Size.dot))
            for name in [NSAppearance.Name.aqua, .darkAqua] {
                let appearance = try XCTUnwrap(NSAppearance(named: name))
                let drawn = try Self.centre(appearance) { image.draw(in: $0) }
                let expected = try Self.centre(appearance) { rect in
                    status.rgba.dynamicNSColor.setFill()
                    rect.fill()
                }
                XCTAssertEqual(drawn, expected, "\(status) \(name.rawValue)")
            }
        }
        // One image per status, made once: the same object every render
        // (#1798 review).
        XCTAssertTrue(GlassPickerDot.image(.on) === GlassPickerDot.image(.on))
        XCTAssertFalse(GlassPickerDot.image(.on) === GlassPickerDot.image(.ask))
        let headless = GlassPickerOption("m", value: 1, dot: .on, detail: "Heading")
        let grouped = GlassPickerOption("m", value: 2, dot: .on, section: "Heading", detail: "Heading")
        XCTAssertEqual(GlassPicker<Int>.menuDetail(headless, subtitles: true), "Heading")
        XCTAssertNil(GlassPicker<Int>.menuDetail(grouped, subtitles: true))
        XCTAssertNil(GlassPicker<Int>.menuDetail(GlassPickerOption("m", value: 3), subtitles: true))
        // Before macOS 14.4 a menu item has no subtitle, so none is drawn;
        // the item's one accessibility label still says the tier.
        XCTAssertNil(GlassPicker<Int>.menuDetail(headless, subtitles: false))
        XCTAssertEqual(GlassPicker<Int>.spoken(headless), "m, Heading")
    }

    /// #1798 review: menu dots are opt-in, so a picker that passes a dot
    /// for its closed pill draws nothing new in its open menu.
    func test_menuDotsAreOptIn() {
        let options = [GlassPickerOption("A", value: 1, dot: .on)]
        let plain = GlassPicker("Pick", selection: .constant(1), options: options, placeholder: "None")
        XCTAssertFalse(plain.menuDots)
        let dotted = GlassPicker("Pick", selection: .constant(1), options: options, placeholder: "None", menuDots: true)
        XCTAssertTrue(dotted.menuDots)
    }

    /// The centre pixel of `draw` in `appearance`, as RGBA bytes.
    private static func centre(_ appearance: NSAppearance, _ draw: (NSRect) -> Void) throws -> [Int] {
        let side = 21
        let rep = try XCTUnwrap(NSBitmapImageRep(
            bitmapDataPlanes: nil, pixelsWide: side, pixelsHigh: side, bitsPerSample: 8,
            samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB,
            bytesPerRow: 0, bitsPerPixel: 0))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
        appearance.performAsCurrentDrawingAppearance {
            draw(NSRect(x: 0, y: 0, width: side, height: side))
        }
        NSGraphicsContext.restoreGraphicsState()
        var pixel = [Int](repeating: 0, count: 4)
        rep.getPixel(&pixel, atX: side / 2, y: side / 2)
        return pixel
    }

    /// The arrow keys move between the enabled options, wrapping, as a
    /// native radio group does.
    func test_theArrowKeysMoveBetweenEnabledRadios() {
        let options = [
            GlassRadioOption("A", value: "a"),
            GlassRadioOption("B", value: "b", isEnabled: false),
            GlassRadioOption("C", value: "c"),
        ]
        XCTAssertEqual(GlassRadioGroup<String>.moved("a", by: 1, in: options), "c")
        XCTAssertEqual(GlassRadioGroup<String>.moved("c", by: 1, in: options), "a")
        XCTAssertEqual(GlassRadioGroup<String>.moved("a", by: -1, in: options), "c")
        XCTAssertEqual(GlassRadioGroup<String>.moved("x", by: 1, in: options), "a")
        XCTAssertEqual(GlassRadioGroup<String>.moved("x", by: -1, in: options), "c")
        let none = [GlassRadioOption("B", value: "b", isEnabled: false)]
        XCTAssertEqual(GlassRadioGroup<String>.moved("b", by: 1, in: none), "b")
    }

    /// The radio row's answer is optional, and the keyboard and assistive
    /// tech can take it back as the pointer can: Space or Return on the
    /// row presses the chosen radio again, and the picker it reads as
    /// passes a "none" choice through as that same press.
    func test_aRadioRowsAnswerCanBeTakenBackWithoutAPointer() {
        typealias Row = GlassRadioRow<String>
        // Space or Return: the chosen answer again, which the caller takes
        // back; with nothing chosen there is nothing to take back.
        XCTAssertEqual(Row.keyPressed("b"), "b")
        XCTAssertNil(Row.keyPressed(nil))
        // The picker: none sends the chosen answer again, another answer
        // is sent as it is, and the chosen one again is no change.
        XCTAssertEqual(Row.picked(nil, selection: "b"), "b")
        XCTAssertEqual(Row.picked("c", selection: "b"), "c")
        XCTAssertEqual(Row.picked("c", selection: nil), "c")
        XCTAssertNil(Row.picked("b", selection: "b"))
        XCTAssertNil(Row.picked(nil, selection: nil))
    }

    /// The arrow keys move the row's answer through the enabled radios;
    /// with nothing chosen, the first one is chosen.
    func test_theArrowKeysMoveARadioRowsAnswer() {
        typealias Row = GlassRadioRow<String>
        let options = [
            GlassRadioOption("A", value: "a"),
            GlassRadioOption("B", value: "b", isEnabled: false),
            GlassRadioOption("C", value: "c"),
        ]
        XCTAssertEqual(Row.moved(nil, by: 1, in: options), "a")
        XCTAssertEqual(Row.moved(nil, by: -1, in: options), "a")
        XCTAssertEqual(Row.moved("a", by: 1, in: options), "c")
        XCTAssertEqual(Row.moved("a", by: -1, in: options), "c")
        XCTAssertNil(Row.moved(nil, by: 1, in: [GlassRadioOption("B", value: "b", isEnabled: false)]))
    }

    /// To assistive tech the row is a radio-group picker that offers
    /// "none" first, under the caller's word, then each answer.
    func test_aRadioRowsPickerOffersNone() {
        let choices = GlassRadioRow<String>.choices(
            none: "Nothing", options: [GlassRadioOption("A", value: "a"), GlassRadioOption("C", value: "c")])
        XCTAssertEqual(choices.map(\.title), ["Nothing", "A", "C"])
        XCTAssertEqual(choices.map(\.value), [nil, "a", "c"])
    }

    /// The pure functions above are what the row's body runs: Space and
    /// Return go through `keyPressed`, the arrow keys through `moved`, and
    /// the picker for assistive tech lists `choices` and passes its change
    /// through `picked`. Read from the source, as there is no UI driver,
    /// so a body that stops calling them fails here rather than passing.
    func test_theRadioRowsBodyRunsItsChooseLogic() throws {
        let sources = Dictionary(uniqueKeysWithValues: try DesignSources.components())
        let forms = try XCTUnwrap(sources["Forms.swift"])
        let start = try XCTUnwrap(forms.range(of: "public struct GlassRadioRow"))
        let end = try XCTUnwrap(forms.range(of: "// MARK: - Check row", range: start.upperBound..<forms.endIndex))
        let row = String(forms[start.lowerBound..<end.lowerBound])
        XCTAssertTrue(row.contains(".onKeyPress(keys: [.space, .return])"), "Space or Return no longer reaches the row")
        XCTAssertTrue(row.contains("Self.keyPressed(selection)"), "the key press does not take the answer back")
        XCTAssertTrue(row.contains("Self.moved(selection, by: step, in: options)"), "the arrow keys bypass `moved`")
        XCTAssertTrue(row.contains("Self.picked($0, selection: selection)"), "the picker ignores a clear")
        XCTAssertTrue(row.contains("Self.choices(none: none, options: options)"), "the picker offers no none choice")
    }

    func test_theRadioIsTheCheckboxSize() {
        XCTAssertEqual(GlassTokens.Size.radio, GlassTokens.Size.checkbox)
        let radio = size(GlassRadio(checked: true))
        XCTAssertEqual(radio.width, GlassTokens.Size.radio, accuracy: 0.5)
        XCTAssertEqual(radio.height, GlassTokens.Size.radio, accuracy: 0.5)
    }

    /// A check row's sentence wraps rather than running on.
    func test_aCheckRowsSentenceWraps() {
        let long = String(repeating: "word ", count: 40)
        let one = NSHostingView(rootView: GlassCheckRow("word", isOn: .constant(true)).frame(width: 200)).fittingSize
        let many = NSHostingView(rootView: GlassCheckRow(long, isOn: .constant(true)).frame(width: 200)).fittingSize
        XCTAssertGreaterThan(many.height, one.height)
    }

    /// A menu's choices draw the system's one check mark: toggles, never an
    /// inline picker, which drew two (owner, 2026-10-20). A toggle only
    /// chooses; turning the chosen one off keeps it.
    func test_menuChoicesCheckOnce() throws {
        var chosen: Int? = 2
        let two = GlassMenuChoices<Int>.isOn(2, chosen: chosen) { chosen = $0 }
        let three = GlassMenuChoices<Int>.isOn(3, chosen: chosen) { chosen = $0 }
        XCTAssertTrue(two.wrappedValue)
        XCTAssertFalse(three.wrappedValue)
        two.wrappedValue = false
        XCTAssertEqual(chosen, 2)
        three.wrappedValue = true
        XCTAssertEqual(chosen, 3)
        for file in ["Forms.swift", "Controls.swift"] {
            let url = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
                .deletingLastPathComponent().appendingPathComponent("Sources/TCDesign/Components/" + file)
            let source = try String(contentsOf: url, encoding: .utf8)
            XCTAssertFalse(source.contains(".pickerStyle(.inline)"), "\(file) draws a menu's choices as an inline picker")
        }
    }
}
