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

    /// The prompt is tertiary text, which clears 4.5:1 on the pane.
    func test_thePromptClearsTextContrast() {
        XCTAssertEqual(GlassTextField.promptInk, GlassTokens.Color.placeholder)
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
