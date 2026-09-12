import XCTest
@testable import TCShellCore

final class InsightsStoreSelectionTests: XCTestCase {
    func testDefaultHasNoExplicitStore() {
        XCTAssertEqual(InsightsStoreSelection.parse(arguments: ["app"]), .standard)
        XCTAssertNil(InsightsStoreSelection.standard.storeDirectory)
    }

    func testAcceptsAndStandardizesExistingAbsoluteDirectory() {
        let result = InsightsStoreSelection.parse(arguments: ["app", "--insights-store", "/pilot/../pilot"]) {
            $0 == "/pilot" ? .directory : .absent
        }
        XCTAssertEqual(result, .custom("/pilot"))
    }

    func testRejectsMalformedAndUnavailableSelections() {
        let directory: @Sendable (String) -> StateDirectory.Probe.Verdict = { _ in .directory }
        XCTAssertEqual(InsightsStoreSelection.parse(
            arguments: ["app", "--insights-store", "/a", "--insights-store", "/b"], probe: directory),
            .refused(.duplicateOption))
        XCTAssertEqual(InsightsStoreSelection.parse(arguments: ["app", "--insights-store"], probe: directory),
                       .refused(.missingPath))
        XCTAssertEqual(InsightsStoreSelection.parse(arguments: ["app", "--insights-store=foo"], probe: directory),
                       .refused(.missingPath))
        XCTAssertEqual(InsightsStoreSelection.parse(
            arguments: ["app", "--insights-store", "relative"], probe: directory), .refused(.relativePath))
        XCTAssertEqual(InsightsStoreSelection.parse(
            arguments: ["app", "--insights-store", "/missing"], probe: { _ in .absent }), .refused(.pathMissing))
        XCTAssertEqual(InsightsStoreSelection.parse(
            arguments: ["app", "--insights-store", "/file"], probe: { _ in .file }), .refused(.notADirectory))
    }
}
