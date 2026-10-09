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
            .suggestions, .menuBarMark, .notifications,
            .notify("digest"), .notify("verdicts_landed"), .notify("idle_sessions"),
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
        let end = try XCTUnwrap(source.range(of: "struct NudgeOfferCard: View {", range: start.upperBound..<source.endIndex))
        XCTAssertFalse(source[start.upperBound..<end.lowerBound].contains("NudgeOfferCard"))
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

    /// Turn on writes the kind on, which also ends its offer; No thanks
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
        let history = try String(contentsOf: monitor.appendingPathComponent("HomeViews.swift"), encoding: .utf8)
        let traces = try String(contentsOf: monitor.appendingPathComponent("TracesViews.swift"), encoding: .utf8)
        XCTAssertTrue(history.contains("NudgeOfferCards(place: .history)"), "History draws no offer")
        XCTAssertTrue(traces.contains("NudgeOfferCards(place: .traces)"), "Traces draws no offer")
    }

    func test_aRefusedWriteIsKept() async {
        let store = NudgeSettingsStore(client: SampleDaemonClient(.coreDown))
        await store.set(.suggestions, on: false)
        XCTAssertEqual(store.writeError, .unreachable)
    }
}
