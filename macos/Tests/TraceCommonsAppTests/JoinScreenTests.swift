import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's Join (#1030 `join-screen.tsx`): the invite with its host before the
/// daemon runs and the joined line after it, the account cards, and "Skip:
/// watch only" until an account exists. The decisions are `JoinLayout`'s, so
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
        XCTAssertEqual(JoinLayout.inviteLine(state, lookup: nil, failure: nil, copy: copy.join), .hidden)

        // Look up reads the host locally; nothing reaches the daemon, which
        // does not run yet: Join holds no daemon to reach.
        let source = try Self.source()
        XCTAssertFalse(source.contains("DaemonClient"))
        XCTAssertFalse(source.contains("FirstRunDaemon"))
        let looked = JoinLayout.lookUp("  invite:issuer.example  ", in: state, failure: nil, host: Self.host)
        XCTAssertEqual(looked.outcome, .found)
        state = looked.state
        XCTAssertEqual(state.invite, "invite:issuer.example")
        XCTAssertEqual(state.issuerHost, "issuer.example")
        XCTAssertEqual(
            JoinLayout.inviteLine(state, lookup: nil, failure: nil, copy: copy.join), .host("issuer.example"))
        XCTAssertTrue(JoinLayout.inviteIsEditable(state))

        // Once the daemon enrolled it, the joined line with the pay range
        // replaces the field, so a second invite cannot be pasted.
        state.daemonStarted = true
        state.enrolledInvite = state.invite
        let found = try Self.lookup(
            #"{"valid":true,"issuer_display_name":"Sample Labs","credit_range":{"min":10,"max":40,"unit":"points"}}"#)
        let joined = copy.join.inviteJoined
            .replacingOccurrences(of: "{host}", with: "issuer.example")
            .replacingOccurrences(of: "{pay_range}", with: "10–40 points")
        XCTAssertEqual(JoinLayout.inviteLine(state, lookup: found, failure: nil, copy: copy.join), .joined(joined))
        XCTAssertFalse(JoinLayout.inviteIsEditable(state))

        // An unknown range reads as a dash, never as zero.
        let noRange = try Self.lookup(#"{"valid":true}"#)
        XCTAssertEqual(
            JoinLayout.inviteLine(state, lookup: noRange, failure: nil, copy: copy.join),
            .joined(
                copy.join.inviteJoined
                    .replacingOccurrences(of: "{host}", with: "issuer.example")
                    .replacingOccurrences(of: "{pay_range}", with: "—")))

        // The screen's own host function is the core's (`TCInvite.issuerHost`),
        // so a real invite shows its issuer's host before the daemon runs.
        XCTAssertNil(JoinScreen.defaultIssuerHost("not an invite"))
        let real = JoinLayout.lookUp(
            "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6", in: FirstRunState(), failure: nil,
            host: JoinScreen.defaultIssuerHost)
        XCTAssertEqual(real.outcome, .found)
        XCTAssertEqual(
            JoinLayout.inviteLine(real.state, lookup: nil, failure: real.failure, copy: copy.join),
            .host("issuer.tracecommons.ai"))
    }

    /// Review Focus 2: back on Join after a dead invite, with the daemon
    /// running, the field is editable again and shows the core's invite error.
    func test_aDeadInviteReopensTheFieldWithTheCoresError() throws {
        let copy = try coreCopy()
        let state = FirstRunState(
            step: .join, invite: "invite:issuer.example", issuerHost: "issuer.example", account: .nearAI,
            daemonStarted: true)
        XCTAssertTrue(JoinLayout.inviteIsEditable(state))
        XCTAssertEqual(
            JoinLayout.inviteLine(state, lookup: nil, failure: .inviteDead(label: "exhausted"), copy: copy.join),
            .error(copy.join.inviteError))

        // Something that is not an invite is refused locally with the same
        // line, and is not kept, so it can never be enrolled. The refused
        // invite stays held while its refusal stands, so the refusal stays
        // tied to it.
        let dead = FirstRunFailure.inviteDead(label: "exhausted")
        let refused = JoinLayout.lookUp("not an invite", in: state, failure: dead, host: Self.host)
        XCTAssertEqual(refused.outcome, .refused)
        XCTAssertEqual(refused.state.invite, "invite:issuer.example")
        XCTAssertEqual(refused.state.issuerHost, "issuer.example")
        XCTAssertEqual(refused.failure, dead)

        // With no refusal pending, a refused paste drops the kept invite.
        let plain = JoinLayout.lookUp("not an invite", in: state, failure: nil, host: Self.host)
        XCTAssertEqual(plain.outcome, .refused)
        XCTAssertEqual(plain.state.invite, "")
        XCTAssertNil(plain.state.issuerHost)
        XCTAssertEqual(
            JoinLayout.inviteLine(refused.state, lookup: nil, failure: nil, copy: copy.join, refused: true),
            .error(copy.join.inviteError))

        // A new invite replaces the one the daemon refused, so its error goes.
        let replaced = JoinLayout.lookUp("invite:other.example", in: state, failure: dead, host: Self.host)
        XCTAssertEqual(replaced.outcome, .found)
        XCTAssertNil(replaced.failure)
        XCTAssertEqual(
            JoinLayout.inviteLine(replaced.state, lookup: nil, failure: replaced.failure, copy: copy.join),
            .host("other.example"))
    }

    /// The default host function is the core's (`TCInvite.issuerHost`), so a
    /// real invite shows its issuer's host on Join before the daemon runs.
    func test_theRealBridgeReadsTheHost() throws {
        let copy = try coreCopy()
        let looked = JoinLayout.lookUp(
            "https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6", in: FirstRunState(), failure: nil,
            host: TCInvite.issuerHost)
        XCTAssertEqual(looked.outcome, .found)
        XCTAssertEqual(
            JoinLayout.inviteLine(looked.state, lookup: nil, failure: looked.failure, copy: copy.join),
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
        let again = JoinLayout.lookUp("invite:issuer.example", in: returned, failure: dead, host: Self.host)
        XCTAssertEqual(again.failure, dead)
        XCTAssertEqual(
            JoinLayout.inviteLine(again.state, lookup: nil, failure: again.failure, copy: copy.join),
            .error(copy.join.inviteError))

        // A refused paste in between does not make the refused invite new:
        // looked up again, it still carries the core's error.
        let garbage = JoinLayout.lookUp("not an invite", in: returned, failure: dead, host: Self.host)
        XCTAssertEqual(garbage.outcome, .refused)
        XCTAssertEqual(garbage.failure, dead)
        let backAgain = JoinLayout.lookUp(
            "invite:issuer.example", in: garbage.state, failure: garbage.failure, host: Self.host)
        XCTAssertEqual(backAgain.outcome, .found)
        XCTAssertEqual(backAgain.failure, dead)
        XCTAssertEqual(
            JoinLayout.inviteLine(backAgain.state, lookup: nil, failure: backAgain.failure, copy: copy.join),
            .error(copy.join.inviteError))

        // And after that refused paste an emptied field can still withdraw
        // the refused invite, taking its error with it.
        XCTAssertTrue(JoinLayout.canLookUp("", in: garbage.state))
        let clearedAfterGarbage = JoinLayout.lookUp("", in: garbage.state, failure: garbage.failure, host: Self.host)
        XCTAssertEqual(clearedAfterGarbage.outcome, .withdrawn)
        XCTAssertNil(clearedAfterGarbage.failure)
        XCTAssertEqual(
            JoinLayout.inviteLine(
                clearedAfterGarbage.state, lookup: nil, failure: clearedAfterGarbage.failure, copy: copy.join),
            .hidden)

        // An empty field withdraws it: no error, no line, nothing planned.
        XCTAssertTrue(JoinLayout.canLookUp("  ", in: returned))
        XCTAssertFalse(JoinLayout.canLookUp("  ", in: FirstRunState()))
        let withdrawn = JoinLayout.lookUp("  ", in: returned, failure: dead, host: Self.host)
        XCTAssertEqual(withdrawn.outcome, .withdrawn)
        XCTAssertNil(withdrawn.failure)
        XCTAssertEqual(withdrawn.state.invite, "")
        XCTAssertNil(withdrawn.state.issuerHost)
        XCTAssertEqual(
            JoinLayout.inviteLine(
                withdrawn.state, lookup: nil, failure: withdrawn.failure, copy: copy.join,
                refused: withdrawn.outcome == .refused),
            .hidden)

        XCTAssertEqual(withdrawn.state.account, AccountAnswer.none)
        let forward = JoinLayout.forward(withdrawn.state)
        XCTAssertEqual(forward.account, .watchOnly)
        let plan = FirstRunPlan.calls(for: forward, at: .leaveRoots)
        XCTAssertFalse(plan.contains { if case .lookupInvite = $0 { return true } else { return false } })
        XCTAssertFalse(plan.contains { if case .enroll = $0 { return true } else { return false } })
        XCTAssertFalse(plan.contains(.signInNearAI))

        // Withdrawing keeps any failure that is not the invite's.
        XCTAssertEqual(
            JoinLayout.lookUp("", in: returned, failure: .signInFailed, host: Self.host).failure, .signInFailed)
    }

    /// Looking an invite up after Skip takes back watch only: the person is
    /// asking to join, so the host shows and the footer still reads Skip.
    func test_lookingUpAnInviteTakesBackWatchOnly() throws {
        let copy = try coreCopy()
        let skipped = FirstRunState(step: .join, account: .watchOnly)
        let looked = JoinLayout.lookUp("invite:issuer.example", in: skipped, failure: nil, host: Self.host)
        XCTAssertEqual(looked.state.account, AccountAnswer.none)
        XCTAssertEqual(
            JoinLayout.inviteLine(looked.state, lookup: nil, failure: nil, copy: copy.join), .host("issuer.example"))
        XCTAssertEqual(JoinLayout.footerTitle(looked.state, copy: copy), copy.join.skip)

        // A refused paste changes no answer.
        let refused = JoinLayout.lookUp("not an invite", in: skipped, failure: nil, host: Self.host)
        XCTAssertEqual(refused.state.account, .watchOnly)
    }

    /// Skip answers watch only, and a watch-only setup joins no invite
    /// (`FirstRunPlan`), so the looked-up host is not shown as if it would be.
    func test_watchOnlyHidesTheInviteItWillNotJoin() throws {
        let copy = try coreCopy()
        let looked = JoinLayout.lookUp(
            "invite:issuer.example", in: FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off]),
            failure: nil, host: Self.host
        ).state
        XCTAssertEqual(
            JoinLayout.inviteLine(looked, lookup: nil, failure: nil, copy: copy.join), .host("issuer.example"))
        let skipped = JoinLayout.forward(looked)
        XCTAssertEqual(skipped.account, .watchOnly)
        let plan = FirstRunPlan.calls(for: skipped, at: .leaveRoots)
        XCTAssertFalse(plan.isEmpty)
        XCTAssertFalse(plan.contains(.enroll("invite:issuer.example")))
        XCTAssertEqual(JoinLayout.inviteLine(skipped, lookup: nil, failure: nil, copy: copy.join), .hidden)

        // An invite the daemon already enrolled stays shown: that is a fact.
        var enrolled = skipped
        enrolled.enrolledInvite = enrolled.invite
        guard case .joined = JoinLayout.inviteLine(enrolled, lookup: nil, failure: nil, copy: copy.join) else {
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
        XCTAssertEqual(JoinLayout.nearAILine(start, copy: copy.join), copy.join.nearAiText)
        XCTAssertEqual(JoinLayout.nearAIAction(start, copy: copy), copy.join.nearAiSignIn)
        XCTAssertTrue(JoinLayout.canToggleNearAI(start))

        let chosen = JoinLayout.toggleNearAI(start)
        XCTAssertEqual(chosen.account, .nearAI)
        XCTAssertEqual(JoinLayout.nearAILine(chosen, copy: copy.join), copy.join.nearAiChosen)
        XCTAssertEqual(JoinLayout.nearAIAction(chosen, copy: copy), copy.frame.undo)
        XCTAssertTrue(JoinLayout.canToggleNearAI(chosen))
        XCTAssertEqual(FirstRunPlan.calls(for: chosen, at: .leaveRoots).last, .signInNearAI)

        let undone = JoinLayout.toggleNearAI(chosen)
        XCTAssertEqual(undone.account, AccountAnswer.none)
        XCTAssertEqual(JoinLayout.footerTitle(undone, copy: copy), copy.join.skip)
        let skipped = JoinLayout.forward(undone)
        XCTAssertEqual(skipped.account, .watchOnly)
        let plan = FirstRunPlan.calls(for: skipped, at: .leaveRoots)
        XCTAssertTrue(plan.contains { if case .startDaemon = $0 { return true } else { return false } })
        XCTAssertFalse(plan.contains(.signInNearAI))

        // Back on Join after a failed sign-in, the choice still undoes.
        var failed = JoinLayout.forward(chosen)
        failed.daemonStarted = true
        failed.step = .join
        XCTAssertEqual(JoinLayout.toggleNearAI(failed).account, AccountAnswer.none)

        // A signed-in near.ai is the daemon's fact: the card shows it and the
        // action does nothing.
        let signedIn = FirstRunState(account: .nearAI, daemonStarted: true, signedIn: true)
        XCTAssertFalse(JoinLayout.canToggleNearAI(signedIn))
        XCTAssertEqual(JoinLayout.toggleNearAI(signedIn), signedIn)
    }

    /// near.ai signs in to the account an invite enrolls: the daemon's
    /// `account_sign_in` refuses without an enrolment
    /// (`account-enrollment-required`). So near.ai waits for an invite and
    /// says so, and withdrawing the invite takes a chosen near.ai back.
    func test_nearAIWaitsForAnInvite() throws {
        let copy = try coreCopy()
        let bare = FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off])
        XCTAssertFalse(JoinLayout.canToggleNearAI(bare))
        XCTAssertEqual(JoinLayout.toggleNearAI(bare), bare)
        XCTAssertEqual(JoinLayout.nearAILine(bare, copy: copy.join), copy.join.nearAiNeedsInvite)

        let looked = JoinLayout.lookUp("invite:issuer.example", in: bare, failure: nil, host: Self.host)
        XCTAssertTrue(JoinLayout.canToggleNearAI(looked.state))
        XCTAssertEqual(JoinLayout.nearAILine(looked.state, copy: copy.join), copy.join.nearAiText)
        let chosen = JoinLayout.toggleNearAI(looked.state)
        XCTAssertEqual(chosen.account, .nearAI)

        // Emptied field: the invite goes, and the near.ai it waited for too.
        let withdrawn = JoinLayout.lookUp("", in: chosen, failure: nil, host: Self.host)
        XCTAssertEqual(withdrawn.outcome, .withdrawn)
        XCTAssertEqual(withdrawn.state.account, AccountAnswer.none)
        XCTAssertFalse(FirstRunPlan.calls(for: withdrawn.state, at: .leaveRoots).contains(.signInNearAI))

        // An enrolled invite is still one to sign in to.
        var enrolled = bare
        enrolled.daemonStarted = true
        enrolled.enrolledInvite = "invite:issuer.example"
        XCTAssertTrue(JoinLayout.canToggleNearAI(enrolled))
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
        XCTAssertFalse(JoinLayout.showsPasskeyAction(withInvite))
        XCTAssertEqual(JoinLayout.togglePasskey(withInvite), withInvite)
        XCTAssertEqual(JoinLayout.passkeyLine(withInvite, copy: copy.join), copy.join.inviteOrPasskey)
        withInvite.daemonStarted = true
        XCTAssertFalse(JoinLayout.passkeyOpensNow(withInvite, hasPasskeyAccount: true))

        // Chosen first, then an invite: the invite replaces the choice.
        let chosen = JoinLayout.togglePasskey(FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off]))
        XCTAssertEqual(chosen.account, .passkeyChosen)
        let looked = JoinLayout.lookUp("invite:issuer.example", in: chosen, failure: nil, host: Self.host)
        XCTAssertEqual(looked.outcome, .found)
        XCTAssertEqual(looked.state.account, AccountAnswer.none)
        XCTAssertFalse(FirstRunPlan.calls(for: looked.state, at: .leaveRoots).contains(.openPasskeySheets))

        // A held passkey: the invite field is closed and says why.
        let held = FirstRunState(account: .passkey(name: "Mac"), daemonStarted: true)
        XCTAssertFalse(JoinLayout.inviteIsEditable(held))
        XCTAssertEqual(
            JoinLayout.inviteLine(held, lookup: nil, failure: nil, copy: copy.join), .note(copy.join.inviteOrPasskey))
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
            XCTAssertFalse(JoinLayout.canToggleNearAI(held))
            let after = JoinLayout.toggleNearAI(held)
            XCTAssertEqual(after, held)
            XCTAssertTrue(JoinLayout.passkeyDone(after))
            XCTAssertFalse(FirstRunPlan.calls(for: after, at: .leaveRoots).contains(.signInNearAI))
        }
    }

    /// One held account leaves the other card with neither its inviting line
    /// nor its action, which could never be taken over that account.
    func test_aHeldAccountQuietsTheOtherCard() throws {
        let copy = try coreCopy()
        let start = FirstRunState(daemonStarted: true)
        XCTAssertTrue(JoinLayout.showsPasskeyAction(start))
        XCTAssertTrue(JoinLayout.showsNearAIAction(start))
        XCTAssertEqual(JoinLayout.passkeyLine(start, copy: copy.join), copy.join.passkeyText)
        XCTAssertEqual(JoinLayout.nearAILine(start, copy: copy.join), copy.join.nearAiNeedsInvite)

        // A near.ai chosen beside its invite: no passkey is created beside
        // the invite, and the passkey card says why.
        var withInvite = start
        withInvite.invite = "invite:issuer.example"
        let chosen = JoinLayout.toggleNearAI(withInvite)
        XCTAssertEqual(chosen.account, .nearAI)
        XCTAssertFalse(JoinLayout.showsPasskeyAction(chosen))
        XCTAssertEqual(JoinLayout.passkeyLine(chosen, copy: copy.join), copy.join.inviteOrPasskey)

        let signedIn = FirstRunState(account: .nearAI, daemonStarted: true, signedIn: true)
        XCTAssertFalse(JoinLayout.showsPasskeyAction(signedIn))
        XCTAssertNil(JoinLayout.passkeyLine(signedIn, copy: copy.join))

        for name in ["Mac", ""] {
            let held = FirstRunState(account: .passkey(name: name), daemonStarted: true)
            XCTAssertFalse(JoinLayout.showsNearAIAction(held))
            XCTAssertNil(JoinLayout.nearAILine(held, copy: copy.join))
        }

        let source = try Self.source()
        XCTAssertTrue(source.contains("JoinLayout.showsPasskeyAction(runner.state)"))
        XCTAssertTrue(source.contains("JoinLayout.showsNearAIAction(runner.state)"))
    }

    /// Create passkey before the daemon runs records the choice, says when
    /// it happens, and can be undone until then; once the daemon started it
    /// opens the sheets straight away. The button is never disabled
    /// without a reason.
    func test_choosingAPasskeyWaitsForTheDaemonAndCanBeUndone() throws {
        let copy = try coreCopy()
        let start = FirstRunState(toolAnswers: [.claudeCode: .off, .codex: .off])
        XCTAssertFalse(JoinLayout.passkeyOpensNow(start, hasPasskeyAccount: true))
        XCTAssertEqual(JoinLayout.passkeyAction(start, copy: copy), copy.join.passkeyCreate)

        let chosen = JoinLayout.togglePasskey(start)
        XCTAssertEqual(chosen.account, .passkeyChosen)
        XCTAssertEqual(JoinLayout.passkeyLine(chosen, copy: copy.join), copy.join.passkeyChosen)
        XCTAssertEqual(JoinLayout.passkeyAction(chosen, copy: copy), copy.frame.undo)
        XCTAssertFalse(JoinLayout.passkeyDone(chosen), "chosen is not created")
        XCTAssertTrue(JoinLayout.hasAccount(chosen))
        XCTAssertEqual(JoinLayout.footerTitle(chosen, copy: copy), copy.frame.continueButton)
        XCTAssertEqual(FirstRunPlan.calls(for: JoinLayout.forward(chosen), at: .leaveRoots).last, .openPasskeySheets)

        let undone = JoinLayout.togglePasskey(chosen)
        XCTAssertEqual(undone.account, AccountAnswer.none)
        XCTAssertEqual(JoinLayout.footerTitle(undone, copy: copy), copy.join.skip)

        // near.ai waits for an invite, and an invite replaces the chosen
        // passkey (`test_anInviteAndANewPasskeyAreNotCombined`), so near.ai
        // never replaces it directly.
        XCTAssertEqual(JoinLayout.toggleNearAI(chosen), chosen)
        XCTAssertEqual(JoinLayout.nearAILine(chosen, copy: copy.join), copy.join.nearAiNeedsInvite)

        // Watch only, then Create passkey: the passkey is the answer.
        XCTAssertEqual(JoinLayout.togglePasskey(FirstRunState(account: .watchOnly)).account, .passkeyChosen)

        // The daemon started: the sheets open now, given the account path;
        // without it the choice is recorded for the next commit.
        let started = FirstRunState(daemonStarted: true)
        XCTAssertTrue(JoinLayout.passkeyOpensNow(started, hasPasskeyAccount: true))
        XCTAssertFalse(JoinLayout.passkeyOpensNow(started, hasPasskeyAccount: false))
        XCTAssertFalse(JoinLayout.passkeyOpensNow(JoinLayout.togglePasskey(started), hasPasskeyAccount: true),
            "a chosen passkey undoes")

        // A signed-in near.ai holds the account: no passkey over it.
        let signedIn = FirstRunState(account: .nearAI, daemonStarted: true, signedIn: true)
        XCTAssertFalse(JoinLayout.passkeyOpensNow(signedIn, hasPasskeyAccount: true))
        XCTAssertEqual(JoinLayout.togglePasskey(signedIn), signedIn)
        // A held passkey is not chosen again.
        let held = FirstRunState(account: .passkey(name: "Mac"), daemonStarted: true)
        XCTAssertEqual(JoinLayout.togglePasskey(held), held)

        let source = try Self.source()
        XCTAssertFalse(source.contains("passkeyAvailable"), "the disabled-with-no-reason button is gone")
        XCTAssertTrue(source.contains("JoinLayout.passkeyOpensNow("))
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

        let source = try Self.source()
        XCTAssertTrue(source.contains("runner.passkeyOutcome?.joinNotice(copy)"))
    }

    func test_skipReadsWatchOnlyUntilAnAccountExists() throws {
        let copy = try coreCopy()
        for account in [AccountAnswer.none, .watchOnly] {
            let state = FirstRunState(account: account)
            XCTAssertFalse(JoinLayout.hasAccount(state))
            XCTAssertEqual(JoinLayout.footerTitle(state, copy: copy), copy.join.skip)
            XCTAssertEqual(JoinLayout.footerNote(state, copy: copy), copy.join.skipNote)
        }
        for account in [AccountAnswer.nearAI, .passkeyChosen, .passkey(name: "Mac"), .passkey(name: "")] {
            let state = FirstRunState(account: account)
            XCTAssertTrue(JoinLayout.hasAccount(state))
            XCTAssertEqual(JoinLayout.footerTitle(state, copy: copy), copy.frame.continueButton)
            XCTAssertNil(JoinLayout.footerNote(state, copy: copy))
        }

        // Skip answers "watch only" and moves on; Continue keeps the account.
        let skipped = JoinLayout.forward(FirstRunState())
        XCTAssertEqual(skipped.account, .watchOnly)
        XCTAssertEqual(skipped.step, .folders)
        let continued = JoinLayout.forward(
            JoinLayout.toggleNearAI(FirstRunState(tier: .custom, invite: "invite:issuer.example")))
        XCTAssertEqual(continued.account, .nearAI)
        XCTAssertEqual(continued.step, .tools)

        // near.ai reads "Signed in" only once the daemon signed in.
        XCTAssertFalse(JoinLayout.showsSignedIn(FirstRunState(account: .nearAI)))
        XCTAssertTrue(JoinLayout.showsSignedIn(FirstRunState(account: .nearAI, signedIn: true)))
    }

    /// The passkey sheets' outcomes as Join records them. A passkey known
    /// without a name never shows the ready line with a blank in it.
    func test_passkeyOutcomesBecomeTheAccount() throws {
        let copy = try coreCopy()
        let start = FirstRunState()
        XCTAssertEqual(JoinLayout.passkeyLine(start, copy: copy.join), copy.join.passkeyText)
        XCTAssertFalse(JoinLayout.passkeyDone(start))

        let created = JoinLayout.apply(.created(name: "Mac"), to: start, copy: copy)
        XCTAssertEqual(created.state.account, .passkey(name: "Mac"))
        XCTAssertNil(created.notice)
        XCTAssertTrue(JoinLayout.passkeyDone(created.state))
        XCTAssertEqual(
            JoinLayout.passkeyLine(created.state, copy: copy.join),
            copy.join.passkeyReady.replacingOccurrences(of: "{name}", with: "Mac"))

        for nameless in [PasskeySheetOutcome.signedIn, .existingAccount] {
            let applied = JoinLayout.apply(nameless, to: start, copy: copy)
            XCTAssertTrue(JoinLayout.hasAccount(applied.state))
            XCTAssertTrue(JoinLayout.passkeyDone(applied.state))
            XCTAssertNil(JoinLayout.passkeyLine(applied.state, copy: copy.join))
        }

        XCTAssertEqual(JoinLayout.apply(.closed, to: start, copy: copy).state, start)

        let signedOut = JoinLayout.apply(.signedOut, to: created.state, copy: copy)
        XCTAssertEqual(signedOut.state.account, .none)
        XCTAssertEqual(signedOut.notice, copy.join.signedOut)

        // The sign-out ends the daemon's account session, a near.ai one
        // included: the chosen near.ai stays chosen and is signed in again.
        let nearAI = FirstRunState(
            account: .nearAI, toolAnswers: [.claudeCode: .off, .codex: .off], daemonStarted: true, signedIn: true)
        let afterNearAI = JoinLayout.apply(.signedOut, to: nearAI, copy: copy)
        XCTAssertEqual(afterNearAI.state.account, .nearAI)
        XCTAssertFalse(afterNearAI.state.signedIn)
        XCTAssertEqual(FirstRunPlan.calls(for: afterNearAI.state, at: .leaveRoots).last, .signInNearAI)
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
}

/// A daemon that confirms nothing; Join's tests never commit.
@MainActor
private final class NoDaemon: FirstRunDaemon {
    func startDaemon(settingsJSON: String) async -> Bool { false }
    func setSourceSettings(settingsJSON: String) async -> Bool { false }
    func lookupInvite(_ invite: String) async -> FirstRunLookup { .refused(label: "none") }
    func enrollInvite(_ invite: String) async -> Bool { false }
    func signInNearAI() async -> Bool { false }
    func saveConsentScopes(_ scopes: [String]) async -> Bool { false }
    func setProjectMode(projectID: String, mode: ProjectMode) async -> Bool { false }
    func includePastSessions(projectID: String, sessionIDs: [String]) async -> Bool { false }
    func setPrivateAI(_ on: Bool) async -> Bool { false }
    func grantAutomatic(witness: String?) async -> FirstRunGrantAnswer { .refused(label: "none") }
    func markComplete() async -> Bool { false }
    func markWatchOnlyComplete() async -> Bool { false }
    func firstRunFinished(notice: String?) {}
}
