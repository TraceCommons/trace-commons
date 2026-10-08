import TCBridge
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

    /// The headings are the core's: the Insights copy's `title`, and the
    /// screens' own word for Missions (Ron's Missions card opens the
    /// drafts). A card whose word has not arrived is not drawn, never drawn
    /// with a Swift-authored word.
    func test_theHomeCardsTakeTheirHeadingsFromTheCore() throws {
        let home = try MonitorNavigationTests.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("private static let insightsCopy = TCInsights.copy()"))
        // One reading: the card and the shell's breadcrumb both take it from
        // `HomeTabView.hostedHeading` (Ron's breadcrumb, #1241).
        XCTAssertEqual(home.components(separatedBy: #"Self.insightsCopy?["title"]"#).count - 1, 1)
        XCTAssertTrue(home.contains("case .missionDrafts: HomeFormat.cardHeading(MonitorWords.missions)"))
        XCTAssertTrue(home.contains("hostedCard(catalogue, action: openMissions)"))
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

    /// Error lines in the hosted screens draw in the status text token and
    /// success notices in the on token, never a system colour.
    func test_hostedErrorsUseTheStatusTextTokens() throws {
        let errorSites: [(String, Int)] = [
            ("Views/InsightsView.swift", 2), ("Views/InsightCardsView.swift", 1),
            ("Views/InsightsEpisodesView.swift", 1), ("Views/ComparisonTasksView.swift", 1),
            ("Views/ComparisonSpecificationsView.swift", 1), ("Views/MissionDraftsView.swift", 1),
        ]
        for (path, count) in errorSites {
            let source = try MonitorNavigationTests.text(path)
            // Ron's rebuilds (#1241) say the status text token through
            // `GlassStatus.textColor`.
            XCTAssertEqual(source.components(separatedBy: ".foregroundStyle(GlassStatus.outside.textColor)").count - 1,
                           count, path)
            XCTAssertFalse(source.contains(".foregroundStyle(GlassColor.textPrimary)"), path)
        }
        for path in ["Views/InsightsEpisodesView.swift", "Views/ComparisonTasksView.swift",
                     "Views/ComparisonSpecificationsView.swift"] {
            let source = try MonitorNavigationTests.text(path)
            XCTAssertEqual(source.components(separatedBy: ".foregroundStyle(GlassStatus.on.textColor)").count - 1, 1, path)
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

    /// M-3: Insights and Mission drafts are reachable only through their
    /// cards, which draw only on the core's words. A core that stops
    /// sending either goes red here rather than hiding a screen.
    func test_theInsightsAndMissionDraftsHeadingsAreTheCores() throws {
        XCTAssertNotNil(HomeFormat.cardHeading(TCInsights.copy()?["title"]), "Insights would be unreachable")
        guard case .copy(let copy) = try TCMissionDrafts.call(.init(operation: .init("copy"))) else {
            return XCTFail("the mission drafts copy call answered something else")
        }
        XCTAssertNotNil(HomeFormat.cardHeading(copy["title"]), "the drafts screen would have no title")
        XCTAssertNotNil(HomeFormat.cardHeading(MonitorWords.missions), "Mission drafts would be unreachable")
    }

    /// M-1: the app's own reads refresh when someone looks, as the legacy
    /// window and menu did: on the Monitor's always-present container and
    /// on the menu panel's root.
    func test_lookingRefreshesTheAppsReads() throws {
        let window = try MonitorNavigationTests.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("        .glassWindow()\n        .onAppear { model.refreshAll() }\n"))
        let panel = try MonitorNavigationTests.text("Views/Monitor/MenuBarGlassPanel.swift")
        XCTAssertTrue(panel.contains("        .task { await store.load() }\n        .onAppear { model.refreshAll() }\n"))
    }
}

