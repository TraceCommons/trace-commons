import TCBridge
import TCDesign
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's Join (#1030 `join-screen.tsx`): the invite with its host before the
/// daemon runs and the joined line after it, the account cards, and "Skip:
/// watch only" until an account exists. The decisions are `JoinScreenLayout`'s, so
/// they are tested without drawing; the source is read for the rest, the
/// house pattern for a SwiftUI view.
@MainActor
final class JoinScreenTests: XCTestCase {
    func test_organizationPreparationRequiresAnAuthenticatedUnfinishedJoin() {
        var state = FirstRunState()
        state.step = .join
        XCTAssertFalse(JoinScreenLayout.canPrepareOrganization(state, isCommitting: false, completed: false))
        state.account = .nearAI
        XCTAssertFalse(JoinScreenLayout.canPrepareOrganization(state, isCommitting: false, completed: false))
        state.signedIn = true
        XCTAssertTrue(JoinScreenLayout.canPrepareOrganization(state, isCommitting: false, completed: false))
        XCTAssertFalse(JoinScreenLayout.canPrepareOrganization(state, isCommitting: true, completed: false))
        XCTAssertFalse(JoinScreenLayout.canPrepareOrganization(state, isCommitting: false, completed: true))
        state.step = .privateAI
        XCTAssertFalse(JoinScreenLayout.canPrepareOrganization(state, isCommitting: false, completed: false))
    }

    func test_singleOrganizationWaitsForMetadataAndStartsOnlyOnce() throws {
        let copy = try coreCopy()
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: dir) }
        let grant = try XCTUnwrap(AutomaticGrantCopy.decode(
            fromJSON: TCCoreCopy.automaticContributionCopyJSON(configDir: dir.path)))
        let state = FirstRunState(
            step: .join, account: .nearAI, sharingChosen: true, enrolledInvite: "INVITE-1", signedIn: true)
        let required = ConsentScope(name: "debugging_evaluation", title: "Debugging", description: "",
                                    alwaysOn: true, grantsDataUse: true)
        func ready(scope: ConsentScope?, attempted: Bool = false, organizationReady: Bool = true) -> Bool {
            JoinScreenLayout.canAutomaticallyStart(
                state, organizationReady: organizationReady, attempted: attempted, starting: false,
                uses: copy.uses, requiredScope: scope, grant: grant, isCommitting: false, completed: false)
        }
        // Organization verification can finish before the daemon's consent metadata arrives.
        XCTAssertFalse(ready(scope: nil))
        XCTAssertTrue(ready(scope: required))
        XCTAssertFalse(ready(scope: required, attempted: true))
        XCTAssertFalse(ready(scope: required, organizationReady: false))
    }

    private func coreCopy() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    private static func source() throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/JoinScreen.swift")
        return try String(contentsOf: url, encoding: .utf8)
    }

    private static func lookup(_ json: String) throws -> DaemonData.InviteLookup {
        try JSONDecoder().decode(DaemonData.InviteLookup.self, from: Data(json.utf8))
    }

    /// A stand-in for `TCInvite.issuerHost`: the host after a fixed prefix,
    /// nil for anything else, as the core refuses anything not an invite.
    private static func host(_ invite: String) -> String? {
        invite.hasPrefix("invite:") ? String(invite.dropFirst("invite:".count)) : nil
    }

    func test_joinShowsTheHostBeforeTheDaemonRuns() throws {
        let copy = try coreCopy()
        var state = FirstRunState()
        XCTAssertEqual(JoinScreenLayout.inviteLine(state, lookup: nil, failure: nil, copy: copy.join), .hidden)

        // Look up reads the host locally; nothing reaches the daemon, which
        // does not run yet: Join holds no daemon to reach.
        let source = try Self.source()
        XCTAssertFalse(source.contains("DaemonClient"))
        XCTAssertFalse(source.contains("FirstRunDaemon"))
        let looked = JoinScreenLayout.lookUp("  invite:issuer.example  ", in: state, failure: nil, host: Self.host)
        XCTAssertEqual(looked.outcome, .found)
        state = looked.state
        XCTAssertEqual(state.invite, "invite:issuer.example")
        XCTAssertEqual(state.issuerHost, "issuer.example")
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(state, lookup: nil, failure: nil, copy: copy.join), .host("issuer.example"))
        XCTAssertTrue(JoinScreenLayout.inviteIsEditable(state))

        // Once the daemon enrolled it, the joined line with the pay range
        // replaces the field, so a second invite cannot be pasted.
        state.daemonStarted = true
        state.enrolledInvite = state.invite
        let found = try Self.lookup(
            #"{"valid":true,"issuer_display_name":"Sample Labs","credit_range":{"min":10,"max":40,"unit":"points_per_accepted_trace"}}"#)
        let range = copy.join.payRangePoints
            .replacingOccurrences(of: "{min}", with: "10")
            .replacingOccurrences(of: "{max}", with: "40")
        let joined = copy.join.inviteJoined
            .replacingOccurrences(of: "{host}", with: "issuer.example")
            .replacingOccurrences(of: "{pay_range}", with: range)
        let line = JoinScreenLayout.inviteLine(state, lookup: found, failure: nil, copy: copy.join)
        XCTAssertEqual(line, .joined(joined))
        // The daemon's wire unit never reaches the screen.
        if case .joined(let text) = line { XCTAssertFalse(text.contains("_"), text) }

        // One figure reads as one figure; a unit this build cannot word reads
        // as the core's Unknown, never as the wire label or a dash.
        let one = try Self.lookup(
            #"{"valid":true,"credit_range":{"min":5,"max":5,"unit":"points_per_accepted_trace"}}"#)
        XCTAssertEqual(
            JoinScreenLayout.payRange(one, copy: copy.join),
            copy.join.payRangePointsOne.replacingOccurrences(of: "{min}", with: "5"))
        let foreign = try Self.lookup(#"{"valid":true,"credit_range":{"min":1,"max":2,"unit":"dollars"}}"#)
        XCTAssertEqual(JoinScreenLayout.payRange(foreign, copy: copy.join), copy.join.unknown)
        XCTAssertEqual(copy.join.unknown, "Unknown")
        XCTAssertFalse(JoinScreenLayout.inviteIsEditable(state))

        // An unknown range reads as Unknown, never as zero or a dash.
        let noRange = try Self.lookup(#"{"valid":true}"#)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(state, lookup: noRange, failure: nil, copy: copy.join),
            .joined(
                copy.join.inviteJoined
                    .replacingOccurrences(of: "{host}", with: "issuer.example")
                    .replacingOccurrences(of: "{pay_range}", with: copy.join.unknown)))
        // An unknown host reads as Unknown too.
        var hostless = state
        hostless.issuerHost = nil
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(hostless, lookup: noRange, failure: nil, copy: copy.join),
            .joined(
                copy.join.inviteJoined
                    .replacingOccurrences(of: "{host}", with: copy.join.unknown)
                    .replacingOccurrences(of: "{pay_range}", with: copy.join.unknown)))
        XCTAssertFalse(try Self.source().contains("\"—\""))

        // The screen's own host function is the core's (`TCInvite.issuerHost`),
        // so a real invite shows its issuer's host before the daemon runs.
        XCTAssertNil(JoinScreen.defaultIssuerHost("not an invite"))
        let real = JoinScreenLayout.lookUp(
            LinkedChannel.invite(), in: FirstRunState(), failure: nil,
            host: JoinScreen.defaultIssuerHost)
        XCTAssertEqual(real.outcome, .found)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(real.state, lookup: nil, failure: real.failure, copy: copy.join),
            .host(LinkedChannel.inviteHost))
    }

    /// Review Focus 2: back on Join after a dead invite, with the daemon
    /// running, the field is editable again and shows the core's invite error.
    func test_aDeadInviteReopensTheFieldWithTheCoresError() throws {
        let copy = try coreCopy()
        let state = FirstRunState(
            step: .join, invite: "invite:issuer.example", issuerHost: "issuer.example", account: .nearAI,
            daemonStarted: true)
        XCTAssertTrue(JoinScreenLayout.inviteIsEditable(state))
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(state, lookup: nil, failure: .inviteDead(label: "exhausted"), copy: copy.join),
            .error(copy.join.inviteDead))
        XCTAssertNotEqual(copy.join.inviteDead, copy.join.inviteError)

        // Something that is not an invite is refused locally with the same
        // line, and is not kept, so it can never be enrolled. The refused
        // invite stays held while its refusal stands, so the refusal stays
        // tied to it.
        let dead = FirstRunFailure.inviteDead(label: "exhausted")
        let refused = JoinScreenLayout.lookUp("not an invite", in: state, failure: dead, host: Self.host)
        XCTAssertEqual(refused.outcome, .refused)
        XCTAssertEqual(refused.state.invite, "invite:issuer.example")
        XCTAssertEqual(refused.state.issuerHost, "issuer.example")
        XCTAssertEqual(refused.failure, dead)

        // With no refusal pending, a refused paste drops the kept invite.
        let plain = JoinScreenLayout.lookUp("not an invite", in: state, failure: nil, host: Self.host)
        XCTAssertEqual(plain.outcome, .refused)
        XCTAssertEqual(plain.state.invite, "")
        XCTAssertNil(plain.state.issuerHost)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(refused.state, lookup: nil, failure: nil, copy: copy.join, refused: true),
            .error(copy.join.inviteError))

        // A new invite replaces the one the daemon refused, so its error goes.
        let replaced = JoinScreenLayout.lookUp("invite:other.example", in: state, failure: dead, host: Self.host)
        XCTAssertEqual(replaced.outcome, .found)
        XCTAssertNil(replaced.failure)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(replaced.state, lookup: nil, failure: replaced.failure, copy: copy.join),
            .host("other.example"))
    }

    /// The default host function is the core's (`TCInvite.issuerHost`), so a
    /// real invite shows its issuer's host on Join before the daemon runs.
    func test_theRealBridgeReadsTheHost() throws {
        let copy = try coreCopy()
        let looked = JoinScreenLayout.lookUp(
            LinkedChannel.invite(), in: FirstRunState(), failure: nil,
            host: TCInvite.issuerHost)
        XCTAssertEqual(looked.outcome, .found)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(looked.state, lookup: nil, failure: looked.failure, copy: copy.join),
            .host(LinkedChannel.inviteHost))
    }

    /// Review Focus 2, carried through: the daemon refused the invite after
    /// Folders and the person is back on Join with near.ai chosen and no
    /// other invite. Clearing the field and looking up withdraws the invite,
    /// with no error, and the near.ai that waited for it: the daemon's
    /// sign-in needs an enrollment, so signing in with no invite would stop
    /// at Folders. Skip then reaches watch only, joining nothing.
    func test_aRefusedInviteCanBeWithdrawnAndSetupGoesOn() throws {
        let copy = try coreCopy()
        let dead = FirstRunFailure.inviteDead(label: "invite-exhausted")
        let returned = FirstRunNavigation.returnToJoin(
            afterDeadInvite: FirstRunState(
                step: .join, invite: "invite:issuer.example", issuerHost: "issuer.example",
                account: .nearAI, toolAnswers: [.claudeCode: .off, .codex: .off], daemonStarted: true))
        XCTAssertEqual(returned.step, .join)

        // Looking the same refused invite up again keeps the core's error:
        // nothing about it changed.
        let again = JoinScreenLayout.lookUp("invite:issuer.example", in: returned, failure: dead, host: Self.host)
        XCTAssertEqual(again.failure, dead)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(again.state, lookup: nil, failure: again.failure, copy: copy.join),
            .error(copy.join.inviteDead))

        // A refused paste in between does not make the refused invite new:
        // looked up again, it still carries the core's error.
        let garbage = JoinScreenLayout.lookUp("not an invite", in: returned, failure: dead, host: Self.host)
        XCTAssertEqual(garbage.outcome, .refused)
        XCTAssertEqual(garbage.failure, dead)
        let backAgain = JoinScreenLayout.lookUp(
            "invite:issuer.example", in: garbage.state, failure: garbage.failure, host: Self.host)
        XCTAssertEqual(backAgain.outcome, .found)
        XCTAssertEqual(backAgain.failure, dead)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(backAgain.state, lookup: nil, failure: backAgain.failure, copy: copy.join),
            .error(copy.join.inviteDead))

        // And after that refused paste an emptied field can still withdraw
        // the refused invite, taking its error with it.
        XCTAssertTrue(JoinScreenLayout.canLookUp("", in: garbage.state))
        let clearedAfterGarbage = JoinScreenLayout.lookUp("", in: garbage.state, failure: garbage.failure, host: Self.host)
        XCTAssertEqual(clearedAfterGarbage.outcome, .withdrawn)
        XCTAssertNil(clearedAfterGarbage.failure)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(
                clearedAfterGarbage.state, lookup: nil, failure: clearedAfterGarbage.failure, copy: copy.join),
            .hidden)

        // An empty field withdraws it: no error, no line, nothing planned.
        XCTAssertTrue(JoinScreenLayout.canLookUp("  ", in: returned))
        XCTAssertFalse(JoinScreenLayout.canLookUp("  ", in: FirstRunState()))
        let withdrawn = JoinScreenLayout.lookUp("  ", in: returned, failure: dead, host: Self.host)
        XCTAssertEqual(withdrawn.outcome, .withdrawn)
        XCTAssertNil(withdrawn.failure)
        XCTAssertEqual(withdrawn.state.invite, "")
        XCTAssertNil(withdrawn.state.issuerHost)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(
                withdrawn.state, lookup: nil, failure: withdrawn.failure, copy: copy.join,
                refused: withdrawn.outcome == .refused),
            .hidden)

        // near.ai needs no invite, so it stays chosen and goes on without
        // one: no lookup, no invite enrollment, the near.ai enrollment instead.
        XCTAssertEqual(withdrawn.state.account, .nearAI)
        let forward = JoinScreenLayout.readyToStart(withdrawn.state)
        XCTAssertEqual(forward.account, .nearAI)
        let plan = FirstRunPlan.calls(for: forward, at: .signIn)
        XCTAssertFalse(plan.contains { if case .lookupInvite = $0 { return true } else { return false } })
        XCTAssertFalse(plan.contains { if case .enroll = $0 { return true } else { return false } })
        XCTAssertFalse(plan.contains(.signInNearAI))
        XCTAssertTrue(plan.contains(.enrollNearAI))

        // Withdrawing keeps any failure that is not the invite's.
        XCTAssertEqual(
            JoinScreenLayout.lookUp("", in: returned, failure: .signInFailed, host: Self.host).failure, .signInFailed)
    }

    /// Looking an invite up takes back a watch-only answer an earlier build
    /// recorded: the person is asking to join, so the host shows. An invite
    /// alone is not a NEAR AI account, so Continue stays off (owner,
    /// 2026-10-12: sign-in required).
    func test_lookingUpAnInviteTakesBackWatchOnly() throws {
        let copy = try coreCopy()
        let skipped = FirstRunState(step: .join, account: .watchOnly)
        let looked = JoinScreenLayout.lookUp("invite:issuer.example", in: skipped, failure: nil, host: Self.host)
        XCTAssertEqual(looked.state.account, AccountAnswer.none)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(looked.state, lookup: nil, failure: nil, copy: copy.join), .host("issuer.example"))
        XCTAssertEqual(JoinScreenLayout.footerTitle(looked.state, copy: copy), copy.uses.start)
        XCTAssertFalse(JoinScreenLayout.canForward(looked.state))

        // A refused paste changes no answer.
        let refused = JoinScreenLayout.lookUp("not an invite", in: skipped, failure: nil, host: Self.host)
        XCTAssertEqual(refused.state.account, .watchOnly)
    }

    /// A watch-only setup joins no invite (`FirstRunPlan`), so the
    /// looked-up host is not shown as if it would be. Join no longer
    /// answers watch only (owner, 2026-10-12: sign-in required): an invite
    /// alone does not go on, so the answer is the one an earlier build
    /// recorded.
    func test_watchOnlyHidesTheInviteItWillNotJoin() throws {
        let copy = try coreCopy()
        let looked = JoinScreenLayout.lookUp(
            "invite:issuer.example", in: FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off]),
            failure: nil, host: Self.host
        ).state
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(looked, lookup: nil, failure: nil, copy: copy.join), .host("issuer.example"))
        XCTAssertEqual(JoinScreenLayout.readyToStart(looked), looked, "an invite alone is not a NEAR AI account")
        var skipped = looked
        skipped.account = .watchOnly
        let plan = FirstRunPlan.calls(for: skipped, at: .signIn)
        XCTAssertFalse(plan.isEmpty)
        XCTAssertFalse(plan.contains(.enroll("invite:issuer.example")))
        XCTAssertEqual(JoinScreenLayout.inviteLine(skipped, lookup: nil, failure: nil, copy: copy.join), .hidden)

        // An invite the daemon already enrolled stays shown: that is a fact.
        var enrolled = skipped
        enrolled.enrolledInvite = enrolled.invite
        guard case .joined = JoinScreenLayout.inviteLine(enrolled, lookup: nil, failure: nil, copy: copy.join) else {
            return XCTFail("an enrolled invite reads as joined")
        }
    }

    /// Join is the last step and the daemon is running, so Sign in chooses
    /// near.ai and signs in at once (`CommitPoint.signIn`). Start sharing
    /// waits for that sign-in to finish (owner, 2026-10-12: sign-in
    /// required, no Skip); a sign-in cancelled or not yet done can simply be
    /// tried again.
    func test_signInRunsAtOnceAndStartWaitsForIt() throws {
        let copy = try coreCopy()
        var start = FirstRunState(step: .join, toolAnswers: [.claudeCode: .off, .codex: .off], daemonStarted: true)
        start.startedSettingsJSON = start.sessionRoots.settingsJSON()
        start.invite = "invite:issuer.example"
        start.issuerHost = "issuer.example"
        XCTAssertEqual(JoinScreenLayout.nearAILine(start, copy: copy.join), copy.join.nearAiText)
        XCTAssertEqual(JoinScreenLayout.nearAIAction(start, copy: copy), copy.join.nearAiSignIn)
        XCTAssertTrue(JoinScreenLayout.canToggleNearAI(start))
        XCTAssertFalse(JoinScreenLayout.canForward(start))

        let chosen = JoinScreenLayout.chooseNearAI(start)
        XCTAssertEqual(chosen.account, .nearAI)
        XCTAssertEqual(JoinScreenLayout.nearAILine(chosen, copy: copy.join), copy.join.nearAiText,
            "the sign-in runs now, so there is no 'once watching starts'")
        XCTAssertEqual(JoinScreenLayout.nearAIAction(chosen, copy: copy), copy.join.nearAiSignIn)
        XCTAssertEqual(
            FirstRunPlan.calls(for: chosen, at: .signIn),
            [.lookupInvite("invite:issuer.example"), .enroll("invite:issuer.example"), .signInNearAI])
        XCTAssertFalse(JoinScreenLayout.canForward(chosen), "not until the sign-in finished")
        XCTAssertEqual(JoinScreenLayout.chooseNearAI(chosen), chosen, "pressed again, it signs in again")

        var signedIn = chosen
        signedIn.enrolledInvite = "invite:issuer.example"
        signedIn.signedIn = true
        XCTAssertTrue(JoinScreenLayout.canForward(signedIn))
        XCTAssertTrue(JoinScreenLayout.showsSignedIn(signedIn))
        XCTAssertFalse(JoinScreenLayout.canToggleNearAI(signedIn), "a signed-in near.ai is the daemon's fact")
        XCTAssertEqual(JoinScreenLayout.chooseNearAI(signedIn), signedIn)

        // The button signs in; it never only records a choice.
        let source = try Self.source()
        let signIn = try XCTUnwrap(source.range(of: "private func signIn() {"))
        let body = String(source[signIn.upperBound...].prefix(200))
        XCTAssertTrue(body.contains("JoinScreenLayout.chooseNearAI(runner.state)"))
        XCTAssertTrue(body.contains("await runner.commit(.signIn)"))
    }

    /// near.ai signs in to the account an invite enrolls: the daemon's
    /// `account_sign_in` refuses without an enrollment
    /// (`account-enrollment-required`). So near.ai waits for an invite and
    /// says so, and withdrawing the invite takes a chosen near.ai back.
    /// The owner's reversal in Ron's review of #1235: signing in with
    /// near.ai does not need an invite. Without one, leaving Folders or
    /// Tools signs in to near.ai and enrolls this Mac through it
    /// (`near_ai_account_enroll`), never `account_sign_in`, which needs an
    /// enrollment; with one, the invite is joined and signed in to as before.
    func test_nearAIDoesNotNeedAnInvite() throws {
        let copy = try coreCopy()
        let bare = FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off])
        XCTAssertTrue(JoinScreenLayout.canToggleNearAI(bare))
        XCTAssertEqual(JoinScreenLayout.nearAILine(bare, copy: copy.join), copy.join.nearAiText)
        let chosen = JoinScreenLayout.chooseNearAI(bare)
        XCTAssertEqual(chosen.account, .nearAI)
        XCTAssertEqual(JoinScreenLayout.nearAILine(chosen, copy: copy.join), copy.join.nearAiText)
        let alone = FirstRunPlan.calls(for: chosen, at: .signIn)
        XCTAssertEqual(Array(alone.dropFirst()), [.nearAILogin, .enrollNearAI])
        XCTAssertFalse(alone.contains(.signInNearAI))

        // With an invite, the invite route, unchanged.
        let looked = JoinScreenLayout.lookUp("invite:issuer.example", in: chosen, failure: nil, host: Self.host)
        XCTAssertEqual(looked.state.account, .nearAI)
        let joined = FirstRunPlan.calls(for: looked.state, at: .signIn)
        XCTAssertTrue(joined.contains(.signInNearAI))
        XCTAssertFalse(joined.contains(.enrollNearAI))

        // Emptying the field withdraws the invite and keeps near.ai.
        let withdrawn = JoinScreenLayout.lookUp("", in: looked.state, failure: nil, host: Self.host)
        XCTAssertEqual(withdrawn.outcome, .withdrawn)
        XCTAssertEqual(withdrawn.state.account, .nearAI)
        XCTAssertTrue(FirstRunPlan.calls(for: withdrawn.state, at: .signIn).contains(.enrollNearAI))

        // Join no longer carries a line asking for an invite.
        XCTAssertFalse(try Self.source().contains("nearAiNeedsInvite"))
    }

    /// #1030's `ftux-row--between`: the account card's text takes the row
    /// and its action keeps its own width. A text column with layout
    /// priority over an action that is not fixed squeezed Sign in to an
    /// empty capsule and stretched the cards.
    func test_anAccountCardsActionKeepsItsWidth() throws {
        let source = try Self.source()
        let card = try XCTUnwrap(source.range(of: "private func accountCard<Action: View>("))
        let body = String(source[card.lowerBound...].prefix(1800))
        XCTAssertFalse(body.contains(".layoutPriority("), "the text column outranks the action again")
        XCTAssertTrue(body.contains("action()\n                    .fixedSize()"), "the action can be squeezed")
        XCTAssertTrue(body.contains("GlassStatusLabel(done, status: .on)\n                    .fixedSize()"),
                      "the done label can be squeezed")
    }

    /// Owner, 2026-10-12: sign-in required. Join has no Skip: the footer
    /// always reads Start sharing and goes on only with a NEAR AI account
    /// signed in.
    func test_startWaitsForANearAIAccount() throws {
        let copy = try coreCopy()
        for account in [AccountAnswer.none, .watchOnly, .enrolled, .nearAI] {
            let state = FirstRunState(account: account)
            XCTAssertFalse(JoinScreenLayout.canForward(state), "\(account)")
            XCTAssertEqual(JoinScreenLayout.readyToStart(state), state, "\(account)")
            XCTAssertEqual(JoinScreenLayout.footerTitle(state, copy: copy), copy.uses.start)
            XCTAssertNil(JoinScreenLayout.footerNote(state, copy: copy))
        }
        XCTAssertTrue(JoinScreenLayout.hasNearAIAccount(FirstRunState(account: .nearAI)))
        XCTAssertTrue(JoinScreenLayout.canForward(FirstRunState(account: .nearAI, signedIn: true)))

        // Nothing reaches watch only from Join any more.
        let source = try Self.source()
        XCTAssertFalse(source.contains("copy.join.skip"), "Join reads no Skip words")
        XCTAssertTrue(source.contains("isEnabled: JoinScreenLayout.canStart("))

        // near.ai reads "Signed in" only once the daemon signed in.
        XCTAssertFalse(JoinScreenLayout.showsSignedIn(FirstRunState(account: .nearAI)))
        XCTAssertTrue(JoinScreenLayout.showsSignedIn(FirstRunState(account: .nearAI, signedIn: true)))
    }

    /// Owner, 2026-10-12: sign-in required. An invite this run enrolled,
    /// with the near.ai sign-in beside it failed, is not a NEAR AI account:
    /// Start waits for near.ai to sign in again, which signs in to the
    /// account the invite enrolled (the invite is not joined twice). A
    /// near.ai enrollment this run made, with the choice cleared, is one:
    /// Start goes on as it. An earlier run's enrollment has near.ai
    /// chosen beside it, so the person is never left with no way on. One
    /// signed out of waits for near.ai to be chosen.
    func test_startWaitsForNearAIWhileTheDaemonHoldsAnEnrolment() throws {
        let copy = try coreCopy()
        var returned = FirstRunState(
            step: .join, account: .nearAI, toolAnswers: [.claudeCode: .off, .codex: .off],
            enrolledInvite: "invite:issuer.example")
        returned = FirstRunNavigation.returnToJoin(afterNearAIFailure: returned)
        XCTAssertEqual(returned.account, AccountAnswer.none)
        XCTAssertEqual(JoinScreenLayout.footerTitle(returned, copy: copy), copy.uses.start)
        XCTAssertNil(JoinScreenLayout.footerNote(returned, copy: copy))
        XCTAssertFalse(JoinScreenLayout.canForward(returned), "an invite alone is not a NEAR AI account")
        XCTAssertEqual(
            JoinScreenLayout.nearAINotice(returned, failure: .signInFailed, copy: copy), copy.folders.signInFailed,
            "the failure is still said")
        var again = JoinScreenLayout.chooseNearAI(returned)
        XCTAssertFalse(JoinScreenLayout.canForward(again), "until its sign-in finished")
        again.signedIn = true
        XCTAssertTrue(JoinScreenLayout.canForward(again))
        let forwardedAgain = JoinScreenLayout.readyToStart(again)
        XCTAssertEqual(forwardedAgain.account, .nearAI)
        XCTAssertEqual(forwardedAgain.step, .join)
        let plan = FirstRunPlan.calls(for: JoinScreenLayout.chooseNearAI(returned), at: .signIn)
        XCTAssertTrue(plan.contains(.signInNearAI))
        XCTAssertFalse(plan.contains(.enroll("invite:issuer.example")), "the invite is not joined twice")

        var nearAIHeld = FirstRunState(step: .join, account: .none)
        nearAIHeld.nearAIEnrolled = true
        XCTAssertTrue(JoinScreenLayout.canForward(nearAIHeld))
        let forwarded = JoinScreenLayout.readyToStart(nearAIHeld)
        XCTAssertEqual(forwarded.account, .enrolled)
        XCTAssertEqual(forwarded.step, .join)
        XCTAssertTrue(forwarded.holdsEnrolment)

        let earlier = OnboardingNavigation.recordEnrolment(
            FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off]))
        XCTAssertEqual(earlier.account, .enrolled)
        XCTAssertFalse(JoinScreenLayout.canForward(earlier))
        XCTAssertTrue(JoinScreenLayout.canToggleNearAI(earlier))
        XCTAssertEqual(JoinScreenLayout.nearAILine(earlier, copy: copy.join), copy.join.nearAiText)
        let earlierChosen = JoinScreenLayout.chooseNearAI(earlier)
        XCTAssertEqual(earlierChosen.account, .nearAI)
        XCTAssertFalse(JoinScreenLayout.canForward(earlierChosen), "until its sign-in finished")
        XCTAssertEqual(FirstRunPlan.calls(for: earlierChosen, at: .signIn).last, .signInNearAI)

        let signedOut = FirstRunState(account: .none, signedOutOfEnrolment: true)
        XCTAssertEqual(JoinScreenLayout.footerTitle(signedOut, copy: copy), copy.uses.start)
        XCTAssertNil(JoinScreenLayout.footerNote(signedOut, copy: copy))
        XCTAssertFalse(JoinScreenLayout.canForward(signedOut))
        XCTAssertEqual(JoinScreenLayout.readyToStart(signedOut), signedOut, "nothing to go on as")
        var chosen = JoinScreenLayout.chooseNearAI(signedOut)
        chosen.signedIn = true
        XCTAssertTrue(JoinScreenLayout.canForward(chosen))
        XCTAssertEqual(JoinScreenLayout.readyToStart(chosen).account, .nearAI)

        // With nothing held, Start waits too: there is no Skip.
        XCTAssertFalse(JoinScreenLayout.canForward(FirstRunState()))
        XCTAssertEqual(JoinScreenLayout.footerTitle(FirstRunState(), copy: copy), copy.uses.start)
    }

    /// An enrollment recorded with no invite (an earlier run joined through
    /// near.ai, or the daemon was enrolled already: the demo's
    /// pre-seeded config) once read "Joined Unknown · Unknown" on the invite
    /// card. It joined no invite the shell knows of, so the card shows its
    /// invite field, closed, and no joined line. A real joined invite still
    /// reads Joined, with its field gone.
    func test_anEnrolmentWithNoInviteShowsTheFieldAndNoJoinedLine() throws {
        let copy = try coreCopy()
        let resumed = OnboardingNavigation.initialState(startAt: .join, daemonRunning: true, enrolled: true)
        XCTAssertEqual(resumed.enrolledInvite, "")
        XCTAssertFalse(JoinScreenLayout.joinedInvite(resumed))
        XCTAssertEqual(JoinScreenLayout.inviteLine(resumed, lookup: nil, failure: nil, copy: copy.join), .hidden)
        XCTAssertTrue(JoinScreenLayout.showsInviteField(resumed))
        XCTAssertFalse(JoinScreenLayout.inviteIsEditable(resumed), "no second invite over the enrollment")

        var joined = resumed
        joined.enrolledInvite = "invite:issuer.example"
        joined.issuerHost = "issuer.example"
        XCTAssertTrue(JoinScreenLayout.joinedInvite(joined))
        guard case .joined(let line) = JoinScreenLayout.inviteLine(joined, lookup: nil, failure: nil, copy: copy.join)
        else { return XCTFail("a joined invite reads Joined") }
        XCTAssertTrue(line.contains("issuer.example"))
        XCTAssertFalse(JoinScreenLayout.showsInviteField(joined))

        // A fresh Join shows the open field.
        XCTAssertTrue(JoinScreenLayout.showsInviteField(FirstRunState()))
        XCTAssertTrue(JoinScreenLayout.inviteIsEditable(FirstRunState()))

        let source = try Self.source()
        XCTAssertTrue(source.contains("if JoinScreenLayout.showsInviteField(runner.state) {"))
        XCTAssertTrue(source.contains(".disabled(!editable)"))
        let lookUp = try XCTUnwrap(source.range(of: "private func lookUp() {"))
        XCTAssertTrue(
            String(source[lookUp.upperBound...].prefix(120))
                .contains("guard JoinScreenLayout.inviteIsEditable(runner.state) else { return }"))
    }

    /// Owner, 2026-10-08: the body is a paragraph of its own under the
    /// title, and the no-sharing line is the footer's note, directly above
    /// the action bar, not in the cards. The near.ai card leads (owner,
    /// 2026-10-12); a held invite follows it.
    func test_joinLaysOutItsBodyAndCardsAsRuled() throws {
        let source = try Self.source()
        XCTAssertEqual(source.components(separatedBy: "Text(copy.join.body").count - 1, 1)
        let body = try XCTUnwrap(source.range(of: "Text(copy.join.body)\n"))
        let bodyStyle = String(source[body.upperBound...].prefix(200))
        XCTAssertTrue(bodyStyle.contains(".glassType(GlassTokens.TypeScale.body)\n"))
        XCTAssertTrue(bodyStyle.contains(".foregroundStyle(GlassColor.textSecondary)"))

        XCTAssertFalse(source.contains("Text(copy.join.noSharing)"), "drawn in the content")
        XCTAssertEqual(source.components(separatedBy: "copy.join.noSharing").count - 1, 1)
        let frameCall = try XCTUnwrap(source.range(of: "FirstRunFrame("))
        let graphic = try XCTUnwrap(source.range(of: ") {\n            orbit\n", range: frameCall.upperBound..<source.endIndex))
        let note = try XCTUnwrap(source.range(of: "note: copy.join.noSharing,"))
        XCTAssertLessThan(frameCall.lowerBound, note.lowerBound)
        XCTAssertLessThan(note.lowerBound, graphic.lowerBound, "an argument of the footer, not content")

        let flat = source.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        let near = try XCTUnwrap(flat.range(of: "VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) { nearAICard"))
        let invite = try XCTUnwrap(flat.range(of: "if JoinScreenLayout.showsInviteCard(runner.state) { inviteCard"))
        XCTAssertLessThan(near.lowerBound, invite.lowerBound)
    }

    /// Join offers near.ai alone (owner, 2026-10-12): a fresh run shows no
    /// invite card; a held invite brings it back, so what was joined is
    /// never hidden.
    func test_joinShowsTheOtherCardsOnlyForWhatIsHeld() {
        var state = FirstRunState()
        XCTAssertFalse(JoinScreenLayout.showsInviteCard(state))

        state.invite = "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6"
        XCTAssertTrue(JoinScreenLayout.showsInviteCard(state))
        state.invite = ""
        state.enrolledInvite = "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6"
        XCTAssertTrue(JoinScreenLayout.showsInviteCard(state))
    }

    /// Every word on Join is the core's: the file holds no literal of two or
    /// more words, and reads each card's words from `copy.join`.
    func test_joinAuthorsNoSentence() throws {
        let source = try Self.source()
        // Comments name Ron's words to say which copy field is which; only
        // code can put a word on screen.
        let code = source.split(separator: "\n", omittingEmptySubsequences: false)
            .filter { !$0.trimmingCharacters(in: .whitespaces).hasPrefix("//") }
            .joined(separator: "\n")
        let literal = try NSRegularExpression(pattern: #""((?:[^"\\\n]|\\.)*)""#)
        let interpolation = try NSRegularExpression(pattern: #"\\\([^)]*\)"#)
        let range = NSRange(code.startIndex..., in: code)
        for match in literal.matches(in: code, range: range) {
            let raw = String(code[Range(match.range(at: 1), in: code)!])
            let text = interpolation.stringByReplacingMatches(
                in: raw, range: NSRange(raw.startIndex..., in: raw), withTemplate: "")
            let words = text.split(whereSeparator: { $0 == " " }).filter { $0.contains(where: \.isLetter) }
            XCTAssertLessThan(words.count, 2, "authored: \(text)")
        }
        for field in [
            "copy.join.titleLight", "copy.join.titleBold", "copy.join.body",
            "copy.join.inviteEyebrow", "copy.join.invitePlaceholder", "copy.join.lookUp",
            "copy.join.nearAiEyebrow", "copy.join.signedIn", "copy.frame.cancel",
            "copy.join.noSharing",
        ] {
            XCTAssertTrue(source.contains(field), "missing \(field)")
        }
        XCTAssertTrue(source.contains("FirstRunFrame("))
        XCTAssertTrue(source.contains("TCInvite.issuerHost"))
        // The near.ai action only records a choice, so it carries no
        // external-link glyph.
        XCTAssertFalse(source.contains("arrow.up.right.square"))
    }

    /// Kristi's review of #1261: a near.ai sign-in or enrollment that did not
    /// succeed returns the person here with the choice cleared, and Join
    /// says why in the core's line the step said it in before. The line
    /// goes once an account is answered again; no other failure shows it.
    func test_joinSaysWhyNearAIWasCleared() throws {
        let copy = try coreCopy()
        let returned = FirstRunState()
        XCTAssertEqual(
            JoinScreenLayout.nearAINotice(returned, failure: .signInFailed, copy: copy), copy.folders.signInFailed)
        XCTAssertEqual(
            JoinScreenLayout.nearAINotice(
                returned, failure: .nearAIEnrollFailed(label: "near_ai_enroll_commons_unreachable"), copy: copy),
            TCNearAiEnroll.line(label: "near_ai_enroll_commons_unreachable"))
        XCTAssertEqual(
            JoinScreenLayout.nearAINotice(
                returned, failure: .nearAIEnrollFailed(label: "a_label_with_no_line"), copy: copy),
            TCNearAiEnroll.line(label: "a_label_with_no_line") ?? copy.folders.enrollRefused)
        XCTAssertNil(JoinScreenLayout.nearAINotice(returned, failure: .inviteDead(label: "x"), copy: copy))
        XCTAssertNil(JoinScreenLayout.nearAINotice(returned, failure: .startFailed, copy: copy))
        XCTAssertNil(JoinScreenLayout.nearAINotice(returned, failure: nil, copy: copy))
        for account in [AccountAnswer.nearAI, .watchOnly] {
            var chosen = returned
            chosen.account = account
            XCTAssertNil(JoinScreenLayout.nearAINotice(chosen, failure: .signInFailed, copy: copy), "\(account)")
        }
        XCTAssertTrue(try Self.source().contains("JoinScreenLayout.nearAINotice("))
    }
}

/// A daemon that confirms nothing; Join's tests never commit.
@MainActor
private final class NoDaemon: FirstRunDaemon {
    func startDaemon(settingsJSON: String) async -> Bool { false }
    func setSourceSettings(settingsJSON: String) async -> Bool { false }
    func lookupInvite(_ invite: String) async -> FirstRunLookup { .refused(label: "none") }
    func enrollInvite(_ invite: String) async -> Bool { false }
    func signInNearAI() async -> Bool { false }
    func nearAILogin() async -> Bool { false }
    func cancelNearAILogin() async -> Bool { false }
    func enrollNearAI(referralCode: String?) async -> FirstRunNearAIEnrolment { .refused(label: "near_ai_enroll_unavailable") }
    func setAskEachTime() async -> Bool { false }
    func saveConsentScopes(_ scopes: [String]) async -> Bool { false }
    func setProjectMode(projectID: String, mode: ProjectMode) async -> Bool { false }
    func includePastSessions(projectID: String, sessionIDs: [String]) async -> Bool { false }
    func setPrivateAI(_ on: Bool) async -> Bool { false }
    func grantAutomatic(witness: String?) async -> FirstRunGrantAnswer { .refused(label: "none") }
    func markComplete() async -> Bool { false }
    func markWatchOnlyComplete() async -> Bool { false }
    func unenroll() async -> Bool { false }
    func firstRunFinished(notice: String?) {}
}
