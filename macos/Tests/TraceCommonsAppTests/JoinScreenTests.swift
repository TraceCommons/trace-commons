import TCBridge
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
            "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6", in: FirstRunState(), failure: nil,
            host: JoinScreen.defaultIssuerHost)
        XCTAssertEqual(real.outcome, .found)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(real.state, lookup: nil, failure: real.failure, copy: copy.join),
            .host("issuer.tracecommons.ai"))
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
            "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6", in: FirstRunState(), failure: nil,
            host: TCInvite.issuerHost)
        XCTAssertEqual(looked.outcome, .found)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(looked.state, lookup: nil, failure: looked.failure, copy: copy.join),
            .host("issuer.tracecommons.ai"))
    }

    /// Review Focus 2, carried through: the daemon refused the invite after
    /// Folders and the person is back on Join with near.ai chosen and no
    /// other invite. Clearing the field and looking up withdraws the invite,
    /// with no error, and the near.ai that waited for it: the daemon's
    /// sign-in needs an enrolment, so signing in with no invite would stop
    /// at Folders. Skip then reaches watch only, joining nothing.
    func test_aRefusedInviteCanBeWithdrawnAndSetupGoesOn() throws {
        let copy = try coreCopy()
        let dead = FirstRunFailure.inviteDead(label: "invite-exhausted")
        let returned = FirstRunNavigation.returnToJoin(
            afterDeadInvite: FirstRunState(
                tier: .quick, step: .folders, invite: "invite:issuer.example", issuerHost: "issuer.example",
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
        // one: no lookup, no invite enrolment, the near.ai enrolment instead.
        XCTAssertEqual(withdrawn.state.account, .nearAI)
        let forward = JoinScreenLayout.forward(withdrawn.state)
        XCTAssertEqual(forward.account, .nearAI)
        let plan = FirstRunPlan.calls(for: forward, at: .leaveRoots)
        XCTAssertFalse(plan.contains { if case .lookupInvite = $0 { return true } else { return false } })
        XCTAssertFalse(plan.contains { if case .enroll = $0 { return true } else { return false } })
        XCTAssertFalse(plan.contains(.signInNearAI))
        XCTAssertTrue(plan.contains(.enrollNearAI))

        // Withdrawing keeps any failure that is not the invite's.
        XCTAssertEqual(
            JoinScreenLayout.lookUp("", in: returned, failure: .signInFailed, host: Self.host).failure, .signInFailed)
    }

    /// Looking an invite up after Skip takes back watch only: the person is
    /// asking to join, so the host shows and the footer still reads Skip.
    func test_lookingUpAnInviteTakesBackWatchOnly() throws {
        let copy = try coreCopy()
        let skipped = FirstRunState(step: .join, account: .watchOnly)
        let looked = JoinScreenLayout.lookUp("invite:issuer.example", in: skipped, failure: nil, host: Self.host)
        XCTAssertEqual(looked.state.account, AccountAnswer.none)
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(looked.state, lookup: nil, failure: nil, copy: copy.join), .host("issuer.example"))
        XCTAssertEqual(JoinScreenLayout.footerTitle(looked.state, copy: copy), copy.join.skip)

        // A refused paste changes no answer.
        let refused = JoinScreenLayout.lookUp("not an invite", in: skipped, failure: nil, host: Self.host)
        XCTAssertEqual(refused.state.account, .watchOnly)
    }

    /// Skip answers watch only, and a watch-only setup joins no invite
    /// (`FirstRunPlan`), so the looked-up host is not shown as if it would be.
    func test_watchOnlyHidesTheInviteItWillNotJoin() throws {
        let copy = try coreCopy()
        let looked = JoinScreenLayout.lookUp(
            "invite:issuer.example", in: FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off]),
            failure: nil, host: Self.host
        ).state
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(looked, lookup: nil, failure: nil, copy: copy.join), .host("issuer.example"))
        let skipped = JoinScreenLayout.forward(looked)
        XCTAssertEqual(skipped.account, .watchOnly)
        let plan = FirstRunPlan.calls(for: skipped, at: .leaveRoots)
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

    /// near.ai is chosen on Join and signed in after the daemon starts, so a
    /// choice not yet signed in can be undone, and Skip then reaches watch
    /// only with no sign-in planned.
    func test_aChosenNearAICanBeUndoneUntilItIsSignedIn() throws {
        let copy = try coreCopy()
        var start = FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off])
        start.invite = "invite:issuer.example"
        start.issuerHost = "issuer.example"
        XCTAssertEqual(JoinScreenLayout.nearAILine(start, copy: copy.join), copy.join.nearAiText)
        XCTAssertEqual(JoinScreenLayout.nearAIAction(start, copy: copy), copy.join.nearAiSignIn)
        XCTAssertTrue(JoinScreenLayout.canToggleNearAI(start))

        let chosen = JoinScreenLayout.toggleNearAI(start)
        XCTAssertEqual(chosen.account, .nearAI)
        XCTAssertEqual(JoinScreenLayout.nearAILine(chosen, copy: copy.join), copy.join.nearAiChosen)
        XCTAssertEqual(JoinScreenLayout.nearAIAction(chosen, copy: copy), copy.frame.undo)
        XCTAssertTrue(JoinScreenLayout.canToggleNearAI(chosen))
        XCTAssertEqual(FirstRunPlan.calls(for: chosen, at: .leaveRoots).last, .signInNearAI)

        let undone = JoinScreenLayout.toggleNearAI(chosen)
        XCTAssertEqual(undone.account, AccountAnswer.none)
        XCTAssertEqual(JoinScreenLayout.footerTitle(undone, copy: copy), copy.join.skip)
        let skipped = JoinScreenLayout.forward(undone)
        XCTAssertEqual(skipped.account, .watchOnly)
        let plan = FirstRunPlan.calls(for: skipped, at: .leaveRoots)
        XCTAssertTrue(plan.contains { if case .startDaemon = $0 { return true } else { return false } })
        XCTAssertFalse(plan.contains(.signInNearAI))

        // Back on Join after a failed sign-in, the choice still undoes.
        var failed = JoinScreenLayout.forward(chosen)
        failed.daemonStarted = true
        failed.step = .join
        XCTAssertEqual(JoinScreenLayout.toggleNearAI(failed).account, AccountAnswer.none)

        // A signed-in near.ai is the daemon's fact: the card shows it and the
        // action does nothing.
        let signedIn = FirstRunState(account: .nearAI, daemonStarted: true, signedIn: true)
        XCTAssertFalse(JoinScreenLayout.canToggleNearAI(signedIn))
        XCTAssertEqual(JoinScreenLayout.toggleNearAI(signedIn), signedIn)
    }

    /// near.ai signs in to the account an invite enrolls: the daemon's
    /// `account_sign_in` refuses without an enrolment
    /// (`account-enrollment-required`). So near.ai waits for an invite and
    /// says so, and withdrawing the invite takes a chosen near.ai back.
    /// The owner's reversal in Ron's review of #1235: signing in with
    /// near.ai does not need an invite. Without one, leaving Folders or
    /// Tools signs in to near.ai and enrolls this Mac through it
    /// (`near_ai_account_enroll`), never `account_sign_in`, which needs an
    /// enrolment; with one, the invite is joined and signed in to as before.
    func test_nearAIDoesNotNeedAnInvite() throws {
        let copy = try coreCopy()
        let bare = FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off])
        XCTAssertTrue(JoinScreenLayout.canToggleNearAI(bare))
        XCTAssertEqual(JoinScreenLayout.nearAILine(bare, copy: copy.join), copy.join.nearAiText)
        let chosen = JoinScreenLayout.toggleNearAI(bare)
        XCTAssertEqual(chosen.account, .nearAI)
        XCTAssertEqual(JoinScreenLayout.nearAILine(chosen, copy: copy.join), copy.join.nearAiChosen)
        let alone = FirstRunPlan.calls(for: chosen, at: .leaveRoots)
        XCTAssertEqual(Array(alone.dropFirst()), [.nearAILogin, .enrollNearAI])
        XCTAssertFalse(alone.contains(.signInNearAI))

        // With an invite, the invite route, unchanged.
        let looked = JoinScreenLayout.lookUp("invite:issuer.example", in: chosen, failure: nil, host: Self.host)
        XCTAssertEqual(looked.state.account, .nearAI)
        let joined = FirstRunPlan.calls(for: looked.state, at: .leaveRoots)
        XCTAssertTrue(joined.contains(.signInNearAI))
        XCTAssertFalse(joined.contains(.enrollNearAI))

        // Emptying the field withdraws the invite and keeps near.ai.
        let withdrawn = JoinScreenLayout.lookUp("", in: looked.state, failure: nil, host: Self.host)
        XCTAssertEqual(withdrawn.outcome, .withdrawn)
        XCTAssertEqual(withdrawn.state.account, .nearAI)
        XCTAssertTrue(FirstRunPlan.calls(for: withdrawn.state, at: .leaveRoots).contains(.enrollNearAI))

        // Join no longer carries a line asking for an invite.
        XCTAssertFalse(try Self.source().contains("nearAiNeedsInvite"))
    }

    /// A new passkey creates an account of its own, and the daemon refuses
    /// to create one over an enrolment (`account-already-enrolled`). So a
    /// held invite holds back Create passkey and says why, an invite looked
    /// up replaces a passkey only chosen, and a held passkey keeps the
    /// invite field closed.
    func test_anInviteAndANewPasskeyAreNotCombined() throws {
        let copy = try coreCopy()
        var withInvite = FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off])
        withInvite.invite = "invite:issuer.example"
        withInvite.issuerHost = "issuer.example"
        XCTAssertFalse(JoinScreenLayout.showsPasskeyAction(withInvite))
        XCTAssertEqual(JoinScreenLayout.togglePasskey(withInvite), withInvite)
        XCTAssertEqual(JoinScreenLayout.passkeyLine(withInvite, copy: copy.join), copy.join.inviteOrPasskey)
        withInvite.daemonStarted = true
        XCTAssertFalse(JoinScreenLayout.passkeyOpensNow(withInvite, hasPasskeyAccount: true))

        // Chosen first, then an invite: the invite replaces the choice.
        let chosen = JoinScreenLayout.togglePasskey(FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off]))
        XCTAssertEqual(chosen.account, .passkeyChosen)
        let looked = JoinScreenLayout.lookUp("invite:issuer.example", in: chosen, failure: nil, host: Self.host)
        XCTAssertEqual(looked.outcome, .found)
        XCTAssertEqual(looked.state.account, AccountAnswer.none)
        XCTAssertFalse(FirstRunPlan.calls(for: looked.state, at: .leaveRoots).contains(.openPasskeySheets))

        // A held passkey: the invite field is closed and says why.
        let held = FirstRunState(account: .passkey(name: "Mac"), daemonStarted: true)
        XCTAssertFalse(JoinScreenLayout.inviteIsEditable(held))
        XCTAssertEqual(
            JoinScreenLayout.inviteLine(held, lookup: nil, failure: nil, copy: copy.join), .note(copy.join.inviteOrPasskey))
        XCTAssertEqual(
            OnboardingNavigation.receive(
                invite: "invite:issuer.example", in: held, failure: nil, isCommitting: false, host: Self.host),
            .discard)
    }

    /// A passkey bound in the daemon is an account; near.ai cannot be chosen
    /// over it, which would plan a near.ai sign-in on top of that session.
    func test_aPasskeyAccountKeepsNearAIFromReplacingIt() throws {
        for name in ["Mac", ""] {
            let held = FirstRunState(
                account: .passkey(name: name), toolAnswers: [.claudeCode: .off, .codex: .off], daemonStarted: true)
            XCTAssertFalse(JoinScreenLayout.canToggleNearAI(held))
            let after = JoinScreenLayout.toggleNearAI(held)
            XCTAssertEqual(after, held)
            XCTAssertTrue(JoinScreenLayout.passkeyDone(after))
            XCTAssertFalse(FirstRunPlan.calls(for: after, at: .leaveRoots).contains(.signInNearAI))
        }
    }

    /// One held account leaves the other card with neither its inviting line
    /// nor its action, which could never be taken over that account.
    func test_aHeldAccountQuietsTheOtherCard() throws {
        let copy = try coreCopy()
        let start = FirstRunState(daemonStarted: true)
        XCTAssertTrue(JoinScreenLayout.showsPasskeyAction(start))
        XCTAssertTrue(JoinScreenLayout.showsNearAIAction(start))
        XCTAssertEqual(JoinScreenLayout.passkeyLine(start, copy: copy.join), copy.join.passkeyText)
        XCTAssertEqual(JoinScreenLayout.nearAILine(start, copy: copy.join), copy.join.nearAiText)

        // A near.ai chosen beside its invite: no passkey is created beside
        // the invite, and the passkey card says why.
        var withInvite = start
        withInvite.invite = "invite:issuer.example"
        let chosen = JoinScreenLayout.toggleNearAI(withInvite)
        XCTAssertEqual(chosen.account, .nearAI)
        XCTAssertFalse(JoinScreenLayout.showsPasskeyAction(chosen))
        XCTAssertEqual(JoinScreenLayout.passkeyLine(chosen, copy: copy.join), copy.join.inviteOrPasskey)

        let signedIn = FirstRunState(account: .nearAI, daemonStarted: true, signedIn: true)
        XCTAssertFalse(JoinScreenLayout.showsPasskeyAction(signedIn))
        XCTAssertNil(JoinScreenLayout.passkeyLine(signedIn, copy: copy.join))

        for name in ["Mac", ""] {
            let held = FirstRunState(account: .passkey(name: name), daemonStarted: true)
            XCTAssertFalse(JoinScreenLayout.showsNearAIAction(held))
            XCTAssertNil(JoinScreenLayout.nearAILine(held, copy: copy.join))
        }

        let source = try Self.source()
        XCTAssertTrue(source.contains("JoinScreenLayout.showsPasskeyAction(runner.state)"))
        XCTAssertTrue(source.contains("JoinScreenLayout.showsNearAIAction(runner.state)"))
    }

    /// Create passkey before the daemon runs records the choice, says when
    /// it happens, and can be undone until then; once the daemon started it
    /// opens the sheets straight away. The button is never disabled
    /// without a reason.
    func test_choosingAPasskeyWaitsForTheDaemonAndCanBeUndone() throws {
        let copy = try coreCopy()
        let start = FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off])
        XCTAssertFalse(JoinScreenLayout.passkeyOpensNow(start, hasPasskeyAccount: true))
        XCTAssertEqual(JoinScreenLayout.passkeyAction(start, copy: copy), copy.join.passkeyCreate)

        let chosen = JoinScreenLayout.togglePasskey(start)
        XCTAssertEqual(chosen.account, .passkeyChosen)
        XCTAssertEqual(JoinScreenLayout.passkeyLine(chosen, copy: copy.join), copy.join.passkeyChosen)
        XCTAssertEqual(JoinScreenLayout.passkeyAction(chosen, copy: copy), copy.frame.undo)
        XCTAssertFalse(JoinScreenLayout.passkeyDone(chosen), "chosen is not created")
        XCTAssertTrue(JoinScreenLayout.hasAccount(chosen))
        XCTAssertEqual(JoinScreenLayout.footerTitle(chosen, copy: copy), copy.frame.continueButton)
        XCTAssertEqual(FirstRunPlan.calls(for: JoinScreenLayout.forward(chosen), at: .leaveRoots).last, .openPasskeySheets)

        let undone = JoinScreenLayout.togglePasskey(chosen)
        XCTAssertEqual(undone.account, AccountAnswer.none)
        XCTAssertEqual(JoinScreenLayout.footerTitle(undone, copy: copy), copy.join.skip)

        // near.ai chosen replaces a passkey only chosen, as a passkey chosen
        // replaces a near.ai only chosen.
        XCTAssertEqual(JoinScreenLayout.toggleNearAI(chosen).account, .nearAI)
        XCTAssertEqual(JoinScreenLayout.togglePasskey(JoinScreenLayout.toggleNearAI(chosen)).account, .passkeyChosen)

        // Watch only, then Create passkey: the passkey is the answer.
        XCTAssertEqual(JoinScreenLayout.togglePasskey(FirstRunState(account: .watchOnly)).account, .passkeyChosen)

        // The daemon started: the sheets open now, given the account path;
        // without it the choice is recorded for the next commit.
        let started = FirstRunState(daemonStarted: true)
        XCTAssertTrue(JoinScreenLayout.passkeyOpensNow(started, hasPasskeyAccount: true))
        XCTAssertFalse(JoinScreenLayout.passkeyOpensNow(started, hasPasskeyAccount: false))
        XCTAssertFalse(JoinScreenLayout.passkeyOpensNow(JoinScreenLayout.togglePasskey(started), hasPasskeyAccount: true),
            "a chosen passkey undoes")

        // A signed-in near.ai holds the account: no passkey over it.
        let signedIn = FirstRunState(account: .nearAI, daemonStarted: true, signedIn: true)
        XCTAssertFalse(JoinScreenLayout.passkeyOpensNow(signedIn, hasPasskeyAccount: true))
        XCTAssertEqual(JoinScreenLayout.togglePasskey(signedIn), signedIn)
        // A held passkey is not chosen again.
        let held = FirstRunState(account: .passkey(name: "Mac"), daemonStarted: true)
        XCTAssertEqual(JoinScreenLayout.togglePasskey(held), held)

        let source = try Self.source()
        XCTAssertFalse(source.contains("passkeyAvailable"), "the disabled-with-no-reason button is gone")
        XCTAssertTrue(source.contains("JoinScreenLayout.passkeyOpensNow("))
        XCTAssertTrue(source.contains("runner.requestPasskey()"))
        XCTAssertFalse(source.contains("firstRunPasskeySheets("), "the first-run host mounts the sheets")
    }

    /// The sheets' outcome lowers the request whatever it was, so a closed
    /// sheet is asked again only by the next commit or the button. The
    /// sheets open after Folders or Tools, so a sign-out can end them on a
    /// later step: it leaves no account, and Join says why.
    func test_finishingThePasskeySheetsLowersTheRequest() throws {
        let copy = try coreCopy()
        var state = FirstRunState(
            step: .uses, account: .passkeyChosen, toolAnswers: [.claudeCode: .off, .codex: .off])
        state.daemonStarted = true
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        let runner = FirstRunRunner(state: state, daemon: NoDaemon())
        runner.requestPasskey()
        XCTAssertTrue(runner.passkeyDue)

        runner.finishPasskey(.closed, copy: copy)
        XCTAssertFalse(runner.passkeyDue)
        XCTAssertEqual(runner.state.account, .passkeyChosen, "closed keeps the choice")
        XCTAssertEqual(runner.state.step, .uses)
        XCTAssertNil(runner.passkeyOutcome?.joinNotice(copy))

        // Verify cancelled: signed out, no account, back on Join with the
        // core's notice and every answer kept.
        runner.requestPasskey()
        runner.finishPasskey(.signedOut, copy: copy)
        XCTAssertFalse(runner.passkeyDue)
        XCTAssertEqual(runner.state.account, AccountAnswer.none)
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertEqual(runner.state.toolAnswers, state.toolAnswers)
        XCTAssertEqual(runner.passkeyOutcome?.joinNotice(copy), copy.join.signedOut)

        runner.state.account = .passkeyChosen
        runner.requestPasskey()
        XCTAssertNil(runner.passkeyOutcome, "asking again clears the notice")
        runner.finishPasskey(.created(name: "Mac"), copy: copy)
        XCTAssertFalse(runner.passkeyDue)
        XCTAssertEqual(runner.state.account, .passkey(name: "Mac"))
        XCTAssertFalse(FirstRunPlan.calls(for: runner.state, at: .leaveRoots).contains(.openPasskeySheets))

        // A sign-in carries the signed-in account's remembered name to Join.
        runner.finishPasskey(.signedIn(name: "Home"), copy: copy)
        XCTAssertEqual(runner.state.account, .passkey(name: "Home"))

        let source = try Self.source()
        XCTAssertTrue(source.contains("runner.passkeyOutcome?.joinNotice(copy)"))
    }

    func test_skipReadsWatchOnlyUntilAnAccountExists() throws {
        let copy = try coreCopy()
        for account in [AccountAnswer.none, .watchOnly] {
            let state = FirstRunState(account: account)
            XCTAssertFalse(JoinScreenLayout.hasAccount(state))
            XCTAssertEqual(JoinScreenLayout.footerTitle(state, copy: copy), copy.join.skip)
            XCTAssertEqual(JoinScreenLayout.footerNote(state, copy: copy), copy.join.skipNote)
        }
        for account in [AccountAnswer.nearAI, .passkeyChosen, .passkey(name: "Mac"), .passkey(name: "")] {
            let state = FirstRunState(account: account)
            XCTAssertTrue(JoinScreenLayout.hasAccount(state))
            XCTAssertEqual(JoinScreenLayout.footerTitle(state, copy: copy), copy.frame.continueButton)
            XCTAssertNil(JoinScreenLayout.footerNote(state, copy: copy))
        }

        // Skip answers "watch only" and moves on; Continue keeps the account.
        let skipped = JoinScreenLayout.forward(FirstRunState())
        XCTAssertEqual(skipped.account, .watchOnly)
        XCTAssertEqual(skipped.step, .folders)
        let continued = JoinScreenLayout.forward(
            JoinScreenLayout.toggleNearAI(FirstRunState(tier: .custom, invite: "invite:issuer.example")))
        XCTAssertEqual(continued.account, .nearAI)
        XCTAssertEqual(continued.step, .tools)

        // near.ai reads "Signed in" only once the daemon signed in.
        XCTAssertFalse(JoinScreenLayout.showsSignedIn(FirstRunState(account: .nearAI)))
        XCTAssertTrue(JoinScreenLayout.showsSignedIn(FirstRunState(account: .nearAI, signedIn: true)))
    }

    /// While the daemon holds an enrolment, Join does not offer "Skip: watch
    /// only": watching only would act under the enrolment and could never
    /// finish. An invite this run enrolled, with no account answered again
    /// (a near.ai sign-in that failed), is the account, as an earlier run's
    /// enrolment is (`recordEnrolment`): Continue goes on as it. One signed
    /// out of waits for an account to be chosen. The footer's words are the
    /// core's either way.
    func test_skipIsNotOfferedWhileTheDaemonHoldsAnEnrolment() throws {
        let copy = try coreCopy()
        var returned = FirstRunState(step: .folders, account: .nearAI, enrolledInvite: "invite:issuer.example")
        returned = FirstRunNavigation.returnToJoin(afterNearAIFailure: returned)
        XCTAssertEqual(returned.account, AccountAnswer.none)
        XCTAssertEqual(JoinScreenLayout.footerTitle(returned, copy: copy), copy.frame.continueButton)
        XCTAssertNil(JoinScreenLayout.footerNote(returned, copy: copy))
        XCTAssertTrue(JoinScreenLayout.canForward(returned))
        XCTAssertEqual(
            JoinScreenLayout.nearAINotice(returned, failure: .signInFailed, copy: copy), copy.folders.signInFailed,
            "the failure is still said")
        let forwarded = JoinScreenLayout.forward(returned)
        XCTAssertEqual(forwarded.account, .enrolled)
        XCTAssertEqual(forwarded.step, .folders)
        XCTAssertTrue(forwarded.holdsEnrolment)

        let signedOut = FirstRunState(account: .none, signedOutOfEnrolment: true)
        XCTAssertEqual(JoinScreenLayout.footerTitle(signedOut, copy: copy), copy.frame.continueButton)
        XCTAssertNil(JoinScreenLayout.footerNote(signedOut, copy: copy))
        XCTAssertFalse(JoinScreenLayout.canForward(signedOut))
        XCTAssertEqual(JoinScreenLayout.forward(signedOut), signedOut, "nothing to go on as")
        let chosen = JoinScreenLayout.toggleNearAI(signedOut)
        XCTAssertTrue(JoinScreenLayout.canForward(chosen))
        XCTAssertEqual(JoinScreenLayout.forward(chosen).account, .nearAI)

        // With nothing held, Skip is offered as before.
        XCTAssertTrue(JoinScreenLayout.canForward(FirstRunState()))
        XCTAssertEqual(JoinScreenLayout.footerTitle(FirstRunState(), copy: copy), copy.join.skip)

        let source = try Self.source()
        XCTAssertTrue(source.contains("isEnabled: JoinScreenLayout.canForward(runner.state)"))
    }

    /// The passkey sheets' outcomes as Join records them. A passkey known
    /// without a name never shows the ready line with a blank in it.
    func test_passkeyOutcomesBecomeTheAccount() throws {
        let copy = try coreCopy()
        let start = FirstRunState()
        XCTAssertEqual(JoinScreenLayout.passkeyLine(start, copy: copy.join), copy.join.passkeyText)
        XCTAssertFalse(JoinScreenLayout.passkeyDone(start))

        let created = JoinScreenLayout.apply(.created(name: "Mac"), to: start, copy: copy)
        XCTAssertEqual(created.state.account, .passkey(name: "Mac"))
        XCTAssertNil(created.notice)
        XCTAssertTrue(JoinScreenLayout.passkeyDone(created.state))
        XCTAssertEqual(
            JoinScreenLayout.passkeyLine(created.state, copy: copy.join),
            copy.join.passkeyReady.replacingOccurrences(of: "{name}", with: "Mac"))

        // A sign-in is named by the signed-in account's remembered record.
        let signedIn = JoinScreenLayout.apply(.signedIn(name: "Home"), to: start, copy: copy)
        XCTAssertEqual(signedIn.state.account, .passkey(name: "Home"))
        XCTAssertEqual(
            JoinScreenLayout.passkeyLine(signedIn.state, copy: copy.join),
            copy.join.passkeyReady.replacingOccurrences(of: "{name}", with: "Home"))

        for nameless in [PasskeySheetOutcome.signedIn(name: nil), .existingAccount] {
            let applied = JoinScreenLayout.apply(nameless, to: start, copy: copy)
            XCTAssertTrue(JoinScreenLayout.hasAccount(applied.state))
            XCTAssertTrue(JoinScreenLayout.passkeyDone(applied.state))
            XCTAssertNil(JoinScreenLayout.passkeyLine(applied.state, copy: copy.join))
        }

        XCTAssertEqual(JoinScreenLayout.apply(.closed, to: start, copy: copy).state, start)

        let signedOut = JoinScreenLayout.apply(.signedOut, to: created.state, copy: copy)
        XCTAssertEqual(signedOut.state.account, .none)
        XCTAssertEqual(signedOut.notice, copy.join.signedOut)

        // #1030 rule 6: signing out clears every sign-in, near.ai included,
        // so nothing is signed in again behind the person's back.
        let nearAI = FirstRunState(
            account: .nearAI, toolAnswers: [.claudeCode: .off, .codex: .off], daemonStarted: true, signedIn: true)
        let afterNearAI = JoinScreenLayout.apply(.signedOut, to: nearAI, copy: copy)
        XCTAssertEqual(afterNearAI.state.account, AccountAnswer.none)
        XCTAssertFalse(afterNearAI.state.signedIn)
        XCTAssertFalse(FirstRunPlan.calls(for: afterNearAI.state, at: .leaveRoots).contains(.signInNearAI))
        XCTAssertFalse(FirstRunPlan.calls(for: afterNearAI.state, at: .leaveRoots).contains(.enrollNearAI))
    }

    /// #1030 rule 6 (owner ruling, 2026-10-06): signing out clears the
    /// invite and the enrolment it wrote, not only the cards. The daemon has
    /// no call that drops an enrolment, so the state stops treating it as an
    /// account: no joined line, no scopes, no grant, no enrolment marker, and
    /// the daemon's report of it is not recorded again.
    func test_signingOutClearsTheInviteAndItsEnrolment() throws {
        let copy = try coreCopy()
        var state = FirstRunState(
            step: .uses, invite: "invite:issuer.example", issuerHost: "issuer.example", account: .nearAI,
            toolAnswers: [.claudeCode: .off, .codex: .off], scopes: ["research"], sharing: .automatic,
            grantReady: true, daemonStarted: true, enrolledInvite: "invite:issuer.example", signedIn: true)
        state.startedSettingsJSON = state.sessionRoots.settingsJSON()
        XCTAssertTrue(state.holdsEnrolment)

        let out = JoinScreenLayout.apply(.signedOut, to: state, copy: copy).state
        XCTAssertEqual(out.account, AccountAnswer.none)
        XCTAssertEqual(out.invite, "")
        XCTAssertNil(out.issuerHost)
        XCTAssertNil(out.enrolledInvite)
        XCTAssertFalse(out.signedIn)
        XCTAssertTrue(out.signedOutOfEnrolment)
        XCTAssertFalse(out.holdsEnrolment)
        XCTAssertFalse(JoinScreenLayout.hasAccount(out))
        XCTAssertEqual(JoinScreenLayout.inviteLine(out, lookup: nil, failure: nil, copy: copy.join), .hidden)
        XCTAssertTrue(JoinScreenLayout.inviteIsEditable(out))
        XCTAssertFalse(FirstRunNavigation.canChooseAutomatic(out))
        // The answers that are not sign-ins are kept.
        XCTAssertEqual(out.scopes, state.scopes)
        XCTAssertEqual(out.toolAnswers, state.toolAnswers)

        // Fail closed: nothing that belongs to an enrolment is sent for it.
        // The daemon still holds it, so watch only is not offered: a
        // watch-only Start would act under it and could never finish (the
        // marker is refused while the daemon is logged in).
        XCTAssertTrue(out.daemonHoldsEnrolment)
        XCTAssertNotEqual(JoinScreenLayout.footerTitle(out, copy: copy), copy.join.skip)
        XCTAssertFalse(JoinScreenLayout.canForward(out))
        var watching = out
        watching.account = .watchOnly
        XCTAssertEqual(FirstRunPlan.calls(for: watching, at: .start), [])

        // The daemon still reports the enrolment; it is not the account again.
        XCTAssertEqual(OnboardingNavigation.recordEnrolment(out), out)

        // An earlier first run's enrolment is signed out of the same way.
        let earlier = JoinScreenLayout.apply(
            .signedOut, to: OnboardingNavigation.recordEnrolment(FirstRunState()), copy: copy
        ).state
        XCTAssertTrue(earlier.signedOutOfEnrolment)
        XCTAssertFalse(earlier.holdsEnrolment)
        XCTAssertNil(earlier.enrolledInvite)

        // near.ai's invite-free enrolment is signed out of the same way.
        var viaNearAI = FirstRunState(account: .nearAI, daemonStarted: true, signedIn: true)
        viaNearAI.nearAIEnrolled = true
        XCTAssertTrue(viaNearAI.holdsEnrolment)
        let nearAIOut = JoinScreenLayout.apply(.signedOut, to: viaNearAI, copy: copy).state
        XCTAssertFalse(nearAIOut.nearAIEnrolled)
        XCTAssertTrue(nearAIOut.signedOutOfEnrolment)
        XCTAssertFalse(nearAIOut.holdsEnrolment)

        // A sign-out with nothing enrolled marks nothing.
        let chosen = JoinScreenLayout.apply(.signedOut, to: FirstRunState(account: .passkeyChosen), copy: copy).state
        XCTAssertFalse(chosen.signedOutOfEnrolment)

        // A later bound passkey is an enrolment again.
        let bound = JoinScreenLayout.apply(.created(name: "Mac"), to: out, copy: copy).state
        XCTAssertFalse(bound.signedOutOfEnrolment)
        XCTAssertTrue(bound.holdsEnrolment)
    }

    /// The runner's side of the sign-out: the invite's lookup goes with the
    /// invite, and a new invite the daemon enrolls clears the mark.
    func test_theRunnerDropsTheLookupOnSignOut() async throws {
        let copy = try coreCopy()
        let daemon = RecordingFirstRunDaemon()
        var state = FirstRunState(
            tier: .quick, step: .folders, invite: "invite:issuer.example", issuerHost: "issuer.example",
            account: .nearAI, toolAnswers: [.claudeCode: .off, .codex: .off])
        state.account = .nearAI
        let runner = FirstRunRunner(state: state, daemon: daemon)
        await runner.commit(.leaveRoots)
        XCTAssertNotNil(runner.lookup)
        XCTAssertNotNil(runner.state.enrolledInvite)

        runner.finishPasskey(.signedOut, copy: copy)
        XCTAssertNil(runner.lookup)
        XCTAssertEqual(runner.state.step, .join)
        XCTAssertTrue(runner.state.signedOutOfEnrolment)

        runner.state.invite = "invite:other.example"
        runner.state.issuerHost = "other.example"
        runner.state.account = .nearAI
        runner.state.step = .folders
        await runner.commit(.leaveRoots)
        XCTAssertEqual(runner.state.enrolledInvite, "invite:other.example")
        XCTAssertFalse(runner.state.signedOutOfEnrolment)
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
            "copy.join.titleLight", "copy.join.titleBold", "copy.join.body", "copy.join.bodyEmphasis",
            "copy.join.inviteEyebrow", "copy.join.invitePlaceholder", "copy.join.lookUp",
            "copy.join.passkeyEyebrow", "copy.join.passkeyCreate", "copy.join.passkeyDone",
            "copy.passkeyChosen", "copy.frame.undo",
            "copy.join.nearAiEyebrow", "copy.join.signedIn",
            "copy.join.noSharing",
        ] {
            XCTAssertTrue(source.contains(field), "missing \(field)")
        }
        XCTAssertTrue(source.contains("FirstRunFrame("))
        // The sheets are the first-run host's (`OnboardingCoordinatorView`).
        XCTAssertFalse(source.contains("PasskeySheets("))
        XCTAssertTrue(source.contains("TCInvite.issuerHost"))
        // The near.ai action only records a choice, so it carries no
        // external-link glyph.
        XCTAssertFalse(source.contains("arrow.up.right.square"))
    }

    /// Kristi's review of #1261: a near.ai sign-in or enrolment that did not
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
        for account in [AccountAnswer.nearAI, .watchOnly, .passkeyChosen] {
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
    func enrollNearAI() async -> FirstRunNearAIEnrolment { .refused(label: "near_ai_enroll_unavailable") }
    func saveConsentScopes(_ scopes: [String]) async -> Bool { false }
    func setProjectMode(projectID: String, mode: ProjectMode) async -> Bool { false }
    func includePastSessions(projectID: String, sessionIDs: [String]) async -> Bool { false }
    func setPrivateAI(_ on: Bool) async -> Bool { false }
    func grantAutomatic(witness: String?) async -> FirstRunGrantAnswer { .refused(label: "none") }
    func markComplete() async -> Bool { false }
    func markWatchOnlyComplete() async -> Bool { false }
    func firstRunFinished(notice: String?) {}
}
