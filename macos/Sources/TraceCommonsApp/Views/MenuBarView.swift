import AppKit
import SwiftUI

/// The menu-bar item.
///
/// The mark, not a tray glyph. A menu bar holds twenty icons drawn from the
/// same SF Symbol set and a generic tray is not findable among them; The Turn
/// is, and it is the same mark every other piece of this product's chrome
/// carries, which is the point.
///
/// It is the mark's **template** variant, at the 15pt the design spec states
/// for the macOS menu bar (`design-import/DESIGN-SPEC.md` sections 1.2 and
/// 1.3): frameless, single ink, drawn in `.primary` so the system recolours
/// it across the menu bar's light, dark and selected states the way a
/// template image behaves. The frame is dropped because a hairline rectangle
/// does not survive 15pt next to the system's own glyphs, and the brackets
/// thicken from 7/64 to 8/64 to carry the mark without it.
///
/// State precedence is unchanged from the shared design: decisions owed
/// (numeric badge) -> unhealthy -> paused -> idle. The badge counts
/// DECISIONS OWED; if it shows 3, there are exactly three things to say yes
/// or no to. Every state that is not "idle" carries a second glyph as well
/// as a count, because a dimmed mark on its own is not a state anybody can
/// read.
struct MenuBarLabel: View {
    @ObservedObject var model: AppModel

    var body: some View {
        HStack(spacing: TC.Space.xxs) {
            BrandMark(size: 15, variant: .template)
                .opacity(model.status.paused ? 0.5 : 1)
            if model.decisionsOwed > 0 {
                // The one countable figure in the chrome, so it is set in the
                // same mono the manifest strips use rather than in the menu
                // bar's default face.
                Text("\(model.decisionsOwed)")
                    .font(TC.Font_.ledger)
                    .monospacedDigit()
            } else if model.health != nil {
                Image(systemName: "exclamationmark.triangle")
                    .imageScale(.small)
            } else if model.status.paused {
                Image(systemName: "pause.fill")
                    .imageScale(.small)
            }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(accessibilityLabel)
    }

    private var accessibilityLabel: String {
        if model.decisionsOwed > 0 {
            return "Trace Commons. ^[\(model.decisionsOwed) session](inflect: true) waiting for your decision."
        }
        if model.health != nil { return "Trace Commons. Needs attention." }
        if model.status.paused { return "Trace Commons. Paused." }
        return "Trace Commons. Nothing waiting."
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
            // A menu is not a shrunken window. There are no cards, no
            // manifest strips and no brand colour down here -- an AppKit
            // menu draws its own vibrancy, its own highlight and its own
            // type, and anything painted over that reads as a bug. The only
            // additions are leading glyphs, which menus have always had.
            //
            // This is the token layer's answer for this surface, not an
            // omission: `MenuBarExtra`'s default `.menu` style hands these
            // rows to AppKit, which resolves its own font and colours and
            // discards a `.font(TC.Font_...)` or a `.foregroundStyle(TC...)`
            // set here. Tokens are applied where they survive -- the status
            // item in `MenuBarLabel` above, and every window this menu opens.
            Button {
                openMain()
            } label: {
                Label("Review waiting sessions…", systemImage: "tray.full")
            }
            pauseSection
            Divider()
            Button {
                openMain()
            } label: {
                Label("Open Trace Commons", systemImage: "macwindow")
            }
            // Straight to terminate, not to the alert. The confirmation now
            // lives in AppDelegate.applicationShouldTerminate, because Cmd-Q,
            // the App menu and the Dock icon's context menu all terminate
            // without passing through here. Asking in both places would
            // confirm twice on this path and once everywhere else.
            Button("Quit…") { NSApp.terminate(nil) }
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
        case .needsRoots:
            Text("Not watching anything yet")
            Text("Open the window to choose which folders to watch")
        case .running:
            if model.decisionsOwed == 0 {
                Text("Nothing waiting")
            } else {
                Text("\(model.decisionsOwed) waiting for your decision")
                // Not approve buttons. Deliberately inert lines: the only
                // forward action in this menu is Review.
                ForEach(model.waitingByProject, id: \.id) { row in
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
            Button {
                model.resume()
            } label: {
                Label("Resume watching", systemImage: "play.circle")
            }
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

    // MARK: - Opening the window

    private func openMain() {
        NSApp.activate(ignoringOtherApps: true)
        openWindow(id: WindowID.main)
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
