import Foundation
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Missions are hidden for now (owner, 2026-10-16): one daemon setting,
/// `missions_enabled`, off by default and flipped only through
/// `set_settings`. These tests hold both halves: off hides every entry
/// point, on restores today's.
@MainActor
final class MissionsHiddenTests: XCTestCase {
    private static func source(_ rel: String) throws -> String {
        try MonitorNavigationTests.text(rel)
    }

    private static func settings(_ extra: String) throws -> DaemonSettingsView {
        let json = """
        {"quiescence_secs":45,"digest_interval_secs":3600,"local_notifications":true,
        "queue_ttl_days":14,"max_queue_entries":500,"max_uploads_per_day":100,
        "near_ai_configured":false,"claude_root_configured":true,"codex_root_configured":true\(extra)}
        """
        return try JSONDecoder().decode(DaemonSettingsView.self, from: Data(json.utf8))
    }

    // MARK: The setting

    func test_onlyAnExactTrueShowsMissions() throws {
        XCTAssertFalse(try Self.settings("").missionsShown, "an older daemon's silence is off")
        XCTAssertNil(try Self.settings("").missionsEnabled)
        XCTAssertFalse(try Self.settings(#","missions_enabled":false"#).missionsShown)
        XCTAssertTrue(try Self.settings(#","missions_enabled":true"#).missionsShown)
    }

    func test_theModelShowsNothingBeforeItsSettingsArrive() {
        XCTAssertFalse(AppModel().missionsShown)
    }

    // MARK: Pages

    func test_aMissionPageIsOnlyShownWhileMissionsAre() {
        for page in [HomePage.missions, .missionDrafts] {
            XCTAssertTrue(page.isMission)
            XCTAssertEqual(page.shown(missions: false), .overview)
            XCTAssertEqual(page.shown(missions: true), page)
        }
        // Insights is its own tab now (2026-10-15), not a Home page.
        for page in [HomePage.overview, .history] {
            XCTAssertFalse(page.isMission)
            XCTAssertEqual(page.shown(missions: false), page)
            XCTAssertEqual(page.shown(missions: true), page)
        }
    }

    func test_aDestinationOrRestoredPageCannotOpenAMissionPageWhileHidden() {
        for destination in [MonitorDestination.home(.missions), .home(.missionDrafts)] {
            var tab = MonitorWindowView.Tab.traces
            var page = HomeTabView.Page.overview
            var selection: MonitorSelection?
            var inspector = false
            MonitorWindowView.land(destination, tab: &tab, homePage: &page,
                                   selection: &selection, showsInspector: &inspector)
            XCTAssertEqual(tab, .home)
            XCTAssertEqual(page, .overview)
            MonitorWindowView.land(destination, tab: &tab, homePage: &page,
                                   selection: &selection, showsInspector: &inspector, missionsEnabled: true)
            if case .home(let wanted) = destination { XCTAssertEqual(page, wanted) }
        }
    }

    // MARK: Traces tag

    func test_theMissionTagIsBlankedWhileHiddenAndTheEstimateStays() {
        let tags = NudgeEntryTags(
            missionFit: "Fits a mission", estimateBand: "10-20", estimateTier: "Likely", estimateExplainer: "why")
        let hidden = tags.showing(missions: false)
        XCTAssertNil(hidden.missionFit)
        XCTAssertEqual(hidden.estimateBand, "10-20")
        XCTAssertEqual(hidden.estimateTier, "Likely")
        XCTAssertEqual(hidden.estimateExplainer, "why")
        XCTAssertEqual(tags.showing(missions: true), tags)
        XCTAssertTrue(NudgeEntryTags(missionFit: "Fits a mission").showing(missions: false).isEmpty)
    }

    // MARK: Entry points, in source

    func test_homeDrawsTheMissionCardsDraftsReadAndPagesOnlyWhileShown() throws {
        let home = try Self.source("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("                if missionsEnabled {\n                    GlassHeroCard(\n"
            + "                        gradient: GlassTokens.Gradient.heroMissions,"))
        XCTAssertTrue(home.contains("if missionsEnabled, store.missions != nil, let catalogue"))
        XCTAssertTrue(home.contains("if missionsEnabled { missionDrafts.preview() }"))
        XCTAssertTrue(home.contains("switch page.shown(missions: missionsEnabled) {"))
        XCTAssertFalse(home.contains(".task { missionDrafts.preview() }"), "the drafts read must be gated")
        let window = try Self.source("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("missionsEnabled: model.missionsShown)"))
        XCTAssertTrue(window.contains("homePage: homePage.shown(missions: model.missionsShown),"))
        XCTAssertTrue(window.contains(
            ".onChange(of: model.missionsShown, initial: true) { _, shown in\n"
            + "            homePage = homePage.shown(missions: shown)"))
    }

    func test_theLaunchReadsNoMissionDraftsCopy() throws {
        let app = try Self.source("TraceCommonsAppMain.swift")
        XCTAssertFalse(app.contains("missionDrafts.loadCopy()"))
    }

    func test_theEdgeRailAndTracesGateTheirMissionSurfaces() throws {
        // The rail's Missions card was in its Home peek, which is gone
        // (owner, 2026-10-22): the rail draws no Missions surface at all.
        let rail = try Self.source("Views/EdgeRail/EdgeRailView.swift")
        XCTAssertFalse(rail.contains("HomeOverviewIcons.missions"), "the rail draws Missions again")
        XCTAssertFalse(rail.contains("MonitorWords.missions"))
        let traces = try Self.source("Views/Monitor/TracesViews.swift")
        let window = try Self.source("Views/MonitorWindowView.swift")
        // The tree takes the switch as a value (no app model in the tree,
        // whose every publish redrew each row); its rows blank the tag.
        XCTAssertTrue(traces.contains("tags: store.rowTags[entry.entryId].map { $0.showing(missions: missionsShown) }"))
        XCTAssertTrue(window.contains("missionsShown: model.missionsShown,"))
    }
}
