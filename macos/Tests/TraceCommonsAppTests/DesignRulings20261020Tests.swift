import SwiftUI
import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The owner's 2026-10-20 review of the round-three build: the map's orbs,
/// the Not contributed card, Home's sections and its Private AI section.
@MainActor
final class DesignRulings20261020Tests: XCTestCase {
    private static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// A shared trace's orb is blue and a kept one's orange, everywhere
    /// the map draws a run's colour; a dismissed trace is not shared.
    func test_sharedOrbsAreBlueAndKeptOrbsOrange() throws {
        XCTAssertEqual(MapTransferDrawing.glowToken(.contribute), GlassTokens.Color.mapOrbShared)
        XCTAssertEqual(MapTransferDrawing.glowToken(.keep), GlassTokens.Color.mapOrbKept)
        XCTAssertEqual(MapTransferDrawing.glowToken(.dismiss), GlassTokens.Color.mapOrbKept)
        XCTAssertEqual(MapTransferDrawing.glowToken(nil), GlassTokens.Color.mapOrbKept)
        XCTAssertEqual(GlassTokens.Color.mapOrbShared.rgb, 0x0091FD)
        XCTAssertEqual(GlassTokens.Color.mapOrbKept.rgb, 0xD95926)
        let pile = try Self.text("Views/Monitor/MapToolPile.swift")
        XCTAssertFalse(pile.contains("statusOn.color : GlassTokens.Color.purpleSoft"), "a run is green again")
        XCTAssertTrue(pile.contains("charge > 0 ? GlassTokens.Color.mapOrbShared.color"))
    }

    /// The Not contributed card's border is the accent, solid.
    func test_theNotContributedBorderIsTheAccent() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("static var border: GlassRGBA { GlassTokens.Color.controlSelected }"))
        XCTAssertTrue(home.contains("static let borderWidth: CGFloat = 1.5"))
    }

    /// The card's line is the core's call to share; an older core's plain
    /// line still shows.
    func test_theNotContributedLineIsTheCallToShare() throws {
        func estimate(_ extra: String) throws -> DaemonData.SettlementEstimate {
            let json = """
                {"low_usd_micros":1500000,"high_usd_micros":4500000,"up_to":false,"known":3,"unknown":0,
                 "table_version":"s1","pricing_version":"p2","estimated_at":"2026-10-20T00:00:00Z",
                 "text":"About $1.50 to $4.50 of inference in traces on this Mac"\(extra)}
                """
            return try DaemonDataDecoding.decoder().decode(DaemonData.SettlementEstimate.self, from: Data(json.utf8))
        }
        let share = try estimate(
            #","share_text":"You could make about $1.50 to $4.50 of inference if you share these""#)
        XCTAssertEqual(HomeUncontributed([], settlement: share).detail(nil),
                       "You could make about $1.50 to $4.50 of inference if you share these")
        let older = try estimate("")
        XCTAssertNil(older.shareText)
        XCTAssertEqual(HomeUncontributed([], settlement: older).detail(nil),
                       "About $1.50 to $4.50 of inference in traces on this Mac")
    }

    /// Home's groups are titled, in the owner's order and words: Private
    /// AI, then Traces, then Account.
    func test_homesSectionsAreTitledInOrder() throws {
        let words = try XCTUnwrap(MonitorWords.table).homeHistory
        XCTAssertEqual(words.sectionPrivateAI, "Private AI")
        XCTAssertEqual(words.sectionTraces, "Traces")
        XCTAssertEqual(words.sectionAccount, "Account")
        XCTAssertEqual(words.privateAINotConnected, "Not connected")
        XCTAssertEqual(words.privateAIConnect, "Connect")
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let privateAI = try XCTUnwrap(home.range(of: "            privateAI\n            HomeSection(words?.sectionTraces) {"))
        let account = try XCTUnwrap(home.range(of: "HomeSection(words?.sectionAccount) {"))
        let waiting = try XCTUnwrap(home.range(of: "HomeStatTile(label: MonitorWords.waiting"))
        let credit = try XCTUnwrap(home.range(of: "HomeStatTile(label: HomeFormat.creditPendingWord"))
        XCTAssertLessThan(privateAI.lowerBound, waiting.lowerBound)
        XCTAssertLessThan(waiting.lowerBound, account.lowerBound)
        XCTAssertLessThan(account.lowerBound, credit.lowerBound)
        XCTAssertEqual(HomeSection<EmptyView>.titleType, GlassTabHeading<EmptyView>.titleType)
    }

    /// Not connected: one card with Connect, which opens the Private AI
    /// tab rather than turning anything on. Working: Calls and Verified.
    func test_thePrivateAISectionConnectsOrCounts() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("Button(words.privateAIConnect, action: openPrivateAI)"))
        XCTAssertTrue(home.contains("HomeStatTile(label: MonitorWords.calls, value: HomeFormat.count(totals.calls),"))
        XCTAssertTrue(home.contains(
            "HomeStatTile(label: InferenceWords.proof(.verified), value: HomeFormat.count(totals.verified),"))
        XCTAssertFalse(home.contains("answerPrivateInferenceOffer"), "Home turns Private AI on in one press")
        // No store, or one not read yet: dashes, never zero.
        let none = HomePrivateAI.totals(nil)
        XCTAssertNil(none.calls)
        XCTAssertNil(none.verified)
        XCTAssertNil(HomePrivateAI.totals(InferenceStore(client: nil)).calls)
        // Connected follows the core's tone, never the switch: no state
        // reported is not connected.
        let calls = AppModel().privateInferenceCalls
        XCTAssertFalse(HomePrivateAI.connected(nil, calls: calls))
    }

    /// The main window's divider: the window keeps its width; the main
    /// pane is passed the room the map gives up.
    func test_theWindowKeepsTheMainPanesWidth() throws {
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("@AppStorage(\"monitor.mainWidth\") private var mainWidth"))
        XCTAssertTrue(window.contains("mainWidth: $mainWidth,"))
    }

    /// An "i" follows its line's last word wherever the line wraps (owner,
    /// 2026-10-20): Insights' section titles, group headings and coverage
    /// line flow their words with the "i" on the last.
    func test_theInfoFollowsTheLastWord() throws {
        XCTAssertEqual(InsightsTextWithInfo.words("Oct 9 \u{b7} Claude Code  trace"),
                       ["Oct", "9", "\u{b7}", "Claude", "Code", "trace"])
        XCTAssertEqual(InsightsTextWithInfo.words(""), [])
        XCTAssertGreaterThan(InsightsTextWithInfo.wordGap(GlassTokens.TypeScale.title), 0)
        let glass = try Self.text("Views/InsightsGlass.swift")
        XCTAssertEqual(glass.components(separatedBy: "InsightsTextWithInfo(").count - 1, 3)
        XCTAssertTrue(glass.contains(
            "Text(words.last ?? \"\").accessibilityHidden(true)\n                    InsightsInfo(infoTitle, info)"))
        XCTAssertFalse(glass.contains("            InsightsInfo(title, info)\n            Spacer("), "the title's i is at the row's end again")
    }
}
