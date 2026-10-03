import SwiftUI
import TCBridge
import TCDesign
import UserNotifications

/// Onboarding screen 6, "Done" -- the last onboarding screen and, by design,
/// the first thing the app ever confirms about itself. Copy is verbatim from
/// the shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Onboarding", "### 6. Done").
///
/// "You're set up. Nothing has been sent." is the entire point of the
/// screen: the first thing this app ever does is nothing. Do not soften or
/// reorder it out of the first line.
struct OnboardingDoneView: View {
    var onFinish: () -> Void

    var body: some View {
        ScrollView {
            OnboardingDoneContent(onFinish: onFinish)
        }
    }
}

/// The screen's content, split out of its `ScrollView` for the same
/// `ImageRenderer` reason documented on `ConsentScopesContent`.
///
/// Carries the login-item offer from the design spec's "## Login item"
/// section, verbatim wording, offered here (end of onboarding) rather than
/// silently at first launch. `ImageRenderer` (see `DebugScreenshot`) never
/// fires a button tap, so rendering this for a screenshot only ever reads
/// `LoginItemManager.currentState` -- it cannot trigger `register()`.
///
/// The only way out is `onFinish`, from Done's own button: nothing here
/// completes onboarding by itself, and the button is disabled while a
/// notification request is pending so the answer cannot be abandoned.
struct OnboardingDoneContent: View {
    var onFinish: () -> Void

    @State private var offerDismissed = false
    @State private var registerOutcome: LoginItemManager.RegisterOutcome?

    /// The notification offer's state. `nil` until the system has been
    /// asked where it stands, which happens on appear; the card shows only
    /// when the answer is "not yet asked". `ImageRenderer` runs no `.task`,
    /// so a screenshot of this screen never shows the card and never asks.
    @State private var notificationStatus: UNAuthorizationStatus?
    @State private var notificationOfferDismissed = false
    @State private var notificationRequestPending = false

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                GlassStatusLabel(OnboardingDoneWords.setUpNothingSent, status: .on)
                Text(Notifier.copy?.doneBody ?? "")
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            loginItemOffer
            notificationOffer

            HStack(spacing: GlassTokens.Space.s4) {
                Spacer(minLength: 0)
                Button(OnboardingDoneWords.done, action: onFinish)
                    .buttonStyle(GlassButtonStyle(.primary))
                    .keyboardShortcut(.defaultAction)
                    .disabled(notificationRequestPending)
            }
        }
        .padding(GlassTokens.Space.panePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .task { await refreshStatus() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            Task { await refreshStatus() }
        }
    }

    private func refreshStatus() async {
        notificationStatus = await Notifier.shared.authorizationStatus()
    }

    private func caption(_ sentence: String) -> some View {
        Text(sentence)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    /// The permission prompt, where the spec puts it: at the end of
    /// onboarding, under a sentence saying what the notifications are for.
    /// It used to be fired from launch, before the app had said what it
    /// was. Shown only while the system has never been asked; a yes or a
    /// no already given is not re-asked here, and Settings shows the state
    /// either way. Until the system answers nothing is drawn: an unanswered
    /// status is never read as a state.
    @ViewBuilder
    private var notificationOffer: some View {
        if notificationStatus == .denied {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                caption(Notifier.copy?.notificationDenied ?? "")
                Link(Notifier.copy?.systemSettings ?? "", destination: Notifier.systemSettingsURL)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.accentText)
            }
        } else if Notifier.canPostDigest(notificationStatus) {
            caption(Notifier.copy?.notificationAllowed ?? "")
        } else if !notificationOfferDismissed && notificationStatus == .notDetermined {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(Notifier.copy?.notificationOffer ?? "")
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                    Text(Notifier.purpose)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                    HStack(spacing: GlassTokens.Space.s3) {
                        Button(Notifier.copy?.notNow ?? "") {
                            notificationOfferDismissed = true
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        Button(Notifier.copy?.notificationAllow ?? "") {
                            requestAuthorization()
                        }
                        .buttonStyle(GlassButtonStyle(.primary))
                    }
                    .disabled(notificationRequestPending)
                }
            }
        }
    }

    private func requestAuthorization() {
        guard !notificationRequestPending else { return }
        notificationRequestPending = true
        Task {
            defer { notificationRequestPending = false }
            _ = await Notifier.shared.requestAuthorization()
            await refreshStatus()
        }
    }

    /// Nothing is shown once the app is already an enabled login item --
    /// re-asking a question already answered "yes" is noise -- or once this
    /// screen's own offer has been answered one way or the other.
    @ViewBuilder
    private var loginItemOffer: some View {
        if let registerOutcome {
            loginItemResult(registerOutcome)
        } else if !offerDismissed && LoginItemManager.currentState != .enabled {
            GlassCard {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(OnboardingDoneWords.startAtLoginQuestion)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                    caption(OnboardingDoneWords.needsToBeRunning)
                    HStack(spacing: GlassTokens.Space.s3) {
                        // Declining is the quiet button; the accent means
                        // "yes" everywhere else in the app.
                        Button(OnboardingDoneWords.notNow) {
                            offerDismissed = true
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        Button(OnboardingDoneWords.startAtLogin) {
                            registerOutcome = LoginItemManager.register()
                        }
                        .buttonStyle(GlassButtonStyle(.primary))
                    }
                }
            }
        }
    }

    /// `.requiresApproval` is the expected result of `register()` when the
    /// user (or a prior denial) has not yet approved this app in System
    /// Settings -- it is not an error, and must not be shown as one. It gets
    /// the same honest treatment as `.failed`: say what happened, point at
    /// where to fix it, and do not retry silently.
    @ViewBuilder
    private func loginItemResult(_ outcome: LoginItemManager.RegisterOutcome) -> some View {
        switch outcome {
        case .enabled:
            caption(OnboardingDoneWords.willStartNextLogin)
        case .requiresApproval:
            caption(OnboardingDoneWords.almostThere)
        case .failed(let message):
            caption(OnboardingDoneWords.couldNotTurnOn(message))
        }
    }
}

/// This screen's sentences, held verbatim from the legacy screen. The
/// notification words are the core's (`Notifier.copy`).
enum OnboardingDoneWords {
    static let setUpNothingSent = "You're set up. Nothing has been sent."
    static let startAtLoginQuestion = "Start Trace Commons when you log in?"
    static let needsToBeRunning = "It needs to be running to notice finished sessions."
    static let willStartNextLogin = "Trace Commons will start automatically next time you log in."
    static let almostThere = """
        Almost there -- macOS needs you to approve this in System Settings -> \
        General -> Login Items before it will start automatically.
        """
    static func couldNotTurnOn(_ message: String) -> String { "Couldn't turn this on: \(message)" }
    static let notNow = "Not now"
    static let startAtLogin = "Start at login"
    static let done = "Done"
}
