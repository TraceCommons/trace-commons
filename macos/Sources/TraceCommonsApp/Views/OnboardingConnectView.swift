import SwiftUI
import TCDesign

/// Onboarding screen 2, "Connect" -- an invite paste field plus a
/// `tracecommons://enroll?...` deep-link handler, so an invite clicked in
/// mail lands here. Copy and rules are from the shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Onboarding", "### 2. Connect"), not paraphrased.
///
/// "Resolve and show the instance before committing" is done entirely
/// client-side: an invite link is `https://issuer.example/onboard#CODE` (see
/// `parse_invite` in `crates/trace-commons-contributor/src/commands.rs`), so
/// the issuer host is visible the moment the link is parsed, with no network
/// round trip and no separate daemon method. Only the `enroll` call itself
/// -- fired once the person taps to actually join -- touches the network.
///
/// The daemon's `enroll` never echoes the underlying HTTP condition back
/// over the socket; any failure it reports is the generic `unavailable` /
/// `enroll-failed` (see "### `enroll`" in
/// `docs/contributor-daemon-ipc-v1_1.md`). So this view has exactly one
/// failure sentence for the whole invite path, and shows it verbatim
/// regardless of what the daemon actually said -- surfacing the raw code
/// would tell an onlooker more than it would tell the contributor.
struct OnboardingConnectView: View {
    @EnvironmentObject private var model: AppModel
    var onEnrolled: () -> Void

    /// The one scroll for this step, in the wrapper as on Welcome and
    /// Folders, so every host scrolls exactly once.
    var body: some View {
        ScrollView {
            OnboardingConnectContent(onEnrolled: onEnrolled)
                .environmentObject(model)
        }
    }
}

/// The screen's layout, split out of its `ScrollView` for the same reason
/// `ConsentScopesContent` is split out of `ConsentScopesView`: `ImageRenderer`
/// renders a `ScrollView` as blank.
struct OnboardingConnectContent: View {
    @EnvironmentObject private var model: AppModel

    @State private var nearBusy = false

    enum Phase: Equatable {
        case idle
        case resolved(InviteLink)
        case enrolling(InviteLink)
        /// Covers both an invite string this app cannot even parse and one
        /// the daemon's `enroll` refused -- the fixed sentence in
        /// `OnboardingConnectWords` is the only thing either case may say.
        case deadInvite
    }

    @State private var inviteText: String
    @State private var phase: Phase
    /// An invite that arrived by URL before this screen existed. See
    /// `PendingInvite`.
    @ObservedObject private var pendingInvite = PendingInvite.shared

    var onEnrolled: () -> Void

    /// `previewPhase`/`previewText` exist so a screenshot (or a preview
    /// canvas) can render any state of this screen without driving a real
    /// daemon call -- the same accommodation `PreviewSheet.Preloaded` makes
    /// for its screen.
    init(onEnrolled: @escaping () -> Void, previewPhase: Phase = .idle, previewText: String = "") {
        self.onEnrolled = onEnrolled
        _phase = State(initialValue: previewPhase)
        _inviteText = State(initialValue: previewText)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            header
            if model.status.loggedIn {
                alreadyConnected
            } else {
                pasteField
                phaseView
                // The way in that needs no wallet, above the wallet ceremony
                // rather than below it. Both are offered and neither replaces
                // the other; this one is first because it is the shorter road
                // for a contributor who already has the NEAR AI account the
                // receipts come from anyway.
                Divider()
                NearAiJoinView(onEnrolled: onEnrolled)
                    .disabled(isEnrolling)
                Divider()
                NearAccountConnectView(onBusyChanged: { nearBusy = $0 }, onEnrolled: onEnrolled)
                    .disabled(isEnrolling)
            }
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Not `onOpenURL`. That fires only on a mounted view, and the app's
        // resting state is running with no window at all -- so a link clicked
        // by a contributor who already has the app running used to land on
        // nothing. `AppDelegate.application(_:open:)` receives it above the
        // view layer and parks it in `PendingInvite`; this screen collects it
        // whenever it next exists.
        .onAppear { consumePendingInvite() }
        // And again while already on screen, which `onAppear` would miss --
        // the window can be sitting open on this very screen when the link
        // arrives.
        .onChange(of: pendingInvite.value) { _, _ in consumePendingInvite() }
    }

    /// Takes the parked invite, if there is one, and shows what it resolves
    /// to. It deliberately stops there: filling the field and naming the
    /// issuer is as far as a link may go, because which commons to enrol
    /// with is the decision this screen exists to ask. Both other clients
    /// say the same thing at their own registration sites.
    private func consumePendingInvite() {
        guard let invite = pendingInvite.take() else { return }
        inviteText = invite
        resolve()
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(OnboardingConnectWords.heading)
                .glassType(GlassTokens.TypeScale.heading)
                .foregroundStyle(GlassColor.textPrimary)
                .accessibilityAddTraits(.isHeader)
            Text(OnboardingConnectWords.pasteTheLink)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var pasteField: some View {
        HStack(alignment: .bottom, spacing: GlassTokens.Space.s4) {
            GlassTextField(OnboardingConnectWords.linkPrompt, text: $inviteText)
                .onSubmit(resolve)
                .disabled(isEnrolling || nearBusy)
            Button(OnboardingConnectWords.lookUp, action: resolve)
                .buttonStyle(GlassButtonStyle(.glass))
                .disabled(inviteText.trimmingCharacters(in: .whitespaces).isEmpty || isEnrolling || nearBusy)
        }
    }

    private var alreadyConnected: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            GlassStatusLabel(OnboardingConnectWords.alreadyConnected, status: .on)
            Button(OnboardingConnectWords.continueButton, action: onEnrolled)
                .buttonStyle(GlassButtonStyle(.primary))
                .keyboardShortcut(.defaultAction)
        }
    }

    private var isEnrolling: Bool {
        if case .enrolling = phase { return true }
        return false
    }

    @ViewBuilder
    private var phaseView: some View {
        switch phase {
        case .idle:
            EmptyView()
        case .resolved(let link):
            resolvedView(link)
        case .enrolling(let link):
            HStack(spacing: GlassTokens.Space.s3) {
                ProgressView().controlSize(.small)
                Text(OnboardingConnectWords.connectingTo(link.issuerHost))
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        case .deadInvite:
            // Refused is a status in this app, and it is never colour alone.
            GlassStatusLabel(OnboardingConnectWords.deadInvite, status: .outside)
        }
    }

    private func resolvedView(_ link: InviteLink) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            // The one thing a contributor must know before this device
            // enrolls: whose commons it is about to join.
            Text(OnboardingConnectWords.inviteIsFor(link.issuerHost))
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
            Button(OnboardingConnectWords.join(link.issuerHost)) { join(link) }
                .buttonStyle(GlassButtonStyle(.primary))
                .disabled(nearBusy)
                .keyboardShortcut(.defaultAction)
        }
    }

    private func resolve() {
        guard let link = InviteLink.parse(inviteText) else {
            phase = .deadInvite
            return
        }
        phase = .resolved(link)
    }

    private func join(_ link: InviteLink) {
        phase = .enrolling(link)
        Task {
            switch await model.enroll(invite: link.raw) {
            case .succeeded:
                onEnrolled()
            case .failed:
                // Never the daemon's actual error code -- see the type
                // comment above.
                phase = .deadInvite
            }
        }
    }
}

/// This screen's sentences, moved here unchanged from the view body.
enum OnboardingConnectWords {
    static let heading = "Connect to a commons"
    static let pasteTheLink = "Paste the invite link someone sent you, or click it from your email."
    static let deadInvite = "This invite link is no longer valid. Ask whoever sent it for a new one."
    static let alreadyConnected = "This device is already connected."
    /// The sentence with the issuer host emphasised: the one thing to know
    /// before enrolling. The host stays inside the sentence, so no fragment
    /// is authored separately.
    static func inviteIsFor(_ host: String) -> AttributedString {
        var sentence = AttributedString("This invite is for \(host).")
        if let range = sentence.range(of: host, options: .backwards) {
            sentence[range].inlinePresentationIntent = .stronglyEmphasized
        }
        return sentence
    }
    static func connectingTo(_ host: String) -> String { "Connecting to \(host)…" }
    static func join(_ host: String) -> String { "Join \(host)" }
    static let linkPrompt = "https://…/onboard#…"
    static let lookUp = "Look up"
    static let continueButton = "Continue"
}

// MARK: - Invite parsing

/// An invite link, parsed just enough to show which instance it is for
/// before committing to it. Mirrors `parse_invite` in
/// `crates/trace-commons-contributor/src/commands.rs`: the code may be the
/// URL fragment (`#CODE`) or a `code` query parameter, and everything before
/// that is the issuer origin this device would be registering with.
struct InviteLink: Equatable {
    /// The full invite string, passed to `enroll` verbatim -- the daemon
    /// does its own parsing and this app must not reconstruct or normalize
    /// it.
    let raw: String
    /// Scheme + host, e.g. "issuer.example.ai" -- shown to the contributor,
    /// never the full URL with its code attached.
    let issuerHost: String

    static func parse(_ text: String) -> InviteLink? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty,
              let url = URL(string: trimmed),
              let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              let host = url.host, !host.isEmpty
        else { return nil }

        let hasFragmentCode = !(url.fragment ?? "").isEmpty
        let hasQueryCode = URLComponents(url: url, resolvingAgainstBaseURL: false)?
            .queryItems?
            .contains { $0.name == "code" && !($0.value ?? "").isEmpty } ?? false
        guard hasFragmentCode || hasQueryCode else { return nil }

        return InviteLink(raw: trimmed, issuerHost: host)
    }
}

// MARK: - Deep link

/// Parses `tracecommons://enroll?invite=<percent-encoded-invite-url>`.
///
/// Mail clients cannot be made to open an arbitrary `https://` link in this
/// app, so an invite email carries the app's own URL scheme instead, with
/// the real invite link (an issuer URL, not this app's) folded into the
/// `invite` query parameter.
enum DeepLink {
    static func inviteURL(from url: URL) -> String? {
        guard url.scheme?.lowercased() == "tracecommons",
              url.host?.lowercased() == "enroll",
              let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        else { return nil }
        // The empty value is dropped, not passed on. `tracecommons://enroll?invite=`
        // used to yield Some("") here and drive the screen into a resolve of
        // nothing, with an empty field and an error. Rust filters it
        // (`commands.rs`, `.filter(|v| !v.is_empty())`) and Windows returns
        // null (`DeepLink.cs`); one invite mail reaches all three clients, so
        // the parse is a contract and this was the one place it diverged.
        return components.queryItems?
            .first(where: { $0.name == "invite" })?
            .value
            .flatMap { $0.isEmpty ? nil : $0 }
    }
}
