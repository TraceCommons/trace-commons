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
    /// An existing passkey signed in, and Verify enrolled this Mac: either
    /// the account was unbound and Verify bound it (`bound`), or another Mac
    /// had bound it and Verify joined this one to it (`enrolled`). A sign-in
    /// alone holds no enrolment and never ends the sheets with this.
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
/// coordinator's label, never a sentence written here; the sheet shows the
/// core's `passkey.refused` for it (`PasskeySheets.refusalLine`).
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
    /// A bound account's Verify joined this Mac to it (`enrolled`).
    case enrolled
    /// The commons refused to join this Mac to the bound account: this Mac's
    /// near.ai sign-in is not the one the account uses
    /// (`account-enrol-mismatch`). Nothing was added.
    case nearAiMismatch
    case failed(PasskeyCallResult)

    /// The daemon's label for `nearAiMismatch`.
    static let nearAiMismatchLabel = "account-enrol-mismatch"

    init(_ result: NativeAccountBindResult) {
        switch (result.outcome, result.bindingState) {
        case ("bound", "bound"): self = .bound
        case ("existing_account", "bound"), ("existing_account", "legacy"): self = .existingAccount
        case ("enrolled", "bound"): self = .enrolled
        default: self = .failed(.refused(label: "account-bind-invalid"))
        }
    }

    /// A failed `account_bind`. The mismatch is told apart by the daemon's
    /// label itself, never by a view sentence that may replace it.
    init(error: any Error) {
        if let failure = error as? DaemonClient.Failure, failure.message == Self.nearAiMismatchLabel {
            self = .nearAiMismatch
        } else {
            self = .failed(PasskeyCallResult(error: error))
        }
    }
}

/// Why the sheet signed a passkey session out and is back on Choose. Each is
/// one of the core's lines; Swift writes neither.
enum PasskeyNotice: Equatable {
    /// "Use existing passkey" reached a legacy account (`passkey.bound_elsewhere`).
    case boundElsewhere
    /// Verify's join was refused: this Mac's near.ai sign-in is not the
    /// account's (`passkey.near_ai_mismatch`).
    case nearAiMismatch
}

/// What Verify is for.
enum PasskeyVerifyKind: Equatable {
    /// After Create: bind the new passkey's unbound account.
    case created
    /// After a sign-in to an account no Mac has bound: bind it.
    case signedInUnbound
    /// After a sign-in to an account another Mac bound: join this Mac to it.
    case signedInBound
}

/// What "Use existing passkey" answered: the daemon's `binding_state` for
/// the account it signed in to, or why it did not sign in.
enum PasskeySignInResult: Equatable {
    /// No Mac has bound the account yet: Verify binds it and enrols this one.
    case unbound
    /// Another Mac bound it: Verify joins this Mac to it, which the commons
    /// allows only when this Mac's near.ai sign-in is the account's own.
    case bound
    /// Not created with a passkey, so it has no binding a join could check.
    /// Adding a Mac to it with a passkey is not built, so it fails closed.
    case legacy
    /// A `binding_state` this build does not know. Fails closed.
    case unrecognised
    case failed(PasskeyCallResult)

    init(bindingState: String) {
        switch bindingState {
        case "unbound": self = .unbound
        case "bound": self = .bound
        case "legacy": self = .legacy
        default: self = .unrecognised
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
    /// P-6: the system sign-in sheet, completed with the daemon, answering
    /// the signed-in account's `binding_state`.
    func signIn() async -> PasskeySignInResult
    /// P-5: bind the passkey to the near.ai account (`account_bind`).
    func bind() async -> PasskeyBindResult
    /// Cancelling P-5 signs out (`account_sign_out`).
    func signOut() async -> PasskeyCallResult
    /// Dismiss a system sheet that is still up.
    func cancel()
    /// `passkey_state`, with the passkeys this Mac remembers, or nil when
    /// the daemon did not answer. A returning person opens at P-7
    /// (`FirstRunRunner.offerWelcomeBack`).
    func passkeyState() async -> NativePasskeyState?
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

    func signIn() async -> PasskeySignInResult {
        do {
            return PasskeySignInResult(bindingState: try await coordinator.perform(.login).bindingState)
        } catch {
            return .failed(PasskeyCallResult(error: error))
        }
    }

    func bind() async -> PasskeyBindResult {
        do {
            return PasskeyBindResult(try await transport.connectNearAI())
        } catch {
            return PasskeyBindResult(error: error)
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

    func passkeyState() async -> NativePasskeyState? {
        try? await transport.passkeys()
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
            return FirstRunCopy.fill(copy.nameTooLong, ["max": "\(maxLength)"])
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
    /// Why the sheet signed out and is back on Choose, once the daemon
    /// confirmed the sign-out (`PasskeySheets.noticeLine`).
    @Published private(set) var notice: PasskeyNotice?

    private let copy: FirstRunCopy.Passkey
    private let account: any PasskeyAccount
    /// The trimmed name the created passkey carries.
    private var createdName: String?
    /// What Verify is for, which decides the one answer it accepts.
    private var verifying: PasskeyVerifyKind?

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

    /// P-1 Use existing and P-7 Sign in: the system sign-in sheet. A
    /// sign-in holds no enrolment, so it never ends the sheets by itself:
    /// an account no Mac has bound goes to Verify, whose bind enrols this
    /// Mac; one another Mac bound goes to Verify too, which joins this Mac to
    /// it; a legacy one is signed out again, fail closed, since adding a Mac
    /// to it with a passkey is not built.
    func useExisting() async {
        guard !busy, step == .choose || step == .welcomeBack else { return }
        busy = true
        refusal = nil
        notice = nil
        defer { busy = false }
        switch await account.signIn() {
        case .unbound:
            verifying = .signedInUnbound
            move(to: .verify)
        case .bound:
            verifying = .signedInBound
            move(to: .verify)
        case .legacy:
            await signOut(saying: .boundElsewhere)
        case .unrecognised:
            await signOut(saying: nil)
        case .failed(.cancelled):
            move(to: .choose)
        case .failed(.refused(let label)):
            refusal = label
        case .failed(.done):
            refusal = "account-sign-in-invalid"
        }
    }

    /// The daemon holds a session this Mac cannot enrol under: sign it out
    /// and stay on Choose. The notice is shown only once the sign-out is
    /// confirmed (it says "you were signed out"); otherwise the refusal is.
    /// No notice means a binding state this build does not know.
    private func signOut(saying said: PasskeyNotice?) async {
        let signedOut = await account.signOut()
        verifying = nil
        createdName = nil
        move(to: .choose)
        switch signedOut {
        case .done:
            if let said { notice = said } else { refusal = "account-binding-unrecognised" }
        case .refused(let label): refusal = label
        case .cancelled: refusal = "account-sign-out-failed"
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
            verifying = .created
            move(to: .verify)
        case .cancelled: move(to: .name)
        case .refused: break
        }
    }

    /// P-5 Verify, after Create, after a sign-in to an unbound account, or
    /// after a sign-in to an account another Mac bound. Each accepts only its
    /// own answer: `bound` (or `existing_account`) binds an unbound account,
    /// `enrolled` joins a bound one, and anything else fails closed.
    func verify() async {
        guard !busy, step == .verify, let verifying else { return }
        busy = true
        refusal = nil
        defer { busy = false }
        switch (verifying, await account.bind()) {
        case (.created, .bound): outcome = createdName.map { .created(name: $0) } ?? .signedIn
        case (.signedInUnbound, .bound): outcome = .signedIn
        case (.created, .existingAccount), (.signedInUnbound, .existingAccount): outcome = .existingAccount
        case (.signedInBound, .enrolled): outcome = .signedIn
        case (_, .nearAiMismatch): await signOut(saying: .nearAiMismatch)
        case (_, .bound), (_, .existingAccount), (_, .enrolled): refusal = "account-bind-invalid"
        case (_, .failed(.refused(let label))): refusal = label
        case (_, .failed): break
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
        notice = nil
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
    /// The remembered passkey's name for P-7, when the daemon has one. The
    /// only thing about the account P-7 shows.
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
        popup(.choose, title: copy.passkey.chooseTitle) {
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
        popup(.name, title: copy.passkey.nameTitle) {
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

    /// P-5. Verify is Ron's outlined button.
    private var verify: some View {
        popup(.verify, title: copy.passkey.verifyTitle, subtitle: copy.passkey.verifyBody) {
            refusalNotice
            Button {
                Task { await model.verify() }
            } label: {
                HStack(spacing: GlassTokens.Space.s3) {
                    if model.busy { GlassSpinner() }
                    Text(copy.passkey.verify)
                }
                .frame(maxWidth: .infinity)
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .passkeyOutlined()
            Button(copy.passkey.cancel) { Task { await model.cancelVerify() } }
                .buttonStyle(GlassButtonStyle(.glass))
                .frame(maxWidth: .infinity)
            caption(copy.passkey.verifyNote)
        }
        .disabled(model.busy)
    }

    /// P-7: a display-size title, then the card with the tinted circle,
    /// the passkey's name when known, and Sign in.
    private var welcomeBack: some View {
        popup(.welcomeBack, title: copy.passkey.welcomeTitle, subtitle: copy.passkey.welcomeBody, display: true) {
            GlassCard {
                VStack(spacing: GlassTokens.Space.s7) {
                    PasskeyPopupIcon(.touch)
                    if let returningName {
                        Text(returningName)
                            .glassType(GlassTokens.TypeScale.heading)
                            .foregroundStyle(GlassColor.textPrimary)
                    }
                    Button(copy.passkey.welcomeSignIn) { Task { await model.useExisting() } }
                        .buttonStyle(GlassButtonStyle(.primary))
                        .frame(maxWidth: .infinity)
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

    /// One popup in Ron's shape for `step`: its icon, centred heading, and
    /// the round Back and Close in its corners.
    private func popup<Body: View>(
        _ step: PasskeySheetStep, title: String, subtitle: String? = nil, display: Bool = false,
        @ViewBuilder body: () -> Body
    ) -> some View {
        let corners = PasskeyPopupLayout.corners(step)
        return PasskeyPopup(
            icon: PasskeyPopupLayout.icon(step), title: title, subtitle: subtitle, display: display,
            backLabel: copy.passkey.back, onBack: corners.back ? { model.back() } : nil,
            closeLabel: copy.passkey.close, onClose: corners.close ? { model.close() } : nil,
            content: body)
    }

    /// The core's sentence for a refused ceremony. The model keeps the
    /// daemon's or the coordinator's label; it is never shown.
    static func refusalLine(_ refusal: String?, copy: FirstRunCopy.Passkey) -> String? {
        refusal == nil ? nil : copy.refused
    }

    /// The sheet's notice: the core's line for why the sheet signed out,
    /// once that sign-out is confirmed, else the refusal's sentence, else
    /// nothing.
    static func noticeLine(_ model: PasskeySheetModel, copy: FirstRunCopy.Passkey) -> String? {
        switch model.notice {
        case .boundElsewhere: copy.boundElsewhere
        case .nearAiMismatch: copy.nearAiMismatch
        case nil: refusalLine(model.refusal, copy: copy)
        }
    }

    @ViewBuilder
    private var refusalNotice: some View {
        if let line = PasskeySheets.noticeLine(model, copy: copy.passkey) {
            GlassNotice(tone: .outside) { Text(line) }
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
/// mounts would present two sheets. It is mounted on the first-run host
/// (`OnboardingCoordinatorView`), which is on screen at every step, so a
/// request the Folders or Tools commit raises presents on the step that
/// follows. Without an account path the request stays raised, never
/// dropped.
private struct FirstRunPasskeyPresenter: ViewModifier {
    let copy: FirstRunCopy
    @ObservedObject var runner: FirstRunRunner
    let account: (any PasskeyAccount)?

    @State private var model: PasskeySheetModel?

    func body(content: Content) -> some View {
        content
            // Over the whole first-run window, on the modal scrim; the
            // sheets' own corners and Escape close them.
            .glassModal(isPresented: presented) {
                if let model {
                    PasskeySheets(copy: copy, model: model, returningName: runner.returningName) { outcome in
                        runner.finishPasskey(outcome, copy: copy)
                        self.model = nil
                    }
                }
            }
            .onAppear(perform: open)
            .onChange(of: runner.passkeyDue) { _, _ in open() }
    }

    /// A fresh model each time: a model's outcome is set once, so a reused
    /// one would leave the second presentation unable to finish. It opens
    /// at the runner's step: P-1 from Join, P-7 for a returning person.
    private func open() {
        guard runner.passkeyDue, model == nil, let account else { return }
        model = PasskeySheetModel(start: runner.passkeyStart, copy: copy.passkey, account: account)
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

/// Ron's passkey popups' shape (#1030 `passkey-flow.tsx`, review of #1235
/// item 14), apart from the view so it can be tested.
enum PasskeyPopupLayout {
    /// The round tinted icon at a popup's head.
    enum Icon: Equatable {
        /// P-1 and P-2.
        case passkey
        /// P-5.
        case lock
        /// P-7's circle, inside its card, tinted on.
        case touch
    }

    struct Corners: Equatable {
        let back: Bool
        let close: Bool
    }

    /// The icon at the head of a popup; P-7 carries its circle in its card.
    static func icon(_ step: PasskeySheetStep) -> Icon? {
        switch step {
        case .choose, .name: return .passkey
        case .verify: return .lock
        case .welcomeBack: return nil
        }
    }

    /// Ron's corner buttons: Close on P-1, Back and Close on P-2; P-5 has
    /// its own Cancel, which signs out, and P-7 its "Other sign-in options".
    static func corners(_ step: PasskeySheetStep) -> Corners {
        switch step {
        case .choose: return Corners(back: false, close: true)
        case .name: return Corners(back: true, close: true)
        case .verify, .welcomeBack: return Corners(back: false, close: false)
        }
    }

    static func symbol(_ icon: Icon) -> String {
        switch icon {
        case .passkey: return "person.badge.key"
        case .lock: return "lock"
        case .touch: return "touchid"
        }
    }
}

/// Ron's `ftux-popup-icon`: a 52pt circle, accent-tinted with the purple
/// glyph, or on-tinted with the on glyph for P-7.
struct PasskeyPopupIcon: View {
    private let icon: PasskeyPopupLayout.Icon

    init(_ icon: PasskeyPopupLayout.Icon) {
        self.icon = icon
    }

    var body: some View {
        let on = icon == .touch
        Image(systemName: PasskeyPopupLayout.symbol(icon))
            .glassGlyph(24)
            .foregroundStyle((on ? GlassTokens.Color.statusOn : GlassTokens.Color.purpleText).color)
            .frame(width: 52, height: 52)
            .background(Circle().fill((on ? GlassTokens.Color.tintOn : GlassTokens.Color.tintAccent).color))
            .accessibilityHidden(true)
    }
}

/// Ron's popup (`tc-modal tc-modal--narrow ftux-popup`): the icon, a
/// centred heading, the content, and round Back and Close buttons in its
/// corners.
struct PasskeyPopup<Content: View>: View {
    let icon: PasskeyPopupLayout.Icon?
    let title: String
    let subtitle: String?
    let display: Bool
    let backLabel: String
    let onBack: (() -> Void)?
    let closeLabel: String
    let onClose: (() -> Void)?
    let content: Content

    init(
        icon: PasskeyPopupLayout.Icon?, title: String, subtitle: String?, display: Bool,
        backLabel: String, onBack: (() -> Void)?, closeLabel: String, onClose: (() -> Void)?,
        @ViewBuilder content: () -> Content
    ) {
        self.icon = icon
        self.title = title
        self.subtitle = subtitle
        self.display = display
        self.backLabel = backLabel
        self.onBack = onBack
        self.closeLabel = closeLabel
        self.onClose = onClose
        self.content = content()
    }

    var body: some View {
        VStack(spacing: GlassTokens.Space.s7) {
            if let icon { PasskeyPopupIcon(icon) }
            VStack(spacing: GlassTokens.Space.s3) {
                Text(title)
                    .glassType(display ? GlassTokens.TypeScale.display : GlassTokens.TypeScale.heading)
                    .foregroundStyle(GlassColor.textPrimary)
                    .multilineTextAlignment(.center)
                    .accessibilityAddTraits(.isHeader)
                if let subtitle {
                    Text(subtitle)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                        .multilineTextAlignment(.center)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            .frame(maxWidth: .infinity)
            content
        }
        .padding(.horizontal, GlassTokens.Space.panePadding)
        .padding(.top, GlassTokens.Space.panePadding + (onBack != nil || onClose != nil ? GlassTokens.Space.s4 : 0))
        .padding(.bottom, GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity)
        .overlay(alignment: .topLeading) {
            if let onBack {
                GlassRoundButton(backLabel, systemImage: "chevron.left", small: true, action: onBack)
                    .padding(GlassTokens.Space.s7)
            }
        }
        .overlay(alignment: .topTrailing) {
            if let onClose {
                GlassRoundButton(closeLabel, systemImage: "xmark", small: true, action: onClose)
                    .padding(GlassTokens.Space.s7)
            }
        }
        .glassTier(.pane)
    }
}

extension View {
    /// Ron's outlined block button (`ftux-btn-outline`): full width, a
    /// 1.5pt inner ring and a soft 3pt halo, both in the overlay ink.
    func passkeyOutlined() -> some View {
        frame(maxWidth: .infinity)
            .overlay(Capsule().strokeBorder(GlassColor.ink(0.7), lineWidth: 1.5))
            .background(Capsule().inset(by: -3).strokeBorder(GlassColor.ink(0.12), lineWidth: 3))
    }
}
