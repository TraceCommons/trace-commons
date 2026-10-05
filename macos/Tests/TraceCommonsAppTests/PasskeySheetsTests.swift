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

        func ceremony(_ action: NativePasskeyAction, label: String?) async -> PasskeyCallResult {
            calls.append("ceremony:\(action.rawValue)")
            labels.append(label)
            return ceremonyAnswer
        }

        func bind() async -> PasskeyBindResult {
            calls.append("bind")
            return bindAnswer
        }

        func cancel() {
            calls.append("cancel")
        }

        func signOut() async -> PasskeyCallResult {
            calls.append("signOut")
            return signOutAnswer
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
        XCTAssertEqual(blocking.calls, ["ceremony:login", "cancel"])
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
            for await _ in release { break }
            return .cancelled
        }
        func bind() async -> PasskeyBindResult { .bound }
        func signOut() async -> PasskeyCallResult { .done }
        func cancel() { calls.append("cancel") }
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
        account.ceremonyAnswer = .cancelled
        await model.useExisting()
        XCTAssertEqual(account.calls, ["ceremony:login"])
        XCTAssertEqual(model.step, .choose)
        XCTAssertNil(model.refusal)
        XCTAssertNil(model.outcome)

        account.ceremonyAnswer = .refused(label: "passkey-authorization-failed")
        await model.useExisting()
        XCTAssertEqual(model.step, .choose)
        XCTAssertEqual(model.refusal, "passkey-authorization-failed")

        account.ceremonyAnswer = .done
        await model.useExisting()
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

        // Welcome back: sign in raises the system sheet; other options closes.
        let returning = PasskeySheetModel(start: .welcomeBack, copy: copy.passkey, account: account)
        account.calls = []
        await returning.useExisting()
        XCTAssertEqual(account.calls, ["ceremony:login"])
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

    /// P-3, P-4 and P-6 are drawn by macOS. The file draws no copy of them:
    /// no marked imitation, no store chooser, no fingerprint button.
    func test_noSimulatedSheetExists() throws {
        let source = try Self.source()
        for imitation in [
            "Simulated", "Touch ID", "Save a passkey", "Save in", "1Password", "Passwords",
            "More Options", "Sign In", "touchid", "fingerprint", "PasskeyStore",
        ] {
            XCTAssertFalse(source.contains(imitation), "found \(imitation)")
        }
        // The four sheets are glass sheets; the system ones go through the coordinator.
        XCTAssertEqual(source.components(separatedBy: "GlassSheet(").count - 1, 4)
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
    /// sources: Join's until port plan Task 11, then the coordinator's.
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
        XCTAssertEqual(mounts, ["JoinScreen.swift"])
    }
}
