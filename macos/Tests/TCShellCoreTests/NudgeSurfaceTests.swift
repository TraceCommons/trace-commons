import XCTest
@testable import TCShellCore

/// The nudges' wire fields and what each surface draws from them: the
/// Traces and History cards, the menu-bar panel row, the mark, the
/// re-engagement notification and the requests an action sends. Every word
/// is the daemon's; these tests pin that a surface draws exactly those words,
/// and draws nothing when the daemon did not say something drawable.
final class NudgeSurfaceTests: XCTestCase {
    private func status(_ json: String) throws -> DaemonData.Status {
        try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(json.utf8))
    }

    private func nudge(_ json: String) throws -> DaemonData.Nudge? {
        try status(#"{"nudge":\#(json)}"#).nudge
    }

    private static let verdictsJSON = #"""
        {"state":"armed","lead":"verdicts_landed","count":3,"accepted":2,"held":1,"final":1,"credit_final":2.5,
         "since":"2026-10-07T09:30:00Z","mark":"news","mark_kinds":["verdicts_landed"],
         "text":{"title":"Since 7 October: 2 accepted, 1 held for privacy review","body":"2.5 credit is now final.",
                 "actions":[{"id":"see_history","label":"See history"}],"panel_row":"2 accepted, 1 held since 7 October"},
         "mark_text":{"accessibility":"New: 2 accepted and 1 held for privacy review.","tooltip":"Verdicts are in."}}
        """#

    private static let backlogJSON = #"""
        {"state":"armed","lead":"review_backlog","count":6,"mark":"none","mark_kinds":[],
         "text":{"title":"6 previewed sessions are waiting for a decision","body":"Body.",
                 "actions":[{"id":"review","label":"Review the 6 in Traces"},{"id":"not_now","label":"Not now"}],
                 "panel_row":"Previewed sessions are waiting for a decision"}}
        """#

    // MARK: - Decoding

    /// The recorded `status` replies carry the idle card's words and the
    /// halo's, exactly as the daemon composed them.
    func testRecordedIdleNudgeDecodesTheDaemonsWords() async throws {
        let status = try await SampleDaemonClient(.busyQueue).status()
        let nudge = try XCTUnwrap(status.nudge)
        XCTAssertEqual(nudge.state, "armed")
        XCTAssertEqual(nudge.lead, "idle_sessions")
        XCTAssertEqual(nudge.mark, "ready")
        XCTAssertEqual(nudge.text?.title, "14 traces from Claude Code and Codex have been idle for 3 days or more")
        XCTAssertEqual(nudge.text?.actions.map(\.id), ["review", "not_now"])
        XCTAssertEqual(nudge.text?.panelRow, "Some traces have been idle for 3 days or more")
        XCTAssertEqual(nudge.markText?.accessibility, "14 of them have been idle for 3 days or more.")
        XCTAssertEqual(nudge.markText?.tooltip, "14 idle for 3 days or more.")
        XCTAssertEqual(status.idleSessions?.count, 14)
        XCTAssertEqual(status.idleSessions?.thresholdDays, 3)
        // Nothing to say decodes as nothing.
        let quiet = try await SampleDaemonClient(.empty).status()
        XCTAssertEqual(quiet.nudge?.state, "none")
        XCTAssertNil(quiet.nudge?.text)
        XCTAssertNil(quiet.nudge?.markText)
        // An older daemon sends no nudge at all.
        let older = try await SampleDaemonClient(.unknownCounts).status()
        XCTAssertNil(older.nudge)
    }

    func testVerdictNewsDecodesItsFinalCredit() throws {
        let news = try XCTUnwrap(try nudge(Self.verdictsJSON))
        XCTAssertEqual(news.creditFinal, 2.5)
        XCTAssertEqual(news.text?.body, "2.5 credit is now final.")
        XCTAssertNil(try nudge(#"{"state":"none","lead":null}"#)?.creditFinal)
    }

    /// `mission_fit` and the estimate on a `list_pending` row; absent is
    /// nil, never 0, and `drawn` absent is nil, never true.
    func testAPendingRowDecodesMissionFitAndItsEstimate() throws {
        let json = #"""
            {"pending":[
             {"entry_id":"e1","source":"codex","project_id":"p","project_label":"P","state":"pending",
              "mission_fit":2,
              "credit_estimate":{"low":2.0,"high":4.5,"tier":"higher","calibration":"lef1.t2/cq3","basis":"published","drawn":true}},
             {"entry_id":"e2","source":"codex","project_id":"p","project_label":"P","state":"pending",
              "credit_estimate":{"low":1.0,"high":3.0,"calibration":"lef1.t1/cq3","basis":"built_in"}}]}
            """#
        let rows = try DaemonDataDecoding.decoder().decode(DaemonData.PendingList.self, from: Data(json.utf8)).pending
        XCTAssertEqual(rows[0].missionFit, 2)
        XCTAssertEqual(rows[0].creditEstimate?.drawn, true)
        XCTAssertEqual(rows[0].creditEstimate?.tier, "higher")
        XCTAssertNil(rows[1].missionFit)
        XCTAssertNil(rows[1].creditEstimate?.drawn)
    }

    /// `digest_due.fold`: the kind folded in, and its sentence, which the
    /// digest's `text` already ends with.
    func testDigestDueDecodesTheFold() {
        let frame = #"""
            {"event":"digest_due","data":{"pending":2,"contributed":0,"contributed_projects":[],"credit_pending":0,
             "text":"2 sessions are waiting. 2 sessions from Codex have been idle for 3 days or more.",
             "fold":{"kind":"idle_sessions","text":"2 sessions from Codex have been idle for 3 days or more."}}}
            """#
        guard case .digestDue(let digest) = DaemonDataEventParser.parse(frame) else {
            return XCTFail("not a digest")
        }
        XCTAssertEqual(digest.fold?.kind, "idle_sessions")
        XCTAssertEqual(digest.text, "2 sessions are waiting. 2 sessions from Codex have been idle for 3 days or more.")
        XCTAssertTrue(digest.text?.hasSuffix(digest.fold?.text ?? "-") ?? false)
        guard case .digestDue(let older) = DaemonDataEventParser.parse(#"{"event":"digest_due","data":{"pending":1,"text":"t"}}"#)
        else { return XCTFail("not a digest") }
        XCTAssertNil(older.fold)
    }

    /// `reengage_due` is its own event, carrying the daemon's words.
    func testReengageDueParsesAsItsOwnEvent() {
        let frame = #"""
            {"event":"reengage_due","data":{"kind":"idle_sessions","title":"Trace Commons",
             "body":"2 sessions from Codex have been idle for 3 days or more. Review them to send or keep.",
             "actions":[{"id":"review","label":"Review"},{"id":"not_now","label":"Not now"}]}}
            """#
        guard case .reengageDue(let due) = DaemonDataEventParser.parse(frame) else {
            return XCTFail("not a re-engagement")
        }
        XCTAssertEqual(due.kind, "idle_sessions")
        XCTAssertEqual(due.title, "Trace Commons")
        XCTAssertEqual(due.actions.map(\.label), ["Review", "Not now"])
        // Missing words are not a notification.
        XCTAssertEqual(
            DaemonDataEventParser.parse(#"{"event":"reengage_due","data":{"kind":"idle_sessions"}}"#),
            .unknown("reengage_due"))
    }

    // MARK: - Cards

    func testTracesDrawsTheIdleAndBacklogCardsOnly() async throws {
        let idle = try await SampleDaemonClient(.normalDay).status().nudge
        let card = try XCTUnwrap(NudgeSurface.card(idle, on: .traces))
        XCTAssertEqual(card.kind, .idleSessions)
        XCTAssertEqual(card.title, "2 traces from Claude Code have been idle for 3 days or more")
        XCTAssertEqual(card.body, idle?.text?.body)
        XCTAssertEqual(card.actions.map(\.label), ["Review", "Not now"])
        XCTAssertEqual(card.actions.map(\.intent), [.review(.idleSessions), .notNow(.idleSessions)])
        XCTAssertNil(NudgeSurface.card(idle, on: .history))

        let backlog = try nudge(Self.backlogJSON)
        XCTAssertEqual(NudgeSurface.card(backlog, on: .traces)?.actions.map(\.label), ["Review the 6 in Traces", "Not now"])
        XCTAssertNil(NudgeSurface.card(backlog, on: .history))
    }

    func testHistoryDrawsTheVerdictsCardOnly() throws {
        let verdicts = try nudge(Self.verdictsJSON)
        let card = try XCTUnwrap(NudgeSurface.card(verdicts, on: .history))
        XCTAssertEqual(card.kind, .verdictsLanded)
        XCTAssertEqual(card.body, "2.5 credit is now final.")
        XCTAssertEqual(card.actions, [NudgeSurface.Action(intent: .seeHistory, label: "See history")])
        XCTAssertNil(NudgeSurface.card(verdicts, on: .traces))
    }

    /// `none`, `unknown`, an absent nudge, a lead this build does not know,
    /// and an armed nudge without words all draw nothing.
    func testNothingDrawableDrawsNoCard() throws {
        XCTAssertNil(NudgeSurface.card(nil, on: .traces))
        XCTAssertNil(NudgeSurface.card(try nudge(#"{"state":"none","lead":null,"mark":"none"}"#), on: .traces))
        XCTAssertNil(NudgeSurface.card(try nudge(#"{"state":"unknown","lead":null,"mark":"unknown"}"#), on: .traces))
        XCTAssertNil(NudgeSurface.card(
            try nudge(#"{"state":"armed","lead":"weekly_recap","count":1,"text":{"title":"t","body":"","actions":[],"panel_row":"r"}}"#),
            on: .history))
        XCTAssertNil(NudgeSurface.card(try nudge(#"{"state":"armed","lead":"idle_sessions","count":2}"#), on: .traces))
        // A lead with words but not armed is not drawn either.
        XCTAssertNil(NudgeSurface.card(
            try nudge(#"{"state":"none","lead":"idle_sessions","text":{"title":"t","body":"","actions":[],"panel_row":"r"}}"#),
            on: .traces))
    }

    /// A button this build cannot act on is not drawn; an empty body is
    /// no body.
    func testUnknownActionsAreNotDrawnAndAnEmptyBodyIsNone() throws {
        let odd = try nudge(#"""
            {"state":"armed","lead":"idle_sessions","count":1,
             "text":{"title":"T","body":"","actions":[{"id":"launch","label":"Go"},{"id":"not_now","label":"Not now"}],"panel_row":"R"}}
            """#)
        let card = try XCTUnwrap(NudgeSurface.card(odd, on: .traces))
        XCTAssertNil(card.body)
        XCTAssertEqual(card.actions.map(\.label), ["Not now"])
    }

    // MARK: - Panel row

    func testThePanelRowIsTheLeadsOwnRowAndOpensItsPlace() async throws {
        let idle = NudgeSurface.panelRow(try await SampleDaemonClient(.normalDay).status().nudge)
        XCTAssertEqual(idle?.text, "Some traces have been idle for 3 days or more")
        XCTAssertEqual(idle?.intent, .review(.idleSessions))
        XCTAssertEqual(NudgeSurface.panelRow(try nudge(Self.verdictsJSON))?.intent, .seeHistory)
        XCTAssertEqual(NudgeSurface.panelRow(try nudge(Self.backlogJSON))?.intent, .review(.reviewBacklog))
        XCTAssertNil(NudgeSurface.panelRow(nil))
        XCTAssertNil(NudgeSurface.panelRow(try nudge(#"{"state":"unknown","lead":null}"#)))
    }

    // MARK: - The mark

    func testTheMarkIsOnlyNewsOrReadyAndOnlyWhileTheStripIsAvailable() throws {
        let news = try nudge(Self.verdictsJSON)
        XCTAssertEqual(NudgeSurface.mark(news, available: true), .news)
        XCTAssertEqual(NudgeSurface.markText(news, available: true)?.tooltip, "Verdicts are in.")
        XCTAssertEqual(NudgeSurface.mark(news, available: false), .none)
        XCTAssertNil(NudgeSurface.markText(news, available: false))

        let ready = try nudge(#"{"state":"armed","lead":"idle_sessions","mark":"ready","mark_kinds":["idle_sessions"],"mark_text":{"accessibility":"a","tooltip":"t"}}"#)
        XCTAssertEqual(NudgeSurface.mark(ready, available: true), .ready)

        for json in [#"{"state":"none","mark":"none"}"#, #"{"state":"unknown","mark":"unknown"}"#,
                     #"{"state":"none"}"#, #"{"state":"none","mark":"sparkle"}"#] {
            XCTAssertEqual(NudgeSurface.mark(try nudge(json), available: true), .none, json)
            XCTAssertNil(NudgeSurface.markText(try nudge(json), available: true), json)
        }
        XCTAssertEqual(NudgeSurface.mark(nil, available: true), .none)
    }

    // MARK: - Actions

    /// What each action sends, and where it goes. Review on the idle card
    /// opens Traces at the idle sessions; on the backlog card, at every
    /// waiting session. See history acknowledges the news. Not now declines
    /// the kind and goes nowhere.
    func testEachActionSendsItsRequestAndOpensItsPlace() {
        XCTAssertEqual(NudgeSurface.effect(.review(.idleSessions)),
                       .init(request: .opened(.idleSessions), destination: .traces(idleOnly: true)))
        XCTAssertEqual(NudgeSurface.effect(.review(.reviewBacklog)),
                       .init(request: .opened(.reviewBacklog), destination: .traces(idleOnly: false)))
        XCTAssertEqual(NudgeSurface.effect(.seeHistory),
                       .init(request: .opened(.verdictsLanded), destination: .history))
        XCTAssertEqual(NudgeSurface.effect(.notNow(.idleSessions)),
                       .init(request: .declined(.idleSessions), destination: nil))
    }

    func testOnlyActionsTheDaemonDefinesForAKindAreActedOn() {
        XCTAssertEqual(NudgeSurface.intent(actionId: "review", kind: .idleSessions), .review(.idleSessions))
        XCTAssertEqual(NudgeSurface.intent(actionId: "not_now", kind: .reviewBacklog), .notNow(.reviewBacklog))
        XCTAssertEqual(NudgeSurface.intent(actionId: "see_history", kind: .verdictsLanded), .seeHistory)
        // Verdict news has no Not now and is not reviewed in Traces.
        XCTAssertNil(NudgeSurface.intent(actionId: "not_now", kind: .verdictsLanded))
        XCTAssertNil(NudgeSurface.intent(actionId: "review", kind: .verdictsLanded))
        XCTAssertNil(NudgeSurface.intent(actionId: "see_history", kind: .idleSessions))
        XCTAssertNil(NudgeSurface.intent(actionId: "dismiss", kind: .idleSessions))
    }

    // MARK: - Notifications

    func testAReengagementNotificationCarriesTheDaemonsWordsAndActions() throws {
        let due = DaemonData.ReengageDue(
            kind: "idle_sessions", title: "Trace Commons", body: "Body.",
            actions: [.init(id: "review", label: "Review"), .init(id: "not_now", label: "Not now")])
        let note = try XCTUnwrap(NudgeSurface.notification(due))
        XCTAssertEqual(note.kind, .idleSessions)
        XCTAssertEqual(note.title, "Trace Commons")
        XCTAssertEqual(note.body, "Body.")
        XCTAssertEqual(note.actions.map(\.intent), [.review(.idleSessions), .notNow(.idleSessions)])
        // A click on the notification itself is its first action that is
        // not Not now.
        XCTAssertEqual(note.defaultIntent, .review(.idleSessions))

        let verdicts = try XCTUnwrap(NudgeSurface.notification(.init(
            kind: "verdicts_landed", title: "Trace Commons", body: "B",
            actions: [.init(id: "see_history", label: "See history")])))
        XCTAssertEqual(verdicts.defaultIntent, .seeHistory)
        // A kind this build does not know, or no words, is not posted.
        XCTAssertNil(NudgeSurface.notification(.init(kind: "weekly_recap", title: "T", body: "B", actions: [])))
        XCTAssertNil(NudgeSurface.notification(.init(kind: "idle_sessions", title: "", body: "", actions: [])))
    }
}
