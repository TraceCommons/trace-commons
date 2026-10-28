import Foundation
@testable import TCShellCore
import XCTest

/// `subscription_usage` (PR #1608 design, 3.3) as the Insights card reads
/// it: the reply's shapes decode, a window not reported is absent rather
/// than zero, and every line the card draws is the core's words filled
/// with the reply's figures.
final class SubscriptionUsageTests: XCTestCase {
    static func decode(_ json: String) throws -> DaemonData.SubscriptionUsage {
        try DaemonDataDecoding.decoder().decode(DaemonData.SubscriptionUsage.self, from: Data(json.utf8))
    }

    /// The words, as `{key}` with their holes, so a test reads which key
    /// and which figure made a line.
    static let words: [String: String] = {
        // The bare "codex" key is the Insights source label ("Codex
        // rollout"); the card must never read it.
        var copy: [String: String] = ["codex": "Codex rollout"]
        for key in SubscriptionUsageWords.Key.all { copy[key] = key }
        copy[SubscriptionUsageWords.Key.provider("codex")] = "Codex"
        copy[SubscriptionUsageWords.Key.used] = "{n}% used"
        copy[SubscriptionUsageWords.Key.resets] = "Resets {relative}"
        copy[SubscriptionUsageWords.Key.updated] = "Updated {age} ago"
        copy[SubscriptionUsageWords.Key.lastUpdated] = "Last updated {age} ago"
        copy[SubscriptionUsageWords.Key.windowHours] = "{n}-hour"
        copy[SubscriptionUsageWords.Key.windowDays] = "{n}-day"
        copy[SubscriptionUsageWords.Key.creditsBalance] = "{balance} credits"
        copy[SubscriptionUsageWords.Key.resetCredits] = "{n} reset credits"
        copy[SubscriptionUsageWords.Key.plan("plus")] = "ChatGPT Plus"
        copy[SubscriptionUsageWords.Key.plan("business")] = "ChatGPT Business"
        return copy
    }()

    static let now = DaemonTimestamp.parse("2026-10-24T15:06:11+00:00")!

    static let observed = """
    {"enabled":true,"logins":[{"provider":"codex","state":"observed","reason":null,"refreshing":false,
    "source":"cli","updated_at":"2026-10-24T15:02:11+00:00","stale":false,"plan":"plus",
    "limit_reached":false,"windows":[
    {"lane":"session","label":null,"duration_mins":300,"used_permille":420,"resets_at":"2026-10-24T18:40:00+00:00"},
    {"lane":"weekly","label":null,"duration_mins":10080,"used_permille":130,"resets_at":"2026-10-29T09:00:00+00:00"},
    {"lane":"model","label":"Spark","duration_mins":null,"used_permille":50,"resets_at":null}],
    "credits":{"has_credits":true,"unlimited":false,"balance":"12.50"},"reset_credits":1}]}
    """

    // MARK: Decoding

    func test_theSwitchOffShapeDecodesWithNoLogins() throws {
        let off = try Self.decode(#"{"enabled":false}"#)
        XCTAssertFalse(off.enabled)
        XCTAssertNil(off.logins)
        XCTAssertEqual(SubscriptionUsageWords.rows(off, copy: Self.words, now: Self.now), [])
    }

    func test_anObservedLoginDecodesEveryField() throws {
        let usage = try Self.decode(Self.observed)
        XCTAssertTrue(usage.enabled)
        let login = try XCTUnwrap(usage.logins?.first)
        XCTAssertEqual(login.provider, "codex")
        XCTAssertEqual(login.state, "observed")
        XCTAssertNil(login.reason)
        XCTAssertEqual(login.refreshing, false)
        XCTAssertEqual(login.source, "cli")
        XCTAssertEqual(login.updatedAt, "2026-10-24T15:02:11+00:00")
        XCTAssertEqual(login.stale, false)
        XCTAssertEqual(login.plan, "plus")
        XCTAssertEqual(login.limitReached, false)
        XCTAssertEqual(login.windows?.map(\.lane), ["session", "weekly", "model"])
        XCTAssertEqual(login.windows?.map(\.usedPermille), [420, 130, 50])
        XCTAssertEqual(login.windows?.map(\.durationMins), [300, 10_080, nil])
        XCTAssertEqual(login.windows?.last?.label, "Spark")
        XCTAssertEqual(login.credits, .init(hasCredits: true, unlimited: false, balance: "12.50"))
        XCTAssertEqual(login.resetCredits, 1)
    }

    /// The shape the recorded Codex exchange produces: only the weekly
    /// primary window was reported, so only it is on the wire. The missing
    /// session window is absent from the card, never drawn as 0%.
    func test_anAbsentWindowIsNotDrawnAsZero() throws {
        let usage = try Self.decode("""
        {"enabled":true,"logins":[{"provider":"codex","state":"observed","reason":null,"refreshing":false,
        "source":"cli","updated_at":"2026-10-24T15:02:11Z","stale":false,"plan":"business",
        "limit_reached":false,"windows":[
        {"lane":"weekly","label":null,"duration_mins":10080,"used_permille":0,"resets_at":"2026-10-29T09:00:00Z"}],
        "credits":{"has_credits":false,"unlimited":false,"balance":null},"reset_credits":2}]}
        """)
        let row = try XCTUnwrap(SubscriptionUsageWords.rows(usage, copy: Self.words, now: Self.now).first)
        XCTAssertEqual(row.title, "ChatGPT Business")
        XCTAssertEqual(row.meters.map(\.name), [SubscriptionUsageWords.Key.windowWeekly])
        XCTAssertEqual(row.meters.first?.used, "0% used", "a measured zero stays zero")
        XCTAssertEqual(row.meters.first?.fraction, 0)
        XCTAssertEqual(row.credits, ["2 reset credits"], "no credits is no credits line")
    }

    /// A login with no reading carries no figures: every optional field is
    /// null or absent, and the reply still decodes.
    func test_anUnknownLoginDecodesWithItsFieldsAbsent() throws {
        let usage = try Self.decode("""
        {"enabled":true,"logins":[{"provider":"codex","state":"unknown","reason":"never_refreshed"}]}
        """)
        let login = try XCTUnwrap(usage.logins?.first)
        XCTAssertEqual(login.reason, "never_refreshed")
        XCTAssertNil(login.refreshing)
        XCTAssertNil(login.windows)
        XCTAssertNil(login.plan)
        XCTAssertNil(login.updatedAt)
        let row = SubscriptionUsageWords.row(login, id: 0, copy: Self.words, now: Self.now)
        XCTAssertEqual(row.title, "Codex")
        XCTAssertNil(row.planLine, "no reading names no plan, not even an unknown one")
        XCTAssertEqual(row.meters, [])
        XCTAssertEqual(row.status, [], "before any Refresh the button is all there is to say")
        XCTAssertNil(row.updated)
        XCTAssertFalse(row.refreshing)
    }

    /// A field this build does not know is ignored, and so is a label a
    /// later daemon adds: it reads as "Usage unknown".
    func test_unknownFieldsAndLabelsDecode() throws {
        let usage = try Self.decode("""
        {"enabled":true,"later":1,"logins":[{"provider":"codex","state":"unknown","reason":"some_new_reason",
        "refreshing":true,"source":null,"updated_at":null,"stale":false,"plan":null,"limit_reached":null,
        "windows":[],"credits":null,"reset_credits":null,"extra_usage":null}]}
        """)
        let row = try XCTUnwrap(SubscriptionUsageWords.rows(usage, copy: Self.words, now: Self.now).first)
        XCTAssertEqual(row.status, [SubscriptionUsageWords.Key.unknown])
        XCTAssertTrue(row.refreshing)
    }

    // MARK: The card's words

    func test_anObservedRowReadsInTheCoresWords() throws {
        let usage = try Self.decode(Self.observed)
        let row = try XCTUnwrap(SubscriptionUsageWords.rows(usage, copy: Self.words, now: Self.now).first)
        XCTAssertEqual(row.title, "ChatGPT Plus")
        XCTAssertNil(row.planLine)
        XCTAssertEqual(row.meters.map(\.name), [
            SubscriptionUsageWords.Key.window5h, SubscriptionUsageWords.Key.windowWeekly, "Spark",
        ])
        XCTAssertEqual(row.meters.map(\.used), ["42% used", "13% used", "5% used"])
        XCTAssertEqual(row.meters.map(\.fraction), [0.42, 0.13, 0.05])
        XCTAssertEqual(row.meters.map(\.model), [false, false, true])
        XCTAssertEqual(row.meters[0].resets,
                       "Resets " + Format.when(DaemonTimestamp.parse("2026-10-24T18:40:00+00:00")!, relativeTo: Self.now))
        XCTAssertNil(row.meters[2].resets, "no reset time, no line")
        XCTAssertEqual(row.credits, ["12.50 credits", SubscriptionUsageWords.Key.resetCreditOne])
        XCTAssertEqual(row.status, [])
        XCTAssertEqual(row.updated, "Updated " + SubscriptionUsageWords.age(240) + " ago")
    }

    /// A model window is drawn after the session and weekly ones whatever
    /// order the daemon sent.
    func test_modelWindowsComeLast() throws {
        let windows: [DaemonData.SubscriptionWindow] = [
            .init(lane: "model", label: "Spark", durationMins: nil, usedPermille: 10, resetsAt: nil),
            .init(lane: "weekly", label: nil, durationMins: 10_080, usedPermille: 20, resetsAt: nil),
        ]
        let row = SubscriptionUsageWords.row(Self.login(windows: windows), id: 0, copy: Self.words, now: Self.now)
        XCTAssertEqual(row.meters.map(\.name), [SubscriptionUsageWords.Key.windowWeekly, "Spark"])
    }

    func test_windowNamesFollowTheDuration() {
        func name(_ lane: String, _ minutes: Int?, _ label: String? = nil) -> String {
            SubscriptionUsageWords.windowName(
                .init(lane: lane, label: label, durationMins: minutes, usedPermille: 0, resetsAt: nil), copy: Self.words)
        }
        XCTAssertEqual(name("session", 300), SubscriptionUsageWords.Key.window5h)
        XCTAssertEqual(name("weekly", 10_080), SubscriptionUsageWords.Key.windowWeekly)
        XCTAssertEqual(name("session", 10_080), SubscriptionUsageWords.Key.windowWeekly,
                       "a weekly primary window is named by its length")
        XCTAssertEqual(name("session", 120), "2-hour")
        XCTAssertEqual(name("weekly", 2_880), "2-day")
        XCTAssertEqual(name("session", 45), SubscriptionUsageWords.Key.windowUnnamed)
        XCTAssertEqual(name("session", nil), SubscriptionUsageWords.Key.windowUnnamed)
        XCTAssertEqual(name("weekly", nil), SubscriptionUsageWords.Key.windowWeekly)
        XCTAssertEqual(name("model", nil, "Spark"), "Spark")
        XCTAssertEqual(name("model", nil, nil), SubscriptionUsageWords.Key.windowUnnamed)
    }

    func test_percentRoundsHalfUp() {
        XCTAssertEqual(SubscriptionUsageWords.percent(0), "0")
        XCTAssertEqual(SubscriptionUsageWords.percent(4), "0")
        XCTAssertEqual(SubscriptionUsageWords.percent(5), "1")
        XCTAssertEqual(SubscriptionUsageWords.percent(425), "43")
        XCTAssertEqual(SubscriptionUsageWords.percent(1_000), "100")
    }

    /// A figure outside 0..=1000 is not one: "Usage unknown", no bar.
    func test_anOutOfRangeFigureIsUnknownNotClamped() {
        let meter = SubscriptionUsageWords.meter(
            .init(lane: "session", label: nil, durationMins: 300, usedPermille: 1_200, resetsAt: nil),
            id: 0, stale: false, copy: Self.words, now: Self.now)
        XCTAssertEqual(meter.used, SubscriptionUsageWords.Key.unknown)
        XCTAssertNil(meter.fraction)
    }

    /// A stale reading says "Last updated" and why it could not be renewed;
    /// a window that has reset since it was read is "Usage unknown".
    func test_aStaleReadingKeepsItsFiguresUntilTheWindowResets() {
        let windows: [DaemonData.SubscriptionWindow] = [
            .init(lane: "session", label: nil, durationMins: 300, usedPermille: 420,
                  resetsAt: "2026-10-24T14:00:00+00:00"),
            .init(lane: "weekly", label: nil, durationMins: 10_080, usedPermille: 130,
                  resetsAt: "2026-10-29T09:00:00+00:00"),
        ]
        let row = SubscriptionUsageWords.row(
            Self.login(windows: windows, reason: "codex_running", stale: true), id: 0, copy: Self.words, now: Self.now)
        XCTAssertEqual(row.meters.map(\.used), [SubscriptionUsageWords.Key.unknown, "13% used"])
        XCTAssertEqual(row.meters.map(\.fraction), [nil, 0.13])
        XCTAssertNil(row.meters[0].resets)
        XCTAssertEqual(row.status, [SubscriptionUsageWords.Key.codexRunning])
        XCTAssertEqual(row.updated, "Last updated " + SubscriptionUsageWords.age(240) + " ago")
    }

    /// A fresh reading whose window has reset since (no Refresh, nothing
    /// polls) is "Usage unknown" for that window, never its old figure and
    /// never a card with no meter at all.
    func test_aWindowPastItsResetIsUnknownEvenWhenNotStale() {
        let windows: [DaemonData.SubscriptionWindow] = [
            .init(lane: "weekly", label: nil, durationMins: 10_080, usedPermille: 130,
                  resetsAt: "2026-10-24T14:00:00+00:00"),
        ]
        let row = SubscriptionUsageWords.row(Self.login(windows: windows), id: 0, copy: Self.words, now: Self.now)
        XCTAssertEqual(row.meters.map(\.name), [SubscriptionUsageWords.Key.windowWeekly])
        XCTAssertEqual(row.meters.map(\.used), [SubscriptionUsageWords.Key.unknown])
        XCTAssertEqual(row.meters.map(\.fraction), [nil])
        XCTAssertNil(row.meters[0].resets)
    }

    /// A stale reading whose refresh failed for a reason with no words of
    /// its own says only "Last updated": "Usage unknown" beside figures
    /// would contradict them.
    func test_aStaleTimeoutSaysOnlyLastUpdated() {
        let row = SubscriptionUsageWords.row(
            Self.login(windows: [], reason: "timeout", stale: true), id: 0, copy: Self.words, now: Self.now)
        XCTAssertEqual(row.status, [])
        XCTAssertEqual(row.updated, "Last updated " + SubscriptionUsageWords.age(240) + " ago")
    }

    func test_eachReasonHasItsWords() {
        let expected: [String: String] = [
            "codex_not_found": SubscriptionUsageWords.Key.codexNotFound,
            "codex_running": SubscriptionUsageWords.Key.codexRunning,
            "signed_out": SubscriptionUsageWords.Key.signedOutCodex,
            "not_subscription": SubscriptionUsageWords.Key.notSubscription,
            "blocked": SubscriptionUsageWords.Key.needsAttention,
            "timeout": SubscriptionUsageWords.Key.unknown,
            "unreadable": SubscriptionUsageWords.Key.unknown,
            "failed": SubscriptionUsageWords.Key.unknown,
        ]
        for (reason, key) in expected {
            let login = DaemonData.SubscriptionLogin(
                provider: "codex", state: "unknown", reason: reason, refreshing: false, source: nil,
                updatedAt: nil, stale: false, plan: nil, limitReached: nil, windows: [], credits: nil,
                resetCredits: nil)
            XCTAssertEqual(SubscriptionUsageWords.row(login, id: 0, copy: Self.words).status, [key], reason)
        }
        let off = DaemonData.SubscriptionLogin(
            provider: "codex", state: "unknown", reason: "off", refreshing: false, source: nil, updatedAt: nil,
            stale: false, plan: nil, limitReached: nil, windows: [], credits: nil, resetCredits: nil)
        XCTAssertEqual(SubscriptionUsageWords.row(off, id: 0, copy: Self.words).status, [], "off says nothing")
        let never = DaemonData.SubscriptionLogin(
            provider: "codex", state: "unknown", reason: "never_refreshed", refreshing: false, source: nil,
            updatedAt: nil, stale: false, plan: nil, limitReached: nil, windows: [], credits: nil, resetCredits: nil)
        let row = SubscriptionUsageWords.row(never, id: 0, copy: Self.words)
        XCTAssertEqual(row.status, [], "never refreshed says nothing beyond the button")
        XCTAssertEqual(row.title, "Codex", "the card's own provider name, not the source label")
    }

    /// With no provider name in the copy, the title is "Plan unknown",
    /// never the bare provider key or its Insights source label.
    func test_aMissingProviderNameIsNeverTheSourceLabel() {
        var copy = Self.words
        copy[SubscriptionUsageWords.Key.provider("codex")] = nil
        let row = SubscriptionUsageWords.row(Self.login(windows: [], plan: nil), id: 0, copy: copy, now: Self.now)
        XCTAssertEqual(row.title, SubscriptionUsageWords.Key.planUnknown)
    }

    func test_limitReachedIsSaid() {
        let row = SubscriptionUsageWords.row(Self.login(windows: [], limitReached: true), id: 0, copy: Self.words,
                                             now: Self.now)
        XCTAssertEqual(row.status, [SubscriptionUsageWords.Key.limitReached])
    }

    /// `unknown`, and a label this build has no display name for, read as
    /// the tool's name with "Plan unknown"; never the raw label.
    func test_anUnknownPlanIsTheToolsNameWithPlanUnknown() {
        for plan in ["unknown", "galactic"] {
            let row = SubscriptionUsageWords.row(Self.login(windows: [], plan: plan), id: 0, copy: Self.words,
                                                 now: Self.now)
            XCTAssertEqual(row.title, "Codex", plan)
            XCTAssertEqual(row.planLine, SubscriptionUsageWords.Key.planUnknown, plan)
        }
    }

    func test_creditLines() {
        func lines(_ credits: DaemonData.SubscriptionCredits?, _ resets: Int?) -> [String] {
            SubscriptionUsageWords.row(Self.login(windows: [], credits: credits, resetCredits: resets),
                                       id: 0, copy: Self.words, now: Self.now).credits
        }
        XCTAssertEqual(lines(.init(hasCredits: true, unlimited: true, balance: "5"), nil),
                       [SubscriptionUsageWords.Key.creditsUnlimited])
        XCTAssertEqual(lines(.init(hasCredits: true, unlimited: false, balance: nil), nil),
                       [SubscriptionUsageWords.Key.creditsAvailable])
        XCTAssertEqual(lines(.init(hasCredits: false, unlimited: false, balance: nil), 0), [])
        XCTAssertEqual(lines(nil, 3), ["3 reset credits"])
        XCTAssertEqual(lines(nil, 1), [SubscriptionUsageWords.Key.resetCreditOne], "one credit is singular")
    }

    func test_ageIsOneWholeUnitAndNeverZero() {
        XCTAssertEqual(SubscriptionUsageWords.age(0), SubscriptionUsageWords.age(60))
        XCTAssertNotEqual(SubscriptionUsageWords.age(0), "")
        XCTAssertNotEqual(SubscriptionUsageWords.age(3 * 3_600), SubscriptionUsageWords.age(60))
    }

    // MARK: Support

    static func login(windows: [DaemonData.SubscriptionWindow], reason: String? = nil, stale: Bool = false,
                      plan: String? = "plus", limitReached: Bool? = false,
                      credits: DaemonData.SubscriptionCredits? = nil,
                      resetCredits: Int? = nil) -> DaemonData.SubscriptionLogin {
        .init(provider: "codex", state: "observed", reason: reason, refreshing: false, source: "cli",
              updatedAt: "2026-10-24T15:02:11+00:00", stale: stale, plan: plan, limitReached: limitReached,
              windows: windows, credits: credits, resetCredits: resetCredits)
    }
}
