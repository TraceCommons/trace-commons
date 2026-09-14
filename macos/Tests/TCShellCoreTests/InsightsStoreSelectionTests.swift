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

    /// The selected path is shown in the Insights header, so it has to be the
    /// path the store opens. Probing the link itself would display one
    /// directory while reading and writing another.
    func testResolvesASymlinkedStoreToThePathTheStoreActuallyUses() throws {
        let manager = FileManager.default
        let root = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString).resolvingSymlinksInPath()
        // The link and its target sit at different depths, so a lexical `..`
        // and a resolved `..` cannot agree by accident.
        let outer = root.appendingPathComponent("outer")
        let target = outer.appendingPathComponent("target")
        let link = root.appendingPathComponent("link")
        try manager.createDirectory(at: target, withIntermediateDirectories: true)
        defer { try? manager.removeItem(at: root) }
        try manager.createSymbolicLink(at: link, withDestinationURL: target)

        let direct = InsightsStoreSelection.parse(arguments: ["app", "--insights-store", link.path])
        XCTAssertEqual(direct, .custom(target.path))
        XCTAssertNotEqual(direct.storeDirectory, link.path)

        let through = InsightsStoreSelection.parse(
            arguments: ["app", "--insights-store", link.path + "/.."])
        XCTAssertEqual(through, .custom(outer.path))
        XCTAssertNotEqual(through.storeDirectory, root.path)
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
