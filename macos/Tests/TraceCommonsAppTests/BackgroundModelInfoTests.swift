import XCTest
@testable import TraceCommonsApp

/// The Launch options' background model picker has an "i" beside its
/// title that says what the background model is (owner, 2026-11-11). Its
/// words are the core's (`background_model_info`), never Swift's.
@MainActor
final class BackgroundModelInfoTests: XCTestCase {
    func testTheExplanationIsTheCoresApprovedWording() {
        XCTAssertEqual(
            AppModel.bundledManagedCopy["background_model_info"],
            "A smaller, faster model the tool uses for quick work in the background, like naming the "
                + "session and searching the code. Only models at least as private as your main model are listed.")
    }

    func testTheBackgroundPickerTitleCarriesAnInfoButton() throws {
        let source = try MonitorNavigationTests.text("Views/ManagedFolderOptions.swift")
        let start = try XCTUnwrap(source.range(of: "private var backgroundPicker: some View {"))
        let end = try XCTUnwrap(source.range(of: "\n    }\n", range: start.upperBound..<source.endIndex))
        let picker = source[start.upperBound..<end.lowerBound]
        XCTAssertTrue(picker.contains("GlassInfoButton("), "the background model title has no info button")
        XCTAssertTrue(picker.contains("managedText(\"background_model_info\")"),
                      "the info button must show the core's words")
    }
}
