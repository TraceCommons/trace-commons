import SwiftUI
import TCBridge
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

    /// The Not contributed card has no border. A shimmer crossed it from
    /// 2026-10-20; since 2026-10-22 a soft light turns round its edge
    /// instead (`DesignRulings20261022Tests`), over its fill and
    /// watermark.
    func test_aLightTurnsRoundTheNotContributedCard() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let card = try XCTUnwrap(home.range(of: "private struct HomeNotContributedCard: View {"))
        let end = try XCTUnwrap(home.range(of: "\n}\n", range: card.upperBound..<home.endIndex))
        let body = home[card.upperBound..<end.lowerBound]
        XCTAssertFalse(body.contains("strokeBorder"), "the border is back")
        XCTAssertFalse(body.contains("glassCardShimmer"), "the shimmer is back")
        let mark = try XCTUnwrap(body.range(of: ".glassCardWatermark("))
        let tier = try XCTUnwrap(body.range(of: ".glassTier(.card"))
        let beacon = try XCTUnwrap(body.range(of: ".glassCardBeacon()"))
        XCTAssertLessThan(mark.lowerBound, tier.lowerBound)
        XCTAssertLessThan(tier.lowerBound, beacon.lowerBound, "the light sits over the card's fill")
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

    /// Home's groups are titled, in the owner's words. The order is the
    /// 2026-10-28 ruling: Traces (with its hero) first, then Account, then
    /// Private AI, below and smaller than Traces.
    func test_homesSectionsAreTitledInOrder() throws {
        let words = try XCTUnwrap(MonitorWords.table).homeHistory
        XCTAssertEqual(words.sectionPrivateAI, "Private AI")
        XCTAssertEqual(words.sectionTraces, "Traces")
        XCTAssertEqual(words.sectionAccount, "Account")
        XCTAssertEqual(words.privateAISignIn, "Sign in to NEAR AI to get started.")
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let traces = try XCTUnwrap(home.range(of: "            HomeSection(words?.sectionTraces) {\n                tracesHero\n"))
        let waiting = try XCTUnwrap(home.range(of: "HomeStatTile(label: MonitorWords.waiting"))
        let account = try XCTUnwrap(home.range(of: "HomeSection(words?.sectionAccount) {"))
        let privateAI = try XCTUnwrap(home.range(of: "                HomeAccountTiles(store: store, openPrivateAI: openPrivateAI)\n            }\n            privateAI\n"))
        let credit = try XCTUnwrap(home.range(of: "HomeStatTile(label: HomeFormat.creditPendingWord"))
        XCTAssertLessThan(traces.lowerBound, waiting.lowerBound)
        XCTAssertLessThan(waiting.lowerBound, account.lowerBound)
        XCTAssertLessThan(account.lowerBound, privateAI.lowerBound)
        XCTAssertLessThan(account.lowerBound, credit.lowerBound)
        XCTAssertEqual(HomeSection<EmptyView>.titleType, GlassTabHeading<EmptyView>.titleType)
    }

    /// Signed out of NEAR AI: one card with no icon, "Sign in to NEAR AI to
    /// get started." and Sign in, which begins the NEAR AI sign-in (owner,
    /// 2026-10-20). Signed in: Calls and Verified.
    func test_thePrivateAISectionSignsInOrCounts() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("let snapshot = model.managedSnapshot {"))
        XCTAssertTrue(home.contains("if snapshot.nearAiSignedIn {"))
        XCTAssertTrue(home.contains("line: words.privateAISignIn, button: model.managedText(\"sign_in_near_ai\"),"))
        XCTAssertTrue(home.contains("NearAiSignIn.begin(model, openURL: openURL)"))
        // The core's managed words, which reach the model with its snapshot.
        let managed = try XCTUnwrap(JSONSerialization.jsonObject(
            with: Data(XCTUnwrap(TCDiscovery.managedCopyJSON()).utf8)) as? [String: String])
        XCTAssertEqual(managed["sign_in_near_ai"], "Sign in")
        XCTAssertEqual(managed["sign_in_near_ai_accessibility"], "Sign in to NEAR AI")
        // Centred on the card's height, with no icon.
        let card = try XCTUnwrap(home.range(of: "private struct HomePrivateAISignInCard: View {"))
        let body = home[card.upperBound...].prefix(1400)
        XCTAssertTrue(body.contains("HStack(alignment: .center, spacing: GlassTokens.Space.s3) {"))
        XCTAssertFalse(body.contains("glassCardWatermark"), "the sign-in card has an icon again")
        XCTAssertFalse(body.contains("Image(systemName:"), "the sign-in card has an icon again")
        XCTAssertFalse(home.contains("HomePrivateAIConnectCard"))
        // Signed in: one slim row, not tiles (owner, 2026-10-28: smaller
        // than Traces).
        XCTAssertTrue(home.contains("(MonitorWords.calls, HomeFormat.count(totals.calls)),"))
        XCTAssertTrue(home.contains("(InferenceWords.proof(.verified), HomeFormat.count(totals.verified)),"))
        XCTAssertFalse(home.contains("HomeStatTile(label: MonitorWords.calls"), "Private AI is tiles again")
        XCTAssertFalse(home.contains("answerPrivateInferenceOffer"), "Home turns Private AI on in one press")
        // No store, or one not read yet: dashes, never zero.
        let none = HomePrivateAI.totals(nil)
        XCTAssertNil(none.calls)
        XCTAssertNil(none.verified)
        XCTAssertNil(HomePrivateAI.totals(InferenceStore(client: nil)).calls)
    }

    /// The Not contributed card has no chevron (owner, 2026-10-20).
    func test_theNotContributedCardHasNoChevron() throws {
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        let card = try XCTUnwrap(home.range(of: "private struct HomeNotContributedCard: View {"))
        let end = try XCTUnwrap(home.range(of: "\n}\n", range: card.upperBound..<home.endIndex))
        XCTAssertFalse(home[card.upperBound..<end.lowerBound].contains("GlassChevron()"))
    }

    /// The Private AI tab's state follows its title (owner, 2026-10-20),
    /// not at the row's far end.
    func test_thePrivateAIStateFollowsItsTitle() throws {
        let tab = try Self.text("Views/Monitor/InferenceViews.swift")
        XCTAssertTrue(tab.contains(
            "GlassTabHeading(copy.inspectorStatus) {\n                GlassStatusLabel(status.word,"))
    }

    /// The refresh glyph's ring sits on the circle's centre: its box is
    /// nudged right and up by the arrowhead's pull (owner, 2026-10-20).
    func test_theRefreshGlyphIsOpticallyCentred() {
        XCTAssertEqual(GlassRefreshButton.opticalOffset.width, 0.5)
        XCTAssertEqual(GlassRefreshButton.opticalOffset.height, -0.6)
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

    /// Every refresh is the round refresh icon button at the trailing end
    /// of what it re-reads (owner, 2026-10-20): no "Refresh" in words, no
    /// bare glyph, and the icon card's beside its title no more.
    func test_everyRefreshIsTheRightAlignedIconButton() throws {
        XCTAssertEqual(GlassRefreshButton.systemImage, "arrow.clockwise")
        let files = ["Views/ComparisonTasksView.swift", "Views/ComparisonSpecificationsView.swift",
                     "Views/InsightsEpisodesView.swift", "Views/InsightsView.swift", "Views/MissionDraftsView.swift",
                     "Views/Settings/ConsentSection.swift", "Views/Settings/StartupSection.swift",
                     "Views/Settings/WatchingSection.swift"]
        for file in files {
            let source = try Self.text(file)
            XCTAssertTrue(source.contains("GlassRefreshButton("), "\(file) has no refresh icon button")
            for words in ["Button(model.text(\"refresh\"))", "Button(text(\"comparison_task_list\"))",
                          "Button(text(\"comparison_specification_refresh\"))", "Button(SettingsLegacyWords.refresh"] {
                let pattern = "(?<![A-Za-z])" + NSRegularExpression.escapedPattern(for: words)
                XCTAssertNil(source.range(of: pattern, options: .regularExpression), "\(file) has a Refresh in words again")
            }
        }
        // In the drafts' row it comes after the spacer, on the right.
        let drafts = try Self.text("Views/MissionDraftsView.swift")
        let spacer = try XCTUnwrap(drafts.range(of: "Spacer(minLength: 0)\n            // At the row's trailing end"))
        let refresh = try XCTUnwrap(drafts.range(of: "GlassRefreshButton(model.text(\"refresh\"))"))
        XCTAssertLessThan(spacer.lowerBound, refresh.lowerBound)
        let card = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCDesign/Components/Containers.swift"),
            encoding: .utf8)
        XCTAssertFalse(card.contains("GlassRefreshButton(refresh.label, size: 12"), "the icon card's refresh is beside its title again")
        XCTAssertTrue(card.contains(
            "Spacer(minLength: GlassTokens.Space.s4)\n                // The re-read control at the head's trailing end"))
        XCTAssertTrue(card.contains("GlassRefreshButton(refresh.label, small: true, isRefreshing: refresh.isRefreshing, action: refresh.action)"))
    }

    /// Home's small cards draw their icons as watermarks (owner,
    /// 2026-10-20): large on the right, tilted as the promotions' graphics
    /// are, cut off by the card's edges, behind the words, a little faded.
    func test_homesSmallCardsCarryWatermarks() throws {
        XCTAssertEqual(GlassCardWatermark.angle, -12)
        XCTAssertLessThan(GlassCardWatermark.opacity, 1)
        XCTAssertGreaterThan(GlassCardWatermark.opacity, 0.4)
        XCTAssertGreaterThan(GlassCardWatermark.bleed.width, 0)
        XCTAssertGreaterThan(GlassCardWatermark.bleed.height, 0)
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertEqual(home.components(separatedBy: "iconPlacement: .watermark").count - 1, 2)
        XCTAssertTrue(home.contains(".glassCardWatermark(HomeStatIcons.uncontributed)"))
        // Behind the words and over the fill: before the card's tier.
        for icon in ["uncontributed"] {
            let mark = try XCTUnwrap(home.range(of: ".glassCardWatermark(HomeStatIcons.\(icon))"))
            let tier = try XCTUnwrap(home.range(of: ".glassTier(.card", range: mark.upperBound..<home.endIndex))
            XCTAssertLessThan(home.distance(from: mark.upperBound, to: tier.lowerBound), 200, icon)
        }
        let card = try String(
            contentsOf: GlassSurfaceRulesTests.root.deletingLastPathComponent()
                .appendingPathComponent("TCDesign/Components/StatCard.swift"),
            encoding: .utf8)
        XCTAssertTrue(card.contains(
            ".glassCardWatermark(iconPlacement == .watermark ? systemImage : nil)\n        .glassTier(.card,"))
        XCTAssertTrue(card.contains(".clipShape(RoundedRectangle(cornerRadius: radius, style: .continuous))"))
    }
}
