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

    func test_aRefusedWriteIsKept() async {
        let store = NudgeSettingsStore(client: SampleDaemonClient(.coreDown))
        await store.set(.suggestions, on: false)
        XCTAssertEqual(store.writeError, .unreachable)
    }
}
