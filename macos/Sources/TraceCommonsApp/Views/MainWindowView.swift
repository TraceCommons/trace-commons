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
        NavigationSplitView {
            List(Section.allCases, selection: $section) { item in
                Label(item.rawValue, systemImage: item.symbol)
                    .badge(item == .queue ? model.decisionsOwed : 0)
                    .tag(item)
            }
            .navigationSplitViewColumnWidth(min: 170, ideal: 190)
        } detail: {
            switch model.startup {
            case .starting:
                CenteredNotice(
                    title: "Starting…",
                    detail: "Nothing has been sent."
                )
            case .refused(let reason):
                // Not-running is a first-class state, not a spinner that
                // never resolves.
                CenteredNotice(title: "The watcher isn't running.", detail: reason)
            case .running:
                switch section ?? .queue {
                case .queue: QueueView()
                case .history: HistoryView()
                case .settings: SettingsView()
                }
            }
        }
        .onAppear { model.refreshAll() }
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
