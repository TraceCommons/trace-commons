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

    private let production = HomebrewDetector.productionBundleIdentifier

    /// A running bundle that sits exactly where the cask puts the app.
    private func caskCopy(id: String? = nil) -> HomebrewDetector.RunningBundle {
        HomebrewDetector.RunningBundle(
            identifier: id ?? production,
            path: root.appendingPathComponent("Applications/Near.ai.app").path,
            appDirectories: [root.appendingPathComponent("Applications").path]
        )
    }

    private func prefixPaths(_ names: [String]) -> [String] {
        names.map { root.appendingPathComponent($0).path }
    }

    func testNoCaskroomMeansWeInstalledItAndMayUpdateOurselves() {
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]),
            running: caskCopy()
        )
        XCTAssertFalse(state.isManaged)
        XCTAssertNil(state.caskroomPath)
        XCTAssertEqual(state.caskName, "trace-commons")
    }

    func testAppleSiliconPrefixIsDetected() throws {
        let prefix = try makeCaskroom(prefix: "opt/homebrew", cask: "trace-commons")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]),
            running: caskCopy()
        )
        XCTAssertTrue(state.isManaged)
        XCTAssertEqual(state.caskroomPath, prefix + "/Caskroom/trace-commons")
    }

    func testIntelPrefixIsDetected() throws {
        let prefix = try makeCaskroom(prefix: "usr/local", cask: "trace-commons")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]),
            running: caskCopy()
        )
        XCTAssertTrue(state.isManaged)
        XCTAssertEqual(state.caskroomPath, prefix + "/Caskroom/trace-commons")
    }

    func testAnUnrelatedCaskDoesNotCountAsOurs() throws {
        try makeCaskroom(prefix: "opt/homebrew", cask: "some-other-app")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]),
            running: caskCopy()
        )
        XCTAssertFalse(state.isManaged)
    }

    func testAFileWhereTheCaskroomEntryShouldBeIsNotAnInstall() throws {
        let dir = root.appendingPathComponent("opt/homebrew/Caskroom")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        try Data().write(to: dir.appendingPathComponent("trace-commons"))
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]),
            running: caskCopy()
        )
        XCTAssertFalse(state.isManaged, "a plain file is not a Caskroom entry")
    }

    func testAStagingBundleIsNeverHomebrewManagedEvenWithTheProductionCaskroom() throws {
        try makeCaskroom(prefix: "opt/homebrew", cask: "trace-commons")
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]),
            running: caskCopy(id: "ai.tracecommons.shell.staging")
        )
        XCTAssertFalse(state.isManaged, "there is no staging cask")
        XCTAssertNil(state.caskroomPath)
    }

    func testAProductionCopyOutsideTheCaskAppDirectoryIsNotManaged() throws {
        try makeCaskroom(prefix: "opt/homebrew", cask: "trace-commons")
        let elsewhere = HomebrewDetector.RunningBundle(
            identifier: production,
            path: root.appendingPathComponent("Users/me/Applications/Near.ai.app").path,
            appDirectories: [root.appendingPathComponent("Applications").path]
        )
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]), running: elsewhere
        )
        XCTAssertFalse(state.isManaged, "a DMG copy is not the cask's app")
    }

    func testABundleWithNoIdentifierIsNotManaged() throws {
        try makeCaskroom(prefix: "opt/homebrew", cask: "trace-commons")
        let bare = HomebrewDetector.RunningBundle(
            identifier: nil,
            path: root.appendingPathComponent("Applications/Near.ai.app").path,
            appDirectories: [root.appendingPathComponent("Applications").path]
        )
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]), running: bare
        )
        XCTAssertFalse(state.isManaged)
    }

    func testASymlinkedAppDirectoryStillMatchesTheCaskCopy() throws {
        try makeCaskroom(prefix: "opt/homebrew", cask: "trace-commons")
        let real = root.appendingPathComponent("Real")
        try FileManager.default.createDirectory(at: real, withIntermediateDirectories: true)
        let link = root.appendingPathComponent("Linked")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: real)
        let running = HomebrewDetector.RunningBundle(
            identifier: production,
            path: real.appendingPathComponent("Near.ai.app").path,
            appDirectories: [link.path]
        )
        let state = HomebrewDetector.detect(
            prefixes: prefixPaths(["opt/homebrew", "usr/local"]), running: running
        )
        XCTAssertTrue(state.isManaged)
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
