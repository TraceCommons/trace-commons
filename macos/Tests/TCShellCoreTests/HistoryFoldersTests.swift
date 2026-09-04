import XCTest

@testable import TCShellCore

/// History groups by folder the way the queue does. The interesting part is
/// what happens to records that predate the change: their project id is
/// empty, they cannot be resolved to a folder, and they must not all be
/// swept into one bogus group.
final class HistoryFoldersTests: XCTestCase {
    private struct Record: Equatable {
        let id: String
        let projectID: String
        let label: String
    }

    private func folders(_ records: [Record]) -> [QueueGroup<Record>] {
        HistoryFolders.folders(
            records,
            projectID: \.projectID,
            projectLabel: \.label
        )
    }

    func testRecordsGroupByProjectID() {
        let groups = folders([
            Record(id: "1", projectID: "proj_a", label: "api"),
            Record(id: "2", projectID: "proj_b", label: "web"),
            Record(id: "3", projectID: "proj_a", label: "api"),
        ])
        XCTAssertEqual(groups.map(\.id), ["proj_a", "proj_b"])
        XCTAssertEqual(groups[0].count, 2)
    }

    func testTwoProjectsSharingALabelStaySeparate() {
        let groups = folders([
            Record(id: "1", projectID: "proj_a", label: "api"),
            Record(id: "2", projectID: "proj_b", label: "api"),
        ])
        XCTAssertEqual(groups.count, 2, "a label is not an identity")
    }

    func testRecordsWithNoProjectIDGroupByLabelInstead() {
        // Pre-upgrade records carry no id. Grouping them all under "" would
        // put two different repositories in one row.
        let groups = folders([
            Record(id: "1", projectID: "", label: "api"),
            Record(id: "2", projectID: "", label: "web"),
            Record(id: "3", projectID: "", label: "api"),
        ])
        XCTAssertEqual(groups.count, 2)
        XCTAssertEqual(groups.first(where: { $0.label == "api" })?.count, 2)
    }

    func testAnIdentifiedAndAnUnidentifiedRecordDoNotMerge() {
        // Same label, but one is resolvable and one is not. Claiming they
        // are the same folder is a guess, and the honest answer is two rows.
        let groups = folders([
            Record(id: "1", projectID: "proj_a", label: "api"),
            Record(id: "2", projectID: "", label: "api"),
        ])
        XCTAssertEqual(groups.count, 2)
    }
}
