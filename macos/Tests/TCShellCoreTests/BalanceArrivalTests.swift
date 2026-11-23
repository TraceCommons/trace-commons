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

    func testIconArrivalTrackerFiresOnAnyRealIncrease() {
        var tracker = BalanceArrival.Tracker()
        // Nil before any reading does nothing.
        XCTAssertFalse(tracker.observe(nil))
        XCTAssertNil(tracker.lastReceivedNanos)
        // First known reading establishes the baseline without firing.
        XCTAssertFalse(tracker.observe(4 * dollar))
        XCTAssertEqual(tracker.lastReceivedNanos, 4 * dollar)
        // Same total does not fire.
        XCTAssertFalse(tracker.observe(4 * dollar))
        // Transient unavailable preserves the baseline.
        XCTAssertFalse(tracker.observe(nil))
        XCTAssertEqual(tracker.lastReceivedNanos, 4 * dollar)
        // The smallest real increase fires and advances the baseline.
        XCTAssertTrue(tracker.observe(4 * dollar + 1))
        XCTAssertEqual(tracker.lastReceivedNanos, 4 * dollar + 1)
        XCTAssertFalse(tracker.observe(4 * dollar + 1))
        // A smaller total (e.g. switched organization) re-baselines silently.
        XCTAssertFalse(tracker.observe(2 * dollar))
        XCTAssertEqual(tracker.lastReceivedNanos, 2 * dollar)
        XCTAssertTrue(tracker.observe(3 * dollar))

        // Trace Commons settled credit works without a NEAR AI sign-in.
        XCTAssertFalse(tracker.observeCreditFinal(nil))
        XCTAssertFalse(tracker.observeCreditFinal(1.5))
        XCTAssertEqual(tracker.lastCreditFinal, 1.5)
        XCTAssertFalse(tracker.observeCreditFinal(1.5))
        XCTAssertFalse(tracker.observeCreditFinal(nil))
        XCTAssertTrue(tracker.observeCreditFinal(1.6))
        XCTAssertEqual(tracker.lastCreditFinal, 1.6)
        XCTAssertFalse(tracker.observeCreditFinal(1.0))
        XCTAssertEqual(tracker.lastCreditFinal, 1.0)
        XCTAssertTrue(tracker.observeCreditFinal(1.1))
    }

    /// The Dock fires exactly when the edge rail's Balance tile does, for
    /// every reading of the NEAR AI total.
    func testTheDockFiresWhenTheBalanceTileDoes() {
        let readings: [Int64?] = [
            nil, 4 * dollar, 4 * dollar, nil, 4 * dollar + 1, 4 * dollar + 1,
            2 * dollar, nil, 3 * dollar, 3 * dollar + 500_000_000, 9 * dollar,
        ]
        var tracker = BalanceArrival.Tracker()
        var shown: BalanceArrival.Reading?
        for received in readings {
            let now = BalanceArrival.Reading(receivedNanos: received, remainingNanos: nil)
            let tile = BalanceArrival.effect(shown: shown, now: now, reduceMotion: false) != .none
            if received != nil { shown = now }
            XCTAssertEqual(tracker.observe(received), tile, "at \(String(describing: received))")
        }
    }

    /// The daemon sums single-precision credit, so a re-read can differ in
    /// the last digits. That is not an arrival.
    func testIconArrivalTrackerIgnoresFloatingPointNoise() {
        var tracker = BalanceArrival.Tracker()
        XCTAssertFalse(tracker.observeCreditFinal(40.77798080444336))
        XCTAssertFalse(tracker.observeCreditFinal(40.77798080444336 + 0.00001))
        XCTAssertEqual(tracker.lastCreditFinal, 40.77798080444336)
    }

    private func scratchDefaults() throws -> UserDefaults {
        let suite = "BalanceArrivalTests.\(UUID().uuidString)"
        let defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
        addTeardownBlock { UserDefaults().removePersistentDomain(forName: suite) }
        return defaults
    }

    /// Credit that lands while the app is closed or signed out plays once on
    /// the next reading, and not again.
    func testASavedBaselineCelebratesWhatArrivedWhileAway() throws {
        let defaults = try scratchDefaults()
        var before = BalanceArrival.Tracker()
        XCTAssertFalse(before.observe(4 * dollar))
        XCTAssertFalse(before.observeCreditFinal(10))
        before.save(forScope: "sha256:a", in: defaults)

        var relaunched = BalanceArrival.Tracker.stored(forScope: "sha256:a", in: defaults)
        XCTAssertEqual(relaunched, before)
        XCTAssertTrue(relaunched.observe(5 * dollar))
        XCTAssertTrue(relaunched.observeCreditFinal(10.1))
        relaunched.save(forScope: "sha256:a", in: defaults)

        var again = BalanceArrival.Tracker.stored(forScope: "sha256:a", in: defaults)
        XCTAssertFalse(again.observe(5 * dollar))
        XCTAssertFalse(again.observeCreditFinal(10.1))
    }

    /// Another account's larger total is not new credit.
    func testASavedBaselineBelongsToItsAccount() throws {
        let defaults = try scratchDefaults()
        var first = BalanceArrival.Tracker()
        XCTAssertFalse(first.observe(1 * dollar))
        first.save(forScope: "sha256:a", in: defaults)

        var other = BalanceArrival.Tracker.stored(forScope: "sha256:b", in: defaults)
        XCTAssertEqual(other, BalanceArrival.Tracker())
        XCTAssertFalse(other.observe(9 * dollar), "the first reading is a baseline")
    }

    /// With no account there is nothing to key a baseline to.
    func testNoAccountScopeNeverSavesOrLoads() throws {
        let defaults = try scratchDefaults()
        var tracker = BalanceArrival.Tracker()
        XCTAssertFalse(tracker.observe(1 * dollar))
        tracker.save(forScope: nil, in: defaults)
        XCTAssertEqual(BalanceArrival.Tracker.stored(forScope: nil, in: defaults), BalanceArrival.Tracker())
        XCTAssertNil(defaults.object(forKey: BalanceArrival.Tracker.baselineKey))
    }

    func testIconAnimationFrameSchedule() {
        let still = BalanceArrival.IconAnimation.frameTimes(reduceMotion: true)
        XCTAssertEqual(still, [BalanceArrival.IconAnimation.stillAt])

        let animated = BalanceArrival.IconAnimation.frameTimes(reduceMotion: false)
        XCTAssertGreaterThan(animated.count, 20)
        XCTAssertEqual(animated.first, 0)
        for pair in zip(animated, animated.dropFirst()) {
            XCTAssertLessThan(pair.0, pair.1)
        }
        XCTAssertLessThan(animated.last ?? .infinity, BalanceArrival.IconAnimation.duration)
    }

    func testIconAnimationPreferenceDefaultsOnAndHonorsOff() {
        let suite = "BalanceArrivalIconPrefTest-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }

        XCTAssertTrue(BalanceArrival.IconAnimation.isEnabled(in: defaults))
        defaults.set(false, forKey: BalanceArrival.IconAnimation.preferenceKey)
        XCTAssertFalse(BalanceArrival.IconAnimation.isEnabled(in: defaults))
        defaults.set(true, forKey: BalanceArrival.IconAnimation.preferenceKey)
        XCTAssertTrue(BalanceArrival.IconAnimation.isEnabled(in: defaults))
    }
}
