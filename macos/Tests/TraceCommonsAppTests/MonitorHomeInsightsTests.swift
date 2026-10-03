@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

private final class SilentTransport: DaemonTransport {
    func call(_ method: String, params paramsJSON: String) -> String { "{}" }
}

/// P4T8: Insights and Mission drafts reachable from Home. Sits beside
/// `MonitorNavigationTests` and reads its source helper.
@MainActor
final class MonitorHomeInsightsTests: XCTestCase {
    func test_insightsAndMissionDraftsAreReachableFromHome() throws {
        let nav = try MonitorNavigationTests.text("MonitorNavigation.swift")
        XCTAssertTrue(nav.contains("case insights\n"))
        XCTAssertTrue(nav.contains("case missionDrafts\n"))
        let home = try MonitorNavigationTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("InsightsView(storeSelection: insightsStoreSelection)"))
        XCTAssertTrue(home.contains("MissionDraftsView(model: missionDrafts)"))
        XCTAssertTrue(home.contains("openInsights: { page = .insights }"))
        XCTAssertTrue(home.contains("openMissionDrafts: { page = .missionDrafts }"))
        let window = try MonitorNavigationTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("insightsStoreSelection: insightsStoreSelection"))
        XCTAssertTrue(window.contains("missionDrafts: missionDrafts"))
        let main = try MonitorNavigationTests.text("TraceCommonsAppMain.swift")
        XCTAssertTrue(main.contains(
            "MonitorWindowView(navigation: navigation, insightsStoreSelection: insightsStoreSelection, missionDrafts: missionDrafts)"))
    }

    /// The headings are the core's: the Insights copy's `title` and the
    /// mission drafts copy's `title`. A card whose word has not arrived is
    /// not drawn, never drawn with a Swift-authored word.
    func test_theHomeCardsTakeTheirHeadingsFromTheCore() throws {
        let home = try MonitorNavigationTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains(#"TCInsights.copy()?["title"]"#))
        XCTAssertTrue(home.contains(#"missionDrafts.copy["title"]"#))
        XCTAssertNil(HomeFormat.cardHeading(nil))
        XCTAssertNil(HomeFormat.cardHeading(""))
        XCTAssertEqual(HomeFormat.cardHeading("Insights"), "Insights")
    }

    func test_insightsAreOnTheGlassRules() {
        let rules = Set(GlassSurfaceRulesTests.files)
        for path in ["Views/InsightsView.swift", "Views/InsightsSummaryView.swift", "Views/InsightsEpisodesView.swift",
                     "Views/InsightCardsView.swift", "Views/InsightEvidenceView.swift", "Views/ComparisonTasksView.swift",
                     "Views/ComparisonSpecificationsView.swift", "Views/MissionDraftsView.swift"] {
            XCTAssertTrue(rules.contains(path), "\(path) is not on the glass rules")
        }
    }

    /// D-9: the live client answers notAvailableYet for the mission
    /// catalogue; the page draws the absent dash, never zero, never a
    /// failure line.
    func test_missionsDrawAbsentNotZeroWhenTheCatalogueIsNotAvailableYet() async {
        let store = HomeStore(client: LiveDaemonClient(transport: SilentTransport()))
        await store.load()
        XCTAssertNil(store.missions)
        XCTAssertNil(store.failures["mission_catalogue"])
        XCTAssertEqual(MissionFormat.count(store.missions), "\u{2014}")
    }

    /// The Private AI inspector's summary is absent, not zero, when the live
    /// client does not answer it yet.
    func test_theInferenceStoreKeepsTheSummaryAbsentWhenItIsNotAvailableYet() async {
        let store = InferenceStore(client: LiveDaemonClient(transport: SilentTransport()))
        await store.load()
        XCTAssertNil(store.summary)
        XCTAssertNil(store.failures["inference_summary"])
    }
}
