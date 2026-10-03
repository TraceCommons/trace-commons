import SwiftUI
import XCTest

@testable import TCDesign

/// The checkbox's accessibility representation is a native toggle, so the
/// system says checked, unchecked or mixed. A mixed group is two sources that
/// disagree; an unmixed box is the caller's binding alone, with no value
/// written over the system's.
final class CheckboxMixedTests: XCTestCase {
    func test_anUnmixedBoxIsTheCallersBindingAlone() {
        var value = true
        let binding = Binding(get: { value }, set: { value = $0 })
        let sources = GlassCheckboxStyle.sources(binding, mixed: false)
        XCTAssertEqual(sources.map(\.wrappedValue), [true])
    }

    func test_aMixedBoxReadsAsMixedToANativeToggle() {
        var value = false
        let binding = Binding(get: { value }, set: { value = $0 })
        let sources = GlassCheckboxStyle.sources(binding, mixed: true)
        XCTAssertEqual(Set(sources.map(\.wrappedValue)), [true, false])
    }

    func test_settingAMixedBoxWritesThroughToTheCaller() {
        var value = false
        let binding = Binding(get: { value }, set: { value = $0 })
        let sources = GlassCheckboxStyle.sources(binding, mixed: true)
        for source in sources { source.wrappedValue = true }
        XCTAssertTrue(value)
    }
}
