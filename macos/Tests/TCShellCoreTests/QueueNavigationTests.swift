import XCTest

@testable import TCShellCore

/// The queue is now two levels, and the second one can be pulled out from
/// under the person standing on it: approving a folder's last session
/// removes the folder. Every one of these tests is that situation.
final class QueueNavigationTests: XCTestCase {
    private struct Entry: Equatable { let id: String }

    private func groups(_ ids: [String]) -> [QueueGroup<Entry>] {
        ids.map { QueueGroup(id: $0, label: $0, bytes: 1, entries: [Entry(id: $0)]) }
    }

    func testRootStaysRoot() {
        XCTAssertEqual(QueueNavigation.resolve(.root, in: groups(["a"])), .root)
        XCTAssertEqual(QueueNavigation.resolve(.root, in: groups([])), .root)
    }

    func testAProjectThatStillExistsIsKept() {
        XCTAssertEqual(
            QueueNavigation.resolve(.project("a"), in: groups(["a", "b"])),
            .project("a")
        )
    }

    func testAProjectThatEmptiedFallsBackToRoot() {
        // Submit all inside a folder: the folder goes, and standing in it
        // would show an empty screen with a back button and no explanation.
        XCTAssertEqual(QueueNavigation.resolve(.project("a"), in: groups(["b"])), .root)
    }

    func testTheLastProjectEmptyingFallsBackToRoot() {
        XCTAssertEqual(QueueNavigation.resolve(.project("a"), in: groups([])), .root)
    }

    func testResolutionIsByIDNotLabel() {
        // Two projects can share a label; only the id identifies one.
        let two = [
            QueueGroup(id: "proj_1", label: "api", bytes: 1, entries: [Entry(id: "x")]),
            QueueGroup(id: "proj_2", label: "api", bytes: 1, entries: [Entry(id: "y")]),
        ]
        XCTAssertEqual(QueueNavigation.resolve(.project("proj_2"), in: two), .project("proj_2"))
        XCTAssertEqual(QueueNavigation.resolve(.project("proj_3"), in: two), .root)
    }
}
