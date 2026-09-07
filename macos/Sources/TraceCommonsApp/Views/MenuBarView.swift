import AppKit
import SwiftUI
import TCShellCore

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
/// (numeric badge) -> unhealthy -> paused -> idle, decided in
/// `MenuBarStatus` where it can be tested. The badge counts DECISIONS OWED;
/// if it shows 3, there are exactly three things to say yes or no to. Every
/// state that is not "idle" carries a second glyph as well as a count,
/// because a dimmed mark on its own is not a state anybody can read.
///
/// The count is ON the mark, not beside it. A bare number in the menu bar
/// belongs to nothing -- twenty status items along, a "3" is not findable
/// as this product's "3" -- so it is a capsule over the mark's top right
/// corner, the way an app badge sits on a Dock icon. It was tried under the
/// mark first: 15pt of mark plus a digit row anyone can read is more than
/// the 22pt the menu bar has, and shrinking the mark to make room defeats
/// the reason it is there. See `TC.MenuBar` and `MenuBarGlyph`.
struct MenuBarLabel: View {
    @ObservedObject var model: AppModel

    var body: some View {
        MenuBarGlyph(state: state)
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(accessibilityLabel)
    }

    private var state: MenuBarState {
        MenuBarStatus.state(
            decisionsOwed: model.decisionsOwed,
            unhealthy: model.health != nil,
            paused: model.status.paused,
            available: model.startup == .running
        )
    }

    private var accessibilityLabel: String {
        MenuBarStatus.accessibilityLabel(
            decisionsOwed: model.decisionsOwed, unhealthy: model.health != nil,
            paused: model.status.paused, available: model.startup == .running)
    }
}

/// The status item's drawing, as a pure function of `MenuBarState` so it
/// can be rendered off a fixture -- the screenshot hook and the label test
/// both do -- without an `AppModel`.
///
/// The badge uses an opaque black/white pair rather than transparent digits.
/// Its contrast is independent of the wallpaper behind the translucent menu bar.
struct MenuBarGlyph: View {
    let state: MenuBarState
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        HStack(spacing: TC.Space.xxs) {
            // Top right, where a Dock badge sits, and not the lower right:
            // the agent's bracket IS the lower right of this mark, and a
            // two-digit capsule there covered it whole, leaving a single
            // bracket and a number. The top right quadrant is the one
            // corner neither bracket draws in.
            //
            // The badge is laid out beside the mark and pulled back over
            // it by a fixed amount, rather than pinned to the corner: its
            // LEADING edge is what stays put, so a wider count grows out
            // into the empty bar instead of back across the user's
            // bracket, which a trailing-pinned "12" was found to do.
            HStack(alignment: .top, spacing: -TC.MenuBar.badgeOverlap) {
                BrandMark(size: TC.MenuBar.mark, variant: .template)
                    .opacity(state.isPaused ? 0.5 : 1)
                if case .count(let text, _) = state {
                    // The halo punches a clear ring out of whatever the
                    // capsule touches so it sits on the mark instead of
                    // merging with a stroke. Digits have an opaque backdrop.
                    badge(text)
                        .offset(y: -TC.MenuBar.badgeOverhang)
                }
            }
            // `compositingGroup` is what makes the destination-out blend
            // modes in `badge` cut into the mark rather than into the menu
            // bar.
            .compositingGroup()
            // Room for the overhang, so the status item's frame -- which is
            // sized from this view -- does not clip the capsule. Padded on
            // both edges, and in every state, so the mark sits at the same
            // height whether or not there is a count.
            .padding(.vertical, TC.MenuBar.badgeOverhang)

            // The two non-numeric states keep their glyph beside the mark:
            // a triangle or a pause bar is not a thing to knock out of a
            // 10pt capsule.
            switch state {
            case .attention:
                Image(systemName: "exclamationmark.triangle")
                    .imageScale(.small)
            case .paused:
                Image(systemName: "pause.fill")
                    .imageScale(.small)
            case .count, .idle:
                EmptyView()
            }
        }
        .foregroundStyle(.primary)
    }

    private func badge(_ text: String) -> some View {
        Text(text)
            .font(TC.Font_.menuBarBadge)
            .monospacedDigit()
            .padding(.horizontal, TC.MenuBar.badgeInset)
            .frame(minWidth: TC.MenuBar.badgeHeight)
            .frame(height: TC.MenuBar.badgeHeight)
            .foregroundStyle(colorScheme == .dark ? Color.black : Color.white)
            .background(Capsule().fill(colorScheme == .dark ? Color.white : Color.black))
            .background(
                Capsule()
                    .fill(.primary)
                    .padding(-TC.MenuBar.badgeHalo)
                    .blendMode(.destinationOut)
            )
    }
}

struct MenuBarContent: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openWindow) private var openWindow

    /// Where the window is pointed before it is raised. Optional so the
    /// screenshot hook can render this menu without one; a menu with no
    /// navigation still opens the window, it just opens it where it was.
    var navigation: MainWindowNavigation?

    var body: some View {
        Group {
            waitingSection
            Divider()
            privateInferenceSection
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

    // MARK: - Answering model calls on this computer

    /// The state line and one action, in the shape `waitingSection` uses:
    /// what is true first, then the one thing to do about it.
    ///
    /// The action is asymmetric and that is the whole point of this section.
    /// See `privateInferenceTrayAction`.
    ///
    /// Renders nothing at all when the words did not arrive, for the reason
    /// `AppModel.privateInferenceCopy` gives.
    @ViewBuilder
    private var privateInferenceSection: some View {
        if let copy = model.privateInferenceCopy {
            let state = model.privateInferenceState
            let on = model.daemonSettings?.privateInferenceOn ?? false
            // The glyph comes from the reported state, never from the
            // switch below it. They differ exactly when it matters -- a
            // listener that refused to start leaves the switch on -- and a
            // menu bar is read at a glance, which is where a green mark
            // over a dead listener does the most damage.
            Label(
                PrivateInferenceSurface.stateLine(
                    state, copy: copy, calls: model.privateInferenceCalls),
                systemImage: Self.privateInferenceSymbol(
                    state, calls: model.privateInferenceCalls)
            )
            // One action, not a switch. A switch in a menu is symmetrical
            // and these two directions are not: see
            // `privateInferenceTrayAction`. Both words are the Rust's.
            Button {
                Self.performPrivateInferenceTray(
                    on: on,
                    turnOff: { model.applyPrivateInference(false) },
                    open: { openPrivateInference() }
                )
            } label: {
                Text(Self.privateInferenceTrayLabel(on: on, copy: copy))
            }
            // Only the direction that writes can be busy. Opening a window is
            // always available, and a menu that greyed the way in while the
            // daemon was quiet would leave nowhere to go.
            .disabled(
                Self.privateInferenceTrayAction(on: on) == .stopAnswering
                    && (model.privateInferenceBusy
                        || model.daemonSettings?.privateInference == nil))
            Divider()
        }
    }

    /// What a press on the model-calls row does.
    ///
    /// Two values rather than a toggle, because the two directions are not
    /// symmetrical. Turning it OFF only ever reduces what this computer will
    /// answer, so it is safe from a menu with nothing else on screen. Turning
    /// it ON changes what anything else running on this computer may send
    /// through, charged to the contributor's own accounts, and the sentence
    /// saying so (`offer_exposure`) is the reason model calls became a
    /// top-level destination rather than a settings switch. A menu press that
    /// enabled it would route around that sentence, so out here the on
    /// direction opens the screen and writes nothing.
    enum PrivateInferenceTrayAction: Equatable {
        /// Stop answering. The one write this menu may make.
        case stopAnswering
        /// Raise the window at the destination. Writes nothing.
        case openDestination
    }

    /// The action for one switch position.
    ///
    /// Reads the switch and not the tone, deliberately: a listener that
    /// refused to start still leaves something to turn off, and a row that
    /// vanished because nothing was running would strand the one press this
    /// menu exists to offer.
    static func privateInferenceTrayAction(on: Bool) -> PrivateInferenceTrayAction {
        on ? .stopAnswering : .openDestination
    }

    /// The row's words, both of them the Rust's. The off direction's ends in
    /// an ellipsis, which is this platform's convention for an item that
    /// opens something rather than acting -- see the copy module.
    static func privateInferenceTrayLabel(on: Bool, copy: PrivateInferenceCopy) -> String {
        on ? copy.trayTurnOff : copy.trayOpenToTurnOn
    }

    /// Runs the action, with the two things it can do handed in so a test can
    /// assert the one that matters: while it is off, `turnOff` is never
    /// called. That is the safety claim, and it is checked directly rather
    /// than inferred from which sentence the row was showing.
    ///
    /// The stop direction raises the destination as well as writing, which
    /// is not decoration. The write goes through
    /// `PrivateInferenceSurface.settingsParams`, and that carries
    /// `private_inference_offer_seen` -- a marker whose entire contract is
    /// that it records an *asking*. A menu press showing no sentence at all
    /// would record a question nobody was asked and permanently suppress the
    /// first-run offer for a contributor who had enabled the switch out of
    /// band. On Windows and in GTK the switch only exists on the screen that
    /// carries `offer_exposure`, so their off press always shows those
    /// words; raising the destination here is what gives this shell the same
    /// property, and the destination shows `offer_exposure` whether or not
    /// the offer card is still due.
    static func performPrivateInferenceTray(
        on: Bool, turnOff: () -> Void, open: () -> Void
    ) {
        switch privateInferenceTrayAction(on: on) {
        case .stopAnswering:
            turnOff()
            open()
        case .openDestination: open()
        }
    }

    /// The glyph beside the state line, as a pure function of the reported
    /// state so a test can ask for it without a menu.
    ///
    /// Takes no switch, deliberately: a function that cannot see what was
    /// asked for cannot paint it as what is true.
    static func privateInferenceSymbol(
        _ state: PrivateInferenceState, calls: PrivateInferenceCalls
    ) -> String {
        PrivateInferenceIndicator.palette(
            PrivateInferenceSurface.tone(state, calls: calls)
        ).symbol
    }

    @ViewBuilder
    private var healthSection: some View {
        if let health = model.health {
            Text(health.title)
            Text(health.detail.replacingOccurrences(of: "\n", with: " "))
        }
        if let budget = model.budgetHealth {
            Text(budget.title)
            Text(budget.detail.replacingOccurrences(of: "\n", with: " "))
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

    /// The window, at the model-calls destination.
    ///
    /// The destination and not just the window: the point of opening rather
    /// than acting is that the sentence about what answering model calls
    /// exposes is on screen when the switch is, and a window raised at
    /// whatever screen it was last on would not carry it.
    private func openPrivateInference() {
        navigation?.section = .privateInference
        openMain()
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
