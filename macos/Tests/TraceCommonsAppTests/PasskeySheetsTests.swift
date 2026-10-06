import TCBridge
import TCShellCore
import XCTest

@testable import TraceCommonsApp

/// Ron's passkey popups (#1030 `passkey-flow.tsx`, `ftux-model.ts`
/// `passkeyTransition` and `passkeyNameError`) on the native passkey
/// coordinator. P-1, P-2, P-5 and P-7 are sheets; P-3, P-4 and P-6 are the
/// system's own, raised by `NativePasskeyCoordinator.perform(_:label:)`, so
/// nothing here draws one.
@MainActor
final class PasskeySheetsTests: XCTestCase {
    /// Records every account call and answers from a script.
    @MainActor
    final class RecordingAccount: PasskeyAccount {
        var calls: [String] = []
        var labels: [String?] = []
        var ceremonyAnswer: PasskeyCallResult = .done
        var bindAnswer: PasskeyBindResult = .bound
        var signOutAnswer: PasskeyCallResult = .done
        var signInAnswer: PasskeySignInResult = .unbound

        func ceremony(_ action: NativePasskeyAction, label: String?) async -> PasskeyCallResult {
            calls.append("ceremony:\(action.rawValue)")
            labels.append(label)
            return ceremonyAnswer
        }

        func bind() async -> PasskeyBindResult {
            calls.append("bind")
            return bindAnswer
        }

        func signIn() async -> PasskeySignInResult {
            calls.append("signIn")
            return signInAnswer
        }

        func cancel() {
            calls.append("cancel")
        }

        func signOut() async -> PasskeyCallResult {
            calls.append("signOut")
            return signOutAnswer
        }

        var existing: Int? = 0
        func existingPasskeys() async -> Int? {
            calls.append("existingPasskeys")
            return existing
        }
    }

    private func coreCopy() throws -> FirstRunCopy {
        try XCTUnwrap(FirstRunCopy.decode(try XCTUnwrap(TCCoreCopy.firstRunCopyJSON())))
    }

    private static func source() throws -> String {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp/Views/FirstRun/PasskeySheets.swift")
        return try String(contentsOf: url, encoding: .utf8)
    }

    func test_aBlankNameIsRefusedWithRonsLine() async throws {
        let copy = try coreCopy()
        let account = RecordingAccount()
        let model = PasskeySheetModel(copy: copy.passkey, account: account)
        XCTAssertEqual(model.step, .choose)
        model.createNew()
        XCTAssertEqual(model.step, .name)
        // Ron's default name, and no error before the field is touched.
        XCTAssertEqual(model.name, copy.passkey.defaultName)
        XCTAssertNil(model.nameError)

        model.name = "   "
        await model.submitName()
        XCTAssertEqual(model.nameError, copy.passkey.nameEmpty)
        XCTAssertEqual(model.step, .name)
        XCTAssertEqual(account.calls, [], "a refused name never reaches the coordinator")

        // Too long: the core's line with its maximum filled, the coordinator's
        // own limit, so a name the sheet accepts the coordinator accepts.
        model.name = String(repeating: "x", count: PasskeyName.maxLength + 1)
        await model.submitName()
        XCTAssertEqual(
            model.nameError,
            copy.passkey.nameTooLong.replacingOccurrences(of: "{max}", with: "\(PasskeyName.maxLength)"))
        XCTAssertFalse(model.nameError?.contains("{") ?? true)
        XCTAssertEqual(account.calls, [])
        XCTAssertEqual(PasskeyName.maxLength, 64)

        // A good name is sent trimmed, raises the system sheet, and reaches Verify.
        model.name = "  Work laptop  "
        await model.submitName()
        XCTAssertNil(model.nameError)
        XCTAssertEqual(account.calls, ["ceremony:create"])
        XCTAssertEqual(account.labels, ["Work laptop"])
        XCTAssertEqual(model.step, .verify)

        // Verify binds the account and finishes with the name.
        await model.verify()
        XCTAssertEqual(account.calls, ["ceremony:create", "bind"])
        XCTAssertEqual(model.outcome, .created(name: "Work laptop"))
    }

    func test_cancelAtVerifySignsOut() async throws {
        let copy = try coreCopy()
        let account = RecordingAccount()
        let model = PasskeySheetModel(copy: copy.passkey, account: account)
        model.createNew()
        await model.submitName()
        XCTAssertEqual(model.step, .verify)

        // A sign-out the daemon refuses is not reported as one: Verify stays,
        // with the daemon's label.
        account.signOutAnswer = .refused(label: "account-session-required")
        await model.cancelVerify()
        XCTAssertNil(model.outcome)
        XCTAssertEqual(model.step, .verify)
        XCTAssertEqual(model.refusal, "account-session-required")

        account.signOutAnswer = .done
        await model.cancelVerify()
        XCTAssertEqual(model.outcome, .signedOut)
        XCTAssertEqual(account.calls, ["ceremony:create", "signOut", "signOut"])
        XCTAssertEqual(account.calls.filter { $0 == "bind" }, [], "nothing is bound after a cancel")

        // Join reads the core's signed-out sentence for that outcome only.
        XCTAssertEqual(PasskeySheetOutcome.signedOut.joinNotice(copy), copy.join.signedOut)
        XCTAssertNil(PasskeySheetOutcome.closed.joinNotice(copy))
        XCTAssertNil(PasskeySheetOutcome.signedIn.joinNotice(copy))
        XCTAssertNil(PasskeySheetOutcome.created(name: "n").joinNotice(copy))
        XCTAssertNil(PasskeySheetOutcome.existingAccount.joinNotice(copy))
    }

    private func bindResult(_ json: String) throws -> NativeAccountBindResult {
        try JSONDecoder().decode(NativeAccountBindResult.self, from: Data(json.utf8))
    }

    /// The daemon's bind answers `bound` (this passkey's account) or
    /// `existing_account` (a switch to an account that already existed, with
    /// no claim that the new passkey moved). Only `bound` is a created passkey.
    func test_anExistingAccountIsNeverReportedAsCreated() async throws {
        XCTAssertEqual(
            PasskeyBindResult(try bindResult(#"{"outcome":"bound","binding_state":"bound"}"#)), .bound)
        XCTAssertEqual(
            PasskeyBindResult(try bindResult(#"{"outcome":"existing_account","binding_state":"legacy"}"#)),
            .existingAccount)
        XCTAssertEqual(
            PasskeyBindResult(try bindResult(#"{"outcome":"existing_account","binding_state":"bound"}"#)),
            .existingAccount)
        // Anything else fails closed: no outcome claims a binding.
        XCTAssertEqual(
            PasskeyBindResult(try bindResult(#"{"outcome":"bound","binding_state":"legacy"}"#)),
            .failed(.refused(label: "account-bind-invalid")))
        XCTAssertEqual(
            PasskeyBindResult(try bindResult(#"{"outcome":"other","binding_state":"bound"}"#)),
            .failed(.refused(label: "account-bind-invalid")))

        let copy = try coreCopy()
        let account = RecordingAccount()
        let model = PasskeySheetModel(copy: copy.passkey, account: account)
        model.createNew()
        model.name = "Work laptop"
        await model.submitName()
        account.bindAnswer = .existingAccount
        await model.verify()
        XCTAssertEqual(account.calls, ["ceremony:create", "bind"])
        XCTAssertEqual(model.outcome, .existingAccount)
        if case .created = model.outcome { XCTFail("existing_account must not claim the passkey") }
    }

    /// Leaving the sheet while a system sheet is up dismisses it, so a later
    /// ceremony does not meet a busy coordinator. Idle, nothing is cancelled.
    func test_disappearingWhileBusyCancelsTheSystemSheet() async throws {
        let copy = try coreCopy()
        let account = RecordingAccount()
        let model = PasskeySheetModel(copy: copy.passkey, account: account)
        model.disappeared()
        XCTAssertEqual(account.calls, [])

        let gate = AsyncStream<Void>.makeStream()
        let blocking = BlockingAccount(release: gate.stream)
        let busyModel = PasskeySheetModel(copy: copy.passkey, account: blocking)
        let running = Task { await busyModel.useExisting() }
        while !busyModel.busy { await Task.yield() }
        busyModel.disappeared()
        XCTAssertEqual(blocking.calls, ["signIn", "cancel"])
        gate.continuation.yield()
        gate.continuation.finish()
        await running.value
    }

    /// Holds the ceremony open until released.
    @MainActor
    final class BlockingAccount: PasskeyAccount {
        var calls: [String] = []
        let release: AsyncStream<Void>
        init(release: AsyncStream<Void>) { self.release = release }

        func ceremony(_ action: NativePasskeyAction, label: String?) async -> PasskeyCallResult {
            calls.append("ceremony:\(action.rawValue)")
            return .cancelled
        }
        func signIn() async -> PasskeySignInResult {
            calls.append("signIn")
            for await _ in release { break }
            return .failed(.cancelled)
        }
        func bind() async -> PasskeyBindResult { .bound }
        func signOut() async -> PasskeyCallResult { .done }
        func cancel() { calls.append("cancel") }
        func existingPasskeys() async -> Int? { 0 }
    }

    /// The daemon counts Unicode scalars, not grapheme clusters; the sheet
    /// matches it so a name it accepts is never refused as invalid later.
    func test_theNameLimitCountsScalarsAsTheDaemonDoes() throws {
        let copy = try coreCopy()
        let tooLong = copy.passkey.nameTooLong.replacingOccurrences(of: "{max}", with: "\(PasskeyName.maxLength)")
        // 33 graphemes, 66 scalars.
        let combining = String(repeating: "e\u{0301}", count: 33)
        XCTAssertEqual(combining.count, 33)
        XCTAssertEqual(PasskeyName.error(combining, copy: copy.passkey), tooLong)
        let fits = String(repeating: "e\u{0301}", count: 32)
        XCTAssertNil(PasskeyName.error(fits, copy: copy.passkey))
    }

    /// Ron's table with the system sheets as ceremony results: a cancelled
    /// system sheet steps back and says nothing; any other failure stays put
    /// with the label; closing before a passkey exists changes nothing.
    func test_theStepsFollowRonsTable() async throws {
        let copy = try coreCopy()
        let account = RecordingAccount()
        let model = PasskeySheetModel(copy: copy.passkey, account: account)
        model.createNew()
        model.back()
        XCTAssertEqual(model.step, .choose)

        // Use existing: the system sign-in sheet; cancelled returns to Choose.
        account.signInAnswer = .failed(.cancelled)
        await model.useExisting()
        XCTAssertEqual(account.calls, ["signIn"])
        XCTAssertEqual(model.step, .choose)
        XCTAssertNil(model.refusal)
        XCTAssertNil(model.outcome)

        account.signInAnswer = .failed(.refused(label: "passkey-authorization-failed"))
        await model.useExisting()
        XCTAssertEqual(model.step, .choose)
        XCTAssertEqual(model.refusal, "passkey-authorization-failed")

        // A sign-in is not an account on this Mac: Verify binds it first.
        account.signInAnswer = .unbound
        await model.useExisting()
        XCTAssertEqual(model.step, .verify)
        XCTAssertNil(model.outcome)
        await model.verify()
        XCTAssertEqual(model.outcome, .signedIn)

        // A cancelled save sheet returns to Name.
        let creating = PasskeySheetModel(copy: copy.passkey, account: account)
        creating.createNew()
        account.ceremonyAnswer = .cancelled
        await creating.submitName()
        XCTAssertEqual(creating.step, .name)
        XCTAssertNil(creating.refusal)

        // A refused bind stays on Verify with the label and claims nothing.
        account.ceremonyAnswer = .done
        account.bindAnswer = .failed(.refused(label: "account-bind-refused"))
        await creating.submitName()
        await creating.verify()
        XCTAssertEqual(creating.step, .verify)
        XCTAssertEqual(creating.refusal, "account-bind-refused")
        XCTAssertNil(creating.outcome)

        // Closing Choose or Name before a passkey exists is a plain close.
        let closing = PasskeySheetModel(copy: copy.passkey, account: account)
        closing.close()
        XCTAssertEqual(closing.outcome, .closed)

        // Welcome back: sign in raises the system sheet, then Verify binds;
        // other options closes.
        let returning = PasskeySheetModel(start: .welcomeBack, copy: copy.passkey, account: account)
        account.calls = []
        account.bindAnswer = .bound
        await returning.useExisting()
        XCTAssertEqual(returning.step, .verify)
        await returning.verify()
        XCTAssertEqual(account.calls, ["signIn", "bind"])
        XCTAssertEqual(returning.outcome, .signedIn)
        let elsewhere = PasskeySheetModel(start: .welcomeBack, copy: copy.passkey, account: account)
        elsewhere.close()
        XCTAssertEqual(elsewhere.outcome, .closed)
    }

    /// The coordinator's failures map to labels, never to a sentence; a
    /// cancelled system sheet is a cancel, not a refusal.
    func test_failuresMapToLabels() {
        XCTAssertEqual(PasskeyCallResult(error: NativePasskeyFailure.cancelled), .cancelled)
        XCTAssertEqual(
            PasskeyCallResult(error: NativePasskeyFailure.busy),
            .refused(label: NativePasskeyFailure.busy.rawValue))
        XCTAssertEqual(
            PasskeyCallResult(error: DaemonClient.Failure(code: "refused", message: "account-bind-refused")),
            .refused(label: "account-bind-refused"))
        // The daemon's own view sentence wins over its bare message.
        var viewed = DaemonClient.Failure(code: "refused", message: "account-bind-refused")
        viewed.viewMessage = "daemon view text"
        XCTAssertEqual(PasskeyCallResult(error: viewed), .refused(label: "daemon view text"))
        struct Other: Error {}
        XCTAssertEqual(
            PasskeyCallResult(error: Other()),
            .refused(label: NativePasskeyFailure.authorizationFailed.rawValue))
    }

    /// A refusal is held as its label (`model.refusal`) and shown as the
    /// core's sentence: a kebab-case daemon label such as
    /// `account-already-enrolled` never reaches the sheet.
    func test_aRefusalShowsTheCoresSentenceNotItsLabel() throws {
        let copy = try coreCopy()
        for label in ["account-already-enrolled", "passkey-busy", NativePasskeyFailure.authorizationFailed.rawValue] {
            XCTAssertEqual(PasskeySheets.refusalLine(label, copy: copy.passkey), copy.passkey.refused, label)
        }
        XCTAssertNil(PasskeySheets.refusalLine(nil, copy: copy.passkey))
        let source = try Self.source()
        XCTAssertTrue(source.contains("PasskeySheets.noticeLine(model"))
        XCTAssertTrue(source.contains("refusalLine(model.refusal"))
        XCTAssertFalse(source.contains("Text(refusal)"))
    }

    /// P-3, P-4 and P-6 are drawn by macOS. The file draws no copy of them:
    /// no marked imitation, no store chooser, no fingerprint button.
    func test_noSimulatedSheetExists() throws {
        let source = try Self.source()
        for imitation in [
            "Simulated", "Touch ID", "Save a passkey", "Save in", "1Password", "Passwords",
            "More Options", "Sign In", "PasskeyStore",
        ] {
            XCTAssertFalse(source.contains(imitation), "found \(imitation)")
        }
        // The one Touch ID glyph is P-7's own tinted circle (Ron's
        // `WelcomeBack`), named once in the popup layout, never a button.
        XCTAssertEqual(source.components(separatedBy: "\"touchid\"").count - 1, 1)
        // The four popups are the app's; the system ones go through the coordinator.
        XCTAssertEqual(source.components(separatedBy: "        popup(.").count - 1, 4)
        XCTAssertTrue(source.contains("coordinator.perform("))
        XCTAssertTrue(source.contains("accountBind()") || source.contains("connectNearAI()"))
        // Every word on the sheets is the core's.
        XCTAssertTrue(source.contains("copy.passkey.chooseTitle"))
        XCTAssertTrue(source.contains("copy.passkey.nameWarning"))
        XCTAssertTrue(source.contains("copy.passkey.verifyTitle"))
        XCTAssertTrue(source.contains("copy.passkey.welcomeTitle"))
    }

    /// Each mount of the presenter holds its own model, so two mounts would
    /// present two sheets for one request. Exactly one mount in the app's
    /// sources: the first-run host's, which is on screen at every step, so
    /// a request the Folders or Tools commit raises presents at once.
    func test_thePasskeySheetsAreMountedExactlyOnce() throws {
        let sources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .deletingLastPathComponent()
            .appendingPathComponent("Sources/TraceCommonsApp")
        let files = try XCTUnwrap(FileManager.default.enumerator(at: sources, includingPropertiesForKeys: nil))
        var mounts: [String] = []
        for case let url as URL in files where url.pathExtension == "swift" {
            let text = try String(contentsOf: url, encoding: .utf8)
            let count = text.components(separatedBy: ".firstRunPasskeySheets(").count - 1
            mounts += Array(repeating: url.lastPathComponent, count: count)
        }
        XCTAssertEqual(mounts, ["OnboardingCoordinatorView.swift"])
    }

    /// Ron's P-7: a returning person, whose Mac already holds a passkey for
    /// this account, opens at Welcome back. None, or a daemon that cannot
    /// say, opens at P-1, which still offers "Use existing passkey".
    func test_aReturningPasskeyOpensAtWelcomeBack() {
        XCTAssertEqual(PasskeySheetModel.startStep(existingPasskeys: 1), .welcomeBack)
        XCTAssertEqual(PasskeySheetModel.startStep(existingPasskeys: 3), .welcomeBack)
        XCTAssertEqual(PasskeySheetModel.startStep(existingPasskeys: 0), .choose)
        XCTAssertEqual(PasskeySheetModel.startStep(existingPasskeys: nil), .choose)
    }

    /// The first-run presenter asks the daemon before it opens the sheets,
    /// and opens them at the step that answer chooses.
    func test_theFirstRunPresenterAsksForExistingPasskeys() throws {
        let source = try Self.source()
        XCTAssertTrue(source.contains("await account.existingPasskeys()"))
        XCTAssertTrue(source.contains("PasskeySheetModel.startStep(existingPasskeys:"))
    }

    /// Kristi's #1235 B1, decision (a): "Use existing passkey" signs in, and
    /// a sign-in alone holds no enrolment. An account no Mac has bound goes
    /// through Verify, whose bind enrols this Mac; only then is the outcome
    /// `.signedIn`, which Join records as a held passkey. Cancelling that
    /// Verify signs out, as it does after Create.
    func test_anUnboundExistingPasskeyIsBoundThroughVerify() async throws {
        let copy = try coreCopy()
        let account = RecordingAccount()
        account.signInAnswer = .unbound
        let model = PasskeySheetModel(copy: copy.passkey, account: account)
        await model.useExisting()
        XCTAssertEqual(model.step, .verify)
        XCTAssertNil(model.outcome, "no outcome before the bind")
        XCTAssertEqual(account.calls, ["signIn"])

        account.bindAnswer = .failed(.refused(label: "near_ai_enroll_no_session"))
        await model.verify()
        XCTAssertNil(model.outcome)
        XCTAssertEqual(model.refusal, "near_ai_enroll_no_session")

        account.bindAnswer = .bound
        await model.verify()
        XCTAssertEqual(model.outcome, .signedIn)
        XCTAssertEqual(account.calls, ["signIn", "bind", "bind"])

        // bind switched to an account that already existed.
        let switching = PasskeySheetModel(copy: copy.passkey, account: account)
        account.bindAnswer = .existingAccount
        await switching.useExisting()
        await switching.verify()
        XCTAssertEqual(switching.outcome, .existingAccount)

        // Cancel at Verify signs out.
        let cancelling = PasskeySheetModel(copy: copy.passkey, account: account)
        await cancelling.useExisting()
        await cancelling.cancelVerify()
        XCTAssertEqual(cancelling.outcome, .signedOut)
    }

    /// Kristi's #1235 B1, decision (b), deferred: an account already bound
    /// (on another Mac, or a legacy one) cannot be enrolled from this Mac yet,
    /// so the sign-in fails closed. The daemon's session is signed out, so
    /// nothing half-held survives, and the sheet says why with the core's
    /// line. No outcome is reported, so Join records no account.
    func test_anAccountBoundElsewhereIsSignedOutAndSaysSo() async throws {
        let copy = try coreCopy()
        let account = RecordingAccount()
        account.signInAnswer = .alreadyBound
        let model = PasskeySheetModel(start: .welcomeBack, copy: copy.passkey, account: account)
        await model.useExisting()
        XCTAssertEqual(account.calls, ["signIn", "signOut"])
        XCTAssertNil(model.outcome)
        XCTAssertEqual(model.step, .choose)
        XCTAssertNil(model.refusal)
        XCTAssertEqual(PasskeySheets.noticeLine(model, copy: copy.passkey), copy.passkey.boundElsewhere)
        XCTAssertNotEqual(copy.passkey.boundElsewhere, copy.passkey.refused)

        // Moving on clears the line.
        model.createNew()
        XCTAssertNil(PasskeySheets.noticeLine(model, copy: copy.passkey))

        // A sign-out the daemon refuses is said as a refusal: the session
        // may still be held, so the bound-elsewhere line would not be true.
        let stuck = PasskeySheetModel(copy: copy.passkey, account: account)
        account.signOutAnswer = .refused(label: "account-session-changed")
        await stuck.useExisting()
        XCTAssertNil(stuck.outcome)
        XCTAssertEqual(stuck.refusal, "account-session-changed")
        XCTAssertEqual(PasskeySheets.noticeLine(stuck, copy: copy.passkey), copy.passkey.refused)

        let source = try Self.source()
        XCTAssertTrue(source.contains("PasskeySheets.noticeLine(model"))
    }

    /// The daemon's `binding_state` after a passkey sign-in. Only `unbound`
    /// can be bound here; `bound` and `legacy` are bound already; anything
    /// else is not a state this build knows and fails closed.
    func test_aSignInsBindingStateDecidesTheNextStep() {
        XCTAssertEqual(PasskeySignInResult(bindingState: "unbound"), .unbound)
        XCTAssertEqual(PasskeySignInResult(bindingState: "bound"), .alreadyBound)
        XCTAssertEqual(PasskeySignInResult(bindingState: "legacy"), .alreadyBound)
        XCTAssertEqual(PasskeySignInResult(bindingState: "closed"), .unrecognised)
        XCTAssertEqual(PasskeySignInResult(bindingState: ""), .unrecognised)
    }

    /// Ron's review of #1235, item 14: P-1 and P-2 carry the passkey icon
    /// and P-5 the lock, in the round tinted circle; P-7 its tinted Touch ID
    /// circle and a display-size title. Headings are centred, and Back and
    /// Close are round icon buttons in the corners (Close on P-1, Back and
    /// Close on P-2, none on P-5 and P-7, as Ron's). P-5's Verify is the
    /// outlined button.
    func test_thePopupsFollowRonsDesign() throws {
        XCTAssertEqual(PasskeyPopupLayout.icon(.choose), .passkey)
        XCTAssertEqual(PasskeyPopupLayout.icon(.name), .passkey)
        XCTAssertEqual(PasskeyPopupLayout.icon(.verify), .lock)
        XCTAssertNil(PasskeyPopupLayout.icon(.welcomeBack), "P-7's circle sits in its card")
        XCTAssertEqual(PasskeyPopupLayout.corners(.choose), PasskeyPopupLayout.Corners(back: false, close: true))
        XCTAssertEqual(PasskeyPopupLayout.corners(.name), PasskeyPopupLayout.Corners(back: true, close: true))
        XCTAssertEqual(PasskeyPopupLayout.corners(.verify), PasskeyPopupLayout.Corners(back: false, close: false))
        XCTAssertEqual(PasskeyPopupLayout.corners(.welcomeBack), PasskeyPopupLayout.Corners(back: false, close: false))

        let source = try Self.source()
        XCTAssertTrue(source.contains("GlassRoundButton(backLabel"))
        XCTAssertTrue(source.contains("GlassRoundButton(closeLabel"))
        XCTAssertFalse(source.contains("Button(copy.passkey.back)"))
        XCTAssertFalse(source.contains("Button(copy.passkey.close)"))
        XCTAssertTrue(source.contains(".multilineTextAlignment(.center)"))
        XCTAssertTrue(source.contains("GlassTokens.TypeScale.display"))
        XCTAssertTrue(source.contains(".passkeyOutlined()"))
    }
}
