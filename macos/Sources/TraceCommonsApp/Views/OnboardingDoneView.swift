import SwiftUI
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
    @State private var notificationsAllowed: Bool?
    @State private var notificationRequestPending = false

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            HStack(spacing: TC.Space.s) {
                Image(systemName: TC.Tone.clear.symbol)
                    .font(.system(size: 18))
                    .foregroundStyle(TC.green)
                    .accessibilityHidden(true)
                Text("You're set up. Nothing has been sent.")
                    .font(TC.Font_.sectionTitle)
            }

            // "and in the Dock" is this shell's addition to the spec's
            // sentence: the app is a regular one with a Dock icon
            // (`AppDelegate` sets `.regular`), and a contributor told it
            // lives in the menu bar would not think to look there.
            Text("""
            Trace Commons lives in your menu bar, and it has a Dock icon too. When a \
            session finishes and goes quiet, it will appear for review. Notification \
            timing follows your reminder settings.
            """)
            .font(.body)

            loginItemOffer
            notificationOffer

            Button("Done", action: onFinish)
                .disabled(notificationRequestPending)
                .tcPrimaryAction()
                .keyboardShortcut(.defaultAction)
        }
        .padding(TC.Space.xxl)
        .tcColumn(TC.Measure.prose)
        .tcScreen()
        .task { notificationStatus = await Notifier.shared.authorizationStatus() }
    }

    /// The permission prompt, where the spec puts it: at the end of
    /// onboarding, under a sentence saying what the notifications are for.
    /// It used to be fired from launch, before the app had said what it
    /// was. Shown only while the system has never been asked; a yes or a
    /// no already given is not re-asked here, and Settings shows the state
    /// either way.
    @ViewBuilder
    private var notificationOffer: some View {
        if let notificationsAllowed {
            Text(
                notificationsAllowed
                    ? "You'll be told when sessions are waiting."
                    : "No notifications. You can turn them on later in Settings."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
        } else if !notificationOfferDismissed && notificationStatus == .notDetermined {
            VStack(alignment: .leading, spacing: 10) {
                Text("Let Trace Commons notify you?")
                    .font(.callout.weight(.semibold))
                Text(Notifier.purpose)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: 12) {
                    Button("Not now") {
                        notificationOfferDismissed = true
                    }
                    .tint(.primary)
                    Button("Allow notifications") {
                        guard !notificationRequestPending else { return }
                        notificationRequestPending = true
                        Task {
                            notificationsAllowed = await Notifier.shared.requestAuthorization()
                            notificationRequestPending = false
                        }
                    }
                    .tcPrimaryAction()
                }
                .disabled(notificationRequestPending)
            }
            .padding(TC.Space.l)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
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
            VStack(alignment: .leading, spacing: 10) {
                Text("Start Trace Commons when you log in?")
                    .font(.callout.weight(.semibold))
                Text("It needs to be running to notice finished sessions.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                HStack(spacing: 12) {
                    Button("Not now") {
                        offerDismissed = true
                    }
                    // Untinted: declining should not wear the accent that
                    // means "yes" everywhere else in the app.
                    .tint(.primary)
                    Button("Start at login") {
                        registerOutcome = LoginItemManager.register()
                    }
                .tcPrimaryAction()
                }
            }
            .padding(TC.Space.l)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
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
            Text("Trace Commons will start automatically next time you log in.")
                .font(.callout)
                .foregroundStyle(.secondary)
        case .requiresApproval:
            Text("""
            Almost there -- macOS needs you to approve this in System Settings -> \
            General -> Login Items before it will start automatically.
            """)
            .font(.callout)
            .foregroundStyle(.secondary)
        case .failed(let message):
            Text("Couldn't turn this on: \(message)")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
    }
}
