import SwiftUI

struct MainWindowView: View {
    @EnvironmentObject private var model: AppModel
    @State private var section: Section? = .queue

    enum Section: String, CaseIterable, Identifiable {
        case queue = "Waiting"
        case history = "History"
        case settings = "Settings"
        var id: String { rawValue }

        var symbol: String {
            switch self {
            case .queue: return "tray.full"
            case .history: return "clock.arrow.circlepath"
            case .settings: return "gearshape"
            }
        }
    }

    var body: some View {
        switch model.startup {
        case .starting:
            CenteredNotice(
                title: "Starting…",
                detail: "Nothing has been sent."
            )
            .onAppear { model.refreshAll() }
        case .refused(let reason):
            // Not-running is a first-class state, not a spinner that never
            // resolves.
            CenteredNotice(title: "The watcher isn't running.", detail: reason)
                .onAppear { model.refreshAll() }
        case .running:
            // First-run detection: `status.logged_in` from the daemon's own
            // `status`, never a local file probe -- see `AppModel.start()`
            // for why the app treats the daemon as the source of truth.
            // `status` defaults to not-logged-in until the first real
            // answer arrives (`DaemonStatus.unknown`), so an already
            // enrolled contributor may see one brief onboarding frame
            // before this flips to `true` -- the fail-closed direction,
            // never the reverse. `isOnboardingComplete` is the second half
            // of that check: see its doc comment on `AppModel` and
            // `OnboardingCoordinatorView`'s "Atomicity" note for why
            // `logged_in` alone cannot tell "fully onboarded" from
            // "enrolled but consent was never confirmed."
            // Both "not enrolled yet" and "enrolled but onboarding not
            // finished" render through this ONE `if` branch, deliberately:
            // `set_consent_scopes` succeeding mid-flow (screen 3 -> 4/5)
            // flips `status.logged_in` from stale-false to true on the very
            // same turn the coordinator advances its own `step` -- see
            // `AppModel.setConsentScopes`. Two separate `if` / `else if`
            // branches, each constructing their own
            // `OnboardingCoordinatorView(startAt:)`, would count as two
            // different view identities to SwiftUI; the moment `logged_in`
            // flips, the view would be torn down and rebuilt from
            // `startAt: .consent`, throwing away whatever step the
            // contributor had just reached. One branch keeps one identity
            // (and therefore one `@State step`) for the entire flow.
            if !model.status.loggedIn || !model.isOnboardingComplete {
                OnboardingCoordinatorView(
                    startAt: model.status.loggedIn ? .consent : .welcome,
                    onComplete: { model.markOnboardingComplete() }
                )
                .onAppear { model.refreshAll() }
            } else {
                NavigationSplitView {
                    List(Section.allCases, selection: $section) { item in
                        Label(item.rawValue, systemImage: item.symbol)
                            .badge(item == .queue ? model.decisionsOwed : 0)
                            .tag(item)
                    }
                    .navigationSplitViewColumnWidth(min: 170, ideal: 190)
                } detail: {
                    switch section ?? .queue {
                    case .queue: QueueView()
                    case .history: HistoryView()
                    case .settings: SettingsView()
                    }
                }
                .onAppear { model.refreshAll() }
            }
        }
    }
}

struct CenteredNotice: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 10) {
            Text(title).font(.title3.weight(.semibold))
            Text(detail)
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 460)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding()
    }
}

/// The health banner. Ambient by default; only states with something to do
/// carry a button.
struct HealthBanner: View {
    let health: HealthCopy

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Circle()
                .fill(health.severity == .actionable ? Color.orange : Color.secondary)
                .frame(width: 9, height: 9)
                .padding(.top, 5)
            VStack(alignment: .leading, spacing: 3) {
                Text(health.title).font(.callout.weight(.semibold))
                Text(health.detail).font(.callout).foregroundStyle(.secondary)
            }
            Spacer()
            if let action = health.actionTitle {
                // Deliberately inert: the flows behind Reconnect and Review
                // and confirm are onboarding surfaces, which are not built
                // yet. A button that lies about working is worse than one
                // that says it is not here.
                Button(action) {}
                    .disabled(true)
                    .help("Not wired up in this build.")
            }
        }
        .padding(12)
        .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 8))
    }
}
