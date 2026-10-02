import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// R10 of #1173: Missions follows #1174's rules, against C1's sample
/// catalogue.
@MainActor
final class MissionsTests: XCTestCase {
    private func catalogue(_ json: String) throws -> DaemonData.MissionCatalogue {
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .iso8601
        return try decoder.decode(DaemonData.MissionCatalogue.self, from: Data(json.utf8))
    }

    /// The sample catalogue loads through the store, and the provisional
    /// method is not reported as a failure.
    func test_theCatalogueLoads() async throws {
        let store = HomeStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        let missions = try XCTUnwrap(store.missions)
        XCTAssertFalse(missions.missions.isEmpty)
        XCTAssertNil(store.failures["mission_catalogue"])
        XCTAssertEqual(MissionFormat.count(missions), String(missions.missions.count))
        XCTAssertEqual(MissionFormat.count(nil), "—")
    }

    /// M3: a credit range is pending credit, shown with the commons'
    /// condition; a mission with no range shows a dash.
    func test_creditIsPendingAndCarriesItsCondition() throws {
        let catalogue = try catalogue(#"""
        {"fetched_at":null,"posture":{"settlement":"disabled","graded":false,"explanation":"Waits on settlement"},
         "missions":[{"id":"a","title":"A","summary":null,"credit_range":{"min":5,"max":20,"unit":"points"}},
                     {"id":"b","title":"B","summary":null,"credit_range":{"min":3,"max":3,"unit":"points"}},
                     {"id":"c","title":"C","summary":null,"credit_range":null}]}
        """#)
        XCTAssertEqual(MissionFormat.condition(catalogue), "Waits on settlement")
        XCTAssertEqual(MissionFormat.credit(catalogue.missions[0], in: catalogue), "5–20 points")
        XCTAssertEqual(MissionFormat.credit(catalogue.missions[1], in: catalogue), "3 points")
        XCTAssertEqual(MissionFormat.credit(catalogue.missions[2], in: catalogue), "—")
    }

    /// M3: with no statement of what the credit waits on, no range is shown
    /// at all; a bare range would read as owed.
    func test_noConditionMeansNoFigure() throws {
        let catalogue = try catalogue(#"""
        {"fetched_at":null,"posture":null,
         "missions":[{"id":"a","title":"A","summary":null,"credit_range":{"min":5,"max":20,"unit":"points"}}]}
        """#)
        XCTAssertNil(MissionFormat.condition(catalogue))
        XCTAssertEqual(MissionFormat.credit(catalogue.missions[0], in: catalogue), "—")
    }

    /// M2: the Missions page has no action of its own. Nothing on it can
    /// arm a folder, approve a session or widen a scope. Its only control
    /// is the breadcrumb back to Home.
    func test_theMissionsPageHasNoActions() throws {
        let source = try String(contentsOf: Self.source("Views/Monitor/MissionsViews.swift"), encoding: .utf8)
        for control in ["Button(", "Toggle(", ".onTapGesture", "Picker(", "Menu {", "GlassPicker(", "setProjectMode", "approve("] {
            XCTAssertFalse(source.contains(control), "MissionsViews.swift contains \(control)")
        }
    }

    private static func source(_ path: String) -> URL {
        URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent() // TraceCommonsAppTests
            .deletingLastPathComponent() // Tests
            .deletingLastPathComponent() // macos
            .appendingPathComponent("Sources/TraceCommonsApp")
            .appendingPathComponent(path)
    }
}
