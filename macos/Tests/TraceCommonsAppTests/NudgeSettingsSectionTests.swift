import TCDesign
@testable import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// The nudge switches in Settings: drawn from the daemon's settings in the
/// core's words, each write sent as its own request, and the one-time
/// offers answered.
@MainActor
final class NudgeSettingsSectionTests: XCTestCase {
    func test_theSwitchesAreTheDaemonsValuesInTheCoresWords() async {
        let store = NudgeSettingsStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertEqual(store.rows.map(\.id), [
            .suggestions, .menuBarMark, .settlementEstimate, .notifications,
            .notify("digest"), .notify("verdicts_landed"), .notify("idle_sessions"),
            .notify("near_ai_credit"),
        ])
        XCTAssertTrue(store.rows.allSatisfy(\.isOn))
        XCTAssertEqual(store.rows.first?.label, "Show suggestions in Traces, History and the menu bar")
        XCTAssertEqual(store.offers, [])
    }

    /// The one-time offers are drawn where their news is (History and
    /// Traces), never in Settings, which holds only the switches.
    func test_settingsDrawsNoOffer() throws {
        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let source = try String(
            contentsOf: root.appendingPathComponent("Sources/TraceCommonsApp/Views/Settings/NudgeSettingsSection.swift"),
            encoding: .utf8)
        let start = try XCTUnwrap(source.range(of: "struct NudgeSettingsSection: View {"))
        XCTAssertFalse(source[start.upperBound...].contains("store.offers("), "Settings draws an offer")
        XCTAssertFalse(source.contains("struct NudgeOfferCard"), "the inline offer card is back")
    }

    /// Unread settings draw no switch at all, never a row of offs.
    func test_unreadSettingsDrawNoSwitch() async {
        let store = NudgeSettingsStore(client: SampleDaemonClient(.coreDown))
        await store.load()
        XCTAssertEqual(store.rows, [])
        XCTAssertEqual(store.readError, .unreachable)
    }

    func test_eachSwitchSendsItsOwnRequest() async {
        let client = SampleDaemonClient(.normalDay)
        let store = NudgeSettingsStore(client: client)
        await store.load()
        await store.set(.suggestions, on: false)
        await store.set(.menuBarMark, on: false)
        await store.set(.notifications, on: false)
        await store.set(.notify("idle_sessions"), on: false)
        XCTAssertEqual(client.nudgeCalls, [
            "set_suggestions_enabled false", "set_menu_bar_mark_enabled false",
            "set_notifications_enabled false", "set_notify_kind idle_sessions false",
        ])
        XCTAssertNil(store.writeError)
    }

    /// Enable writes the kind on, which also ends its offer; No thanks
    /// only clears the offer's marker.
    func test_anOfferIsAnsweredWithOneWrite() async {
        let client = SampleDaemonClient(.normalDay)
        let store = NudgeSettingsStore(client: client)
        let offer = NudgeSettings.Offer(kind: "verdicts_landed", text: "t", accept: "a", decline: "d")
        await store.answer(offer, accept: true)
        await store.answer(NudgeSettings.Offer(kind: "idle_sessions", text: "t", accept: "a", decline: "d"), accept: false)
        XCTAssertEqual(client.nudgeCalls, ["set_notify_kind verdicts_landed true", "set_settings idle_offer_pending false"])
    }

    /// The system's permission prompt follows an accepted offer only when
    /// the daemon took the write: a refused one leaves the kind off, and a
    /// prompt for it would ask permission for nothing.
    func test_thePromptFollowsOnlyAnAcceptedWrite() async {
        let offer = NudgeSettings.Offer(kind: "verdicts_landed", text: "t", accept: "a", decline: "d")
        var asked = 0

        let refused = NudgeSettingsStore(client: SampleDaemonClient(.coreDown))
        let refusedTook = await refused.accept(offer) { asked += 1 }
        XCTAssertFalse(refusedTook)
        XCTAssertEqual(refused.writeError, .unreachable)
        XCTAssertEqual(asked, 0, "prompted after a refused write")

        let took = NudgeSettingsStore(client: SampleDaemonClient(.normalDay))
        let tookTook = await took.accept(offer) { asked += 1 }
        XCTAssertTrue(tookTook)
        XCTAssertEqual(asked, 1)

        // No thanks never prompts.
        let declined = await took.answer(offer, accept: false)
        XCTAssertTrue(declined)
        XCTAssertEqual(asked, 1)
    }

    /// History and Traces each draw their own kind's offer, through the
    /// same store and words as Settings.
    func test_thePagesDrawTheirOwnOffers() async throws {
        let store = NudgeSettingsStore(client: SampleDaemonClient(.normalDay))
        await store.load()
        XCTAssertEqual(store.offers(on: .history), NudgeSettings.offers(store.settings, copy: store.copy, on: .history))
        XCTAssertEqual(store.offers(on: .traces), NudgeSettings.offers(store.settings, copy: store.copy, on: .traces))

        let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        let monitor = root.appendingPathComponent("Sources/TraceCommonsApp/Views/Monitor")
        // As popover notices over each page (owner, 2026-10-19).
        let notices = try String(contentsOf: monitor.appendingPathComponent("MonitorNotices.swift"), encoding: .utf8)
        let history = try XCTUnwrap(notices.range(of: "    private var historyOffers: [GlassPopoverItem] {"))
        let traces = try XCTUnwrap(notices.range(of: "    private var tracesOffers: [GlassPopoverItem] {"))
        XCTAssertTrue(notices[history.upperBound...].prefix(400).contains("offerItems(.history)"), "History draws no offer")
        XCTAssertTrue(notices[traces.upperBound...].prefix(2000).contains("offerItems(.traces)"), "Traces draws no offer")
        XCTAssertTrue(notices.contains("offers.offers(on: place)"), "the offers are not the store's")
    }

    func test_aRefusedWriteIsKept() async {
        let store = NudgeSettingsStore(client: SampleDaemonClient(.coreDown))
        await store.set(.suggestions, on: false)
        XCTAssertEqual(store.writeError, .unreachable)
    }

    func testSettingsShowTheEstimateSwitch() throws {
        let copy = NudgeCopy(table: [
            "SETTING_SETTLEMENT_ESTIMATE": "Estimate what traces on this Mac are worth",
            "SETTING_SETTLEMENT_ESTIMATE_HELP": "Worked out on this Mac.",
        ])
        let settings = try JSONDecoder().decode(
            DaemonData.Settings.self, from: Data(#"{"settlement_estimate_enabled":true}"#.utf8))
        let rows = NudgeSettings.rows(settings, copy: copy, digestHelp: nil)
        XCTAssertEqual(rows.map(\.id), [.settlementEstimate])
        XCTAssertEqual(rows.first?.label, "Estimate what traces on this Mac are worth")
        XCTAssertEqual(rows.first?.isOn, true)
        // Unknown is no switch, never off.
        let unread = try JSONDecoder().decode(DaemonData.Settings.self, from: Data("{}".utf8))
        XCTAssertEqual(NudgeSettings.rows(unread, copy: copy, digestHelp: nil), [])
    }

    func testTheEstimateSwitchWritesSettlementEstimateEnabled() async throws {
        let client = SampleDaemonClient(.normalDay)
        try await NudgeSettings.write(.settlementEstimate, on: false, through: client)
        XCTAssertEqual(client.nudgeCalls, ["set_settings settlement_estimate_enabled false"])
    }
}
