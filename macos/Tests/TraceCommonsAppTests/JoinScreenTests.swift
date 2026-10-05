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
        // does not run yet.
        let looked = JoinLayout.lookUp("  invite:issuer.example  ", in: state, host: Self.host)
        XCTAssertTrue(looked.found)
        state = looked.state
        XCTAssertEqual(state.invite, "invite:issuer.example")
        XCTAssertEqual(state.issuerHost, "issuer.example")
        XCTAssertFalse(state.daemonStarted)
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
        // line, and is not kept, so it can never be enrolled.
        let refused = JoinLayout.lookUp("not an invite", in: state, host: Self.host)
        XCTAssertFalse(refused.found)
        XCTAssertEqual(refused.state.invite, "")
        XCTAssertNil(refused.state.issuerHost)
        XCTAssertEqual(
            JoinLayout.inviteLine(refused.state, lookup: nil, failure: nil, copy: copy.join, refused: true),
            .error(copy.join.inviteError))
    }

    func test_skipReadsWatchOnlyUntilAnAccountExists() throws {
        let copy = try coreCopy()
        for account in [AccountAnswer.none, .watchOnly] {
            let state = FirstRunState(account: account)
            XCTAssertFalse(JoinLayout.hasAccount(state))
            XCTAssertEqual(JoinLayout.footerTitle(state, copy: copy), copy.join.skip)
            XCTAssertEqual(JoinLayout.footerNote(state, copy: copy), copy.join.skipNote)
        }
        for account in [AccountAnswer.nearAI, .passkey(name: "Mac"), .passkey(name: "")] {
            let state = FirstRunState(account: account)
            XCTAssertTrue(JoinLayout.hasAccount(state))
            XCTAssertEqual(JoinLayout.footerTitle(state, copy: copy), copy.frame.continueButton)
            XCTAssertNil(JoinLayout.footerNote(state, copy: copy))
        }

        // Skip answers "watch only" and moves on; Continue keeps the account.
        let skipped = JoinLayout.forward(FirstRunState())
        XCTAssertEqual(skipped.account, .watchOnly)
        XCTAssertEqual(skipped.step, .folders)
        let continued = JoinLayout.forward(JoinLayout.chooseNearAI(FirstRunState(tier: .custom)))
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
            "copy.join.nearAiEyebrow", "copy.join.nearAiText", "copy.join.nearAiSignIn", "copy.join.signedIn",
            "copy.join.noSharing",
        ] {
            XCTAssertTrue(source.contains(field), "missing \(field)")
        }
        XCTAssertTrue(source.contains("FirstRunFrame("))
        XCTAssertTrue(source.contains("PasskeySheets("))
        XCTAssertTrue(source.contains("TCInvite.issuerHost"))
    }
}
