import SwiftUI
import TCShellCore

struct MainWindowView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(ComputeModel.self) private var compute
    let navigation: MainWindowNavigation
    private var section: Section { navigation.section }

    enum Section: String, CaseIterable, Identifiable {
        case queue = "Waiting"
        case history = "History"
        case settings = "Settings"
        case compute = "compute"
        // The raw value is an identity, not a label: this destination takes
        // its words from the Rust copy payload, the way `compute` does.
        case privateInference = "privateInference"
        var id: String { rawValue }

        /// The nav glyph, drawn from the design's own path data rather than
        /// taken from SF Symbols: these three are part of the mark's family
        /// and a system symbol brings its own weight and optical size.
        ///
        /// `queue` and `compute` already share `.monitor`. A third reuse
        /// would leave three of five rows with one glyph and make the
        /// sidebar unreadable at a glance, so this destination gets its own.
        var glyph: MacGlyphs {
            switch self {
            case .queue: return .monitor
            case .history: return .clock
            case .settings: return .gear
            case .compute: return .monitor
            case .privateInference: return .exchange
            }
        }

        /// The in-app shortcut: Cmd-N for the Nth row of the sidebar, taken
        /// from the row's own position so the shortcut and the sidebar can
        /// never disagree about which is which.
        ///
        /// In-app only. A global, system-wide hotkey is deliberately out of
        /// scope: it would need an accessibility grant this app does not
        /// ask for, and it would fire while another app owned the keyboard.
        var shortcut: Character? {
            guard let index = Self.allCases.firstIndex(of: self), index < 9 else { return nil }
            return Character("\(index + 1)")
        }

        /// What the section is for, in the window's subtitle. A person who
        /// opened this app from a notification needs to know where they are
        /// before they need to know what to do.
        var subtitle: String {
            switch self {
            case .queue: return "Nothing is sent unless you say so."
            case .history: return "What you have contributed, and what is still being reviewed."
            case .settings: return "What this machine watches, and what your traces are allowed to do."
            case .compute: return ""
            // Like `compute`: the real subtitle is the Rust's, read through
            // the copy payload in `subtitle(_:compute:privateInference:)`.
            case .privateInference: return ""
            }
        }
    }

    var body: some View {
        shell
    }

    /// Only trace destinations pass through enrollment and session-root gates.
    /// The sidebar and Compute remain reachable in every watcher startup state.
    @ViewBuilder
    private var traceContent: some View {
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
        case .needsRoots, .running:
            // One branch for BOTH states, deliberately: `.needsRoots` is the
            // start of a fresh install's onboarding, not a notice, and the
            // roots screen it leads to flips `startup` to `.running` when
            // the daemon starts. Two `case` arms would be two view
            // identities, and the coordinator's `@State step` would be
            // thrown away at exactly that flip -- the same reason the two
            // enrolment states below share one `if` branch.
            //
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
            if model.requiresOnboarding {
                OnboardingCoordinatorView(
                    startAt: model.status.loggedIn ? .consent : .welcome,
                    onComplete: { model.markOnboardingComplete() }
                )
                .tcScreen()
                .onAppear { model.refreshAll() }
            } else {
                traceDestination
                    .onAppear { model.refreshAll() }
            }
        }
    }

    /// Sidebar plus a content header. Without them the window read as a
    /// preview canvas: content floating in an unowned field with nothing to
    /// anchor it and nowhere to put a global control.
    ///
    /// The header is drawn rather than taken from the toolbar. The design
    /// puts the screen's title, the sentence that says what the screen
    /// promises, and the two watch controls on one banded row directly above
    /// the content, and a `ToolbarItem` cannot sit next to prose.
    private var shell: some View {
        NavigationSplitView {
            sidebar
                .navigationSplitViewColumnWidth(184)
        } detail: {
            VStack(spacing: 0) {
                if navigation.displaysCompute {
                    contentHeader
                    ComputeView(model: compute)
                } else {
                    if model.traceNavigationReady { contentHeader }
                    traceContent
                }
            }
            // The brand ground stops here. The sidebar and the title bar
            // above it stay system materials, which is what keeps this
            // looking like a Mac window rather than a web page in one.
            .tcScreen()
        }
    }

    @ViewBuilder
    private var traceDestination: some View {
        switch section {
        case .queue: QueueView()
        case .history: HistoryView()
        case .settings: SettingsView(navigation: navigation)
        case .compute: EmptyView()
        case .privateInference: PrivateInferenceView()
        }
    }

    private func title(_ item: Section) -> String {
        Self.title(item, compute: compute.copy, privateInference: model.privateInferenceCopy)
    }

    private func subtitle(_ item: Section) -> String {
        Self.subtitle(item, compute: compute.copy, privateInference: model.privateInferenceCopy)
    }

    /// The nav label, from the Rust for the two destinations whose words the
    /// Rust owns and from the raw value for the three whose words this shell
    /// still authors.
    ///
    /// Static and copy-taking so a test can ask it what it would render
    /// without standing up a window, and so the answer for a payload that
    /// never arrived is no words at all -- never the raw value, which on
    /// this destination is an identifier and not a label.
    static func title(
        _ item: Section, compute: ComputeCopy?, privateInference: PrivateInferenceCopy?
    ) -> String {
        switch item {
        case .compute: return compute?.destination ?? ""
        case .privateInference: return privateInference?.destination ?? ""
        case .queue, .history, .settings: return item.rawValue
        }
    }

    /// The line under the title, on the same terms as `title`.
    static func subtitle(
        _ item: Section, compute: ComputeCopy?, privateInference: PrivateInferenceCopy?
    ) -> String {
        switch item {
        case .compute: return compute?.subtitle ?? ""
        case .privateInference: return privateInference.map(\.subtitle) ?? ""
        case .queue, .history, .settings: return item.subtitle
        }
    }

    // MARK: - Sidebar

    /// A drawn sidebar rather than a `List`. The design's nav rows carry the
    /// product's own glyphs and its own selected treatment, and a system list
    /// row will render neither.
    private var sidebar: some View {
        VStack(alignment: .leading, spacing: 0) {
            // The window chrome's mark. macOS puts the traffic lights in this
            // strip and the system draws those, so the mark takes the space
            // beside them rather than a title bar of its own.
            HStack(spacing: TC.Space.s) {
                BrandMark(size: 16)
                Text("Trace Commons")
                    .font(TC.Font_.caption.weight(.semibold))
                    .foregroundStyle(TC.inkSecondary)
            }
            .padding(.horizontal, TC.Space.lg)
            .padding(.bottom, TC.Space.l)

            VStack(spacing: TC.Space.micro) {
                ForEach(Section.allCases) { item in
                    navRow(item)
                }
            }
            .padding(.horizontal, TC.Space.sm)
            Spacer(minLength: 0)
        }
        .padding(.top, TC.Space.s)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(TC.sidebarGround)
        .overlay(alignment: .trailing) {
            Rectangle().fill(TC.divider).frame(width: TC.Space.hairline)
        }
    }

    private func navRow(_ item: Section) -> some View {
        let selected = section == item
        let count = item == .queue ? model.decisionsOwed : 0
        // Beside the count, never instead of it: an icon meaning "some" is a
        // downgrade at exactly the scale that prompted the request. See
        // `QueueShieldState`.
        let shield: QueueShieldState = item == .queue
            ? QueueShieldState.state(
                waiting: model.decisionsOwed,
                nothingMatched: model.nothingMatchedCount,
                trimmed: model.awaitingDecision.filter(\.wasTrimmed).count
            )
            : .clear
        return Button {
            navigation.section = item
        } label: {
            HStack(spacing: TC.Space.s) {
                MacGlyph(
                    glyph: item.glyph,
                    size: 13,
                    color: Self.navGlyphColor(shield: shield, selected: selected)
                )
                Text(title(item))
                    .font(.system(size: 13, weight: selected ? .medium : .regular))
                    .foregroundStyle(TC.inkPrimary)
                Spacer(minLength: TC.Space.s)
                if count > 0 {
                    Text("\(count)")
                        .font(.system(size: 11, weight: .semibold))
                        .monospacedDigit()
                        .foregroundStyle(TC.inkSecondary)
                }
            }
            .padding(.horizontal, TC.Space.s)
            .padding(.vertical, 5)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: TC.Radius.control, style: .continuous)
                    .fill(selected ? TC.surfaceSelected : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(selected ? [.isButton, .isSelected] : .isButton)
        .accessibilityLabel(Self.navLabel(title(item), count: count, shield: shield))
    }

    /// The nav glyph's colour, with the shield's state taking precedence
    /// over selection: an item worth a second look says so whether or not it
    /// is the one on screen.
    private static func navGlyphColor(shield: QueueShieldState, selected: Bool) -> Color {
        switch shield {
        case .attention: return TC.goldText
        case .waiting: return TC.greenText
        case .clear: return selected ? TC.greenText : TC.inkSecondary
        }
    }

    private static func navLabel(_ name: String, count: Int, shield: QueueShieldState) -> String {
        guard count > 0 else { return name }
        let waiting = "\(name), \(count) waiting"
        return shield == .attention ? waiting + ", some worth a second look" : waiting
    }

    // MARK: - Content header

    /// Title, the promise, and the watch controls, on one banded row with a
    /// hairline under it.
    private var contentHeader: some View {
        HStack(alignment: .center, spacing: TC.Space.m) {
            VStack(alignment: .leading, spacing: 1) {
                Text(title(section))
                    .font(TC.Font_.screenTitle)
                    .foregroundStyle(TC.inkPrimary)
                Text(subtitle(section))
                    .font(TC.Font_.caption)
                    .foregroundStyle(TC.inkSecondary)
            }
            Spacer(minLength: TC.Space.m)
            if section != .compute {
                watchChip
                watchControl
            }
        }
        .padding(.horizontal, TC.Space.Header.horizontal)
        .padding(.vertical, TC.Space.Header.vertical)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(TC.groundTranslucent)
        .overlay(alignment: .bottom) {
            Rectangle().fill(TC.divider).frame(height: TC.Space.hairline)
        }
    }

    /// A permanent readout of whether this machine is watching at all. Paused
    /// is a state a person can forget they chose, so it is never left
    /// implicit -- and it is told in a glyph and words as well as a colour.
    private var watchChip: some View {
        HStack(spacing: TC.Space.xxs) {
            MacGlyph(
                glyph: model.status.paused ? .pauseBars : .eye,
                size: 11,
                color: model.status.paused ? TC.goldText : TC.inkSecondary
            )
            Text(model.status.paused ? "Paused" : "Watching")
                .font(TC.Font_.monoChip)
                .foregroundStyle(model.status.paused ? TC.goldText : TC.inkSecondary)
        }
        .padding(.horizontal, TC.Space.s)
        .padding(.vertical, TC.Space.micro)
        .overlay {
            Capsule().strokeBorder(
                model.status.paused ? TC.gold.opacity(TC.Border.chipAlpha) : TC.line,
                lineWidth: TC.Border.hairline
            )
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel(
            model.status.paused
                ? "Paused. Nothing is being queued or sent."
                : "Watching for finished sessions."
        )
        .fixedSize()
    }

    /// The one global control worth a permanent slot: a split-button, because
    /// pausing has a duration and the duration is the decision.
    @ViewBuilder
    private var watchControl: some View {
        if model.status.paused {
            Button { model.resume() } label: {
                Text("Resume watching").font(TC.Font_.labelControl)
            }
            .buttonStyle(.plain)
            .padding(.horizontal, TC.Space.sm)
            .padding(.vertical, TC.Space.xxs)
            .background(controlChrome)
            .help("Start noticing finished sessions again.")
            .fixedSize()
        } else {
            Menu {
                Button("For 1 hour") { model.pause(until: Date().addingTimeInterval(3600)) }
                Button("Until tomorrow morning") { model.pause(until: Format.tomorrowMorning()) }
                Button("Until I turn it back on") { model.pause(until: nil) }
            } label: {
                HStack(spacing: TC.Space.xxs) {
                    MacGlyph(glyph: .pauseBars, size: 11, color: TC.inkSecondary)
                    Text("Pause").font(TC.Font_.labelControl)
                    MacGlyph(glyph: .chevronDown, size: 9, color: TC.inkSecondary)
                }
                .padding(.horizontal, TC.Space.sm)
                .padding(.vertical, TC.Space.xxs)
                .background(controlChrome)
                .contentShape(Rectangle())
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .help("Stop noticing finished sessions.")
        }
    }

    private var controlChrome: some View {
        RoundedRectangle(cornerRadius: TC.Radius.control, style: .continuous)
            .fill(TC.surface)
            .overlay {
                RoundedRectangle(cornerRadius: TC.Radius.control, style: .continuous)
                    .strokeBorder(TC.line, lineWidth: TC.Border.hairline)
            }
    }
}

// MARK: - Keyboard

/// The View menu's navigation commands, and the one shortcut that acts
/// rather than navigates.
///
/// In-app only, by construction: these are menu items, so they fire when
/// this app is frontmost and never while another app owns the keyboard. A
/// global, system-wide hotkey would need a registration this app does not
/// make and is deliberately out of scope.
///
/// It authors no wording. Every title is `MainWindowView.title` -- the raw
/// value for the three destinations this shell still names, and the Rust's
/// own words for the two it does not.
struct MainWindowCommands: Commands {
    @ObservedObject var model: AppModel
    var compute: ComputeModel
    var navigation: MainWindowNavigation

    /// Cmd-N for the Nth destination.
    static let destinationModifiers: EventModifiers = [.command]
    /// Cmd-Shift-M for the switch. Shifted so it cannot be reached by the
    /// same reflex that reaches a destination: this one changes what the
    /// machine is doing, the other five only change what is on screen.
    static let toggleModifiers: EventModifiers = [.command, .shift]
    static let toggleKey: Character = "m"

    var body: some Commands {
        CommandGroup(after: .sidebar) {
            ForEach(MainWindowView.Section.allCases) { item in
                destination(item)
            }
            Divider()
            toggle
        }
    }

    @ViewBuilder
    private func destination(_ item: MainWindowView.Section) -> some View {
        let label = MainWindowView.title(
            item, compute: compute.copy, privateInference: model.privateInferenceCopy)
        // A destination whose words never arrived gets no menu item rather
        // than a blank one.
        if !label.isEmpty {
            Button(label) {
                navigation.section = item
                OpenMainWindow.request()
            }
            .keyboardShortcut(
                item.shortcut.map { KeyEquivalent($0) } ?? "0",
                modifiers: Self.destinationModifiers)
        }
    }

    /// The switch, under the same asymmetry the menu bar row follows.
    ///
    /// This is a menu, and a menu press must not enable answering: the on
    /// direction raises the window at the destination, where
    /// `offer_exposure` is, and writes nothing. It shares
    /// `MenuBarContent.performPrivateInferenceTray` with the menu-bar row
    /// rather than restating the rule, because two statements of one rule is
    /// how this shortcut came to disagree with that row in the first place.
    ///
    /// The off direction *sets* false; it does not invert. An inverted press
    /// against a stale switch position is an enable.
    @ViewBuilder
    private var toggle: some View {
        if let copy = model.privateInferenceCopy {
            let on = model.daemonSettings?.privateInferenceOn ?? false
            Button(MenuBarContent.privateInferenceTrayLabel(on: on, copy: copy)) {
                MenuBarContent.performPrivateInferenceTray(
                    on: on,
                    turnOff: { model.applyPrivateInference(false) },
                    open: {
                        navigation.section = .privateInference
                        OpenMainWindow.request()
                    })
            }
            .keyboardShortcut(KeyEquivalent(Self.toggleKey), modifiers: Self.toggleModifiers)
            // Only the writing direction can be unavailable. Navigation stays
            // reachable whatever the daemon is doing.
            .disabled(
                on && (model.privateInferenceBusy || model.daemonSettings?.privateInference == nil))
        }
    }
}

// MARK: - Glyphs

/// One of the design's glyphs, stated on its own 16-unit grid and stroked at
/// whatever size the call site asks for.
///
/// The paths are transcribed from `design-import/DESIGN-SPEC.md` rather than
/// approximated with SF Symbols, for the same reason the mark is drawn instead
/// of shipped as an asset: these are the product's own line weights, and a
/// system symbol substitutes its own.
fileprivate struct MacGlyph: View {
    let glyph: MacGlyphs
    var size: CGFloat = 13
    /// Stroke width in grid units, converted against `size`.
    var stroke: CGFloat = 1.4
    var color: Color

    var body: some View {
        GlyphShape(glyph: glyph)
            .stroke(
                color,
                style: StrokeStyle(
                    lineWidth: stroke * size / 16,
                    lineCap: .round,
                    lineJoin: .round
                )
            )
            .frame(width: size, height: size)
            .accessibilityHidden(true)
    }
}

fileprivate struct GlyphShape: Shape {
    let glyph: MacGlyphs

    func path(in rect: CGRect) -> Path {
        var path = Path()
        glyph.draw(into: &path)
        let scale = min(rect.width, rect.height) / 16
        return path
            .applying(CGAffineTransform(scaleX: scale, y: scale))
            .offsetBy(dx: rect.minX, dy: rect.minY)
    }
}

enum MacGlyphs: Equatable {
    case monitor
    case clock
    case gear
    case eye
    case pauseBars
    case chevronDown
    case warningTriangle
    case exchange

    func draw(into path: inout Path) {
        switch self {
        case .monitor: Self.monitor(&path)
        case .clock: Self.clock(&path)
        case .gear: Self.gear(&path)
        case .eye: Self.eye(&path)
        case .pauseBars: Self.pauseBars(&path)
        case .chevronDown: Self.chevronDown(&path)
        case .warningTriangle: Self.warningTriangle(&path)
        case .exchange: Self.exchange(&path)
        }
    }

    /// `<rect x=2 y=3.5 w=12 h=9.5 rx=1.5/><path d="M2 9h3l1.5 2h3L11 9h3"/>`
    static func monitor(_ path: inout Path) {
        path.addRoundedRect(
            in: CGRect(x: 2, y: 3.5, width: 12, height: 9.5),
            cornerSize: CGSize(width: 1.5, height: 1.5)
        )
        path.move(to: CGPoint(x: 2, y: 9))
        path.addLine(to: CGPoint(x: 5, y: 9))
        path.addLine(to: CGPoint(x: 6.5, y: 11))
        path.addLine(to: CGPoint(x: 9.5, y: 11))
        path.addLine(to: CGPoint(x: 11, y: 9))
        path.addLine(to: CGPoint(x: 14, y: 9))
    }

    /// `<circle cx=8 cy=8 r=5.7/><path d="M8 4.8V8l2.3 1.4"/>`
    static func clock(_ path: inout Path) {
        path.addEllipse(in: CGRect(x: 2.3, y: 2.3, width: 11.4, height: 11.4))
        path.move(to: CGPoint(x: 8, y: 4.8))
        path.addLine(to: CGPoint(x: 8, y: 8))
        path.addLine(to: CGPoint(x: 10.3, y: 9.4))
    }

    /// `<circle cx=8 cy=8 r=2.2/>` plus eight spokes.
    static func gear(_ path: inout Path) {
        path.addEllipse(in: CGRect(x: 5.8, y: 5.8, width: 4.4, height: 4.4))
        let spokes: [(CGPoint, CGPoint)] = [
            (CGPoint(x: 8, y: 1.6), CGPoint(x: 8, y: 3.8)),
            (CGPoint(x: 8, y: 12.2), CGPoint(x: 8, y: 14.4)),
            (CGPoint(x: 1.6, y: 8), CGPoint(x: 3.8, y: 8)),
            (CGPoint(x: 12.2, y: 8), CGPoint(x: 14.4, y: 8)),
            (CGPoint(x: 3.5, y: 3.5), CGPoint(x: 5.1, y: 5.1)),
            (CGPoint(x: 10.9, y: 10.9), CGPoint(x: 12.5, y: 12.5)),
            (CGPoint(x: 12.5, y: 3.5), CGPoint(x: 10.9, y: 5.1)),
            (CGPoint(x: 5.1, y: 10.9), CGPoint(x: 3.5, y: 12.5)),
        ]
        for spoke in spokes {
            path.move(to: spoke.0)
            path.addLine(to: spoke.1)
        }
    }

    /// The Watching chip's eye, lid and pupil.
    static func eye(_ path: inout Path) {
        path.move(to: CGPoint(x: 1.5, y: 8))
        path.addCurve(
            to: CGPoint(x: 8, y: 3.7),
            control1: CGPoint(x: 3, y: 5.2),
            control2: CGPoint(x: 5.2, y: 3.7)
        )
        path.addCurve(
            to: CGPoint(x: 14.5, y: 8),
            control1: CGPoint(x: 10.8, y: 3.7),
            control2: CGPoint(x: 13, y: 5.2)
        )
        path.addCurve(
            to: CGPoint(x: 8, y: 12.3),
            control1: CGPoint(x: 13, y: 10.8),
            control2: CGPoint(x: 10.8, y: 12.3)
        )
        path.addCurve(
            to: CGPoint(x: 1.5, y: 8),
            control1: CGPoint(x: 5.2, y: 12.3),
            control2: CGPoint(x: 3, y: 10.8)
        )
        path.closeSubpath()
        path.addEllipse(in: CGRect(x: 5.9, y: 5.9, width: 4.2, height: 4.2))
    }

    /// `M5.8 4v8M10.2 4v8`
    static func pauseBars(_ path: inout Path) {
        path.move(to: CGPoint(x: 5.8, y: 4))
        path.addLine(to: CGPoint(x: 5.8, y: 12))
        path.move(to: CGPoint(x: 10.2, y: 4))
        path.addLine(to: CGPoint(x: 10.2, y: 12))
    }

    /// Two arrows, one out and one back, on the same 16-unit grid as the
    /// rest: a call leaving and an answer returning.
    ///
    /// Not a lock and not a shield. Answering a model call on this computer
    /// moves where the call is answered; it does not make the call private,
    /// and a padlock in the sidebar would promise exactly the thing the
    /// wording is careful not to.
    static func exchange(_ path: inout Path) {
        path.move(to: CGPoint(x: 2.5, y: 6))
        path.addLine(to: CGPoint(x: 13.5, y: 6))
        path.move(to: CGPoint(x: 11, y: 3.5))
        path.addLine(to: CGPoint(x: 13.5, y: 6))
        path.addLine(to: CGPoint(x: 11, y: 8.5))
        path.move(to: CGPoint(x: 13.5, y: 10))
        path.addLine(to: CGPoint(x: 2.5, y: 10))
        path.move(to: CGPoint(x: 5, y: 7.5))
        path.addLine(to: CGPoint(x: 2.5, y: 10))
        path.addLine(to: CGPoint(x: 5, y: 12.5))
    }

    /// `m5 6.5 3 3 3-3` -- the split-button's chevron.
    static func chevronDown(_ path: inout Path) {
        path.move(to: CGPoint(x: 5, y: 6.5))
        path.addLine(to: CGPoint(x: 8, y: 9.5))
        path.addLine(to: CGPoint(x: 11, y: 6.5))
    }

    /// `M8 2.2 14.6 13.4H1.4Z` plus the bar and the dot.
    static func warningTriangle(_ path: inout Path) {
        path.move(to: CGPoint(x: 8, y: 2.2))
        path.addLine(to: CGPoint(x: 14.6, y: 13.4))
        path.addLine(to: CGPoint(x: 1.4, y: 13.4))
        path.closeSubpath()
        path.move(to: CGPoint(x: 8, y: 6.6))
        path.addLine(to: CGPoint(x: 8, y: 9.6))
        path.addEllipse(in: CGRect(x: 7.5, y: 11.2, width: 1, height: 1))
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
    var onAction: (() -> Void)? = nil

    private var tone: TC.Tone {
        health.severity == .actionable ? .attention : .neutral
    }

    var body: some View {
        HStack(alignment: .top, spacing: TC.Space.m) {
            // A glyph, not a coloured dot: the severity has to survive
            // greyscale and it has to reach VoiceOver.
            MacGlyph(glyph: .warningTriangle, size: 14, color: tone.color)
                .padding(.top, 1)
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                Text(health.title)
                    .font(TC.Font_.cardTitle)
                    .foregroundStyle(TC.inkPrimary)
                Text(health.detail)
                    .font(TC.Font_.meta)
                    .foregroundStyle(TC.inkSecondary)
                    .lineSpacing(TC.Font_.LineHeight.spacing(for: 11, TC.Font_.LineHeight.caption))
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: TC.Space.m)
            if let action = health.actionTitle {
                Button(action) { onAction?() }
                    .disabled(onAction == nil)
                    .lineLimit(1)
                    .fixedSize()
                    .help(onAction == nil ? "Not wired up in this build." : action)
            }
        }
        .padding(.vertical, TC.Space.m)
        .padding(.horizontal, TC.Space.md)
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
