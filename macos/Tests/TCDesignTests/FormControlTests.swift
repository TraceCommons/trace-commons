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
}
