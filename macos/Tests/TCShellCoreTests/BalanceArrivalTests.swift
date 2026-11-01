import XCTest

@testable import TCShellCore

/// NEAR AI credit arriving: what the balance reply carries for it, and when
/// the Balance tile animates. Every figure is synthetic.
final class BalanceArrivalTests: XCTestCase {
    private let dollar: Int64 = 1_000_000_000

    func testTheReplyCarriesTheChangeAndTheReceivedTotal() {
        let status = BalanceStatus.parse(fromJSON: """
            {"state":"known","scale":9,"remaining_nanos":8200000000,
             "change_24h_nanos":3200000000,"change_24h_text":"+$3.20 in the last 24 hours",
             "received_total_nanos":12000000000}
            """)
        XCTAssertEqual(status.change24hNanos, 3_200_000_000)
        XCTAssertEqual(status.change24hText, "+$3.20 in the last 24 hours")
        XCTAssertEqual(status.receivedTotalNanos, 12_000_000_000)
    }

    /// Null is not zero: a daemon with no reading from a day ago, or one too
    /// old to send these keys, gives nothing to draw.
    func testNullAndAbsentKeysStayUnknown() {
        let nulls = BalanceStatus.parse(fromJSON: """
            {"state":"known","scale":9,"remaining_nanos":1,
             "change_24h_nanos":null,"change_24h_text":null,"received_total_nanos":null}
            """)
        XCTAssertNil(nulls.change24hNanos)
        XCTAssertNil(nulls.change24hText)
        XCTAssertNil(nulls.receivedTotalNanos)
        let older = BalanceStatus.parse(fromJSON: #"{"state":"known","scale":9,"remaining_nanos":1}"#)
        XCTAssertNil(older.change24hNanos)
        XCTAssertNil(older.receivedTotalNanos)
    }

    func testDisplayDirectionIsOptionalAndBounded() {
        let neutral = BalanceStatus.parse(fromJSON: #"{"state":"known","change_24h_nanos":9000000,"change_24h_direction":0}"#)
        XCTAssertEqual(neutral.change24hNanos, 9_000_000)
        XCTAssertEqual(neutral.change24hDirection, 0)
        let older = BalanceStatus.parse(fromJSON: #"{"state":"known","change_24h_nanos":2000000000}"#)
        XCTAssertNil(older.change24hDirection)
        let invalid = BalanceStatus.parse(fromJSON: #"{"state":"known","change_24h_direction":2}"#)
        XCTAssertNil(invalid.change24hDirection)
    }

    func testNewCreditCountsUpFromTheFigureLastShown() {
        let effect = BalanceArrival.effect(
            shown: .init(receivedNanos: 4 * dollar, remainingNanos: 5 * dollar),
            now: .init(receivedNanos: 6 * dollar, remainingNanos: 7 * dollar),
            reduceMotion: false)
        XCTAssertEqual(effect, .countUp(fromNanos: 5 * dollar, toNanos: 7 * dollar))
    }

    /// Reduce Motion: no count, only a highlight that does not move.
    func testReduceMotionIsAStaticHighlight() {
        let effect = BalanceArrival.effect(
            shown: .init(receivedNanos: 4 * dollar, remainingNanos: 5 * dollar),
            now: .init(receivedNanos: 6 * dollar, remainingNanos: 7 * dollar),
            reduceMotion: true)
        XCTAssertEqual(effect, .highlight)
    }

    /// Every arrival animates, however small: the 25% rule is only for
    /// notifications.
    func testEveryArrivalAnimates() {
        let effect = BalanceArrival.effect(
            shown: .init(receivedNanos: 100 * dollar, remainingNanos: 5 * dollar),
            now: .init(receivedNanos: 100 * dollar + 1, remainingNanos: 5 * dollar),
            reduceMotion: false)
        // The figure did not move enough to count: highlighted instead.
        XCTAssertEqual(effect, .highlight)
    }

    func testNothingNewOrNothingKnownDoesNotAnimate() {
        let same = BalanceArrival.Reading(receivedNanos: 4 * dollar, remainingNanos: 5 * dollar)
        // The first figure this app shows: nothing to compare with.
        XCTAssertEqual(BalanceArrival.effect(shown: nil, now: same, reduceMotion: false), .none)
        // Nothing new arrived; spending moved the balance down.
        XCTAssertEqual(BalanceArrival.effect(
            shown: same, now: .init(receivedNanos: 4 * dollar, remainingNanos: 2 * dollar),
            reduceMotion: false), .none)
        // The daemon has not finished reading the ledger.
        XCTAssertEqual(BalanceArrival.effect(
            shown: same, now: .init(receivedNanos: nil, remainingNanos: 9 * dollar),
            reduceMotion: false), .none)
        // A smaller total is another organization's ledger, not an arrival.
        XCTAssertEqual(BalanceArrival.effect(
            shown: same, now: .init(receivedNanos: dollar, remainingNanos: 9 * dollar),
            reduceMotion: false), .none)
    }

    /// No remaining figure (no limit set) still highlights the arrival.
    func testAnArrivalWithNoFigureToCountHighlights() {
        XCTAssertEqual(BalanceArrival.effect(
            shown: .init(receivedNanos: 4 * dollar, remainingNanos: nil),
            now: .init(receivedNanos: 6 * dollar, remainingNanos: nil),
            reduceMotion: false), .highlight)
    }

    /// The count's steps run from the old figure to the new, ending exactly
    /// on the new one.
    func testTheCountEndsOnTheNewFigure() {
        let steps = BalanceArrival.countSteps(fromNanos: 5 * dollar, toNanos: 7 * dollar, frames: 4)
        XCTAssertEqual(steps, [5_500_000_000, 6_000_000_000, 6_500_000_000, 7_000_000_000])
        XCTAssertEqual(BalanceArrival.countSteps(fromNanos: 1, toNanos: 2, frames: 0), [2])
    }

    func testCreditNotificationsOpenPrivateAI() {
        XCTAssertEqual(NudgeSurface.Kind(rawValue: "near_ai_credit"), .nearAiCredit)
        XCTAssertEqual(NudgeSurface.intent(actionId: "review", kind: .nearAiCredit), .review(.nearAiCredit))
        XCTAssertEqual(NudgeSurface.intent(actionId: "not_now", kind: .nearAiCredit), .notNow(.nearAiCredit))
        XCTAssertNil(NudgeSurface.intent(actionId: "see_history", kind: .nearAiCredit))
        // The daemon keeps no opened or declined stamp for this kind.
        XCTAssertEqual(NudgeSurface.effect(.review(.nearAiCredit)),
                       .init(request: .dismissed, destination: .privateAI))
        XCTAssertEqual(NudgeSurface.effect(.notNow(.nearAiCredit)),
                       .init(request: .dismissed, destination: nil))
        for place in [NudgeSurface.Place.traces, .history] {
            XCTAssertFalse(place.kinds.contains(.nearAiCredit))
        }
        let note = NudgeSurface.notification(.init(
            kind: "near_ai_credit", title: "T", body: "B",
            actions: [.init(id: "review", label: "View balance"), .init(id: "not_now", label: "Not now")]))
        XCTAssertEqual(note?.defaultIntent, .review(.nearAiCredit))
        XCTAssertEqual(note?.actions.count, 2)
    }
}
