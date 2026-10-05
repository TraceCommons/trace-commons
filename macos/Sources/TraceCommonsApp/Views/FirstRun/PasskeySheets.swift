import AppKit
import SwiftUI
import TCDesign
import TCShellCore

/// The passkey popups Swift draws (#1030 `passkey-flow.tsx`): P-1 Choose,
/// P-2 Name, P-5 Verify and P-7 Welcome back. P-3, P-4 and P-6 are system
/// sheets that `NativePasskeyCoordinator.perform(_:label:)` raises, so they
/// appear here only as the result of a ceremony.
enum PasskeySheetStep: Equatable {
    case choose
    case name
    case verify
    case welcomeBack
}

/// How the sheets ended, for Join.
enum PasskeySheetOutcome: Equatable {
    /// A passkey was created and bound to the account (`bound`).
    case created(name: String)
    /// An existing passkey signed in.
    case signedIn
    /// Verify's bind answered `existing_account`: the daemon switched to an
    /// account that already existed and makes no claim that the new passkey
    /// moved, so no passkey name is carried.
    case existingAccount
    /// Closed before a passkey existed; Join is unchanged.
    case closed
    /// Verify was cancelled and the daemon confirmed the sign-out.
    case signedOut

    /// The core's sentence Join shows for this outcome, if any.
    func joinNotice(_ copy: FirstRunCopy) -> String? {
        self == .signedOut ? copy.join.signedOut : nil
    }
}

/// What one account call answered. A refusal carries the daemon's or the
/// coordinator's label, never a sentence written here.
enum PasskeyCallResult: Equatable {
    case done
    /// The person dismissed the system sheet.
    case cancelled
    case refused(label: String)

    init(error: any Error) {
        switch error {
        case NativePasskeyFailure.cancelled:
            self = .cancelled
        case let failure as NativePasskeyFailure:
            self = .refused(label: failure.rawValue)
        case let failure as DaemonClient.Failure:
            self = .refused(label: failure.viewMessage ?? failure.message)
        default:
            self = .refused(label: NativePasskeyFailure.authorizationFailed.rawValue)
        }
    }
}

/// What P-5's `account_bind` answered. Only `bound` with `binding_state`
/// `bound` is the new passkey's account; anything the daemon did not send
/// fails closed.
enum PasskeyBindResult: Equatable {
    case bound
    case existingAccount
    case failed(PasskeyCallResult)

    init(_ result: NativeAccountBindResult) {
        switch (result.outcome, result.bindingState) {
        case ("bound", "bound"): self = .bound
        case ("existing_account", "bound"), ("existing_account", "legacy"): self = .existingAccount
        default: self = .failed(.refused(label: "account-bind-invalid"))
        }
    }
}

/// The account calls behind the sheets. `LivePasskeyAccount` is the real
/// one; tests record.
@MainActor
protocol PasskeyAccount: AnyObject {
    /// Raise the system sheet for `action` (P-3 and P-4 for create, P-6 for
    /// login) and complete the ceremony with the daemon.
    func ceremony(_ action: NativePasskeyAction, label: String?) async -> PasskeyCallResult
    /// P-5: bind the passkey to the near.ai account (`account_bind`).
    func bind() async -> PasskeyBindResult
    /// Cancelling P-5 signs out (`account_sign_out`).
    func signOut() async -> PasskeyCallResult
    /// Dismiss a system sheet that is still up.
    func cancel()
}

/// The live account: the native passkey coordinator for the system sheets
/// and the identity transport for binding and signing out.
@MainActor
final class LivePasskeyAccount: PasskeyAccount {
    private let coordinator: NativePasskeyCoordinator
    private let transport: NativeIdentityTransport

    init(client: DaemonClient, presentationAnchor: @escaping @MainActor () -> NSWindow?) {
        self.coordinator = client.nativePasskeyCoordinator(presentationAnchor: presentationAnchor)
        self.transport = NativeIdentityTransport(client: client)
    }

    func ceremony(_ action: NativePasskeyAction, label: String?) async -> PasskeyCallResult {
        do {
            _ = try await coordinator.perform(action, label: label)
            return .done
        } catch {
            return PasskeyCallResult(error: error)
        }
    }

    func bind() async -> PasskeyBindResult {
        do {
            return PasskeyBindResult(try await transport.connectNearAI())
        } catch {
            return .failed(PasskeyCallResult(error: error))
        }
    }

    func signOut() async -> PasskeyCallResult {
        do {
            try await transport.signOut()
            return .done
        } catch {
            return PasskeyCallResult(error: error)
        }
    }

    func cancel() {
        coordinator.cancel()
    }
}

/// Ron's `passkeyNameError`, worded by the core.
enum PasskeyName {
    /// Ron's `PASSKEY_NAME_MAX`, and the daemon's own label limit, so a
    /// name the sheet accepts is one the daemon accepts.
    static let maxLength = 64

    static func trimmed(_ name: String) -> String {
        name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    static func error(_ name: String, copy: FirstRunCopy.Passkey) -> String? {
        let trimmed = trimmed(name)
        if trimmed.isEmpty { return copy.nameEmpty }
        // The daemon counts Unicode scalars; a grapheme count would pass a
        // name the daemon then refuses as invalid.
        if trimmed.unicodeScalars.count > maxLength {
            return copy.nameTooLong.replacingOccurrences(of: "{max}", with: "\(maxLength)")
        }
        return nil
    }
}

/// Ron's `passkeyTransition`, with the system sheets as ceremony results:
/// a dismissed system sheet steps back and says nothing, any other failure
/// stays on the step with its label, and cancelling Verify signs out.
@MainActor
final class PasskeySheetModel: ObservableObject {
    @Published private(set) var step: PasskeySheetStep
    @Published var name: String
    /// The name error shows only once the field was edited or submitted.
    @Published var nameTouched = false
    @Published private(set) var busy = false
    @Published private(set) var refusal: String?
    @Published private(set) var outcome: PasskeySheetOutcome?

    private let copy: FirstRunCopy.Passkey
    private let account: any PasskeyAccount
    /// The trimmed name the created passkey carries.
    private var createdName: String?

    init(start: PasskeySheetStep = .choose, copy: FirstRunCopy.Passkey, account: any PasskeyAccount) {
        self.step = start
        self.copy = copy
        self.account = account
        self.name = copy.defaultName
    }

    var nameError: String? {
        nameTouched ? PasskeyName.error(name, copy: copy) : nil
    }

    /// P-1 Create new.
    func createNew() {
        guard !busy, step == .choose else { return }
        move(to: .name)
    }

    /// P-2 Back.
    func back() {
        guard !busy, step == .name else { return }
        move(to: .choose)
    }

    /// Close or Escape on P-1, P-2 and P-7; Other sign-in options on P-7.
    /// Verify is not closed this way: leaving it signs out.
    func close() {
        guard !busy, step != .verify else { return }
        outcome = .closed
    }

    /// P-1 Use existing and P-7 Sign in: the system sign-in sheet.
    func useExisting() async {
        guard !busy, step == .choose || step == .welcomeBack else { return }
        switch await run({ await $0.ceremony(.login, label: nil) }) {
        case .done: outcome = .signedIn
        case .cancelled: move(to: .choose)
        case .refused: break
        }
    }

    /// P-2 Create: the system save sheets, then Verify.
    func submitName() async {
        guard !busy, step == .name else { return }
        nameTouched = true
        guard PasskeyName.error(name, copy: copy) == nil else { return }
        let label = PasskeyName.trimmed(name)
        switch await run({ await $0.ceremony(.create, label: label) }) {
        case .done:
            createdName = label
            move(to: .verify)
        case .cancelled: move(to: .name)
        case .refused: break
        }
    }

    /// P-5 Verify.
    func verify() async {
        guard !busy, step == .verify, let createdName else { return }
        busy = true
        refusal = nil
        defer { busy = false }
        switch await account.bind() {
        case .bound: outcome = .created(name: createdName)
        case .existingAccount: outcome = .existingAccount
        case .failed(.refused(let label)): refusal = label
        case .failed: break
        }
    }

    /// The sheet left the screen: a system sheet still up is dismissed, so
    /// the coordinator is not left running.
    func disappeared() {
        guard busy else { return }
        account.cancel()
    }

    /// P-5 Cancel. Signed out only once the daemon says so.
    func cancelVerify() async {
        guard !busy, step == .verify else { return }
        if case .done = await run({ await $0.signOut() }) {
            outcome = .signedOut
        }
    }

    private func move(to next: PasskeySheetStep) {
        refusal = nil
        step = next
    }

    private func run(_ call: (any PasskeyAccount) async -> PasskeyCallResult) async -> PasskeyCallResult {
        busy = true
        refusal = nil
        defer { busy = false }
        let result = await call(account)
        if case .refused(let label) = result { refusal = label }
        return result
    }
}

/// The current passkey sheet. `onFinish` gets the outcome once; Join maps
/// it with `PasskeySheetOutcome.joinNotice(_:)`.
struct PasskeySheets: View {
    let copy: FirstRunCopy
    @ObservedObject var model: PasskeySheetModel
    /// The returning passkey's name for P-7, when Join knows it.
    let returningName: String?
    let onFinish: (PasskeySheetOutcome) -> Void

    var body: some View {
        content
            .frame(width: 360)
            .onExitCommand { escape() }
            .onDisappear { model.disappeared() }
            .onChange(of: model.outcome) { _, outcome in
                if let outcome { onFinish(outcome) }
            }
    }

    @ViewBuilder
    private var content: some View {
        switch model.step {
        case .choose: choose
        case .name: nameSheet
        case .verify: verify
        case .welcomeBack: welcomeBack
        }
    }

    /// P-1.
    private var choose: some View {
        GlassSheet(title: copy.passkey.chooseTitle) {
            corner(back: false)
            Button(copy.passkey.useExisting) { Task { await model.useExisting() } }
                .buttonStyle(GlassButtonStyle(.primary))
                .frame(maxWidth: .infinity)
            Button(copy.passkey.createNew) { model.createNew() }
                .buttonStyle(GlassButtonStyle(.glass))
                .frame(maxWidth: .infinity)
            refusalNotice
            caption(copy.passkey.chooseNote)
        }
        .disabled(model.busy)
    }

    /// P-2.
    private var nameSheet: some View {
        GlassSheet(title: copy.passkey.nameTitle) {
            corner(back: true)
            HStack(alignment: .bottom, spacing: GlassTokens.Space.s3) {
                GlassTextField(
                    copy.passkey.nameField,
                    text: Binding(
                        get: { model.name },
                        set: {
                            model.name = $0
                            model.nameTouched = true
                        }))
                    .onSubmit { Task { await model.submitName() } }
                if !model.name.isEmpty {
                    Button {
                        model.name = ""
                        model.nameTouched = true
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(copy.passkey.clearName)
                }
            }
            if let error = model.nameError {
                GlassNotice(tone: .outside) { Text(error) }
            }
            refusalNotice
            Button(copy.passkey.nameTitle) { Task { await model.submitName() } }
                .buttonStyle(GlassButtonStyle(.primary))
                .frame(maxWidth: .infinity)
                .disabled(model.nameError != nil)
            GlassNotice(tone: .ask) { Text(copy.passkey.nameWarning) }
        }
        .disabled(model.busy)
    }

    /// P-5.
    private var verify: some View {
        GlassSheet(title: copy.passkey.verifyTitle, subtitle: copy.passkey.verifyBody) {
            refusalNotice
            Button {
                Task { await model.verify() }
            } label: {
                HStack(spacing: GlassTokens.Space.s3) {
                    if model.busy { ProgressView().controlSize(.small) }
                    Text(copy.passkey.verify)
                }
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .frame(maxWidth: .infinity)
            Button(copy.passkey.cancel) { Task { await model.cancelVerify() } }
                .buttonStyle(GlassButtonStyle(.glass))
                .frame(maxWidth: .infinity)
            caption(copy.passkey.verifyNote)
        }
        .disabled(model.busy)
    }

    /// P-7.
    private var welcomeBack: some View {
        GlassSheet(title: copy.passkey.welcomeTitle, subtitle: copy.passkey.welcomeBody) {
            GlassCard {
                VStack(spacing: GlassTokens.Space.s6) {
                    Image(systemName: "person.badge.key.fill")
                        .foregroundStyle(GlassColor.accentText)
                    if let returningName {
                        Text(returningName)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                    }
                    Button(copy.passkey.welcomeSignIn) { Task { await model.useExisting() } }
                        .buttonStyle(GlassButtonStyle(.primary))
                }
                .frame(maxWidth: .infinity)
            }
            refusalNotice
            Button(copy.passkey.otherOptions) { model.close() }
                .buttonStyle(GlassButtonStyle(.link))
                .frame(maxWidth: .infinity)
        }
        .disabled(model.busy)
    }

    /// Back on the left (P-2 only) and Close on the right, as Ron's corners.
    private func corner(back: Bool) -> some View {
        HStack {
            if back {
                Button(copy.passkey.back) { model.back() }
                    .buttonStyle(GlassButtonStyle(.link))
            }
            Spacer(minLength: 0)
            Button(copy.passkey.close) { model.close() }
                .buttonStyle(GlassButtonStyle(.link))
        }
    }

    @ViewBuilder
    private var refusalNotice: some View {
        if let refusal = model.refusal {
            GlassNotice(tone: .outside) { Text(refusal) }
        }
    }

    private func caption(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textTertiary)
            .multilineTextAlignment(.center)
            .frame(maxWidth: .infinity)
    }

    /// Escape: Ron's `otherwise` row. Verify signs out; elsewhere it closes.
    private func escape() {
        guard !model.busy else { return }
        if model.step == .verify {
            Task { await model.cancelVerify() }
        } else {
            model.close()
        }
    }
}

/// Presents the passkey sheets whenever the runner asks for them
/// (`FirstRunRunner.passkeyDue`): after the commit that started the daemon
/// for a passkey chosen on Join, or from Create passkey once it runs.
///
/// Mount it exactly once per runner: each mount holds its own model, so two
/// mounts would present two sheets. It belongs on the first-run host, which
/// is on screen at every step; until that host exists (port plan Task 11)
/// it is mounted on Join, so a request the Folders or Tools commit raises
/// presents only when Join is on screen again. Task 11 moves the mount to
/// the coordinator and drops Join's. Without an account path the request
/// stays raised, never dropped.
private struct FirstRunPasskeyPresenter: ViewModifier {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    let account: (any PasskeyAccount)?

    @State private var model: PasskeySheetModel?

    func body(content: Content) -> some View {
        content
            .sheet(isPresented: presented) {
                if let model {
                    PasskeySheets(copy: copy, model: model, returningName: nil) { outcome in
                        runner.finishPasskey(outcome, copy: copy)
                        self.model = nil
                    }
                }
            }
            .onAppear(perform: open)
            .onChange(of: runner.passkeyDue) { _, _ in open() }
    }

    /// A fresh model each time: a model's outcome is set once, so a reused
    /// one would leave the second presentation unable to finish.
    private func open() {
        guard runner.passkeyDue, model == nil, let account else { return }
        model = PasskeySheetModel(copy: copy.passkey, account: account)
    }

    /// Dismissed without an outcome: closed.
    private var presented: Binding<Bool> {
        Binding(
            get: { model != nil },
            set: {
                guard !$0, model != nil else { return }
                model = nil
                if runner.passkeyDue { runner.finishPasskey(.closed, copy: copy) }
            })
    }
}

extension View {
    /// Mount the first run's passkey sheets (`FirstRunPasskeyPresenter`).
    func firstRunPasskeySheets(
        copy: FirstRunCopy, runner: FirstRunRunner, account: (any PasskeyAccount)?
    ) -> some View {
        modifier(FirstRunPasskeyPresenter(copy: copy, runner: runner, account: account))
    }
}
