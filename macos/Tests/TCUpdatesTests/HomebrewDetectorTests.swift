import Foundation
import XCTest

@testable import TCUpdates

final class HomebrewDetectorTests: XCTestCase {
    private var root: URL!

    override func setUpWithError() throws {
        root = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent("tc-homebrew-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    }

    override func tearDownWithError() throws {
        try? FileManager.default.removeItem(at: root)
    }

    /// Creates `<root>/<prefix>/Caskroom/<cask>` and returns `<root>/<prefix>`.
    @discardableResult
    private func makeCaskroom(prefix: String, cask: String) throws -> String {
        let dir = root.appendingPathComponent(prefix).appendingPathComponent("Caskroom")
            .appendingPathComponent(cask)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return root.appendingPathComponent(prefix).path
    }

    private func prefixPaths(_ names: [String]) -> [String] {
        names.map { root.appendingPathComponent($0).path }
    }

    func testNoCaskroomMeansWeInstalledItAndMayUpdateOurselves() {
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertFalse(state.isManaged)
        XCTAssertNil(state.caskroomPath)
        XCTAssertEqual(state.caskName, "trace-commons")
    }

    func testAppleSiliconPrefixIsDetected() throws {
        let prefix = try makeCaskroom(prefix: "opt/homebrew", cask: "trace-commons")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertTrue(state.isManaged)
        XCTAssertEqual(state.caskroomPath, prefix + "/Caskroom/trace-commons")
    }

    func testIntelPrefixIsDetected() throws {
        let prefix = try makeCaskroom(prefix: "usr/local", cask: "trace-commons")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertTrue(state.isManaged)
        XCTAssertEqual(state.caskroomPath, prefix + "/Caskroom/trace-commons")
    }

    func testAnUnrelatedCaskDoesNotCountAsOurs() throws {
        try makeCaskroom(prefix: "opt/homebrew", cask: "some-other-app")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertFalse(state.isManaged)
    }

    func testAFileWhereTheCaskroomEntryShouldBeIsNotAnInstall() throws {
        let dir = root.appendingPathComponent("opt/homebrew/Caskroom")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data().write(to: dir.appendingPathComponent("trace-commons"))
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"])
        )
        XCTAssertFalse(state.isManaged, "a plain file is not a Caskroom entry")
    }

    func testTheUpgradeCommandIsExactlyWhatWeTellPeopleToRun() {
        let state = HomebrewInstallState(
            isManaged: true, caskName: "trace-commons",
            caskroomPath: "/opt/homebrew/Caskroom/trace-commons"
        )
        XCTAssertEqual(state.upgradeCommand, "brew upgrade --cask trace-commons")
    }

    func testTheShippingPrefixesAreBothHomebrewLocations() {
        XCTAssertEqual(HomebrewDetector.defaultPrefixes, ["/opt/homebrew", "/usr/local"])
    }
}
