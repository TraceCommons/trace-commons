import XCTest
@testable import TCDesign

final class GlassPickerRunTests: XCTestCase {
    private func option(_ value: Int, _ section: String?) -> GlassPickerOption<Int> {
        GlassPickerOption("m\(value)", value: value, section: section)
    }

    /// Neighbours with one heading share a run; the same heading apart stays
    /// two runs; ids count the runs in order.
    func testNeighboursMergeAndNonNeighboursStayApart() {
        let runs = GlassPickerRun.runs([
            option(1, nil), option(2, "A"), option(3, "A"), option(4, nil), option(5, "A"),
        ])
        XCTAssertEqual(runs.map(\.id), [0, 1, 2, 3])
        XCTAssertEqual(runs.map(\.section), [nil, "A", nil, "A"])
        XCTAssertEqual(runs.map { $0.options.map(\.value) }, [[1], [2, 3], [4], [5]])
    }

    /// The menu draws each run as a section, so a rule falls between any two
    /// runs; that needs no two neighbouring runs to share a heading, headless
    /// included.
    func testNoTwoNeighbouringRunsShareAHeading() {
        let runs = GlassPickerRun.runs([
            option(1, nil), option(2, nil), option(3, "A"), option(4, nil), option(5, nil), option(6, "B"),
        ])
        XCTAssertEqual(runs.count, 4)
        for (a, b) in zip(runs, runs.dropFirst()) {
            XCTAssertNotEqual(a.section, b.section)
        }
    }

    func testEmptyAndSingleRun() {
        XCTAssertTrue(GlassPickerRun<Int>.runs([]).isEmpty)
        XCTAssertEqual(GlassPickerRun.runs([option(1, nil), option(2, nil)]).count, 1)
    }
}
