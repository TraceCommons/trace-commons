import AppKit
import SwiftUI
import TCUpdates

/// What this machine is doing, and what permissions traces carry.
///
/// The consent list comes from `consent_options`, never hardcoded, and
/// nothing optional is pre-checked. `public_attribution` is visually
/// separated because it grants no data use at all.
///
/// Layout follows `design-import/DESIGN-SPEC.md` §5.4 (`1d`): the standard
/// macOS content padding (`18 22 22`), an 18pt gap between sections, and a
/// prose column kept deliberately narrow. §5.4 is truncated in the imported
/// source -- it ends mid-attribute just after the Startup section -- so only
/// the Connection and Startup sections have drawn geometry to follow. The
/// consent list, "Watching" and "Projects" keep the treatment they already
/// had, expressed in tokens; nothing has been invented to fill the gap.
///
/// The public-profile block below (§5.6) and the go-public dialog (§5.7) are
/// rendered in the community brand rather than in `TC`. That seam is the
/// point: per §7.3 the black frame is the exact boundary of what becomes
/// public. See `CommunityBrand` for why those values are not `TC` tokens.
struct SettingsView: View {
    @EnvironmentObject private var model: AppModel
    @ObservedObject private var updates = UpdateController.shared

    // Read fresh on appear rather than cached across the view's lifetime:
    // the user can flip this in System Settings -> General -> Login Items
    // while this window is open, and a value captured once at init would
    // then claim a state that is no longer true. See `LoginItemManager`.
    @State private var loginItemState: LoginItemManager.State = LoginItemManager.currentState
    @State private var loginItemActionError: String?
    @State private var showingGoPublic = false

    /// Spec §5.4: the Settings content column is `max-width:520px` ("prose
    /// column, kept narrow on purpose"), narrower than the 660 that
    /// `TC.Measure.prose` carries for onboarding. There is no token for it,
    /// so it is stated here rather than widening a shared one.
    private static let proseColumn: CGFloat = 520

    var body: some View {
        ScrollView {
            // Spec §5.4 gap: 18 between sections (`TC.Space.lg`), not the
            // 28 this screen used before.
            VStack(alignment: .leading, spacing: TC.Space.lg) {
                connection
                loginItem
                updatesSection
                consent
                publicProfile
                watching
                projects
            }
            .padding(.top, TC.Space.Content.top)
            .padding(.horizontal, TC.Space.Content.horizontal)
            .padding(.bottom, TC.Space.Content.bottom)
            .tcColumn(Self.proseColumn)
        }
        .tcScreen()
        .onAppear {
            loginItemState = LoginItemManager.currentState
            // Sparkle moves this on its own schedule; a Settings window left
            // open would otherwise keep showing the time it had at open.
            updates.refreshLastCheckDate()
        }
        .sheet(isPresented: $showingGoPublic) {
            GoPublicDialog(onDismiss: { showingGoPublic = false })
        }
    }

    // MARK: - Connection (spec §5.4)

    private var connection: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: "Connection")
            if model.status.loggedIn {
                TCTag(text: "Connected", tone: .clear, symbol: "link")
            } else {
                VStack(alignment: .leading, spacing: TC.Space.xs) {
                    TCTag(text: "Not connected", tone: .attention, symbol: "link.badge.plus")
                    Text("Sessions are being queued, but nothing can be sent.")
                        .font(TC.Font_.meta)
                        .foregroundStyle(.secondary)
                }
            }
            if let settings = model.daemonSettings {
                // Booleans only. The contract reports the credential and both
                // session roots as configured-or-not, and this view has
                // nowhere to put a value even if it were sent one.
                checkRow("Claude Code sessions folder set", settings.claudeRootConfigured)
                checkRow("Codex sessions folder set", settings.codexRootConfigured)
                checkRow("Extra privacy scan configured", settings.nearAIConfigured)
            }
        }
    }

    // MARK: - Startup (spec §5.4)

    /// Reflects the live `SMAppService.mainApp.status`, not a locally cached
    /// bool -- see `loginItemState`'s doc comment. `.requiresApproval` is
    /// rendered as guidance, not an error: it is the normal result of the
    /// user not yet approving the app in System Settings, or having denied
    /// it there, and retrying `register()` again would not change that.
    private var loginItem: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: "Startup")
            switch loginItemState {
            case .enabled:
                startupToggle(isOn: Binding(
                    get: { true },
                    set: { newValue in if !newValue { setLoginItem(enabled: false) } }
                ))
            case .notRegistered, .notFound:
                startupToggle(isOn: Binding(
                    get: { false },
                    set: { newValue in if newValue { setLoginItem(enabled: true) } }
                ))
            case .requiresApproval:
                Text("Waiting on approval in System Settings.")
                    .font(TC.Font_.body)
                Text("""
                Turn it on in System Settings -> General -> Login Items to let \
                Trace Commons start automatically.
                """)
                .font(TC.Font_.caption)
                .foregroundStyle(.secondary)
            }
            if let loginItemActionError {
                Text(loginItemActionError)
                    .font(TC.Font_.caption)
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// Spec §6.8 draws this as a hand-built 34x20 track with a 16x16 knob,
    /// filled `#178F70` when on. The system switch tinted with the same green
    /// is that drawing, at the platform's own metrics, and it keeps the
    /// keyboard and VoiceOver behaviour a hand-drawn track would have to
    /// re-earn -- which is the same rule `DesignSystem.swift` states for the
    /// rest of the window chrome.
    private func startupToggle(isOn: Binding<Bool>) -> some View {
        Toggle("Start Trace Commons when you log in", isOn: isOn)
            .toggleStyle(.switch)
            .tint(TC.green)
            .font(TC.Font_.body)
    }

    private func setLoginItem(enabled: Bool) {
        loginItemActionError = nil
        if enabled {
            switch LoginItemManager.register() {
            case .enabled, .requiresApproval:
                break
            case .failed(let message):
                loginItemActionError = "Couldn't turn this on: \(message)"
            }
        } else {
            if case .failed(let message) = LoginItemManager.unregister() {
                loginItemActionError = "Couldn't turn this off: \(message)"
            }
        }
        loginItemState = LoginItemManager.currentState
    }

    // MARK: - Updates

    /// Version, update state, and -- when Homebrew owns this copy -- the one
    /// command that actually works.
    ///
    /// The Homebrew branch is not an apology for a missing feature. Homebrew
    /// placed these bytes and Homebrew replaces them; an app that offered a
    /// "Check Now" button here would be offering to fight the package
    /// manager over the same file.
    private var updatesSection: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCSectionHeader(title: "Updates")

            HStack(spacing: TC.Space.s) {
                TCFieldLabel("Version")
                Text(updates.currentVersion)
                    .font(TC.Font_.ledger)
                    .textSelection(.enabled)
            }

            switch updates.mode {
            case .selfUpdating:
                TCTag(text: "Checks daily", tone: .clear, symbol: "arrow.triangle.2.circlepath")
                Text(lastCheckSentence)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                // Deliberately does NOT claim the download already happened.
                // With SUAutomaticallyUpdate false, Sparkle's stock driver
                // finds the update in the background and then asks; the
                // download follows the yes. Copy that promised an
                // already-downloaded update would be describing a
                // configuration this app does not ship.
                Text("""
                    Trace Commons looks for new versions on its own. Nothing on \
                    disk changes until you say yes.
                    """)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button("Check Now") { updates.checkNow() }
                    .buttonStyle(.bordered)
                    .disabled(!updates.canCheckNow)

            case .managedByHomebrew(let command):
                TCTag(text: "Updates managed by Homebrew", tone: .held, symbol: "shippingbox")
                Text("""
                    Homebrew installed this copy, so Homebrew replaces it. Run \
                    this in a terminal:
                    """)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                HStack(spacing: TC.Space.s) {
                    Text(command)
                        .font(TC.Font_.ledger)
                        .textSelection(.enabled)
                        .padding(TC.Space.s)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .tcCard()
                    Button("Copy") {
                        NSPasteboard.general.clearContents()
                        NSPasteboard.general.setString(command, forType: .string)
                    }
                    .buttonStyle(.bordered)
                }

            case .disabled(let reason):
                TCTag(text: "Updates unavailable", tone: .refused, symbol: "arrow.down.circle")
                Text(disabledSentence(reason))
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var lastCheckSentence: String {
        guard let date = updates.lastCheckDate else {
            return "Not checked yet on this machine."
        }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .full
        return "Last checked \(formatter.localizedString(for: date, relativeTo: Date()))."
    }

    /// Turns the policy's stable label into a sentence. The label itself is
    /// what gets logged; this is what a person reads.
    private func disabledSentence(_ reason: String) -> String {
        switch reason {
        case UpdatePolicy.noFeedReason:
            return """
                This build has no update feed configured, so it will not look \
                for new versions. Development builds are like this. Install \
                from a release DMG to receive updates.
                """
        case UpdatePolicy.insecureFeedReason:
            return """
                This build's update feed is not HTTPS, so it has been refused. \
                Reinstall from a release DMG.
                """
        default:
            return "Updates are turned off for this build."
        }
    }

    // MARK: - Consent

    /// Scopes that grant no data use at all -- keyed off the daemon's
    /// `grants_data_use`, never off a scope name. This is the group the
    /// design calls "List my handle publicly": being on it is attribution
    /// and nothing more, which is why it is separated from the real
    /// data-use scopes and why the public-profile panel keys off it.
    private var creditScopes: [ConsentScope] {
        model.consentScopes.filter { !$0.alwaysOn && !$0.grantsDataUse }
    }

    /// Whether this contributor is on the public roster, derived from the
    /// daemon's granted scope list rather than from any local flag.
    private var listedPublicly: Bool {
        let granted = Set(model.status.consentScopes)
        return creditScopes.contains { granted.contains($0.name) }
    }

    private var consent: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: "How may your traces be used?")
            Text("Applies to traces you send from now on.")
                .font(TC.Font_.meta)
                .foregroundStyle(.secondary)

            let granted = Set(model.status.consentScopes)
            let alwaysOn = model.consentScopes.filter(\.alwaysOn)
            let optional = model.consentScopes.filter { !$0.alwaysOn && $0.grantsDataUse }

            if !alwaysOn.isEmpty {
                TCFieldLabel("Always included")
                ForEach(alwaysOn) { scope in
                    scopeRow(scope, checked: true, alwaysOn: true)
                }
            }
            if !optional.isEmpty {
                TCFieldLabel("Optional — each one lets your traces do more")
                ForEach(optional) { scope in
                    scopeRow(scope, checked: granted.contains(scope.name), alwaysOn: false)
                }
            }
            if !creditScopes.isEmpty {
                // Visually separated: it grants no data use at all, and
                // listing it beside four real scopes misleads both ways.
                TCFieldLabel("Credit")
                ForEach(creditScopes) { scope in
                    scopeRow(scope, checked: granted.contains(scope.name), alwaysOn: false)
                }
                if !listedPublicly {
                    // The one door into the community-brand surface. Going
                    // public is a consent dialog, not a toggle flip (§5.7),
                    // so the private tool's own button opens it and the
                    // foreign visual language starts at the sheet's edge.
                    Button("Go public") { showingGoPublic = true }
                        .buttonStyle(.bordered)
                        .font(TC.Font_.labelControl)
                }
            }
            Text("""
            Changing permissions needs an enrolled account, which this build does \
            not set up yet, so these show what is in force rather than offering to \
            change it. Nothing here is pre-selected on your behalf.
            """)
            .font(TC.Font_.caption)
            .foregroundStyle(.secondary)
        }
    }

    private func scopeRow(_ scope: ConsentScope, checked: Bool, alwaysOn: Bool) -> some View {
        HStack(alignment: .top, spacing: TC.Space.m) {
            TCReadGateCheckbox(checked: checked)
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                HStack(spacing: TC.Space.s) {
                    Text(ScopeCopy.title(for: scope.name, options: model.consentScopes))
                        .font(TC.Font_.cardTitle)
                    if alwaysOn {
                        TCTag(text: "always on", tone: .clear, symbol: "lock")
                    }
                }
                Text(scope.description)
                    .font(TC.Font_.body)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(TC.Space.m)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
        // `TCReadGateCheckbox` is drawn, and drawn shapes are hidden from
        // VoiceOver, so without this a scope announces its title and
        // description with no indication of whether it is granted. The row
        // becomes one element carrying that answer, which is the same shape
        // `ConsentScopesView` gives its own rows.
        .accessibilityElement(children: .combine)
        .accessibilityValue(checked ? "Granted" : "Not granted")
        .accessibilityAddTraits(checked ? [.isSelected] : [])
    }

    // MARK: - Public profile (spec §5.6)

    /// The brand panel that draws the exact boundary of what is public.
    ///
    /// It renders only while the daemon reports the attribution scope as
    /// granted, matching 2a's own rule: "Shown only while 'List my handle
    /// publicly' is on. Turn it off in Settings and this section disappears
    /// with it."
    ///
    /// The handle, the bio and the roster date are all mockup fixtures in the
    /// spec, and the daemon contract carries none of them -- `DaemonStatus`
    /// has no handle, `HistoryRollup` has no roster date, and there is no
    /// profile call to save a bio through. The frame, its labels and its byte
    /// counter are therefore drawn against empty values rather than against
    /// invented ones, and the controls that would write are disabled.
    /// Empty until the daemon carries a profile. Named rather than inlined so
    /// the byte counter is visibly derived from the value it counts, and so
    /// there is one place to bind when the contract grows these fields.
    private var publishedHandle: String { "" }
    private var publishedBio: String { "" }

    @ViewBuilder
    private var publicProfile: some View {
        if listedPublicly {
            VStack(alignment: .leading, spacing: TC.Space.sm) {
                communityBrandPanel {
                    Text("Your public profile".uppercased())
                        .font(CommunityBrand.Font_.displayPanel)
                        .tracking(CommunityBrand.Font_.displayPanelTracking)
                        .foregroundStyle(CommunityBrand.ink)

                    profileField(label: "Handle", value: publishedHandle, mono: true, minHeight: nil)

                    VStack(alignment: .leading, spacing: TC.Space.xs) {
                        profileField(
                            label: "Bio — 280 bytes, plaintext, no HTML",
                            value: publishedBio,
                            mono: false,
                            minHeight: 56
                        )
                        // Counted off the value above, not the mockup's
                        // "74/280": a counter that does not count is worse
                        // than no counter. Bytes, because the limit is
                        // stated in bytes.
                        Text("\(publishedBio.utf8.count)/280")
                            .font(CommunityBrand.Font_.labelMono)
                            .tracking(CommunityBrand.Font_.monoTracking)
                            .foregroundStyle(CommunityBrand.muted)
                            .frame(maxWidth: .infinity, alignment: .trailing)
                    }

                    HStack(spacing: TC.Space.sm) {
                        Button("Save profile") {}
                            .buttonStyle(CommunityBrandButtonStyle(fill: CommunityBrand.accent))
                            .disabled(true)
                        Button("Leave the roster") {}
                            .buttonStyle(CommunityBrandButtonStyle(fill: CommunityBrand.paper))
                            .disabled(true)
                    }
                }
                Text("""
                Attribution only — being listed grants no data use at all. Leaving \
                the roster removes you from future snapshots.
                """)
                .font(TC.Font_.caption)
                .lineSpacing(TC.Font_.LineHeight.spacing(for: 11, TC.Font_.LineHeight.caption))
                .foregroundStyle(.secondary)
                Text("""
                This build cannot read or change a public profile -- the daemon \
                reports no handle, bio or roster date yet -- so this shows what is \
                published rather than offering to edit it.
                """)
                .font(TC.Font_.caption)
                .foregroundStyle(.secondary)
            }
        }
    }

    /// Spec §6.10: a brand field box is `border:1px solid #000`,
    /// `padding:8px 12px`, no radius, with its `label.mono` above it.
    private func profileField(
        label: String,
        value: String,
        mono: Bool,
        minHeight: CGFloat?
    ) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            Text(label.uppercased())
                .font(CommunityBrand.Font_.labelMono)
                .tracking(CommunityBrand.Font_.monoTracking)
                .foregroundStyle(CommunityBrand.muted)
            Text(value)
                .font(mono ? CommunityBrand.Font_.fieldValueMono : CommunityBrand.Font_.fieldValue)
                .tracking(CommunityBrand.Font_.fieldValueTracking)
                .foregroundStyle(CommunityBrand.ink)
                .frame(maxWidth: .infinity, minHeight: minHeight, alignment: .topLeading)
                .padding(.vertical, TC.Space.s)
                .padding(.horizontal, TC.Space.m)
                .overlay(
                    Rectangle().strokeBorder(
                        CommunityBrand.ink,
                        lineWidth: CommunityBrand.Metric.rule
                    )
                )
        }
    }

    // MARK: - Watching and projects

    private var watching: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: "Watching")
            if let settings = model.daemonSettings {
                Text("A session counts as finished after \(settings.quiescenceSecs) seconds of quiet.")
                    .font(TC.Font_.body)
                Text("At most one notification every \(settings.digestIntervalSecs / 3600) hours, and none when nothing is waiting.")
                    .font(TC.Font_.body)
                Text("Undecided sessions are dropped after \(settings.queueTtlDays) days. Dropped means never sent.")
                    .font(TC.Font_.body)
                checkRow("Notifications rendered by this app", !settings.localNotifications)
            }
            if model.status.paused {
                Text("Paused. Nothing is being queued or sent.").font(TC.Font_.body)
            }
        }
    }

    // Only `ask` <-> `ignore` is offered here, same as onboarding screen 5:
    // arming `auto_upload` outside a deliberate confirmation flow is still
    // not built. Changing a mind about "ignore" -- set during onboarding or
    // never revisited since -- should not require a terminal, so that half
    // is wired to `AppModel.setProjectMode`, the same real `set_project_mode`
    // call the onboarding screen uses. See `DaemonClient.setProjectMode` for
    // why that call is expected to fail with `project-key-unrecognized` for
    // every real project today; this view surfaces that the same way
    // onboarding does rather than pretending the toggle always lands.
    private var projects: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: "Projects")
            if let error = model.lastActionError {
                Text(error).font(TC.Font_.body).foregroundStyle(.secondary)
            }
            if model.projects.isEmpty {
                Text("No projects seen yet.").font(TC.Font_.body).foregroundStyle(.secondary)
            } else {
                ForEach(model.projects) { project in
                    HStack {
                        Text(project.projectLabel)
                        Spacer()
                        Text(modeSentence(project.mode)).foregroundStyle(.secondary)
                        if project.mode == .ask || project.mode == .ignore {
                            Button(project.mode == .ignore ? "Ask again" : "Ignore") {
                                model.setProjectMode(
                                    project,
                                    mode: project.mode == .ignore ? .ask : .ignore
                                )
                            }
                            .buttonStyle(.bordered)
                        }
                    }
                    .font(TC.Font_.body)
                }
                Text("""
                Arming a project so it contributes without asking is a deliberate \
                confirmation flow, and it is not built yet.
                """)
                .font(TC.Font_.caption)
                .foregroundStyle(.secondary)
            }
        }
    }

    private func modeSentence(_ mode: ProjectMode) -> String {
        switch mode {
        case .ask: return "Asks you first"
        case .autoUpload: return "Contributed without asking"
        case .ignore: return "Never offered"
        }
    }

    /// Spec §5.4 / §6.9: a 12pt filled green disc carrying a white tick, then
    /// the label. Colour, glyph and words together -- the state survives
    /// greyscale.
    private func checkRow(_ title: String, _ value: Bool) -> some View {
        HStack(spacing: TC.Space.s) {
            Image(systemName: value ? "checkmark.circle.fill" : "circle")
                .font(.system(size: 12))
                .symbolRenderingMode(value ? .palette : .monochrome)
                .foregroundStyle(value ? TC.onAccent : Color.secondary, TC.green)
            Text(title).font(TC.Font_.body)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("\(title): \(value ? "yes" : "no")")
    }
}

// MARK: - The go-public dialog (spec §5.7)

/// Going public is a deliberate consent dialog, not a toggle flip: what gets
/// published and what never does sit side by side, nothing is pre-checked,
/// and "Go public" stays disabled until the acknowledgement is checked.
///
/// The sheet is a pure brand surface, edge to edge -- the private tool ends
/// at the sheet's boundary. Per §7.3 that seam is the design.
private struct GoPublicDialog: View {
    var onDismiss: () -> Void

    @State private var acknowledged = false

    /// Spec §4.6: the dialog is drawn at 560px.
    private static let width: CGFloat = 560

    /// This build has no path that writes consent -- see the Settings
    /// consent section's own note -- so the primary action stays disabled
    /// even once the acknowledgement is on, and the dialog says why rather
    /// than accepting a decision it cannot carry out. §5.7 draws only the
    /// disabled state, so nothing depicted is lost by that.
    private var canGoPublic: Bool { false }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.l) {
            Text("Put your handle on the public roster?".uppercased())
                .font(CommunityBrand.Font_.displayDialog)
                .tracking(CommunityBrand.Font_.displayDialogTracking)
                .foregroundStyle(CommunityBrand.ink)
                .fixedSize(horizontal: false, vertical: true)

            consentColumns

            acknowledgement

            HStack(spacing: TC.Space.sm) {
                Spacer(minLength: 0)
                Button("Not now") { onDismiss() }
                    .buttonStyle(CommunityBrandButtonStyle(fill: CommunityBrand.paper))
                Button("Go public") { onDismiss() }
                    .buttonStyle(CommunityBrandButtonStyle(fill: CommunityBrand.accent))
                    .disabled(!(acknowledged && canGoPublic))
            }

            Text("""
            Nothing is pre-checked, and Go public stays off until the \
            acknowledgement is on. This changes attribution only — it grants no \
            data use.
            """)
            .font(CommunityBrand.Font_.footnote)
            .foregroundStyle(CommunityBrand.muted)
            .fixedSize(horizontal: false, vertical: true)

            Text("""
            Joining the roster needs an enrolled account, which this build does \
            not set up yet, so Go public stays off here.
            """)
            .font(CommunityBrand.Font_.footnote)
            .foregroundStyle(CommunityBrand.muted)
            .fixedSize(horizontal: false, vertical: true)
        }
        .padding(TC.Space.xl)
        .frame(width: Self.width, alignment: .leading)
        .background(CommunityBrand.paper)
    }

    /// A single 2px box split by one 1px rule, per §5.7. The two columns are
    /// deliberately the same weight: what is published and what never is are
    /// the same size of fact.
    private var consentColumns: some View {
        HStack(alignment: .top, spacing: 0) {
            column(
                title: "What gets published",
                lines: [
                    "Your handle — real handles only, no pseudonyms.",
                    "Aggregate counts: accepted, novelty credit, accept rate.",
                    "The date you went public.",
                    "Your bio, if you write one."
                ]
            )
            Rectangle().fill(CommunityBrand.ink).frame(width: CommunityBrand.Metric.rule)
            column(
                title: "What never does",
                lines: [
                    "Your traces or anything in them.",
                    "Per-trace data of any kind.",
                    "Anything about sessions you didn't send."
                ]
            )
        }
        .fixedSize(horizontal: false, vertical: true)
        .overlay(
            Rectangle().strokeBorder(
                CommunityBrand.ink,
                lineWidth: CommunityBrand.Metric.frame
            )
        )
    }

    private func column(title: String, lines: [String]) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            Text(title.uppercased())
                .font(CommunityBrand.Font_.labelMono)
                .tracking(CommunityBrand.Font_.monoTracking)
                .foregroundStyle(CommunityBrand.muted)
            ForEach(lines, id: \.self) { line in
                Text(line)
                    .font(CommunityBrand.Font_.body)
                    .tracking(CommunityBrand.Font_.bodyTracking)
                    .foregroundStyle(CommunityBrand.ink)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.vertical, TC.Space.m)
        .padding(.horizontal, TC.Space.md)
    }

    /// Spec §6.9's brand checkbox: a bare 14x14 square with a 2px border and
    /// no fill. Checked adds a tick inside the same square -- the shape
    /// changes, not only the colour.
    private var acknowledgement: some View {
        Button {
            acknowledged.toggle()
        } label: {
            HStack(alignment: .top, spacing: TC.Space.sm) {
                ZStack {
                    Rectangle().strokeBorder(
                        CommunityBrand.ink,
                        lineWidth: CommunityBrand.Metric.frame
                    )
                    if acknowledged {
                        Image(systemName: "checkmark")
                            .font(.system(size: 9, weight: .heavy))
                            .foregroundStyle(CommunityBrand.ink)
                    }
                }
                .frame(width: 14, height: 14)
                .padding(.top, 1)
                Text("""
                I understand my handle and aggregate counts become public. Leaving \
                the roster removes me from future snapshots.
                """)
                .font(CommunityBrand.Font_.body)
                .tracking(CommunityBrand.Font_.bodyTracking)
                .foregroundStyle(CommunityBrand.ink)
                .fixedSize(horizontal: false, vertical: true)
                Spacer(minLength: 0)
            }
            .padding(.vertical, TC.Space.m)
            .padding(.horizontal, TC.Space.md)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(CommunityBrand.tint)
            .overlay(
            Rectangle().strokeBorder(
                CommunityBrand.ink,
                lineWidth: CommunityBrand.Metric.frame
            )
        )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(acknowledged ? [.isSelected] : [])
    }
}

