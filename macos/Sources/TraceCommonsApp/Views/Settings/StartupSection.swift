import AppKit
import SwiftUI
import TCDesign
import TCUpdates
import UserNotifications

/// The login item. Reflects the live `SMAppService.mainApp.status`, not a
/// locally cached bool: the user can flip it in System Settings while this
/// window is open, so it is read fresh on appear. `.requiresApproval` is
/// guidance, not an error -- retrying `register()` would not change it.
struct StartupSection: View {
    /// The login item's last refusal is the model's (`loginItemActionError`):
    /// this view is thrown away when the section changes (G8 of #1229).
    @EnvironmentObject private var model: AppModel
    @State private var loginItemState: LoginItemManager.State = LoginItemManager.currentState

    var body: some View {
        // #1146's `DESKTOP / System integrations`, with Refresh re-reading
        // the login item from the system.
        GlassEyebrowCard(SettingsLegacyWords.desktopEyebrow, title: SettingsLegacyWords.desktopTitle) {
            Button(SettingsLegacyWords.refresh) { loginItemState = LoginItemManager.currentState }
                .buttonStyle(GlassButtonStyle(.link))
                .fixedSize()
        } content: {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                switch loginItemState {
                case .enabled:
                    Toggle(SettingsLegacyWords.startAtLogin, isOn: Binding(
                        get: { true }, set: { if !$0 { setLoginItem(enabled: false) } }))
                        .toggleStyle(GlassToggleStyle(.settings))
                case .notRegistered, .notFound:
                    Toggle(SettingsLegacyWords.startAtLogin, isOn: Binding(
                        get: { false }, set: { if $0 { setLoginItem(enabled: true) } }))
                        .toggleStyle(GlassToggleStyle(.settings))
                case .requiresApproval:
                    Text(SettingsLegacyWords.waitingOnApproval).glassType(GlassTokens.TypeScale.body)
                    Text(SettingsLegacyWords.turnOnInSystemSettings)
                        .glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textSecondary)
                }
                // A refused switch, under it, unboxed (Ron, 2026-10-09).
                if let loginItemActionError = model.loginItemActionError {
                    GlassAlert(loginItemActionError)
                }
            }
        }
        .onAppear { loginItemState = LoginItemManager.currentState }
    }

    private func setLoginItem(enabled: Bool) {
        model.loginItemActionError = nil
        if enabled {
            switch LoginItemManager.register() {
            case .enabled, .requiresApproval:
                break
            case .failed(let message):
                model.loginItemActionError = SettingsLegacyWords.couldNotTurnOn(message)
            }
        } else {
            if case .failed(let message) = LoginItemManager.unregister() {
                model.loginItemActionError = SettingsLegacyWords.couldNotTurnOff(message)
            }
        }
        loginItemState = LoginItemManager.currentState
    }
}

/// Where the system stands on this app's notifications. Nil until the
/// system has answered, and nil for good where there is no notification
/// centre (a bare `swift run` binary has no bundle identifier): the card
/// renders nothing then, never a healthy-looking row. A denial is not an
/// error; it says where to change the answer, because this app cannot
/// re-ask once the system has been told no.
struct NotificationsSection: View {
    @State private var notificationStatus: UNAuthorizationStatus?
    @State private var notificationRequestPending = false

    var body: some View {
        // The refresh hangs on a node that always exists: with no status yet
        // the card below can be empty, and a modifier on an empty container
        // may never run, which would leave the status nil for good.
        VStack(alignment: .leading, spacing: 0) {
            Color.clear.frame(width: 0, height: 0).accessibilityHidden(true)
            if let heading = Notifier.copy?.notificationHeading {
                GlassEyebrowCard(heading) { card }
            }
        }
        .task { await refreshStatus() }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            Task { await refreshStatus() }
        }
    }

    /// Until the system answers (and for good where there is no centre) the
    /// card says the state is unknown, never a healthy row.
    @ViewBuilder
    private var card: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            Text(Notifier.purpose)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if let status = notificationStatus {
                switch status {
                case .authorized, .provisional, .ephemeral:
                    SettingsStateRow(title: Notifier.copy?.notificationAllowed ?? "", isOn: true)
                case .denied:
                    SettingsStateRow(title: Notifier.copy?.notificationDenied ?? "", isOn: false)
                    settingsLink
                case .notDetermined:
                    SettingsStateRow(title: Notifier.copy?.notificationNotAsked ?? "", isOn: false)
                    Button(Notifier.copy?.notificationAllow ?? "") { requestAuthorization() }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(notificationRequestPending)
                @unknown default:
                    unknown
                }
            } else {
                unknown
            }
            GlassHairline()
            NudgeSettingsSection()
        }
    }

    @ViewBuilder
    private var unknown: some View {
        Text(Notifier.copy?.notificationUnknown ?? "").glassType(GlassTokens.TypeScale.body)
        settingsLink
    }

    private func refreshStatus() async {
        notificationStatus = await Notifier.shared.authorizationStatus()
    }

    private var settingsLink: some View {
        Link(Notifier.copy?.systemSettings ?? "", destination: Notifier.systemSettingsURL)
            .glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(GlassColor.accentText)
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
}

/// Version, update state, and -- when Homebrew owns this copy -- the one
/// command that actually works. The Homebrew branch is not an apology for a
/// missing feature: Homebrew placed these bytes and Homebrew replaces them,
/// so "Check Now" there would fight the package manager over the same file.
struct UpdatesSection: View {
    @ObservedObject private var updates = UpdateController.shared

    var body: some View {
        GlassEyebrowCard(SettingsWords.updates) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                HStack(spacing: GlassTokens.Space.s3) {
                    Text(SettingsLegacyWords.version).glassType(GlassTokens.TypeScale.label)
                        .foregroundStyle(GlassColor.textSecondary)
                    Text(updates.currentVersion)
                        .glassType(GlassTokens.TypeScale.mono)
                        .textSelection(.enabled)
                }
                switch updates.mode {
                case .selfUpdating:
                    GlassTag(SettingsLegacyWords.checksDaily, tone: .on)
                    caption(lastCheckSentence)
                    // Deliberately does NOT claim the download already
                    // happened: Sparkle finds the update in the background
                    // and then asks; the download follows the yes.
                    caption(SettingsLegacyWords.checksAutomatically)
                    Button(SettingsLegacyWords.checkNow) { updates.checkNow() }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(!updates.canCheckNow)
                case .managedByHomebrew(let command):
                    GlassTag(SettingsLegacyWords.managedByHomebrew, tone: .ask)
                    caption(SettingsLegacyWords.homebrewReplaces)
                    HStack(spacing: GlassTokens.Space.s3) {
                        GlassWell {
                            Text(command)
                                .glassType(GlassTokens.TypeScale.mono)
                                .textSelection(.enabled)
                                .padding(GlassTokens.Space.s3)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        Button(SettingsLegacyWords.copy) {
                            NSPasteboard.general.clearContents()
                            NSPasteboard.general.setString(command, forType: .string)
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                    }
                case .disabled(let reason):
                    GlassTag(SettingsLegacyWords.updatesUnavailable, tone: .outside)
                    caption(disabledSentence(reason))
                }
            }
        }
    }

    private func caption(_ sentence: String) -> some View {
        Text(sentence)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var lastCheckSentence: String {
        guard let date = updates.lastCheckDate else {
            return SettingsLegacyWords.notCheckedYet
        }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return SettingsLegacyWords.lastChecked(formatter.localizedString(for: date, relativeTo: Date()))
    }

    /// Turns the policy's stable label into a sentence. The label itself is
    /// what gets logged; this is what a person reads.
    private func disabledSentence(_ reason: String) -> String {
        switch reason {
        case UpdatePolicy.noFeedReason: SettingsLegacyWords.noFeed
        case UpdatePolicy.insecureFeedReason: SettingsLegacyWords.insecureFeed
        default: SettingsLegacyWords.updatesOff
        }
    }
}
