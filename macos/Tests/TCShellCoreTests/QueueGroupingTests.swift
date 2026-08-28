import XCTest

@testable import TCShellCore

/// `QueueGrouping` exists to make the queue's per-project grouping a thing
/// that happens once per change, not once per SwiftUI body evaluation. The
/// linearity test below is the point of the type: the view it replaces
/// grouped, and then re-filtered the whole waiting list once per group, so
/// the cost of drawing a 500-entry queue scaled with entries times projects.
final class QueueGroupingTests: XCTestCase {
    private struct Entry: Equatable {
        let id: String
        let project: String
        let label: String
        let bytes: Int
    }

    private func group(_ entries: [Entry]) -> [QueueGroup<Entry>] {
        QueueGrouping.groups(
            entries,
            projectID: \.project,
            projectLabel: \.label,
            sizeBytes: \.bytes
        )
    }

    func testAnEmptyListGroupsToNothing() {
        XCTAssertTrue(group([]).isEmpty)
    }

    func testGroupsKeepFirstSeenOrder() {
        let entries = [
            Entry(id: "a", project: "p2", label: "Two", bytes: 1),
            Entry(id: "b", project: "p1", label: "One", bytes: 1),
            Entry(id: "c", project: "p2", label: "Two", bytes: 1),
        ]
        XCTAssertEqual(group(entries).map(\.id), ["p2", "p1"])
    }

    func testEntriesStayInTheirOwnGroupInOrder() {
        let entries = [
            Entry(id: "a", project: "p1", label: "One", bytes: 1),
            Entry(id: "b", project: "p2", label: "Two", bytes: 1),
            Entry(id: "c", project: "p1", label: "One", bytes: 1),
        ]
        let groups = group(entries)
        XCTAssertEqual(groups.first(where: { $0.id == "p1" })?.entries.map(\.id), ["a", "c"])
        XCTAssertEqual(groups.first(where: { $0.id == "p2" })?.entries.map(\.id), ["b"])
    }

    func testCountAndBytesAreTheGroupsOwnTotals() {
        let entries = [
            Entry(id: "a", project: "p1", label: "One", bytes: 30),
            Entry(id: "b", project: "p1", label: "One", bytes: 12),
            Entry(id: "c", project: "p2", label: "Two", bytes: 7),
        ]
        let groups = group(entries)
        XCTAssertEqual(groups[0].count, 2)
        XCTAssertEqual(groups[0].bytes, 42)
        XCTAssertEqual(groups[1].count, 1)
        XCTAssertEqual(groups[1].bytes, 7)
    }

    /// A label is a display name and is not guaranteed unique. Grouping by
    /// it would merge two different projects into one bucket with one
    /// `Submit all` that could approve the wrong project's entries.
    func testTwoProjectsSharingALabelStaySeparate() {
        let entries = [
            Entry(id: "a", project: "p1", label: "same", bytes: 1),
            Entry(id: "b", project: "p2", label: "same", bytes: 1),
        ]
        let groups = group(entries)
        XCTAssertEqual(groups.count, 2)
        XCTAssertEqual(groups.map(\.id), ["p1", "p2"])
        XCTAssertEqual(groups.map(\.label), ["same", "same"])
    }

    /// The first entry of a project names the group; a later entry carrying
    /// a stale label does not rename it out from under the buttons.
    func testTheFirstEntryNamesTheGroup() {
        let entries = [
            Entry(id: "a", project: "p1", label: "first", bytes: 1),
            Entry(id: "b", project: "p1", label: "second", bytes: 1),
        ]
        XCTAssertEqual(group(entries).map(\.label), ["first"])
    }

    /// One pass. The view this replaces cost entries times projects; this
    /// asserts the accessors are consulted a number of times that does not
    /// grow with the project count.
    func testGroupingIsOnePassOverTheEntries() {
        var projectReads = 0
        let entries = (0..<120).map {
            Entry(id: "e\($0)", project: "p\($0 % 12)", label: "L\($0 % 12)", bytes: 1)
        }
        let groups = QueueGrouping.groups(
            entries,
            projectID: { projectReads += 1; return $0.project },
            projectLabel: \.label,
            sizeBytes: \.bytes
        )
        XCTAssertEqual(groups.count, 12)
        XCTAssertEqual(projectReads, entries.count)
    }
}
