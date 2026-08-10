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

        /// What the section is for, in the window's subtitle. A person who
        /// opened this app from a notification needs to know where they are
        /// before they need to know what to do.
        var subtitle: String {
            switch self {
            case .queue: return "Nothing is sent unless you say so."
            case .history: return "What you have contributed, and what is still being reviewed."
            case .settings: return "What this machine watches, and what your traces are allowed to do."
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
                .tcScreen()
                .onAppear { model.refreshAll() }
            } else {
                shell
                    .onAppear { model.refreshAll() }
            }
        }
    }

    /// Sidebar plus a real title bar. Without a toolbar the window read as a
    /// preview canvas: content floating in an unowned field with nothing to
    /// anchor it and nowhere to put a global control.
    private var shell: some View {
        NavigationSplitView {
            List(Section.allCases, selection: $section) { item in
                Label(item.rawValue, systemImage: item.symbol)
                    .badge(item == .queue ? model.decisionsOwed : 0)
                    .tag(item)
            }
            .navigationSplitViewColumnWidth(min: 180, ideal: 200)
        } detail: {
            Group {
                switch section ?? .queue {
                case .queue: QueueView()
                case .history: HistoryView()
                case .settings: SettingsView()
                }
            }
            // The brand ground stops here. The sidebar and the title bar
            // above it stay system materials, which is what keeps this
            // looking like a Mac window rather than a web page in one.
            .tcScreen()
            .navigationTitle((section ?? .queue).rawValue)
            .navigationSubtitle((section ?? .queue).subtitle)
            .toolbar { watchState }
        }
    }

    /// The one global control worth a toolbar slot, and a permanent readout
    /// of whether this machine is watching at all. Paused is a state a
    /// person can forget they chose, so it is never left implicit.
    @ToolbarContentBuilder
    private var watchState: some ToolbarContent {
        ToolbarItem(placement: .status) {
            if model.status.paused {
                TCTag(text: "Paused", tone: .attention, symbol: "pause.circle")
                    .accessibilityLabel("Paused. Nothing is being queued or sent.")
            } else {
                TCTag(text: "Watching", tone: .neutral, symbol: "eye")
                    .accessibilityLabel("Watching for finished sessions.")
            }
        }
        ToolbarItem(placement: .primaryAction) {
            if model.status.paused {
                Button {
                    model.resume()
                } label: {
                    Label("Resume watching", systemImage: "play.circle")
                }
                .help("Start noticing finished sessions again.")
            } else {
                Menu {
                    Button("For 1 hour") { model.pause(until: Date().addingTimeInterval(3600)) }
                    Button("Until tomorrow morning") { model.pause(until: Format.tomorrowMorning()) }
                    Button("Until I turn it back on") { model.pause(until: nil) }
                } label: {
                    Label("Pause", systemImage: "pause.circle")
                }
                .help("Stop noticing finished sessions.")
            }
        }
    }
}

struct CenteredNotice: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: TC.Space.s) {
            Text(title).font(TC.Font_.screenTitle)
            Text(detail)
                .font(TC.Font_.meta)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: 460)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(TC.Space.xl)
    }
}

/// The health banner. Ambient by default; only states with something to do
/// carry a button.
struct HealthBanner: View {
    let health: HealthCopy

    private var tone: TC.Tone {
        health.severity == .actionable ? .attention : .neutral
    }

    var body: some View {
        HStack(alignment: .top, spacing: TC.Space.m) {
            // Symbol, not a coloured dot: the severity has to survive
            // greyscale and it has to reach VoiceOver.
            Image(systemName: tone.symbol)
                .foregroundStyle(tone.color)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                Text(health.title).font(TC.Font_.cardTitle)
                Text(health.detail)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: TC.Space.m)
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
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard(emphasised: health.severity == .actionable)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(
            health.severity == .actionable
                ? "Needs attention. \(health.title)"
                : health.title
        )
    }
}
