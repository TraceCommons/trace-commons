import AppKit
import SwiftUI

/// Icon precedence, from the shared design: attention (numeric badge) ->
/// unhealthy (amber dot) -> paused (struck through) -> idle.
struct MenuBarLabel: View {
    @ObservedObject var model: AppModel

    var body: some View {
        if model.decisionsOwed > 0 {
            // The badge counts DECISIONS OWED. If it shows 3, there are
            // exactly three things to say yes or no to.
            Label("\(model.decisionsOwed)", systemImage: "tray.full")
        } else if model.health != nil {
            Image(systemName: "exclamationmark.triangle")
        } else if model.status.paused {
            Image(systemName: "tray.and.arrow.down.fill")
                .foregroundStyle(.tertiary)
        } else {
            Image(systemName: "tray")
        }
    }
}

struct MenuBarContent: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            waitingSection
            Divider()
            healthSection
            armedSection
            weekSection
            Divider()
            Button("Review waiting sessions…") { openMain() }
            pauseSection
            Divider()
            Button("Open Trace Commons") { openMain() }
            Button("Quit…") { confirmQuit() }
        }
        .onAppear { model.refreshAll() }
    }

    // MARK: - What is waiting

    @ViewBuilder
    private var waitingSection: some View {
        switch model.startup {
        case .starting:
            Text("Starting…")
        case .refused:
            Text("Not watching anything")
            Text("Open the window for what to do about it")
        case .running:
            if model.decisionsOwed == 0 {
                Text("Nothing waiting")
            } else {
                Text("\(model.decisionsOwed) waiting for your decision")
                // Not approve buttons. Deliberately inert lines: the only
                // forward action in this menu is Review.
                ForEach(model.waitingByProject, id: \.label) { row in
                    Text("   \(row.label) — \(row.count) · \(Format.bytes(row.bytes))")
                }
            }
        }
    }

    @ViewBuilder
    private var healthSection: some View {
        if let health = model.health {
            Text(health.title)
            Text(health.detail.replacingOccurrences(of: "\n", with: " "))
        }
    }

    @ViewBuilder
    private var armedSection: some View {
        // Armed projects are shown persistently and never collapsed away, so
        // a contributor always knows what uploads without asking.
        if !model.armedProjects.isEmpty {
            Divider()
            Text("Armed: \(model.armedProjects.count) project(s) — contributed without asking")
            ForEach(model.armedProjects) { project in
                Text("   \(project.projectLabel)")
            }
        }
    }

    @ViewBuilder
    private var weekSection: some View {
        if let rollup = model.rollup {
            Divider()
            Text("This week: \(rollup.week.submitted) contributed, "
                + "\(rollup.week.quarantined) held for privacy review")
        }
    }

    // MARK: - Pause

    @ViewBuilder
    private var pauseSection: some View {
        if model.status.paused {
            Button("Resume watching") { model.resume() }
        } else {
            Menu("Pause") {
                Button("For 1 hour") {
                    model.pause(until: Date().addingTimeInterval(3600))
                }
                Button("Until tomorrow morning") {
                    model.pause(until: Format.tomorrowMorning())
                }
                Button("Until I turn it back on") {
                    model.pause(until: nil)
                }
            }
        }
    }

    // MARK: - Quit

    private func openMain() {
        NSApp.activate(ignoringOtherApps: true)
        openWindow(id: WindowID.main)
    }

    /// The quit confirmation says what actually happens.
    ///
    /// The shared spec's copy ("The background watcher keeps running") is
    /// written for a shell with a separate daemon process. On macOS the app
    /// IS the daemon -- that is the entire point of the in-process shape --
    /// so the watcher stops when the app does, and this says so rather than
    /// repeating a sentence that would be false here.
    private func confirmQuit() {
        NSApp.activate(ignoringOtherApps: true)
        let alert = NSAlert()
        alert.messageText = "Quit Trace Commons?"
        alert.informativeText = """
        The watcher runs inside this app, so quitting stops it. Nothing will be \
        noticed or sent while it is closed.

        Sessions already waiting stay on this machine and will be here when you \
        come back. Nothing is sent while nobody's approving.
        """
        alert.addButton(withTitle: "Quit")
        alert.addButton(withTitle: "Keep running")
        if alert.runModal() == .alertFirstButtonReturn {
            NSApp.terminate(nil)
        }
    }
}

enum Format {
    static func bytes(_ count: Int) -> String {
        let formatter = ByteCountFormatter()
        formatter.countStyle = .file
        formatter.allowedUnits = [.useKB, .useMB, .useGB]
        return formatter.string(fromByteCount: Int64(count))
    }

    static func when(_ date: Date) -> String {
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return formatter.localizedString(for: date, relativeTo: Date())
    }

    static func tomorrowMorning() -> Date {
        let calendar = Calendar.current
        let tomorrow = calendar.date(byAdding: .day, value: 1, to: Date()) ?? Date()
        return calendar.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow) ?? tomorrow
    }
}
