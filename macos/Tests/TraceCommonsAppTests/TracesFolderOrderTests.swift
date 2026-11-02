import XCTest
import TCBridge
import TCShellCore
@testable import TraceCommonsApp

/// The person's folder order on the Traces tab (owner, 2026-11-02): folders
/// dragged into place, beside Suggested first and Oldest first.
final class TracesFolderOrderTests: XCTestCase {
    private func folders(_ ids: String...) -> [TracesTree.FolderNode] {
        ids.map { TracesTree.FolderNode(id: $0, label: $0, mode: .ask, sessions: []) }
    }

    func testFoldersFollowTheArrangementAndNewOnesComeAfter() {
        let drawn = TracesTree.arranged(folders("a", "b", "c", "new"), by: ["c", "a", "b"])
        XCTAssertEqual(drawn.map(\.id), ["c", "a", "b", "new"])
    }

    func testAnArrangementNamingGoneFoldersStillDrawsTheRest() {
        let drawn = TracesTree.arranged(folders("a", "b"), by: ["gone", "b", "a"])
        XCTAssertEqual(drawn.map(\.id), ["b", "a"])
    }

    func testDroppedOnAFolderBelowItGoesAfterIt() {
        XCTAssertEqual(
            TracesTree.moving("a", onto: "c", shown: ["a", "b", "c", "d"], order: []),
            ["b", "c", "a", "d"])
    }

    func testDroppedOnAFolderAboveItGoesBeforeIt() {
        XCTAssertEqual(
            TracesTree.moving("d", onto: "b", shown: ["a", "b", "c", "d"], order: []),
            ["a", "d", "b", "c"])
    }

    func testDroppedOnItselfOrSomethingUnknownChangesNothing() {
        let shown = ["a", "b", "c"]
        XCTAssertEqual(TracesTree.moving("b", onto: "b", shown: shown, order: []), shown)
        XCTAssertEqual(TracesTree.moving("x", onto: "b", shown: shown, order: []), shown)
    }

    func testFoldersNotDrawnKeepTheirPlaceInTheArrangement() {
        XCTAssertEqual(
            TracesTree.moving("b", onto: "a", shown: ["a", "b"], order: ["a", "b", "hidden"]),
            ["b", "a", "hidden"])
    }

    @MainActor
    func testTheArrangementIsReadBackOnThisMac() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "TracesFolderOrderTests"))
        defer { defaults.removePersistentDomain(forName: "TracesFolderOrderTests") }
        defaults.set(["b", "a"], forKey: TracesStore.folderOrderKey)
        defaults.set(true, forKey: TracesStore.arrangedKey)
        let store = TracesStore(client: nil, orderDefaults: defaults)
        XCTAssertEqual(store.folderOrder, ["b", "a"])
        XCTAssertTrue(store.arranged)
        XCTAssertEqual(TracesListControls.orderChoice(store), .arranged)

        let fresh = TracesStore(client: nil, orderDefaults: nil)
        XCTAssertFalse(fresh.arranged)
        XCTAssertEqual(TracesListControls.orderChoice(fresh), .pending(.suggested))
    }

    func testTheCoreHasTheWordsForYourOrderAndTheMoves() throws {
        let copy = try XCTUnwrap(NudgeCopy.decode(fromJSON: TCCoreCopy.nudgeCopyJSON()))
        XCTAssertNotNil(copy[.listOrderArranged])
        XCTAssertNotNil(copy[.listOrderMoveUp])
        XCTAssertNotNil(copy[.listOrderMoveDown])
    }
}
